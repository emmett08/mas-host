//! Own immutable bounded text and an in-memory binding that never enters public metadata.

use crate::{Error, Result};
use sha2::{Digest, Sha256};
use std::{fmt, io::Read};

/// The caller supplies a fresh opaque source ID from its trusted identity owner.
/// Raw fingerprints are private to the process and are never serialised or logged.
/// Durable protected commitments and their key lifecycle belong to E2/N2.
#[derive(Clone)]
pub struct Source {
    id: String,
    text: String,
    fingerprint: [u8; 32],
    token: String,
}

impl Source {
    pub fn text(id: &str, text: &str, max_bytes: usize) -> Result<Self> {
        identifier(id)?;
        if max_bytes == 0 || text.len() > max_bytes {
            return Err(Error::InputTooLarge);
        }
        let mut nonce = [0u8; 32];
        std::fs::File::open("/dev/urandom")
            .and_then(|mut file| file.read_exact(&mut nonce))
            .map_err(|_| Error::EntropyUnavailable)?;
        Ok(Self {
            id: id.into(),
            text: text.into(),
            fingerprint: Sha256::digest(text.as_bytes()).into(),
            token: nonce.iter().map(|byte| format!("{byte:02x}")).collect(),
        })
    }

    /// N0 admits text only. Parsing JSON would require original-byte mappings.
    pub fn from_format(id: &str, text: &str, format: &str, max_bytes: usize) -> Result<Self> {
        if format != "utf8-text/1" {
            return Err(Error::UnsupportedFormat);
        }
        Self::text(id, text, max_bytes)
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn content(&self) -> &str {
        &self.text
    }

    pub(crate) fn binding(&self) -> [u8; 32] {
        self.fingerprint
    }

    pub(crate) fn token(&self) -> &str {
        &self.token
    }
}

impl fmt::Debug for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Source")
            .field("id", &self.id)
            .field("bytes", &self.text.len())
            .finish_non_exhaustive()
    }
}

pub(crate) fn identifier(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-._:/".contains(&c))
    {
        return Err(Error::InvalidIdentifier);
    }
    Ok(())
}
