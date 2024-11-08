use std::sync::Arc;

use async_trait::async_trait;

use crate::error::{MirsError, Result};

use super::{context::Context, MirrorResult};

pub mod release;
pub mod metadata;
pub mod diffs;
pub mod packages;
pub mod debian_installer;

pub enum StepResult {
    Continue,
    End(MirrorResult)
}

#[async_trait]
pub trait Step : Send + Sync {
    async fn execute(&self, ctx: Arc<Context>) -> Result<StepResult>;
    fn step_name(&self) -> &'static str;
    fn error(&self, e: MirsError) -> MirsError;
}