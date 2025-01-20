use std::{str::FromStr, sync::Arc};

use compact_str::{format_compact, CompactString, ToCompactString};
use pgp::{cleartext::CleartextSignedMessage, SignedPublicKey, StandaloneSignature};
use reqwest::Url;

use crate::{config::MirrorOpts, downloader::Download, error::{MirsError, Result}, metadata::{checksum::Checksum, release::FileEntry, FilePath, IndexFileEntry}, pgp::{read_public_key, KeyStore}, CliOpts};

#[derive(Default)]
pub struct Repository {
    pub root_url: CompactString,
    pub root_dir: FilePath,
    pub dist_url: CompactString,
    pub tmp_dir: FilePath,
    pub pgp_pub_key: Option<SignedPublicKey>,
}

impl Repository {
    pub fn build(mirror_opts: &MirrorOpts, cli_opts: &CliOpts) -> Result<Self> {
        let root_url = match mirror_opts.url.as_str().strip_prefix('/') {
            Some(url) => url.to_compact_string(),
            None => mirror_opts.url.clone(),
        };

        let dist_url = format_compact!("{root_url}/{}", mirror_opts.dist_part());

        let parsed_url = Url::parse(&root_url)
            .map_err(|_| MirsError::UrlParsing { url: root_url.clone() })?;

        let pgp_pub_key = if let Some(pgp_signing_key) = &mirror_opts.pgp_pub_key {
            let file = FilePath::from_str(pgp_signing_key.as_ref())?;
            Some(read_public_key(&file)?)
        } else {
            None
        };

        let root_dir = local_dir_from_archive_url(&parsed_url, &cli_opts.output)?;

        Ok(Self {
            root_url,
            root_dir,
            dist_url,
            tmp_dir: FilePath::from(""),
            pgp_pub_key
        })
    }

    pub fn build_with_tmp(mirror_opts: &MirrorOpts, cli_opts: &CliOpts) -> Result<Arc<Self>> {
        let root_url = match mirror_opts.url.as_str().strip_prefix('/') {
            Some(url) => url.to_compact_string(),
            None => mirror_opts.url.clone(),
        };

        let dist_url = format_compact!("{root_url}/{}", mirror_opts.dist_part());

        let parsed_url = Url::parse(&root_url)
            .map_err(|_| MirsError::UrlParsing { url: root_url.clone() })?;

        let pgp_pub_key = if let Some(pgp_signing_key) = &mirror_opts.pgp_pub_key {
            let file = FilePath::from_str(pgp_signing_key.as_ref())?;
            Some(read_public_key(&file)?)
        } else {
            None
        };

        let root_dir = local_dir_from_archive_url(&parsed_url, &cli_opts.output)?;
        let tmp_dir = create_tmp_dir(&parsed_url, &mirror_opts.suite, &cli_opts.output)?;

        Ok(Arc::new(Self {
            root_url,
            root_dir,
            dist_url,
            tmp_dir,
            pgp_pub_key
        }))
    }

    pub fn release_urls(&self) -> [CompactString; 3] {
        [
            format_compact!("{}/InRelease", self.dist_url),
            format_compact!("{}/Release", self.dist_url),
            format_compact!("{}/Release.gpg", self.dist_url)
        ]
    }

    pub fn has_specified_pgp_key(&self) -> bool {
        self.pgp_pub_key.is_some()
    }

    fn to_path_in_local_dir(&self, base: &FilePath, url: &str) -> FilePath {
        let relative_path = url
            .strip_prefix(self.root_url.as_str())
            .expect("implementation error; download url should be in archive root");

        let relative_path = match relative_path.strip_prefix('/') {
            Some(path) => path,
            None => relative_path,
        };

        base.join(relative_path)
    }

    pub fn delete_tmp(&self) -> Result<()> {
        if !self.tmp_dir.exists() {
            return Ok(())
        }
        
        std::fs::remove_dir_all(&self.tmp_dir)
            .map_err(MirsError::from)
    }

    pub fn strip_root<'a>(&'a self, path: &'a str) -> &'a str {
        let Some(path) = path.strip_prefix(self.root_dir.as_str()) else {
            return path
        };

        match path.strip_prefix('/') {
            Some(p) => p,
            None => path
        }
    }

