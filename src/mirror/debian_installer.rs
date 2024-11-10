use std::sync::Arc;

use async_trait::async_trait;
use tokio::{runtime::Handle, task::spawn_blocking};

use crate::{context::Context, error::{MirsError, Result}, metadata::{sum_file::{to_strongest_by_checksum, SumFileEntry}, FilePath}, step::{Step, StepResult}};

use super::{MirrorResult, MirrorState};

pub struct DownloadDebianInstaller;

#[async_trait]
impl Step<MirrorState> for DownloadDebianInstaller {
    type Result = MirrorResult;

    fn step_name(&self) -> &'static str {
        "Downloading debian installer image"
    }
    
    fn error(&self, e: MirsError) -> Self::Result {
        MirrorResult::Error(MirsError::DownloadDebianInstaller { inner: Box::new(e) })
    }
    
    async fn execute(&self, ctx: Arc<Context<MirrorState>>) -> Result<StepResult<Self::Result>> {
        let mut progress_bar = ctx.progress.create_download_progress_bar().await;
    
        let mut state = ctx.state.lock().await;

        let sum_files = to_strongest_by_checksum(&mut state.di_sumfiles)?;

        let mut paths_to_delete = sum_files.iter()
            .map(|sum_file| {
                let base = sum_file.path().parent()
                    .expect("there should always be a parent");

                let rel_path = state.repo.strip_tmp_base(base).expect("sum files should be in tmp");

                state.repo.rebase_to_root(rel_path)
            })
            .collect();

        let task_repo = state.repo.clone();
        let task_downloader = state.downloader.clone();
        spawn_blocking(move || {
            let async_handle = Handle::current();

            for sum_file in sum_files {
                let base_path = sum_file.path().parent()
                    .expect("sum files should have a parent");

                let base_path = FilePath::from(base_path);

                for entry in sum_file.try_into_iter()? {
                    let SumFileEntry { checksum, path } = entry?;

                    let new_path = base_path.join(path);

                    let new_rel_path = task_repo.strip_tmp_base(new_path)
                        .expect("the new path should be in tmp");

                    let url = task_repo.to_url_in_root(new_rel_path.as_str());
                    let target_path = task_repo.to_path_in_tmp(&url);

                    let dl = task_repo.create_raw_download(target_path, url, Some(checksum));

                    async_handle.block_on(async {
                        task_downloader.queue(dl).await
                    })?;
                }
            }
            Ok::<(), MirsError>(())
        }).await??;
        
        state.total_bytes_downloaded += ctx.progress.bytes.success();
        state.delete_paths.append(&mut paths_to_delete);

        ctx.progress.wait_for_completion(&mut progress_bar).await;

        Ok(StepResult::Continue)
    }
}