use std::{path::Path, fs::File, io::{BufReader, BufRead, Read}, sync::{atomic::{AtomicU64, Ordering}, Arc}};

use crate::error::{Result, MirsError};

pub struct TrackingReader<R: Read> {
    inner: R,
    read: Arc<AtomicU64>
}

impl<R: Read> Read for TrackingReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self.inner.read(buf) {
            Ok(read) => {
                self.read.fetch_add(read as u64, Ordering::SeqCst);
                Ok(read)
            },
            Err(e) => Err(e),
        }
    }
}

pub struct Package {
    reader: Box<dyn BufRead>,
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
                let xz_decoder = xz2::read::XzDecoder::new(file_reader);
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
    type Item = Result<(String, u64)>;

    fn next(&mut self) -> Option<Self::Item> {
        let mut path = None;
        let mut size = None;

        loop {
            match self.reader.read_line(&mut self.buf) {
                Ok(_) if self.buf.starts_with("Filename: ") => 
                    path = Some(
                        String::from(
                            self.buf.strip_prefix("Filename: ")
                                .expect("prefix is guaranteed to be here")
                                .trim()
                        )
                    ),
                Ok(_) if self.buf.starts_with("Size: ") => {
                    let value = self.buf.strip_prefix("Size: ")
                        .expect("prefix is guaranteed to be here!")
                        .trim();

                    let value = value.parse()
                        .expect("value of Size should be an integer");

                    size = Some(value)
                },
                Ok(0) => return None,
                Ok(_) => (),
                Err(e) => {
                    return Some(Err(MirsError::Io(e)))
                }
            };

            self.buf.clear();

            if path.is_some() && size.is_some() {
                break
            }
        }

        if let (Some(p), Some(s)) = (path, size) {
            return Some(Ok((p, s)))
        }

        None
    }
}