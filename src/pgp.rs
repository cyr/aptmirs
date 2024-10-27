use std::collections::BTreeMap;
use std::fs::File;
use std::sync::Arc;

use pgp::cleartext::CleartextSignedMessage;
use pgp::types::PublicKeyTrait;
use pgp::{Deserializable, SignedPublicKey, SignedPublicSubKey, StandaloneSignature};
use walkdir::WalkDir;

use crate::metadata::FilePath;
use crate::error::{MirsError, Result};

pub struct PgpKeyStore {
    primary_fingerprints: BTreeMap<String, Arc<SignedPublicKey>>,
    primary_key_ids: BTreeMap<String, Arc<SignedPublicKey>>,
    sub_fingerprints: BTreeMap<String, Arc<SignedPublicSubKey>>,
    sub_key_ids: BTreeMap<String, Arc<SignedPublicSubKey>>,
}

impl PgpKeyStore {
    pub fn build_from_path(path: &FilePath) -> Result<Self> {
        let mut primary_fingerprints = BTreeMap::new();
        let mut sub_fingerprints = BTreeMap::new();
        let mut primary_key_ids = BTreeMap::new();
        let mut sub_key_ids = BTreeMap::new();

        for entry in WalkDir::new(path).follow_links(true) {
            let entry = match entry {
                Ok(entry) => entry,
                Err(inner) => return Err(MirsError::PgpKeyStore { inner }),
            };

            if entry.file_type().is_dir() {
                continue
            }

            let file = FilePath::from(entry.path());

            if !matches!(file.extension(), Some("asc") | Some("gpg") | Some("pgp") | None) {
                continue
            }

            let public_key = match read_public_key(&file) {
                Ok(key) => Arc::new(key),
                Err(MirsError::PgpKeyVerification { path, msg }) => {
                    println!("{} WARNING: {path} is not valid and will not be used: {msg}", crate::now());
                    continue
                },
                Err(e) => return Err(e)
            };

            let fingerprint = hex::encode(public_key.fingerprint().as_bytes());
            let key_id = hex::encode(&public_key.key_id());

            primary_fingerprints.insert(fingerprint, public_key.clone());
            primary_key_ids.insert(key_id, public_key.clone());

            for sub_key in &public_key.public_subkeys {
                let sub_key = Arc::new(sub_key.clone());

                let fingerprint = hex::encode(sub_key.fingerprint().as_bytes());
                let key_id = hex::encode(&sub_key.key_id());

                sub_fingerprints.insert(fingerprint, sub_key.clone());
                sub_key_ids.insert(key_id, sub_key);
            }
        }

        Ok(PgpKeyStore {
            primary_fingerprints,
            sub_fingerprints,
            primary_key_ids,
            sub_key_ids
        })
    }    
}

pub trait KeyStore {
    fn verify_inlined_signed_release(&self, msg: &CleartextSignedMessage, content: &str) -> Result<()>;
    fn verify_release_with_standalone_signature(&self, signature: &StandaloneSignature, content: &str) -> Result<()>;

    fn verify_inlined(&self, inlined_message: &FilePath) -> Result<()> {
        let content = std::fs::read_to_string(inlined_message)?;

        let (msg, _) = CleartextSignedMessage::from_string(&content)?;
        let content = msg.signed_text();
        
        self.verify_inlined_signed_release(&msg, &content)
    }

    fn verify_standalone(&self, signature: &FilePath, message: &FilePath) -> Result<()> {
        let sign_handle = File::open(signature)?;
        let content = std::fs::read_to_string(message)?;

        let (signature, _) = StandaloneSignature::from_reader_single(&sign_handle)?;

        self.verify_release_with_standalone_signature(&signature, &content)
    }
}

