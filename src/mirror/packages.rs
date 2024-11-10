use std::{collections::BTreeMap, sync::{atomic::Ordering, Arc}};

use async_trait::async_trait;
use compact_str::format_compact;
use indicatif::MultiProgress;
use tokio::{runtime::Handle, task::spawn_blocking};

use crate::{context::Context, error::{MirsError, Result}, metadata::{FilePath, IndexSource}, progress::Progress, step::{Step, StepResult}};

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
        let mut state = ctx.state.lock().await;

        let multi_bar = MultiProgress::new();

        let file_progress = Progress::new_with_step(3, "Processing indices");
        let dl_progress = state.downloader.progress();

        let mut file_progress_bar = multi_bar.add(file_progress.create_processing_progress_bar().await);
        let mut dl_progress_bar = multi_bar.add(dl_progress.create_download_progress_bar().await);

        let mut existing_indices = BTreeMap::<FilePath, FilePath>::new();

        while let Some(index_file_path) = state.package_indices.pop() {
            let file_stem = index_file_path.file_stem();
            let path_with_stem = FilePath(format_compact!(
                "{}/{}", 
                index_file_path.parent().unwrap(), 
                file_stem
            ));

            if let Some(val) = existing_indices.get_mut(&path_with_stem) {
                if is_extension_preferred(val.extension(), index_file_path.extension()) {
                    *val = index_file_path
                }
            } else {
                existing_indices.insert(path_with_stem, index_file_path);
            }
        }

        file_progress.files.inc_total(existing_indices.len() as u64);

        let packages_files = existing_indices.into_values()
            .map(IndexSource::from)
            .map(|v| v.into_reader())
            .collect::<Result<Vec<_>>>()?;

        let total_size = packages_files.iter().map(|v| v.size()).sum();
        let mut incremental_size_base = 0;

        file_progress.bytes.inc_total(total_size);

        let task_downloader = state.downloader.clone();
        let task_repo = state.repo.clone();
        let mut task_dl_progress_bar = dl_progress_bar.clone();
        let task_dl_progress = dl_progress.clone();

        let async_handle = Handle::current();
        spawn_blocking(move || {
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

        state.total_bytes_downloaded += ctx.progress.bytes.success();
        state.total_packages_downloaded += ctx.progress.files.success();

        Ok(StepResult::Continue)
    }
}

fn is_extension_preferred(old: Option<&str>, new: Option<&str>) -> bool {
    matches!((old, new),
        (_, Some("gz")) |
        (_, Some("xz")) |
        (_, Some("bz2")) 
    )
}