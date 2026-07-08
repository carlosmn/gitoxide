#![allow(dead_code)]
use crate::{Namespace, store};
use std::path::Path;

impl crate::Store {
    /// Return the backing file store if this store is file-based.
    ///
    /// This is necessary for now as a lot of the code assumes that it's dealing
    /// with the files storage.
    pub fn as_file(&self) -> Option<&crate::file::Store> {
        match &self.inner {
            store::State::Loose { store } => Some(store),
            #[cfg(feature = "reftable")]
            store::State::Reftable { .. } => None,
        }
    }

    /// Return the git directory this store is rooted at.
    pub fn git_dir(&self) -> &Path {
        match &self.inner {
            store::State::Loose { store } => store.git_dir(),
            #[cfg(feature = "reftable")]
            store::State::Reftable { store } => store.git_dir(),
        }
    }

    /// Return if the reference store is pristine relative to `default_ref`.
    pub fn is_pristine(&self, default_ref: &crate::FullNameRef) -> Option<bool> {
        match &self.inner {
            store::State::Loose { store } => store.is_pristine(default_ref),
            #[cfg(feature = "reftable")]
            store::State::Reftable { .. } => {
                todo!("is_pristine for reftable stores")
            }
        }
    }

    /// Return the currently configured namespace.
    pub fn namespace(&self) -> Option<&Namespace> {
        match &self.inner {
            store::State::Loose { store } => store.namespace.as_ref(),
            #[cfg(feature = "reftable")]
            store::State::Reftable { store } => store.namespace.as_ref(),
        }
    }

    /// Set the namespace and return the previous one.
    pub fn set_namespace(&mut self, namespace: Option<Namespace>) -> Option<Namespace> {
        match &mut self.inner {
            store::State::Loose { store } => std::mem::replace(&mut store.namespace, namespace),
            #[cfg(feature = "reftable")]
            store::State::Reftable { store } => std::mem::replace(&mut store.namespace, namespace),
        }
    }

    /// Return the current reflog write mode.
    pub fn write_reflog(&self) -> crate::store::WriteReflog {
        match &self.inner {
            store::State::Loose { store } => store.write_reflog,
            #[cfg(feature = "reftable")]
            store::State::Reftable { store } => store.write_reflog,
        }
    }

    /// Set the reflog write mode.
    pub fn set_write_reflog(&mut self, mode: crate::store::WriteReflog) {
        match &mut self.inner {
            store::State::Loose { store } => store.write_reflog = mode,
            #[cfg(feature = "reftable")]
            store::State::Reftable { store } => store.write_reflog = mode,
        }
    }

    /// Find a reference by `partial` name.
    pub fn find<'a, Name, E>(&self, partial: Name) -> Result<crate::Reference, crate::find::existing::Error>
    where
        Name: TryInto<&'a crate::PartialNameRef, Error = E>,
        crate::name::Error: From<E>,
    {
        match &self.inner {
            store::State::Loose { store } => store.find(partial).map_err(Into::into),
            #[cfg(feature = "reftable")]
            store::State::Reftable { .. } => {
                todo!("find for reftable stores")
            }
        }
    }

    /// Try to find a reference by `partial` name.
    pub fn try_find<'a, Name, E>(&self, partial: Name) -> Result<Option<crate::Reference>, crate::find::Error>
    where
        Name: TryInto<&'a crate::PartialNameRef, Error = E>,
        crate::find::Error: From<E>,
    {
        let partial = partial.try_into()?;
        match &self.inner {
            store::State::Loose { store } => store.try_find(partial).map_err(Into::into),
            #[cfg(feature = "reftable")]
            store::State::Reftable { .. } => {
                todo!("try_find for reftable stores")
            }
        }
    }

    /// Return an iterator platform for references.
    pub fn iter(&self) -> Result<crate::file::iter::Platform<'_>, crate::packed::buffer::open::Error> {
        match &self.inner {
            store::State::Loose { store } => store.iter(),
            #[cfg(feature = "reftable")]
            store::State::Reftable { .. } => {
                todo!("iter for reftable stores")
            }
        }
    }

    /// Start a references transaction.
    pub fn transaction(&self) -> crate::file::Transaction<'_, '_> {
        match &self.inner {
            store::State::Loose { store } => store.transaction(),
            #[cfg(feature = "reftable")]
            store::State::Reftable { .. } => {
                todo!("transaction for reftable stores")
            }
        }
    }
}