impl KeyStore for PgpKeyStore {
    fn verify_inlined_signed_release(&self, msg: &CleartextSignedMessage, content: &str) -> Result<()> {
        for signature in msg.signatures() {
            if signature.signature.issuer_fingerprint().is_empty() && signature.signature.issuer().is_empty() {
                for key in self.primary_key_ids.values() {
                    if signature.verify(key.as_ref(), content.as_bytes()).is_ok() {
                        return Ok(())
                    }
                }
                
                for key in self.sub_key_ids.values() {
                    if signature.verify(key.as_ref(), content.as_bytes()).is_ok() {
                        return Ok(())
                    }
                }

                continue
            }

            for fingerprint in signature.signature.issuer_fingerprint() {
                let hex_fingerprint = hex::encode(fingerprint.as_bytes());

                if let Some(key) = self.primary_fingerprints.get(&hex_fingerprint) {
                    if signature.verify(key.as_ref(), content.as_bytes()).is_ok() {
                        return Ok(())
                    }
                }

                if let Some(key) = self.sub_fingerprints.get(&hex_fingerprint) {
                    if signature.verify(key.as_ref(), content.as_bytes()).is_ok() {
                        return Ok(())
                    }
                }
            }

            for key_id in signature.signature.issuer() {
                let hex_key_id = hex::encode(key_id.as_ref());

                if let Some(key) = self.primary_key_ids.get(&hex_key_id) {
                    if signature.verify(key.as_ref(), content.as_bytes()).is_ok() {
                        return Ok(())
                    }
                }

                if let Some(key) = self.sub_key_ids.get(&hex_key_id) {
                    if signature.verify(key.as_ref(), content.as_bytes()).is_ok() {
                        return Ok(())
                    }
                }
            }
        }

        Err(MirsError::PgpNotVerified)
    }
    
    fn verify_release_with_standalone_signature(&self, signature: &StandaloneSignature, content: &str) -> Result<()> {
        if signature.signature.issuer_fingerprint().is_empty() && signature.signature.issuer().is_empty() {
            for key in self.primary_key_ids.values() {
                if signature.verify(key.as_ref(), content.as_bytes()).is_ok() {
                    return Ok(())
                }
            }
            
            for key in self.sub_key_ids.values() {
                if signature.verify(key.as_ref(), content.as_bytes()).is_ok() {
                    return Ok(())
                }
            }

            return Err(MirsError::PgpNotVerified)
        }

        for fingerprint in signature.signature.issuer_fingerprint() {
            let hex_fingerprint = hex::encode(fingerprint.as_bytes());

            if let Some(key) = self.primary_fingerprints.get(&hex_fingerprint) {
                if signature.verify(key.as_ref(), content.as_bytes()).is_ok() {
                    return Ok(())
                }
            }

            if let Some(key) = self.sub_fingerprints.get(&hex_fingerprint) {
                if signature.verify(key.as_ref(), content.as_bytes()).is_ok() {
                    return Ok(())
                }
            }
        }

        for key_id in signature.signature.issuer() {
            let hex_key_id = hex::encode(key_id.as_ref());

            if let Some(key) = self.primary_key_ids.get(&hex_key_id) {
                if signature.verify(key.as_ref(), content.as_bytes()).is_ok() {
                    return Ok(())
                }
            }

            if let Some(key) = self.sub_key_ids.get(&hex_key_id) {
                if signature.verify(key.as_ref(), content.as_bytes()).is_ok() {
                    return Ok(())
                }
            }
        }

        Err(MirsError::PgpNotVerified)
    }
}

pub fn read_public_key(path: &FilePath) -> Result<SignedPublicKey> {
    let key_file = std::fs::File::open(path)
        .map_err(|e| MirsError::PgpPubKey { path: path.clone(), inner: Box::new(e.into()) })?;

    let (signed_public_key, _) = SignedPublicKey::from_reader_single(&key_file)
        .map_err(|e| MirsError::PgpPubKey { path: path.clone(), inner: Box::new(e.into()) })?;

    if let Some(expiry_date) = signed_public_key.expires_at() {
        if expiry_date < chrono::Utc::now() {
            return Err(MirsError::PgpKeyVerification { path: path.clone(), msg: String::from("public key is expired") })
        }
    }

    Ok(signed_public_key)
}

pub fn verify_release_signature<K: KeyStore>(files: &[FilePath], key_store: &K) -> Result<()> {
    if let Some(inrelease_file) = files.iter().find(|v| v.file_name() == "InRelease") {
        key_store.verify_inlined(inrelease_file)?;
    } else {
        let Some(release_file) = files.iter().find(|v| v.file_name() == "Release") else {
            return Err(MirsError::PgpNotSupported)
        };

        let Some(release_file_signature) = files.iter().find(|v| v.file_name() == "Release.pgp") else {
            return Err(MirsError::PgpNotSupported)
        };

        key_store.verify_standalone(release_file_signature, release_file)?;
    }

    Ok(())
}