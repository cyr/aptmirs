use std::{fmt::Display, sync::Arc};

use async_trait::async_trait;
use clap::{command, Parser};

use crate::context::Context;
use crate::downloader::Downloader;
use crate::log;
use crate::metadata::repository::Repository;
use crate::{mirror::{debian_installer::DownloadDebianInstaller, diffs::DownloadFromDiffs, metadata::DownloadMetadata, packages::DownloadFromPackageIndices, release::DownloadRelease, MirrorResult, MirrorState}, step::{Step, StepResult}};
use crate::{config::MirrorOpts, pgp::PgpKeyStore, CliOpts};
use crate::error::Result;

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
                let downloader = Downloader::build(cli_opts.dl_threads);

                let ctxs = opts.into_iter()
                    .map(|o| {
                        let repo = Repository::build(&o, &cli_opts)?;

                        let mut steps: Vec<Box<dyn Step<MirrorState, Result = MirrorResult>>> = vec![
                            Box::new(DownloadRelease),
                            Box::new(DownloadMetadata),
                            Box::new(DownloadFromDiffs),
                            Box::new(DownloadFromPackageIndices),
                        ];

                        if o.debian_installer() {
                            steps.push(Box::new(DownloadDebianInstaller))
                        }

                        let progress = downloader.progress();

                        let state = MirrorState {
                            repo,
                            opts: Arc::new(o),
                            downloader: downloader.clone(),
                            pgp_key_store: pgp_key_store.clone(),
                            ..Default::default()
                        };

                        Ok((Context::build(state, cli_opts.clone(), progress)?, steps))
                    })
                    .collect::<Result<Vec<(_, _)>>>()?;

                for (ctx, steps) in ctxs {
                    {
                        let state = ctx.state.lock().await;
                        log(format!("{self} {state}"));
                    }
                    let result = self.run(ctx, steps).await;
                    log(result.to_string());
                }
            },
            Cmd::Verify => todo!(),
            Cmd::Prune => todo!(),
        }

        Ok(())
    }

    async fn run<T: CmdState<Result = R>, R: CmdResult>(self, ctx: Arc<Context<T>>, steps: Vec<Box<dyn Step<T, Result = R>>>) -> R {
        ctx.progress.reset();

        ctx.progress.set_total_steps(steps.len() as u8);

        for step in steps {
            ctx.next_step(step.step_name()).await;
    
            match step.execute(ctx.clone()).await {
                Ok(result) => match result {
                    StepResult::Continue => (),
                    StepResult::End(result) => {
                        return ctx.state.lock().await.finalize_with_result(result).await
                    },
                }
                Err(e) => {
                    return ctx.state.lock().await.finalize_with_result(step.error(e)).await
                },
            }
        }
    
        ctx.state.lock().await.finalize().await
    }
}

pub trait CmdResult : Display { }

#[async_trait]
pub trait CmdState : Display + Sized {
    type Result;

    async fn finalize(&self) -> Self::Result;
    async fn finalize_with_result(&self, result: Self::Result) -> Self::Result;
}