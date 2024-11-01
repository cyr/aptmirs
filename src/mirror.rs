use std::collections::BTreeMap;
use std::fmt::Display;
use std::str::FromStr;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use compact_str::format_compact;
use indicatif::{MultiProgress, HumanBytes};
use tokio::runtime::Handle;
use tokio::task::spawn_blocking;

use crate::metadata::diff_index_file::DiffIndexFile;
use crate::metadata::sum_file::{to_strongest_by_checksum, SumFileEntry};
use crate::pgp::{verify_release_signature, PgpKeyStore};
use crate::CliOpts;
use crate::config::MirrorOpts;
use crate::error::{Result, MirsError};
use crate::metadata::{FilePath, IndexSource};
use crate::metadata::checksum::Checksum;
use crate::metadata::release::Release;
use self::{progress::Progress, repository::Repository, downloader::{Downloader, Download}};

pub mod downloader;
pub mod progress;
pub mod repository;

pub enum MirrorResult {
    NewRelease { total_download_size: u64, num_packages_downloaded: u64 },
    ReleaseUnchanged,
    IrrelevantChanges
}

impl Display for MirrorResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MirrorResult::NewRelease { total_download_size, num_packages_downloaded } =>
                f.write_fmt(format_args!(
                    "{} downloaded, {} packages/source files", 
                    HumanBytes(*total_download_size),
                    num_packages_downloaded
                )),
            MirrorResult::ReleaseUnchanged =>
                f.write_str("release unchanged"),
            MirrorResult::IrrelevantChanges =>
                f.write_str("new release, but changes do not apply to configured selections")
        }
    }
}

pub async fn mirror(opts: &MirrorOpts, cli_opts: &CliOpts, mut downloader: Downloader, key_store: &Option<PgpKeyStore>) -> Result<MirrorResult> {
    let repo = Repository::build(opts, cli_opts)?;

    let mut progress = downloader.progress();
    progress.reset();

    if opts.debian_installer() {
        progress.set_total_steps(5);
    }

    let mut total_download_size = 0_u64;

    progress.next_step("Downloading release").await;

    let release = match download_release(&repo, &mut downloader, opts, cli_opts, key_store).await {
        Ok(Some(release)) => release,
        Ok(None) => {
            _ = repo.delete_tmp();
            return Ok(MirrorResult::ReleaseUnchanged)
        }
        Err(e) => {
            _ = repo.delete_tmp();
            return Err(MirsError::DownloadRelease { inner: Box::new(e) })
        },
    };

    total_download_size += progress.bytes.success();

    if let Some(release_components) = release.components() {
        let components = release_components.split_ascii_whitespace().collect::<Vec<&str>>();

        for requested_component in &opts.components {
            if !components.contains(&requested_component.as_str()) {
                println!("{} WARNING: {requested_component} is not in this repo", crate::now());
            }
        }
    }

    progress.next_step("Downloading indices").await;

    let (indices, diff_indices, di_indices) = match download_indices(release, opts, cli_opts, &mut progress, &repo, &mut downloader).await {
        Ok((indices, diff_indices, di_indices)) if indices.is_empty() && diff_indices.is_empty() && di_indices.is_empty() => {
            repo.finalize(Vec::new()).await?;
            return Ok(MirrorResult::IrrelevantChanges)
        }
        Ok(indices) => indices,
        Err(e) => {
            _ = repo.delete_tmp();
            return Err(MirsError::DownloadIndices { inner: Box::new(e) })
        }
    };

    total_download_size += progress.bytes.success();

    progress.next_step("Downloading diffs").await;

    if let Err(e) = download_from_diff_indices(&repo, &mut downloader, &mut progress, diff_indices).await {
        _ = repo.delete_tmp();
        return Err(MirsError::DownloadDiffs { inner: Box::new(e) })
    }
    
    total_download_size += progress.bytes.success();

    progress.next_step("Downloading packages").await;

     
    if let Err(e) = download_from_indices(repo.clone(), downloader.clone(), indices).await {
        _ = repo.delete_tmp();
        return Err(MirsError::DownloadPackages { inner: Box::new(e) })
    }

    let packages_size = progress.bytes.success();
    let num_packages_downloaded = progress.files.success();

    total_download_size += packages_size;

    let paths_to_delete = if opts.debian_installer() { 
        progress.next_step("Downloading debian installer").await;

        let paths = match download_debian_installer(&repo, &mut downloader, &mut progress, di_indices).await {
            Ok(paths) => paths,
            Err(e) => {
                _ = repo.delete_tmp();
                return Err(MirsError::DownloadDebianInstaller { inner: Box::new(e) })
            }
        };
        
        total_download_size += progress.bytes.success();

        paths
    } else {
        Vec::new()
    };

    if let Err(e) = repo.finalize(paths_to_delete).await {
        _ = repo.delete_tmp();
        return Err(MirsError::Finalize { inner: Box::new(e) })
    }

    Ok(MirrorResult::NewRelease {
        total_download_size,
        num_packages_downloaded
    })
}

