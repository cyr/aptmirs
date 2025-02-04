use std::sync::Arc;

use ahash::{HashMap, HashMapExt};
use async_trait::async_trait;
use compact_str::CompactString;
use tokio::{runtime::Handle, task::spawn_blocking};

use crate::{context::Context, error::{MirsError, Result}, metadata::{metadata_file::MetadataFile, FilePath, IndexFileEntry}, step::{Step, StepResult}};

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
                |f| matches!(f, MetadataFile::SumFile(..) )
            ).into_iter()
            .map(MetadataFile::into_reader)
            .collect::<Result<Vec<_>>>()?;
        
        let task_repo = ctx.state.repo.clone();
        let task_downloader = ctx.state.downloader.clone();
        let old_files = spawn_blocking(move || {
            let async_handle = Handle::current();
            let mut files_to_delete = Vec::new();

            for sum_file in sum_files {
                let rel_path = task_repo.strip_tmp_base(sum_file.file().path());
                let old_path = task_repo.rebase_rel_to_root(&rel_path);
                let old_base = FilePath::from(old_path.parent().expect("sumfiles should have a parent"));

                let mut old_map = if old_path.exists() {
                    MetadataFile::SumFile(old_path).into_reader()?
                        .map(|v| v.unwrap())
                        .map(|v| (v.path.clone(), v))
                        .collect::<HashMap<CompactString, IndexFileEntry>>()
                } else {
                    HashMap::new()
                };

                let base_path = FilePath::from(sum_file.file().path().parent().expect("sum files should have a parent"));

                for file in sum_file {
                    let file = file?;

                    if let Some(old_file) = old_map.remove(&file.path) {
                        if old_file.checksum == file.checksum {
                            continue
                        }
                    }
 
                    let new_path = base_path.join(&file.path);

                    let new_rel_path = task_repo.strip_tmp_base(&new_path);

                    let url = task_repo.to_url_in_root(new_rel_path.as_str());

                    let dl = task_repo.create_raw_download(new_path, url, file.checksum);

                    async_handle.block_on(async {
                        task_downloader.queue(dl).await
                    })?;
                }

                files_to_delete.extend(old_map.into_keys().map(|v| old_base.join(v))); 
            }
            Ok::<Vec<FilePath>, MirsError>(files_to_delete)
        }).await??;

        ctx.progress.wait_for_completion(&mut progress_bar).await;

        output.total_bytes_downloaded += ctx.progress.bytes.success();
        output.delete_paths.extend(old_files.into_iter());

        Ok(StepResult::Continue)
    }
}