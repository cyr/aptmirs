use std::sync::Arc;

use ahash::HashSet;
use async_trait::async_trait;
use compact_str::format_compact;

use crate::{context::Context, error::MirsError, metadata::{metadata_file::{deduplicate_metadata, MetadataFile}, release::Release, FilePath}, step::{Step, StepResult}};
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
        

        for (opts, repo) in &ctx.state.mirrors {
            let dist_root = FilePath(format_compact!("{}/dists/{}", repo.root_dir, opts.suite));
            let dist_rel = FilePath(format_compact!("dists/{}", opts.suite));

            let release_files = get_rooted_release_files(&dist_root);

            let Some(release_file) = pick_release(&release_files) else {
                return Err(MirsError::NoReleaseFile)
            };

            let release = Release::parse(release_file).await?;

            let metadata: Vec<MetadataFile> = release.into_filtered_files(opts)
                .map(|(path, _)| path)
                .collect();

            {
                let mut state = ctx.state.output.lock().await;
                for f in release_files {
                    let path = f.as_str().strip_prefix(repo.root_dir.as_str()).expect("path is in root");
                    state.files.insert(path.into());
                }

                for metadata_file in &metadata {
                    let path = dist_rel.join(metadata_file);
                    state.files.insert(path);
                }
            }

            let metadata: Vec<MetadataFile> = deduplicate_metadata(
                metadata.into_iter()
                    .filter(|v| v.is_index())
                    .collect()
            );
    
            let index_files = metadata.into_iter()
                .map(|mut v| {
                    v.prefix_with(dist_root.as_str());
                    v
                })
                .map(MetadataFile::into_reader)
                .collect::<Result<Vec<_>>>()?;
            
            for mut meta_file in index_files {
                
                
            }

            
        }

        todo!()

        
    }
}

fn add_valid_file(files: &mut HashSet<FilePath>, file: FilePath) {
    files.insert(file);
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