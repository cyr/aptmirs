use std::fmt::Display;

pub struct VerifyResult { corrupt_files: u64, missing_files: u64 }

impl Display for VerifyResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!("corrupt {}, missing: {}", self.corrupt_files, self.missing_files))
    }
}