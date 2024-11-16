use std::{collections::BTreeMap, fs::File, io::BufRead, sync::{atomic::AtomicU64, Arc}};

use compact_str::{CompactString, ToCompactString};

use crate::error::{MirsError, Result};

use super::{checksum::Checksum, create_reader, metadata_file::MetadataFile, release::FileEntry, IndexFileEntry, IndexFileEntryIterator};

pub struct DiffIndexFile {
    pub files: BTreeMap<CompactString, FileEntry>,
    reader: Box<dyn BufRead + Send>,
    file: MetadataFile,
    buf: String,
    size: u64,
    read: Arc<AtomicU64>
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
        let mut in_download_scope = false;

        loop {
            self.buf.clear();

            let len = match self.reader.read_line(&mut self.buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) => return Some(Err(e.into()))
            };

            let line = (self.buf[..len]).trim_end();

            match line {
                _ if line.ends_with("Download:") => {
                    in_download_scope = true;
                },
                _ if line.starts_with(' ') && in_download_scope => {
                    let mut split = line.split_ascii_whitespace();

                    let (Some(hash), Some(size), Some(path)) = (split.next(), split.next(), split.next()) else {
                        return Some(Err(MirsError::ParsingDiffIndex { path: self.file.path().to_owned() }))
                    };

                    let Ok(size) = size.parse() else {
                        return Some(Err(MirsError::ParsingDiffIndex { path: self.file.path().to_owned() }))
                    };

                    if !self.files.contains_key(path) {
                        self.files.insert(path.to_compact_string(), 
                            FileEntry { 
                                size,
                                md5: None,
                                sha1: None,
                                sha256: None,
                                sha512: None
                            }
                        );
                    }

                    let entry = self.files.get_mut(path).unwrap();

                    let Ok(checksum) = Checksum::try_from(hash) else {
                        return Some(Err(MirsError::ParsingDiffIndex { path: self.file.path().to_owned() }))
                    };

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

        let (reader, counter) = create_reader(file, meta_file.path())?;

        Ok(Box::new(Self {
            files: BTreeMap::new(),
            reader,
            file: meta_file,
            buf: String::with_capacity(1024*8),
            size,
            read: counter,
        }))
    }
}