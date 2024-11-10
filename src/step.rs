use std::{fmt::Display, sync::Arc};

use async_trait::async_trait;

use crate::{cmd::{CmdResult, CmdState}, error::{MirsError, Result}};

use super::context::Context;

pub enum StepResult<T: Display> {
    Continue,
    End(T)
}

#[async_trait]
pub trait Step<T: CmdState<Result: CmdResult>> : Send + Sync {
    type Result: Display;

    async fn execute(&self, ctx: Arc<Context<T>>) -> Result<StepResult<Self::Result>>;
    fn step_name(&self) -> &'static str;
    fn error(&self, e: MirsError) -> Self::Result;
}