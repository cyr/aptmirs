use std::{cmp::Ordering, fmt::Display};
use compact_str::{format_compact, CompactString, ToCompactString};
use tokio::io::{BufReader, AsyncBufReadExt};

use crate::{error::{MirsError, Result}, metadata::FilePath};

pub async fn read_config(path: &FilePath) -> Result<Vec<MirrorOpts>> {
    let file = tokio::fs::File::open(path).await
        .map_err(|e| MirsError::Config { msg: format_compact!("could not read {path}: {e}") })?;

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

    if mirrors.is_empty() {
        return Err(MirsError::Config { msg: format_compact!("no valid repositories in config") })
    }

    Ok(mirrors)
}

fn merge_similar(mut mirrors: Vec<MirrorOpts>) -> Vec<MirrorOpts> {
    for mirror in mirrors.iter_mut() {
        mirror.arch.sort();
        mirror.components.sort_unstable();
    }

    mirrors.sort();

    let merged_mirrors = mirrors.into_iter().fold(Vec::new(), |mut a: Vec<MirrorOpts>, mut v| {
        if let Some(last) = a.last_mut() {
            if last == &v {
                last.components.append(&mut v.components);
                v.debian_installer_arch.append(&mut last.debian_installer_arch);
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
    pub url: CompactString,
    pub suite: CompactString,
    pub components: Vec<CompactString>,
    pub arch: Vec<CompactString>,
    pub debian_installer_arch: Vec<CompactString>,
    pub source: bool,
}

impl Ord for MirrorOpts {
    fn cmp(&self, other: &Self) -> Ordering {
        match self.url.cmp(&other.url) {
            Ordering::Equal => {},
            ord => return ord
        }

        match self.suite.cmp(&other.suite) {
            Ordering::Equal => {}
            ord => return ord
        }

        self.arch.cmp(&other.arch)
    }
}

impl PartialEq for MirrorOpts {
    fn eq(&self, other: &Self) -> bool {
        self.url == other.url && self.suite == other.suite && self.arch == other.arch
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
                self.suite,
                self.arch.join(", "),
                self.components.join(" ")
            )
        )
    }
}

impl MirrorOpts {
    pub fn try_from(mut line: &str) -> Result<MirrorOpts> {
        let mut arch = Vec::new();
        let mut debian_installer_arch = Vec::new();
        
        let mut source = false;

        line = if let Some(line) = line.strip_prefix("deb-src") {
            source = true;
            line
        } else if let Some(line) = line.strip_prefix("deb") {
            line
        } else {
            return Err(MirsError::Config { msg: CompactString::new("mirror config must start with either 'deb' or 'deb-src'") })
        };

        line = line.trim_start();

        if line.starts_with('[') {
            let Some(bracket_end) = line.find(']') else {
                return Err(MirsError::Config { msg: CompactString::new("options bracket is not closed") })
            };

            let options_line = (&line[1..bracket_end]).trim();
            line = &line[bracket_end+1..];

            for part in options_line.split_whitespace() {
                let Some((opt_key, opt_val)) = part.split_once('=') else {
                    return Err(MirsError::Config { msg: CompactString::new("invalid format of options bracket") })
                };

                if opt_key == "arch" {
                    arch.push(opt_val.to_compact_string())
                }

                if opt_key == "di_arch" {
                    debian_installer_arch.push(opt_val.to_compact_string())
                }
            }
        }

        line = line.trim_start();

        let mut line_parts = line.split_whitespace();

        let Some(url) = line_parts.next() else {
            return Err(MirsError::Config { msg: CompactString::const_new("no url specified") })
        };

        let Some(suite) = line_parts.next() else {
            return Err(MirsError::Config { msg: CompactString::const_new("no suite specified") })
        };

        let components = line_parts
            .map(|v| v.to_compact_string())
            .collect::<Vec<_>>();

        if components.is_empty() {
            return Err(MirsError::Config { msg: CompactString::const_new("no components specified") })
        }

        if arch.is_empty() {
            arch.push("amd64".to_compact_string())
        }

        Ok(Self {
            url: url.to_compact_string(),
            suite: suite.to_compact_string(),
            components,
            arch,
            debian_installer_arch,
            source
        })
    }

    pub fn debian_installer(&self) -> bool {
        !self.debian_installer_arch.is_empty()
    }
}