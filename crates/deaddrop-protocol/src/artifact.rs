use std::fmt;
use std::str::FromStr;

const SHA256_BYTES: usize = 32;
const SHA256_HEX_LEN: usize = SHA256_BYTES * 2;
const SHA256_PREFIX: &str = "sha256:";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArtifactRefError {
    UnsupportedScheme,
    InvalidDigestLength,
    InvalidHex,
}

impl fmt::Display for ArtifactRefError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedScheme => write!(f, "artifact reference must use sha256"),
            Self::InvalidDigestLength => {
                write!(
                    f,
                    "sha256 digest must contain exactly 64 hexadecimal characters"
                )
            }
            Self::InvalidHex => write!(f, "sha256 digest contains invalid hexadecimal data"),
        }
    }
}

impl std::error::Error for ArtifactRefError {}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ArtifactRef {
    digest: [u8; SHA256_BYTES],
}

impl ArtifactRef {
    pub fn from_sha256(digest: [u8; SHA256_BYTES]) -> Self {
        Self { digest }
    }

    pub fn digest(&self) -> &[u8; SHA256_BYTES] {
        &self.digest
    }
}

impl fmt::Display for ArtifactRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(SHA256_PREFIX)?;

        for byte in self.digest {
            write!(f, "{byte:02x}")?;
        }

        Ok(())
    }
}

impl FromStr for ArtifactRef {
    type Err = ArtifactRefError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let hex = value
            .strip_prefix(SHA256_PREFIX)
            .ok_or(ArtifactRefError::UnsupportedScheme)?;

        if hex.len() != SHA256_HEX_LEN {
            return Err(ArtifactRefError::InvalidDigestLength);
        }

        let mut digest = [0_u8; SHA256_BYTES];

        for (index, chunk) in hex.as_bytes().chunks_exact(2).enumerate() {
            let pair = std::str::from_utf8(chunk).map_err(|_| ArtifactRefError::InvalidHex)?;
            digest[index] =
                u8::from_str_radix(pair, 16).map_err(|_| ArtifactRefError::InvalidHex)?;
        }

        Ok(Self { digest })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ZERO_REF: &str =
        "sha256:0000000000000000000000000000000000000000000000000000000000000000";

    #[test]
    fn parses_canonical_sha256_reference() {
        let artifact: ArtifactRef = ZERO_REF.parse().expect("valid artifact ref");
        assert_eq!(artifact.to_string(), ZERO_REF);
    }

    #[test]
    fn canonicalizes_hex_to_lowercase() {
        let artifact: ArtifactRef =
            "sha256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
                .parse()
                .expect("valid artifact ref");

        assert_eq!(
            artifact.to_string(),
            "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        );
    }

    #[test]
    fn rejects_unknown_scheme() {
        assert_eq!(
            "sha512:0000".parse::<ArtifactRef>(),
            Err(ArtifactRefError::UnsupportedScheme)
        );
    }

    #[test]
    fn rejects_wrong_digest_length() {
        assert_eq!(
            "sha256:abcd".parse::<ArtifactRef>(),
            Err(ArtifactRefError::InvalidDigestLength)
        );
    }

    #[test]
    fn rejects_non_hex_digest() {
        let bad = "sha256:gg00000000000000000000000000000000000000000000000000000000000000";

        assert_eq!(
            bad.parse::<ArtifactRef>(),
            Err(ArtifactRefError::InvalidHex)
        );
    }
}
