use std::path::{PathBuf, Path};

use reqwest::Url;

use super::downloader::Download;

use crate::{error::{Result, MirsError}, metadata::{release::FileEntry, checksum::Checksum}};

pub struct Repository {
    root_url: String,
    root_dir: PathBuf,
    dist_url: String,
    tmp_dir: PathBuf,
}

impl Repository {
    pub fn build(archive_root: &str, dist: &str, base_dir: &Path) -> Result<Self> {
        let root_url = match archive_root.strip_prefix('/') {
            Some(url) => url.to_string(),
            None => archive_root.to_string(),
        };

        let dist_url = format!("{root_url}/dists/{dist}");

        let parsed_url = Url::parse(&root_url)
            .map_err(|_| MirsError::UrlParsing { url: root_url.to_string() })?;

        let root_dir = local_dir_from_archive_url(&parsed_url, base_dir)?;

        let tmp_dir = create_tmp_dir(&parsed_url, dist, base_dir)?;

        Ok(Self {
            root_url,
            root_dir,
            dist_url,
            tmp_dir
        })
    }

    pub fn release_urls(&self) -> [String; 3] {
        [
            format!("{}/InRelease", self.dist_url),
            format!("{}/Release", self.dist_url),
            format!("{}/Release.gpg", self.dist_url)
        ]
    }

    fn to_path_in_local_dir(&self, base: &Path, url: &str) -> PathBuf {
        let relative_path = url
            .strip_prefix(&self.root_url)
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
            rebase_dir(&tmp_dir, &tmp_dir, &root_dir)?;
            
            std::fs::remove_dir_all(&tmp_dir)?;
            
            Ok(())
        }).await?
    }

    pub fn to_path_in_tmp(&self, url: &str) -> PathBuf {
        self.to_path_in_local_dir(&self.tmp_dir, url)
    }

    pub fn to_path_in_root(&self, url: &str) -> PathBuf {
        self.to_path_in_local_dir(&self.root_dir, url)
    }

    pub fn to_url_in_dist(&self, path: &str) -> String {
        format!("{}/{}", self.dist_url, path)
    }

    pub fn to_url_in_root(&self, path: &str) -> String {
        format!("{}/{}", self.root_url, path)
    }

    pub fn tmp_to_root(&self, path: &Path) -> Option<PathBuf> {
        path.strip_prefix(&self.tmp_dir)
            .map(|v| self.root_dir.join(v))
            .ok()
    }

    pub fn create_file_download(&self, path: &str, size: u64, checksum: Option<Checksum>) -> Box<Download> {
        let url = self.to_url_in_root(path);
        let primary_target_path = self.to_path_in_root(&url);

        Box::new(Download {
            url,
            size: Some(size),
            checksum,
            primary_target_path,
            symlink_paths: Vec::new(),
            always_download: false,
        })
    }

    pub fn create_metadata_download(&self, url: String, file_path: PathBuf, file_entry: FileEntry, by_hash: bool) -> Result<Box<Download>> {
        let by_hash_base = file_path
            .parent()
            .expect("all files needs a parent(?)")
            .to_owned();

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


fn create_tmp_dir(url: &Url, dist: &str, base_dir: &Path) -> Result<PathBuf> {
    let Some(host) = url.host() else {
        return Err(MirsError::UrlParsing { url: url.to_string() })
    };

    let path = url.path();

    let path_part = if path == "/" {
        String::new()
    } else {
        path.replace('/', "_")
    };

    let tmp_dir = base_dir
        .join(".tmp")
        .join(format!("{host}{path_part}_{dist}"));

    match std::fs::metadata(&tmp_dir) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            std::fs::create_dir_all(&tmp_dir)?;
            Ok(tmp_dir)
        },
        Err(e) => Err(MirsError::Tmp { msg: e.to_string() }),
        Ok(_) => Err(MirsError::Tmp { 
            msg: format!(
                "tmp folder already exists for this repository. aptmirs is probably currently running. if it is not, delete {}",
                tmp_dir.to_string_lossy()
            )
        })
    }
}

fn local_dir_from_archive_url(url: &Url, dir: &Path) -> Result<PathBuf> {
    let Some(host) = url.host() else {
        return Err(MirsError::UrlParsing { url: url.to_string() })
    };

    let mut base_dir = dir
        .join(host.to_string());

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