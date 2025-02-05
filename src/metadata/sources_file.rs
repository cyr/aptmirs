use std::{collections::BTreeMap, fs::File, io::BufRead, sync::{atomic::AtomicU64, Arc}};

use compact_str::{format_compact, CompactString, ToCompactString};

use crate::error::{Result, MirsError};

use super::{checksum::Checksum, create_reader, metadata_file::MetadataFile, IndexFileEntry, IndexFileEntryIterator};

pub struct SourceEntry {
    pub size: u64,
    pub checksum: Checksum
}

pub struct SourcesFile {
    reader: Box<dyn BufRead + Send>,
    file: MetadataFile,
    buf: String,
    files_buf: BTreeMap<CompactString, SourceEntry>,
    size: u64,
    read: Arc<AtomicU64>
}

impl SourcesFile {
    pub fn build(meta_file: MetadataFile) -> Result<Box<dyn IndexFileEntryIterator>> {
        let file = File::open(meta_file.path())?;
        let size = file.metadata()?.len();

        let (reader, counter) = create_reader(file, meta_file.path())?;

        Ok(Box::new(Self {
            reader,
            file: meta_file,
            buf: String::with_capacity(1024*8),
            files_buf: BTreeMap::new(),
            size,
            read: counter,
        }))
    }
}

impl IndexFileEntryIterator for SourcesFile {
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

impl Iterator for SourcesFile {
    type Item = Result<IndexFileEntry>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.files_buf.is_empty() {
            let mut maybe_dir = None;

            loop {
                match self.reader.read_line(&mut self.buf) {
                    Ok(0) => return None,
                    Ok(1) => break,
                    Ok(_) => (),
                    Err(e) => return Some(Err(
                        MirsError::ReadingPackage { 
                            path: self.file.path().clone(), 
                            inner: Box::new(e.into()) 
                        }
                    ))
                }
            }
    
            let mut line_iter = self.buf.lines().peekable();
    
            while let Some(line) = line_iter.next() {
                if let Some(d) = line.strip_prefix("Directory: ") {
                    maybe_dir = Some(d)
                } else if matches!(line, "Files:" | "Checksums-Sha1:" | "Checksums-Sha256:" | "Checksums-Sha512:") {
                    while let Some(line) = line_iter.next() {
                        let mut parts = line.split_whitespace();

                        let Some(checksum_part) = parts.next() else {
                            return Some(Err(MirsError::ParsingSources { path: self.file.path().clone() }))
                        };
    
                        let checksum = match Checksum::try_from(checksum_part) {
                            Ok(v) => v,
                            Err(e) => return Some(Err(e))
                        };
                        
                        let Some(size_part) = parts.next() else {
                            return Some(Err(MirsError::ParsingSources { path: self.file.path().clone() }))
                        };
    
                        let size: u64 = match size_part.parse() {
                            Ok(v) => v,
                            Err(e) => return Some(Err(e.into())),
                        };
    
                        let Some(file_name) = parts.next() else {
                            return Some(Err(MirsError::ParsingSources { path: self.file.path().clone() }))
                        };

                        let file_name = CompactString::from(file_name);
                        
                        if let Some(entry) = self.files_buf.get_mut(&file_name) {
                            entry.checksum.replace_if_stronger(checksum)
                        } else {
                            self.files_buf.insert(file_name.to_compact_string(), SourceEntry {
                                size,
                                checksum
                            });
                        }

                        if line_iter.peek().is_some_and(|v| !v.starts_with(' ')) {
                            break
                        }
                    }
                }
            }

            let Some(dir) = maybe_dir else {
                return Some(Err(MirsError::ParsingSources { path: self.file.path().clone() }))
            };

            let mut new_map = BTreeMap::new();

            while let Some((file_name, entry)) = self.files_buf.pop_first() {
                new_map.insert(format_compact!("{dir}/{file_name}"), entry);
            }

            self.files_buf = new_map;

            self.buf.clear();
        }

        if let Some((path, entry)) = self.files_buf.pop_first() {
            return Some(Ok(IndexFileEntry {
                path,
                size: Some(entry.size),
                checksum: Some(entry.checksum)
            }))
        }

        None
    }
}