use std::{sync::{Arc, atomic::{AtomicU64, Ordering, AtomicU8}}, time::Duration};

use console::{style, pad_str};
use indicatif::{ProgressBar, ProgressStyle, ProgressFinish, HumanBytes};
use tokio::{sync::Mutex, time::sleep};

pub const TOTAL_STEPS: u8 = 3;

#[derive(Clone)]
pub struct Progress {
    step: Arc<AtomicU8>,
    step_name: Arc<Mutex<String>>,
    pub files: ProgressPart,
    pub bytes: ProgressPart
}

impl Progress {
    pub fn new() -> Self {
        Self {
            step_name: Arc::new(Mutex::new(String::new())),
            step: Arc::new(AtomicU8::new(0)),
            files: ProgressPart::new(),
            bytes: ProgressPart::new()
        }
    }

    pub fn new_with_step(step: u8, step_name: &str) -> Self {
        Self {
            step_name: Arc::new(Mutex::new(step_name.to_string())),
            step: Arc::new(AtomicU8::new(step)),
            files: ProgressPart::new(),
            bytes: ProgressPart::new()
        }
    }

    pub async fn create_prefix(&self) -> String {
        pad_str(
            &style(format!(
                "[{}/{TOTAL_STEPS}] {}", 
                self.step.load(Ordering::SeqCst), 
                self.step_name.lock().await)
            ).bold().to_string(), 
            26, 
            console::Alignment::Left, 
            None
        ).to_string()
    }

    pub async fn create_processing_progress_bar(&self) -> ProgressBar {
        let prefix = self.create_prefix().await;

        ProgressBar::new(self.files.total())
            .with_style(
                ProgressStyle::default_bar()
                    .template(
                        "{prefix} [{wide_bar:.green/dim}] {pos}/{len}",
                    )
                    .expect("template is correct")
                    .progress_chars("###"),
            )
            .with_finish(ProgressFinish::AndLeave)
            .with_prefix(prefix)
    }

    pub async fn create_download_progress_bar(&self) -> ProgressBar {
        let prefix = self.create_prefix().await;

        ProgressBar::new(self.files.total())
            .with_style(
                ProgressStyle::default_bar()
                    .template(
                        "{prefix} [{wide_bar:.cyan/dim}] {pos}/{len} [{elapsed_precise}] [{msg}]",
                    )
                    .expect("template is correct")
                    .progress_chars("###"),
                    
            )
            .with_finish(ProgressFinish::AndLeave)
            .with_prefix(prefix)
    }

    pub fn update_progress_bar(&self, progress_bar: &mut ProgressBar) {
        progress_bar.set_length(self.files.total());
        progress_bar.set_position(self.files.success());
        progress_bar.set_message(HumanBytes(self.bytes.success()).to_string());
    }

    pub async fn next_step(&mut self, step_name: &str) {
        *self.step_name.lock().await = step_name.to_string();

        self.bytes.reset();
        self.files.reset();

        self.step.fetch_add(1, Ordering::SeqCst);
    }
    
    pub async fn wait_for_completion(&self, progress_bar: &mut ProgressBar)  {
        while self.files.remaining() > 0 {
            self.update_progress_bar(progress_bar);
            sleep(Duration::from_millis(100)).await
        }

        self.update_progress_bar(progress_bar);

        progress_bar.finish_using_style();
    }
}

#[derive(Clone)]
pub struct ProgressPart {
    total: Arc<AtomicU64>,
    success: Arc<AtomicU64>,
    skipped: Arc<AtomicU64>
}

impl ProgressPart {
    pub fn new() -> Self {
        Self {
            total: Arc::new(AtomicU64::new(0)),
            success: Arc::new(AtomicU64::new(0)),
            skipped: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn inc_total(&mut self, count: u64) {
        self.total.fetch_add(count, Ordering::SeqCst);
    }

    pub fn inc_success(&mut self, count: u64) {
        self.success.fetch_add(count, Ordering::SeqCst);
    }

    pub fn inc_skipped(&mut self, count: u64) {
        self.skipped.fetch_add(count, Ordering::SeqCst);
    }

    pub fn total(&self) -> u64 {
        self.total.load(Ordering::SeqCst)
    }

    pub fn success(&self) -> u64 {
        self.success.load(Ordering::SeqCst)
    }

    pub fn remaining(&self) -> u64 {
        self.total.load(Ordering::SeqCst) -
            self.success.load(Ordering::SeqCst) -
            self.skipped.load(Ordering::SeqCst)
    }

    pub fn reset(&mut self) {
        self.total.store(0, Ordering::SeqCst);
        self.success.store(0, Ordering::SeqCst);
        self.skipped.store(0, Ordering::SeqCst);
    }
}