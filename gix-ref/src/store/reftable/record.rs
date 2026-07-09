//! The different kinds of records in reftables

use crate::ObjectId;

use gix_features::decode::leb64_from_read;

use bytes::{Buf, Bytes};

use super::{BlockType, Error, Result};

use std::cmp::Ordering;

#[derive(Clone, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum RefValueType {
    /// Tombstone to hide deletions from earlier tables
    Deletion = 0x0,
    /// A simple ref
    Val1 = 0x1,
    /// A tag plus its peeled hash
    Val2 = 0x2,
    /// A symbolic reference
    Symref = 0x3,
}

impl TryFrom<u8> for RefValueType {
    type Error = Error;

    fn try_from(value: u8) -> Result<Self> {
        let val = match value {
            0x0 => Self::Deletion,
            0x1 => Self::Val1,
            0x2 => Self::Val2,
            0x3 => Self::Symref,
            _ => return Err(Error::FormatError),
        };

        Ok(val)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RefValue {
    Val1(ObjectId),
    Val2(ObjectId, ObjectId),
    Symref(Vec<u8>),
}

fn decode_string(b: &mut Bytes) -> Result<Vec<u8>> {
    let (tsize, _) = leb64_from_read(b.reader()).map_err(|_| Error::FormatError)?;
    if (b.remaining() as u64) < tsize {
        return Err(Error::FormatError);
    }

    let v = b[..tsize as usize].to_vec();
    b.advance(tsize as usize);

    Ok(v)
}

/// A record from a table
///
/// We use an enum instead of a trait plus structs so we don't have to box every
/// record.
#[derive(Clone, Debug, Eq)]
pub enum Record {
    Ref(RefRecord),
    Log(LogRecord),
    Obj(ObjRecord),
    Index(IndexRecord),
}

impl Record {
    pub fn for_search(typ: BlockType, key: Option<Vec<u8>>) -> Self {
        match typ {
            BlockType::Ref => Record::Ref(RefRecord::for_search(key)),
            BlockType::Log => Record::Log(LogRecord::for_search(key)),
            BlockType::Obj => Record::Obj(ObjRecord::for_search(key)),
            BlockType::Index => Record::Index(IndexRecord::for_search(key)),
        }
    }

    pub fn record_type(&self) -> BlockType {
        match self {
            Self::Ref(_) => BlockType::Ref,
            Self::Log(_) => BlockType::Log,
            Self::Obj(_) => BlockType::Obj,
            Self::Index(_) => BlockType::Index,
        }
    }

    pub fn clone_key(&self) -> Vec<u8> {
        match self {
            Self::Ref(RefRecord { refname, .. }) => refname.clone(),
            Self::Index(IndexRecord { last_key, .. }) => last_key.clone(),
            _ => todo!(),
        }
    }

    /// Decode the record given by the type in `rec`.
    ///
    /// Replacing the record in-place allows us to reduce allocations. This is
    /// an optimisation copied from the implementation in git.git.
    pub fn decode(
        rec: &mut Self,
        key: &[u8],
        b: &mut Bytes,
        extra: u8,
        hash_size: u32,
        scratch: &mut Vec<u8>,
    ) -> Result<()> {
        match rec {
            Self::Ref(rec) => RefRecord::decode(rec, key, b, extra, hash_size, scratch),
            Self::Index(rec) => IndexRecord::decode(rec, key, b, extra, hash_size, scratch),
            _ => todo!(),
        }
    }

    pub fn is_deletion(&self) -> bool {
        match self {
            Self::Ref(rec) => rec.is_deletion(),
            Self::Log(rec) => rec.is_deletion(),
            Self::Obj(rec) => rec.is_deletion(),
            Self::Index(rec) => rec.is_deletion(),
        }
    }
}

impl PartialEq for Record {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Ref(a), Self::Ref(b)) => a.eq(b),
            (Self::Log(a), Self::Log(b)) => a.eq(b),
            (Self::Obj(a), Self::Obj(b)) => a.eq(b),
            (Self::Index(a), Self::Index(b)) => a.eq(b),
            _ => false,
        }
    }
}

impl PartialOrd for Record {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        match (self, other) {
            (Self::Ref(a), Self::Ref(b)) => a.partial_cmp(b),
            (Self::Log(a), Self::Log(b)) => a.partial_cmp(b),
            (Self::Obj(a), Self::Obj(b)) => a.partial_cmp(b),
            (Self::Index(a), Self::Index(b)) => a.partial_cmp(b),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RefRecord {
    pub(crate) refname: Vec<u8>,
    pub(crate) update_index: u64,
    pub(crate) value_type: RefValueType,
    pub(crate) value: Option<RefValue>,
}

impl RefRecord {
    pub fn for_search(refname: Option<Vec<u8>>) -> Self {
        let refname = refname.unwrap_or_default();

        Self {
            refname,
            update_index: 0,
            value_type: RefValueType::Deletion,
            value: None,
        }
    }

