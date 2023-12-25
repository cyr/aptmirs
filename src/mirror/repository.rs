use std::path::{PathBuf, Path};

use reqwest::Url;

use super::downloader::Download;

use crate::{error::{Result, MirsError}, metadata::release::FileEntry};

pub struct Repository {
    root_uri: String,
    root_dir: PathBuf,
    dist_uri: String
}

impl Repository {
    pub fn build(archive_root: &str, dist: &str, base_dir: &Path) -> Result<Self> {
        let root_uri = match archive_root.strip_prefix('/') {
            Some(uri) => uri.to_string(),
            None => archive_root.to_string(),
        };

        let dist_uri = format!("{root_uri}/dists/{dist}");
        
        let root_dir = local_dir_from_archive_uri(&root_uri, base_dir)?;

        Ok(Self {
            root_uri,
            root_dir,
            dist_uri
        })
    }


    pub fn release_files(&self) -> [String; 3] {
        [
            format!("{}/InRelease", self.dist_uri),
            format!("{}/Release", self.dist_uri),
            format!("{}/Release.gpg", self.dist_uri)
        ]
    }

    pub fn to_local_path(&self, uri: &str) -> PathBuf {
        let relative_path = uri
            .strip_prefix(&self.root_uri).expect("implementation error; download uri should be in archive root");

        let relative_path = match relative_path.strip_prefix('/') {
            Some(path) => path,
            None => relative_path,
        };

        self.root_dir.join(relative_path)
    }

    pub fn to_uri_in_dist(&self, path: &str) -> String {
        format!("{}/{}", self.dist_uri, path)
    }

    pub fn to_uri_in_root(&self, path: &str) -> String {
        format!("{}/{}", self.root_uri, path)
    }

    pub fn create_file_download(&self, path: &str, size: u64) -> Download {
        let uri = self.to_uri_in_root(path);
        let primary_target_path = self.to_local_path(&uri);

        Download {
            uri,
            primary_target_path,
            size: Some(size),
            symlink_paths: Vec::new(),
            always_download: false,
        }
    }

    pub fn create_metadata_download(&self, path: &str, file_entry: FileEntry, by_hash: bool) -> Result<Download> {
        let uri = self.to_uri_in_dist(path);
        let file_path = self.to_local_path(&uri);

        let by_hash_base = file_path
            .parent()
            .expect("all files needs a parent(?)")
            .to_owned();

        let size = file_entry.size;

        let mut checksum_iter = file_entry.into_iter();

        let mut symlink_paths = Vec::new();
        let primary_target_path = if by_hash {
            symlink_paths.push(file_path);

            let rel_path = checksum_iter.next()
                .ok_or_else(|| MirsError::InvalidRepository)?
                .relative_path();

            by_hash_base.join(rel_path)
        } else {
            file_path
        };

        for checksum in checksum_iter {
            let hash_path = by_hash_base.join(checksum.relative_path());
            symlink_paths.push(hash_path)
        }

        Ok(Download {
            uri,
            size: Some(size),
            primary_target_path,
            symlink_paths,
            always_download: false
        })
    }
}

fn local_dir_from_archive_uri(uri: &str, dir: &Path) -> Result<PathBuf> {
    let parsed_uri = Url::parse(uri)
        .map_err(|_| MirsError::UrlParsing { url: uri.to_string() })?;

    let Some(host) = parsed_uri.host() else {
        return Err(MirsError::UrlParsing { url: uri.to_string() })
    };

    let mut base_dir = dir
        .join(host.to_string());

    if let Some(path) = parsed_uri.path().strip_prefix('/') {
        base_dir = base_dir.join(path);
    }

    Ok(base_dir)
}