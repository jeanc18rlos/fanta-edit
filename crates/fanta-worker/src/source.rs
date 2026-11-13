//! Reads a run's GitHub source into a private folder: the commit's project
//! folder, extracted from the tarball as it streams in and within limits,
//! with the overlay laid over it.

use std::{
    io::{self, Read},
    path::{Component, Path, PathBuf},
    time::Duration,
};

use base64::Engine as _;

use crate::{EngineError, ErrorCode, Overlay, Token};

/// What one extraction may write.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub bytes: u64,
    /// Files and folders together.
    pub files: usize,
}

impl Limits {
    /// The contract's archive limits.
    pub const SOURCE: Self = Self {
        bytes: 512 * 1024 * 1024,
        files: 20_000,
    };
}

/// A `/`-separated path inside the project made only of ordinary names: no
/// `..`, no root, no drive prefix and no `.`. Empty means the root.
pub fn relative_path(path: &str) -> Result<PathBuf, EngineError> {
    let mut relative = PathBuf::new();
    for component in Path::new(path).components() {
        match component {
            Component::Normal(name) => relative.push(name),
            _ => {
                return Err(EngineError::new(
                    ErrorCode::BadRequest,
                    format!("{path:?} must be a relative path of plain names"),
                ));
            }
        }
    }
    Ok(relative)
}

/// An overlay file's path: a [`relative_path`] that names a file outside
/// the folders the engine never reads.
pub fn overlay_path(path: &str) -> Result<PathBuf, EngineError> {
    let relative = relative_path(path)?;
    if relative.as_os_str().is_empty() {
        return Err(EngineError::new(
            ErrorCode::BadRequest,
            "an overlay file needs a path",
        ));
    }
    if reserved(&relative) {
        return Err(EngineError::new(
            ErrorCode::BadRequest,
            format!(
                "overlay file {path:?} is inside .fanta or .git, which the engine does not read"
            ),
        ));
    }
    Ok(relative)
}

/// Whether `path` is, or is inside, a `.fanta` or `.git` folder. Git runs
/// programs named by the files of both, and a project's source needs neither:
/// the app's snapshot store ignores itself, so a commit holds one only when
/// someone put it there.
fn reserved(path: &Path) -> bool {
    path.components().any(|component| {
        let name = component.as_os_str();
        name.eq_ignore_ascii_case(".fanta") || name.eq_ignore_ascii_case(".git")
    })
}

pub struct Github {
    api: String,
    agent: ureq::Agent,
}

impl Github {
    pub fn new(api: impl Into<String>) -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(120)))
            .http_status_as_error(false)
            .user_agent("fanta-engine")
            .build()
            .into();
        Self {
            api: api.into().trim_end_matches('/').to_owned(),
            agent,
        }
    }

    /// Checks that `token` can read the commit, then extracts its `path`
    /// folder into `destination`.
    pub fn fetch(
        &self,
        repo: &str,
        sha: &str,
        path: &str,
        token: &Token,
        destination: &Path,
    ) -> Result<(), EngineError> {
        self.get(&format!("/repos/{repo}/commits/{sha}"), repo, sha, token)?;
        let mut tarball = self.get(&format!("/repos/{repo}/tarball/{sha}"), repo, sha, token)?;
        extract(
            tarball.body_mut().as_reader(),
            path,
            destination,
            Limits::SOURCE,
        )
    }

    fn get(
        &self,
        route: &str,
        repo: &str,
        sha: &str,
        token: &Token,
    ) -> Result<ureq::http::Response<ureq::Body>, EngineError> {
        let response = self
            .agent
            .get(format!("{}{route}", self.api))
            .header("Authorization", format!("Bearer {}", token.0))
            .header("Accept", "application/vnd.github+json")
            .call()
            .map_err(|error| {
                EngineError::new(ErrorCode::Internal, format!("reaching GitHub: {error}"))
            })?;
        match response.status().as_u16() {
            200 => Ok(response),
            401 | 403 => Err(EngineError::new(
                ErrorCode::Forbidden,
                format!("the token cannot read {repo}"),
            )),
            404 | 422 => Err(EngineError::new(
                ErrorCode::NotFound,
                format!("{repo}@{sha} does not exist or is not visible to the token"),
            )),
            status => Err(EngineError::new(
                ErrorCode::Internal,
                format!("GitHub answered {status} for {route}"),
            )),
        }
    }
}

