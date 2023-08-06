
use std::path::{Path, PathBuf};

use async_channel::{bounded, Sender, Receiver};
use reqwest::{Client, StatusCode};
use tokio::{task::JoinHandle, io::AsyncWriteExt, fs::symlink};

use crate::error::{Result, MirsError};

use super::progress::Progress;

pub struct Downloader {
    sender: Sender<Download>,
    _tasks: Vec<JoinHandle<()>>,
    progress: Progress,
    http_client: Client
}

impl Downloader {
    pub fn build(num_threads: u8) -> Self {
        let (sender, receiver) = bounded(1024);

        let mut tasks = Vec::with_capacity(num_threads as usize);

        let progress = Progress::new();

        let http_client = reqwest::Client::new();

        for _ in 0..num_threads {
            let task_receiver: Receiver<Download> = receiver.clone();

            let mut task_progress = progress.clone();

            let mut task_http_client = http_client.clone();

            let handle = tokio::spawn(async move {
                while let Ok(dl) = task_receiver.recv().await {
                    let file_size = dl.size;

                    match download_file(&mut task_http_client, dl, 
                        |downloaded| { 
                            task_progress.bytes.inc_success(downloaded);
                        }
                    ).await {
                        Ok(()) => task_progress.files.inc_success(1),
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
            _tasks: tasks,
            progress,
            http_client
        }
    }

    pub async fn queue(&mut self, download_entry: Download) -> Result<()> {
        if let Some(size) = download_entry.size {
            self.progress.bytes.inc_total(size);
        }

        self.progress.files.inc_total(1);

        self.sender.send(download_entry).await?;

        Ok(())
    }

    pub async fn download(&mut self, download: Download) -> Result<()> {
        match download_file(&mut self.http_client, download, |bytes| {
            self.progress.bytes.inc_success(bytes)
        }).await {
            Ok(()) => self.progress.files.inc_success(1),
            Err(e) => {
                self.progress.files.inc_skipped(1);
                return Err(e)
            }
        }
        
        Ok(())
    }


    pub fn progress(&self) -> Progress {
        self.progress.clone()
    }
}

async fn download_file<F>(http_client: &mut Client, download: Download, mut progress_callback: F) -> Result<()>
    where F: FnMut(u64) {
    let mut response = http_client.get(&download.uri).send().await?;

    if response.status() == StatusCode::NOT_FOUND {
        return Err(MirsError::Download { uri: download.uri.clone(), status_code: response.status() })
    }

    if needs_downloading(&download) {
        create_dirs(&download.primary_target_path).await?;

        let mut output = tokio::fs::File::create(&download.primary_target_path).await?;
    
        while let Some(chunk) = response.chunk().await? {
            output.write_all(&chunk).await?;
    
            progress_callback(chunk.len() as u64);
        }
    
        output.flush().await?;
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
    
    Ok(())
}

pub async fn create_dirs(path: &Path) -> Result<()> {
    if let Some(parent_dir) = path.parent() {
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
    pub uri: String,
    pub size: Option<u64>,
    pub primary_target_path: PathBuf,
    pub symlink_paths: Vec<PathBuf>,
    pub always_download: bool
}