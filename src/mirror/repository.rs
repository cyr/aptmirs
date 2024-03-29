use std::path::Path;

use compact_str::{format_compact, CompactString, ToCompactString};
use reqwest::Url;

use super::downloader::Download;

use crate::{error::{MirsError, Result}, metadata::{release::FileEntry, FilePath, IndexFileEntry}};

pub struct Repository {
    root_url: CompactString,
    root_dir: FilePath,
    dist_url: CompactString,
    tmp_dir: FilePath,
}

impl Repository {
    pub fn build(archive_root: &str, suite: &str, base_dir: &FilePath) -> Result<Self> {
        let root_url = match archive_root.strip_prefix('/') {
            Some(url) => url.to_compact_string(),
            None => archive_root.to_compact_string(),
        };
        let dist_url = format_compact!("{root_url}/dists/{suite}");

        let parsed_url = Url::parse(&root_url)
            .map_err(|_| MirsError::UrlParsing { url: root_url.clone() })?;

        let root_dir = local_dir_from_archive_url(&parsed_url, base_dir)?;
        let tmp_dir = create_tmp_dir(&parsed_url, suite, base_dir)?;

        Ok(Self {
            root_url,
            root_dir,
            dist_url,
            tmp_dir
        })
    }

    pub fn release_urls(&self) -> [CompactString; 3] {
        [
            format_compact!("{}/InRelease", self.dist_url),
            format_compact!("{}/Release", self.dist_url),
            format_compact!("{}/Release.gpg", self.dist_url)
        ]
    }

    fn to_path_in_local_dir(&self, base: &FilePath, url: &str) -> FilePath {
        let relative_path = url
            .strip_prefix(&self.root_url.as_str())
            .expect("implementation error; download url should be in archive root");

        let relative_path = match relative_path.strip_prefix('/') {
            Some(path) => path,
            None => relative_path,
        };

        base.join(relative_path)
    }

    pub fn delete_tmp(&self) -> Result<()> {
        if let Err(e) = std::fs::remove_dir_all(&self.tmp_dir) {
            return Err(e.into())
        }

        Ok(())
    }

    pub async fn finalize(&self) -> Result<()> {
        let tmp_dir = self.tmp_dir.clone();
        let root_dir = self.root_dir.clone();

        tokio::task::spawn_blocking(move || {
            rebase_dir(&tmp_dir.as_ref(), &tmp_dir.as_ref(), &root_dir.as_ref())?;
            
            std::fs::remove_dir_all(&tmp_dir)?;
            
            Ok(())
        }).await?
    }

    pub fn rel_from_tmp<'a>(&self, path: &'a str) -> &'a str {
        path.strip_prefix(&self.tmp_dir.as_str())
            .expect("input path should be in tmp dir")
    }

    pub fn to_path_in_tmp(&self, url: &str) -> FilePath {
        self.to_path_in_local_dir(&self.tmp_dir, url)
    }

    pub fn to_path_in_root(&self, url: &str) -> FilePath {
        self.to_path_in_local_dir(&self.root_dir, url)
    }

    pub fn to_url_in_dist(&self, path: &str) -> CompactString {
        format_compact!("{}/{}", self.dist_url, path)
    }

    pub fn to_url_in_root(&self, path: &str) -> CompactString {
        format_compact!("{}/{}", self.root_url, path)
    }

    pub fn tmp_to_root<P: AsRef<str>>(&self, path: P) -> Option<FilePath> {
        path.as_ref().strip_prefix(self.tmp_dir.as_str())
            .map(|v| self.root_dir.join(v))
    }

    pub fn create_file_download(&self, package: IndexFileEntry) -> Box<Download> {
        let url = self.to_url_in_root(&package.path);
        let primary_target_path = self.to_path_in_root(&url);

        Box::new(Download {
            url,
            size: Some(package.size),
            checksum: package.checksum,
            primary_target_path,
            symlink_paths: Vec::new(),
            always_download: false,
        })
    }

    pub fn create_metadata_download(&self, url: CompactString, file_path: FilePath, file_entry: FileEntry, by_hash: bool) -> Result<Box<Download>> {
        let by_hash_base = FilePath(
            file_path
                .parent()
                .expect("all files needs a parent(?)")
                .to_compact_string()
        );

        let size = file_entry.size;

        let strongest_checksum = file_entry.strongest_hash();
        let mut checksum_iter = file_entry.into_iter();

        let mut symlink_paths = Vec::new();
        let primary_target_path = if by_hash {
            symlink_paths.push(file_path);

            let checksum = checksum_iter.next()
                .ok_or_else(|| MirsError::NoReleaseFile)?;

            let rel_path = checksum.relative_path();

            by_hash_base.join(rel_path)
        } else {
            file_path
        };

        for checksum in checksum_iter {
            let hash_path = by_hash_base.join(checksum.relative_path());
            symlink_paths.push(hash_path);
        }

        Ok(Box::new(Download {
            url,
            size: Some(size),
            checksum: strongest_checksum,
            primary_target_path,
            symlink_paths,
            always_download: false
        }))
    }
}

fn sanitize_path_part(part: &str) -> CompactString {
    let mut sanitized = CompactString::new("");

    let mut char_iter = part.chars();

    while let Some(c) = char_iter.next() {
        if c == '/' {
            sanitized.push('_')
        } else {
            sanitized.push(c)
        }
    }

    sanitized
}

fn create_tmp_dir(url: &Url, suite: &str, base_dir: &FilePath) -> Result<FilePath> {
    let Some(host) = url.host() else {
        return Err(MirsError::UrlParsing { url: url.to_compact_string() })
    };

    let path = url.path();

    let path_part = if path == "/" {
        CompactString::new("")
    } else {
        sanitize_path_part(path)
    };

    let tmp_dir = base_dir
        .join(format_compact!(".tmp/{host}{path_part}_{suite}"));

    match std::fs::metadata(&tmp_dir) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            std::fs::create_dir_all(&tmp_dir)?;
            Ok(tmp_dir)
        },
        Err(e) => Err(MirsError::Tmp { msg: e.to_compact_string() }),
        Ok(_) => Err(MirsError::Tmp { 
            msg: format_compact!(
                "tmp folder already exists for this repository. aptmirs is probably currently running. if it is not, delete {}",
                tmp_dir.as_str()
            )
        })
    }
}

fn local_dir_from_archive_url(url: &Url, dir: &FilePath) -> Result<FilePath> {
    let Some(host) = url.host() else {
        return Err(MirsError::UrlParsing { url: url.to_compact_string() })
    };

    let mut base_dir = dir
        .join(host.to_compact_string());

    if let Some(path) = url.path().strip_prefix('/') {
        base_dir = base_dir.join(path);
    }

    Ok(base_dir)
}

fn rebase_dir(dir: &Path, from: &Path, to: &Path) -> Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            rebase_dir(&path, from, to)?;
        } else {
            let rel_path = path.strip_prefix(from)
                .expect("implemention error; path should be in tmp");

            let new_path = to.join(rel_path);

            let parent = new_path.parent().unwrap();
            if !parent.exists() {
                std::fs::create_dir_all(parent)?;
            }
            
            if std::fs::rename(&path, &new_path).is_err() {
                std::fs::copy(&path, &new_path)?;
            }
        }
    }

    Ok(())
}