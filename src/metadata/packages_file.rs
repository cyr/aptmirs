use std::{collections::BTreeMap, fs::File, io::BufRead, sync::{atomic::AtomicU64, Arc}};

use compact_str::{format_compact, ToCompactString};

use crate::error::{Result, MirsError};

use super::{checksum::{Checksum, ChecksumType}, create_reader, FilePath, IndexFileEntry, IndexFileEntryIterator};

pub struct PackagesFile {
    reader: Box<dyn BufRead + Send>,
    path: FilePath,
    buf: String,
    size: u64,
    read: Arc<AtomicU64>
}

impl PackagesFile {
    pub fn build(path: &FilePath) -> Result<Box<dyn IndexFileEntryIterator>> {
        let file = File::open(path)?;
        let size = file.metadata()?.len();

        let (reader, counter) = create_reader(file, path)?;

        Ok(Box::new(Self {
            reader,
            path: path.to_owned(),
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
    
    fn path(&self) -> &FilePath {
        &self.path
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
                        path: self.path.clone(), 
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
            } else if let Some(line_hash) = line.strip_prefix("MD5Sum: ") {
                if ChecksumType::is_stronger(&hash, ChecksumType::Md5) {
                    let mut md5 = [0_u8; 16];
                    if let Err(e) = hex::decode_to_slice(line_hash, &mut md5) {
                        return Some(Err(e.into()))
                    }   
                    hash = Some(Checksum::Md5(md5))
                }
            } else if let Some(line_hash) = line.strip_prefix("SHA1: ") {
                if ChecksumType::is_stronger(&hash, ChecksumType::Sha1) {
                    let mut sha1 = [0_u8; 20];
                    if let Err(e) = hex::decode_to_slice(line_hash, &mut sha1) {
                        return Some(Err(e.into()))
                    }
                    hash = Some(Checksum::Sha1(sha1))
                }
            } else if let Some(line_hash) = line.strip_prefix("SHA256: ") {
                if ChecksumType::is_stronger(&hash, ChecksumType::Sha256) {
                    let mut sha256 = [0_u8; 32];
                    if let Err(e) = hex::decode_to_slice(line_hash, &mut sha256) {
                        return Some(Err(e.into()))
                    }
                    hash = Some(Checksum::Sha256(sha256))
                }
            } else if let Some(line_hash) = line.strip_prefix("SHA512: ") {
                if ChecksumType::is_stronger(&hash, ChecksumType::Sha512) {
                    let mut sha512 = [0_u8; 64];
                    if let Err(e) = hex::decode_to_slice(line_hash, &mut sha512) {
                        return Some(Err(e.into()))
                    }
                    hash = Some(Checksum::Sha512(sha512))
                }
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

pub fn into_filtered_by_extension<T: AsRef<FilePath>>(list: &mut Vec<T>) -> Vec<T> {
    let mut existing_indices = BTreeMap::<FilePath, T>::new();

    while let Some(index_file_path) = list.pop() {
        let file_path = index_file_path.as_ref();

        let file_stem = file_path.file_stem();
        let path_with_stem = FilePath(format_compact!(
            "{}/{}", 
            file_path.parent().unwrap(), 
            file_stem
        ));

        if let Some(val) = existing_indices.get_mut(&path_with_stem) {
            let file_path = val.as_ref();
            if is_extension_preferred(file_path.extension(), file_path.extension()) {
                *val = index_file_path
            }
        } else {
            existing_indices.insert(path_with_stem, index_file_path);
        }
    }
    
    existing_indices.into_values().collect()
}

fn is_extension_preferred(old: Option<&str>, new: Option<&str>) -> bool {
    matches!((old, new),
        (_, Some("gz")) |
        (_, Some("xz")) |
        (_, Some("bz2")) 
    )
}