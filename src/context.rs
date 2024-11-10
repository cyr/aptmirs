use std::sync::Arc;

use tokio::sync::Mutex;

use crate::cmd::{CmdResult, CmdState};
use crate::{progress::Progress, CliOpts};
use crate::error::Result;

#[derive(Clone)]
pub struct Context<T> where T: CmdState<Result: CmdResult> {
    pub progress: Progress,
    pub cli_opts: Arc<CliOpts>,
    pub state: Arc<Mutex<T>>
}

impl<T> Context<T> where T: CmdState<Result: CmdResult> {
    pub fn build(state: T, cli_opts: Arc<CliOpts>, progress: Progress) -> Result<Arc<Self>> {
        Ok(Arc::new(Context {
            progress,
            cli_opts,
            state: Arc::new(Mutex::new(state))
        }))
    }

    pub async fn next_step(&self, step_name: &str) {
        self.progress.next_step(step_name).await;
    }
}