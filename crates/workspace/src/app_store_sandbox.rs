use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::LazyLock,
};

use anyhow::{Context as _, Result, anyhow};
use objc2::{rc::Retained, runtime::Bool};
use objc2_foundation::{
    NSData, NSURL, NSURLBookmarkCreationOptions, NSURLBookmarkResolutionOptions,
};
use parking_lot::Mutex;

static BOOKMARKS: LazyLock<Mutex<BookmarkStore>> =
    LazyLock::new(|| Mutex::new(BookmarkStore::default()));

#[derive(Default)]
struct BookmarkStore {
    data: BTreeMap<PathBuf, Vec<u8>>,
    active: BTreeMap<PathBuf, ActiveScope>,
}

struct ActiveScope {
    url: Retained<NSURL>,
    path: PathBuf,
    is_stale: bool,
}

impl Drop for ActiveScope {
    fn drop(&mut self) {
        unsafe { self.url.stopAccessingSecurityScopedResource() };
    }
}

fn bookmarks_path() -> PathBuf {
    paths::data_dir().join("security-scoped-bookmarks.json")
}

fn create_bookmark(path: &Path) -> Result<Vec<u8>> {
    let url = if path.is_dir() {
        NSURL::from_directory_path(path)
    } else {
        NSURL::from_file_path(path)
    }
    .ok_or_else(|| anyhow!("invalid file path {}", path.display()))?;
    let bookmark = url
        .bookmarkDataWithOptions_includingResourceValuesForKeys_relativeToURL_error(
            NSURLBookmarkCreationOptions::WithSecurityScope,
            None,
            None,
        )
        .map_err(|error| {
            anyhow!(
                "could not create bookmark for {}: {error:?}",
                path.display()
            )
        })?;
    Ok(bookmark.to_vec())
}

fn activate_bookmark(data: &[u8]) -> Result<ActiveScope> {
    let bookmark = NSData::with_bytes(data);
    let mut is_stale = Bool::NO;
    let url = unsafe {
        NSURL::URLByResolvingBookmarkData_options_relativeToURL_bookmarkDataIsStale_error(
            &bookmark,
            NSURLBookmarkResolutionOptions::WithSecurityScope
                | NSURLBookmarkResolutionOptions::WithoutUI,
            None,
            &mut is_stale,
        )
    }
    .map_err(|error| anyhow!("could not resolve security-scoped bookmark: {error:?}"))?;
    if !url.isFileURL() {
        return Err(anyhow!("bookmark did not resolve to a local file"));
    }
    let path = url
        .to_file_path()
        .ok_or_else(|| anyhow!("bookmark resolved to an invalid file path"))?;
    if !unsafe { url.startAccessingSecurityScopedResource() } {
        return Err(anyhow!("macOS did not grant access to {}", path.display()));
    }
    Ok(ActiveScope {
        url,
        path,
        is_stale: is_stale.is_true(),
    })
}

fn persist(data: &BTreeMap<PathBuf, Vec<u8>>) -> Result<()> {
    let path = bookmarks_path();
    let directory = path
        .parent()
        .ok_or_else(|| anyhow!("bookmark store has no parent directory"))?;
    std::fs::create_dir_all(directory)
        .with_context(|| format!("could not create {}", directory.display()))?;
    let temporary_path = path.with_extension("json.tmp");
    let encoded = serde_json::to_vec(data).context("could not serialize file access bookmarks")?;
    std::fs::write(&temporary_path, encoded)
        .with_context(|| format!("could not write {}", temporary_path.display()))?;
    std::fs::rename(&temporary_path, &path)
        .with_context(|| format!("could not save {}", path.display()))?;
    Ok(())
}

pub(crate) fn restore_access() -> Result<()> {
    let data = match std::fs::read(bookmarks_path()) {
        Ok(bytes) => serde_json::from_slice::<BTreeMap<PathBuf, Vec<u8>>>(&bytes)
            .context("could not parse saved file access bookmarks")?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
        Err(error) => return Err(error).context("could not read saved file access bookmarks"),
    };

    let mut active = BTreeMap::new();
    let mut refreshed = data.clone();
    let mut failures = 0;
    for (original_path, bookmark) in &data {
        match activate_bookmark(bookmark) {
            Ok(scope) => {
                if scope.path != *original_path {
                    log::warn!(
                        "Selected file moved from {} to {}; select its new location to relink recent workspaces",
                        original_path.display(),
                        scope.path.display(),
                    );
                }
                if scope.is_stale {
                    match create_bookmark(&scope.path) {
                        Ok(new_data) => {
                            refreshed.insert(original_path.clone(), new_data);
                        }
                        Err(error) => {
                            log::warn!(
                                "Could not refresh bookmark for {}: {error:#}",
                                original_path.display()
                            );
                        }
                    }
                }
                active.insert(original_path.clone(), scope);
            }
            Err(error) => {
                failures += 1;
                log::warn!(
                    "Could not restore access to {}: {error:#}",
                    original_path.display(),
                );
            }
        }
    }
    if refreshed != data {
        persist(&refreshed)?;
    }
    let mut store = BOOKMARKS.lock();
    store.data = refreshed;
    store.active = active;

    if failures > 0 {
        Err(anyhow!(
            "access to {failures} previously selected file or folder(s) expired; select them again from Recent Projects"
        ))
    } else {
        Ok(())
    }
}

/// Persist access granted by a macOS Open or Save panel before using the selected paths.
pub fn remember_user_selected_paths(selected_paths: &[PathBuf]) -> Result<()> {
    let mut additions = Vec::new();
    for path in selected_paths {
        if path.starts_with(paths::data_dir()) {
            continue;
        }
        let data = create_bookmark(path)?;
        let scope = activate_bookmark(&data)?;
        additions.push((path.clone(), data, scope));
    }
    if additions.is_empty() {
        return Ok(());
    }

    let mut store = BOOKMARKS.lock();
    let mut data = store.data.clone();
    for (path, bookmark, _) in &additions {
        data.insert(path.clone(), bookmark.clone());
    }
    persist(&data)?;
    store.data = data;
    for (path, _, scope) in additions {
        store.active.insert(path, scope);
    }
    Ok(())
}
