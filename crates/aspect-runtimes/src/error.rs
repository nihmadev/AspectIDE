use crate::to_hex;

/// Integrity expectation for a download. Every provisioned runtime must declare
/// one — a managed toolchain is prepended to PATH and (for rustup/get-pip) even
/// executed, so unverified bytes are never extracted or run.
pub enum Integrity {
    /// Vendor-published SHA-256 (lowercase hex, 64 chars), enforced byte-for-byte.
    Sha256(String),
}

impl Integrity {
    /// Parse + normalize an expected hex digest, rejecting anything that is not a
    /// well-formed SHA-256 so a malformed/empty pin fails closed (never skips).
    pub fn sha256(expected: &str) -> Result<Self, String> {
        let hex = expected.trim().to_ascii_lowercase();
        if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(format!(
                "Refusing download: malformed SHA-256 checksum ({} chars)",
                hex.len()
            ));
        }
        Ok(Self::Sha256(hex))
    }

    /// Compare the digest of the fully-downloaded bytes against the expectation.
    pub fn verify(&self, actual: &[u8; 32]) -> Result<(), String> {
        let Self::Sha256(expected) = self;
        let actual_hex = to_hex(actual);
        if actual_hex == *expected {
            Ok(())
        } else {
            Err(format!(
                "checksum mismatch — expected SHA-256 {expected}, got {actual_hex}"
            ))
        }
    }
}

/// Look up the SHA-256 for `file_name` in a `SHASUMS256.txt`-style manifest, whose
/// lines are `<hex>  <filename>` (filenames may carry a `*` binary-mode marker).
pub fn shasum_for(manifest: &str, file_name: &str) -> Result<String, String> {
    manifest
        .lines()
        .filter_map(|line| {
            let (hash, name) = line.split_once(char::is_whitespace)?;
            Some((hash.trim(), name.trim().trim_start_matches('*')))
        })
        .find(|(_, name)| *name == file_name)
        .map(|(hash, _)| hash.to_string())
        .ok_or_else(|| format!("No checksum for {file_name} in vendor manifest"))
}

/// Extract the digest from a `.sha256` sibling file, which is either a bare hex
/// digest or the `<hex>  <filename>` form. Returns the first whitespace-delimited
/// token, leaving final validation to [`Integrity::sha256`].
pub fn lone_sha256(body: &str) -> Result<String, String> {
    body.split_whitespace()
        .next()
        .map(str::to_string)
        .ok_or_else(|| "Empty .sha256 checksum file".to_string())
}

