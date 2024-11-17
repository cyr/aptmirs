use std::sync::Arc;

use async_trait::async_trait;
use tokio::fs::remove_file;
use walkdir::WalkDir;

use crate::{context::Context, error::MirsError, step::{Step, StepResult}};
use crate::error::Result;

use super::{PruneResult, PruneState};

pub struct Delete;

#[async_trait]
impl Step<PruneState> for Delete {
    type Result = PruneResult;

    fn step_name(&self) -> &'static str {
        "Pruning"
    }
    
    fn error(&self, e: MirsError) -> Self::Result {
        PruneResult::Error(MirsError::Delete { inner: Box::new(e) })
    }

    async fn execute(&self, ctx: Arc<Context<PruneState>>) -> Result<StepResult<Self::Result>> {
        let (_, repo) = ctx.state.mirrors.first().expect("there should be a mirror on prune");

        let mut progress_bar = ctx.progress.create_unbounded_progress_bar().await;
        
        let mut output = ctx.state.output.lock().await;

        for entry in WalkDir::new(&repo.root_dir).into_iter().filter_entry(|v| {
            let path = v.path().as_os_str().to_str().expect("path should be utf8");

            !ctx.state.exclude_paths.iter().any(|excl| path.starts_with(excl.as_str()))
        }) {
            let entry = entry?;

            if entry.file_type().is_dir() {
                continue
            }

            let path = repo.strip_root(entry.path().as_os_str().to_str().expect("path should be utf8"));

            let size = entry.metadata()?.len();

            ctx.progress.files.inc_total(1);

            if output.files.contains(path) {
                ctx.progress.files.inc_skipped(1);
                ctx.progress.bytes.inc_skipped(size);
            } else {
                ctx.progress.files.inc_success(1);
                ctx.progress.bytes.inc_success(size);

                if ctx.state.dry_run {
                    eprintln!("{path}");
                } else {
                    remove_file(repo.root_dir.join(path)).await?;
                }
            }

            ctx.progress.update_for_files(&mut progress_bar);
        }

        progress_bar.abandon();

        output.total_valid = ctx.progress.files.skipped();
        output.total_valid_bytes = ctx.progress.bytes.skipped();
        output.total_deleted = ctx.progress.files.success();
        output.total_deleted_bytes = ctx.progress.bytes.success();

        Ok(StepResult::Continue)
    }
}