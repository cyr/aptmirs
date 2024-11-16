use std::{path::{Path, Component}, collections::{BTreeMap, BTreeSet}};

use compact_str::{format_compact, CompactString, ToCompactString};
use tokio::{fs::File, io::{BufReader, AsyncBufReadExt}};

use crate::{error::{Result, MirsError}, config::MirrorOpts};

use super::{checksum::Checksum, metadata_file::MetadataFile, FilePath};

#[derive(Debug)]
pub struct Release {
    map: BTreeMap<CompactString, CompactString>,
    pub files: BTreeMap<CompactString, FileEntry>
}

impl Release {
    pub async fn parse(path: &FilePath) -> Result<Release> {
        let file = File::open(path).await?;
        let file_size = file.metadata().await?.len();

        let reader_capacity = file_size.min(1024*1024) as usize;
        let buf_capacity = reader_capacity.min(1024*8*8);

        let mut buf = String::with_capacity(buf_capacity);
        let mut reader = BufReader::with_capacity(reader_capacity, file);        

        let mut checksum_state = ChecksumState::No;

        let mut map = BTreeMap::new();
        let mut files = BTreeMap::<CompactString, FileEntry>::new();

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
                        files.insert(file_line.path.to_compact_string(), 
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
                        ChecksumState::Sha1 => {
                            let mut checksum = [0_u8; 20];
                            hex::decode_to_slice(file_line.checksum, &mut checksum)?;
                            entry.sha1 = Some(checksum);
                        },
                        ChecksumState::Sha256 => {
                            let mut checksum = [0_u8; 32];
                            hex::decode_to_slice(file_line.checksum, &mut checksum)?;
                            entry.sha256 = Some(checksum);
                        },
                        ChecksumState::Sha512 => {
                            let mut checksum = [0_u8; 64];
                            hex::decode_to_slice(file_line.checksum, &mut checksum)?;
                            entry.sha512 = Some(checksum);
                        },
                        ChecksumState::Md5 => {
                            let mut checksum = [0_u8; 16];
                            hex::decode_to_slice(file_line.checksum, &mut checksum)?;
                            entry.md5 = Some(checksum);
                        },
                        ChecksumState::No => return Err(MirsError::ParsingRelease { line: v.to_compact_string() }),
                        _ => continue
                    };
                },
                Line::Metadata(v) => {
                    if let ChecksumState::PgpMessage | ChecksumState::PgpSignature = checksum_state {
                        continue
                    }
                    
                    checksum_state = ChecksumState::No;

                    let (k, v) = v.split_once(": ")
                        .ok_or_else(|| MirsError::ParsingRelease { line: v.to_compact_string() })?;

                    map.insert(k.to_compact_string(), v.to_compact_string());
                },
                Line::Md5Start              => checksum_state = ChecksumState::Md5,
                Line::Sha1Start             => checksum_state = ChecksumState::Sha1,
                Line::Sha256Start           => checksum_state = ChecksumState::Sha256,
                Line::Sha512Start           => checksum_state = ChecksumState::Sha512,
                Line::PGPSignedMessageStart => checksum_state = ChecksumState::PgpMessage,
                Line::PGPSignatureStart     => checksum_state = ChecksumState::PgpSignature,
                Line::PGPSignatureEnd       => checksum_state = ChecksumState::No,
                Line::Unknown               => {
                    match checksum_state {
                        ChecksumState::PgpSignature => (),
                        _ => checksum_state = ChecksumState::No
                    }
                },
            }
        }

        Ok(Release {
            map,
            files
        })
    }

    pub fn acquire_by_hash(&self) -> bool {
        self.map.get("Acquire-By-Hash")
            .map(|v| v.as_str() == "yes")
            .unwrap_or(false)
    }

    pub fn components(&self) -> Option<&CompactString> {
        self.map.get("Components")
    }

    pub fn into_filtered_files(self, opts: &MirrorOpts) -> ReleaseFileIterator {
        ReleaseFileIterator::new(self, opts)
    }

    pub fn deduplicate(&mut self, mut old_release: Release) {
        let mut filtered_files = BTreeMap::new();
        
        while let Some((path, entry)) = self.files.pop_first() {
            if let Some(old_entry) = old_release.files.remove(&path) {
                if entry != old_entry {
                    filtered_files.insert(path, entry);
                }
            }
        }

        self.files = filtered_files;
    }
}

pub struct ReleaseFileIterator<'a> {
    release: Release,
    opts: &'a MirrorOpts,
    file_prefix_filter: Vec<CompactString>,
    dir_filter: BTreeSet<CompactString>
}

