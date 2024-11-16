use std::{fmt::Display, sync::Arc};

use async_trait::async_trait;
use clap::{command, Parser};

use crate::context::Context;
use crate::log;
use crate::prune::PruneState;
use crate::{mirror::MirrorState, step::{Step, StepResult}};
use crate::{config::MirrorOpts, pgp::PgpKeyStore, CliOpts};
use crate::error::Result;

pub type DynStep<T, R> = Box<dyn Step<T, Result = R>>;
pub type ArcContext<T> = Arc<Context<T>>;
pub type ContextWithSteps<T, R> = (ArcContext<T>, Vec<DynStep<T, R>>);

#[derive(Parser, Clone, Copy, Default)]
#[command()]
pub enum Cmd {
    #[default]
    /// Mirrors the configured repositories. If no command is specified, this is the default behavior.
    Mirror,
    /// Verifies the downloaded mirror(s) against the mirror configuration and outputs a report
    Verify,
    /// Removes unreferenced files in the downloaded mirror(s)  
    Prune
}

impl Display for Cmd {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Cmd::Mirror => f.write_str("Mirroring"),
            Cmd::Verify => f.write_str("Verifying"),
            Cmd::Prune => f.write_str("Pruning"),
        }
    }
}

impl Cmd {
    pub async fn execute(self, opts: Vec<MirrorOpts>, cli_opts: Arc<CliOpts>, pgp_key_store: Arc<PgpKeyStore>) -> Result<()> {
        match self {
            Cmd::Mirror => {
                let ctxs = Context::<MirrorState>::create(opts, cli_opts, pgp_key_store)?;
                self.run_all(ctxs).await;
            },
            Cmd::Prune => {
                let ctxs = Context::<PruneState>::create(opts, cli_opts)?;
                self.run_all(ctxs).await;
            },
            Cmd::Verify => todo!(),
        }

        Ok(())
    }

    async fn run<T: CmdState<Result = R>, R: CmdResult>(self, ctx: ArcContext<T>, steps: Vec<DynStep<T, R>>) -> R {
        ctx.progress.reset();

        ctx.progress.set_total_steps(steps.len() as u8);

        for step in steps {
            ctx.next_step(step.step_name()).await;
    
            match step.execute(ctx.clone()).await {
                Ok(result) => match result {
                    StepResult::Continue => (),
                    StepResult::End(result) => {
                        return ctx.state.finalize_with_result(result).await
                    },
                }
                Err(e) => {
                    return ctx.state.finalize_with_result(step.error(e)).await
                },
            }
        }
    
        ctx.state.finalize().await
    }

    async fn run_all<T: CmdState<Result = R>, R: CmdResult>(self, ctxs: Vec<ContextWithSteps<T, R>>) {
        for (ctx, steps) in ctxs {
            log(format!("{self} {}", ctx.state));
            let result = self.run(ctx, steps).await;
            log(result.to_string());
        }
    }
}

pub trait CmdResult : Display { }

#[async_trait]
pub trait CmdState : Display + Sized {
    type Result;

    async fn finalize(&self) -> Self::Result;
    async fn finalize_with_result(&self, result: Self::Result) -> Self::Result;
}