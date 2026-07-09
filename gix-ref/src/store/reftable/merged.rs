use super::record::Record;
use super::table::{Table, TableIter};
use super::{BlockType, Error, Iter, Result};
use gix_features::threading::{Mutable, OwnShared, lock};

use std::cmp::Ordering;
use std::collections::BinaryHeap;

pub struct MergedTable {
    tables: Vec<OwnShared<Table>>,
    hash_id: gix_hash::Kind,
    /// If unset, produce deletions. This is useful for compaction. For the
    /// full stack, deletions should be produced.
    suppress_deletions: bool,
    min: u64,
    max: u64,
}

impl MergedTable {
    pub fn new(tables: Vec<OwnShared<Table>>, hash_id: gix_hash::Kind) -> Result<Self> {
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

pub struct MergedIter {
    /// List of iterators. We diverge from upstream wrt subiterators because
    /// keeping references around like they do is hard to prove in Rust.
    iters: Vec<TableIter>,
    /// The last record the nth iterator produced
    recs: OwnShared<Vec<Mutable<Record>>>,

    pq: BinaryHeap<PqEntry>,
    suppress_deletions: bool,
    // TODO: this can probably be Option<usize>
    advance_index: isize,
}

struct PqEntry {
    /// The subiter this came from
    index: usize,
    /// Our way to figure out what the nth record is
    recs: OwnShared<Vec<Mutable<Record>>>,
}

impl Eq for PqEntry {}

fn cmp_records(recs: &[Mutable<Record>], lhs: usize, rhs: usize) -> Option<Ordering> {
    if lhs == rhs {
        return Some(Ordering::Equal);
    }

    let lhs = lock(&recs[lhs]);
    let rhs = lock(&recs[rhs]);
    lhs.partial_cmp(&rhs)
}

impl PartialEq for PqEntry {
    fn eq(&self, other: &Self) -> bool {
        cmp_records(&self.recs, self.index, other.index) == Some(Ordering::Equal)
    }
}

#[allow(clippy::non_canonical_partial_ord_impl)]
impl PartialOrd for PqEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        // We want a min-heap but BinaryHeap is a max-heap so we return this the other way around
        cmp_records(&self.recs, other.index, self.index)
    }
}

impl Ord for PqEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        // We shouldn't panic in Ord because implementing it means that there is
        // a total order (or just weak order). However we're doing this for the
        // benefit of BinaryHeap and we check beforehand that we're not going to
        // get into this situation.
        //
        // An alternative would be to do what upstream does and return
        // -1/Ordering::Less if given different record types. This breaks
        // a different promise of implementing Ord because we'd make both `a <
        // b` and `a > b` true.
        //
        // This is also why PartialOrd doesn't just defer to Ord (though we
        // might not end up using them differently).
        //
        // As in `PartialOrd`, we want a min-heap but BinaryHeap is a max-heap
        // so we return this the other way around
        cmp_records(&self.recs, other.index, self.index).expect("we should never see different types in the iterator")
    }
}

impl MergedIter {
    pub fn from_merged(mt: &MergedTable, typ: BlockType) -> Self {
        let mut iters = Vec::with_capacity(mt.tables.len());
        let mut recs = Vec::with_capacity(mt.tables.len());
        for table in mt.tables.iter() {
            iters.push(TableIter::new(table.clone(), typ));
            recs.push(Mutable::new(Record::for_search(typ, None)));
        }

        Self {
            iters,
            recs: OwnShared::new(recs),
            pq: BinaryHeap::new(),
            suppress_deletions: mt.suppress_deletions,
            advance_index: -1,
        }
    }

    pub fn advance_subiter(&mut self, i: usize) -> Result<bool> {
        let iter = &mut self.iters[i];
        let mut rec = lock(&self.recs[i]);

        if !iter.next(&mut rec)? {
            return Ok(false);
        }
        drop(rec);

        let entry = PqEntry {
            index: i,
            recs: self.recs.clone(),
        };

        self.pq.push(entry);

        Ok(true)
    }

