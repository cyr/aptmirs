use std::sync::Arc;

use async_trait::async_trait;
use compact_str::format_compact;

use crate::{context::Context, error::MirsError, metadata::{metadata_file::{deduplicate_metadata, MetadataFile}, repository::{INRELEASE_FILE_NAME, RELEASE_FILE_NAME}, FilePath}, mirror::MirrorResult, step::{Step, StepResult}};
use crate::error::Result;

use super::{verify_and_prune, MirrorState};

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
        let mut output = ctx.state.output.lock().await;

        let Some(release) = output.release.take() else {
            return Err(MirsError::NoReleaseFile)
        };

        let progress_bar = ctx.progress.create_download_progress_bar().await;

        let by_hash = release.acquire_by_hash();

        let mut metadata = Vec::new();

        for (mut file, file_entry) in release.into_iter() {
            let mut add_by_hash = by_hash;
            let url = ctx.state.repo.to_url_in_dist(file.as_ref());

            let file_path_in_tmp = ctx.state.repo.to_path_in_tmp(&url);

            if file_path_in_tmp.exists() && file_path_in_tmp.file_name() == RELEASE_FILE_NAME || file_path_in_tmp.file_name() == INRELEASE_FILE_NAME {
                eprintln!("WARNING: Self-referential metadata exists, ignoring");
                continue
            }

            let file_path_in_root = ctx.state.repo.to_path_in_root(&url);
            
            // since all files have their checksums verified on download, any file that is local can
            // presumably be trusted to be correct. and since we only move in the metadata files on 
            // a successful mirror operation, if we see the metadata file and its hash file, there is
            // no need to queue its content.
            if let Some(checksum) = file_entry.strongest_hash() {
                let by_hash_base = file_path_in_root.parent().unwrap_or("");

                let checksum_path = FilePath(format_compact!("{by_hash_base}/{}", checksum.relative_path()));

                if let MetadataFile::SumFile(..) = &file {
                    if file_path_in_root.exists() && !ctx.cli_opts.force {
                        continue
                    }
                } else if (checksum_path.exists()) && file_path_in_root.exists() && !ctx.cli_opts.force {
                    continue
                }
            }

            if file.is_index() {
                if let MetadataFile::SumFile(..) = &file {
                    add_by_hash = false;
                }

                *file.path_mut() = file_path_in_tmp.clone();
                metadata.push(file);
            }

            let download = ctx.state.repo.create_metadata_download(url, file_path_in_tmp, file_entry, add_by_hash)?;
            ctx.state.downloader.queue(download).await?;
        }

        ctx.progress.wait_for_completion(&progress_bar).await;

        if ctx.progress.files.failed() > 0 {
            return Err(MirsError::InconsistentRepository { progress: ctx.progress.files.clone() })
        }

        verify_and_prune(&mut metadata);

        output.indices = deduplicate_metadata(metadata);

        output.total_bytes_downloaded += ctx.progress.bytes.success();

        if output.is_empty() {

            let result = if output.new_release {
                MirrorResult::IrrelevantChanges
            } else {
                MirrorResult::ReleaseUnchangedButIncomplete
            };

            return Ok(StepResult::End(result))
        }
        
        Ok(StepResult::Continue)
    }
}