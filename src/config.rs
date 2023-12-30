use std::{path::Path, fmt::Display, cmp::Ordering};
use tokio::io::{BufReader, AsyncBufReadExt};

use crate::error::{Result, MirsError};

pub async fn read_config(path: &Path) -> Result<Vec<MirrorOpts>> {
    let file = tokio::fs::File::open(path).await
        .map_err(|e| MirsError::Config { msg: format!("could not read {}: {e}", path.display()) })?;

    let mut reader = BufReader::with_capacity(8192, file);

    let mut buf = String::with_capacity(8192);

    let mut mirrors = Vec::new();

    let mut line_num = 0_usize;

    loop {
        buf.clear();

        line_num += 1;

        let line = match reader.read_line(&mut buf).await {
            Ok(0) => break,
            Ok(len) => (buf[..len]).trim(),
            Err(e) => return Err(e.into()),
        };

        if line.is_empty() || line.starts_with('#') {
            continue
        }

        match MirrorOpts::try_from(line) {
            Ok(opts) => mirrors.push(opts),
            Err(e) => {
                println!("{} failed parsing config on line {line_num}: {e}", crate::now());
                continue
            },
        }
    }

    let mirrors = merge_similar(mirrors);

    Ok(mirrors)
}

fn merge_similar(mut mirrors: Vec<MirrorOpts>) -> Vec<MirrorOpts> {
    for mirror in mirrors.iter_mut() {
        mirror.arch.sort();
        mirror.components.sort();
    }

    mirrors.sort();

    let merged_mirrors = mirrors.into_iter().fold(Vec::new(), |mut a: Vec<MirrorOpts>, mut v| {
        if let Some(last) = a.last_mut() {
            if last == &v {
                last.components.append(&mut v.components)
            } else {
                a.push(v)
            }
        } else {
            a.push(v);
        }

        a
    });

    merged_mirrors
}

#[derive(Eq)]
pub struct MirrorOpts {
    pub url: String,
    pub distribution: String,
    pub components: Vec<String>,
    pub arch: Vec<String>,
}


impl Ord for MirrorOpts {
    fn cmp(&self, other: &Self) -> Ordering {
        match self.url.cmp(&other.url) {
            Ordering::Equal => {},
            ord => return ord
        }

        match self.distribution.cmp(&other.distribution) {
            Ordering::Equal => {}
            ord => return ord
        }

        self.arch.cmp(&other.arch)
    }
}

impl PartialEq for MirrorOpts {
    fn eq(&self, other: &Self) -> bool {
        self.url == other.url && self.distribution == other.distribution && self.arch == other.arch
    }
}

impl PartialOrd for MirrorOpts {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Display for MirrorOpts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(
            format_args!(
                "{} {}[{}] {}",
                self.url,
                self.distribution,
                self.arch.join(", "),
                self.components.join(" ")
            )
        )
    }
}

impl MirrorOpts {
    pub fn try_from(mut line: &str) -> Result<MirrorOpts> {
        let mut arch = Vec::new();
        
        line = if let Some(line) = line.strip_prefix("deb-src") {
            arch.push("source".to_string());
            line
        } else if let Some(line) = line.strip_prefix("deb") {
            line
        } else {
            return Err(MirsError::Config { msg: String::from("mirror config must start with either 'deb' or 'deb-src'") })
        };

        line = line.trim_start();

        if line.starts_with('[') {
            let Some(bracket_end) = line.find(']') else {
                return Err(MirsError::Config { msg: String::from("options bracket is not closed") })
            };

            let options_line = &line[..bracket_end];

            for part in options_line.split(' ') {
                let mut opt_parts = part.split('=');

                let Some(opt_key) = opt_parts.next() else {
                    return Err(MirsError::Config { msg: String::from("invalid format of options bracket") })
                };

                let Some(opt_val) = opt_parts.next() else {
                    return Err(MirsError::Config { msg: format!("empty value of option key {opt_key}") })
                };

                if opt_key == "arch" {
                    arch.push(opt_val.to_string())
                }
            }
        }

        line = line.trim_start();

        let mut line_parts = line.split_whitespace();

        let Some(url) = line_parts.next() else {
            return Err(MirsError::Config { msg: String::from("no url specified") })
        };

        let Some(distribution) = line_parts.next() else {
            return Err(MirsError::Config { msg: String::from("no distribution specified") })
        };

        let components = line_parts
            .map(|v| v.to_owned())
            .collect::<Vec<_>>();

        if components.is_empty() {
            return Err(MirsError::Config { msg: String::from("no components specified") })
        }

        if arch.is_empty() {
            arch.push("amd64".to_string())
        }

        Ok(Self {
            url: url.to_owned(),
            distribution: distribution.to_owned(),
            components,
            arch
        })
    }
}