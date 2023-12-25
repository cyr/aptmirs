use std::{path::{Path, Component}, collections::{BTreeMap, BTreeSet}};

use tokio::{fs::File, io::{BufReader, AsyncBufReadExt}};

use crate::{error::{Result, MirsError}, config::MirrorOpts};

#[derive(Debug)]
pub struct Release {
    map: BTreeMap<String, String>,
    pub files: BTreeMap<String, FileEntry>
}

impl Release {
    pub async fn parse(path: &Path) -> Result<Release> {
        let file = File::open(path).await?;
        let file_size = file.metadata().await?.len();

        let reader_capacity = file_size.min(1024*1024) as usize;
        let buf_capacity = reader_capacity.min(1024*8*8);

        let mut buf = String::with_capacity(buf_capacity);
        let mut reader = BufReader::with_capacity(reader_capacity, file);        

        let mut checksum_state = ChecksumState::No;

        let mut map = BTreeMap::new();
        let mut files = BTreeMap::<String, FileEntry>::new();

        loop {
            buf.clear();

            let len = match reader.read_line(&mut buf).await {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) => return Err(e.into())
            };

            let line: Line = (buf[..len]).trim_end().into();

            match line {
                Line::FileEntry(v) => {
                    let file_line = FileLine::parse(v)?;

                    if !files.contains_key(file_line.path) {
                        files.insert(file_line.path.to_string(), 
                            FileEntry { 
                                size: file_line.size,
                                md5: None,
                                sha1: None,
                                sha256: None,
                                sha512: None
                            }
                        );
                    }

                    let entry = files.get_mut(file_line.path).unwrap();
                        
                    match checksum_state {
                        ChecksumState::Sha1   => entry.sha1 = Some(file_line.checksum.to_string()),
                        ChecksumState::Sha256 => entry.sha256 = Some(file_line.checksum.to_string()),
                        ChecksumState::Sha512 => entry.sha512 = Some(file_line.checksum.to_string()),
                        ChecksumState::Md5    => entry.md5 = Some(file_line.checksum.to_string()),
                        ChecksumState::No     => return Err(MirsError::ParsingRelease { line: v.to_string() }),
                        _                     => continue
                    };
                },
                Line::Metadata(v) => {
                    checksum_state = ChecksumState::No;

                    match checksum_state {
                        ChecksumState::PgpMessage | ChecksumState::PgpSignature => {
                            continue
                        },
                        _ => ()
                    }

                    let (k, v) = v.split_once(": ")
                        .ok_or_else(|| MirsError::ParsingRelease { line: v.to_string() })?;

                    map.insert(k.to_string(), v.to_string());
                },
                Line::Md5Start                 => checksum_state = ChecksumState::Md5,
                Line::Sha1Start                => checksum_state = ChecksumState::Sha1,
                Line::Sha256Start              => checksum_state = ChecksumState::Sha256,
                Line::Sha512Start              => checksum_state = ChecksumState::Sha512,
                Line::UnknownChecksumStart     => checksum_state = ChecksumState::Unknown,
                Line::PGPSignedMessageStart    => checksum_state = ChecksumState::PgpMessage,
                Line::PGPSignatureStart        => checksum_state = ChecksumState::PgpSignature,
                Line::PGPSignatureEnd          => checksum_state = ChecksumState::No,
                Line::UnknownLine(_)           => continue,
            }
        }

        Ok(Release {
            map,
            files
        })
    }

    pub fn acquire_by_hash(&self) -> bool {
        self.map.get("Acquire-By-Hash")
            .map(|v|v == "yes")
            .unwrap_or(false)
    }

    pub fn into_filtered_files(self, opts: &MirrorOpts) -> ReleaseFileIterator {
        ReleaseFileIterator::new(self, opts)
    }
}

pub struct ReleaseFileIterator<'a> {
    release: Release,
    opts: &'a MirrorOpts,
    file_prefix_filter: Vec<String>,
    dir_filter: BTreeSet<String>
}

impl<'a> ReleaseFileIterator<'a> {
    pub fn new(release: Release, opts: &'a MirrorOpts) -> Self {
        let mut file_prefix_filter = Vec::from([
            String::from("Release"),
            String::from("Contents-all"),
            String::from("Components-all"),
            String::from("Commands-all"),
            String::from("Packages"),
            String::from("icons"),
            String::from("Translation"),
            String::from("Sources"),
            String::from("Index"),
        ]);
        
        let mut dir_filter = BTreeSet::from([
            String::from("dep11"),
            String::from("i18n"),
            String::from("binary-all"),
            String::from("cnf"),
            String::from("Contents-all.diff"),
            String::from("Packages.diff"),
        ]);

        for arch in &opts.arch {
            dir_filter.insert(format!("binary-{arch}"));
            dir_filter.insert(format!("Contents-{arch}.diff"));
            //dir_filter.insert(format!("Translation-{lang}.diff")); ?? Translation-en.diff

            file_prefix_filter.push(format!("Components-{arch}"));
            file_prefix_filter.push(format!("Contents-{arch}"));
            file_prefix_filter.push(format!("Commands-{arch}"));
        }

        Self {
            release,
            opts,
            file_prefix_filter,
            dir_filter
        }
    }
}

