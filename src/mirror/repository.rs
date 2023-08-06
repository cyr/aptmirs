use std::path::{PathBuf, Path};

use super::downloader::{Downloader, Download};

use crate::{error::{Result, MirsError}, metadata::release::FileEntry};

pub struct Repository {
    archive_root_uri: String,
    dist_uri: String,
    base_dir: PathBuf
}

impl Repository {
    pub fn new(archive_root: &str, dist: &str, base_dir: &Path) -> Self {
        let archive_root_uri = if !archive_root.ends_with('/') {
            format!("{archive_root}/")
        } else {
            archive_root.to_string()
        };

        let dist_uri = format!("{}dists/{}", archive_root_uri, dist);

        Self {
            archive_root_uri,
            dist_uri,
            base_dir: base_dir.to_path_buf()
        }
    }

    pub async fn download_release(&mut self, downloader: &mut Downloader) -> Result<Vec<PathBuf>> {
        let mut files = Vec::with_capacity(3);

        let mut progress = downloader.progress();
        progress.files.inc_total(3);

        let mut progress_bar = progress.create_download_progress_bar().await;

        for file_uri in self.release_files() {
            let destination = self.to_local_path(&file_uri);

            let dl = Download {
                primary_target_path: destination.clone(),
                uri: file_uri,
                size: None,
                symlink_paths: Vec::new(),
                always_download: true
            };

            let download_res = downloader.download(dl).await;

            progress.update_progress_bar(&mut progress_bar);

            if let Err(e) = download_res {
                eprintln!("{e}");
                continue
            }

            files.push(destination);
        }

        progress_bar.finish_using_style();

        Ok(files)
    }

    fn release_files(&self) -> Vec<String> {
        vec![
            format!("{}/InRelease", self.dist_uri),
            format!("{}/Release", self.dist_uri),
            format!("{}/Release.gpg", self.dist_uri)
        ]
    }

    pub fn to_local_path(&self, uri: &str) -> PathBuf {
        let relative_path = uri
            .strip_prefix(&self.archive_root_uri).expect("implementation error; download uri is not in archive root");

        self.base_dir.join(relative_path)
    }

    pub fn to_uri_in_dist(&self, path: &String) -> String {
        format!("{}/{}", self.dist_uri, path)
    }

    pub fn create_package_download(&self, path: &Path, size: u64) -> Download {
        let primary_target_path = self.base_dir.join(path);
        let uri = format!("{}{}", self.archive_root_uri, path.to_string_lossy());

        Download {
            uri,
            primary_target_path,
            size: Some(size),
            symlink_paths: Vec::new(),
            always_download: false,
        }
    }

    pub fn create_metadata_download(&self, path: &String, file_entry: FileEntry, by_hash: bool) -> Result<Download> {
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
