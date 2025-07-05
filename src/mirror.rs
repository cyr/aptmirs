use std::{fmt::Display, path::Path, sync::Arc};

use async_trait::async_trait;
use debian_installer::DownloadDebianInstaller;
use diffs::DownloadFromDiffs;
use indicatif::HumanBytes;
use metadata::DownloadMetadata;
use packages::DownloadFromPackageIndices;
use release::DownloadRelease;
use thiserror::Error;
use tokio::{sync::Mutex, task::spawn_blocking};

use crate::{cmd::{CmdResult, CmdState}, config::MirrorOpts, context::Context, downloader::Downloader, error::MirsError, metadata::{metadata_file::MetadataFile, release::Release, repository::Repository, FilePath}, pgp::PgpKeyStore, step::Step, CliOpts};
use crate::error::Result;

pub mod release;
pub mod metadata;
pub mod diffs;
pub mod packages;
pub mod debian_installer;

pub type MirrorDynStep = Box<dyn Step<MirrorState, Result = MirrorResult>>;
pub type MirrorContext = Arc<Context<MirrorState>>;

#[derive(Error, Debug)]
pub enum MirrorResult {
    #[error("Ok: {} downloaded, {} packages/source files", HumanBytes(*.total_download_size), .num_packages_downloaded)]
    NewRelease { total_download_size: u64, num_packages_downloaded: u64 },
    #[error("Ok: release unchanged")]
    ReleaseUnchanged,
    #[error("Ok: new release, but changes do not apply to configured selections")]
    IrrelevantChanges,
    #[error("Ok: release unchanged, but attempted to download missing files")]
    ReleaseUnchangedButIncomplete,
    #[error("Fail: {0}")]
    Error(MirsError)
}

impl CmdResult for MirrorResult { }

#[derive(Default)]
pub struct MirrorState {
    pub repo: Arc<Repository>,
    pub opts: Arc<MirrorOpts>,
    pub downloader: Downloader,
    pub pgp_key_store: Arc<PgpKeyStore>,
    pub output: Arc<Mutex<MirrorOutput>>
}

#[derive(Default)]
pub struct MirrorOutput {
    pub release: Option<Release>,
    pub indices: Vec<MetadataFile>,
    pub delete_paths: Vec<FilePath>,
    pub total_bytes_downloaded: u64,
    pub total_packages_downloaded: u64,
    pub new_release: bool,
} 

impl MirrorOutput {
    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }

    pub fn take_metadata<F: Fn(&MetadataFile) -> bool>(&mut self, filter_func: F) -> Vec<MetadataFile> {
        let mut vec = Vec::new();

        for i in (0..self.indices.len()).rev() {
            if filter_func(&self.indices[i]) {
                let file = self.indices.swap_remove(i);

                vec.push(file);
            }
        }

        vec
    }
}

impl MirrorState {
    async fn move_metadata_into_root(&self) -> Result<MirrorResult> {
        let output = self.output.lock().await;

        let tmp_dir = self.repo.tmp_dir.clone();
        let root_dir = self.repo.root_dir.clone();

        for path in &output.delete_paths {
            if path.exists() {
                tokio::fs::remove_file(path).await?;
            }
        }

        spawn_blocking(move || {
            rebase_dir(tmp_dir.as_ref(), tmp_dir.as_ref(), root_dir.as_ref())?;
            
            std::fs::remove_dir_all(&tmp_dir)?;
            
            Ok::<(), MirsError>(())
        }).await??;

        Ok(MirrorResult::NewRelease { 
            total_download_size: output.total_bytes_downloaded,
            num_packages_downloaded: output.total_packages_downloaded
        })
    }
}

impl Display for MirrorState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.opts.fmt(f)
    }
}

#[async_trait]
impl CmdState for MirrorState {
    type Result = MirrorResult;

    async fn finalize(&self) -> Self::Result {
        let result = {
            let output = self.output.lock().await;
            
            MirrorResult::NewRelease {
                total_download_size: output.total_bytes_downloaded,
                num_packages_downloaded: output.total_packages_downloaded
            }
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
            MirrorResult::ReleaseUnchangedButIncomplete |
            MirrorResult::ReleaseUnchanged |
            MirrorResult::Error(..) => {
                _ = self.repo.delete_tmp();
            },
        }
            
        result
    }
}

impl Context<MirrorState> {
    fn create_steps(opts: &MirrorOpts) -> Vec<MirrorDynStep> {
        let mut steps: Vec<MirrorDynStep> = vec![
            Box::new(DownloadRelease),
            Box::new(DownloadMetadata),
            Box::new(DownloadFromDiffs),
            Box::new(DownloadFromPackageIndices),
        ];

        if opts.debian_installer() {
            steps.push(Box::new(DownloadDebianInstaller))
        }

        steps
    }

    pub fn create(opts: Vec<MirrorOpts>, cli_opts: Arc<CliOpts>, pgp_key_store: Arc<PgpKeyStore>) -> Result<Vec<(MirrorContext, Vec<MirrorDynStep>)>> {
        let downloader = Downloader::build(cli_opts.dl_threads);

        opts.into_iter()
            .map(|o| {
                let repo = Repository::build_with_tmp(&o, &cli_opts)?;

                let steps = Self::create_steps(&o);

                let progress = downloader.progress();

                let state = MirrorState {
                    repo,
                    opts: Arc::new(o),
                    downloader: downloader.clone(),
                    pgp_key_store: pgp_key_store.clone(),
                    ..Default::default()
                };

                Ok((Context::build(state, cli_opts.clone(), progress), steps))
            })
            .collect::<Result<Vec<(_, _)>>>()
    }
}

pub fn verify_and_prune(files: &mut Vec<MetadataFile>) {
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