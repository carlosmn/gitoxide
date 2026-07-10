use std::collections::BTreeSet;

use gix_hash::ObjectId;

use crate::{
    Target, peel,
    raw::Reference,
    store_impl::{file, file::log},
};

fn file_store(store: &crate::Store) -> &file::Store {
    match &store.inner {
        crate::store::State::Loose { store } => store,
        #[cfg(feature = "reftable")]
        crate::store::State::Reftable { .. } => {
            panic!("reference operations currently require a file-backed reference store")
        }
    }
}

pub trait Sealed {}
impl Sealed for crate::Reference {}

/// A trait to extend [Reference][crate::Reference] with functionality requiring a [file::Store].
pub trait ReferenceExt: Sealed {
    /// A step towards obtaining forward or reverse iterators on reference logs.
    fn log_iter<'a, 's>(&'a self, store: &'s crate::Store) -> log::iter::Platform<'a, 's>;

    /// For details, see [`Reference::log_exists()`].
    fn log_exists(&self, store: &crate::Store) -> bool;

    /// Follow all symbolic targets this reference might point to and peel the underlying object
    /// to the end of the tag-chain, returning the first non-tag object the annotated tag points to,
    /// using `objects` to access them and `store` to lookup symbolic references.
    ///
    /// This is useful to learn where this reference is ultimately pointing to after following all symbolic
    /// refs and all annotated tags to the first non-tag object.
    #[deprecated = "Use `peel_to_id()` instead"]
    fn peel_to_id_in_place(
        &mut self,
        store: &crate::Store,
        objects: &dyn gix_object::Find,
    ) -> Result<ObjectId, peel::to_id::Error>;

    /// Follow all symbolic targets this reference might point to and peel the underlying object
    /// to the end of the tag-chain, returning the first non-tag object the annotated tag points to,
    /// using `objects` to access them and `store` to lookup symbolic references.
    ///
    /// This is useful to learn where this reference is ultimately pointing to after following all symbolic
    /// refs and all annotated tags to the first non-tag object.
    ///
    /// Note that this method mutates `self` in place if it does not already point to a
    /// non-symbolic object.
    fn peel_to_id(
        &mut self,
        store: &crate::Store,
        objects: &dyn gix_object::Find,
    ) -> Result<ObjectId, peel::to_id::Error>;

    /// Like [`ReferenceExt::follow()`], but follows all symbolic references while gracefully handling loops,
    /// altering this instance in place.
    fn follow_to_object(&mut self, store: &crate::Store) -> Result<ObjectId, peel::to_object::Error>;

    /// Follow this symbolic reference one level and return the ref it refers to.
    ///
    /// Returns `None` if this is not a symbolic reference, hence the leaf of the chain.
    fn follow(&self, store: &crate::Store) -> Option<Result<Reference, crate::find::existing::Error>>;
}

impl ReferenceExt for Reference {
    fn log_iter<'a, 's>(&'a self, store: &'s crate::Store) -> log::iter::Platform<'a, 's> {
        let store = file_store(store);
        log::iter::Platform {
            store,
            name: self.name.as_ref(),
            buf: Vec::new(),
        }
    }

    fn log_exists(&self, store: &crate::Store) -> bool {
        let store = file_store(store);
        store
            .reflog_exists(self.name.as_ref())
            .expect("infallible name conversion")
    }

    fn peel_to_id_in_place(
        &mut self,
        store: &crate::Store,
        objects: &dyn gix_object::Find,
    ) -> Result<ObjectId, peel::to_id::Error> {
        self.peel_to_id(store, objects)
    }

    fn peel_to_id(
        &mut self,
        store: &crate::Store,
        objects: &dyn gix_object::Find,
    ) -> Result<ObjectId, peel::to_id::Error> {
        match self.peeled {
            Some(peeled) => {
                self.target = Target::Object(peeled.to_owned());
                Ok(peeled)
            }
            None => {
                let mut oid = self.follow_to_object(store)?;
                let mut buf = Vec::new();
                let peeled_id = loop {
                    let gix_object::Data {
                        kind,
                        data,
                        object_hash: hash_kind,
                    } = objects
                        .try_find(&oid, &mut buf)?
                        .ok_or_else(|| peel::to_id::Error::NotFound {
                            oid,
                            name: self.name.0.clone(),
                        })?;
                    match kind {
                        gix_object::Kind::Tag => {
                            oid = gix_object::TagRefIter::from_bytes(data, hash_kind)
                                .target_id()
                                .map_err(|_err| peel::to_id::Error::NotFound {
                                    oid,
                                    name: self.name.0.clone(),
                                })?;
                        }
                        _ => break oid,
                    }
                };
                self.peeled = Some(peeled_id);
                self.target = Target::Object(peeled_id);
                Ok(peeled_id)
            }
        }
    }

    fn follow_to_object(&mut self, store: &crate::Store) -> Result<ObjectId, peel::to_object::Error> {
        match self.target {
            Target::Object(id) => Ok(id),
            Target::Symbolic(_) => {
                let mut seen = BTreeSet::new();
                let cursor = &mut *self;
                while let Some(next) = store.follow_reference(cursor) {
                    let next = next?;
                    if seen.contains(&next.name) {
                        return Err(peel::to_object::Error::Cycle {
                            start_absolute: store.git_dir().join(cursor.name.to_path()),
                        });
                    }
                    *cursor = next;
                    seen.insert(cursor.name.clone());
                    const MAX_REF_DEPTH: usize = 5;
                    if seen.len() == MAX_REF_DEPTH {
                        return Err(peel::to_object::Error::DepthLimitExceeded {
                            max_depth: MAX_REF_DEPTH,
                        });
                    }
                }
                let oid = self.target.try_id().expect("peeled ref").to_owned();
                Ok(oid)
            }
        }
    }

    fn follow(&self, store: &crate::Store) -> Option<Result<Reference, crate::find::existing::Error>> {
        store.follow_reference(self)
    }
}
