use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use indicatif::{MultiProgress, ProgressBar};
use tokio::time::sleep;

use crate::error::{Result, MirsError};
use crate::MirrorOpts;
use crate::metadata::{package::Package, release::{Release, FileEntry}};
use self::downloader::Downloader;
use self::progress::Progress;

mod downloader;
mod progress;

pub async fn mirror(opts: &MirrorOpts, output: &Path) -> Result<()> {
    let mut repo = Repository::new(&opts.uri, &opts.distribution, output);

    let mut progress = repo.downloader.progress();

    progress.next_step("Downloading release").await;

    let files = repo.download_release().await?;
    
    let Some(release_file) = get_release_file(&files) else {
        return Err(MirsError::InvalidRepository)
    };

    let release = Release::parse(release_file).await?;

    progress.next_step("Downloading indices").await;

    let mut packages = Vec::new();

    let by_hash = release.acquire_by_hash();
    for (path, file_entry) in release.files {
        let download = repo.create_metadata_download(&path, file_entry, by_hash)?;

        if is_package(&path) {
            packages.push(repo.to_local_path(&repo.to_uri_in_dist(&path)));
        }

        repo.downloader.queue(download).await?;
    }

    repo.wait_for_step_completion().await;

    progress.next_step("Downloading packages").await;

    repo.download_from_packages(packages).await?;

    eprintln!("Done");

    Ok(())
}

fn is_package(path: &str) -> bool {
    path.ends_with("Packages") ||
        path.ends_with("Packages.gz") || 
        path.ends_with("Packages.xz")
}

#[derive(Debug)]
pub struct Download {
    pub uri: String,
    pub size: Option<u64>,
    pub primary_target_path: PathBuf,
    pub symlink_paths: Vec<PathBuf>,
    pub always_download: bool
}

pub struct Repository {
    archive_root_uri: String,
    dist_uri: String,
    base_dir: PathBuf,
    downloader: Downloader
}

impl Repository {
    pub fn new(archive_root: &str, dist: &str, base_dir: &Path) -> Self {
        let archive_root_uri = if !archive_root.ends_with('/') {
            format!("{archive_root}/")
        } else {
            archive_root.to_string()
        };

        let dist_uri = format!("{}dists/{}", archive_root_uri, dist);

        let downloader = Downloader::build(8);

        Self {
            archive_root_uri,
            dist_uri,
            base_dir: base_dir.to_path_buf(),
            downloader
        }
    }

    pub async fn download_release(&mut self) -> Result<Vec<PathBuf>> {
        let mut files = Vec::with_capacity(3);

        let mut progress = self.downloader.progress();
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

            let download_res = self.downloader.download(dl).await;

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

    pub async fn wait_for_completion(&self, progress_bar: &mut ProgressBar)  {
        let progress = self.downloader.progress();

        while progress.files.remaining() > 0 {
            progress.update_progress_bar(progress_bar);
            sleep(Duration::from_millis(100)).await
        }

        progress.update_progress_bar(progress_bar);

        progress_bar.finish_using_style();
    }

    pub async fn wait_for_step_completion(&self) {
        let progress = self.downloader.progress();
        let mut progress_bar = progress.create_download_progress_bar().await;

        self.wait_for_completion(&mut progress_bar).await
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

    async fn download_from_packages(&mut self, packages: Vec<PathBuf>) -> Result<()> {
        let multi_bar = MultiProgress::new();

        let mut file_progress = Progress::new_with_step(3, "Processing indices");
        let mut file_progress_bar = file_progress.create_processing_progress_bar().await;

        let mut dl_progress_bar = self.downloader.progress().create_download_progress_bar().await;

        file_progress_bar = multi_bar.add(file_progress_bar);
        dl_progress_bar = multi_bar.add(dl_progress_bar);
            
        let mut existing_packages = BTreeMap::<PathBuf, PathBuf>::new();

        for package_file in packages.into_iter().filter(|f| f.exists()) {
            let file_stem = package_file.file_stem().unwrap();
            let path_with_stem = package_file.parent().unwrap().join(file_stem);

            if let Some(val) = existing_packages.get_mut(&path_with_stem) {
                if is_extension_preferred(val.extension(), package_file.extension()) {
                    *val = package_file
                }
            } else {
                existing_packages.insert(path_with_stem, package_file);
            }
        }

        file_progress.files.inc_total(existing_packages.len() as u64);

        let dl_progress = self.downloader.progress();

        for package_path in existing_packages.values() {
            file_progress.update_progress_bar(&mut file_progress_bar);

            let package = Package::build(package_path)?;

            for maybe_entry in package {
                let (package_path, package_size) = maybe_entry?;

                let dl = self.create_package_download(&package_path, package_size);
                
                self.downloader.queue(dl).await?;

                dl_progress.update_progress_bar(&mut dl_progress_bar);
                file_progress.update_progress_bar(&mut file_progress_bar);
            }

            file_progress.files.inc_success(1);
            file_progress.update_progress_bar(&mut file_progress_bar);
        }

        self.wait_for_completion(&mut dl_progress_bar).await;

        Ok(())
    }
}

fn get_release_file(files: &Vec<PathBuf>) -> Option<&PathBuf> {
    for file in files {
        match file.file_name()
            .expect("release files need to be files")
            .to_str().expect("specific file name") {
            "InRelease" |
            "Release" => return Some(file),
            _ => ()
        }
    }

    None
}

fn is_extension_preferred(old: Option<&OsStr>, new: Option<&OsStr>) -> bool {
    let old = old.map(|v| v.to_str().unwrap());
    let new = new.map(|v| v.to_str().unwrap());

    matches!((old, new), (_, Some("xz")) | (None, Some("gz")))
}