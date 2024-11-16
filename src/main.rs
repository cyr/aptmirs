use std::{fmt::Display, sync::Arc};

use clap::{command, arg, Parser};
use cmd::Cmd;
use config::read_config;
use metadata::FilePath;
use pgp::PgpKeyStore;

use crate::error::Result;

mod mirror;
mod error;
mod metadata;
mod config;
mod pgp;
mod prune;
mod verify;
mod step;
mod context;
mod downloader;
mod progress;
mod cmd;

#[tokio::main]
async fn main() -> Result<()> {
    let cli_opts = Arc::new(CliOpts::parse());

    let result = {
        let opts = read_config(&cli_opts.config).await?;
        let pgp_key_store = Arc::new(PgpKeyStore::try_from(&cli_opts)?);
    
        cli_opts.command().execute(opts, cli_opts, pgp_key_store).await
    };

    if let Err(e) = result {
        println!("FATAL: {e}")
    }

    Ok(())
}

#[derive(Parser)]
#[command(author, version, about)]
struct CliOpts {
    #[arg(short, long, env, value_name = "CONFIG_FILE", default_value = "/etc/apt/mirror.list", 
        help = "The path to the config file containing the mirror options")]
    config: FilePath,
    
    #[arg(short, long, env, value_name = "OUTPUT",
        help = "The directory where the mirrors will be downloaded into")]
    output: FilePath,

    #[arg(short, long, env, value_name = "DL_THREADS", default_value_t = 8_u8,
        help = "The maximum number of concurrent downloading tasks")]
    dl_threads: u8,

    #[arg(short, long, env, value_name = "PGP_KEY_PATH",
        help = "Path to folder where PGP public keys reside. All valid keys will be used in signature verification where applicable")]
    pgp_key_path: Option<FilePath>,

    #[arg(short, long, env, value_name = "FORCE",
        help = "Ignore current release file and package files and assume all metadata is stale")]
    force: bool,

    #[command(subcommand)]
    command: Option<Cmd>,
}

impl CliOpts {
    pub fn command(&self) -> Cmd {
        self.command.unwrap_or_default()
    }
}

fn now() -> String {
    chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn log<M: Display>(msg: M) {
    println!("{} {msg}", now());
}