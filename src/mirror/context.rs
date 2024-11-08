use std::{path::Path, sync::Arc};

use tokio::{sync::Mutex, task::spawn_blocking};

use crate::{config::MirrorOpts, error::MirsError, metadata::{release::Release, FilePath}, pgp::PgpKeyStore, CliOpts};
use crate::error::Result;

use super::{downloader::Downloader, progress::Progress, repository::Repository, step::{debian_installer::DownloadDebianInstaller, diffs::DownloadFromDiffs, metadata::DownloadMetadata, packages::DownloadFromPackageIndices, release::DownloadRelease, Step}, MirrorResult};

#[derive(Clone)]
pub struct Context {
    pub repository: Arc<Repository>,
    pub downloader: Downloader,
    pub progress: Progress,
    pub pgp_key_store: Arc<PgpKeyStore>,
    pub mirror_opts: Arc<MirrorOpts>,
    pub cli_opts: Arc<CliOpts>,
    pub output: Arc<Mutex<StepOutput>>,
}

impl Context {
    pub fn build(mirror_opts: MirrorOpts, cli_opts: Arc<CliOpts>, downloader: Downloader, pgp_key_store: Arc<PgpKeyStore>) -> Result<Arc<Self>> {
        let repository = Repository::build(&mirror_opts, &cli_opts)?;

        let progress = downloader.progress();
    
        Ok(Arc::new(Context {
            repository,
            downloader,
            progress,
            pgp_key_store,
            mirror_opts: Arc::new(mirror_opts),
            cli_opts,
            output: Arc::new(Mutex::new(Default::default())),
        }))
    }

    pub async fn next_step(&self, step_name: &str) {
        self.progress.next_step(step_name).await;
    }

    pub fn create_steps(&self) -> Vec<Box<dyn Step>> {
        let mut vec: Vec<Box<dyn Step>> = vec![
            Box::new(DownloadRelease),
            Box::new(DownloadMetadata),
            Box::new(DownloadFromDiffs),
            Box::new(DownloadFromPackageIndices),
        ];

        if self.mirror_opts.debian_installer() {
            vec.push(Box::new(DownloadDebianInstaller))
        }

        self.progress.reset();
        self.progress.set_total_steps(vec.len() as u8);

        vec
    }

    pub async fn finalize(&self) -> Result<MirrorResult> {
        let tmp_dir = self.repository.tmp_dir.clone();
        let root_dir = self.repository.root_dir.clone();

        let output = self.output.lock().await;

        for path in &output.delete_paths {
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
            total_download_size: output.total_bytes_downloaded,
            num_packages_downloaded: output.total_packages_downloaded
        })
    }
}

#[derive(Default)]
pub struct StepOutput {
    pub release: Option<Release>,
    pub package_indices: Vec<FilePath>,
    pub diff_indices: Vec<FilePath>,
    pub di_sumfiles: Vec<FilePath>,
    pub delete_paths: Vec<FilePath>,
    pub total_bytes_downloaded: u64,
    pub total_packages_downloaded: u64,
}

impl StepOutput {
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