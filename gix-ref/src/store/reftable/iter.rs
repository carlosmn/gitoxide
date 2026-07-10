use std::io;

use gix_object::bstr::{BString, ByteSlice};
use gix_path::RelativePath;

use super::{
    BlockType, Iter, Store,
    record::{Record, RefRecord, RefValue},
};

/// Iterator platform for reftable-backed references.
#[must_use = "Iterators should be obtained from this platform"]
pub struct Platform<'s> {
    store: &'s Store,
}

/// Iterator over references in a reftable-backed store.
pub struct References {
    iter: super::merged::MergedIter,
    rec: Record,
    filter: Filter,
}

enum Filter {
    All,
    Prefixed(BString),
    Pseudo,
}

impl Store {
    /// Return a platform to obtain streaming iterators over this reftable store.
    pub fn iter(&self) -> Result<Platform<'_>, crate::open::Error> {
        self.assure_stack_uptodate()
            .map_err(|_err| crate::open::Error::HeaderParsing)?;
        Ok(Platform { store: self })
    }

    fn references_with_filter(&self, filter: Filter) -> io::Result<References> {
        self.assure_stack_uptodate()
            .map_err(|err| io::Error::other(err.to_string()))?;

        let mut stack_slot = gix_features::threading::lock(&self.stack);
        let stack = stack_slot
            .as_mut()
            .expect("BUG: stack should be loaded after assure_stack_uptodate()");

        let mut iter = stack.iter_refs();
        let rec = match &filter {
            Filter::Prefixed(prefix) => Record::for_search(BlockType::Ref, Some(prefix.to_vec())),
            _ => Record::for_search(BlockType::Ref, None),
        };
        iter.seek(&rec).map_err(|err| io::Error::other(err.to_string()))?;

        Ok(References { iter, rec, filter })
    }
}

impl Platform<'_> {
    /// Return an iterator over all references.
    pub fn all(&self) -> io::Result<References> {
        self.store.references_with_filter(Filter::All)
    }

    /// Return an iterator over references whose names match `prefix`.
    pub fn prefixed(&self, prefix: &RelativePath) -> io::Result<References> {
        self.store
            .references_with_filter(Filter::Prefixed(BString::from(prefix.as_ref())))
    }

    /// Return an iterator over pseudo references.
    pub fn pseudo(&self) -> io::Result<References> {
        self.store.references_with_filter(Filter::Pseudo)
    }
}

impl Iterator for References {
    type Item = io::Result<crate::Reference>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match self.iter.next(&mut self.rec) {
                Ok(true) => {}
                Ok(false) => return None,
                Err(err) => return Some(Err(io::Error::other(err.to_string()))),
            }

            let Record::Ref(RefRecord { refname, value, .. }) = &self.rec else {
                continue;
            };
            let Some(value) = value.as_ref() else {
                continue;
            };

            let matches = match &self.filter {
                Filter::All => !crate::name::is_pseudo_ref(refname.as_bstr()),
                Filter::Prefixed(prefix) => {
                    if refname.starts_with(prefix.as_slice()) {
                        true
                    } else {
                        return None;
                    }
                }
                Filter::Pseudo => crate::name::is_pseudo_ref(refname.as_bstr()),
            };
            if !matches {
                continue;
            }

            let (target, peeled) = match value {
                RefValue::Val1(id) => (crate::Target::Object(*id), None),
                RefValue::Val2(target, peeled) => (crate::Target::Object(*target), Some(*peeled)),
                RefValue::Symref(target) => (crate::Target::Symbolic(crate::FullName(target.clone().into())), None),
            };

            return Some(Ok(crate::Reference {
                name: crate::FullName(refname.clone().into()),
                target,
                peeled,
            }));
        }
    }
}