    fn next_entry(&mut self, rec: &mut Record) -> Result<bool> {
        let mut empty = self.pq.is_empty();

        if self.advance_index >= 0 {
            let i = self.advance_index as usize;

            // When there are no pqueue entries then we only have a single
            // subiter left. There is no need to use the pqueue in that
            // case anymore as we know that the subiter will return entries
            // in the correct order already.
            //
            // While this may sound like a very specific edge case, it may
            // happen more frequently than you think. Most repositories
            // will end up having a single large base table that contains
            // most of the refs. It's thus likely that we exhaust all
            // subiters but the one from that base ref.
            if empty {
                return self.iters[i].next(rec);
            }

            self.advance_subiter(i)?;

            empty = false; // is this necesary?
            self.advance_index = -1;
        }

        let entry = match self.pq.pop() {
            Some(entry) => entry,
            None => return Ok(false),
        };

        // One can also use reftable as datacenter-local storage, where the ref
        // database is maintained in globally consistent database (eg.
        // CockroachDB or Spanner). In this scenario, replication delays together
        // with compaction may cause newer table;s to contain older entries. In
        // such a deployment, the loop below must be changed to collect all
        // entries for the same key, and return new the newest one.
        while let Some(top) = self.pq.peek() {
            match cmp_records(&self.recs, top.index, entry.index) {
                None => return Err(Error::Iterator),
                Some(Ordering::Greater) => break,
                _ => {}
            }

            let top_index = top.index;
            self.pq.pop().expect("pq is not empty");
            self.advance_subiter(top_index)?;
        }

        self.advance_index = entry.index as isize;
        let mut entry_rec = lock(&self.recs[entry.index]);
        std::mem::swap(rec, &mut entry_rec);

        Ok(true)
    }
}

impl super::Iter for MergedIter {
    fn seek(&mut self, want: &Record) -> Result<()> {
        self.advance_index = -1;
        self.pq.clear();

        for i in 0..self.iters.len() {
            match self.iters[i].seek(want) {
                // This is for now how we indicate not finding the thing, we
                // should change it I think
                Err(Error::Iterator) => continue,
                Err(e) => return Err(e),
                Ok(_) => {}
            }

            self.advance_subiter(i)?;
        }

        Ok(())
    }

    fn next(&mut self, rec: &mut Record) -> Result<bool> {
        loop {
            match self.next_entry(rec) {
                Err(e) => return Err(e),
                Ok(false) => return Ok(false),
                Ok(true) => {
                    if self.suppress_deletions && rec.is_deletion() {
                        continue;
                    }

                    return Ok(true);
                }
            }
        }
    }
}

#[cfg(test)]
mod test {
    use super::super::Iter;
    use super::super::blocksource::BufferSource;
    use super::super::record::{Record, RefValue};
    use super::super::table::test::INITIAL_REF_FILE;
    use super::{BlockType, MergedIter, MergedTable};
    use gix_features::threading::OwnShared;

    #[test]
    fn test_initial_table() {
        let source = Box::new(BufferSource::from_slice(&INITIAL_REF_FILE[..]));
        let table = super::Table::new(source, "01-01-rand.ref".into()).expect("parsing");
        let hash_id = table.hash_id;

        let tables = vec![OwnShared::new(table)];
        let merged = MergedTable::new(tables, hash_id).expect("merged table creation");
        let mut iter = MergedIter::from_merged(&merged, BlockType::Ref);

        let mut rec = Record::for_search(BlockType::Ref, None);

        iter.seek(&rec).expect("finding the nil/first record");

        let res = iter.next(&mut rec).expect("iterate once");
        assert!(res);

        let head = match &rec {
            Record::Ref(rec) => rec,
            _ => panic!("record is not a ref"),
        };

        assert_eq!(b"HEAD", head.refname.as_slice());
        assert_eq!(Some(RefValue::Symref("refs/heads/main".into())), head.value);

        let res = iter.next(&mut rec).expect("no error on iterating to the end");
        assert!(!res);
    }
}
