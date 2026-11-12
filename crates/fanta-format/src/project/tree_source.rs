//! Where the project reader's bytes come from: the real directory tree, or an
//! in-memory file map (spec 0008 R1 — the same assembler runs in the browser,
//! where there is no filesystem). `Mem` readers pass the project root as an
//! empty path, so every `dir.join(...)` the assembler builds is exactly the
//! map's relative key and no path plumbing changes shape.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::error::Result;

#[derive(Clone, Copy)]
pub(crate) enum TreeSource<'a> {
    /// Read through `std::fs`; paths are absolute (rooted at the project dir).
    Dir,
    /// Read from the map; paths are the map's own relative keys.
    Mem(&'a BTreeMap<PathBuf, Vec<u8>>),
}

impl TreeSource<'_> {
    pub(crate) fn read(&self, path: &Path) -> std::io::Result<Vec<u8>> {
        match self {
            TreeSource::Dir => std::fs::read(path),
            TreeSource::Mem(files) => files.get(path).cloned().ok_or_else(|| not_found(path)),
        }
    }

    pub(crate) fn read_to_string(&self, path: &Path) -> std::io::Result<String> {
        match self {
            TreeSource::Dir => std::fs::read_to_string(path),
            TreeSource::Mem(files) => {
                let bytes = files.get(path).ok_or_else(|| not_found(path))?;
                String::from_utf8(bytes.clone()).map_err(|error| {
                    std::io::Error::new(std::io::ErrorKind::InvalidData, error.utf8_error())
                })
            }
        }
    }

    pub(crate) fn exists(&self, path: &Path) -> bool {
        match self {
            TreeSource::Dir => path.exists(),
            TreeSource::Mem(_) => self.is_file(path) || self.is_dir(path),
        }
    }

    pub(crate) fn is_file(&self, path: &Path) -> bool {
        match self {
            TreeSource::Dir => path.is_file(),
            TreeSource::Mem(files) => files.contains_key(path),
        }
    }

    pub(crate) fn is_dir(&self, path: &Path) -> bool {
        match self {
            TreeSource::Dir => path.is_dir(),
            TreeSource::Mem(files) => files
                .keys()
                .any(|key| key != path && key.starts_with(path)),
        }
    }

    /// Immediate children of `dir`, sorted — files and directories alike,
    /// mirroring [`super::layout::sorted_entries`]. A `Mem` dir that holds
    /// nothing yields an empty list (every reader call site probes
    /// `is_dir`/`is_file` first, so the fs variant's missing-dir error has no
    /// equivalent to preserve).
    pub(crate) fn sorted_entries(&self, dir: &Path) -> Result<Vec<PathBuf>> {
        match self {
            TreeSource::Dir => super::layout::sorted_entries(dir),
            TreeSource::Mem(_) => Ok(self.children(dir)),
        }
    }

    /// Immediate subdirectories of `dir`, sorted; the fs variant skips
    /// symlinks (a symlinked design dir is not a design).
    pub(crate) fn sorted_dirs(&self, dir: &Path) -> std::io::Result<Vec<PathBuf>> {
        match self {
            TreeSource::Dir => {
                let mut dirs = Vec::new();
                for entry in std::fs::read_dir(dir)? {
                    let path = entry?.path();
                    if std::fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.is_dir()) {
                        dirs.push(path);
                    }
                }
                dirs.sort();
                Ok(dirs)
            }
            TreeSource::Mem(_) => Ok(self
                .children(dir)
                .into_iter()
                .filter(|child| self.is_dir(child))
                .collect()),
        }
    }

    /// Newest file mtime directly inside `dir` — the duplicate-design
    /// tie-break signal. A `Mem` map carries no times; duplicate dirs are a
    /// crashed save's disk artifact, so the deterministic index fallback in
    /// `select_design_winners` decides there.
    pub(crate) fn newest_mtime(&self, dir: &Path) -> SystemTime {
        match self {
            TreeSource::Dir => {
                let mut newest = SystemTime::UNIX_EPOCH;
                if let Ok(entries) = std::fs::read_dir(dir) {
                    for entry in entries.flatten() {
                        if let Ok(modified) =
                            entry.metadata().and_then(|metadata| metadata.modified())
                        {
                            newest = newest.max(modified);
                        }
                    }
                }
                newest
            }
            TreeSource::Mem(_) => SystemTime::UNIX_EPOCH,
        }
    }

    fn children(&self, dir: &Path) -> Vec<PathBuf> {
        let TreeSource::Mem(files) = self else {
            return Vec::new();
        };
        let mut children = BTreeSet::new();
        for key in files.keys() {
            let Ok(rest) = key.strip_prefix(dir) else {
                continue;
            };
            if let Some(first) = rest.components().next() {
                children.insert(dir.join(first));
            }
        }
        children.into_iter().collect()
    }
}

fn not_found(path: &Path) -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::NotFound,
        format!("{} is not in the project file set", path.display()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(entries: &[&str]) -> BTreeMap<PathBuf, Vec<u8>> {
        entries
            .iter()
            .map(|path| (PathBuf::from(path), b"x".to_vec()))
            .collect()
    }

    #[test]
    fn mem_probes_and_lists_like_a_tree() {
        let files = map(&[
            "fanta.json",
            "pages/home/page.svg",
            "pages/home/page.json",
            "pages/two/page.json",
            "components/chip/def.json",
        ]);
        let source = TreeSource::Mem(&files);
        assert!(source.is_file(Path::new("fanta.json")));
        assert!(!source.is_dir(Path::new("fanta.json")));
        assert!(source.is_dir(Path::new("pages")));
        assert!(source.is_dir(Path::new("pages/home")));
        assert!(!source.is_file(Path::new("pages/home")));
        assert!(source.exists(Path::new("pages")) && source.exists(Path::new("fanta.json")));
        assert!(!source.exists(Path::new("missing")));
        assert_eq!(
            source.sorted_entries(Path::new("pages")).unwrap(),
            vec![PathBuf::from("pages/home"), PathBuf::from("pages/two")]
        );
        // The project root is the empty path: children are the top entries.
        assert_eq!(
            source.sorted_dirs(Path::new("")).unwrap(),
            vec![PathBuf::from("components"), PathBuf::from("pages")]
        );
        assert!(source.read(Path::new("pages/home/page.svg")).is_ok());
        assert_eq!(
            source.read(Path::new("pages/home")).unwrap_err().kind(),
            std::io::ErrorKind::NotFound
        );
    }
}
