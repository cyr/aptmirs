use std::sync::{atomic::Ordering, Arc};

use async_trait::async_trait;
use indicatif::MultiProgress;
use tokio::{runtime::Handle, task::spawn_blocking};

use crate::{context::Context, error::{MirsError, Result}, metadata::{metadata_file::MetadataFile, IndexSource}, progress::Progress, step::{Step, StepResult}};

use super::{MirrorResult, MirrorState};

pub struct DownloadFromPackageIndices;

#[async_trait]
impl Step<MirrorState> for DownloadFromPackageIndices {
    type Result = MirrorResult;

    fn step_name(&self) -> &'static str {
        "Downloading packages"
    }
    
    fn error(&self, e: MirsError) -> Self::Result {
        MirrorResult::Error(MirsError::DownloadPackages { inner: Box::new(e) })
    }

    async fn execute(&self, ctx: Arc<Context<MirrorState>>) -> Result<StepResult<Self::Result>> {
        let mut output = ctx.state.output.lock().await;

        let multi_bar = MultiProgress::new();

        let file_progress = Progress::new_with_step(0, "Processing indices");
        let dl_progress = ctx.state.downloader.progress();

        let mut file_progress_bar = multi_bar.add(file_progress.create_processing_progress_bar().await);
        let mut dl_progress_bar = multi_bar.add(dl_progress.create_download_progress_bar().await);

        let packages_files = output.take_metadata(
                |f| matches!(f, MetadataFile::Packages(..) | MetadataFile::Sources(..) )
            ).into_iter()
            .map(IndexSource::from)
            .map(IndexSource::into_reader)
            .collect::<Result<Vec<_>>>()?;

        file_progress.files.inc_total(packages_files.len() as u64);

        let total_size = packages_files.iter().map(|v| v.size()).sum();
        let mut incremental_size_base = 0;

        file_progress.bytes.inc_total(total_size);

        let task_downloader = ctx.state.downloader.clone();
        let task_repo = ctx.state.repo.clone();
        let mut task_dl_progress_bar = dl_progress_bar.clone();
        let task_dl_progress = dl_progress.clone();

        spawn_blocking(move || {
            let async_handle = Handle::current();
            
            for packages_file in packages_files {
                let counter = packages_file.counter();
                file_progress.update_for_bytes(&mut file_progress_bar);
                let package_size = packages_file.size();
        
                for package in packages_file {
                    let package = package?;
        
                    let dl = task_repo.create_file_download(package);
                    async_handle.block_on(async {
                        task_downloader.queue(dl).await
                    })?;
                    
                    file_progress.bytes.set_success(counter.load(Ordering::SeqCst) + incremental_size_base);
        
                    task_dl_progress.update_for_files(&mut task_dl_progress_bar);
                    file_progress.update_for_bytes(&mut file_progress_bar);
                }
        
                incremental_size_base += package_size;
                file_progress.update_for_bytes(&mut file_progress_bar);
            }

            Ok::<(), MirsError>(())
        }).await??;

        dl_progress.wait_for_completion(&mut dl_progress_bar).await;

        output.total_bytes_downloaded += ctx.progress.bytes.success();
        output.total_packages_downloaded += ctx.progress.files.success();

        Ok(StepResult::Continue)
    }
}