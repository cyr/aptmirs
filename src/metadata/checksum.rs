use std::{fmt::Display, path::Path};

use md5::Context;
use sha1::{Sha1, Digest, digest::{FixedOutput, Update}};
use sha2::{Sha256, Sha512};
use tokio::io::AsyncReadExt;

use crate::error::{Result, MirsError};

#[derive(Debug, PartialEq)]
pub enum Checksum {
    Md5([u8; 16]),
    Sha1([u8; 20]),
    Sha256([u8; 32]),
    Sha512([u8; 64])
}

impl TryFrom<&str> for Checksum {
    type Error = MirsError;

    fn try_from(value: &str) -> std::prelude::v1::Result<Self, Self::Error> {
        match value.len() {
            32 => {
                let mut bytes = [0_u8; 16];
                hex::decode_to_slice(value, &mut bytes)?;
                Ok(bytes.into())
            },
            40 => {
                let mut bytes = [0_u8; 20];
                hex::decode_to_slice(value, &mut bytes)?;
                Ok(bytes.into())
            },
            64 => {
                let mut bytes = [0_u8; 32];
                hex::decode_to_slice(value, &mut bytes)?;
                Ok(bytes.into())
            },
            128 => {
                let mut bytes = [0_u8; 64];
                hex::decode_to_slice(value, &mut bytes)?;
                Ok(bytes.into())
            }
            _ => Err(MirsError::IntoChecksum { value: value.to_string() })
        }
    }
}

impl From<[u8; 16]> for Checksum {
    fn from(value: [u8; 16]) -> Self {
        Self::Md5(value)
    }
}

impl From<[u8; 20]> for Checksum {
    fn from(value: [u8; 20]) -> Self {
        Self::Sha1(value)
    }
}

impl From<[u8; 32]> for Checksum {
    fn from(value: [u8; 32]) -> Self {
        Self::Sha256(value)
    }
}

impl From<[u8; 64]> for Checksum {
    fn from(value: [u8; 64]) -> Self {
        Self::Sha512(value)
    }
}

impl Display for Checksum {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Checksum::Md5(v)    => f.write_str(&hex::encode(v)),
            Checksum::Sha1(v)   => f.write_str(&hex::encode(v)),
            Checksum::Sha256(v) => f.write_str(&hex::encode(v)),
            Checksum::Sha512(v) => f.write_str(&hex::encode(v)),
        }
    }
}

impl Checksum {
    pub fn create_hasher(&self) -> Box<dyn Hasher> {
        match self {
            Checksum::Md5(_)    => Box::new(Md5Hasher::new()),
            Checksum::Sha1(_)   => Box::new(Sha1Hasher::new()),
            Checksum::Sha256(_) => Box::new(Sha256Hasher::new()),
            Checksum::Sha512(_) => Box::new(Sha512Hasher::new()),
        }
    }

    pub async fn checksum_file(file: &Path) -> Result<Checksum> {
        let mut f = tokio::fs::File::open(file).await?;
        
        let mut buf = vec![0_u8; 8192];

        let mut hasher = Box::new(Sha512Hasher::new());

        loop {
            match f.read_buf(&mut buf).await {
                Ok(0) => break,
                Ok(n) => hasher.consume(&buf[..n]),
                Err(e) => return Err(e.into())
            }
        }

        Ok(hasher.compute())
    }

    pub fn bits(&self) -> usize {
        match self {
            Checksum::Md5(v) => v.len(),
            Checksum::Sha1(v) => v.len(),
            Checksum::Sha256(v) => v.len(),
            Checksum::Sha512(v) => v.len(),
        }
    }

    pub fn replace_if_stronger(&mut self, other: Checksum) {
        if self.bits() >= other.bits() {
            return
        }

        *self = other
    }
}

impl ChecksumType {
    pub fn is_stronger(first: &Option<Checksum>, second: ChecksumType) -> bool {
        matches!((first, second), 
            (_, ChecksumType::Sha512) |
            (_, ChecksumType::Sha256) |
            (_, ChecksumType::Sha1) |
            (_, ChecksumType::Md5)
        )
    }
}

pub enum ChecksumType {
    Md5,
    Sha1,
    Sha256,
    Sha512
}

pub trait Hasher : Sync + Send {
    fn consume(&mut self, data: &[u8]);
    fn compute(self: Box<Self>) -> Checksum;
}

struct Md5Hasher {
    ctx: Context
}

impl Md5Hasher {
    pub fn new() -> Self {
        Self {
            ctx: Context::new()
        }
    }
}

impl Hasher for Md5Hasher {
    fn consume(&mut self, data: &[u8]) {
        self.ctx.consume(data)
    }

    fn compute(self: Box<Self>) -> Checksum {
        Checksum::Md5(self.ctx.compute().0)
    }
}

struct Sha1Hasher {
    hasher: Sha1
}

impl Sha1Hasher {
    pub fn new() -> Self {
        Self {
            hasher: Sha1::new()
        }
    }
}

impl Hasher for Sha1Hasher {
    fn consume(&mut self, data: &[u8]) {
        Update::update(&mut self.hasher, data)
    }

    fn compute(self: Box<Self>) -> Checksum {
        Checksum::Sha1(self.hasher.finalize_fixed().into())
    }
}

struct Sha256Hasher {
    hasher: Sha256
}

impl Sha256Hasher {
    pub fn new() -> Self {
        Self {
            hasher: sha2::Sha256::new()
        }
    }
}

impl Hasher for Sha256Hasher {
    fn consume(&mut self, data: &[u8]) {
        Update::update(&mut self.hasher, data)
    }

    fn compute(self: Box<Self>) -> Checksum {
        Checksum::Sha256(self.hasher.finalize_fixed().into())
    }
}

struct Sha512Hasher {
    hasher: Sha512
}

impl Sha512Hasher {
    pub fn new() -> Self {
        Self {
            hasher: sha2::Sha512::new()
        }
    }
}

impl Hasher for Sha512Hasher {
    fn consume(&mut self, data: &[u8]) {
        Update::update(&mut self.hasher, data)
    }

    fn compute(self: Box<Self>) -> Checksum {
        Checksum::Sha512(self.hasher.finalize_fixed().into())
    }
}