/// Extracts the `project_path` folder of a GitHub commit tarball into
/// `destination`. GitHub wraps the tree in one top folder, which is dropped.
/// Entries outside the project folder are skipped; inside it only plain files
/// and folders are accepted, so a link can't point a later write outside
/// `destination`, and nothing under `.fanta` or `.git` is.
pub fn extract(
    reader: impl Read,
    project_path: &str,
    destination: &Path,
    limits: Limits,
) -> Result<(), EngineError> {
    let prefix = relative_path(project_path)?;
    let corrupt = |error: io::Error| {
        EngineError::new(
            ErrorCode::ProjectLoadFailed,
            format!("reading the commit's archive: {error}"),
        )
    };
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(reader));
    let (mut bytes, mut files, mut found) = (0u64, 0usize, false);
    for entry in archive.entries().map_err(corrupt)? {
        let mut entry = entry.map_err(corrupt)?;
        let kind = entry.header().entry_type();
        // GitHub's archives start with a pax header holding the commit id.
        if kind.is_pax_global_extensions() || kind.is_pax_local_extensions() {
            continue;
        }
        let raw = entry.path().map_err(corrupt)?.into_owned();
        let mut components = raw.components();
        if !matches!(components.next(), Some(Component::Normal(_))) {
            return Err(unsafe_entry(&raw));
        }
        let Ok(relative) = components.as_path().strip_prefix(&prefix) else {
            continue;
        };
        if relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
        {
            return Err(unsafe_entry(&raw));
        }
        if reserved(relative) {
            return Err(EngineError::new(
                ErrorCode::ProjectLoadFailed,
                format!(
                    "{} is inside .fanta or .git, which the engine does not read; remove it from the commit",
                    raw.display()
                ),
            ));
        }
        if !kind.is_dir() && !kind.is_file() {
            return Err(unsafe_entry(&raw));
        }
        found = true;
        // Folders count too: an archive of empty folders costs inodes.
        files += 1;
        if kind.is_file() {
            bytes = bytes.saturating_add(entry.header().size().map_err(corrupt)?);
        }
        if files > limits.files || bytes > limits.bytes {
            return Err(EngineError::new(
                ErrorCode::PayloadTooLarge,
                format!(
                    "the project folder is over {} files and folders or {} bytes",
                    limits.files, limits.bytes
                ),
            ));
        }
        let target = destination.join(relative);
        if kind.is_dir() {
            std::fs::create_dir_all(&target).map_err(write_error)?;
        } else {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).map_err(write_error)?;
            }
            let mut file = std::fs::File::create(&target).map_err(write_error)?;
            io::copy(&mut entry, &mut file).map_err(corrupt)?;
        }
    }
    if !found {
        return Err(EngineError::new(
            ErrorCode::NotFound,
            format!("the commit has no project folder {project_path:?}"),
        ));
    }
    Ok(())
}

/// Lays the overlay's files over the extracted project folder.
pub fn apply_overlay(overlay: &Overlay, destination: &Path) -> Result<(), EngineError> {
    for file in &overlay.files {
        let relative = overlay_path(&file.path)?;
        let target = destination.join(&relative);
        match &file.content_b64 {
            Some(content) if !file.deleted => {
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(content)
                    .map_err(|error| {
                        EngineError::new(
                            ErrorCode::BadRequest,
                            format!("overlay file {:?} is not base64: {error}", file.path),
                        )
                    })?;
                if let Some(parent) = target.parent() {
                    std::fs::create_dir_all(parent).map_err(write_error)?;
                }
                std::fs::write(&target, bytes).map_err(write_error)?;
            }
            _ => match std::fs::remove_file(&target) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(write_error(error)),
            },
        }
    }
    Ok(())
}

fn unsafe_entry(path: &Path) -> EngineError {
    EngineError::new(
        ErrorCode::ProjectLoadFailed,
        format!(
            "{} is a link, a special file or an unsafe path; the engine reads only plain files and folders",
            path.display()
        ),
    )
}

