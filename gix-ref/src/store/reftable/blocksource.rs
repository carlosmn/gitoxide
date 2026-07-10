//! Possible sources for the table files

use super::{Error, Result};

use bytes::Bytes;

pub use buffer::BufferSource;
pub use file::FileSource;

/// Interface implemented by any kind of source
pub(crate) trait Source: Send + Sync {
    /// Read from the source at the specified offset
    fn read(&self, offset: u64, len: u32) -> Result<Bytes>;
    /// Total size of the source
    fn size(&self) -> u64;
}

//impl Source for Box<>

pub mod buffer {
    use super::{Error, Result};
    use bytes::Bytes;

    /// Source backed by a buffer in memory
    ///
    /// This is mostly useful for unit tests
    pub struct BufferSource {
        buf: Vec<u8>,
    }

    impl BufferSource {
        pub fn from_slice(slice: &[u8]) -> Self {
            let mut buf = Vec::with_capacity(slice.len());
            buf.extend_from_slice(slice);
            Self { buf }
        }
    }

    impl super::Source for BufferSource {
        fn read(&self, offset: u64, len: u32) -> Result<Bytes> {
            // Enforce it can fit in usize
            let offset: usize = offset.try_into().unwrap();
            let end = offset.checked_add(len as usize).ok_or(Error::IoError)?;
            assert!(end <= self.buf.len());

            Ok(Bytes::copy_from_slice(&self.buf[offset..end]))
        }

        fn size(&self) -> u64 {
            self.buf.len() as u64
        }
    }
}

pub mod file {
    use super::{Error, Result};
    use bytes::Bytes;
    use std::fs::File;
    use std::path::Path;

    /// Source backed by an mapped file
    pub struct FileSource {
        f: File,
        b: Bytes,
    }

    #[allow(unsafe_code)]
    impl FileSource {
        pub fn new<P: AsRef<Path>>(path: P) -> std::io::Result<Self> {
            let f = File::open(path)?;
            let mmap = unsafe { memmap2::Mmap::map(&f)? };

            Ok(Self {
                f,
                b: Bytes::from_owner(mmap),
            })
        }
    }

    impl super::Source for FileSource {
        fn read(&self, offset: u64, len: u32) -> Result<Bytes> {
            // Enforce it can fit in usize
            let offset: usize = offset.try_into().unwrap();
            let end = offset.checked_add(len as usize).ok_or(Error::IoError)?;
            assert!(end <= self.b.len());

            Ok(self.b.slice(offset..end))
        }

        fn size(&self) -> u64 {
            self.b.len() as u64
        }
    }
}
