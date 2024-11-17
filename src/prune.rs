use std::{collections::BTreeMap, fmt::Display, sync::Arc};

use ahash::HashSet;
use async_trait::async_trait;
use compact_str::CompactString;
use delete::Delete;
use indicatif::HumanBytes;
use inventory::Inventory;
use thiserror::Error;
use tokio::sync::Mutex;

use crate::{cmd::{CmdResult, CmdState}, config::MirrorOpts, context::Context, error::MirsError, metadata::{repository::Repository, FilePath}, progress::Progress, step::Step, CliOpts};
use crate::error::Result;

mod inventory;
mod delete;

pub type PruneDynStep = Box<dyn Step<PruneState, Result = PruneResult>>;
pub type PruneContext = Arc<Context<PruneState>>;

#[derive(Error, Debug)]
pub enum PruneResult { 
    #[error("Ok: valid {valid_files} ({}), pruned {deleted_files} ({})", HumanBytes(*.valid_bytes), HumanBytes(*.deleted_bytes))]
    Pruned { valid_files: u64, valid_bytes: u64, deleted_files: u64, deleted_bytes: u64 },
    #[error("Fail: {0}")]
    Error(MirsError)
}

impl CmdResult for PruneResult { }

#[derive(Default)]
pub struct PruneState {
    pub mirrors: Vec<(MirrorOpts, Arc<Repository>)>,
    pub output: Arc<Mutex<PruneOutput>>,
    pub exclude_paths: Vec<FilePath>,
    pub dry_run: bool,
}

impl Display for PruneState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let packages = self.mirrors.iter().fold(false, |acc, (opts, _)| acc | opts.packages);
        let source = self.mirrors.iter().fold(false, |acc, (opts, _)| acc | opts.source);

        let url = &self.mirrors.first().unwrap().0.url;

        let suites = self.mirrors.iter()
            .map(|(opts, _)| opts.suite.as_str())
            .collect::<Vec<_>>()
            .join(" ");

        if packages && source {
            f.write_str("deb+deb-src")?
        } else if packages {
            f.write_str("deb")?
        } else if source {
            f.write_str("deb-src")?
        }

        f.write_fmt(format_args!(" {url} {suites}"))
    }
}

#[derive(Default)]
pub struct PruneOutput {
    pub files: HashSet<FilePath>, 
    pub total_valid: u64,
    pub total_valid_bytes: u64,
    pub total_deleted: u64,
    pub total_deleted_bytes: u64,
}

#[async_trait]
impl CmdState for PruneState {
    type Result = PruneResult;

    async fn finalize(&self) -> Self::Result {
        let output = self.output.lock().await;

        PruneResult::Pruned {
            valid_files: output.total_valid,
            valid_bytes: output.total_valid_bytes,
            deleted_files: output.total_deleted,
            deleted_bytes: output.total_deleted_bytes
        }
    }

    async fn finalize_with_result(&self, result: Self::Result) -> Self::Result {
        result
    }
}

impl Context<PruneState> {
    fn create_steps() -> Vec<PruneDynStep> {
        vec![
            Box::new(Inventory),
            Box::new(Delete),
        ]
    }

    pub fn create(opts: Vec<MirrorOpts>, cli_opts: Arc<CliOpts>, dry_run: bool) -> Result<Vec<(PruneContext, Vec<PruneDynStep>)>> {
        let mut mirrors: BTreeMap<CompactString, Vec<(MirrorOpts, Repository)>> = BTreeMap::new();

        for opt in opts {
            let repo = Repository::build(&opt, &cli_opts)?;

            if let Some(set) = mirrors.get_mut(&opt.url) {
                set.push((opt, repo));
            } else {
                mirrors.insert(opt.url.clone(), vec![(opt, repo)]);
            }
        }

        let mirrors: Vec<Vec<(MirrorOpts, Repository)>> = mirrors.into_values().collect();

        let mut exclude_paths = vec![Vec::new(); mirrors.len()];

        for i in 0..mirrors.len() {
            let root_dir = mirrors[i].first().expect("there should be at least one mirror").1.root_dir.as_str();

            let exclude: Vec<FilePath> = mirrors.iter()
                .map(|v| {
                    v.first().expect("there should be at least one mirror").1.root_dir.as_str()
                })
                .filter(|v| root_dir != *v && v.starts_with(root_dir))
                .map(FilePath::from)
                .collect();

            exclude_paths[i] = exclude;
        }

        let ctxs: Vec<(PruneContext, Vec<PruneDynStep>)> = mirrors.into_iter()
            .zip(exclude_paths)
            .map(|(mirrors, exclude_paths)| {
                let mirrors = mirrors.into_iter().map(|(opts, repo)| (opts, Arc::new(repo))).collect();
                (Context::build(PruneState { mirrors, exclude_paths, dry_run, .. Default::default() }, cli_opts.clone(), Progress::new()), Self::create_steps())
            })
            .collect();

        Ok(ctxs)
    }
}