    pub fn decode(
        rec: &mut Self,
        key: &[u8],
        b: &mut Bytes,
        val_type: u8,
        hash_size: u32,
        _scratch: &mut Vec<u8>,
    ) -> Result<()> {
        let mut refname = std::mem::take(&mut rec.refname);

        let (update_index, _) = leb64_from_read(b.reader()).map_err(|_| Error::FormatError)?;
        let value_type: RefValueType = val_type.try_into()?; // C version aborts

        let hash_size = hash_size as usize;
        let value = match value_type {
            RefValueType::Val1 => {
                if b.remaining() < hash_size {
                    return Err(Error::FormatError);
                }

                let val = ObjectId::from_bytes_or_panic(&b[..hash_size]);
                b.advance(hash_size);

                Some(RefValue::Val1(val))
            }
            RefValueType::Val2 => {
                if b.remaining() < (2 * hash_size) {
                    return Err(Error::FormatError);
                }

                let val1 = ObjectId::from_bytes_or_panic(&b[..hash_size]);
                b.advance(hash_size);
                let val2 = ObjectId::from_bytes_or_panic(&b[..hash_size]);
                b.advance(hash_size);

                Some(RefValue::Val2(val1, val2))
            }
            RefValueType::Symref => {
                let target = decode_string(b)?;
                Some(RefValue::Symref(target))
            }
            RefValueType::Deletion => None,
        };

        refname.clear();
        refname.extend_from_slice(key);

        *rec = Self {
            refname,
            update_index,
            value_type,
            value,
        };

        Ok(())
    }

    pub fn is_deletion(&self) -> bool {
        matches!(self.value_type, RefValueType::Deletion)
    }
}

impl PartialOrd for RefRecord {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.refname.partial_cmp(&other.refname)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[repr(u8)]
enum LogValueType {
    /// Tombstone to hide deletions from earlier tables
    Deletion = 0x0,
    /// A simple update
    Update = 0x1,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogRecord {
    refname: Vec<u8>,
    value_type: LogValueType,
    update_index: u64,

    new_hash: ObjectId,
    old_hash: ObjectId,
    name: Vec<u8>,
    email: Vec<u8>,
    time: u64,
    tz_offset: u16,
    message: Vec<u8>,
}

impl LogRecord {
    fn for_search(refname: Option<Vec<u8>>) -> Self {
        let refname = refname.unwrap_or_default();

        Self {
            refname,
            value_type: LogValueType::Deletion,
            update_index: 0,
            new_hash: ObjectId::null(gix_hash::Kind::Sha1),
            old_hash: ObjectId::null(gix_hash::Kind::Sha1),
            name: Vec::new(),
            email: Vec::new(),
            time: 0,
            tz_offset: 0,
            message: Vec::new(),
        }
    }

    pub fn is_deletion(&self) -> bool {
        matches!(self.value_type, LogValueType::Deletion)
    }
}

impl PartialOrd for LogRecord {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        let cmp = self.refname.partial_cmp(&other.refname);
        if cmp != Some(Ordering::Equal) {
            return cmp;
        }

        // Note that the comparison here is reversed. This is because the
        // update index is reversed when comparing keys. For reference, see how
        // we handle this in reftable_log_record_key()`.
        other.update_index.partial_cmp(&self.update_index)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjRecord {
    /// Leading bytes of the object ID
    hash_prefix: Vec<u8>,
    /// A vector of file offsets
    offsets: Vec<u64>,
}

impl ObjRecord {
    pub fn for_search(hash_prefix: Option<Vec<u8>>) -> Self {
        let hash_prefix = hash_prefix.unwrap_or_default();

        Self {
            hash_prefix,
            offsets: Vec::new(),
        }
    }

    pub fn is_deletion(&self) -> bool {
        false
    }
}

impl PartialOrd for ObjRecord {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.hash_prefix.partial_cmp(&other.hash_prefix)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexRecord {
    /// Offset of block
    pub(crate) offset: u64,
    /// Last key of the block
    pub(crate) last_key: Vec<u8>,
}

impl IndexRecord {
    pub fn for_search(last_key: Option<Vec<u8>>) -> Self {
        let last_key = last_key.unwrap_or_default();

        Self { offset: 0, last_key }
    }

    pub fn decode(
        rec: &mut Self,
        key: &[u8],
        b: &mut Bytes,
        _val_type: u8,
        _hash_size: u32,
        _scratch: &mut Vec<u8>,
    ) -> Result<()> {
        rec.last_key.clear();
        rec.last_key.extend_from_slice(key);

        let (offset, _) = leb64_from_read(b.reader()).map_err(|_| Error::FormatError)?;
        rec.offset = offset;

        Ok(())
    }

    pub fn is_deletion(&self) -> bool {
        false
    }
}

impl PartialOrd for IndexRecord {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.last_key.partial_cmp(&other.last_key)
    }
}
