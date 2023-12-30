use std::{path::{Path, PathBuf}, fs::File, io::{BufReader, BufRead, Read}, sync::{atomic::{AtomicU64, Ordering}, Arc}};

use crate::error::{Result, MirsError};

use super::checksum::{Checksum, ChecksumType};

pub struct TrackingReader<R: Read> {
    inner: R,
    read: Arc<AtomicU64>
}

impl<R: Read> Read for TrackingReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let result = self.inner.read(buf);

        if let Ok(read) = result {
            self.read.fetch_add(read as u64, Ordering::SeqCst);
        }

        result
    }
}

pub struct Package {
    reader: Box<dyn BufRead>,
    path: PathBuf,
    buf: String,
    size: u64,
    read: Arc<AtomicU64>
}

impl Package {
    pub fn build(path: &Path) -> Result<Self> {
        let file = File::open(path)?;
        let size = file.metadata()?.len();

        let counter = Arc::new(AtomicU64::from(0));

        let file_reader = TrackingReader {
            inner: file,
            read: counter.clone(),
        };

        let reader: Box<dyn BufRead> = match path.extension()
            .map(|v|
                v.to_str().expect("extension must be valid ascii")
            ) {
            Some("xz") => {
                let xz_decoder = xz2::read::XzDecoder::new_multi_decoder(file_reader);
                Box::new(BufReader::with_capacity(1024*1024, xz_decoder))
            }
            Some("gz") => {
                let gz_decoder = flate2::read::GzDecoder::new(file_reader);
                Box::new(BufReader::with_capacity(1024*1024, gz_decoder))
            },
            Some("bz2") => {
                let bz2_decoder = bzip2::read::BzDecoder::new(file_reader);
                Box::new(BufReader::with_capacity(1024*1024, bz2_decoder))
            },
            None => {
                Box::new(BufReader::with_capacity(1024*1024, file_reader))
            },
            _ => return Err(MirsError::ParsingPackage { path: path.to_owned() })
        };

        Ok(Self {
            reader,
            path: path.to_path_buf(),
            buf: String::with_capacity(1024*8),
            size,
            read: counter,
        })
    }

    pub fn size(&self) -> u64 {
        self.size
    }

    pub fn counter(&self) -> Arc<AtomicU64> {
        self.read.clone()
    }
}

impl Iterator for Package {
    type Item = Result<(String, u64, Option<Checksum>)>;

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
                        path: self.path.to_string_lossy().to_string(), 
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
                path = Some(filename.to_string())
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

        if let (Some(p), Some(s), h) = (path, size, hash) {
            Some(Ok((p, s, h)))
        } else {
            None
        }
    }
}