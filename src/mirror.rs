use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use indicatif::MultiProgress;

use crate::config::MirrorOpts;
use crate::error::{Result, MirsError};
use crate::metadata::checksum::Checksum;
use crate::metadata::{package::Package, release::Release};
use self::{progress::Progress, repository::Repository, downloader::{Downloader, Download}};

pub mod downloader;
pub mod progress;
pub mod repository;

pub struct MirrorResult {
    pub total_downloaded_size: u64,
    pub num_packages: u64,
    pub packages_size: u64
}

pub async fn mirror(opts: &MirrorOpts, output_dir: &Path) -> Result<Option<MirrorResult>> {
    let repo = Repository::build(&opts.url, &opts.suite, output_dir)?;

    let mut downloader = Downloader::build(8);
    let mut progress = downloader.progress();

    let mut total_downloaded_size = 0_u64;

    progress.next_step("Downloading release").await;

    let maybe_release = match download_release(&repo, &mut downloader).await {
        Ok(release) => release,
        Err(e) => {
            _ = repo.delete_tmp();
            return Err(MirsError::DownloadRelease { inner: Box::new(e) })
        },
    };

    total_downloaded_size += progress.bytes.success();

    let Some(release) = maybe_release else {
        _ = repo.delete_tmp();
        return Ok(None)
    };

    if let Some(release_components) = release.components() {
        let components = release_components.split_ascii_whitespace().collect::<Vec<&str>>();

        for requested_component in &opts.components {
            if !components.contains(&requested_component.as_str()) {
                println!("{} WARNING: {requested_component} is not in this repo", crate::now());
            }
        }
    }

    progress.next_step("Downloading indices").await;

    let indices = match download_indices(release, opts, &mut progress, &repo, &mut downloader).await {
        Ok(indices) => indices,
        Err(e) => {
            _ = repo.delete_tmp();
            return Err(MirsError::DownloadIndices { inner: Box::new(e) })
        }
    };

    total_downloaded_size += progress.bytes.success();

    progress.next_step("Downloading packages").await;

    if let Err(e) = download_from_indices(&repo, &mut downloader, indices).await {
        _ = repo.delete_tmp();
        return Err(MirsError::DownloadPackages { inner: Box::new(e) })
    }

    let packages_size = progress.bytes.success();
    let num_packages = progress.files.success();

    if let Err(e) = repo.finalize().await {
        _ = repo.delete_tmp();
        return Err(MirsError::Finalize { inner: Box::new(e) })
    }

    total_downloaded_size += packages_size;

    Ok(Some(MirrorResult {
        total_downloaded_size,
        packages_size,
        num_packages
    }))
}

async fn download_indices(release: Release, opts: &MirrorOpts, progress: &mut Progress, repo: &Repository, downloader: &mut Downloader) -> Result<Vec<PathBuf>> {
    let mut indices = Vec::new();

    let by_hash = release.acquire_by_hash();
    
    for (path, file_entry) in release.into_filtered_files(opts) {
        let url = repo.to_url_in_dist(&path);
        let file_path = repo.to_path_in_tmp(&url);
        
        // since all files have their checksums verified on download, any file that is local can
        // presumably be trusted to be correct. and since we only move in the package files on 
        // a successful mirror operation, if we see the package file and its hash file, there is
        // no need to queue its packages.
        if let Some(checksum) = file_entry.strongest_hash() {
            let by_hash_base = file_path
                .parent()
                .expect("all files needs a parent(?)")
                .to_owned();

            let checksum_path = by_hash_base.join(checksum.relative_path());

            if checksum_path.exists() && file_path.exists() {
                continue
            }
        }

        if is_package(&path) {
            indices.push(repo.to_path_in_tmp(&repo.to_url_in_dist(&path)));
        }

        let download = repo.create_metadata_download(url, file_path, file_entry, by_hash)?;
        downloader.queue(download).await?;
    }

    let mut progress_bar = progress.create_download_progress_bar().await;
    progress.wait_for_completion(&mut progress_bar).await;

    Ok(indices)
}

