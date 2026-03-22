use std::{
    collections::BTreeMap,
    fs::File,
    io::BufRead,
    sync::{Arc, atomic::AtomicU64},
};

use compact_str::{CompactString, ToCompactString};

use crate::error::{MirsError, Result};

use super::{
    IndexFileEntry, IndexFileEntryIterator, checksum::Checksum, create_reader,
    metadata_file::MetadataFile, release::FileEntry,
};

pub struct DiffIndexFile {
    pub files: BTreeMap<CompactString, FileEntry>,
    file: MetadataFile,
    size: u64,
    read: Arc<AtomicU64>,
}

impl IndexFileEntryIterator for DiffIndexFile {
    fn size(&self) -> u64 {
        self.size
    }

    fn counter(&self) -> Arc<AtomicU64> {
        self.read.clone()
    }

    fn file(&self) -> &MetadataFile {
        &self.file
    }
}

impl Iterator for DiffIndexFile {
    type Item = Result<IndexFileEntry>;

    fn next(&mut self) -> Option<Self::Item> {
        self.files.pop_first().map(|(path, value)| {
            Ok(IndexFileEntry {
                path,
                size: Some(value.size),
                checksum: value.strongest_hash(),
            })
        })
    }
}

impl DiffIndexFile {
    pub fn build(meta_file: MetadataFile) -> Result<Box<dyn IndexFileEntryIterator>> {
        let file = File::open(meta_file.path())?;
        let size = file.metadata()?.len();

        let (mut reader, counter) = create_reader(file, meta_file.path())?;

        let mut files = BTreeMap::new();

        let mut buf = String::with_capacity(1024 * 8);

        let mut in_download_scope = false;

        loop {
            buf.clear();

            let len = match reader.read_line(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) => return Err(e.into()),
            };

            let line = (buf[..len]).trim_end();

            match line {
                _ if line.ends_with("Download:") => {
                    in_download_scope = true;
                }
                _ if line.starts_with(' ') && in_download_scope => {
                    let mut split = line.split_ascii_whitespace();

                    let (Some(hash), Some(size), Some(path)) =
                        (split.next(), split.next(), split.next())
                    else {
                        return Err(MirsError::ParsingDiffIndex {
                            path: meta_file.path().to_owned(),
                        });
                    };

                    let Ok(size) = size.parse() else {
                        return Err(MirsError::ParsingDiffIndex {
                            path: meta_file.path().to_owned(),
                        });
                    };

                    if !files.contains_key(path) {
                        files.insert(
                            path.to_compact_string(),
                            FileEntry {
                                size,
                                md5: None,
                                sha1: None,
                                sha256: None,
                                sha512: None,
                            },
                        );
                    }

                    let entry = files.get_mut(path).unwrap();

                    let Ok(checksum) = Checksum::try_from(hash) else {
                        return Err(MirsError::ParsingDiffIndex {
                            path: meta_file.path().to_owned(),
                        });
                    };

                    match checksum {
                        Checksum::Md5(v) => entry.md5 = Some(v),
                        Checksum::Sha1(v) => entry.sha1 = Some(v),
                        Checksum::Sha256(v) => entry.sha256 = Some(v),
                        Checksum::Sha512(v) => entry.sha512 = Some(v),
                    }
                }
                _ => {
                    in_download_scope = false;
                }
            }
        }

        Ok(Box::new(Self {
            files,
            file: meta_file,
            size,
            read: counter,
        }))
    }
}
