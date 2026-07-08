use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    rc::Rc,
};

use crate::{Namespace, store::WriteReflog};

use super::{Result, stack};

/// A store for references persisted in git's reftable format.
#[derive(Clone)]
pub struct Store {
    git_dir: PathBuf,
    reftable_dir: PathBuf,

    /// The kind of hash for the repository
    object_hash: gix_hash::Kind,

    /// The way to handle reflog edits.
    pub write_reflog: WriteReflog,
    /// The namespace to use for reads and edits.
    pub namespace: Option<Namespace>,
    /// A cached stack that we should be able to refresh and update as necessary
    stack: Option<Rc<RefCell<stack::Stack>>>,
}

impl Store {
    /// Create a reftable-backed store rooted at `git_dir/reftable`.
    pub fn at(git_dir: PathBuf, opts: crate::store::init::Options) -> Self {
        let crate::store::init::Options {
            write_reflog,
            object_hash,
            ..
        } = opts;
        let reftable_dir = git_dir.join("reftable");
        Self {
            git_dir,
            reftable_dir,
            object_hash,
            write_reflog,
            namespace: None,
            stack: None,
        }
    }

    /// Return the `.git` directory at which this store is rooted.
    pub fn git_dir(&self) -> &Path {
        &self.git_dir
    }

    /// Return the directory holding the reftable stack files.
    pub fn reftable_dir(&self) -> &Path {
        &self.reftable_dir
    }
}
