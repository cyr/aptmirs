use std::fmt::Display;

use indicatif::HumanBytes;

pub struct PruneResult { total_bytes: u64, total_files: u64 }

impl Display for PruneResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!("pruned {}, total: {}", self.total_files, HumanBytes(self.total_bytes)))
    }
}