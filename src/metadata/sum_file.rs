use std::{cmp::Ordering, fs::File, io::{BufRead, BufReader}, str::FromStr};

use compact_str::{CompactString, ToCompactString};

use crate::error::{MirsError, Result};

use super::{checksum::Checksum, FilePath};

pub enum SumFile {
    Md5(FilePath),
    Sha1(FilePath),
    Sha256(FilePath),
    Sha512(FilePath)
}

impl SumFile {
    pub fn path(&self) -> &FilePath {
        match self {
            SumFile::Md5(file_path) |
            SumFile::Sha1(file_path) |
            SumFile::Sha256(file_path) |
            SumFile::Sha512(file_path) => file_path
        }
    }

    pub fn try_into_iter(self) -> Result<SumFileIterator> {
        SumFileIterator::new(self)
    }
}

impl TryFrom<FilePath> for SumFile {
    type Error = MirsError;
    
    fn try_from(value: FilePath) -> Result<Self> {
        let Some(name) = value.file_name() else {
            return Err(MirsError::UnrecognizedSumFile { path: value })
        };

        let name = name.to_str().expect("file names need to be valid utf8");

        let file = match name {
            "MD5SUMS"    => SumFile::Md5(value),
            "SHA1SUMS"   => SumFile::Sha1(value),
            "SHA256SUMS" => SumFile::Sha256(value),
            "SHA512SUMS" => SumFile::Sha512(value),
            _ => return Err(MirsError::UnrecognizedSumFile { path: value })
        };

        Ok(file)
    }
}

impl PartialEq for SumFile {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (SumFile::Md5(_), SumFile::Md5(_)) => true,
            (SumFile::Md5(_), _) => false,
            (SumFile::Sha1(_), SumFile::Sha1(_)) => true,
            (SumFile::Sha1(_), _) => false,
            (SumFile::Sha256(_), SumFile::Sha256(_)) => true,
            (SumFile::Sha256(_), _) => false,
            (SumFile::Sha512(_), SumFile::Sha512(_)) => true,
            (SumFile::Sha512(_), _) => false,
        }
    }
}

impl PartialOrd for SumFile {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        match (self, other) {
            (SumFile::Md5(_),    SumFile::Md5(_))    => Some(Ordering::Equal),
            (SumFile::Md5(_),    _)                  => Some(Ordering::Less),
            (SumFile::Sha1(_),   SumFile::Md5(_))    => Some(Ordering::Greater),
            (SumFile::Sha1(_),   SumFile::Sha1(_))   => Some(Ordering::Equal),
            (SumFile::Sha1(_),   _)                  => Some(Ordering::Less),
            (SumFile::Sha256(_), SumFile::Sha256(_)) => Some(Ordering::Equal),
            (SumFile::Sha256(_), SumFile::Sha512(_)) => Some(Ordering::Less),
            (SumFile::Sha256(_), _)                  => Some(Ordering::Greater),
            (SumFile::Sha512(_), SumFile::Sha512(_)) => Some(Ordering::Equal),
            (SumFile::Sha512(_), _)                  => Some(Ordering::Greater),
        }
    }
}

pub fn to_strongest_by_checksum(mut di_indices: Vec<FilePath>) -> Result<Vec<SumFile>> {
    di_indices.sort_by(|a, b| a.parent().cmp(&b.parent()));
    
    let mut sum_files = Vec::with_capacity(di_indices.len());

    for file in di_indices.into_iter() {
        sum_files.push(SumFile::try_from(file)?);
    }

    let deduped = sum_files.into_iter().fold(Vec::new(), |mut list: Vec<SumFile>, v| {
        if let Some(last) = list.last_mut() {
            if last.path().parent().expect("should have parent") == v.path().parent().expect("should have parent") {
                if v > *last {
                    *last = v;
                }
            } else {
                list.push(v)
            }
        } else {
            list.push(v)
        }

        list
    });

    Ok(deduped)
}

pub struct SumFileEntry {
    pub checksum: Checksum,
    pub path: CompactString
}

impl TryFrom<&str> for SumFileEntry {
    type Error = MirsError;

    fn try_from(value: &str) -> std::result::Result<Self, Self::Error> {
        let mut split = value.split_whitespace();

        let Some(checksum_str) = split.next() else {
            return Err(MirsError::InvalidSumEntry { line: value.to_compact_string() } )
        };

        let Some(path_str) = split.next() else {
            return Err(MirsError::InvalidSumEntry { line: value.to_compact_string() } )
        };

        let checksum = Checksum::try_from(checksum_str)?;
        let path = CompactString::from_str(path_str)
            .expect("str should always convert into compactstring");

        Ok(SumFileEntry {
            checksum,
            path
        })
    }
}

pub struct SumFileIterator {
    pub sum_file: SumFile,
    pub reader: BufReader<File>,
    buf: String
}

impl SumFileIterator {
    pub fn new(sum_file: SumFile) -> Result<Self> {
        let file = File::open(sum_file.path())?;

        let reader = BufReader::with_capacity(1024*1024, file);

        Ok(Self {
            sum_file,
            reader,
            buf: String::with_capacity(1024*1024)
        })
    }
}

impl Iterator for SumFileIterator {
    type Item = Result<SumFileEntry>;

    fn next(&mut self) -> Option<Self::Item> {
        let line = match self.reader.read_line(&mut self.buf) {
            Ok(0) => return None,
            Ok(size) => &self.buf[..size],
            Err(e) => return Some(Err(MirsError::SumFileParsing { 
                path: self.sum_file.path().clone(), 
                inner: Box::new(e.into())
            }))
        };

        match SumFileEntry::try_from(line) {
            Ok(entry) => {
                self.buf.clear();

                Some(Ok(entry))
            },
            Err(e) => return Some(Err(MirsError::SumFileParsing { 
                path: self.sum_file.path().clone(), 
                inner: Box::new(e)
            }))
        }
    }
}