impl<'a> ReleaseFileIterator<'a> {
    pub fn new(release: Release, opts: &'a MirrorOpts) -> Self {
        let mut file_prefix_filter = Vec::new();
        let mut dir_filter = BTreeSet::new();

        file_prefix_filter.push(CompactString::const_new("Release"));

        if opts.source {
            file_prefix_filter.push(CompactString::const_new("Sources"));

            dir_filter.insert(CompactString::const_new("source"));
        }

        if opts.packages {
            file_prefix_filter.push(CompactString::const_new("Contents-all"));
            file_prefix_filter.push(CompactString::const_new("Components-all"));
            file_prefix_filter.push(CompactString::const_new("Commands-all"));
            file_prefix_filter.push(CompactString::const_new("Packages"));
            file_prefix_filter.push(CompactString::const_new("icons"));
            file_prefix_filter.push(CompactString::const_new("Translation"));
            file_prefix_filter.push(CompactString::const_new("Index"));
            
            dir_filter.insert(CompactString::const_new("dep11"));
            dir_filter.insert(CompactString::const_new("i18n"));
            dir_filter.insert(CompactString::const_new("binary-all"));
            dir_filter.insert(CompactString::const_new("cnf"));
            dir_filter.insert(CompactString::const_new("Contents-all.diff"));
            dir_filter.insert(CompactString::const_new("Packages.diff"));

            if opts.udeb {
                file_prefix_filter.push(CompactString::const_new("Contents-udeb-all"));
                dir_filter.insert(CompactString::const_new("debian-installer"));
            }

            for arch in &opts.arch {
                dir_filter.insert(format_compact!("binary-{arch}"));
                dir_filter.insert(format_compact!("Contents-{arch}.diff"));

                file_prefix_filter.push(format_compact!("Components-{arch}"));
                file_prefix_filter.push(format_compact!("Contents-{arch}"));
                file_prefix_filter.push(format_compact!("Commands-{arch}"));

                if opts.udeb {
                    file_prefix_filter.push(format_compact!("Contents-udeb-{arch}"));
                }
            }

            if !opts.debian_installer_arch.is_empty() {
                file_prefix_filter.push(CompactString::const_new("MD5SUMS"));
                file_prefix_filter.push(CompactString::const_new("SHA256SUMS"));
                file_prefix_filter.push(CompactString::const_new("SHA512SUMS"));

                dir_filter.insert(CompactString::const_new("current"));
                dir_filter.insert(CompactString::const_new("images"));

                for di_arch in &opts.debian_installer_arch {
                    dir_filter.insert(format_compact!("installer-{di_arch}"));
                }
            }
            
        };

        Self {
            release,
            opts,
            file_prefix_filter,
            dir_filter
        }
    }
}

impl<'a> Iterator for ReleaseFileIterator<'a> {
    type Item = (MetadataFile, FileEntry);

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

                    if !self.opts.components.iter().any(|v| v.as_str() == component) {
                        continue
                    }

                    while let Some(Component::Normal(part)) = parts.next() {
                        let part_name = part.to_str()
                            .expect("path should be utf8");

                        if parts.peek().is_none() {
                            if self.file_prefix_filter.iter().any(|v| part_name.starts_with(v.as_str())) {
                                return Some((path.into(), file_entry))
                            }

                            break
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
        let mut parts = value.split_ascii_whitespace();

        let (Some(checksum), Some(size), Some(path)) = (parts.next(), parts.next(), parts.next()) else {
            return Err(MirsError::ParsingRelease { line: value.to_compact_string() })
        };

        let size = size.parse()?;

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
    FileEntry(&'a str),
    Metadata(&'a str),
    Unknown,
    PGPSignedMessageStart,
    PGPSignatureStart,
    PGPSignatureEnd
}

impl<'a> From<&'a str> for Line<'a> {
    fn from(value: &'a str) -> Self {
        match value {
            v if v.starts_with(' ')        => Line::FileEntry(v),
            "MD5Sum:"                            => Line::Md5Start,
            "SHA1:"                              => Line::Sha1Start,
            "SHA256:"                            => Line::Sha256Start,
            "SHA512:"                            => Line::Sha512Start,
            "-----BEGIN PGP SIGNED MESSAGE-----" => Line::PGPSignedMessageStart,
            "-----BEGIN PGP SIGNATURE-----"      => Line::PGPSignatureStart,
            "-----END PGP SIGNATURE-----"        => Line::PGPSignatureEnd,
            v if v.contains(':')           => Line::Metadata(v),
            _                                    => Line::Unknown
        }
    }
}

impl Checksum {
    pub fn relative_path(&self) -> String {
        match self {
            Checksum::Md5(checksum)    => format!("by-hash/MD5Sum/{}", hex::encode(checksum)),
            Checksum::Sha1(checksum)   => format!("by-hash/SHA1/{}",   hex::encode(checksum)),
            Checksum::Sha256(checksum) => format!("by-hash/SHA256/{}", hex::encode(checksum)),
            Checksum::Sha512(checksum) => format!("by-hash/SHA512/{}", hex::encode(checksum)),
        }
    }
}

#[derive(Debug, PartialEq, Copy, Clone)]
pub struct FileEntry {
    pub size: u64,
    pub md5: Option<[u8; 16]>,
    pub sha1: Option<[u8; 20]>,
    pub sha256: Option<[u8; 32]>,
    pub sha512: Option<[u8; 64]>
}

impl FileEntry {
    pub fn strongest_hash(&self) -> Option<Checksum> {
        if let Some(hash) = self.sha512 {
            Some(hash.into())
        } else if let Some(hash) = self.sha256 {
            Some(hash.into())
        } else if let Some(hash) = self.sha1 {
            Some(hash.into())
        } else {
            self.md5.map(|v| v.into())
        }
    }

    pub fn into_paths(self, file_path: &FilePath, by_hash: bool) -> Result<(Option<Checksum>, FilePath, Vec<FilePath>)> {
        let strongest_checksum = self.strongest_hash();

        let mut checksum_iter = self.into_iter();

        let mut symlink_paths = Vec::new();
        let primary_target_path = if by_hash {
            let by_hash_base = FilePath(
                file_path.parent().unwrap_or("").to_compact_string()
            );

            symlink_paths.push(file_path.clone());

            let strongest_checksum = checksum_iter.next()
                .ok_or_else(|| MirsError::NoReleaseFile)?;

            for checksum in checksum_iter {
                let hash_path = by_hash_base.join(checksum.relative_path());
                symlink_paths.push(hash_path);
            }

            by_hash_base.join(strongest_checksum.relative_path())
        } else {
            file_path.clone()
        };
        
        Ok((strongest_checksum, primary_target_path, symlink_paths))
    }
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