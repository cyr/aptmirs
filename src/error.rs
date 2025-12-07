use std::{num::ParseIntError, sync::Arc};

use async_channel::SendError;
use compact_str::CompactString;
use hex::FromHexError;
use reqwest::StatusCode;
use thiserror::Error;
use tokio::task::JoinError;

use crate::{downloader::Download, metadata::FilePath, progress::ProgressPart, verifier::VerifyTask};

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
    Download { url: CompactString, status_code: StatusCode },

    #[error("failed to parse line {line}")]
    ParsingRelease { line: CompactString },

    #[error("invalid release file: {inner}")]
    InvalidReleaseFile { inner: Box<MirsError> },

    #[error(transparent)]
    Send(#[from]SendError<Box<Download>>),

    #[error(transparent)]
    VerifyTaskSend(#[from]SendError<Arc<VerifyTask>>),

    #[error("unable to verify {path}")]
    VerifyTask { path: FilePath },

    #[error("url does not point to a valid repository, no release file found")]
    NoReleaseFile,

    #[error("error parsing sum file {path}: {inner}")]
    SumFileParsing { path: FilePath, inner: Box<MirsError> },

    #[error("invalid entry in sum file")]
    InvalidSumEntry { line: CompactString },

    #[error("unable to parse packages file {path}")]
    ParsingPackages { path: FilePath },

    #[error("unable to parse sources file {path}")]
    ParsingSources { path: FilePath },

    #[error("unable to parse index diff file {path}")]
    ParsingDiffIndex { path: FilePath },

    #[error("unable to parse url {url}")]
    UrlParsing { url: CompactString },

    #[error("{msg}")]
    Config { msg: CompactString },

    #[error("could not create a tmp folder: {msg}")]
    Tmp { msg: CompactString},

    #[error(transparent)]
    Hex(#[from]FromHexError),

    #[error("{value} is not a recognized checksum")]
    IntoChecksum { value: String },

    #[error("checksum failed for: {url}, expected hash: {expected}, calculated hash: {hash}")]
    Checksum { url: CompactString, expected: CompactString, hash: String },
    
    #[error(transparent)]
    TokioJoin(#[from]JoinError),

    #[error("error occurred while downloading release files: {inner}")]
    DownloadRelease { inner: Box<MirsError> },
    
    #[error("error occurred while downloading indices: {inner}")]
    DownloadMetadata { inner: Box<MirsError> },
    
    #[error("error occurred while downloading diffs: {inner}")]
    DownloadDiffs { inner: Box<MirsError> },

    #[error("error occurred while downloading packages: {inner}")]
    DownloadPackages { inner: Box<MirsError> },

    #[error("error occurred while downloading debian installer: {inner}")]
    DownloadDebianInstaller { inner: Box<MirsError> },

    #[error("error occurred while taking inventory for pruning: {inner}")]
    Inventory { inner: Box<MirsError> },

    #[error("error occurred while verifying: {inner}")]
    Verify { inner: Box<MirsError> },

    #[error("error occurred while pruning: {inner}")]
    Delete { inner: Box<MirsError> },

    #[error("error occurred while finalizing mirror operation: {inner}")]
    Finalize { inner: Box<MirsError> },

    #[error("error reading {path}: {inner}")]
    ReadingPackage { path: FilePath, inner: Box<MirsError> },

    #[error("error reading path: {inner}")]
    WalkDir { #[from]inner: walkdir::Error },

    #[error("PGP error: {inner}")]
    Pgp { #[from] inner: pgp::errors::Error },

    #[error("PGP key path error: {inner}")]
    PgpKeyStore { inner: walkdir::Error },

    #[error("unable to read PGP pub key: {inner}")]
    PgpPubKey { path: FilePath, inner: Box<MirsError> },

    #[error("this repository does not provide a PGP signature, yet a public key has been provided - no verification can be made")]
    PgpNotSupported,

    #[error("could not verify PGP signature")]
    PgpNotVerified,

    #[error("non-index file can't be made into readers")]
    NonIndexFileBuild { path: FilePath },

    #[error("repository is in an inconsistent state, file stats: {progress}")]
    InconsistentRepository { progress: ProgressPart }
}