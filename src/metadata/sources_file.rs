use std::{fs::File, io::BufRead, sync::{atomic::AtomicU64, Arc}, collections::BTreeMap};

use compact_str::{format_compact, CompactString};

use crate::error::{Result, MirsError};

use super::{checksum::Checksum, create_reader, FilePath, IndexFileEntry, IndexFileEntryIterator};

pub struct SourceEntry {
    pub size: u64,
    pub checksum: Checksum
}

pub struct SourcesFile {
    reader: Box<dyn BufRead + Send>,
    path: FilePath,
    buf: String,
    files_buf: BTreeMap<CompactString, SourceEntry>,
    size: u64,
    read: Arc<AtomicU64>
}

impl SourcesFile {
    pub fn build(path: &FilePath) -> Result<Box<dyn IndexFileEntryIterator>> {
        let file = File::open(path)?;
        let size = file.metadata()?.len();

        let (reader, counter) = create_reader(file, path)?;

        Ok(Box::new(Self {
            reader,
            path: path.to_owned(),
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
}

impl Iterator for SourcesFile {
    type Item = Result<IndexFileEntry>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.files_buf.is_empty() {
            loop {
                match self.reader.read_line(&mut self.buf) {
                    Ok(0) => return None,
                    Ok(1) => break,
                    Ok(_) => (),
                    Err(e) => return Some(Err(
                        MirsError::ReadingPackage { 
                            path: self.path.clone(), 
                            inner: Box::new(e.into()) 
                        }
                    ))
                }
            }
    
            let mut dir = None;
    
            let mut line_iter = self.buf.lines().peekable();
    
            while let Some(line) = line_iter.next() {
                if let Some(d) = line.strip_prefix("Directory: ") {
                    dir = Some(d)
                } else if matches!(line, "Files:" | "Checksums-Sha1:" | "Checksums-Sha256:" | "Checksums-Sha512:") {
                    while let Some(line) = line_iter.next() {
                        let mut parts = line.split_whitespace();

                        let Some(checksum_part) = parts.next() else {
                            return Some(Err(MirsError::ParsingSources { path: self.path.clone() }))
                        };
    
                        let checksum = match Checksum::try_from(checksum_part) {
                            Ok(v) => v,
                            Err(e) => return Some(Err(e))
                        };
                        
                        let Some(size_part) = parts.next() else {
                            return Some(Err(MirsError::ParsingSources { path: self.path.clone() }))
                        };
    
                        let size: u64 = match size_part.parse() {
                            Ok(v) => v,
                            Err(e) => return Some(Err(e.into())),
                        };
    
                        let Some(file_name) = parts.next() else {
                            return Some(Err(MirsError::ParsingSources { path: self.path.clone() }))
                        };
    
                        let rel_path = if let Some(dir) = dir {
                            format_compact!("{dir}/{file_name}")
                        } else {
                            return Some(Err(MirsError::ParsingSources { path: self.path.clone() }))
                        };
                        
                        if let Some(entry) = self.files_buf.get_mut(&rel_path) {
                            entry.checksum.replace_if_stronger(checksum)
                        } else {
                            self.files_buf.insert(rel_path, SourceEntry {
                                size,
                                checksum
                            });
                        }

                        if line_iter.peek().is_some_and(|v| !v.starts_with(' ')) {
                            _ = line_iter.next();
                            break
                        }
                    }
                }
            }

            self.buf.clear();
        }

        if let Some((path, entry)) = self.files_buf.pop_first() {
            return Some(Ok(IndexFileEntry {
                path,
                size: entry.size,
                checksum: Some(entry.checksum)
            }))
        }

        None
    }
}