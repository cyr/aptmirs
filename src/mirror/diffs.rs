use std::{str::FromStr, sync::Arc};

use async_trait::async_trait;

use crate::{context::Context, downloader::Download, error::MirsError, metadata::{diff_index_file::DiffIndexFile, FilePath}, step::{Step, StepResult}};
use crate::error::Result;

use super::{MirrorResult, MirrorState};

pub struct DownloadFromDiffs;

#[async_trait]
impl Step<MirrorState> for DownloadFromDiffs {
    type Result = MirrorResult;

    fn step_name(&self) -> &'static str {
        "Downloading diffs"
    }
    
    fn error(&self, e: MirsError) -> Self::Result {
        MirrorResult::Error(MirsError::DownloadDiffs { inner: Box::new(e) })
    }
    
    async fn execute(&self, ctx: Arc<Context<MirrorState>>) -> Result<StepResult<Self::Result>> {
        let mut state = ctx.state.lock().await;
        
        let mut progress_bar = ctx.progress.create_download_progress_bar().await;

        for path in &state.diff_indices {
            let rel_path = FilePath::from_str(
                state.repo.rel_from_tmp(path.as_str())
            )?;

            let rel_base_path = FilePath::from_str(rel_path.parent().unwrap())?;

            let mut diff_index = DiffIndexFile::parse(path).await?;

            while let Some((path, entry)) = diff_index.files.pop_first() {
                let rel_file_path = rel_base_path.join(&path);

                let url = state.repo.to_url_in_root(rel_file_path.as_str());
                let primary_target_path = state.repo.to_path_in_root(&url);

                let checksum = entry.strongest_hash();

                let download = Download {
                    url,
                    size: Some(entry.size),
                    checksum,
                    primary_target_path,
                    symlink_paths: Vec::new(),
                    always_download: false,
                };

                state.downloader.queue(Box::new(download)).await?;
            }
        }

        ctx.progress.wait_for_completion(&mut progress_bar).await;

        state.total_bytes_downloaded += ctx.progress.bytes.success();
        
        Ok(StepResult::Continue)
    }
}
