mod block;
mod blocksource;
mod record;
mod table;

use record::Record;

/// Reftable result with its own set of errors
type Result<T> = std::result::Result<T, Error>;

/// The type of the block in the reftable
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq)]
enum BlockType {
    Log = b'g',
    RefIndex = b'i',
    Ref = b'r',
    Obj = b'o',
}

impl From<BlockType> for u8 {
    fn from(value: BlockType) -> u8 {
        value as u8
    }
}

impl TryFrom<u8> for BlockType {
    type Error = Error;

    fn try_from(value: u8) -> Result<Self> {
        let v = match value {
            b'g' => BlockType::Log,
            b'i' => BlockType::RefIndex,
            b'r' => BlockType::Ref,
            b'o' => BlockType::Obj,
            _ => return Err(Error::FormatError),
        };

        Ok(v)
    }
}

impl PartialEq<u8> for BlockType {
    fn eq(&self, other: &u8) -> bool {
        *self as u8 == *other
    }
}

impl PartialEq<BlockType> for u8 {
    fn eq(&self, other: &BlockType) -> bool {
        *self == *other as u8
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("unexpected file system behavior")]
    IoError,
    #[error("format inconsistency on reading data")]
    FormatError,
    #[error("mismatched block type")]
    MismatchedBlockType,
    #[error("invalid offset")]
    InvalidOffset,
    #[error("there was an error in an iterator")]
    Iterator,
}

/// Common iterator trait for an iterator that yields records
trait Iter {
    /// Position the iterator at the wanted record such that a call to `next()`
    /// would return that record, if it exists.
    ///
    /// This probably actually wants a &Record and a Result<()>
    fn seek(&mut self, want: Record) -> Result<()>;

    /// Yield the next record and advance the iterator. Returns <0 on error, 0 when
    /// a record was yielded, and >0 when the iterator hit an error.
    ///
    /// Provide the last record provided so we can re-use allocations.
    /// Alternatively for the first time, provide a `Record::Empty` with the
    /// type you wish.
    fn next(&mut self, rec: Record) -> Option<Result<Record>>;
}

/// Read a big-endian 24 bit value as a u32
fn get_be24(buf: &[u8]) -> u32 {
    let bytes = [0, buf[0], buf[1], buf[2]];
    u32::from_be_bytes(bytes)
}

use bytes::{Buf, Bytes, buf::Reader};
use gix_features::decode::leb64_from_read;
use std::io::Read;

fn decode_keylen(b: &mut Bytes) -> Result<(u64, u64, u8)> {
    let (prefix_len, _) = leb64_from_read(b.reader()).map_err(|_| Error::FormatError)?;
    let (mut suffix_len, _) = leb64_from_read(b.reader()).map_err(|_| Error::FormatError)?;

    // We encode e.g. the value_type for references here
    let extra = (suffix_len & 0x7) as u8;
    suffix_len >>= 3;

    Ok((prefix_len, suffix_len, extra))
}

fn decode_key(b: &mut Bytes, last_key: &mut Vec<u8>) -> Result<u8> {
    let (prefix_len, suffix_len, extra) = decode_keylen(b)?;

    let len_left = b.remaining() as u64;
    if len_left < suffix_len || prefix_len > last_key.len() as u64 {
        return Err(Error::FormatError);
    }

    // Most of the time refs aren't wildly different lengths so we expect the
    // initialization isn't going to be a significant cost.
    last_key.resize((prefix_len + suffix_len) as usize, 0);
    b.reader()
        .read_exact(&mut last_key[prefix_len as usize..])
        .map_err(|_| Error::IoError)?;

    Ok(extra)
}
