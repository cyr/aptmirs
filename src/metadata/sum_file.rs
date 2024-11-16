use std::{fs::File, io::BufRead, sync::{atomic::AtomicU64, Arc}};

use compact_str::{CompactString, ToCompactString};

use crate::error::{MirsError, Result};

use super::{checksum::Checksum, create_reader, FilePath, IndexFileEntry, IndexFileEntryIterator};

pub struct SumFile {
    reader: Box<dyn BufRead + Send>,
    path: FilePath,
    buf: String,
    size: u64,
    read: Arc<AtomicU64>
}

impl SumFile {
    pub fn path(&self) -> &FilePath {
        &self.path
    }
    
    pub fn build(path: &FilePath) -> Result<Box<dyn IndexFileEntryIterator>> {
        let file = File::open(path)?;
        let size = file.metadata()?.len();

        let (reader, counter) = create_reader(file, path)?;

        Ok(Box::new(Self {
            reader,
            path: path.to_owned(),
            buf: String::with_capacity(1024*8),
            size,
            read: counter
        }))
    }
}

impl IndexFileEntryIterator for SumFile {
    fn size(&self) -> u64 {
        self.size
    }

    fn counter(&self) -> Arc<AtomicU64> {
        self.read.clone()
    }
    
    fn path(&self) -> &FilePath {
        &self.path
    }
}

impl Iterator for SumFile {
    type Item = Result<IndexFileEntry>;

    fn next(&mut self) -> Option<Self::Item> {
        self.buf.clear();

        let line = match self.reader.read_line(&mut self.buf) {
            Ok(0) => return None,
            Ok(size) => &self.buf[..size],
            Err(e) => return Some(Err(MirsError::SumFileParsing { 
                path: self.path().clone(), 
                inner: Box::new(e.into())
            }))
        };

        eprintln!("line is: {line}");

        let mut split = line.split_whitespace();

        let (Some(checksum_str), Some(path_str)) = (split.next(), split.next()) else {
            return Some(Err(MirsError::InvalidSumEntry { line: line.to_compact_string() } ))
        };

        let Ok(checksum) = Checksum::try_from(checksum_str) else {
            return Some(Err(MirsError::InvalidSumEntry { line: line.to_compact_string() } ))
        };

        let path = CompactString::from(path_str);

        Some(Ok(IndexFileEntry {
            path,
            size: None,
            checksum: Some(checksum),
        }))
    }
}