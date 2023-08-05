use std::{num::ParseIntError, path::PathBuf};

use async_channel::SendError;
use reqwest::StatusCode;
use thiserror::Error;

use crate::mirror::Download;

pub type Result<T> = std::result::Result<T, MirsError>;

#[derive(Error, Debug)]
pub enum MirsError {
    #[error(transparent)]
    Io(#[from]std::io::Error),

    #[error(transparent)]
    Reqwest(#[from]reqwest::Error),

    #[error(transparent)]
    ParseInt(#[from]ParseIntError),

    #[error("failed to download {uri}: {status_code}")]
    Download { uri: String, status_code: StatusCode },

    #[error("failed to parse line {line}")]
    ParsingRelease { line: String },

    #[error(transparent)]
    Send(#[from]SendError<Download>),

    #[error("uri does not point to a valid repository")]
    InvalidRepository,

    #[error("unable to parse package file {path}")]
    ParsingPackage { path: PathBuf }
}