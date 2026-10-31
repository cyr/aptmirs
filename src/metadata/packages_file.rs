use std::{
    fs::File,
    io::BufRead,
    sync::{Arc, atomic::AtomicU64},
};

use compact_str::{CompactString, ToCompactString};
use regex::Regex;

use crate::error::{MirsError, Result};

use super::{
    IndexFileEntry, IndexFileEntryIterator,
    checksum::{Checksum, ChecksumType},
    create_reader,
    metadata_file::MetadataFile,
};

pub struct PackagesFile {
    reader: Box<dyn BufRead + Send>,
    file: MetadataFile,
    buf: String,
    size: u64,
    read: Arc<AtomicU64>,
    filter: Arc<[Regex]>,
}

impl PackagesFile {
    pub fn build(
        meta_file: MetadataFile,
        filter: Arc<[Regex]>,
    ) -> Result<Box<dyn IndexFileEntryIterator>> {
        let file = File::open(meta_file.path())?;
        let size = file.metadata()?.len();

        let (reader, counter) = create_reader(file, meta_file.path())?;

        Ok(Box::new(Self {
            reader,
            file: meta_file,
            buf: String::with_capacity(1024 * 8),
            size,
            read: counter,
            filter,
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
                Ok(0) if self.buf.is_empty() => {
                    return None;
                }
                Ok(0..=1) => match parse_index_file_entry(&mut self.buf) {
                    Some(Ok((entry, package_name))) => {
                        if self.filter.iter().any(|r| r.is_match(&package_name)) {
                            continue;
                        }

                        return Some(Ok(entry));
                    }
                    Some(Err(e)) => return Some(Err(e)),
                    None => return None,
                },
                Ok(..) => {}
                Err(e) => {
                    return Some(Err(MirsError::ReadingPackage {
                        path: self.file.path().clone(),
                        inner: Box::new(e.into()),
                    }));
                }
            }
        }
    }
}

fn parse_index_file_entry(buf: &mut String) -> Option<Result<(IndexFileEntry, CompactString)>> {
    let mut path = None;
    let mut size = None;
    let mut hash = None;
    let mut package: Option<CompactString> = None;

    for line in buf.lines() {
        let Some((prefix, line)) = line.split_once(':') else {
            continue;
        };

        let line = line.trim();

        match prefix.trim() {
            "Filename" => {
                path = Some(line.to_compact_string());
            }
            "Size" => match line.parse() {
                Ok(v) => size = Some(v),
                Err(e) => return Some(Err(MirsError::ParseInt(e))),
            },
            "MD5Sum" if ChecksumType::is_stronger(&hash, ChecksumType::Md5) => {
                let mut md5 = [0_u8; 16];
                if let Err(e) = hex::decode_to_slice(line, &mut md5) {
                    return Some(Err(e.into()));
                }
                hash = Some(Checksum::Md5(md5));
            }
            "SHA1" if ChecksumType::is_stronger(&hash, ChecksumType::Sha1) => {
                let mut sha1 = [0_u8; 20];
                if let Err(e) = hex::decode_to_slice(line, &mut sha1) {
                    return Some(Err(e.into()));
                }
                hash = Some(Checksum::Sha1(sha1));
            }
            "SHA256" if ChecksumType::is_stronger(&hash, ChecksumType::Sha256) => {
                let mut sha256 = [0_u8; 32];
                if let Err(e) = hex::decode_to_slice(line, &mut sha256) {
                    return Some(Err(e.into()));
                }
                hash = Some(Checksum::Sha256(sha256));
            }
            "SHA512" if ChecksumType::is_stronger(&hash, ChecksumType::Sha512) => {
                let mut sha512 = [0_u8; 64];
                if let Err(e) = hex::decode_to_slice(line, &mut sha512) {
                    return Some(Err(e.into()));
                }
                hash = Some(Checksum::Sha512(sha512));
            }
            "Package" => {
                package = Some(line.to_compact_string());
            }
            _ => {}
        }
    }

    buf.clear();

    if let (Some(path), Some(size), checksum, Some(package)) = (path, size, hash, package) {
        Some(Ok((
            IndexFileEntry {
                path,
                size: Some(size),
                checksum,
            },
            package,
        )))
    } else {
        None
    }
}
