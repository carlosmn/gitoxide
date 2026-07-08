use gix_path::RelativePath;

/// A backend-agnostic platform to create reference iterators.
#[must_use = "Iterators should be obtained from this platform"]
pub struct Platform<'s> {
    state: State<'s>,
}

enum State<'s> {
    File(crate::file::iter::Platform<'s>),
    #[cfg(feature = "reftable")]
    Reftable {
        store: &'s crate::reftable::Store,
    },
}

/// An iterator over references, independent of the underlying ref storage backend.
pub struct References<'p, 's> {
    state: IterState<'p, 's>,
}

enum IterState<'p, 's> {
    File(crate::file::iter::LooseThenPacked<'p, 's>),
    #[cfg(feature = "reftable")]
    Reftable {
        _store: &'s crate::reftable::Store,
        _marker: std::marker::PhantomData<&'p ()>,
    },
}

/// The error returned while iterating references.
#[derive(Debug, thiserror::Error)]
#[allow(missing_docs)]
pub enum Error {
    #[error(transparent)]
    Backend(#[from] crate::file::iter::loose_then_packed::Error),
}

impl<'s> Platform<'s> {
    pub(crate) fn from_file(platform: crate::file::iter::Platform<'s>) -> Self {
        Self {
            state: State::File(platform),
        }
    }

    #[cfg(feature = "reftable")]
    pub(crate) fn from_reftable(store: &'s crate::reftable::Store) -> Self {
        Self {
            state: State::Reftable { store },
        }
    }

    /// Return an iterator over all references.
    pub fn all<'p>(&'p self) -> std::io::Result<References<'p, 's>> {
        match &self.state {
            State::File(platform) => Ok(References {
                state: IterState::File(platform.all()?),
            }),
            #[cfg(feature = "reftable")]
            State::Reftable { store } => Ok(References {
                state: IterState::Reftable {
                    _store: store,
                    _marker: std::marker::PhantomData,
                },
            }),
        }
    }

    /// Return an iterator over references with the given path `prefix`.
    pub fn prefixed<'p>(&'p self, prefix: &RelativePath) -> std::io::Result<References<'p, 's>> {
        match &self.state {
            State::File(platform) => Ok(References {
                state: IterState::File(platform.prefixed(prefix)?),
            }),
            #[cfg(feature = "reftable")]
            State::Reftable { store } => {
                let _ = (store, prefix);
                todo!("prefixed() for reftable stores")
            }
        }
    }

    /// Return an iterator over pseudo references like `HEAD` or `FETCH_HEAD`.
    pub fn pseudo<'p>(&'p self) -> std::io::Result<References<'p, 's>> {
        match &self.state {
            State::File(platform) => Ok(References {
                state: IterState::File(platform.pseudo()?),
            }),
            #[cfg(feature = "reftable")]
            State::Reftable { store } => {
                let _ = store;
                todo!("pseudo() for reftable stores")
            }
        }
    }
}

impl Iterator for References<'_, '_> {
    type Item = Result<crate::Reference, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        match &mut self.state {
            IterState::File(iter) => iter.next().map(|res| res.map_err(Into::into)),
            #[cfg(feature = "reftable")]
            IterState::Reftable { .. } => {
                todo!("reference iteration for reftable stores")
            }
        }
    }
}