async fn download_indices(release: Release, opts: &MirrorOpts, cli_opts: &CliOpts, progress: &mut Progress, repo: &Repository, downloader: &mut Downloader) -> Result<(Vec<FilePath>, Vec<FilePath>, Vec<FilePath>)> {
    let mut indices = Vec::new();
    let mut index_files = Vec::new();
    let mut debian_installer_sumfiles = Vec::new();

    let by_hash = release.acquire_by_hash();

    for (path, file_entry) in release.into_filtered_files(opts) {
        let mut add_by_hash = by_hash;
        let url = repo.to_url_in_dist(&path);

        let file_path_in_tmp = repo.to_path_in_tmp(&url);

        let file_path_in_root = repo.to_path_in_root(&url);
        
        // since all files have their checksums verified on download, any file that is local can
        // presumably be trusted to be correct. and since we only move in the metadata files on 
        // a successful mirror operation, if we see the metadata file and its hash file, there is
        // no need to queue its content.
        if let Some(checksum) = file_entry.strongest_hash() {
            let by_hash_base = file_path_in_root
                .parent()
                .expect("all files need a parent(?)");

            let checksum_path = FilePath(format_compact!("{by_hash_base}/{}", checksum.relative_path()));

            if checksum_path.exists() && file_path_in_root.exists() && !cli_opts.force {
                continue
            }
        }

        if is_packages_file(&path) || is_sources_file(&path) {
            indices.push(file_path_in_tmp.clone());
        }

        if is_index_file(&path) {
            index_files.push(file_path_in_tmp.clone());
        }

        if is_debian_installer_file(&path) {
            debian_installer_sumfiles.push(file_path_in_tmp.clone());
            add_by_hash = false;
        }

        let download = repo.create_metadata_download(url, file_path_in_tmp, file_entry, add_by_hash)?;
        downloader.queue(download).await?;
    }

    let mut progress_bar = progress.create_download_progress_bar().await;
    progress.wait_for_completion(&mut progress_bar).await;

    Ok((indices, index_files, debian_installer_sumfiles))
}

pub async fn download_debian_installer(repo: &Repository, downloader: &mut Downloader, progress: &mut Progress, di_indices: Vec<FilePath>) -> Result<Vec<FilePath>> {
    let sum_files = to_strongest_by_checksum(di_indices)?;

    let mut paths_to_delete = Vec::with_capacity(sum_files.len());
    
    for sum_file in sum_files.iter() {
        let base = sum_file.path().parent()
            .expect("there should always be a parent");

        let rel_path = repo.strip_tmp_base(base).expect("sum files should be in tmp");

        let current_image = repo.rebase_to_root(rel_path);

        paths_to_delete.push(current_image);
    }

    let mut progress_bar = progress.create_download_progress_bar().await;

    for sum_file in sum_files {
        let base_path = sum_file.path().parent()
            .expect("sum files should have a parent");

        let base_path = FilePath::from_str(base_path)?;

        for entry in sum_file.try_into_iter()? {
            let SumFileEntry { checksum, path } = entry?;

            let new_path = base_path.join(path);

            let new_rel_path = repo.strip_tmp_base(new_path)
                .expect("the new path should be in tmp");

            let url = repo.to_url_in_root(new_rel_path.as_str());
            let target_path = repo.to_path_in_tmp(&url);

            let download = repo.create_raw_download(target_path, url, Some(checksum));

            downloader.queue(download).await?;
        }
    }
    
    progress.wait_for_completion(&mut progress_bar).await;

    Ok(paths_to_delete)
}

pub async fn download_from_diff_indices(repo: &Repository, downloader: &mut Downloader, progress: &mut Progress, diff_indices: Vec<FilePath>) -> Result<()> {
    for path in diff_indices {
        let rel_path = FilePath::from_str(
            repo.rel_from_tmp(path.as_str())
        )?;

        let rel_base_path = FilePath::from_str(rel_path.parent().unwrap())?;

        let mut diff_index = DiffIndexFile::parse(&path).await?;

        while let Some((path, entry)) = diff_index.files.pop_first() {
            let rel_file_path = rel_base_path.join(&path);

            let url = repo.to_url_in_root(rel_file_path.as_str());
            let primary_target_path = repo.to_path_in_root(&url);

            let checksum = entry.strongest_hash();

            let download = Download {
                url,
                size: Some(entry.size),
                checksum,
                primary_target_path,
                symlink_paths: Vec::new(),
                always_download: false,
            };

            downloader.queue(Box::new(download)).await?;
        }
    }

    let mut progress_bar = progress.create_download_progress_bar().await;
    progress.wait_for_completion(&mut progress_bar).await;

    Ok(())
}

