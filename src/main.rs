use std::path::PathBuf;

use clap::{command, arg, Parser, Subcommand, Args, ArgAction};

use crate::error::Result;

mod mirror;
mod error;
mod metadata;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let output = match cli.output {
        Some(output) => output,
        None => std::env::current_dir()?
    };

    if let Some(command) = cli.command {
        match command {
            Commands::Mirror(opts) => mirror::mirror(&opts, &output).await?
        }
    }

    Ok(())
}

#[derive(Parser)]
#[command(author, version, about)]
struct Cli {
    #[arg(short, long, value_name = "CONFIG_FILE")]
    config: Option<PathBuf>,
    
    #[arg(short, long, value_name = "OUTPUT")]
    output: Option<PathBuf>,

    #[arg(short, long, action = ArgAction::Count)]
    verbose: u8,

    #[command(subcommand)]
    command: Option<Commands>,
}   

#[derive(Subcommand)]
enum Commands {
    Mirror(MirrorOpts)
}

#[derive(Args, Debug, Clone)]
pub struct MirrorOpts {
    uri: String,
    distribution: String,
    components: Vec<String>,
    #[arg(short, long, value_name = "ARCH")]
    arch: Option<Vec<String>>,
}