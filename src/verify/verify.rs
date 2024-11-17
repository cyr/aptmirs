use std::sync::{atomic::Ordering, Arc};

use async_trait::async_trait;
use compact_str::format_compact;
use tokio::io::AsyncReadExt;

use crate::{context::Context, error::MirsError, metadata::{checksum::Checksum, metadata_file::{deduplicate_metadata, MetadataFile}, release::{FileEntry, Release}, FilePath}, mirror::verify_and_prune, progress::Progress, step::{Step, StepResult}};
use crate::error::Result;

use super::{VerifyResult, VerifyState};

pub struct Verify;

#[async_trait]
impl Step<VerifyState> for Verify {
    type Result = VerifyResult;
    
    fn step_name(&self) -> &'static str {
        "Verifying"
    }
    
    fn error(&self, e: MirsError) -> Self::Result {
        VerifyResult::Error(MirsError::Verify { inner: Box::new(e) })
    }

    async fn execute(&self, ctx: Arc<Context<VerifyState>>) -> Result<StepResult<Self::Result>> {
        let progress = ctx.progress.clone();
        let mut buf = vec![0u8; 1024*1024];
        let mut output = ctx.state.output.lock().await;

        let mut progress_bar = progress.create_count_progress_bar().await;

        let mut incremental_size_base = 0;

        let dist_root = FilePath(format_compact!("{}/dists/{}", ctx.state.repo.root_dir, ctx.state.opts.suite));

        let release_files = get_rooted_release_files(&dist_root);

        let Some(release_file) = pick_release(&release_files) else {
            return Err(MirsError::NoReleaseFile)
        };

        let release = Release::parse(release_file).await?;

        let by_hash = release.acquire_by_hash();

        let mut metadata: Vec<(MetadataFile, FileEntry)> = release.into_filtered_files(&ctx.state.opts).collect();

        for (metadata_file, file_entry) in &mut metadata {
            metadata_file.prefix_with(dist_root.as_str());

            let (checksum, primary, other) = file_entry.into_paths(metadata_file.path(), by_hash)?;

            verify_file(&progress, &mut buf, &primary, &checksum).await?;

            if !matches!(metadata_file, MetadataFile::SumFile(..)) {
                for f in other {
                    verify_file(&progress, &mut buf, &f, &checksum).await?;
                }
            }
        }

        let mut metadata = metadata.into_iter()
            .map(|(v, _)| v)
            .filter(MetadataFile::is_index)
            .collect();

        verify_and_prune(&mut metadata);

        let metadata = deduplicate_metadata(metadata);

        let index_files = metadata.into_iter()
            .map(MetadataFile::into_reader)
            .collect::<Result<Vec<_>>>()?;
        
        let total_size = index_files.iter().map(|v| v.size()).sum();
        progress.bytes.inc_total(total_size);
        
        for meta_file in index_files {
            let counter = meta_file.counter();
            let meta_file_size = meta_file.size();

            let base_path = match meta_file.file() {
                MetadataFile::Packages(..) |
                MetadataFile::Sources(..) => ctx.state.repo.root_dir.clone(),
                MetadataFile::SumFile(file_path) |
                MetadataFile::DiffIndex(file_path) => {
                    FilePath::from(file_path.parent().expect("diff indicies should have parents"))
                },
                MetadataFile::Other(..) => unreachable!()
            };
            
            for entry in meta_file {
                let entry = entry?;

                let path = base_path.join(entry.path);

                verify_file(&progress, &mut buf, &path, &entry.checksum).await?;

                progress.bytes.set_success(counter.load(Ordering::SeqCst) + incremental_size_base);

                progress.update_for_count(&mut progress_bar);
            }

            incremental_size_base += meta_file_size;
        }
        
        progress_bar.finish_using_style();

        output.total_corrupt = progress.files.failed();
        output.total_missing = progress.files.skipped();
        output.total_valid = progress.files.success();

        Ok(StepResult::Continue)
    }
}

fn get_rooted_release_files(root: &FilePath) -> Vec<FilePath> {
    [
        root.join("InRelease"),
        root.join("Release"),
        root.join("Release.gpg")
    ].into_iter()
        .filter(|v| v.exists())
        .collect()
}

fn pick_release(files: &[FilePath]) -> Option<&FilePath> {
    for f in files {
        if let "InRelease" | "Release" = f.file_name() {
            return Some(f)
        }
    }

    None
}

pub async fn verify_file(progress: &Progress, buf: &mut [u8], file: &FilePath, checksum: &Option<Checksum>) -> Result<()> {
    if !file.exists() {
        eprintln!("missing: {}", file.as_str());
        progress.files.inc_skipped(1);
        return Ok(())
    }

    let expected_checksum = checksum.as_ref().unwrap();

    let mut hasher = expected_checksum.create_hasher();

    let mut f = tokio::fs::File::open(file).await?;

    loop {
        match f.read(buf).await {
            Ok(0) => break,
            Ok(n) => hasher.consume(&buf[..n]),
            Err(e) => {
                return Err(e.into())
            }
        }
    }

    let checksum = hasher.compute();

    if checksum == *expected_checksum {
        progress.files.inc_success(1);
    } else {
        eprintln!("checksum failed: {}", file.as_str());
        progress.files.inc_failed(1);
    }

    Ok(())
}
