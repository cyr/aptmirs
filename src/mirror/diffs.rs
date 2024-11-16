use std::{str::FromStr, sync::Arc};

use async_trait::async_trait;
use tokio::{runtime::Handle, task::spawn_blocking};

use crate::{context::Context, error::MirsError, metadata::{metadata_file::MetadataFile, FilePath, IndexSource}, step::{Step, StepResult}};
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
        let mut output = ctx.state.output.lock().await;
        
        let mut progress_bar = ctx.progress.create_download_progress_bar().await;

        let diff_indices = output.take_metadata(
                |f| matches!(f, MetadataFile::DiffIndex(..) )
            ).into_iter()
            .map(IndexSource::from)
            .map(IndexSource::into_reader)
            .collect::<Result<Vec<_>>>()?;

        let task_repo = ctx.state.repo.clone();
        let task_downloader = ctx.state.downloader.clone();
        spawn_blocking(move || {
            let async_handle = Handle::current();

            for diff_index in diff_indices {
                let rel_base_path = FilePath::from_str(
                    task_repo.rel_from_tmp(diff_index.path().parent().expect("diff indicies should have parents"))
                )?;

                for diff_file in diff_index {
                    let mut diff_file = diff_file?;

                    let FilePath(rel_file_path) = rel_base_path.join(diff_file.path);

                    diff_file.path = rel_file_path;

                    let dl = task_repo.create_file_download(diff_file);
                    async_handle.block_on(async {
                        task_downloader.queue(dl).await
                    })?;
                }
            }
            
            Ok::<(), MirsError>(())
        }).await??;

        ctx.progress.wait_for_completion(&mut progress_bar).await;

        output.total_bytes_downloaded += ctx.progress.bytes.success();
        
        Ok(StepResult::Continue)
    }
}
