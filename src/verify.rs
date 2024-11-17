use std::{fmt::Display, sync::Arc};

use async_trait::async_trait;
use thiserror::Error;
use tokio::sync::Mutex;
use verify::Verify;

use crate::{cmd::{CmdResult, CmdState}, config::MirrorOpts, context::Context, error::MirsError, metadata::repository::Repository, progress::Progress, step::Step, CliOpts};
use crate::error::Result;

pub type VerifyDynStep = Box<dyn Step<VerifyState, Result = VerifyResult>>;
pub type VerifyContext = Arc<Context<VerifyState>>;

pub mod verify;

#[derive(Error, Debug)]
pub enum VerifyResult {
    #[error("Ok: {valid_files} valid, {corrupt_files} corrupt, {missing_files} missing")]
    Done { valid_files: u64, corrupt_files: u64, missing_files: u64 },
    #[error("Fail: {0}")]
    Error(MirsError)
}

impl CmdResult for VerifyResult { }

#[derive(Default)]
pub struct VerifyState {
    pub repo: Arc<Repository>,
    pub opts: Arc<MirrorOpts>,
    pub output: Arc<Mutex<VerifyOutput>>,
}

impl Display for VerifyState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.opts.packages && self.opts.source {
            f.write_str("deb+deb-src")?
        } else if self.opts.packages {
            f.write_str("deb")?
        } else if self.opts.source {
            f.write_str("deb-src")?
        }

        f.write_fmt(format_args!(
            " {} {}[{}] {}",
            self.opts.url,
            self.opts.suite,
            self.opts.arch.join(", "),
            self.opts.components.join(" ")
        ))
    }
}
#[derive(Default)]
pub struct VerifyOutput {
    pub total_corrupt: u64,
    pub total_missing: u64,
    pub total_valid: u64,
}

#[async_trait]
impl CmdState for VerifyState {
    type Result = VerifyResult;

    async fn finalize(&self) -> Self::Result {
        let output = self.output.lock().await;

        VerifyResult::Done {
            valid_files: output.total_valid,
            corrupt_files: output.total_corrupt,
            missing_files: output.total_missing,
        }
    }

    async fn finalize_with_result(&self, result: Self::Result) -> Self::Result {
        result
    }
}

impl Context<VerifyState> {
    fn create_steps() -> Vec<VerifyDynStep> {
        vec![
            Box::new(Verify)
        ]
    }

    pub fn create(opts: Vec<MirrorOpts>, cli_opts: Arc<CliOpts>) -> Result<Vec<(VerifyContext, Vec<VerifyDynStep>)>> {
        opts.into_iter()
            .map(|o| {
                let repo = Arc::new(Repository::build(&o, &cli_opts)?);

                let steps = Self::create_steps();
                let progress = Progress::new();

                let state = VerifyState {
                    repo,
                    opts: Arc::new(o),
                    ..Default::default()
                };

                Ok((Context::build(state, cli_opts.clone(), progress), steps))
            })
            .collect::<Result<Vec<(_, _)>>>()
    }
}