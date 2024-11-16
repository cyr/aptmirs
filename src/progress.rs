use std::{sync::{Arc, atomic::{AtomicU64, Ordering, AtomicU8}}, time::Duration};

use console::{style, pad_str};
use indicatif::{ProgressBar, ProgressStyle, ProgressFinish, HumanBytes};
use tokio::{sync::Mutex, time::sleep};

#[derive(Clone, Default)]
pub struct Progress {
    pub step: Arc<AtomicU8>,
    step_name: Arc<Mutex<String>>,
    pub files: ProgressPart,
    pub bytes: ProgressPart,
    pub total_bytes: Arc<AtomicU64>,
    total_steps: Arc<AtomicU8>
}

impl Progress {
    pub fn new() -> Self {
        Self {
            step_name: Arc::new(Mutex::new(String::new())),
            step: Arc::new(AtomicU8::new(0)),
            files: ProgressPart::new(),
            bytes: ProgressPart::new(),
            total_bytes: Arc::new(AtomicU64::new(0)),
            total_steps: Arc::new(AtomicU8::new(4))
        }
    }

    pub fn new_with_step(step: u8, step_name: &str) -> Self {
        Self {
            step_name: Arc::new(Mutex::new(step_name.to_string())),
            step: Arc::new(AtomicU8::new(step)),
            files: ProgressPart::new(),
            bytes: ProgressPart::new(),
            total_bytes: Arc::new(AtomicU64::new(0)),
            total_steps: Arc::new(AtomicU8::new(4))
        }
    }

    pub async fn create_prefix_stepless(&self) -> String {
        pad_str(
            &style(format!(
                "{}", 
                self.step_name.lock().await)
            ).bold().to_string(), 
            26, 
            console::Alignment::Right, 
            None
        ).to_string()
    }

    pub async fn create_prefix(&self) -> String {
        pad_str(
            &style(format!(
                "[{}/{}] {}", 
                self.step.load(Ordering::SeqCst),
                self.total_steps.load(Ordering::SeqCst), 
                self.step_name.lock().await)
            ).bold().to_string(), 
            26, 
            console::Alignment::Left, 
            None
        ).to_string()
    }

    pub async fn create_processing_progress_bar(&self) -> ProgressBar {
        let prefix = self.create_prefix_stepless().await;

        ProgressBar::new(self.bytes.total())
            .with_style(
                ProgressStyle::default_bar()
                    .template(
                        "{prefix} [{wide_bar:.green/dim}] [{percent}%]",
                    )
                    .expect("template string should follow the syntax")
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
                    .expect("template string should follow the syntax")
                    .progress_chars("###"),
                    
            )
            .with_finish(ProgressFinish::AndLeave)
            .with_prefix(prefix)
    }

    pub fn update_for_files(&self, progress_bar: &mut ProgressBar) {
        progress_bar.set_length(self.files.total());
        progress_bar.set_position(self.files.success());
        progress_bar.set_message(HumanBytes(self.bytes.success()).to_string());
    }

    pub fn update_for_bytes(&self, progress_bar: &mut ProgressBar) {
        progress_bar.set_length(self.bytes.total());
        progress_bar.set_position(self.bytes.success());
        progress_bar.set_message(HumanBytes(self.bytes.success()).to_string());
    }

    pub fn reset(&self) {
        self.bytes.reset();
        self.files.reset();
        self.step.store(0, Ordering::SeqCst);
        self.total_steps.store(5, Ordering::SeqCst);
    }

    pub fn set_total_steps(&self, num_steps: u8) {
        self.total_steps.store(num_steps, Ordering::SeqCst);
    }

    pub async fn next_step(&self, step_name: &str) {
        *self.step_name.lock().await = step_name.to_string();

        self.bytes.reset();
        self.files.reset();

        self.step.fetch_add(1, Ordering::SeqCst);
    }
    
    pub async fn wait_for_completion(&self, progress_bar: &mut ProgressBar)  {
        while self.files.remaining() > 0 {
            self.update_for_files(progress_bar);
            sleep(Duration::from_millis(100)).await
        }

        self.total_bytes.fetch_add(self.bytes.success(), Ordering::SeqCst);

        self.update_for_files(progress_bar);

        progress_bar.finish_using_style();
    }
}

#[derive(Clone, Default, Debug)]
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

    pub fn inc_total(&self, count: u64) {
        self.total.fetch_add(count, Ordering::SeqCst);
    }

    pub fn inc_success(&self, count: u64) {
        self.success.fetch_add(count, Ordering::SeqCst);
    }

    pub fn set_success(&self, count: u64) {
        self.success.store(count, Ordering::SeqCst)
    }

    pub fn inc_skipped(&self, count: u64) {
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

    pub fn reset(&self) {
        self.total.store(0, Ordering::SeqCst);
        self.success.store(0, Ordering::SeqCst);
        self.skipped.store(0, Ordering::SeqCst);
    }
}