use std::{fs::File, io::BufRead, sync::{atomic::AtomicU64, Arc}};

use compact_str::ToCompactString;

use crate::error::{Result, MirsError};

use super::{checksum::{Checksum, ChecksumType}, create_reader, metadata_file::MetadataFile, IndexFileEntry, IndexFileEntryIterator};

pub struct PackagesFile {
    reader: Box<dyn BufRead + Send>,
    file: MetadataFile,
    buf: String,
    size: u64,
    read: Arc<AtomicU64>
}

impl PackagesFile {
    pub fn build(meta_file: MetadataFile) -> Result<Box<dyn IndexFileEntryIterator>> {
        let file = File::open(meta_file.path())?;
        let size = file.metadata()?.len();

        let (reader, counter) = create_reader(file, meta_file.path())?;

        Ok(Box::new(Self {
            reader,
            file: meta_file,
            buf: String::with_capacity(1024*8),
            size,
            read: counter,
        }))
    }
}

impl IndexFileEntryIterator for PackagesFile {
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

impl Iterator for PackagesFile {
    type Item = Result<IndexFileEntry>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match self.reader.read_line(&mut self.buf) {
                Ok(0) => return None,
                Ok(len) => {
                    if len == 1 {
                        break
                    }
                }
                Err(e) => return Some(Err(
                    MirsError::ReadingPackage { 
                        path: self.file.path().clone(), 
                        inner: Box::new(e.into()) 
                    }
                ))
            }
        }

        let mut path = None;
        let mut size = None;
        let mut hash = None;

        for line in self.buf.lines() {
            if let Some(filename) = line.strip_prefix("Filename: ") {
                path = Some(filename.to_compact_string())
            } else if let Some(line_size) = line.strip_prefix("Size: ") {
                size = Some(line_size.parse().expect("value of Size should be an integer"))
            } else if let Some(line_hash) = line.strip_prefix("MD5Sum: ") && ChecksumType::is_stronger(&hash, ChecksumType::Md5) {
                let mut md5 = [0_u8; 16];
                if let Err(e) = hex::decode_to_slice(line_hash, &mut md5) {
                    return Some(Err(e.into()))
                }   
                hash = Some(Checksum::Md5(md5))
            } else if let Some(line_hash) = line.strip_prefix("SHA1: ") && ChecksumType::is_stronger(&hash, ChecksumType::Sha1) {
                let mut sha1 = [0_u8; 20];
                if let Err(e) = hex::decode_to_slice(line_hash, &mut sha1) {
                    return Some(Err(e.into()))
                }
                hash = Some(Checksum::Sha1(sha1))
            } else if let Some(line_hash) = line.strip_prefix("SHA256: ") && ChecksumType::is_stronger(&hash, ChecksumType::Sha256) {
                let mut sha256 = [0_u8; 32];
                if let Err(e) = hex::decode_to_slice(line_hash, &mut sha256) {
                    return Some(Err(e.into()))
                }
                hash = Some(Checksum::Sha256(sha256))
            } else if let Some(line_hash) = line.strip_prefix("SHA512: ") && ChecksumType::is_stronger(&hash, ChecksumType::Sha512) {
                let mut sha512 = [0_u8; 64];
                if let Err(e) = hex::decode_to_slice(line_hash, &mut sha512) {
                    return Some(Err(e.into()))
                }
                hash = Some(Checksum::Sha512(sha512))
            }
        }

        self.buf.clear();

        if let (Some(path), Some(size), checksum) = (path, size, hash) {
            Some(Ok(IndexFileEntry {
                path,
                size: Some(size), 
                checksum
            }))
        } else {
            None
        }
    }
}