use std::{fs::File, sync::Arc};

use async_trait::async_trait;
use compact_str::format_compact;

use crate::{
    context::Context,
    downloader::{Download, time_from_atomic},
    error::{MirsError, Result},
    metadata::{
        FilePath,
        checksum::Checksum,
        release::Release,
        repository::{INRELEASE_FILE_NAME, RELEASE_FILE_NAME, RELEASE_GPG_FILE_NAME},
    },
    mirror::MirrorResult,
    pgp::KeyStore,
    progress::Progress,
    step::{Step, StepResult},
};

use super::MirrorState;

pub struct DownloadRelease;

#[async_trait]
impl Step<MirrorState> for DownloadRelease {
    type Result = MirrorResult;

    fn step_name(&self) -> &'static str {
        "Downloading release"
    }

    fn error(&self, e: MirsError) -> Self::Result {
        MirrorResult::Error(MirsError::DownloadRelease { inner: Box::new(e) })
    }

    async fn execute(&self, ctx: Arc<Context<MirrorState>>) -> Result<StepResult<Self::Result>> {
        let mut output = ctx.state.output.lock().await;

        let progress_bar = ctx.progress.create_download_progress_bar().await;

        let mut files = Vec::with_capacity(3);

        ctx.progress.files.inc_total(3);

        for file_url in ctx.state.repo.release_urls() {
            let destination = ctx.state.repo.to_path_in_tmp(&file_url);

            let dl = Box::new(Download {
                primary_target_path: destination.clone(),
                url: file_url,
                checksum: None,
                size: None,
                symlink_paths: Vec::new(),
                always_download: true,
            });

            ctx.state.downloader.download(dl).await;

            ctx.progress.update_for_files(&progress_bar);

            files.push(destination);
        }

        progress_bar.finish_using_style();

        let new_release = ReleaseFile::try_from(files.as_ref())?;

        if ctx.state.opts.pgp_verify {
            if ctx.state.repo.has_specified_pgp_key() {
                ctx.state.repo.verify(&new_release)?;
            } else {
                ctx.state.pgp_key_store.verify(&new_release)?;
            }
        }

        let local_release = ctx.state.repo.tmp_to_root(new_release.release());

        if local_release.exists() {
            let local_checksum = Checksum::checksum_file(&local_release).await?;
            let new_checksum = Checksum::checksum_file(new_release.release()).await?;

            output.new_release = local_checksum != new_checksum;
        } else {
            output.new_release = true;
        }

        let mut release = Release::parse(new_release.release(), &ctx.state.opts)
            .await
            .map_err(|e| MirsError::InvalidReleaseFile { inner: Box::new(e) })?;

        // we prune all the metadata files that this release references that we already have, by comparing the actual checksum.
        // this way, we will attempt to redownload missing files as well as files that are there as a result of a previous
        // sync, where a later release had that file referenced, but wasn't available at the time of mirroring. if all the
        // files are okay, then there is nothing more to do!

        let total_meta_size: u64 = release.files.values().map(|entry| entry.size).sum();

        let file_progress = Progress::new_with_step(0, "Verifying existing");
        file_progress.bytes.inc_total(total_meta_size);

        let processing_progress_bar = file_progress.create_processing_progress_bar().await;

        let dist_root = FilePath(format_compact!(
            "{}/{}",
            ctx.state.repo.root_dir,
            ctx.state.opts.dist_part()
        ));
        release
            .prune_existing(dist_root.as_str(), file_progress.clone())
            .await?;

        file_progress
            .wait_for_completion(&processing_progress_bar)
            .await;

        if release.files.is_empty() {
            if output.new_release {
                return Ok(StepResult::End(MirrorResult::IrrelevantChanges));
            } else {
                return Ok(StepResult::End(MirrorResult::ReleaseUnchanged));
            }
        }

        if let Some(release_components) = release.components() {
            let components = release_components
                .split_ascii_whitespace()
                .map(|v| {
                    v.split('/')
                        .next_back()
                        .expect("last should always exist here")
                })
                .collect::<Vec<&str>>();

            for requested_component in &ctx.state.opts.components {
                if !components.contains(&requested_component.as_str()) {
                    println!(
                        "{} WARNING: {requested_component} is not in this repo",
                        crate::now()
                    );
                }
            }
        }

        if ctx.state.mtime
            && let Some(time_to_set) = release.release_time()
        {
            ctx.state.downloader.set_time(time_to_set);

            let system_time = time_from_atomic(ctx.state.downloader.time_to_set.clone());

            for f in files.into_iter().filter(FilePath::exists) {
                File::open(f)?.set_modified(system_time)?;
            }
        }

        output.total_bytes_downloaded += ctx.progress.bytes.success();
        output.release = Some(release);

        Ok(StepResult::Continue)
    }
}

pub enum ReleaseFile<'a> {
    Detached {
        release: &'a FilePath,
        signature: &'a FilePath,
    },
    Inline {
        release: &'a FilePath,
    },
}

impl<'a> TryFrom<&'a [FilePath]> for ReleaseFile<'a> {
    type Error = MirsError;

    fn try_from(files: &'a [FilePath]) -> std::result::Result<Self, Self::Error> {
        for file in files.iter().filter(|f| f.exists()) {
            if file.file_name() == INRELEASE_FILE_NAME {
                return Ok(ReleaseFile::Inline { release: file });
            }

            if file.file_name() == RELEASE_FILE_NAME {
                let Some(gpg_file) = files
                    .iter()
                    .find(|v| v.file_name() == RELEASE_GPG_FILE_NAME && v.exists())
                else {
                    continue;
                };

                return Ok(ReleaseFile::Detached {
                    release: file,
                    signature: gpg_file,
                });
            }
        }

        Err(MirsError::NoReleaseFile)
    }
}

impl<'a> ReleaseFile<'a> {
    pub fn release(&self) -> &'a FilePath {
        match self {
            ReleaseFile::Detached { release, .. } => release,
            ReleaseFile::Inline { release } => release,
        }
    }
}
