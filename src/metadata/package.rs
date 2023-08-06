use std::{path::{Path, PathBuf}, fs::File, io::{BufReader, BufRead}};

use crate::error::{Result, MirsError};

pub struct Package {
    reader: Box<dyn BufRead>,
    buf: String
}

impl Package {
    pub fn build(path: &Path) -> Result<Self> {
        let file = File::open(path)?;

        let reader: Box<dyn BufRead> = match path.extension()
            .map(|v|
                v.to_str().expect("extension must be valid ascii")
            ) {
            Some("xz") => {
                let xz_decoder = xz2::read::XzDecoder::new(file);
                Box::new(BufReader::with_capacity(1024*1024, xz_decoder))
            }
            Some("gz") => {
                let gz_decoder = flate2::read::GzDecoder::new(file);
                Box::new(BufReader::with_capacity(1024*1024, gz_decoder))
            },
            None => {
                Box::new(BufReader::with_capacity(1024*1024, file))
            },
            _ => return Err(MirsError::ParsingPackage { path: path.to_owned() })
        };

        Ok(Self {
            reader,
            buf: String::with_capacity(1024*8)
        })
    }
}

impl Iterator for Package {
    type Item = Result<(PathBuf, u64)>;

    fn next(&mut self) -> Option<Self::Item> {
        let mut path = None;
        let mut size = None;

        loop {
            match self.reader.read_line(&mut self.buf) {
                Ok(_) if self.buf.starts_with("Filename: ") => 
                    path = Some(PathBuf::from(self.buf.strip_prefix("Filename: ").expect("prefix is guaranteed to be here").trim())),
                Ok(_) if self.buf.starts_with("Size: ") => {
                    let value = self.buf.strip_prefix("Size: ").expect("prefix is guaranteed to be here!").trim();

                    let value = value.parse().expect("value of Size should be an integer");

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