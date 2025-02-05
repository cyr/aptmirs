use std::{borrow::Borrow, fmt::Display, fs::Metadata, io::{BufRead, BufReader, Read}, path::{Path, PathBuf}, str::FromStr, sync::{atomic::{AtomicU64, Ordering}, Arc}};

use compact_str::{format_compact, CompactString, ToCompactString};
use metadata_file::MetadataFile;

use crate::error::{Result, MirsError};

use self::checksum::Checksum;

pub mod release;
pub mod packages_file;
pub mod sources_file;
pub mod checksum;
pub mod diff_index_file;
pub mod sum_file;
pub mod repository;
pub mod metadata_file;

#[derive(Clone, Debug, PartialEq, PartialOrd, Eq, Ord, Default, Hash)]
pub struct FilePath(pub CompactString);

impl Borrow<str> for FilePath {
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

impl FromStr for FilePath {
    type Err = MirsError;

    fn from_str(s: &str) -> std::prelude::v1::Result<Self, Self::Err> {
        Ok(FilePath(s.to_compact_string()))
    }
}

impl From<&str> for FilePath {
    fn from(value: &str) -> Self {
        Self(CompactString::from(value))
    }
}

impl From<PathBuf> for FilePath {
    fn from(value: PathBuf) -> Self {
        Self(CompactString::from(value.into_os_string().into_string().expect("file paths should be utf8")))
    }
}

impl From<&Path> for FilePath {
    fn from(value: &Path) -> Self {
        Self(value.as_os_str().to_str().expect("file paths should be utf8").to_compact_string())
    }
}

impl AsRef<FilePath> for FilePath {
    fn as_ref(&self) -> &FilePath {
        self
    }
}

impl AsRef<Path> for FilePath {
    fn as_ref(&self) -> &Path {
        self.0.as_str().as_ref()
    }
}

impl AsRef<str> for FilePath {
    fn as_ref(&self) -> &str {
        self.0.as_str()
    }
}

impl Display for FilePath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl FilePath {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn file_stem(&self) -> &str {
        let p: &Path = self.as_ref();

        p.file_stem()
            .expect("a FilePath should have a filename")
            .to_str()
            .expect("a FilePath name should be utf8")
    }

    pub fn file_name(&self) -> &str {
        let p: &Path = self.as_ref();

        p.file_name()
            .expect("a FilePath should have a filename")
            .to_str()
            .expect("a FilePath name should be utf8")
    }

    pub fn exists(&self) -> bool {
        self.metadata().is_ok()
    }

    pub async fn symlink_path(&self) -> Result<Option<FilePath>> {
        let metadata = tokio::fs::symlink_metadata(&self.0).await?;

        if metadata.is_symlink() {
            let link = tokio::fs::read_link(&self.0).await?;

            Ok(Some(FilePath::from(link)))
        } else {
            Ok(None)
        }
    }

    pub fn extension(&self) -> Option<&str> {
        let p: &Path = self.as_ref();

        p.extension().map(|v| v.to_str().expect("path extensions should be utf8"))
    }

    pub fn metadata(&self) -> std::result::Result<Metadata, std::io::Error> {
        let p: &Path = self.as_ref();

        p.metadata()
    }

    pub fn parent(&self) -> Option<&str> {
        let split_iter = self.0.rsplit_once('/')?;
        
        Some(split_iter.0)
    }

    pub fn join<T: AsRef<str>>(&self, other: T) -> FilePath {
        let first = match self.0.strip_suffix('/') {
            Some(s) => s,
            None => &self.0,
        };

        let other = other.as_ref();

        let other = match other.strip_prefix('/') {
            Some(s) => s,
            None => match other.strip_prefix("./") {
                Some(s) => s,
                None => other,
            }
        };

        if first.is_empty() {
            return FilePath::from(other)
        }

        FilePath(format_compact!("{first}/{other}"))
    }
}

pub trait IndexFileEntryIterator : Iterator<Item = Result<IndexFileEntry>> + Send {
    fn size(&self) -> u64;
    fn counter(&self) -> Arc<AtomicU64>;
    fn file(&self) -> &MetadataFile;
}

#[derive(Debug)]
pub struct IndexFileEntry {
    pub path: CompactString,
    pub size: Option<u64>,
    pub checksum: Option<Checksum>
}
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

pub fn create_reader<R: Read + Send + 'static>(file: R, path: &FilePath) -> Result<(Box<dyn BufRead + Send>, Arc<AtomicU64>)> {
    let counter = Arc::new(AtomicU64::from(0));

    let file_reader = TrackingReader {
        inner: file,
        read: counter.clone(),
    };

    let reader: Box<dyn BufRead + Send> = match path.extension() {
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
        _ => return Err(MirsError::ParsingPackages { path: path.to_owned() })
    };

    Ok((reader, counter))
}
