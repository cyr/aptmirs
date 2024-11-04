use std::collections::BTreeMap;

use compact_str::{CompactString, ToCompactString};
use tokio::{fs::File, io::{AsyncBufReadExt, BufReader}};

use crate::error::{MirsError, Result};

use super::{checksum::Checksum, release::FileEntry, FilePath};

pub struct DiffIndexFile {
    pub files: BTreeMap<CompactString, FileEntry>
}

impl DiffIndexFile {
    pub async fn parse(path: &FilePath) -> Result<DiffIndexFile> {
        let file = File::open(path).await?;
        let file_size = file.metadata().await?.len();

        let mut files = BTreeMap::new();

        let reader_capacity = file_size.min(1024*1024) as usize;
        let buf_capacity = reader_capacity.min(1024*8*8);

        let mut buf = String::with_capacity(buf_capacity);
        let mut reader = BufReader::with_capacity(reader_capacity, file);   
        
        let mut in_download_scope = false;

        loop {
            buf.clear();

            let len = match reader.read_line(&mut buf).await {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) => return Err(e.into())
            };

            let line = (buf[..len]).trim_end();

            match line {
                _ if line.ends_with("Download:") => {
                    in_download_scope = true;
                },
                _ if line.starts_with(' ') && in_download_scope => {
                    let mut split = line.split_ascii_whitespace();

                    let (Some(hash), Some(size), Some(path)) = (split.next(), split.next(), split.next()) else {
                        return Err(MirsError::ParsingDiffIndex { path: path.to_owned() })
                    };

                    let size = size.parse()?;

                    if !files.contains_key(path) {
                        files.insert(path.to_compact_string(), 
                            FileEntry { 
                                size,
                                md5: None,
                                sha1: None,
                                sha256: None,
                                sha512: None
                            }
                        );
                    }

                    let entry = files.get_mut(path).unwrap();

                    let checksum = Checksum::try_from(hash)?;

                    match checksum {
                        Checksum::Md5(v) => entry.md5 = Some(v),
                        Checksum::Sha1(v) => entry.sha1 = Some(v),
                        Checksum::Sha256(v) => entry.sha256 = Some(v),
                        Checksum::Sha512(v) => entry.sha512 = Some(v),
                    }
                },
                _ => {
                    in_download_scope = false;
                }
            }
        }

        Ok(Self {
            files
        })
    }
}