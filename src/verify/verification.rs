use std::sync::Arc;

use async_trait::async_trait;
use compact_str::format_compact;
use tokio::{runtime::Handle, task::spawn_blocking};

use crate::{context::Context, error::MirsError, metadata::{metadata_file::{deduplicate_metadata, MetadataFile}, release::{FileEntry, Release}, repository::{INRELEASE_FILE_NAME, RELEASE_FILE_NAME, RELEASE_GPG_FILE_NAME}, FilePath}, mirror::verify_and_prune, step::{Step, StepResult}, verifier::VerifyTask};
use crate::error::Result;

use super::{VerifyResult, VerifyState};

pub struct Verify;

#[async_trait]
impl Step<VerifyState> for Verify {
    type Result = VerifyResult;
    
    fn step_name(&self) -> &'static str {
        "Verifying"
    }
    
    fn error(&self, e: MirsError) -> Self::Result {
        VerifyResult::Error(MirsError::Verify { inner: Box::new(e) })
    }

    async fn execute(&self, ctx: Arc<Context<VerifyState>>) -> Result<StepResult<Self::Result>> {
        let progress = ctx.progress.clone();
        let mut output = ctx.state.output.lock().await;

        let mut progress_bar = progress.create_download_progress_bar().await;

        let dist_root = FilePath(format_compact!("{}/{}", ctx.state.repo.root_dir, ctx.state.opts.dist_part()));

        let release_files = get_rooted_release_files(&dist_root);

        let Some(release_file) = pick_release(&release_files) else {
            return Err(MirsError::NoReleaseFile)
        };

        let release = Release::parse(release_file, &ctx.state.opts).await?;

        let by_hash = release.acquire_by_hash();

        let mut metadata: Vec<(MetadataFile, FileEntry)> = release.into_iter().collect();

        for (metadata_file, file_entry) in &mut metadata {
            metadata_file.prefix_with(dist_root.as_str());

            let size = file_entry.size;
            let (checksum, primary, ..) = file_entry.into_paths(metadata_file.path(), by_hash)?;

            ctx.state.verifier.queue(Arc::new(VerifyTask {
                size: Some(size),
                checksum: checksum.ok_or_else(|| MirsError::VerifyTask { path: primary.clone() })?,
                paths: vec![primary]
            })).await?;
        }

        let mut metadata = metadata.into_iter()
            .map(|(v, _)| v)
            .filter(MetadataFile::is_index)
            .collect();

        verify_and_prune(&mut metadata);

        let metadata = deduplicate_metadata(metadata);

        let index_files = metadata.into_iter()
            .map(MetadataFile::into_reader)
            .collect::<Result<Vec<_>>>()?;
        
        let total_size = index_files.iter().map(|v| v.size()).sum();
        progress.bytes.inc_total(total_size);

        let task_verifier = ctx.state.verifier.clone();
        let task_progress = progress.clone();
        let task_repo = ctx.state.repo.clone();
        let mut task_progress_bar = progress_bar.clone();
        
        spawn_blocking(move || {
            let async_handle = Handle::current();

            for meta_file in index_files {
                let base_path = match meta_file.file() {
                    MetadataFile::Packages(..) |
                    MetadataFile::Sources(..) => task_repo.root_dir.clone(),
                    MetadataFile::SumFile(file_path) |
                    MetadataFile::DiffIndex(file_path) => {
                        FilePath::from(file_path.parent().expect("diff indicies should have parents"))
                    },
                    MetadataFile::Other(..) => unreachable!()
                };
                
                for entry in meta_file {
                    let mut entry = entry?;

                    entry.path = base_path.join(&entry.path).0;

                    let verify_task = Arc::new(VerifyTask::try_from(entry)?);

                    async_handle.block_on(async {
                        task_verifier.queue(verify_task).await
                    })?;

                    task_progress.update_for_files(&mut task_progress_bar);
                }
            }

            Ok::<(), MirsError>(())
        }).await??;
        
        progress.wait_for_completion(&mut progress_bar).await;

        output.total_corrupt = progress.files.failed();
        output.total_missing = progress.files.skipped();
        output.total_valid = progress.files.success();

        Ok(StepResult::Continue)
    }
}

fn get_rooted_release_files(root: &FilePath) -> Vec<FilePath> {
    [
        root.join(INRELEASE_FILE_NAME),
        root.join(RELEASE_FILE_NAME),
        root.join(RELEASE_GPG_FILE_NAME)
    ].into_iter()
        .filter(|v| v.exists())
        .collect()
}

fn pick_release(files: &[FilePath]) -> Option<&FilePath> {
    for f in files {
        if let INRELEASE_FILE_NAME | RELEASE_FILE_NAME = f.file_name() {
            return Some(f)
        }
    }

    None
}