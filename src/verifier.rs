
use std::sync::Arc;

use async_channel::{bounded, Sender, Receiver};
use tokio::{io::AsyncReadExt, task::JoinHandle};

use crate::{error::{MirsError, Result}, metadata::{checksum::Checksum, FilePath, IndexFileEntry}};

use super::progress::Progress;

#[derive(Clone)]
pub struct Verifier {
    sender: Sender<Box<VerifyTask>>,
    _tasks: Arc<Vec<JoinHandle<()>>>,
    progress: Progress
}

impl Default for Verifier {
    fn default() -> Self {
        let (sender, _) = bounded(1);
        Self {
            sender,
            _tasks: Default::default(),
            progress: Default::default()
        }
    }
}

impl Verifier {
    pub fn build(num_threads: u8) -> Self {
        let (sender, receiver) = bounded(1024);

        let mut tasks = Vec::with_capacity(num_threads as usize);
        let progress = Progress::new();

        for _ in 0..num_threads {
            let task_receiver: Receiver<Box<VerifyTask>> = receiver.clone();
            let task_progress = progress.clone();

            let handle = tokio::spawn(async move {
                let mut buf = vec![0u8; 1024*1024];

                while let Ok(task) = task_receiver.recv().await {
                    let file_size = task.size;

                    let path = task.paths.first().unwrap().clone();
                    match verify_file(&mut buf, task, 
                        |downloaded| task_progress.bytes.inc_success(downloaded)
                    ).await {
                        Ok(true) => task_progress.files.inc_success(1),
                        Ok(false) => {
                            task_progress.files.inc_failed(1);
                            eprintln!("checksum failed: {path}");
                        },
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
            progress
        }
    }

    pub async fn queue(&self, verify_task: Box<VerifyTask>) -> Result<()> {
        if let Some(size) = verify_task.size {
            self.progress.bytes.inc_total(size);
        }

        self.progress.files.inc_total(1);

        self.sender.send(verify_task).await?;

        Ok(())
    }

    pub fn progress(&self) -> Progress {
        self.progress.clone()
    }
}

async fn verify_file<F>(buf: &mut [u8], verify_task: Box<VerifyTask>, mut progress_cb: F) -> Result<bool>
    where F: FnMut(u64) {
    
    for path in &verify_task.paths {
        let mut file = tokio::fs::File::open(path).await?;

        if verify_task.size.is_some_and(|v| v > 0) || verify_task.size.is_none() {
    
            let mut hasher = verify_task.checksum.create_hasher();
    
            loop {
                match file.read(buf).await {
                    Ok(0) => break,
                    Ok(n) => {
                        progress_cb(n as u64);
                        hasher.consume(&buf[..n]);
                    },
                    Err(e) => {
                        return Err(e.into())
                    }
                }
            }
        
            let checksum = hasher.compute();
    
            if verify_task.checksum != checksum {
                return Ok(false)
            }
        }
    }
    
    Ok(true)
}

#[derive(Debug)]
pub struct VerifyTask {
    pub size: Option<u64>,
    pub checksum: Checksum,
    pub paths: Vec<FilePath>,
}

impl TryFrom<IndexFileEntry> for VerifyTask {
    type Error = MirsError;

    fn try_from(value: IndexFileEntry) -> std::result::Result<Self, Self::Error> {
        Ok(Self {
            size: value.size,
            checksum: value.checksum.ok_or_else(|| MirsError::VerifyTask { path: FilePath(value.path.clone()) })?,
            paths: vec![FilePath(value.path)],
        })
    }
} 