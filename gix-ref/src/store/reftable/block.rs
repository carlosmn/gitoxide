use super::blocksource::Source;
use super::record::Record;
use super::{BlockType, Error, Iter, Result, binsearch, decode_key, decode_keylen, get_be24};

use bytes::{Buf, Bytes};

use std::cmp::Ordering;
use std::ops::Range;

const DEFAULT_BLOCK_SIZE: u32 = 4096;

/// A block in the reftable.
///
/// Once created, a block is immutable
#[derive(Clone, Default)]
pub(crate) struct Block {
    /// Offset of the block header; nonzero for the first block in a reftable
    pub(crate) header_off: u32,

    /// Memory block
    pub(crate) block_data: Bytes,
    pub(crate) hash_size: u32,

    /* some compression fields for use with log entries come here */
    /// Restart point data. Restart points are located after the block's record
    /// data.
    pub(crate) restart_count: u16,
    pub(crate) restart_off: u32,

    /// Size of the data in the file. For log blocks, this is the compressed
    /// size.
    pub(crate) full_block_size: u32,

    pub(crate) block_type: Option<BlockType>,
}

/// Read a block from the source while making sure we don't run off the end
///
/// We cut off at the end of the block rather than returning an error like the
/// source itself would so we can guess at the block size in [`Block::new()`]
/// safely.
fn read_block(source: &dyn Source, off: u64, mut sz: u32) -> Result<Bytes> {
    let size = source.size();
    if off >= size {
        // The C git.git implementation returns 0 here after setting everything
        // to NULL, indicating success but trying to continue would lead to a
        // segfault. Here we can simulate that via unreachable!() as we in fact
        // do not expect the offset to ever be larger than the source.
        unreachable!();
    }

    if off.checked_add(sz as u64).ok_or(Error::IoError)? > size {
        sz = size
            .checked_sub(off)
            .and_then(|n| n.try_into().ok())
            .ok_or(Error::IoError)?;
    }

    source.read(off, sz)
}

impl Block {
    /// Create a new block from the source and given data from the table.
    pub fn new(
        source: &dyn Source,
        offset: u32,
        header_size: u32,
        table_block_size: u32,
        hash_size: u32,
        want_type: Option<BlockType>,
    ) -> Result<Self> {
        // If there is no given table block size we guess at a sensible 4k for
        // the block and adjust if necessary once we figure out the block size.
        let guess_block_size = if table_block_size > 0 {
            table_block_size
        } else {
            DEFAULT_BLOCK_SIZE
        };

        let mut block_data = read_block(source, offset as u64, guess_block_size)?;
        let block_type: BlockType = block_data[header_size as usize].try_into()?;
        if want_type.is_some() && want_type != Some(block_type) {
            return Err(Error::MismatchedBlockType);
        }

        let block_size = super::get_be24(&block_data[(header_size + 1) as usize..]);
        if block_size > guess_block_size {
            block_data = read_block(source, offset as u64, block_size)?;
        }

        let mut full_block_size = table_block_size;
        if block_type == BlockType::Log {
            /* Compression setup happens here  */
            todo!();
        } else if full_block_size == 0 {
            full_block_size = block_size;
        } else if block_size < full_block_size
            && block_size < block_data.len() as u32
            && block_data[block_size as usize] != 0
        {
            // If the block is smaller than the full block size, it is
            // padded (data followed by '\0') or the next block is
            // unaligned.
            full_block_size = block_size;
        }

        let restart_count = (&block_data[(block_size - 2) as usize..]).get_u16();
        let restart_off = block_size - 2 - (3 * restart_count) as u32;

        Ok(Self {
            header_off: header_size,
            block_data,
            hash_size,
            restart_count,
            restart_off,
            full_block_size,
            block_type: Some(block_type),
        })
    }

    /// Retrieve the first key of the block
    ///
    /// `key` is filled with the key.
    ///
    /// We should be able to return a `&[u8]` by doing a bit of manual work like
    /// with the search in [`Block::seek_key`] but for now we copy what git
    /// does. We expect to re-use the buffer so it's not a big deal generally.
    pub fn first_key(&self, key: &mut Vec<u8>) -> Result<()> {
        let off = (self.header_off + 4) as usize;
        let mut view = self.block_data.slice(off..);
        let _ = decode_key(&mut view, key)?;

        Ok(())
    }

    pub fn restart_offset(&self, idx: usize) -> u32 {
        let off = self.restart_off as usize + 3 * idx;
        get_be24(&self.block_data[off..])
    }
}

#[derive(Clone, Default)]
pub struct BlockIter {
    pub(crate) block: Block,
    /// Offset from the start of the block to the next block to read
    next_off: u32,

    /// Key for the last entry we read
    last_key: Vec<u8>,
    /// Re-used scratch buffer
    scratch: Vec<u8>,
}

impl BlockIter {
    pub fn from_block(block: Block) -> Self {
        let next_off = block.header_off + 4;
        Self {
            block,
            next_off,
            last_key: Vec::new(),
            scratch: Vec::new(),
        }
    }

    pub fn seek_start(&mut self) {
        self.next_off = self.block.header_off + 4;
        self.last_key.clear();
    }