pub async fn download_from_indices(repo: &Repository, downloader: &mut Downloader, indices: Vec<PathBuf>) -> Result<()> {
    let multi_bar = MultiProgress::new();

    let mut file_progress = Progress::new_with_step(3, "Processing indices");
    let mut dl_progress = downloader.progress();

    let mut file_progress_bar = multi_bar.add(file_progress.create_processing_progress_bar().await);
    let mut dl_progress_bar = multi_bar.add(dl_progress.create_download_progress_bar().await);
        
    let mut existing_indices = BTreeMap::<PathBuf, PathBuf>::new();

    for package_file in indices.into_iter().filter(|f| f.exists()) {
        let file_stem = package_file.file_stem().unwrap();
        let path_with_stem = package_file.parent().unwrap().join(file_stem);

        if let Some(val) = existing_indices.get_mut(&path_with_stem) {
            if is_extension_preferred(val.extension(), package_file.extension()) {
                *val = package_file
            }
        } else {
            existing_indices.insert(path_with_stem, package_file);
        }
    }

    file_progress.files.inc_total(existing_indices.len() as u64);

    let packages = existing_indices
        .values()
        .map(|v| Package::build(v))
        .collect::<Result<Vec<_>>>()?;

    let total_size = packages.iter().map(|v| v.size()).sum();
    let mut incremental_size_base = 0;

    file_progress.bytes.inc_total(total_size);

    for package in packages {
        let counter = package.counter();
        file_progress.update_for_bytes(&mut file_progress_bar);
        let package_size = package.size();

        for maybe_entry in package {
            let (file_path, file_size, file_checksum) = maybe_entry?;

            let dl = repo.create_file_download(&file_path, file_size, file_checksum);
            downloader.queue(dl).await?;
            
            file_progress.bytes.set_success(counter.load(Ordering::SeqCst) + incremental_size_base);

            dl_progress.update_for_files(&mut dl_progress_bar);
            file_progress.update_for_bytes(&mut file_progress_bar);
        }

        incremental_size_base += package_size;
        file_progress.update_for_bytes(&mut file_progress_bar);
    }

    dl_progress.wait_for_completion(&mut dl_progress_bar).await;

    Ok(())
}

pub async fn download_release(repository: &Repository, downloader: &mut Downloader) -> Result<Option<Release>> {
    let mut files = Vec::with_capacity(3);

    let mut progress = downloader.progress();
    progress.files.inc_total(3);

    let mut progress_bar = progress.create_download_progress_bar().await;

    for file_url in repository.release_urls() {
        let destination = repository.to_path_in_tmp(&file_url);

        let dl = Box::new(Download {
            primary_target_path: destination.clone(),
            url: file_url,
            checksum: None,
            size: None,
            symlink_paths: Vec::new(),
            always_download: true
        });

        let download_res = downloader.download(dl).await;

        progress.update_for_files(&mut progress_bar);

        if let Err(e) = download_res {
            println!("{} {e}", crate::now());
            continue
        }

        files.push(destination);
    }

    progress_bar.finish_using_style();

    let Some(release_file) = get_release_file(&files) else {
        return Err(MirsError::NoReleaseFile)
    };

    // if the release file we already have has the same checksum as the one we downloaded, because
    // of how all metadata files are moved into the repository path after the mirroring operation
    // is completed successfully, there should be nothing more to do. save bandwidth, save lives!
    if let Some(local_release_file) = repository.tmp_to_root(release_file) {
        if local_release_file.exists() {
            let tmp_checksum = Checksum::checksum_file(&local_release_file).await?;
            let local_checksum = Checksum::checksum_file(release_file).await?;

            if tmp_checksum == local_checksum {
                return Ok(None)
            }
        }
    }

    let release = Release::parse(release_file).await
        .map_err(|e| MirsError::InvalidReleaseFile { inner: Box::new(e) })?;

    Ok(Some(release))
}

fn is_extension_preferred(old: Option<&OsStr>, new: Option<&OsStr>) -> bool {
    let old = old.map(|v| v.to_str().unwrap());
    let new = new.map(|v| v.to_str().unwrap());

    matches!((old, new),
        (_, Some("gz")) |
        (_, Some("xz")) |
        (_, Some("bz2")) 
    )
}

fn is_package(path: &str) -> bool {
    path.ends_with("Packages") ||
        path.ends_with("Packages.gz") || 
        path.ends_with("Packages.xz") ||
        path.ends_with("Packages.bz2")
}

fn get_release_file(files: &Vec<PathBuf>) -> Option<&PathBuf> {
    for file in files {
        let file_name = file.file_name()
            .expect("release files should be files");

        if let b"InRelease" | b"Release" = file_name.as_bytes() {
            return Some(file)
        }
    }

    None
}