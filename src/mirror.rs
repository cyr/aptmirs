use std::fmt::Display;
use std::sync::Arc;

use context::Context;
use indicatif::HumanBytes;
use step::StepResult;

use crate::error::{MirsError, Result};

pub mod downloader;
pub mod progress;
pub mod repository;
pub mod context;
pub mod step;

pub enum MirrorResult {
    NewRelease { total_download_size: u64, num_packages_downloaded: u64 },
    ReleaseUnchanged,
    IrrelevantChanges
}

impl Display for MirrorResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MirrorResult::NewRelease { total_download_size, num_packages_downloaded } =>
                f.write_fmt(format_args!(
                    "{} downloaded, {} packages/source files", 
                    HumanBytes(*total_download_size),
                    num_packages_downloaded
                )),
            MirrorResult::ReleaseUnchanged =>
                f.write_str("release unchanged"),
            MirrorResult::IrrelevantChanges =>
                f.write_str("new release, but changes do not apply to configured selections")
        }
    }
}

pub async fn mirror(ctx: Arc<Context>) -> Result<MirrorResult> {
    let steps = ctx.create_steps();

    for step in steps {
        ctx.next_step(step.step_name()).await;

        match step.execute(ctx.clone()).await {
            Ok(result) => match result {
                StepResult::Continue => (),
                StepResult::End(mirror_result) => {
                    match mirror_result {
                        MirrorResult::ReleaseUnchanged => {
                            _ = ctx.repository.delete_tmp();
                        },
                        MirrorResult::IrrelevantChanges => {
                            _ = ctx.finalize().await?;
                        },
                        _ => unreachable!()
                    }

                    return Ok(mirror_result)
                },
            }
            Err(e) => {
                _ = ctx.repository.delete_tmp();
                return Err(step.error(e))
            },
        }
    }

    ctx.finalize().await
        .map_err(|e| MirsError::Finalize { inner: Box::new(e) })
}