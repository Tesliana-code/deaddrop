use std::fmt;
use std::fmt::Write as _;
use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

use deaddrop_protocol::ArtifactRef;
use sha2::{Digest, Sha256};
use tempfile::NamedTempFile;

const SHA256_DIR: &str = "sha256";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArtifactPutOutcome {
    Stored(ArtifactRef),
    AlreadyPresent(ArtifactRef),
}

impl ArtifactPutOutcome {
    pub fn artifact(&self) -> &ArtifactRef {
        match self {
            Self::Stored(artifact) | Self::AlreadyPresent(artifact) => artifact,
        }
    }
}

/// Persistence boundary for content-addressed artifact bytes.
///
/// An ArtifactRef is the SHA-256 identity of the exact stored bytes. Replaying
/// identical bytes is idempotent. Bytes are returned only after their identity
/// has been re-verified against the requested reference.
pub trait ArtifactStore {
    type Error;

    fn put(&mut self, bytes: &[u8]) -> Result<ArtifactPutOutcome, Self::Error>;

    fn get(&self, artifact: &ArtifactRef) -> Result<Option<Vec<u8>>, Self::Error>;
}

#[derive(Debug)]
pub enum FilesystemArtifactStoreError {
    Io {
        path: PathBuf,
        source: io::Error,
    },
    /// Bytes persisted at the canonical path for `artifact` do not hash to
    /// `artifact`. Fails closed; the bytes are neither returned nor repaired.
    IntegrityMismatch {
        artifact: ArtifactRef,
        path: PathBuf,
    },
}

impl fmt::Display for FilesystemArtifactStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => {
                write!(f, "artifact store io error at {}: {source}", path.display())
            }
            Self::IntegrityMismatch { artifact, path } => write!(
                f,
                "persisted artifact at {} does not match {artifact}",
                path.display()
            ),
        }
    }
}

impl std::error::Error for FilesystemArtifactStoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::IntegrityMismatch { .. } => None,
        }
    }
}

fn io_error(path: &Path) -> impl FnOnce(io::Error) -> FilesystemArtifactStoreError + '_ {
    move |source| FilesystemArtifactStoreError::Io {
        path: path.to_owned(),
        source,
    }
}

fn artifact_ref_of(bytes: &[u8]) -> ArtifactRef {
    ArtifactRef::from_sha256(Sha256::digest(bytes).into())
}

/// Durable local artifact store rooted at an operator-supplied directory.
///
/// Layout: `<root>/sha256/<first 2 hex>/<remaining 62 hex>`. Paths derive only
/// from digest bytes; no caller-supplied name ever reaches the filesystem.
/// Writes go to a temporary file in the target directory and are published by
/// a no-clobber rename, so the canonical path never holds a partial write.
pub struct FilesystemArtifactStore {
    root: PathBuf,
}

impl FilesystemArtifactStore {
    pub fn open(root: impl AsRef<Path>) -> Result<Self, FilesystemArtifactStoreError> {
        let root = root.as_ref().to_owned();
        let objects = root.join(SHA256_DIR);
        fs::create_dir_all(&objects).map_err(io_error(&objects))?;
        Ok(Self { root })
    }

    /// Canonical filesystem path for an artifact in this store.
    pub fn path_for(&self, artifact: &ArtifactRef) -> PathBuf {
        let mut hex = String::with_capacity(64);
        for byte in artifact.digest() {
            let _ = write!(hex, "{byte:02x}");
        }

        self.root.join(SHA256_DIR).join(&hex[..2]).join(&hex[2..])
    }

    fn read_verified(
        &self,
        artifact: &ArtifactRef,
    ) -> Result<Option<Vec<u8>>, FilesystemArtifactStoreError> {
        let path = self.path_for(artifact);

        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(io_error(&path)(error)),
        };

        if artifact_ref_of(&bytes) != *artifact {
            return Err(FilesystemArtifactStoreError::IntegrityMismatch {
                artifact: artifact.clone(),
                path,
            });
        }

        Ok(Some(bytes))
    }
}

impl ArtifactStore for FilesystemArtifactStore {
    type Error = FilesystemArtifactStoreError;

    fn put(&mut self, bytes: &[u8]) -> Result<ArtifactPutOutcome, Self::Error> {
        let artifact = artifact_ref_of(bytes);

        if self.read_verified(&artifact)?.is_some() {
            return Ok(ArtifactPutOutcome::AlreadyPresent(artifact));
        }

        let path = self.path_for(&artifact);
        let dir = path.parent().expect("artifact path has a fan-out parent");
        fs::create_dir_all(dir).map_err(io_error(dir))?;

        let mut temp = NamedTempFile::new_in(dir).map_err(io_error(dir))?;
        temp.write_all(bytes).map_err(io_error(temp.path()))?;
        temp.as_file().sync_all().map_err(io_error(temp.path()))?;

        if let Err(error) = temp.persist_noclobber(&path) {
            // Another writer published first; accept it only if it verifies.
            if error.error.kind() == io::ErrorKind::AlreadyExists
                && self.read_verified(&artifact)?.is_some()
            {
                return Ok(ArtifactPutOutcome::AlreadyPresent(artifact));
            }

            return Err(io_error(&path)(error.error));
        }

        #[cfg(unix)]
        fs::File::open(dir)
            .and_then(|dir| dir.sync_all())
            .map_err(io_error(dir))?;

        Ok(ArtifactPutOutcome::Stored(artifact))
    }

    fn get(&self, artifact: &ArtifactRef) -> Result<Option<Vec<u8>>, Self::Error> {
        self.read_verified(artifact)
    }
}