    pub fn rel_from_tmp<'a>(&self, path: &'a str) -> &'a str {
        path.strip_prefix(self.tmp_dir.as_str())
            .expect("input path should be in tmp dir")
    }

    pub fn to_path_in_tmp(&self, url: &str) -> FilePath {
        self.to_path_in_local_dir(&self.tmp_dir, url)
    }

    pub fn to_path_in_root(&self, url: &str) -> FilePath {
        self.to_path_in_local_dir(&self.root_dir, url)
    }

    pub fn to_url_in_dist(&self, path: &str) -> CompactString {
        format_compact!("{}/{}", self.dist_url, path)
    }

    pub fn to_url_in_root(&self, path: &str) -> CompactString {
        format_compact!("{}/{}", self.root_url, path)
    }

    pub fn rebase_rel_to_root<P: AsRef<str>>(&self, path: P) -> FilePath {
        FilePath(format_compact!("{}/{}", self.root_dir, path.as_ref()))
    }

    pub fn tmp_to_root<P: AsRef<str>>(&self, path: P) -> Option<FilePath> {
        path.as_ref().strip_prefix(self.tmp_dir.as_str())
            .map(|v| self.root_dir.join(v))
    }

    pub fn strip_tmp_base<P: AsRef<str>>(&self, path: P) -> Option<FilePath> {
        path.as_ref().strip_prefix(self.tmp_dir.as_str())
            .map(|v| v.strip_prefix('/').expect("Paths that strip tmp base should always start with / here"))
            .map(FilePath::from)
    }

    pub fn create_file_download(&self, package: IndexFileEntry) -> Box<Download> {
        let url = self.to_url_in_root(&package.path);
        let primary_target_path = self.to_path_in_root(&url);

        Box::new(Download {
            url,
            size: package.size,
            checksum: package.checksum,
            primary_target_path,
            symlink_paths: Vec::new(),
            always_download: false,
        })
    }

    pub fn create_raw_download(&self, target_path: FilePath, url: CompactString, checksum: Option<Checksum>) -> Box<Download> {
        Box::new(Download {
            url,
            size: None,
            checksum,
            primary_target_path: target_path,
            symlink_paths: Vec::new(),
            always_download: true
        })
    }

    pub fn create_metadata_download(&self, url: CompactString, file_path: FilePath, file_entry: FileEntry, by_hash: bool) -> Result<Box<Download>> {
        let size = file_entry.size;

        let (checksum, primary_target_path, symlink_paths) = file_entry.into_paths(&file_path, by_hash)?;

        Ok(Box::new(Download {
            url,
            size: Some(size),
            checksum,
            primary_target_path,
            symlink_paths,
            always_download: false
        }))
    }
}

impl KeyStore for Repository {
    fn verify_inlined_signed_release(&self, msg: &CleartextSignedMessage, content: &str) -> Result<()> {
        let Some(key) = &self.pgp_pub_key else {
            return Err(MirsError::PgpNotVerified)
        };

        for signature in msg.signatures() {
            if signature.verify(key, content.as_bytes()).is_ok() {
                return Ok(())
            }
        }

        for sub_key in &key.public_subkeys {
            for signature in msg.signatures() {
                if signature.verify(sub_key, content.as_bytes()).is_ok() {
                    return Ok(())
                }
            }
        }

        Err(MirsError::PgpNotVerified)
    }

    fn verify_release_with_standalone_signature(&self, signature: &StandaloneSignature, content: &str) -> Result<()> {
        let Some(key) = &self.pgp_pub_key else {
            return Err(MirsError::PgpNotVerified)
        };

        if signature.verify(key, content.as_bytes()).is_ok() {
            return Ok(())
        }

        for sub_key in &key.public_subkeys {
            if signature.verify(sub_key, content.as_bytes()).is_ok() {
                return Ok(())
            }
        }

        Err(MirsError::PgpNotVerified)
    }
}

fn sanitize_path_part(part: &str) -> CompactString {
    let mut sanitized = CompactString::new("");

    let char_iter = part.chars();

    for c in char_iter {
        if c == '/' {
            sanitized.push('_')
        } else {
            sanitized.push(c)
        }
    }

    sanitized
}

fn create_tmp_dir(url: &Url, suite: &str, base_dir: &FilePath) -> Result<FilePath> {
    let Some(host) = url.host() else {
        return Err(MirsError::UrlParsing { url: url.to_compact_string() })
    };

    let path = url.path();

    let path_part = if path == "/" {
        CompactString::new("")
    } else {
        sanitize_path_part(path)
    };

    let tmp_dir = base_dir
        .join(format_compact!(".tmp/{host}{path_part}_{suite}"));

    match std::fs::metadata(&tmp_dir) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            std::fs::create_dir_all(&tmp_dir)?;
            Ok(tmp_dir)
        },
        Err(e) => Err(MirsError::Tmp { msg: e.to_compact_string() }),
        Ok(_) => Err(MirsError::Tmp { 
            msg: format_compact!(
                "tmp folder already exists for this repository. aptmirs is probably currently running. if it is not, delete {}",
                tmp_dir.as_str()
            )
        })
    }
}

fn local_dir_from_archive_url(url: &Url, dir: &FilePath) -> Result<FilePath> {
    let Some(host) = url.host() else {
        return Err(MirsError::UrlParsing { url: url.to_compact_string() })
    };

    let mut base_dir = dir
        .join(host.to_compact_string());

    if let Some(path) = url.path().strip_prefix('/') {
        base_dir = base_dir.join(path);
    }

    Ok(base_dir)
}
