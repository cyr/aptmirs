use std::sync::Arc;

use async_trait::async_trait;
use compact_str::format_compact;

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

        let Some(release_file) = get_release_file(&files) else {
            return Err(MirsError::NoReleaseFile)
        };

        if ctx.state.opts.pgp_verify {
            if ctx.state.repo.has_specified_pgp_key() {
                verify_release_signature(&files, ctx.state.repo.as_ref())?;
            } else {
                verify_release_signature(&files, ctx.state.pgp_key_store.as_ref())?;
            }
        }

        let local_release = ctx.state.repo.tmp_to_root(release_file);

        if local_release.exists() {
            let local_checksum = Checksum::checksum_file(&local_release).await?;
            let new_checksum = Checksum::checksum_file(release_file).await?;

            output.new_release = local_checksum != new_checksum;
        }

        let mut release = Release::parse(release_file, &ctx.state.opts).await
            .map_err(|e| MirsError::InvalidReleaseFile { inner: Box::new(e) })?;


        // we prune all the metadata files that this release references that we already have, by comparing the actual checksum.
        // this way, we will attempt to redownload missing files as well as files that are there as a result of a previous 
        // sync, where a later release had that file referenced, but wasn't available at the time of mirroring. if all the
        // files are okay, then there is nothing more to do!
        let dist_root = FilePath(format_compact!("{}/{}", ctx.state.repo.root_dir, ctx.state.opts.dist_part()));
        release.prune_existing(dist_root.as_str()).await?;
        
        if release.files.is_empty() {
            return Ok(StepResult::End(MirrorResult::ReleaseUnchanged))
        }

        if let Some(release_components) = release.components() {
            let components = release_components.split_ascii_whitespace()
                .map(|v| v.split('/').last().expect("last should always exist here"))
                .collect::<Vec<&str>>();

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

fn get_release_file(files: &[FilePath]) -> Option<&FilePath> {
    for file in files.iter().filter(|f| f.exists()) {
        if let "InRelease" | "Release" = file.file_name() {
            return Some(file)
        }
    }
    
    None
}