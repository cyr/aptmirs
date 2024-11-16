use std::sync::Arc;

use async_trait::async_trait;

use crate::{context::Context, downloader::Download, error::{MirsError, Result}, log, metadata::{checksum::Checksum, release::Release, FilePath}, mirror::MirrorResult, pgp::verify_release_signature, step::{Step, StepResult}};

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

        let mut progress_bar = ctx.progress.create_download_progress_bar().await;

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
                always_download: true
            });

            let download_res = ctx.state.downloader.download(dl).await;

            ctx.progress.update_for_files(&mut progress_bar);

            if let Err(e) = download_res {
                log(e.to_string());
                continue
            }

            files.push(destination);
        }

        progress_bar.finish_using_style();

        if ctx.state.opts.pgp_verify {
            if ctx.state.repo.has_specified_pgp_key() {
                verify_release_signature(&files, ctx.state.repo.as_ref())?;
            } else {
                verify_release_signature(&files, ctx.state.pgp_key_store.as_ref())?;
            }
        }

        let Some(release_file) = get_release_file(&files) else {
            return Err(MirsError::NoReleaseFile)
        };

        // if the release file we already have has the same checksum as the one we downloaded, because
        // of how all metadata files are moved into the repository path after the mirroring operation
        // is completed successfully, there should be nothing more to do. save bandwidth, save lives!
        let old_release = if let Some(local_release_file) = ctx.state.repo.tmp_to_root(release_file) {
            if !ctx.cli_opts.force && local_release_file.exists() {
                let tmp_checksum = Checksum::checksum_file(&local_release_file).await?;
                let local_checksum = Checksum::checksum_file(release_file).await?;

                if tmp_checksum == local_checksum {
                    return Ok(StepResult::End(MirrorResult::ReleaseUnchanged))
                }

                Some(
                    Release::parse(&local_release_file).await
                        .map_err(|e| MirsError::InvalidReleaseFile { inner: Box::new(e) })?
                )
            } else {
                None
            }
        } else {
            None
        };

        let mut release = Release::parse(release_file).await
            .map_err(|e| MirsError::InvalidReleaseFile { inner: Box::new(e) })?;

        if let Some(old_release) = old_release {
            release.deduplicate(old_release);
        }
        
        if let Some(release_components) = release.components() {
            let components = release_components.split_ascii_whitespace().collect::<Vec<&str>>();

            for requested_component in &ctx.state.opts.components {
                if !components.contains(&requested_component.as_str()) {
                    println!("{} WARNING: {requested_component} is not in this repo", crate::now());
                }
            }
        }

        output.total_bytes_downloaded += ctx.progress.bytes.success();
        output.release = Some(release);

        Ok(StepResult::Continue)
    }
}

fn get_release_file(files: &Vec<FilePath>) -> Option<&FilePath> {
    for file in files {
        if let "InRelease" | "Release" = file.file_name() {
            return Some(file)
        }
    }

    None
}