pub async fn download_from_indices(repo: Arc<Repository>, mut downloader: Downloader, indices: Vec<FilePath>) -> Result<()> {
    let multi_bar = MultiProgress::new();

    let mut file_progress = Progress::new_with_step(3, "Processing indices");
    let mut dl_progress = downloader.progress();

    let mut file_progress_bar = multi_bar.add(file_progress.create_processing_progress_bar().await);
    let mut dl_progress_bar = multi_bar.add(dl_progress.create_download_progress_bar().await);
        
    let mut existing_indices = BTreeMap::<FilePath, FilePath>::new();

    for index_file_path in indices.into_iter().filter(|f| f.exists()) {
        let file_stem = index_file_path.file_stem();
        let path_with_stem = FilePath(format_compact!(
            "{}/{}", 
            index_file_path.parent().unwrap(), 
            file_stem
        ));

        if let Some(val) = existing_indices.get_mut(&path_with_stem) {
            if is_extension_preferred(val.extension(), index_file_path.extension()) {
                *val = index_file_path
            }
        } else {
            existing_indices.insert(path_with_stem, index_file_path);
        }
    }

    file_progress.files.inc_total(existing_indices.len() as u64);

    let packages_files = existing_indices.into_values()
        .map(IndexSource::from)
        .map(|v| v.into_reader())
        .collect::<Result<Vec<_>>>()?;

    let total_size = packages_files.iter().map(|v| v.size()).sum();
    let mut incremental_size_base = 0;

    file_progress.bytes.inc_total(total_size);

    let task_repo = repo.clone();
    let mut task_dl_progress_bar = dl_progress_bar.clone();
    let task_dl_progress = dl_progress.clone();

    let async_handle = Handle::current();
    spawn_blocking(move || {
        for packages_file in packages_files {
            let counter = packages_file.counter();
            file_progress.update_for_bytes(&mut file_progress_bar);
            let package_size = packages_file.size();
    
            for package in packages_file {
                let package = package?;
    
                let dl = task_repo.create_file_download(package);
                async_handle.block_on(async {
                    downloader.queue(dl).await
                })?;
                
                file_progress.bytes.set_success(counter.load(Ordering::SeqCst) + incremental_size_base);
    
                task_dl_progress.update_for_files(&mut task_dl_progress_bar);
                file_progress.update_for_bytes(&mut file_progress_bar);
            }
    
            incremental_size_base += package_size;
            file_progress.update_for_bytes(&mut file_progress_bar);
        }

        Ok::<(), MirsError>(())
    }).await??;

    dl_progress.wait_for_completion(&mut dl_progress_bar).await;

    Ok(())
}

pub async fn download_release(repository: &Repository, downloader: &mut Downloader, opts: &MirrorOpts, cli_opts: &CliOpts, key_store: &Option<PgpKeyStore>) -> Result<Option<Release>> {
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

    if opts.pgp_verify {
        if repository.has_specified_pgp_key() {
            verify_release_signature(&files, repository)?;
        } else {
            let Some(key_store) = key_store else {
                return Err(MirsError::PgpNotVerified)
            };

            verify_release_signature(&files, key_store)?;
        }
    }

    let Some(release_file) = get_release_file(&files) else {
        return Err(MirsError::NoReleaseFile)
    };

    // if the release file we already have has the same checksum as the one we downloaded, because
    // of how all metadata files are moved into the repository path after the mirroring operation
    // is completed successfully, there should be nothing more to do. save bandwidth, save lives!
    let old_release = if let Some(local_release_file) = repository.tmp_to_root(release_file) {
        if local_release_file.exists() && !cli_opts.force {
            let tmp_checksum = Checksum::checksum_file(&local_release_file).await?;
            let local_checksum = Checksum::checksum_file(release_file).await?;

            if tmp_checksum == local_checksum {
                return Ok(None)
            }

            Some(
                Release::parse(&local_release_file).await
                    .map_err(|e| MirsError::InvalidReleaseFile { inner: Box::new(e) })?
            )
        } else {
            None
        }
    } else {
        None
    };

    let mut release = Release::parse(release_file).await
        .map_err(|e| MirsError::InvalidReleaseFile { inner: Box::new(e) })?;

    if let Some(old_release) = old_release {
        release.deduplicate(old_release);
    }

    Ok(Some(release))
}

fn is_extension_preferred(old: Option<&str>, new: Option<&str>) -> bool {
    matches!((old, new),
        (_, Some("gz")) |
        (_, Some("xz")) |
        (_, Some("bz2")) 
    )
}

fn is_packages_file(path: &str) -> bool {
    path.ends_with("Packages") ||
        path.ends_with("Packages.gz") || 
        path.ends_with("Packages.xz") ||
        path.ends_with("Packages.bz2")
}

fn is_index_file(path: &str) -> bool {
    path.ends_with("Index")
}

fn is_debian_installer_file(path: &str) -> bool {
    path.contains("installer-") &&
        path.ends_with("SUMS")
}

fn is_sources_file(path: &str) -> bool {
    path.ends_with("Sources") || 
        path.ends_with("Sources.gz") ||
        path.ends_with("Sources.xz") ||
        path.ends_with("Sources.bz2")
}

fn get_release_file(files: &Vec<FilePath>) -> Option<&FilePath> {
    for file in files {
        if let "InRelease" | "Release" = file.file_name() {
            return Some(file)
        }
    }

    None
}