use std::sync::Arc;

use async_trait::async_trait;
use compact_str::format_compact;

use crate::{context::Context, error::MirsError, metadata::{release::MetadataFile, FilePath}, mirror::MirrorResult, step::{Step, StepResult}};
use crate::error::Result;

use super::MirrorState;

pub struct DownloadMetadata;

#[async_trait]
impl Step<MirrorState> for DownloadMetadata {
    type Result = MirrorResult;

    fn step_name(&self) -> &'static str {
        "Downloading metadata"
    }
    
    fn error(&self, e: MirsError) -> Self::Result {
        MirrorResult::Error(MirsError::DownloadMetadata { inner: Box::new(e) })
    }

    async fn execute(&self, ctx: Arc<Context<MirrorState>>) -> Result<StepResult<Self::Result>> {
        let mut state = ctx.state.lock().await;

        let Some(release) = state.release.take() else {
            return Err(MirsError::NoReleaseFile)
        };

        let mut progress_bar = ctx.progress.create_download_progress_bar().await;

        let by_hash = release.acquire_by_hash();

        for (path, file_entry) in release.into_filtered_files(&state.opts.clone()) {
            let mut add_by_hash = by_hash;
            let url = state.repo.to_url_in_dist(path.as_ref());

            let file_path_in_tmp = state.repo.to_path_in_tmp(&url);

            let file_path_in_root = state.repo.to_path_in_root(&url);
            
            // since all files have their checksums verified on download, any file that is local can
            // presumably be trusted to be correct. and since we only move in the metadata files on 
            // a successful mirror operation, if we see the metadata file and its hash file, there is
            // no need to queue its content.
            if let Some(checksum) = file_entry.strongest_hash() {
                let by_hash_base = file_path_in_root
                    .parent()
                    .expect("all files need a parent(?)");

                let checksum_path = FilePath(format_compact!("{by_hash_base}/{}", checksum.relative_path()));

                if let MetadataFile::DebianInstallerSumFile(_) = path {
                    if file_path_in_root.exists() && !ctx.cli_opts.force {
                        continue
                    }
                } else if (!by_hash || checksum_path.exists()) && file_path_in_root.exists() && !ctx.cli_opts.force {
                    continue
                }
            }

            match path {
                MetadataFile::Packages(..) |
                MetadataFile::Sources(..) => {
                    state.package_indices.push(file_path_in_tmp.clone());
                },
                MetadataFile::DiffIndex(..) =>{
                    state.diff_indices.push(file_path_in_tmp.clone());
                },
                MetadataFile::DebianInstallerSumFile(..) => {
                    state.di_sumfiles.push(file_path_in_tmp.clone());
                    add_by_hash = false;
                },
                MetadataFile::Other(..) => ()
            }

            let download = state.repo.create_metadata_download(url, file_path_in_tmp, file_entry, add_by_hash)?;
            state.downloader.queue(download).await?;
        }

        ctx.progress.wait_for_completion(&mut progress_bar).await;

        state.verify_and_prune();

        state.total_bytes_downloaded += ctx.progress.bytes.success();

        if state.is_empty() {
            return Ok(StepResult::End(MirrorResult::IrrelevantChanges))
        }
        
        Ok(StepResult::Continue)
    }
}