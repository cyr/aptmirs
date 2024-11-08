use std::sync::Arc;

use async_trait::async_trait;

use crate::{error::{MirsError, Result}, metadata::{checksum::Checksum, release::Release, FilePath}, mirror::{context::Context, downloader::Download, MirrorResult}, pgp::verify_release_signature};

use super::{Step, StepResult};

pub struct DownloadRelease;

#[async_trait]
impl Step for DownloadRelease {
    fn step_name(&self) -> &'static str {
        "Downloading release"
    }
    
    fn error(&self, e: MirsError) -> MirsError {
        MirsError::DownloadRelease { inner: Box::new(e) }
    }

    async fn execute(&self, ctx: Arc<Context>) -> Result<StepResult> {
        let mut output = ctx.output.lock().await;

        let mut files = Vec::with_capacity(3);

        ctx.progress.files.inc_total(3);

        let mut progress_bar = ctx.progress.create_download_progress_bar().await;

        for file_url in ctx.repository.release_urls() {
            let destination = ctx.repository.to_path_in_tmp(&file_url);

            let dl = Box::new(Download {
                primary_target_path: destination.clone(),
                url: file_url,
                checksum: None,
                size: None,
                symlink_paths: Vec::new(),
                always_download: true
            });

            let download_res = ctx.downloader.download(dl).await;

            ctx.progress.update_for_files(&mut progress_bar);

            if let Err(e) = download_res {
                println!("{} {e}", crate::now());
                continue
            }

            files.push(destination);
        }

        progress_bar.finish_using_style();

        if ctx.mirror_opts.pgp_verify {
            if ctx.repository.has_specified_pgp_key() {
                verify_release_signature(&files, ctx.repository.as_ref())?;
            } else {
                verify_release_signature(&files, ctx.pgp_key_store.as_ref())?;
            }
        }

        let Some(release_file) = get_release_file(&files) else {
            return Err(MirsError::NoReleaseFile)
        };

        // if the release file we already have has the same checksum as the one we downloaded, because
        // of how all metadata files are moved into the repository path after the mirroring operation
        // is completed successfully, there should be nothing more to do. save bandwidth, save lives!
        let old_release = if let Some(local_release_file) = ctx.repository.tmp_to_root(release_file) {
            if local_release_file.exists() && !ctx.cli_opts.force {
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

            for requested_component in &ctx.mirror_opts.components {
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