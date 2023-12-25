use std::path::PathBuf;

use clap::{command, arg, Parser, ArgAction};
use config::read_config;
use error::MirsError;
use indicatif::HumanBytes;

use crate::error::Result;

mod mirror;
mod error;
mod metadata;
mod config;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let output = match cli.output {
        Some(output) => output,
        None => std::env::current_dir()?
    };

    let opts = read_config(
        &cli.config.expect("config should have a default value")
    ).await?;

    if opts.is_empty() {
        return Err(MirsError::Config { msg: String::from("config file did not contain any valid repositories") })
    }

    for opt in opts {
        println!("{} Mirroring {}", now(), &opt);

        let downloaded_bytes = mirror::mirror(&opt, &output).await?;

        println!("{} Mirroring done, {} downloaded", now(), HumanBytes(downloaded_bytes));
    }

    Ok(())
}

#[derive(Parser)]
#[command(author, version, about)]
struct Cli {
    #[arg(short, long, env, value_name = "CONFIG_FILE", default_value = "./mirror.list")]
    config: Option<PathBuf>,
    
    #[arg(short, long, env, value_name = "OUTPUT")]
    output: Option<PathBuf>,

    #[arg(short, long, action = ArgAction::Count)]
    verbose: u8,
}

fn now() -> String {
    chrono::Local::now().to_rfc3339()
}