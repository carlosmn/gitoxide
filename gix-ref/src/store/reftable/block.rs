use super::blocksource::Source;
use super::record::Record;
use super::{BlockType, Error, Result, decode_key};

use bytes::{Buf, Bytes};

use std::ops::Range;

const DEFAULT_BLOCK_SIZE: u32 = 4096;

/// A block in the reftable.
///
/// Once created, a block is immutable
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

    pub(crate) block_type: BlockType,
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
            block_type,
        })
    }
}

pub struct BlockIter {
    block: Block,
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
    }

    /// Get the next record for this iterator
    ///
    /// Provide the last record provided so we can re-use allocations.
    /// Alternatively for the first time, provide a `Record::Empty` with the
    /// type you wish.
    pub fn next(&mut self, rec: Record) -> Option<Result<Record>> {
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
