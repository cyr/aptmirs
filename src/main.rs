use clap::{command, arg, Parser};
use config::read_config;
use metadata::FilePath;
use mirror::downloader::Downloader;
use pgp::PgpKeyStore;

use crate::error::Result;

mod mirror;
mod error;
mod metadata;
mod config;
mod pgp;

#[tokio::main]
async fn main() -> Result<()> {
    let cli_opts = CliOpts::parse();

    let opts = read_config(&cli_opts.config).await?;

    let mut downloader = Downloader::build(cli_opts.dl_threads);

    let pgp_key_store = if let Some(key_path) = &cli_opts.pgp_key_path {
        Some(PgpKeyStore::build_from_path(key_path)?)
    } else {
        None
    };

    for opt in opts {
        println!("{} Mirroring {}", now(), &opt);

        match mirror::mirror(&opt, &cli_opts, &mut downloader, &pgp_key_store).await {
            Ok(result) => println!("{} Mirroring done: {result}", now()),
            Err(e) => println!("{} Mirroring failed: {e}", now())
        }
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

    #[arg(short, long, env, value_name = "PGP_KEY_PATH")]
    pgp_key_path: Option<FilePath>,
}

fn now() -> String {
    chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}