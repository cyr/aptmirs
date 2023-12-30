use std::{num::ParseIntError, path::PathBuf};

use async_channel::SendError;
use hex::FromHexError;
use reqwest::StatusCode;
use thiserror::Error;
use tokio::task::JoinError;

use crate::mirror::downloader::Download;

pub type Result<T> = std::result::Result<T, MirsError>;

#[derive(Error, Debug)]
pub enum MirsError {
    #[error(transparent)]
    Io(#[from]std::io::Error),

    #[error(transparent)]
    Reqwest(#[from]reqwest::Error),

    #[error(transparent)]
    ParseInt(#[from]ParseIntError),

    #[error("failed to download {url}: {status_code}")]
    Download { url: String, status_code: StatusCode },

    #[error("failed to parse line {line}")]
    ParsingRelease { line: String },

    #[error("invalid release file: {inner}")]
    InvalidReleaseFile { inner: Box<MirsError> },

    #[error(transparent)]
    Send(#[from]SendError<Box<Download>>),

    #[error("url does not point to a valid repository, no release file found")]
    NoReleaseFile,

    #[error("unable to parse package file {path}")]
    ParsingPackage { path: PathBuf },

    #[error("unable to parse url {url}")]
    UrlParsing { url: String },

    #[error("{msg}")]
    Config { msg: String },

    #[error("could not create a tmp folder: {msg}")]
    Tmp { msg: String},

    #[error(transparent)]
    Hex(#[from]FromHexError),

    #[error("checksum failed for: {url}, expected hash: {expected}, calculated hash: {hash}")]
    Checksum { url: String, expected: String, hash: String },
    
    #[error(transparent)]
    TokioJoin(#[from]JoinError),

    #[error("error occurred while downloading release files: {inner}")]
    DownloadRelease { inner: Box<MirsError> },
    
    #[error("error occurred while downloading indices: {inner}")]
    DownloadIndices { inner: Box<MirsError> },
    
    #[error("error occurred while downloading packages: {inner}")]
    DownloadPackages { inner: Box<MirsError> },

    #[error("error occurred while finalizing mirror operation: {inner}")]
    Finalize { inner: Box<MirsError> },

    #[error("error reading {path}: {inner}")]
    ReadingPackage { path: String, inner: Box<MirsError> }
}