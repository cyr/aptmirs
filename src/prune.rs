use std::{fmt::Display, sync::Arc};

use ahash::HashSet;
use async_trait::async_trait;
use indicatif::HumanBytes;
use thiserror::Error;
use tokio::sync::Mutex;

use crate::{cmd::{CmdResult, CmdState}, config::MirrorOpts, error::MirsError, metadata::{repository::Repository, FilePath}};

mod inventory;

#[derive(Error, Debug)]
pub enum PruneResult { 
    #[error("Ok: pruned {total_files} files, total: {}", HumanBytes(*.total_bytes))]
    Pruned { total_files: u64, total_bytes: u64 },
    #[error("Fail: {0}")]
    Error(MirsError)
}

impl CmdResult for PruneResult { }

pub struct PruneState {
    pub mirrors: Vec<(MirrorOpts, Repository)>,
    pub output: Arc<Mutex<PruneOutput>>,
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

pub struct PruneOutput {
    pub files: HashSet<FilePath>, 
    pub total_files_deleted: u64,
    pub total_bytes_deleted: u64,
}

#[async_trait]
impl CmdState for PruneState {
    type Result = PruneResult;

    async fn finalize(&self) -> Self::Result {
        let output = self.output.lock().await;

        PruneResult::Pruned { total_files: output.total_files_deleted, total_bytes: output.total_bytes_deleted }
    }

    async fn finalize_with_result(&self, result: Self::Result) -> Self::Result {
        result
    }
}