impl<'a> Iterator for ReleaseFileIterator<'a> {
    type Item = (String, FileEntry);

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match self.release.files.pop_first() {
                Some((path, file_entry)) => {
                    let p = Path::new(&path);

                    let mut parts = p.components().peekable();

                    let Some(Component::Normal(component)) = parts.next() else {
                        continue
                    };
                    
                    let component = component.to_str()
                        .expect("path should be utf8");

                    if !self.opts.components.iter().any(|v| v == component) {
                        continue
                    }

                    while let Some(Component::Normal(part)) = parts.next() {
                        let part_name = part.to_str()
                            .expect("path should be utf8");

                        if parts.peek().is_none() && 
                            self.file_prefix_filter.iter().any(|v| part_name.starts_with(v)) {
                            return Some((path, file_entry))
                        } 

                        if !self.dir_filter.contains(part_name) {
                            break
                        }
                    }
                },
                None => return None
            }
        }
    }
}

#[derive(Debug)]
pub struct FileLine<'a> {
    path: &'a str, 
    size: u64, 
    checksum: &'a str
}

impl<'a> FileLine<'a> {
    pub fn parse(value: &'a str) -> Result<Self> {
        let mut v = &value[1..];

        // Read checksum part
        let checksum_end = v.find(' ')
            .ok_or_else(|| MirsError::ParsingRelease { line: value.to_string() })?;

        let checksum = &v[..checksum_end];

        v = &v[checksum_end..];

        // Skip until file size
        while let Some(0) = v.find(' ') {
            v = &v[1..];
        }

        // Read file size
        let size_end = v.find(' ')
            .ok_or_else(|| MirsError::ParsingRelease { line: value.to_string() })?;

        let size = (v[..size_end]).parse()?;

        // Whatever is left is the path
        let path = &v[size_end+1..];

        Ok(Self {
            path,
            size,
            checksum
        })
    }
}

pub enum ChecksumState {
    Sha1,
    Sha256,
    Sha512,
    Md5,
    PgpMessage,
    PgpSignature,
    Unknown,
    No
}

impl From<&str> for ChecksumState {
    fn from(value: &str) -> Self {
        match value {
            "MD5Sum:" => ChecksumState::Md5,
            "SHA1:"   => ChecksumState::Sha1,
            "SHA256:" => ChecksumState::Sha256,
            "SHA512:" => ChecksumState::Sha512,
            _         => ChecksumState::Unknown
        }
    }
}

#[derive(Debug)]
pub enum Line<'a> {
    Md5Start,
    Sha1Start,
    Sha256Start,
    Sha512Start,
    UnknownChecksumStart,
    FileEntry(&'a str),
    Metadata(&'a str),
    UnknownLine(&'a str),
    PGPSignedMessageStart,
    PGPSignatureStart,
    PGPSignatureEnd
}

impl<'a> From<&'a str> for Line<'a> {
    fn from(value: &'a str) -> Self {
        match value {
            v if v.starts_with(' ') => Line::FileEntry(v),
            v if v.ends_with(':') => {
                match v {
                    "MD5Sum:" => Line::Md5Start,
                    "SHA1:"   => Line::Sha1Start,
                    "SHA256:" => Line::Sha256Start,
                    "SHA512:" => Line::Sha512Start,
                    _         => Line::UnknownChecksumStart
                }
            }
            "-----BEGIN PGP SIGNED MESSAGE-----" => Line::PGPSignedMessageStart,
            "-----BEGIN PGP SIGNATURE-----" => Line::PGPSignatureStart,
            "-----END PGP SIGNATURE-----" => Line::PGPSignatureEnd,
            v if v.contains(':') => Line::Metadata(v),
            v => Line::UnknownLine(v)
        }
    }
}

#[derive(Debug)]
pub enum Checksum {
    Md5(String),
    Sha1(String),
    Sha256(String),
    Sha512(String)
}

impl Checksum {
    pub fn relative_path(&self) -> String {
        match self {
            Checksum::Md5(checksum)    => format!("by-hash/MD5Sum/{checksum}"),
            Checksum::Sha1(checksum)   => format!("by-hash/SHA1/{checksum}"),
            Checksum::Sha256(checksum) => format!("by-hash/SHA256/{checksum}"),
            Checksum::Sha512(checksum) => format!("by-hash/SHA512/{checksum}"),
        }
    }
}

#[derive(Debug)]
pub struct FileEntry {
    pub size: u64,
    pub md5: Option<String>,
    pub sha1: Option<String>,
    pub sha256: Option<String>,
    pub sha512: Option<String>
}

impl IntoIterator for FileEntry {
    type Item = Checksum;

    type IntoIter = FileEntryChecksumIterator;

    fn into_iter(self) -> Self::IntoIter {
        FileEntryChecksumIterator::new(self)
    }
}

pub struct FileEntryChecksumIterator {
    file_entry: FileEntry,
    pos: u8
}

impl FileEntryChecksumIterator {
    pub fn new(file_entry: FileEntry) -> Self {
        Self {
            file_entry,
            pos: 0
        }
    }
}

impl Iterator for FileEntryChecksumIterator {
    type Item = Checksum;

    fn next(&mut self) -> Option<Self::Item> {
        for i in self.pos..=3 {
            let checksum = match i {
                0 => self.file_entry.sha512.take().map(Checksum::Sha512),
                1 => self.file_entry.sha256.take().map(Checksum::Sha256),
                2 => self.file_entry.sha1.take().map(Checksum::Sha1),
                3 => self.file_entry.md5.take().map(Checksum::Md5),
                _ => break
            };

            if checksum.is_none() {
                continue
            }

            self.pos = i+1;
            
            return checksum
        }

        None
    }
}