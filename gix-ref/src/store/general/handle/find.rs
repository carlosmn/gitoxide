use crate::{PartialNameRef, Reference, store};

mod error {
    use std::convert::Infallible;
    use std::path::PathBuf;

    /// The error returned by [`crate::Store::find()`] and [`crate::Store::try_find()`].
    #[derive(Debug, thiserror::Error)]
    #[allow(missing_docs)]
    pub enum Error {
        #[error("The ref name or path is not a valid ref name")]
        RefnameValidation(#[from] crate::name::Error),
        #[error("The ref file {path:?} could not be read in full")]
        ReadFileContents { source: std::io::Error, path: PathBuf },
        #[error("The reference at \"{relative_path}\" could not be instantiated")]
        ReferenceCreation { relative_path: PathBuf },
        #[error("A packed ref lookup failed")]
        PackedRef(#[from] crate::packed::find::Error),
        #[error("Could not open the packed refs buffer when trying to find references")]
        PackedOpen(#[from] crate::open::Error),
    }

    impl From<crate::file::find::Error> for Error {
        fn from(value: crate::file::find::Error) -> Self {
            match value {
                crate::file::find::Error::RefnameValidation(err) => Self::RefnameValidation(err),
                crate::file::find::Error::ReadFileContents { source, path } => Self::ReadFileContents { source, path },
                crate::file::find::Error::ReferenceCreation {
                    source: _,
                    relative_path,
                } => Self::ReferenceCreation { relative_path },
                crate::file::find::Error::PackedRef(err) => Self::PackedRef(err),
                crate::file::find::Error::PackedOpen(err) => Self::PackedOpen(err),
            }
        }
    }

    impl From<Infallible> for Error {
        fn from(_: Infallible) -> Self {
            unreachable!("this impl is needed to allow passing a known valid partial path as parameter")
        }
    }
}

pub use error::Error;

use crate::store::handle;

impl store::Handle {
    /// TODO: actually implement this with handling of the packed buffer.
    pub fn try_find<'a, Name, E>(&self, partial: Name) -> Result<Option<Reference>, Error>
    where
        Name: TryInto<&'a PartialNameRef, Error = E>,
        Error: From<E>,
    {
        let _name = partial.try_into()?;
        match &self.state {
            handle::State::Loose { store: _, .. } => {
                todo!()
            }
            #[cfg(feature = "reftable")]
            handle::State::Reftable { store: _, .. } => {
                todo!()
            }
        }
    }
}

/// Errors for [`crate::Store::find()`] where absence is considered an error.
pub mod existing {
    mod error {
        use std::path::PathBuf;

        /// The error returned by [file::Store::find_existing()][crate::file::Store::find_existing()].
        #[derive(Debug, thiserror::Error)]
        #[allow(missing_docs)]
        pub enum Error {
            #[error("An error occurred while finding a reference in the database")]
            Find(#[from] crate::find::Error),
            #[error("The ref partially named {name:?} could not be found")]
            NotFound { name: PathBuf },
        }

        impl From<crate::file::find::existing::Error> for Error {
            fn from(value: crate::file::find::existing::Error) -> Self {
                match value {
                    crate::file::find::existing::Error::Find(err) => Self::Find(err.into()),
                    crate::file::find::existing::Error::NotFound { name } => Self::NotFound { name },
                }
            }
        }
    }

    pub use error::Error;

    use crate::{PartialNameRef, Reference, store};

    impl store::Handle {
        /// Similar to [`crate::file::Store::find()`] but a non-existing ref is treated as error.
        pub fn find<'a, Name, E>(&self, _partial: Name) -> Result<Reference, Error>
        where
            Name: TryInto<&'a PartialNameRef, Error = E>,
            crate::name::Error: From<E>,
        {
            todo!()
            // match self.try_find(partial) {}
            // match self.find_one_with_verified_input(path.to_partial_path().as_ref(), packed) {
            //     Ok(Some(r)) => Ok(r),
            //     Ok(None) => Err(Error::NotFound(path.to_partial_path().into_owned())),
            //     Err(err) => Err(err.into()),
            // }
        }
    }
}
