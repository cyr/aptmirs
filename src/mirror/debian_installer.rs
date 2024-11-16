use std::sync::Arc;

use async_trait::async_trait;
use tokio::{runtime::Handle, task::spawn_blocking};

use crate::{context::Context, error::{MirsError, Result}, metadata::{metadata_file::MetadataFile, FilePath}, step::{Step, StepResult}};

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
    
        let mut output = ctx.state.output.lock().await;

        let sum_files = output.take_metadata(
                |f| matches!(f, MetadataFile::DebianInstallerSumFile(..) )
            ).into_iter()
            .map(MetadataFile::into_reader)
            .collect::<Result<Vec<_>>>()?;

        let mut paths_to_delete = sum_files.iter()
            .map(|file| {
                let base = file.file().path().parent()
                    .expect("there should always be a parent");

                let rel_path = ctx.state.repo.strip_tmp_base(base).expect("sum files should be in tmp");

                ctx.state.repo.rebase_to_root(rel_path)
            })
            .collect();

        let task_repo = ctx.state.repo.clone();
        let task_downloader = ctx.state.downloader.clone();
        spawn_blocking(move || {
            let async_handle = Handle::current();

            for sum_file in sum_files {
                let base_path = sum_file.file().path().parent()
                    .expect("sum files should have a parent");

                let base_path = FilePath::from(base_path);

                for file in sum_file {
                    
                    let file = file?;

                    let new_path = base_path.join(&file.path);

                    let new_rel_path = task_repo.strip_tmp_base(&new_path)
                        .expect("the new path should be in tmp");

                    let url = task_repo.to_url_in_root(new_rel_path.as_str());

                    let dl = task_repo.create_raw_download(new_path, url, file.checksum);

                    async_handle.block_on(async {
                        task_downloader.queue(dl).await
                    })?;
                }
            }
            Ok::<(), MirsError>(())
        }).await??;

        ctx.progress.wait_for_completion(&mut progress_bar).await;

        output.total_bytes_downloaded += ctx.progress.bytes.success();
        output.delete_paths.append(&mut paths_to_delete);

        Ok(StepResult::Continue)
    }
}