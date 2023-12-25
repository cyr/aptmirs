use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use indicatif::MultiProgress;

use crate::config::MirrorOpts;
use crate::error::{Result, MirsError};
use crate::metadata::{package::Package, release::Release};
use self::{progress::Progress, repository::Repository, downloader::{Downloader, Download}};

pub mod downloader;
pub mod progress;
pub mod repository;

pub async fn mirror(opts: &MirrorOpts, output: &Path) -> Result<u64> {
    let repo = Repository::build(&opts.uri, &opts.distribution, output)?;

    let mut downloader = Downloader::build(8);
    let mut progress = downloader.progress();

    let mut total_downloaded_size = 0_u64;

    progress.next_step("Downloading release").await;

    let release = download_release(&repo, &mut downloader).await?;

    total_downloaded_size += progress.bytes.success();

    progress.next_step("Downloading indices").await;

    let indices = download_indices(release, opts, &mut progress, &repo, &mut downloader).await?;

    total_downloaded_size += progress.bytes.success();

    progress.next_step("Downloading packages").await;

    download_from_indices(&repo, &mut downloader, indices).await?;
    
    total_downloaded_size += progress.bytes.success();

    Ok(total_downloaded_size)
}

async fn download_indices(release: Release, opts: &MirrorOpts, progress: &mut Progress, repo: &Repository, downloader: &mut Downloader) -> Result<Vec<PathBuf>> {
    let mut indices = Vec::new();

    let by_hash = release.acquire_by_hash();
    
    for (path, file_entry) in release.into_filtered_files(opts) {
        let download = repo.create_metadata_download(&path, file_entry, by_hash)?;

        if is_package(&path) {
            indices.push(repo.to_local_path(&repo.to_uri_in_dist(&path)));
        }

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
            let (file_path, file_size) = maybe_entry?;

            let dl = repo.create_file_download(&file_path, file_size);
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

pub async fn download_release(repository: &Repository, downloader: &mut Downloader) -> Result<Release> {
    let mut files = Vec::with_capacity(3);

    let mut progress = downloader.progress();
    progress.files.inc_total(3);

    let mut progress_bar = progress.create_download_progress_bar().await;

    for file_uri in repository.release_files() {
        let destination = repository.to_local_path(&file_uri);

        let dl = Download {
            primary_target_path: destination.clone(),
            uri: file_uri,
            size: None,
            symlink_paths: Vec::new(),
            always_download: true
        };

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
        return Err(MirsError::InvalidRepository)
    };

    let release = Release::parse(release_file).await?;

    Ok(release)
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
        match file.file_name()
            .expect("release files should be files")
            .to_str().expect("file names should be valid utf8") {
            "InRelease" |
            "Release" => return Some(file),
            _ => ()
        }
    }

    None
}