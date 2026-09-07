//! Content hashing for artifact file sets (design N9).

use sha2::{Digest, Sha256};
use std::fmt;
use std::path::Path;

/// SHA-256 digest of one artifact file set.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ContentHash(pub [u8; 32]);

impl ContentHash {
    pub fn is_zero(&self) -> bool {
        self.0.iter().all(|&b| b == 0)
    }

    pub fn to_hex(&self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }
}

impl fmt::Debug for ContentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ContentHash({})", &self.to_hex()[..16])
    }
}

/// Domain-separated single digest over ordered `(relative_path, bytes)` pairs.
///
/// Skips entries with empty path that are marked optional-missing by not
/// including them in `files` at all.
pub fn hash_file_set(files: &[(impl AsRef<str>, impl AsRef<[u8]>)]) -> ContentHash {
    let mut hasher = Sha256::new();
    hasher.update(b"fanta-artifact-v1\0");
    for (path, data) in files {
        let path = path.as_ref().as_bytes();
        let data = data.as_ref();
        hasher.update(path);
        hasher.update([0x00]);
        hasher.update((data.len() as u64).to_le_bytes());
        hasher.update(data);
    }
    let digest = hasher.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    ContentHash(out)
}

/// Hash files that exist under `dir` for the given relative names (in order).
/// Missing files are skipped (optional slots).
pub fn hash_named_files_in_dir(dir: &Path, names: &[&str]) -> std::io::Result<ContentHash> {
    let mut pairs: Vec<(String, Vec<u8>)> = Vec::new();
    for name in names {
        let path = dir.join(name);
        if path.is_file() {
            let bytes = std::fs::read(&path)?;
            pairs.push(((*name).to_owned(), bytes));
        }
    }
    let refs: Vec<(&str, &[u8])> = pairs
        .iter()
        .map(|(n, b)| (n.as_str(), b.as_slice()))
        .collect();
    Ok(hash_file_set(&refs))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_is_stable() {
        let a = hash_file_set(&[
            ("page.fnx", b"hello".as_slice()),
            ("page.ids.json", b"{}".as_slice()),
        ]);
        let b = hash_file_set(&[
            ("page.fnx", b"hello".as_slice()),
            ("page.ids.json", b"{}".as_slice()),
        ]);
        assert_eq!(a, b);
    }

    #[test]
    fn hash_changes_with_content() {
        let a = hash_file_set(&[("page.fnx", b"hello".as_slice())]);
        let b = hash_file_set(&[("page.fnx", b"world".as_slice())]);
        assert_ne!(a, b);
    }

    #[test]
    fn hash_order_matters_in_table() {
        let a = hash_file_set(&[("a", b"1".as_slice()), ("b", b"2".as_slice())]);
        let b = hash_file_set(&[("b", b"2".as_slice()), ("a", b"1".as_slice())]);
        assert_ne!(a, b);
    }
}
