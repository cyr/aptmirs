use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use indicatif::MultiProgress;

use crate::error::{Result, MirsError};
use crate::MirrorOpts;
use crate::metadata::{package::Package, release::Release};
use crate::mirror::repository::Repository;
use self::downloader::Downloader;
use self::progress::Progress;

pub mod downloader;
pub mod progress;
pub mod repository;

pub async fn mirror(opts: &MirrorOpts, output: &Path) -> Result<()> {
    let mut repo = Repository::new(&opts.uri, &opts.distribution, output);

    let mut downloader = Downloader::build(8);
    let mut progress = downloader.progress();

    progress.next_step("Downloading release").await;

    let files = repo.download_release(&mut downloader).await?;
    
    let Some(release_file) = get_release_file(&files) else {
        return Err(MirsError::InvalidRepository)
    };

    let release = Release::parse(release_file).await?;

    progress.next_step("Downloading indices").await;

    let indices = download_indices(release, &repo, &mut downloader).await?;

    let mut progress_bar = progress.create_download_progress_bar().await;
    progress.wait_for_completion(&mut progress_bar).await;

    progress.next_step("Downloading packages").await;

    download_from_indices(&repo, &mut downloader, indices).await?;

    eprintln!("Done");

    Ok(())
}

async fn download_indices(release: Release, repo: &Repository, downloader: &mut Downloader) -> Result<Vec<PathBuf>> {
    let mut indices = Vec::new();

    let by_hash = release.acquire_by_hash();
    
    for (path, file_entry) in release.files {
        let download = repo.create_metadata_download(&path, file_entry, by_hash)?;

        if is_package(&path) {
            indices.push(repo.to_local_path(&repo.to_uri_in_dist(&path)));
        }

        downloader.queue(download).await?;
    }
    
    Ok(indices)
}

pub async fn download_from_indices(repo: &Repository, downloader: &mut Downloader, indices: Vec<PathBuf>) -> Result<()> {
    let multi_bar = MultiProgress::new();

    let mut file_progress = Progress::new_with_step(3, "Processing indices");
    let mut file_progress_bar = file_progress.create_processing_progress_bar().await;

    let dl_progress = downloader.progress();
    let mut dl_progress_bar = dl_progress.create_download_progress_bar().await;

    file_progress_bar = multi_bar.add(file_progress_bar);
    dl_progress_bar = multi_bar.add(dl_progress_bar);
        
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

    for index_path in existing_indices.values() {
        file_progress.update_progress_bar(&mut file_progress_bar);

        let package = Package::build(index_path)?;

        for maybe_entry in package {
            let (package_path, package_size) = maybe_entry?;

            let dl = repo.create_package_download(&package_path, package_size);
            
            downloader.queue(dl).await?;

            dl_progress.update_progress_bar(&mut dl_progress_bar);
            file_progress.update_progress_bar(&mut file_progress_bar);
        }

        file_progress.files.inc_success(1);
        file_progress.update_progress_bar(&mut file_progress_bar);
    }

    dl_progress.wait_for_completion(&mut dl_progress_bar).await;

    Ok(())
}

fn is_extension_preferred(old: Option<&OsStr>, new: Option<&OsStr>) -> bool {
    let old = old.map(|v| v.to_str().unwrap());
    let new = new.map(|v| v.to_str().unwrap());

    matches!((old, new), (_, Some("xz")) | (None, Some("gz")))
}

fn is_package(path: &str) -> bool {
    path.ends_with("Packages") ||
        path.ends_with("Packages.gz") || 
        path.ends_with("Packages.xz")
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