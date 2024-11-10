use std::{fmt::Display, path::Path, sync::Arc};

use async_trait::async_trait;
use indicatif::HumanBytes;
use thiserror::Error;
use tokio::task::spawn_blocking;

use crate::{cmd::{CmdResult, CmdState}, config::MirrorOpts, downloader::Downloader, error::MirsError, metadata::{release::Release, repository::Repository, FilePath}, pgp::PgpKeyStore};
use crate::error::Result;

pub mod release;
pub mod metadata;
pub mod diffs;
pub mod packages;
pub mod debian_installer;

#[derive(Error, Debug)]
pub enum MirrorResult {
    #[error("Ok: {} downloaded, {} packages/source files", HumanBytes(*.total_download_size), .num_packages_downloaded)]
    NewRelease { total_download_size: u64, num_packages_downloaded: u64 },
    #[error("Ok: release unchanged")]
    ReleaseUnchanged,
    #[error("Ok: new release, but changes do not apply to configured selections")]
    IrrelevantChanges,
    #[error("Fail: {0}")]
    Error(MirsError)
}

impl CmdResult for MirrorResult { }

#[derive(Default)]
pub struct MirrorState {
    pub release: Option<Release>,
    pub package_indices: Vec<FilePath>,
    pub diff_indices: Vec<FilePath>,
    pub di_sumfiles: Vec<FilePath>,
    pub delete_paths: Vec<FilePath>,
    pub total_bytes_downloaded: u64,
    pub total_packages_downloaded: u64,
    pub repo: Arc<Repository>,
    pub opts: Arc<MirrorOpts>,
    pub downloader: Downloader,
    pub pgp_key_store: Arc<PgpKeyStore>,
}

impl MirrorState {
    pub fn is_empty(&self) -> bool {
        self.package_indices.is_empty() &&
            self.diff_indices.is_empty() &&
            self.di_sumfiles.is_empty()
    }
    
    pub fn verify_and_prune(&mut self) {
        verify_and_prune(&mut self.package_indices);
        verify_and_prune(&mut self.diff_indices);
        verify_and_prune(&mut self.di_sumfiles);
    }
    
    async fn move_metadata_into_root(&self) -> Result<MirrorResult> {
        let tmp_dir = self.repo.tmp_dir.clone();
        let root_dir = self.repo.root_dir.clone();

        for path in &self.delete_paths {
            if tokio::fs::try_exists(path).await? {
                tokio::fs::remove_dir_all(path).await?;
            }
        }

        spawn_blocking(move || {
            rebase_dir(tmp_dir.as_ref(), tmp_dir.as_ref(), root_dir.as_ref())?;
            
            std::fs::remove_dir_all(&tmp_dir)?;
            
            Ok::<(), MirsError>(())
        }).await??;

        Ok(MirrorResult::NewRelease { 
            total_download_size: self.total_bytes_downloaded,
            num_packages_downloaded: self.total_packages_downloaded
        })
    }
}

impl Display for MirrorState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.opts.packages && self.opts.source {
            f.write_str("deb+deb-src")?
        } else if self.opts.packages {
            f.write_str("deb")?
        } else if self.opts.source {
            f.write_str("deb-src")?
        }

        f.write_fmt(format_args!(
            " {} {}[{}] {}",
            self.opts.url,
            self.opts.suite,
            self.opts.arch.join(", "),
            self.opts.components.join(" ")
        ))
    }
}

#[async_trait]
impl CmdState for MirrorState {
    type Result = MirrorResult;

    async fn finalize(&self) -> Self::Result {
        let result = MirrorResult::NewRelease {
            total_download_size: self.total_bytes_downloaded,
            num_packages_downloaded: self.total_packages_downloaded
        };

        self.finalize_with_result(result).await
    }

    async fn finalize_with_result(&self, result: Self::Result) -> Self::Result {
        match &result {
            MirrorResult::NewRelease { .. } |
            MirrorResult::IrrelevantChanges => {
                if let Err(e) = self.move_metadata_into_root().await {
                    return MirrorResult::Error(MirsError::Finalize { inner: Box::new(e) })
                }
            },
            MirrorResult::ReleaseUnchanged |
            MirrorResult::Error(..) => {
                _ = self.repo.delete_tmp();
            },
        }
            
        result
    }
}

fn verify_and_prune(files: &mut Vec<FilePath>) {
    let mut pos = 0;
    loop {
        if pos >= files.len() {
            break
        }

        if !files[pos].exists() {
            files.swap_remove(pos);
        } else {
            pos += 1;
        }
    }
}

fn rebase_dir(dir: &Path, from: &Path, to: &Path) -> Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            rebase_dir(&path, from, to)?;
        } else {
            let rel_path = path.strip_prefix(from)
                .expect("implemention error; path should be in tmp");

            let new_path = to.join(rel_path);

            let parent = new_path.parent().unwrap();
            if !parent.exists() {
                std::fs::create_dir_all(parent)?;
            }
            
            if std::fs::rename(&path, &new_path).is_err() {
                std::fs::copy(&path, &new_path)?;
            }
        }
    }

    Ok(())
}
