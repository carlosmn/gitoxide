use super::blocksource::FileSource;
use super::merged::MergedTable;
use super::table::Table;
use super::{Error, Result};

use std::fs::{File, Metadata};
use std::io::{BufRead, BufReader, ErrorKind};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, SystemTime};

/// Options for how the reftable code should behave
///
/// This is called `reftable_write_options` in git.git but it also controls the
/// hash on read.
pub struct Options {
    hash_id: gix_hash::Kind,
}

impl Options {
    pub fn from_init_options(opts: crate::store::init::Options) -> Self {
        Self {
            hash_id: opts.object_hash,
        }
    }
}

impl Default for Options {
    fn default() -> Self {
        Self {
            hash_id: gix_hash::Kind::Sha1,
        }
    }
}

pub struct Stack {
    list_file: PathBuf,
    file: Option<File>,
    file_md: Option<Metadata>,
    reftable_dir: PathBuf,
    opts: Options,
    tables: Vec<Rc<Table>>,
    merged: Option<MergedTable>,
}

impl Stack {
    pub fn open<P: AsRef<Path>>(dir: P, opts: Option<Options>) -> Result<Self> {
        let dir = dir.as_ref();
        let list_file = dir.join("tables.list");

        let mut me = Self {
            list_file,
            file: None,
            file_md: None,
            reftable_dir: dir.into(),
            opts: opts.unwrap_or_default(),
            tables: Vec::new(),
            merged: None,
        };

        me.reload_maybe_reuse(true)?;

        Ok(me)
    }

    fn reload_maybe_reuse(&mut self, reuse_open: bool) -> Result<()> {
        let deadline = SystemTime::now() + Duration::from_secs(3);
        let mut tries = 0;
        let mut delay = 0_u32;
        let mut file = None;

        loop {
            let now = SystemTime::now();

            // Only look at deadlines after the first few times. This
            // simplifies debugging in GDB.
            tries += 1;
            if tries > 3 && now >= deadline {
                break;
            }

            // Open the file. If it doesn't exist the we pretend it's empty.
            let (f, names) = read_lines(&self.list_file)?;

            match self.reload_once(&names, reuse_open) {
                Ok(_) => break,
                Err(Error::NotExist) => {}
                Err(e) => return Err(e),
            }
            file = f;

            // REFTABLE_NOT_EXIST_ERROR can be caused by a concurrent
            // writer. Check if there was one by checking if the name list
            // changed.
            let (_, names_after) = read_lines(&self.list_file)?;
            if names == names_after {
                return Err(Error::NotExist);
            }

            let delay_mult = gix_utils::rng::usize(0..u32::MAX as usize) as u32;
            delay = delay + delay.wrapping_mul(delay_mult) + 1;
            std::thread::sleep(Duration::from_millis(delay as u64));
        }

        // Invalidate the stat cache. It is sufficient to only close the file
        // descriptor and keep the cached stat info because we never use the
        // latter when the former is negative.
        self.file.take();

        // Cache stat information in case it provides a useful signal to us.
        // According to POSIX, "The st_ino and st_dev fields taken together
        // uniquely identify the file within the system." That being said,
        // Windows is not POSIX compliant and we do not have these fields
        // available. So the information we have there is insufficient to
        // determine whether two file descriptors point to the same file.
        //
        // While we could fall back to using other signals like the file's
        // mtime, those are not sufficient to avoid races. We thus refrain from
        // using the stat cache on such systems and fall back to the secondary
        // caching mechanism, which is to check whether contents of the file
        // have changed.
        //
        // On other systems which are POSIX compliant we must keep the file
        // descriptor open. This is to avoid a race condition where two
        // processes access the reftable stack at the same point in time:
        //
        //   1. A reads the reftable stack and caches its stat info.
        //
        //   2. B updates the stack, appending a new table to "tables.list".
        //      This will both use a new inode and result in a different file
        //      size, thus invalidating A's cache in theory.
        //
        //   3. B decides to auto-compact the stack and merges two tables. The
        //      file size now matches what A has cached again. Furthermore, the
        //      filesystem may decide to recycle the inode number of the file
        //      we have replaced in (2) because it is not in use anymore.
        //
        //   4. A reloads the reftable stack. Neither the inode number nor the
        //      file size changed. If the timestamps did not change either then
        //      we think the cached copy of our stack is up-to-date.
        //
        // By keeping the file descriptor open the inode number cannot be
        // recycled, mitigating the race.
        if let Some(file) = file {
            if let Ok(md) = file.metadata() {
                // TOD: we would need a version of this for non-unix
                if md.dev() != 0 && md.ino() != 0 {
                    self.file = Some(file);
                    self.file_md = Some(md);
                }
            }
        }

        // TODO: upstream has a callback here on_reload

        Ok(())
    }

    fn reload_once(&mut self, names: &[String], reuse_open: bool) -> Result<()> {
        let mut new_tables = Vec::with_capacity(names.len());
        let mut cur = self.tables.clone();

        for name in names {
            // this is linear; we assume compaction keeps the number of
            // tables under control so this is not quadratic.
            let found_table = reuse_open
                .then(|| cur.iter().position(|t| t.name() == name).map(|i| cur.remove(i)))
                .flatten();

            match found_table {
                Some(table) => new_tables.push(table),
                None => {
                    let source = match FileSource::new(self.filename_for(name)) {
                        Ok(s) => s,
                        Err(e) if e.kind() == ErrorKind::NotFound => return Err(Error::NotExist),
                        Err(_) => return Err(Error::IoError),
                    };
                    let t = Table::new(Box::new(source), name.into())?;
                    new_tables.push(Rc::new(t));
                }
            }
        }

        let new_merged = MergedTable::new(new_tables.clone(), self.opts.hash_id)?;

        self.merged = Some(new_merged);
        self.tables = new_tables;

        Ok(())
    }

    /// Generate a path for the given reftable file
    fn filename_for(&self, name: &str) -> PathBuf {
        self.reftable_dir.join(name)
    }
}

/// Open `filename` and read the lines from it
///
/// If the file does not exist, we pretend it's empty.
fn read_lines<P: AsRef<Path>>(filename: P) -> Result<(Option<File>, Vec<String>)> {
    let f = match File::open(filename) {
        Ok(f) => f,
        Err(e) if e.kind() == ErrorKind::NotFound => {
            return Ok((None, Vec::new()));
        }
        Err(_) => return Err(Error::IoError),
    };

    let mut reader = BufReader::new(f);
    let mut names = Vec::new();
    loop {
        let mut s = String::with_capacity(43);
        match reader.read_line(&mut s) {
            Ok(0) => break,
            Ok(_) => {
                // LF is included
                let popped = s.pop();
                if popped != Some('\n') {
                    return Err(Error::FormatError);
                }
                names.push(s);
            }
            Err(_) => return Err(Error::IoError),
        }
    }

    Ok((Some(reader.into_inner()), names))
}
