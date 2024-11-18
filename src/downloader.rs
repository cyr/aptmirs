
use std::{path::Path, sync::Arc};

use async_channel::{bounded, Sender, Receiver};
use compact_str::{CompactString, ToCompactString};
use reqwest::{Client, StatusCode};
use tokio::{task::JoinHandle, io::AsyncWriteExt, fs::symlink};

use crate::{error::{MirsError, Result}, metadata::{checksum::Checksum, FilePath}};

use super::progress::Progress;

#[derive(Clone)]
pub struct Downloader {
    sender: Sender<Box<Download>>,
    _tasks: Arc<Vec<JoinHandle<()>>>,
    progress: Progress,
    http_client: Client
}

impl Default for Downloader {
    fn default() -> Self {
        let (sender, _) = bounded(1);
        Self {
            sender,
            _tasks: Default::default(),
            progress: Default::default(),
            http_client: Default::default()
        }
    }
}

impl Downloader {
    pub fn build(num_threads: u8) -> Self {
        let (sender, receiver) = bounded(1024);

        let mut tasks = Vec::with_capacity(num_threads as usize);
        let progress = Progress::new();
        let http_client = reqwest::Client::new();

        for _ in 0..num_threads {
            let task_receiver: Receiver<Box<Download>> = receiver.clone();
            let task_progress = progress.clone();
            let task_http_client = http_client.clone();

            let handle = tokio::spawn(async move {
                while let Ok(dl) = task_receiver.recv().await {
                    let file_size = dl.size;

                    match download_file(&task_http_client, dl, 
                        |downloaded| task_progress.bytes.inc_success(downloaded)
                    ).await {
                        Ok(true) => task_progress.files.inc_success(1),
                        Ok(false) => task_progress.files.inc_skipped(1),
                        Err(e) => {
                            if let MirsError::Download { .. } = e {
                                if let Some(size) = file_size {
                                    task_progress.bytes.inc_skipped(size);
                                }
                            }
    
                            task_progress.files.inc_skipped(1);
                        }
                    }
                }
            });

            tasks.push(handle);
        }

        Self {
            sender,
            _tasks: Arc::new(tasks),
            progress,
            http_client
        }
    }

    pub async fn queue(&self, download_entry: Box<Download>) -> Result<()> {
        if let Some(size) = download_entry.size {
            self.progress.bytes.inc_total(size);
        }

        self.progress.files.inc_total(1);

        self.sender.send(download_entry).await?;

        Ok(())
    }

    pub async fn download(&self, download: Box<Download>) -> Result<()> {
        match download_file(&self.http_client, download, |bytes| {
            self.progress.bytes.inc_success(bytes)
        }).await {
            Ok(downloaded) => {
                if downloaded {
                    self.progress.files.inc_success(1);
                } else {
                    self.progress.files.inc_skipped(1);
                }
            },
            Err(e) => {
                self.progress.files.inc_skipped(1);
                return Err(e)
            },
        }
        
        Ok(())
    }

    pub fn progress(&self) -> Progress {
        self.progress.clone()
    }
}

async fn download_file<F>(http_client: &Client, download: Box<Download>, mut progress_cb: F) -> Result<bool>
    where F: FnMut(u64) {
    
    let mut downloaded = false;

    if needs_downloading(&download) {
        create_dirs(&download.primary_target_path).await?;

        let mut output = tokio::fs::File::create(&download.primary_target_path).await?;

        if download.size.is_some_and(|v| v > 0) || download.size.is_none() {
            let mut response = http_client.get(download.url.as_str()).send().await?;

            if response.status() == StatusCode::NOT_FOUND {
                drop(output);
                tokio::fs::remove_file(&download.primary_target_path).await?;
                return Err(MirsError::Download { url: download.url.clone(), status_code: response.status() })
            }

            if let Some(expected_checksum) = download.checksum {
                let mut hasher = expected_checksum.create_hasher();

                while let Some(chunk) = response.chunk().await? {
                    output.write_all(&chunk).await?;
                    hasher.consume(&chunk);
            
                    progress_cb(chunk.len() as u64);
                }

                let checksum = hasher.compute();

                if expected_checksum != checksum {
                    drop(output);
                    tokio::fs::remove_file(&download.primary_target_path).await?;
                    return Err(MirsError::Checksum { 
                        url: download.url, 
                        expected: expected_checksum.to_compact_string(), 
                        hash: checksum.to_string() 
                    })
                }
            } else {
                while let Some(chunk) = response.chunk().await? {
                    output.write_all(&chunk).await?;
            
                    progress_cb(chunk.len() as u64);
                }
            }
        
            output.flush().await?;
            downloaded = true;
        }
    }

    for symlink_path in download.symlink_paths {
        if symlink_path.exists() {
            continue
        }

        let rel_primary_path = pathdiff::diff_paths(
            &download.primary_target_path,
            symlink_path.parent().expect("base dir needs to exist"),
        ).expect("all files will be in some relative path");

        create_dirs(&symlink_path).await?;
        
        symlink(&rel_primary_path, &symlink_path).await?;
    }
    
    Ok(downloaded)
}

pub async fn create_dirs<P: AsRef<Path>>(path: P) -> Result<()> {
    if let Some(parent_dir) = path.as_ref().parent() {
        if !parent_dir.exists() {
            tokio::fs::create_dir_all(parent_dir).await?;
        }
    }

    Ok(())
}

fn needs_downloading(dl: &Download) -> bool {
    if dl.always_download {
        return true
    }

    if let Ok(metadata) = dl.primary_target_path.metadata() {
        if let Some(size) = dl.size {
            return size != metadata.len()
        }

        return false
    }

    true
}

#[derive(Debug)]
pub struct Download {
    pub url: CompactString,
    pub size: Option<u64>,
    pub checksum: Option<Checksum>,
    pub primary_target_path: FilePath,
    pub symlink_paths: Vec<FilePath>,
    pub always_download: bool
}