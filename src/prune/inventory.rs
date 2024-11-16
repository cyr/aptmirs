use std::sync::{atomic::Ordering, Arc};

use ahash::HashSet;
use async_trait::async_trait;
use compact_str::format_compact;

use crate::{context::Context, error::MirsError, metadata::{metadata_file::{deduplicate_metadata, MetadataFile}, release::{FileEntry, Release}, FilePath}, mirror::verify_and_prune, progress::Progress, step::{Step, StepResult}};
use crate::error::Result;

use super::{PruneResult, PruneState};

pub struct Inventory;

#[async_trait]
impl Step<PruneState> for Inventory {
    type Result = PruneResult;
    
    fn step_name(&self) -> &'static str {
        "Taking inventory"
    }
    
    fn error(&self, e: MirsError) -> Self::Result {
        PruneResult::Error(MirsError::Inventory { inner: Box::new(e) })
    }

    async fn execute(&self, ctx: Arc<Context<PruneState>>) -> Result<StepResult<Self::Result>> {
        let mut progress = ctx.progress.clone();
        let mut state = ctx.state.output.lock().await;

        let mut progress_bar = progress.create_count_progress_bar().await;

        let mut incremental_size_base = 0;

        for (opts, repo) in &ctx.state.mirrors {
            let dist_root = FilePath(format_compact!("{}/dists/{}", repo.root_dir, opts.suite));

            let release_files = get_rooted_release_files(&dist_root);

            let Some(release_file) = pick_release(&release_files) else {
                return Err(MirsError::NoReleaseFile)
            };

            let release = Release::parse(release_file).await?;

            let by_hash = release.acquire_by_hash();

            let mut metadata: Vec<(MetadataFile, FileEntry)> = release.into_filtered_files(opts).collect();

            for f in release_files {
                add_valid_metadata_file(&mut progress, &mut state.files, &f, repo.root_dir.as_str());
            }

            for (metadata_file, file_entry) in &mut metadata {
                metadata_file.prefix_with(dist_root.as_str());

                let (_, primary, other) = file_entry.into_paths(metadata_file.path(), by_hash)?;

                add_valid_metadata_file(&mut progress, &mut state.files, &primary, repo.root_dir.as_str());

                for f in other {
                    add_valid_metadata_file(&mut progress, &mut state.files, &f, repo.root_dir.as_str());
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
                    MetadataFile::Sources(..) => FilePath::from(""),
                    MetadataFile::SumFile(file_path) |
                    MetadataFile::DiffIndex(file_path) => {
                        FilePath::from(
                            file_path.parent()
                                .expect("diff indicies should have parents")
                                .strip_prefix(repo.root_dir.as_str())
                                .expect("metadata paths should be rooted")
                        )
                    },
                    MetadataFile::Other(..) => unreachable!()
                };
                
                for entry in meta_file {
                    let entry = entry?;

                    let path = base_path.join(entry.path);

                    add_valid_file(&mut progress, &mut state.files, path);

                    progress.bytes.set_success(counter.load(Ordering::SeqCst) + incremental_size_base);

                    progress.update_for_count(&mut progress_bar);
                }

                incremental_size_base += meta_file_size;
            }
        }

        state.total_valid_files += state.files.len() as u64;
        
        progress_bar.finish_using_style();

        Ok(StepResult::Continue)
    }
}

fn add_valid_metadata_file(progress: &mut Progress, files: &mut HashSet<FilePath>, file: &FilePath, root_dir: &str) {
    let path = file.as_str().strip_prefix(root_dir).expect("path is in root");

    add_valid_file(progress, files, path.into());
}

fn add_valid_file(progress: &mut Progress, files: &mut HashSet<FilePath>, file: FilePath) {
    if files.insert(file) {
        progress.files.inc_success(1);
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