use std::{io::{Read, BufRead, BufReader}, sync::{Arc, atomic::{AtomicU64, Ordering}}, path::{Path, PathBuf}};

use crate::error::{Result, MirsError};

use self::{checksum::Checksum, packages_file::PackagesFile, sources_file::SourcesFile};

pub mod release;
pub mod packages_file;
pub mod sources_file;
pub mod checksum;
pub mod diff_index_file;

pub enum IndexSource {
    Packages(PathBuf),
    Sources(PathBuf)
}

impl IndexSource {
    pub fn into_reader(self) -> Result<Box<dyn IndexFileEntryIterator>> {
        match self {
            IndexSource::Packages(path) => PackagesFile::build(&path),
            IndexSource::Sources(path) => SourcesFile::build(&path),
        }
    }
}

impl From<PathBuf> for IndexSource {
    fn from(value: PathBuf) -> Self {
        match value.file_name().expect("indices should have names")
            .to_str().expect("the file name of indices should be valid utf8") {
            v if v.starts_with("Packages") => IndexSource::Packages(value),
            v if v.starts_with("Sources") => IndexSource::Sources(value),
            _ => unreachable!("implementation error; non-index file as IndexSource")
        }
    }
}

pub trait IndexFileEntryIterator : Iterator<Item = Result<IndexFileEntry>> {
    fn size(&self) -> u64;
    fn counter(&self) -> Arc<AtomicU64>;
}

pub struct IndexFileEntry {
    pub path: String,
    pub size: u64,
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

pub fn create_reader<R: Read + 'static>(file: R, path: &Path) -> Result<(Box<dyn BufRead>, Arc<AtomicU64>)> {
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
        _ => return Err(MirsError::ParsingPackages { path: path.to_owned() })
    };

    Ok((reader, counter))
}