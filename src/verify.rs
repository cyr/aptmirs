use std::{fmt::Display, sync::Arc};

use async_trait::async_trait;
use thiserror::Error;
use tokio::sync::Mutex;
use verification::Verify;

use crate::{cmd::{CmdResult, CmdState}, config::MirrorOpts, context::Context, error::MirsError, metadata::repository::Repository, step::Step, verifier::Verifier, CliOpts};
use crate::error::Result;

pub type VerifyDynStep = Box<dyn Step<VerifyState, Result = VerifyResult>>;
pub type VerifyContext = Arc<Context<VerifyState>>;

pub mod verification;

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
    pub verifier: Verifier,
}

impl Display for VerifyState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.opts.fmt(f)
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
        let verifier = Verifier::build(cli_opts.dl_threads);

        opts.into_iter()
            .map(|o| {
                let repo = Arc::new(Repository::build(&o, &cli_opts)?);

                let steps = Self::create_steps();

                let state = VerifyState {
                    repo,
                    opts: Arc::new(o),
                    verifier: verifier.clone(),
                    ..Default::default()
                };

                Ok((Context::build(state, cli_opts.clone(), verifier.progress()), steps))
            })
            .collect::<Result<Vec<(_, _)>>>()
    }
}