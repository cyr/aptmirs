use std::{str::FromStr, sync::Arc};

use async_trait::async_trait;
use tokio::{runtime::Handle, task::spawn_blocking};

use crate::{error::{MirsError, Result}, metadata::{sum_file::{to_strongest_by_checksum, SumFileEntry}, FilePath}, mirror::context::Context};

use super::{Step, StepResult};

pub struct DownloadDebianInstaller;

#[async_trait]
impl Step for DownloadDebianInstaller {
    fn step_name(&self) -> &'static str {
        "Downloading debian installer image"
    }
    
    fn error(&self, e: MirsError) -> MirsError {
        MirsError::DownloadDebianInstaller { inner: Box::new(e) }
    }
    
    async fn execute(&self, ctx: Arc<Context>) -> Result<StepResult> {
        let mut progress_bar = ctx.progress.create_download_progress_bar().await;
    
        let mut output = ctx.output.lock().await;

        let sum_files = to_strongest_by_checksum(&mut output.di_sumfiles)?;

        let mut paths_to_delete = sum_files.iter()
            .map(|sum_file| {
                let base = sum_file.path().parent()
                    .expect("there should always be a parent");

                let rel_path = ctx.repository.strip_tmp_base(base).expect("sum files should be in tmp");

                ctx.repository.rebase_to_root(rel_path)
            })
            .collect();

        let task_ctx = ctx.clone();
        spawn_blocking(move || {
            let async_handle = Handle::current();

            for sum_file in sum_files {
                let base_path = sum_file.path().parent()
                    .expect("sum files should have a parent");

                let base_path = FilePath::from_str(base_path)?;

                for entry in sum_file.try_into_iter()? {
                    let SumFileEntry { checksum, path } = entry?;

                    let new_path = base_path.join(path);

                    let new_rel_path = task_ctx.repository.strip_tmp_base(new_path)
                        .expect("the new path should be in tmp");

                    let url = task_ctx.repository.to_url_in_root(new_rel_path.as_str());
                    let target_path = task_ctx.repository.to_path_in_tmp(&url);

                    let dl = task_ctx.repository.create_raw_download(target_path, url, Some(checksum));

                    async_handle.block_on(async {
                        task_ctx.downloader.queue(dl).await
                    })?;
                }
            }
            Ok::<(), MirsError>(())
        }).await??;
        
        output.total_bytes_downloaded += ctx.progress.bytes.success();
        output.delete_paths.append(&mut paths_to_delete);

        ctx.progress.wait_for_completion(&mut progress_bar).await;

        Ok(StepResult::Continue)
    }
}