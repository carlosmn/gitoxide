//! A single reftable file

use super::block::Block;
use super::blocksource::Source;

use super::{BlockType, Error, Result};

use bytes::Buf;

use std::path::PathBuf;

/// Return the header size for the given version
fn header_size(version: u8) -> u32 {
    match version {
        1 => 24,
        2 => 28,
        _ => unreachable!(),
    }
}

fn footer_size(version: u8) -> u32 {
    match version {
        1 => 68,
        2 => 72,
        _ => unreachable!(),
    }
}

fn hash_size(hash: gix_hash::Kind) -> u32 {
    hash.len_in_bytes() as u32
}

/// Where each block and its index live
#[derive(Debug, Default)]
struct Offsets {
    /// Offset for this block
    offset: u64,
    /// Offset for the index corresponding to this block
    index_offset: u64,
}

pub struct Table {
    /// Name used to identify this table, typically the basename of the file.
    /// This will almost definitely be an ASCII string but the spec doesn't
    /// specify anything beyond a recommendation.
    name: PathBuf,

    source: Box<dyn Source>,

    /// Size of the file excluding footer
    size: u64,
    version: u8,
    block_size: u32,

    min_update_index: u64,
    max_update_index: u64,
    /// Only available in v2, defaults to SHA-1
    hash_id: gix_hash::Kind,

    ref_offsets: Option<Offsets>,
    obj_offsets: Option<Offsets>,
    log_offsets: Option<Offsets>,
}
impl Table {
    pub fn new(source: Box<dyn Source>, name: PathBuf) -> Result<Self> {
        let file_size = source.size();

        // Use v2 for the larger size plus one extra byte to get the type of the
        // first block
        let read_size = header_size(2) + 1;
        if read_size as u64 > file_size {
            return Err(Error::FormatError);
        }

        let header = source.read(0, read_size)?;
        if &header[..4] != b"REFT" {
            return Err(Error::FormatError);
        }

        let version = header[4];
        if version != 1 && version != 2 {
            return Err(Error::FormatError);
        }

        // Table size not including the footer
        let size = file_size - footer_size(version) as u64;
        let mut footer = source.read(size, footer_size(version))?;

        if &footer[..4] != b"REFT" {
            return Err(Error::FormatError);
        }

        if header[..header_size(version) as usize] != footer[..header_size(version) as usize] {
            return Err(Error::FormatError);
        }

        // Create a copy so we can from `footer` but leave a reference so we can
        // do the crc32 later.
        let footer_start = footer.clone();

        // Skip over the `REFT` and version we've already handled.
        footer.advance(5);

        let block_size = super::get_be24(&footer[..]);
        footer.advance(3);

        let min_update_index = footer.get_u64();
        let max_update_index = footer.get_u64();

        let hash_id = if version == 1 {
            gix_hash::Kind::Sha1
        } else {
            let kind = match &footer[..4] {
                b"sha1" => gix_hash::Kind::Sha1,
                b"s256" => gix_hash::Kind::Sha256,
                _ => return Err(Error::FormatError),
            };
            footer.advance(4);
            kind
        };

        let ref_index_offset = footer.get_u64();

        let mut obj_offset = footer.get_u64();
        let object_id_len = obj_offset & ((1 << 5) - 1);
        obj_offset >>= 5;

        let obj_index_offset = footer.get_u64();
        let log_offset = footer.get_u64();
        let log_index_offset = footer.get_u64();

        // crc32 covers the footer up to the CRC itself
        let computed_crc = gix_features::hash::crc32_update(0, &footer_start[..footer_start.len() - 4]);
        let file_crc = footer.get_u32();

        if computed_crc != file_crc {
            return Err(Error::FormatError);
        }

        let first_block_type = header[header_size(version) as usize];
        let obj_offsets_present = obj_offset > 0;

        if obj_offsets_present && object_id_len == 0 {
            return Err(Error::FormatError);
        }

        let ref_offsets = (first_block_type == BlockType::Ref).then_some(Offsets {
            offset: 0,
            index_offset: ref_index_offset,
        });

        let log_offsets = (first_block_type == BlockType::Log || log_offset > 0).then_some(Offsets {
            offset: log_offset,
            index_offset: log_index_offset,
        });

        let obj_offsets = (obj_offset > 0).then_some(Offsets {
            offset: obj_offset,
            index_offset: obj_index_offset,
        });

        Ok(Self {
            name,
            source,
            size,
            version,
            block_size,
            min_update_index,
            max_update_index,
            hash_id,
            ref_offsets,
            obj_offsets,
            log_offsets,
        })
    }

    pub fn init_block(&self, next_off: u64, want_type: Option<BlockType>) -> Result<Block> {
        let header_off = if next_off > 0 { 0 } else { header_size(self.version) };
        if next_off >= self.size {
            return Err(Error::InvalidOffset);
        }

        Block::new(
            self.source.as_ref(),
            next_off as u32,
            header_off,
            self.block_size,
            hash_size(self.hash_id),
            want_type,
        )
    }
}

#[cfg(test)]
mod test {
    use super::super::blocksource::BufferSource;
    use super::{BlockType, header_size};

    // This is from a fresh git repository with an unborn main branch
    const INITIAL_REF_FILE: [u8; 124] = [
        0x52, 0x45, 0x46, 0x54, 0x01, 0x00, 0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x72, 0x00, 0x00, 0x38, 0x00, 0x23, 0x48, 0x45, 0x41, 0x44, 0x00, 0x0f,
        0x72, 0x65, 0x66, 0x73, 0x2f, 0x68, 0x65, 0x61, 0x64, 0x73, 0x2f, 0x6d, 0x61, 0x69, 0x6e, 0x00, 0x00, 0x1c,
        0x00, 0x01, 0x52, 0x45, 0x46, 0x54, 0x01, 0x00, 0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xb6, 0xbf, 0xf7, 0x8a,
    ];

    #[test]
    fn test_initial_table() {
        let source = Box::new(BufferSource::from_slice(&INITIAL_REF_FILE[..]));
        let table = super::Table::new(source, "01-01-rand.ref".into()).expect("parsing");

        assert_eq!(1, table.version);
        assert_eq!(gix_hash::Kind::Sha1, table.hash_id);
        assert_eq!(4096, table.block_size);
        assert_eq!(1, table.min_update_index);
        assert_eq!(1, table.max_update_index);

        let block = table.init_block(0, Some(BlockType::Ref)).expect("first ref block");

        assert_eq!(header_size(table.version), block.header_off);
        assert_eq!(1, block.restart_count);
        assert_eq!(BlockType::Ref, block.block_type);
        assert_eq!(56, block.full_block_size);
    }
}
