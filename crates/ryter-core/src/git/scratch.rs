//! Operation-owned Git scratch files, never shared with another session.

use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

pub(crate) struct Scratch(PathBuf);

impl Scratch {
    pub(crate) fn new(repo: &Path) -> Result<Self> {
        let git_dir = super::git(repo, &["rev-parse", "--absolute-git-dir"])?;
        let path = Path::new(git_dir.trim()).join(format!("ryter-tmp-{}", uuid::Uuid::now_v7()));
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder
            .create(&path)
            .map_err(|e| Error::Io(e.to_string()))?;
        Ok(Self(path))
    }

    pub(crate) fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
