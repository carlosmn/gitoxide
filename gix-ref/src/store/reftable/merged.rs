use super::record::Record;
use super::table::{Table, TableIter};
use super::{BlockType, Error, Result};

use std::collections::BinaryHeap;
use std::rc::Rc;

pub struct MergedTable {
    tables: Vec<Rc<Table>>,
    hash_id: gix_hash::Kind,
    /// If unset, produce deletions. This is useful for compaction. For the
    /// full stack, deletions should be produced.
    suppress_deletions: bool,
    min: u64,
    max: u64,
}

impl MergedTable {
    pub fn new(tables: Vec<Rc<Table>>, hash_id: gix_hash::Kind) -> Result<Self> {
        let mut last_max = 0;
        let mut first_min = 0;

        for (i, table) in tables.iter().enumerate() {
            let min = table.min_update_index;
            let max = table.max_update_index;

            if table.hash_id != hash_id {
                return Err(Error::FormatError);
            }

            if i == 0 || min < first_min {
                first_min = min;
            }
            if i == 0 || max > last_max {
                last_max = max;
            }
        }

        Ok(Self {
            tables,
            hash_id,
            suppress_deletions: false,
            min: first_min,
            max: last_max,
        })
    }
}

struct SubIter {
    /// The iterator we're going over
    iter: TableIter,
    /// The last record our iterator produced
    rec: Record,
}

pub struct MergedIter {
    subiters: Vec<SubIter>,
    pq: BinaryHeap<Record>,
    suppress_deletions: bool,
    // TODO: this can probably be Option<usize>
    advance_index: isize,
}

struct PqEntry<'r> {
    /// The subiter this came from
    index: usize,
    rec: &'r Record,
}

impl MergedIter {
    pub fn from_merged(mt: &MergedTable, typ: BlockType) -> Self {
        let mut subiters = Vec::with_capacity(mt.tables.len());
        for table in mt.tables.iter() {
            let iter = TableIter::new(table.clone(), typ);
            subiters.push(SubIter {
                iter,
                rec: Record::Want(typ, None),
            });
        }

        Self {
            subiters,
            pq: BinaryHeap::new(),
            suppress_deletions: mt.suppress_deletions,
            advance_index: -1,
        }
    }
}
