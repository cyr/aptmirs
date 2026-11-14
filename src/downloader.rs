use std::{
    fs::FileTimes,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use async_channel::{Receiver, Sender, bounded};
use compact_str::ToCompactString;
use reqwest::{Client, RequestBuilder, Response, StatusCode};
use tokio::{fs::symlink, io::AsyncWriteExt, task::JoinHandle};
use url::Url;

use crate::{
    apt_auth::AptAuth,
    error::{MirsError, Result},
    metadata::{
        FilePath,
        checksum::{Checksum, Hasher},
    },
};

use super::progress::Progress;

fn now() -> Arc<AtomicU64> {
    Arc::new(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("now should always be after unix epoch")
            .as_secs()
            .into(),
    )
}

#[derive(Debug, Default, Clone)]
pub struct HttpClient {
    pub client: Client,
    auth: Arc<Vec<AptAuth>>,
}

impl HttpClient {
    pub fn get(&self, url: Url) -> RequestBuilder {
        let auth: Option<&AptAuth> = self.auth.iter().find(|v| v.matches(&url));

        let mut builder = self.client.get(url);

        if let Some(auth) = auth {
            builder = builder.basic_auth(auth.username.clone(), Some(auth.password.clone()));
        }

        builder
    }
}

#[derive(Clone)]
pub struct Downloader {
    sender: Sender<Box<Download>>,
    _tasks: Arc<Vec<JoinHandle<()>>>,
    progress: Progress,
    http_client: HttpClient,
    pub time_to_set: Arc<AtomicU64>,
    mtime: bool,
}

impl Default for Downloader {
    fn default() -> Self {
        let (sender, _) = bounded(1);
        Self {
            sender,
            _tasks: Arc::default(),
            progress: Progress::default(),
            http_client: HttpClient::default(),
            time_to_set: now(),
            mtime: false,
        }
    }
}

impl Downloader {
    pub fn build(num_threads: u8, mtime: bool, apt_auth: Vec<AptAuth>) -> Self {
        let (sender, receiver) = bounded(1024);

        let mut tasks = Vec::with_capacity(num_threads as usize);
        let progress = Progress::new();
        let http_client = HttpClient {
            client: reqwest::ClientBuilder::new()
                .read_timeout(Duration::from_secs(10))
                .build()
                .expect("reqwest client should be buildable"),
            auth: Arc::new(apt_auth),
        };

        let time_to_set = now();

        for _ in 0..num_threads {
            let task_receiver: Receiver<Box<Download>> = receiver.clone();
            let task_progress = progress.clone();
            let task_http_client = http_client.clone();
            let task_time = if mtime {
                Some(time_to_set.clone())
            } else {
                None
            };

            let handle = tokio::spawn(async move {
                while let Ok(dl) = task_receiver.recv().await {
                    _ = Downloader::download_and_track(
                        &task_http_client,
                        task_time.clone(),
                        task_progress.clone(),
                        dl,
                    )
                    .await;
                }
            });

            tasks.push(handle);
        }

        Self {
            sender,
            _tasks: Arc::new(tasks),
            progress,
            http_client,
            time_to_set,
            mtime,
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

    async fn download_and_track(
        http_client: &HttpClient,
        time: Option<Arc<AtomicU64>>,
        progress: Progress,
        dl: Box<Download>,
    ) {
        let file_size = dl.size;

        match download_file(http_client, time, dl, |downloaded| {
            progress.bytes.inc_success(downloaded);
        })
        .await
        {
            Ok(true) => progress.files.inc_success(1),
            Ok(false) => progress.files.inc_skipped(1),
            Err(e) => match e {
                MirsError::Checksum { .. } => progress.files.inc_failed(1),
                MirsError::Download { .. } => {
                    if let Some(size) = file_size {
                        progress.bytes.inc_skipped(size);
                    }

                    progress.files.inc_skipped(1);
                }
                _ => progress.files.inc_skipped(1),
            },
        }
    }

    pub async fn download(&self, download: Box<Download>) {
        let time = if self.mtime {
            Some(self.time_to_set.clone())
        } else {
            None
        };

        Downloader::download_and_track(&self.http_client, time, self.progress.clone(), download)
            .await;
    }

    pub fn progress(&self) -> Progress {
        self.progress.clone()
    }

    pub fn set_time(&self, new_time: u64) {
        self.time_to_set.store(new_time, Ordering::Relaxed);
    }
}

async fn download_file<F>(
    http_client: &HttpClient,
    time: Option<Arc<AtomicU64>>,
    download: Box<Download>,
    progress_cb: F,
) -> Result<bool>
where
    F: FnMut(u64),
{
    let mut downloaded = false;

    if needs_downloading(&download) {
        create_dirs(&download.primary_target_path).await?;

        let mut output = tokio::fs::File::create(&download.primary_target_path).await?;

        if download.size.is_none_or(|v| v > 0) {
            if let Err(e) = get(http_client, &mut output, &download, progress_cb).await {
                delete_file(output, &download.primary_target_path).await;
                return Err(e);
            }

            output.flush().await?;
            downloaded = true;
        }

        if let Some(time_to_set) = time {
            set_time(time_to_set, output).await?;
        }
    }

    for symlink_path in download.symlink_paths {
        if symlink_path.exists() {
            continue;
        }

        let rel_primary_path = pathdiff::diff_paths(
            &download.primary_target_path,
            symlink_path.parent().expect("base dir needs to exist"),
        )
        .expect("all files will be in some relative path");

        create_dirs(&symlink_path).await?;

        symlink(&rel_primary_path, &symlink_path).await?;
    }

    Ok(downloaded)
}

async fn get<F>(
    http_client: &HttpClient,
    output: &mut tokio::fs::File,
    download: &Download,
    progress_cb: F,
) -> Result<()>
where
    F: FnMut(u64),
{
    let Ok(response) = http_client.get(download.url.clone()).send().await else {
        return Err(MirsError::Download {
            url: download.url.to_string(),
            status_code: None,
        });
    };

    if response.status() != StatusCode::OK {
        return Err(MirsError::Download {
            url: response.url().to_string(),
            status_code: Some(response.status()),
        });
    }

    if let Some(expected_checksum) = &download.checksum {
        let mut hasher = expected_checksum.create_hasher();

        write_from_response_with_hasher(response, output, &mut hasher, progress_cb).await?;

        let checksum = hasher.compute();

        if expected_checksum != &checksum {
            return Err(MirsError::Checksum {
                url: download.url.to_string(),
                expected: expected_checksum.to_compact_string(),
                hash: checksum.to_string(),
            });
        }
    } else {
        write_from_response(response, output, progress_cb).await?;
    }

    Ok(())
}

async fn set_time(time_to_set: Arc<AtomicU64>, output: tokio::fs::File) -> Result<()> {
    let time = time_from_atomic(&time_to_set);
    let std_file = output.into_std().await;
    tokio::task::spawn_blocking(move || std_file.set_times(FileTimes::new().set_modified(time)))
        .await??;

    Ok(())
}

async fn delete_file(output: tokio::fs::File, path: &FilePath) {
    drop(output);
    _ = tokio::fs::remove_file(path).await;
}

async fn write_from_response<F>(
    mut response: Response,
    output: &mut tokio::fs::File,
    mut progress_cb: F,
) -> Result<()>
where
    F: FnMut(u64),
{
    while let Some(chunk) = response.chunk().await? {
        output.write_all(&chunk).await?;

        progress_cb(chunk.len() as u64);
    }

    Ok(())
}

async fn write_from_response_with_hasher<F>(
    mut response: Response,
    output: &mut tokio::fs::File,
    hasher: &mut Box<dyn Hasher>,
    mut progress_cb: F,
) -> Result<()>
where
    F: FnMut(u64),
{
    while let Some(chunk) = response.chunk().await? {
        output.write_all(&chunk).await?;
        hasher.consume(&chunk);

        progress_cb(chunk.len() as u64);
    }

    Ok(())
}

pub fn time_from_atomic(time: &Arc<AtomicU64>) -> SystemTime {
    SystemTime::UNIX_EPOCH
        .checked_add(Duration::from_secs(time.load(Ordering::Relaxed)))
        .expect("reversing stored timestamp should always be safe")
}

pub async fn create_dirs<P: AsRef<Path>>(path: P) -> Result<()> {
    if let Some(parent_dir) = path.as_ref().parent()
        && !parent_dir.exists()
    {
        tokio::fs::create_dir_all(parent_dir).await?;
    }

    Ok(())
}

fn needs_downloading(dl: &Download) -> bool {
    if dl.always_download {
        return true;
    }

    if let Ok(metadata) = dl.primary_target_path.metadata() {
        if let Some(size) = dl.size {
            return size != metadata.len();
        }

        return false;
    }

    true
}

#[derive(Debug)]
pub struct Download {
    pub url: Url,
    pub size: Option<u64>,
    pub checksum: Option<Checksum>,
    pub primary_target_path: FilePath,
    pub symlink_paths: Vec<FilePath>,
    pub always_download: bool,
}
