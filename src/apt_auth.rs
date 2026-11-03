use std::{os::unix::fs::MetadataExt, path::Path, sync::Arc};

use tokio::{fs::File, io::AsyncReadExt};

use crate::{CliOpts, error::Result, log};

const MACHINE: &str = "machine";
const LOGIN: &str = "login";
const PASSWORD: &str = "password";

#[derive(Debug, Clone)]
pub struct AptAuth {
    pub host_prefix: String,
    pub username: Arc<str>,
    pub password: Arc<str>,
}

impl AptAuth {
    pub async fn parse(path: &Path) -> Result<Vec<Self>> {
        let mut file = File::open(path).await?;
        let mut buf = String::with_capacity(file.metadata().await?.size() as usize);

        let mut vec = Vec::new();

        file.read_to_string(&mut buf).await?;

        let s = buf.trim();

        let mut tokens = s.split_whitespace().peekable();

        while tokens.peek().is_some() {
            let (
                Some(MACHINE),
                Some(machine),
                Some(LOGIN),
                Some(login),
                Some(PASSWORD),
                Some(password),
            ) = (
                tokens.next(),
                tokens.next(),
                tokens.next(),
                tokens.next(),
                tokens.next(),
                tokens.next(),
            )
            else {
                continue;
            };

            let host_prefix = if !machine.contains("://") {
                format!("https://{machine}")
            } else {
                machine.to_string()
            };

            vec.push(AptAuth {
                host_prefix,
                username: login.into(),
                password: password.into(),
            })
        }

        Ok(vec)
    }
}

pub async fn read_auth(opts: &CliOpts) -> Result<Vec<AptAuth>> {
    let mut vec = Vec::new();

    let Some(path) = &opts.apt_auth_path else {
        return Ok(Vec::new());
    };

    for maybe_entry in walkdir::WalkDir::new(path) {
        let entry = maybe_entry?;

        if entry.file_type().is_dir() {
            continue;
        }

        let Some(entry_path) = entry.file_name().to_str() else {
            continue;
        };

        if !entry_path.ends_with(".conf") {
            continue;
        }

        match AptAuth::parse(entry.path()).await {
            Ok(v) => vec.extend(v),
            Err(e) => {
                log(format!("unable to parse apt_auth config at {}: {e}", entry.path().to_string_lossy()));
                continue
            },
        }
    }

    Ok(vec)
}
