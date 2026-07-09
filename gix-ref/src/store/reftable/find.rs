use super::{
    BlockType, Iter, Store,
    record::{Record, RefRecord, RefValue},
};

use gix_features::threading::{Mutable, OwnShared, get_ref, lock};

impl Store {
    /// Find a reference in the reftable stack.
    pub fn find<'a, Name, E>(&self, partial: Name) -> Result<crate::Reference, crate::find::existing::Error>
    where
        Name: TryInto<&'a crate::PartialNameRef, Error = E>,
        crate::name::Error: From<E>,
    {
        let name = partial
            .try_into()
            .map_err(|err| crate::find::existing::Error::Find(crate::find::Error::RefnameValidation(err.into())))?;
        self.assure_stack_uptodate().map_err(|err| {
            crate::find::existing::Error::Find(crate::find::Error::ReadFileContents {
                source: std::io::Error::other(err.to_string()),
                path: self.reftable_dir().to_owned(),
            })
        })?;

        let stack_borrow = lock(&self.stack);
        let stack = stack_borrow.as_ref().expect("stack must be loaded");
        let mut iter = stack.iter_refs();
        let key = name.as_bstr().to_vec();
        let mut rec = Record::for_search(BlockType::Ref, Some(key));

        // FIXME: this is a nonsense error so we don't have to redefine everything
        iter.seek(&rec)
            .map_err(|_| crate::find::Error::PackedOpen(crate::open::Error::HeaderParsing))?;
        let found = iter
            .next(&mut rec)
            .map_err(|_| crate::find::Error::PackedOpen(crate::open::Error::HeaderParsing))?;

        if !found {
            return Err(crate::find::existing::Error::NotFound {
                name: name.to_partial_path().into(),
            });
        }

        let (refname, value) = match rec {
            Record::Ref(RefRecord { refname, value, .. }) => (refname, value),
            _ => unreachable!(),
        };

        let (target, peeled) = match value {
            Some(RefValue::Val1(id)) => (crate::Target::Object(id), None),
            Some(RefValue::Val2(target, peeled)) => (crate::Target::Object(target), Some(peeled)),
            Some(RefValue::Symref(target)) => (crate::Target::Symbolic(crate::FullName(target.into())), None),
            None => unreachable!(),
        };

        Ok(crate::Reference {
            name: crate::FullName(refname.into()),
            target,
            peeled,
        })
    }

    /// Try to find a reference in the reftable stack.
    pub fn try_find<'a, Name, E>(&self, partial: Name) -> Result<Option<crate::Reference>, crate::find::Error>
    where
        Name: TryInto<&'a crate::PartialNameRef, Error = E>,
        crate::name::Error: From<E>,
    {
        match self.find(partial) {
            Ok(r) => Ok(Some(r)),
            Err(crate::find::existing::Error::NotFound { .. }) => Ok(None),
            Err(e) => Err(crate::find::Error::PackedRef(crate::packed::find::Error::Parse)),
        }
    }

    /// Return pristine status for the reftable stack.
    pub fn is_pristine(&self, default_ref: &crate::FullNameRef) -> Option<bool> {
        let _ = default_ref;
        self.assure_stack_uptodate().unwrap_or_else(|err| {
            panic!(
                "BUG: could not initialize reftable stack at {:?}: {err:?}",
                self.reftable_dir()
            )
        });
        panic!("BUG: native reftable is_pristine() is not implemented yet")
    }

    /// Start a transaction in the reftable stack.
    pub fn transaction(&self) -> crate::file::Transaction<'_, '_> {
        self.assure_stack_uptodate().unwrap_or_else(|err| {
            panic!(
                "BUG: could not initialize reftable stack at {:?}: {err:?}",
                self.reftable_dir()
            )
        });
        panic!("BUG: native reftable transaction() is not implemented yet")
    }
}