    pub fn seek_key(&mut self, want: &[u8]) -> Result<()> {
        // Perform a binary search over the block's restart points, which
        // avoids doing a linear scan over the whole block. Like this, we
        // identify the section of the block that should contain our key.
        //
        // Note that we explicitly search for the first restart point _greater_
        // than the sought-after record, not _greater or equal_ to it. In case
        // the sought-after record is located directly at the restart point we
        // would otherwise start doing the linear search at the preceding
        // restart point. While that works alright, we would end up scanning
        // too many record.
        let block = &self.block;
        let i = binsearch(self.block.restart_count as usize, |idx| {
            restart_needle_less(idx, want, block)
        })?;

        // Now there are multiple cases:
        //
        //   - `i == 0`: The wanted record is smaller than the record found at
        //     the first restart point. As the first restart point is the first
        //     record in the block, our wanted record cannot be located in this
        //     block at all. We still need to position the iterator so that the
        //     next call to `block_iter_next()` will yield an end-of-iterator
        //     signal.
        //
        //   - `i == restart_count`: The wanted record was not found at any of
        //     the restart points. As there is no restart point at the end of
        //     the section the record may thus be contained in the last block.
        //
        //   - `i > 0`: The wanted record must be contained in the section
        //     before the found restart point. We thus do a linear search
        //     starting from the preceding restart point.
        if i > 0 {
            self.next_off = self.block.restart_offset(i - 1);
        } else {
            self.next_off = self.block.header_off + 4;
        }

        // We're looking for the last entry less than the wanted key so that
        // the next call to `block_reader_next()` would yield the wanted
        // record. We thus don't want to position our iterator at the sought
        // after record, but one before. To do so, we have to go one entry too
        // far and then back up.
        loop {
            let prev_off = self.next_off;
            let rec = Record::Want(self.block.block_type.expect("block is loaded"), None);
            let rec = match self.next(rec) {
                Some(Err(e)) => return Err(e),
                Some(Ok(rec)) => rec,
                None => {
                    self.next_off = prev_off;
                    return Ok(());
                }
            };

            // Check whether the current key is greater or equal to the
            // sought-after key. In case it is greater we know that the
            // record does not exist in the block and can thus abort early.
            // In case it is equal to the sought-after key we have found
            // the desired record.
            //
            // Note that we store the next record's key record directly in
            // `last_key` without restoring the key of the preceding record
            // in case we need to go one record back. This is safe to do as
            // `block_iter_next()` would return the ref whose key is equal
            // to `last_key` now, and naturally all keys share a prefix
            // with themselves.
            match self.last_key[..].cmp(want) {
                Ordering::Equal | Ordering::Greater => {
                    self.next_off = prev_off;
                    return Ok(());
                }
                Ordering::Less => {}
            }
        }
    }
}

fn restart_needle_less(idx: usize, needle: &[u8], block: &Block) -> Result<Ordering> {
    let off = block.restart_offset(idx) as usize;

    // Records at restart points are stored without prefix compression, so
    // there is no need to fully decode the record key here. This removes
    // the need for allocating memory.
    let mut view = block.block_data.slice(off..block.restart_off as usize);

    let (prefix_len, suffix_len, _extra) = decode_keylen(&mut view)?;
    if prefix_len > 0 {
        return Err(Error::FormatError);
    }

    // This is done via the numbers in the reference implementation which makes
    // it challenging to convert seamlessly into `Ordering`.
    //
    // n = memcmp(args->needle.buf, in.buf,
    //            args->needle.len < suffix_len ? args->needle.len : suffix_len);
    // if (n)
    //   return n < 0;
    // return args->needle.len < suffix_len;
    let len = needle.len().min(suffix_len as usize);
    let cmp = match needle[..len].cmp(&view[..len]) {
        Ordering::Equal if needle.len() < suffix_len as usize => Ordering::Greater,
        Ordering::Equal => Ordering::Equal,
        Ordering::Less => Ordering::Greater,
        Ordering::Greater => Ordering::Equal,
    };

    Ok(cmp)
}

impl super::Iter for BlockIter {
    fn seek(&mut self, _want: Record) -> Result<()> {
        todo!();
    }

    fn next(&mut self, rec: Record) -> Option<Result<Record>> {
        if self.next_off >= self.block.restart_off {
            return None;
        }

        let initial_len = (self.block.restart_off - self.next_off) as usize;
        let range = {
            let start = self.next_off as usize;
            let end = self.next_off as usize + initial_len;

            Range { start, end }
        };

        let mut data = self.block.block_data.slice(range);
        let extra = match decode_key(&mut data, &mut self.last_key) {
            Ok(extra) => extra,
            Err(e) => return Some(Err(e)),
        };
        if self.last_key.is_empty() {
            return Some(Err(Error::FormatError));
        }

        let record = match Record::decode(
            rec,
            &self.last_key,
            &mut data,
            extra,
            self.block.hash_size,
            &mut self.scratch,
        ) {
            Ok(rec) => rec,
            Err(e) => return Some(Err(e)),
        };

        self.next_off += (initial_len - data.remaining()) as u32;

        Some(Ok(record))
    }
}
