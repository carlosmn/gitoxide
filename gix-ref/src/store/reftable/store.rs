use std::path::{Path, PathBuf};

use gix_features::threading::{Mutable, OwnShared, lock};

use crate::{Namespace, store::WriteReflog};

use super::stack;

/// A store for references persisted in git's reftable format.
#[derive(Clone)]
pub struct Store {
    git_dir: PathBuf,
    reftable_dir: PathBuf,

    /// The kind of hash for the repository
    pub(super) object_hash: gix_hash::Kind,

    /// The way to handle reflog edits.
    pub write_reflog: WriteReflog,
    /// The namespace to use for reads and edits.
    pub namespace: Option<Namespace>,
    /// A cached stack that we should be able to refresh and update as necessary
    pub(super) stack: OwnShared<Mutable<stack::Stack>>,
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
        let stack_options = Some(stack::Options::from_init_options(crate::store::init::Options {
            object_hash,
            ..Default::default()
        }));
        Self {
            git_dir,
            reftable_dir: reftable_dir.clone(),
            object_hash,
            write_reflog,
            namespace: None,
            stack: OwnShared::new(Mutable::new(stack::Stack::empty(
                reftable_dir,
                stack_options,
            ))),
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

    pub(super) fn assure_stack_uptodate(&self) -> super::Result<()> {
        let mut stack_slot = lock(&self.stack);
        stack_slot.reload()
    }
}
