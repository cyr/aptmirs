use std::sync::Arc;

use async_trait::async_trait;

use crate::{context::Context, error::MirsError, step::{Step, StepResult}};
use crate::error::Result;

use super::{PruneResult, PruneState};


pub struct Inventory;

#[async_trait]
impl Step<PruneState> for Inventory {
    type Result = PruneResult;

    fn step_name(&self) -> &'static str {
        "Pruning"
    }
    
    fn error(&self, e: MirsError) -> Self::Result {
        PruneResult::Error(MirsError::Delete { inner: Box::new(e) })
    }

    async fn execute(&self, ctx: Arc<Context<PruneState>>) -> Result<StepResult<Self::Result>> {
        todo!()
    }
}