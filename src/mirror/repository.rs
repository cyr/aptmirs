use std::{fs::File, path::Path, str::FromStr};

use compact_str::{format_compact, CompactString, ToCompactString};
use pgp::{cleartext::CleartextSignedMessage, Deserializable, SignedPublicKey, StandaloneSignature};
use reqwest::Url;

use super::downloader::Download;

use crate::{config::MirrorOpts, error::{MirsError, Result}, metadata::{checksum::Checksum, release::FileEntry, FilePath, IndexFileEntry}, CliOpts};

pub struct Repository {
    root_url: CompactString,
    root_dir: FilePath,
    dist_url: CompactString,
    tmp_dir: FilePath,
    pgp_pub_key: Option<SignedPublicKey>,
}

impl Repository {
    pub fn build(mirror_opts: &MirrorOpts, cli_opts: &CliOpts) -> Result<Self> {
        let root_url = match &mirror_opts.url.as_str().strip_prefix('/') {
            Some(url) => url.to_compact_string(),
            None => mirror_opts.url.clone(),
        };

        let dist_url = format_compact!("{root_url}/dists/{}", mirror_opts.suite);

        let parsed_url = Url::parse(&root_url)
            .map_err(|_| MirsError::UrlParsing { url: root_url.clone() })?;

        let pgp_pub_key = if let Some(pgp_signing_key) = &mirror_opts.pgp_pub_key {
            let key_file = File::open(pgp_signing_key)
                .map_err(|e| MirsError::PgpPubKey { inner: Box::new(e.into()) })?;

            let (signed_public_key, _) = SignedPublicKey::from_reader_single(&key_file)
                .map_err(|e| MirsError::PgpPubKey { inner: Box::new(e.into()) })?;

            Some(signed_public_key)
        } else {
            None
        };

        let root_dir = local_dir_from_archive_url(&parsed_url, &cli_opts.output)?;
        let tmp_dir = create_tmp_dir(&parsed_url, &mirror_opts.suite, &cli_opts.output)?;

        Ok(Self {
            root_url,
            root_dir,
            dist_url,
            tmp_dir,
            pgp_pub_key
        })
    }

    pub fn release_urls(&self) -> [CompactString; 3] {
        [
            format_compact!("{}/InRelease", self.dist_url),
            format_compact!("{}/Release", self.dist_url),
            format_compact!("{}/Release.gpg", self.dist_url)
        ]
    }

    fn to_path_in_local_dir(&self, base: &FilePath, url: &str) -> FilePath {
        let relative_path = url
            .strip_prefix(self.root_url.as_str())
            .expect("implementation error; download url should be in archive root: {base}, url: {url}");

        let relative_path = match relative_path.strip_prefix('/') {
            Some(path) => path,
            None => relative_path,
        };

        base.join(relative_path)
    }

    pub fn delete_tmp(&self) -> Result<()> {
        if let Err(e) = std::fs::remove_dir_all(&self.tmp_dir) {
            return Err(e.into())
        }

        Ok(())
    }

