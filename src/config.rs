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

        let mut line = match reader.read_line(&mut buf).await {
            Ok(0) => break,
            Ok(len) => (buf[..len]).trim(),
            Err(e) => return Err(e.into()),
        };

        if let Some(pos) = line.find('#') {
            line = &line[..pos];
        }

        if line.is_empty() {
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
    mirrors.sort();

    mirrors.into_iter().fold(Vec::new(), |mut a: Vec<MirrorOpts>, mut new| {
        if let Some(last) = a.last_mut() {
            if last == &new {
                for component in new.components {
                    if !last.components.contains(&component) {
                        last.components.push(component);
                    }
                }

                for arch in new.arch {
                    if !last.arch.contains(&arch) {
                        last.arch.push(arch);
                    }
                }

                for di_arch in new.debian_installer_arch {
                    if !last.debian_installer_arch.contains(&di_arch) {
                        last.debian_installer_arch.push(di_arch);
                    }
                }
                
                last.udeb |= new.udeb;
                last.packages |= new.packages;
                last.source |= new.source;

                last.pgp_verify |= new.pgp_verify;
                
                if let Some(pgp_pub_key) = new.pgp_pub_key.take() {
                    last.pgp_pub_key = Some(pgp_pub_key)
                }
            } else {
                a.push(new)
            }
        } else {
            a.push(new);
        }

        a
    })
}

#[derive(Eq, Default)]
pub struct MirrorOpts {
    pub url: CompactString,
    pub suite: CompactString,
    pub components: Vec<CompactString>,
    pub arch: Vec<CompactString>,
    pub debian_installer_arch: Vec<CompactString>,
    pub source: bool,
    pub packages: bool,
    pub pgp_pub_key: Option<CompactString>,
    pub pgp_verify: bool,
    pub udeb: bool,
}

impl Ord for MirrorOpts {
    fn cmp(&self, other: &Self) -> Ordering {
        match self.url.cmp(&other.url) {
            Ordering::Equal => {},
            ord => return ord
        }

        self.suite.cmp(&other.suite)
    }
}

impl PartialEq for MirrorOpts {
    fn eq(&self, other: &Self) -> bool {
        self.url == other.url && self.suite == other.suite
    }
}

impl PartialOrd for MirrorOpts {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl MirrorOpts {
    pub fn try_from(mut line: &str) -> Result<MirrorOpts> {
        let mut arch = Vec::new();
        let mut debian_installer_arch = Vec::new();
        let mut pgp_pub_key: Option<CompactString> = None;
        let mut pgp_verify = false;
        let mut udeb = false;
        
        let mut packages = false;
        let mut source = false;

        line = if let Some(line) = line.strip_prefix("deb-src") {
            source = true;
            line
        } else if let Some(line) = line.strip_prefix("deb") {
            packages = true;
            line
        } else {
            return Err(MirsError::Config { msg: CompactString::new("mirror config must start with either 'deb' or 'deb-src'") })
        };

        line = line.trim_start();

        if line.starts_with('[') {
            let Some(bracket_end) = line.find(']') else {
                return Err(MirsError::Config { msg: CompactString::new("options bracket is not closed") })
            };

            let options_line = line[1..bracket_end].trim();
            line = &line[bracket_end+1..];

            for part in options_line.split_whitespace() {
                let Some((opt_key, opt_val)) = part.split_once('=') else {
                    return Err(MirsError::Config { msg: CompactString::new("invalid format of options bracket") })
                };

                match opt_key {
                    "arch"            => arch.extend(opt_val.split(',').map(|v|v.to_compact_string())),
                    "di_arch"         => debian_installer_arch.extend(opt_val.split(',').map(|v|v.to_compact_string())),
                    "pgp_pub_key"     => { 
                        pgp_pub_key = Some(opt_val.to_compact_string());
                        pgp_verify = true; 
                    },
                    "pgp_verify"      => pgp_verify = opt_val.to_lowercase() == "true",
                    "udeb"            => udeb = opt_val.to_lowercase() == "true",
                    _ => ()
                }

            }
        }

        line = line.trim_start();

        let mut line_parts = line.split_whitespace();

        let Some(url) = line_parts.next() else {
            return Err(MirsError::Config { msg: CompactString::const_new("no url specified") })
        };

        let url = url.strip_suffix('/').unwrap_or(url);

        let Some(suite) = line_parts.next() else {
            return Err(MirsError::Config { msg: CompactString::const_new("no suite specified") })
        };

        // we split off the path of the component name because they are not used in the release file,
        // and might be a holdover from older repository structures. debian-security uses this and the path
        // is just symlinked back to the repository root. should we support this? maybe, but probably not.
        let mut components = line_parts
            .map(|v| v.split('/').next_back().expect("last should always exist here").to_compact_string())
            .collect::<Vec<_>>();

        if components.is_empty() {
            components.push(CompactString::const_new("main"));
        }

        if arch.is_empty() {
            arch.push(CompactString::const_new("amd64"))
        }

        Ok(Self {
            url: url.to_compact_string(),
            suite: suite.to_compact_string(),
            components,
            arch,
            debian_installer_arch,
            source,
            packages,
            pgp_pub_key,
            pgp_verify,
            udeb
        })
    }

    pub fn debian_installer(&self) -> bool {
        !self.debian_installer_arch.is_empty()
    }

    pub fn flat(&self) -> bool {
        self.suite == "/"
    }

    pub fn dist_part(&self) -> CompactString {
        if self.flat() {
            CompactString::with_capacity(0)
        } else {
            format_compact!("dists/{}", self.suite)
        }
    }
}

impl Display for MirrorOpts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.packages && self.source {
            f.write_str("deb+deb-src")?
        } else if self.packages {
            f.write_str("deb")?
        } else if self.source {
            f.write_str("deb-src")?
        }

        if self.flat() {
            f.write_fmt(format_args!(
                " {} (flat)",
                self.url
            ))
        } else {
            f.write_fmt(format_args!(
                " {} {}[{}] {}",
                self.url,
                self.suite,
                self.arch.join(", "),
                self.components.join(" ")
            ))
        }
    }
}