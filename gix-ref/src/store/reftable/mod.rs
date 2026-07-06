mod blocksource;

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
}
