//! The local node's Ed25519 signing key (ADR-002).
//!
//! The secret never leaves the node's home directory. Signatures are
//! detached `SignatureV0` records over the exact canonical EnvelopeV0 bytes.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

use deaddrop_protocol::{Ed25519PublicKey, EnvelopeV0, NodeId, SignatureV0, encode_envelope_v0};
use ed25519_dalek::{Signer, SigningKey};

use crate::shell::ShellError;

pub struct LocalKey(SigningKey);

impl LocalKey {
    /// A new key from the operating system CSPRNG.
    pub fn generate() -> Result<Self, ShellError> {
        let mut seed = [0u8; 32];
        getrandom::fill(&mut seed).map_err(|error| ShellError::Randomness(error.to_string()))?;
        Ok(Self::from_seed(seed))
    }

    /// Deterministic key from a 32-byte secret seed. For fixtures and for
    /// reloading a stored key.
    pub fn from_seed(seed: [u8; 32]) -> Self {
        Self(SigningKey::from_bytes(&seed))
    }

    pub fn public(&self) -> Ed25519PublicKey {
        Ed25519PublicKey::from_bytes(self.0.verifying_key().to_bytes())
    }

    /// Sign the exact canonical bytes of `envelope` as `signer`.
    pub fn sign(&self, signer: &NodeId, envelope: &EnvelopeV0) -> SignatureV0 {
        let bytes = encode_envelope_v0(envelope).expect("an EnvelopeV0 always encodes");
        SignatureV0::new(
            envelope.id().clone(),
            signer.clone(),
            self.public(),
            self.0.sign(&bytes).to_bytes(),
        )
    }

    /// Load a secret written by [`LocalKey::save`].
    pub fn load(path: &Path) -> Result<Self, ShellError> {
        let text = std::fs::read_to_string(path)?;
        let text = text.trim_end();
        let mut seed = [0u8; 32];
        if text.len() != 64 {
            return Err(ShellError::Corrupt("secret key length".into()));
        }
        for (i, pair) in text.as_bytes().chunks_exact(2).enumerate() {
            let pair =
                std::str::from_utf8(pair).map_err(|_| ShellError::Corrupt("secret key".into()))?;
            seed[i] = u8::from_str_radix(pair, 16)
                .map_err(|_| ShellError::Corrupt("secret key".into()))?;
        }
        Ok(Self::from_seed(seed))
    }

    /// Write the secret to a new file readable only by its owner.
    pub(crate) fn save(&self, path: &Path) -> Result<(), ShellError> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(path)?;
        let hex: String = self
            .0
            .to_bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        writeln!(file, "{hex}")?;
        file.sync_all()?;
        Ok(())
    }
}