    pub async fn finalize(&self, paths_to_delete: Vec<FilePath>) -> Result<()> {
        let tmp_dir = self.tmp_dir.clone();
        let root_dir = self.root_dir.clone();

        for path in paths_to_delete {
            if tokio::fs::try_exists(&path).await? {
                tokio::fs::remove_dir_all(&path).await?;
            }
        }

        tokio::task::spawn_blocking(move || {
            rebase_dir(tmp_dir.as_ref(), tmp_dir.as_ref(), root_dir.as_ref())?;
            
            std::fs::remove_dir_all(&tmp_dir)?;
            
            Ok(())
        }).await?
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

    pub fn rebase_to_root<P: AsRef<str>>(&self, path: P) -> FilePath {
        FilePath::from_str(&format_compact!("{}/{}", self.root_dir, path.as_ref())).expect("FilePath from str should always work")
    }

    pub fn tmp_to_root<P: AsRef<str>>(&self, path: P) -> Option<FilePath> {
        path.as_ref().strip_prefix(self.tmp_dir.as_str())
            .map(|v| self.root_dir.join(v))
    }

    pub fn strip_tmp_base<P: AsRef<str>>(&self, path: P) -> Option<FilePath> {
        path.as_ref().strip_prefix(self.tmp_dir.as_str())
            .map(|v| v.strip_prefix('/').expect("Paths that strip tmp base should always start with / here"))
            .map(|v| FilePath::from_str(v).expect("FilePath from str should always work"))
    }

    pub fn create_file_download(&self, package: IndexFileEntry) -> Box<Download> {
        let url = self.to_url_in_root(&package.path);
        let primary_target_path = self.to_path_in_root(&url);

        Box::new(Download {
            url,
            size: Some(package.size),
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

        let strongest_checksum = file_entry.strongest_hash();
        let mut checksum_iter = file_entry.into_iter();

        let mut symlink_paths = Vec::new();
        let primary_target_path = if by_hash {
            let by_hash_base = FilePath(
                file_path
                    .parent()
                    .expect("all files needs a parent(?)")
                    .to_compact_string()
            );

            symlink_paths.push(file_path);

            let strongest_checksum = checksum_iter.next()
                .ok_or_else(|| MirsError::NoReleaseFile)?;

            for checksum in checksum_iter {
                let hash_path = by_hash_base.join(checksum.relative_path());
                symlink_paths.push(hash_path);
            }

            by_hash_base.join(strongest_checksum.relative_path())
        } else {
            file_path
        };

        Ok(Box::new(Download {
            url,
            size: Some(size),
            checksum: strongest_checksum,
            primary_target_path,
            symlink_paths,
            always_download: false
        }))
    }

    pub fn verify_release_signature(&self, files: &[FilePath]) -> Result<()> {
        if let Some(pgp_pub_key) = &self.pgp_pub_key {
            if let Some(inrelease_file) = files.iter().find(|v| v.file_name() == "InRelease") {
                self.verify_signed_message(pgp_pub_key, inrelease_file)?;
            } else {
                let Some(release_file) = files.iter().find(|v| v.file_name() == "Release") else {
                    return Err(MirsError::PgpNotSupported)
                };

                let Some(release_file_signature) = files.iter().find(|v| v.file_name() == "Release.pgp") else {
                    return Err(MirsError::PgpNotSupported)
                };
                
                self.verify_message_with_standlone_signature(pgp_pub_key, release_file, release_file_signature)?;
            }
        }

        Ok(())
    }

    fn verify_signed_message(&self, pgp_pub_key: &SignedPublicKey, file: &FilePath) -> Result<()> {
        let content = std::fs::read_to_string(file)?;

        let (msg, _) = CleartextSignedMessage::from_string(&content)?;

        if msg.verify(&pgp_pub_key).is_ok() {
            return Ok(())
        }

        for subkey in &pgp_pub_key.public_subkeys {
            if msg.verify(subkey).is_ok() {
                return Ok(())
            }
        }

        Err(MirsError::PgpNotVerified)
    }

    fn verify_message_with_standlone_signature(&self, pgp_pub_key: &SignedPublicKey, release_file: &FilePath, release_file_signature: &FilePath) -> Result<()> {
        let sign_handle = File::open(release_file_signature)?;
        let content = std::fs::read_to_string(release_file)?;

        let (signature, _) = StandaloneSignature::from_reader_single(&sign_handle)?;

        if signature.verify(&pgp_pub_key, content.as_bytes()).is_ok() {
            return Ok(())
        }

        for subkey in &pgp_pub_key.public_subkeys {
            if signature.verify(&subkey, content.as_bytes()).is_ok() {
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

fn rebase_dir(dir: &Path, from: &Path, to: &Path) -> Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            rebase_dir(&path, from, to)?;
        } else {
            let rel_path = path.strip_prefix(from)
                .expect("implemention error; path should be in tmp");

            let new_path = to.join(rel_path);

            let parent = new_path.parent().unwrap();
            if !parent.exists() {
                std::fs::create_dir_all(parent)?;
            }
            
            if std::fs::rename(&path, &new_path).is_err() {
                std::fs::copy(&path, &new_path)?;
            }
        }
    }

    Ok(())
}