fn write_error(error: io::Error) -> EngineError {
    EngineError::new(
        ErrorCode::Internal,
        format!("writing the checkout: {error}"),
    )
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) enum Entry<'a> {
        File(&'a str, &'a [u8]),
        Link(&'a str, &'a str),
        Folder(&'a str),
    }

    /// A gzipped tarball shaped like GitHub's: a pax global header, then the
    /// tree under one `owner-repo-sha/` folder.
    pub(crate) fn tarball(entries: &[Entry]) -> Vec<u8> {
        let mut builder = tar::Builder::new(flate2::write::GzEncoder::new(
            Vec::new(),
            flate2::Compression::fast(),
        ));
        let mut pax = tar::Header::new_ustar();
        pax.set_entry_type(tar::EntryType::XGlobalHeader);
        let comment = b"52 comment=0123456789abcdef0123456789abcdef01234567\n";
        pax.set_size(comment.len() as u64);
        pax.set_cksum();
        builder
            .append_data(&mut pax, "pax_global_header", &comment[..])
            .expect("pax header");
        for entry in entries {
            let mut header = tar::Header::new_gnu();
            match entry {
                Entry::File(path, bytes) => {
                    header.set_size(bytes.len() as u64);
                    header.set_mode(0o644);
                    header.set_cksum();
                    builder
                        .append_data(&mut header, format!("acme-site-0123456/{path}"), *bytes)
                        .expect("file entry");
                }
                Entry::Folder(path) => {
                    header.set_entry_type(tar::EntryType::Directory);
                    header.set_size(0);
                    header.set_mode(0o755);
                    header.set_cksum();
                    builder
                        .append_data(&mut header, format!("acme-site-0123456/{path}/"), &[][..])
                        .expect("folder entry");
                }
                Entry::Link(path, target) => {
                    header.set_entry_type(tar::EntryType::Symlink);
                    header.set_size(0);
                    builder
                        .append_link(&mut header, format!("acme-site-0123456/{path}"), target)
                        .expect("link entry");
                }
            }
        }
        builder.into_inner().expect("tar").finish().expect("gzip")
    }

    fn code(result: Result<(), EngineError>) -> Result<(), ErrorCode> {
        result.map_err(|error| error.error.code)
    }

    #[test]
    fn only_the_project_folder_is_extracted() {
        let archive = tarball(&[
            Entry::File("README.md", b"readme"),
            Entry::Link("docs/latest", "../README.md"),
            Entry::File("design/fanta.json", b"{}"),
            Entry::File("design/pages/home/page.fnx", b"<Frame/>"),
        ]);
        let destination = tempfile::tempdir().expect("destination");
        extract(&archive[..], "design", destination.path(), Limits::SOURCE).expect("extract");
        assert_eq!(
            std::fs::read(destination.path().join("pages/home/page.fnx")).expect("page"),
            b"<Frame/>"
        );
        assert!(destination.path().join("fanta.json").exists());
        assert!(!destination.path().join("README.md").exists());
    }

    #[test]
    fn links_escapes_and_oversized_folders_are_refused() {
        let destination = tempfile::tempdir().expect("destination");
        let linked = tarball(&[
            Entry::File("design/fanta.json", b"{}"),
            Entry::Link("design/assets", "/etc"),
        ]);
        assert_eq!(
            code(extract(
                &linked[..],
                "design",
                destination.path(),
                Limits::SOURCE
            )),
            Err(ErrorCode::ProjectLoadFailed)
        );
        let many = tarball(&[
            Entry::File("fanta.json", b"{}"),
            Entry::File("a.txt", b"a"),
            Entry::File("b.txt", b"b"),
        ]);
        let small = Limits {
            bytes: 1024,
            files: 2,
        };
        assert_eq!(
            code(extract(&many[..], "", destination.path(), small)),
            Err(ErrorCode::PayloadTooLarge)
        );
        let large = Limits {
            bytes: 3,
            files: 10,
        };
        assert_eq!(
            code(extract(&many[..], "", destination.path(), large)),
            Err(ErrorCode::PayloadTooLarge)
        );
        assert_eq!(
            code(extract(
                &many[..],
                "elsewhere",
                destination.path(),
                Limits::SOURCE
            )),
            Err(ErrorCode::NotFound)
        );
        assert_eq!(
            code(extract(
                &many[..],
                "../design",
                destination.path(),
                Limits::SOURCE
            )),
            Err(ErrorCode::BadRequest)
        );
    }

    #[test]
    fn the_overlay_writes_and_deletes_inside_the_project_only() {
        let destination = tempfile::tempdir().expect("destination");
        std::fs::write(destination.path().join("old.fnx"), "old").expect("old file");
        let overlay: Overlay = serde_json::from_value(serde_json::json!({"files": [
            {"path": "pages/new/page.fnx", "content_b64": "PEZyYW1lLz4="},
            {"path": "old.fnx", "deleted": true},
            {"path": "missing.fnx", "deleted": true}
        ]}))
        .expect("overlay");
        apply_overlay(&overlay, destination.path()).expect("overlay");
        assert_eq!(
            std::fs::read(destination.path().join("pages/new/page.fnx")).expect("new"),
            b"<Frame/>"
        );
        assert!(!destination.path().join("old.fnx").exists());

        for path in ["../escape.fnx", "/etc/passwd", ""] {
            let overlay: Overlay = serde_json::from_value(serde_json::json!({"files": [
                {"path": path, "content_b64": "eA=="}
            ]}))
            .expect("overlay");
            assert_eq!(
                code(apply_overlay(&overlay, destination.path())),
                Err(ErrorCode::BadRequest),
                "{path}"
            );
        }
    }

    #[test]
    fn the_snapshot_store_and_git_folders_are_refused() {
        for path in [
            ".fanta/history.git/config",
            ".git/config",
            "pages/.fanta/history.git/HEAD",
            ".FANTA/history.git/config",
        ] {
            let destination = tempfile::tempdir().expect("destination");
            let archive = tarball(&[
                Entry::File("design/fanta.json", b"{}"),
                Entry::File(&format!("design/{path}"), b"planted"),
            ]);
            assert_eq!(
                code(extract(
                    &archive[..],
                    "design",
                    destination.path(),
                    Limits::SOURCE
                )),
                Err(ErrorCode::ProjectLoadFailed),
                "{path}"
            );
            assert!(!destination.path().join(path).exists(), "{path}");

            for file in [
                serde_json::json!({"path": path, "content_b64": "eA=="}),
                serde_json::json!({"path": path, "deleted": true}),
            ] {
                let overlay: Overlay =
                    serde_json::from_value(serde_json::json!({"files": [file]})).expect("overlay");
                assert_eq!(
                    code(apply_overlay(&overlay, destination.path())),
                    Err(ErrorCode::BadRequest),
                    "{path}"
                );
            }
            assert!(!destination.path().join(path).exists(), "{path}");
        }

        let destination = tempfile::tempdir().expect("destination");
        let folder = tarball(&[
            Entry::File("fanta.json", b"{}"),
            Entry::Folder(".fanta/history.git/hooks"),
        ]);
        assert_eq!(
            code(extract(&folder[..], "", destination.path(), Limits::SOURCE)),
            Err(ErrorCode::ProjectLoadFailed)
        );
        // A name that only resembles them is an ordinary file.
        let ordinary = tarball(&[
            Entry::File("fanta.json", b"{}"),
            Entry::File(".fanta-notes/readme.txt", b"x"),
            Entry::File(".gitignore", b"previews/\n"),
        ]);
        extract(&ordinary[..], "", destination.path(), Limits::SOURCE).expect("ordinary names");
    }

    #[test]
    fn folders_count_toward_the_entry_limit() {
        let destination = tempfile::tempdir().expect("destination");
        let folders = tarball(&[
            Entry::File("fanta.json", b"{}"),
            Entry::Folder("a"),
            Entry::Folder("a/b"),
            Entry::Folder("a/b/c"),
        ]);
        let small = Limits {
            bytes: 1024,
            files: 3,
        };
        assert_eq!(
            code(extract(&folders[..], "", destination.path(), small)),
            Err(ErrorCode::PayloadTooLarge)
        );
        let enough = Limits {
            bytes: 1024,
            files: 4,
        };
        extract(&folders[..], "", destination.path(), enough).expect("within the limit");
    }
}
