//! The `--frames` mode (spec 0008 T4): keep one PNG of a project page fresh
//! on disk while the project changes, printing a JSON status line per event.
//! A terminal pane points an image element at the file, so the terminal reads
//! the pixels itself and none cross the plugin boundary.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use anyhow::{Context as _, Result};
use fanta_headless::{HeadlessProject, PageRef, RenderTarget, pages};
use serde_json::{Value, json};

/// Directories a change scan ignores, matching `script/serve-project`.
const SKIPPED_DIRECTORIES: [&str; 4] = [".git", "exports", "previews", "node_modules"];

pub struct FrameOptions {
    pub root: PathBuf,
    pub out: PathBuf,
    pub page: Option<usize>,
    pub max_dimension: u32,
    pub interval: Duration,
    pub once: bool,
}

pub fn run(options: &FrameOptions) -> Result<()> {
    let mut generation: u64 = 0;
    let mut last_change = None;
    let stdout = std::io::stdout();
    loop {
        let change = latest_change(&options.root, &options.out);
        if last_change != Some(change) {
            last_change = Some(change);
            let line = match render_frame(options, generation + 1) {
                Ok(line) => {
                    generation += 1;
                    line
                }
                // A half-saved project must not kill the watch; the pane
                // shows the message and the next change renders again.
                Err(error) if !options.once => {
                    json!({ "event": "error", "message": format!("{error:#}") })
                }
                Err(error) => return Err(error),
            };
            let mut handle = stdout.lock();
            serde_json::to_writer(&mut handle, &line).context("writing a status line")?;
            handle.write_all(b"\n")?;
            handle.flush()?;
        }
        if options.once {
            return Ok(());
        }
        std::thread::sleep(options.interval);
    }
}

/// shortcut: reopens the whole project per change; move to an incremental
/// reload when watching large projects feels slow.
fn render_frame(options: &FrameOptions, generation: u64) -> Result<Value> {
    let mut project = HeadlessProject::open(&options.root)?;
    let index = match options.page {
        Some(index) => index,
        None => project.active_page_index()?,
    };
    let target = RenderTarget {
        id: format!("frame-{generation}"),
        page: PageRef::Index { index },
        node: None,
        max_dimension: options.max_dimension,
        motion_clip: None,
        playhead_ms: None,
    };
    let image = project.render(&target)?;
    let staged = options.out.with_extension("png.tmp");
    std::fs::write(&staged, &image.png)
        .with_context(|| format!("writing {}", staged.display()))?;
    std::fs::rename(&staged, &options.out)
        .with_context(|| format!("replacing {}", options.out.display()))?;

    let doc = project.document().doc();
    let roots = pages::editor_page_roots(doc);
    let page_list: Vec<Value> = roots
        .iter()
        .enumerate()
        .map(|(page_index, root)| {
            let (name, hidden) = pages::page_label(doc, *root, page_index);
            json!({ "index": page_index, "name": name, "hidden": hidden })
        })
        .collect();
    let (name, _hidden) = pages::page_label(doc, roots.get(index).copied().flatten(), index);
    Ok(json!({
        "event": "frame",
        "generation": generation,
        "out": options.out.display().to_string(),
        "page": { "index": index, "name": name },
        "width": image.width,
        "height": image.height,
        "pages": page_list,
    }))
}

/// The newest file mtime under `root` in nanoseconds, ignoring
/// [`SKIPPED_DIRECTORIES`] and the frame file itself (in case it is written
/// inside the project, which would otherwise re-trigger forever).
fn latest_change(root: &Path, out: &Path) -> u128 {
    let staged = out.with_extension("png.tmp");
    let mut latest = 0;
    let mut stack = vec![root.to_path_buf()];
    while let Some(directory) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path == out || path == staged {
                continue;
            }
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                let name = entry.file_name();
                if !SKIPPED_DIRECTORIES.iter().any(|skip| name == *skip) {
                    stack.push(path);
                }
            } else if let Ok(metadata) = entry.metadata() {
                if let Ok(since) = metadata.modified().map(|time| time.duration_since(UNIX_EPOCH)) {
                    if let Ok(since) = since {
                        latest = latest.max(since.as_nanos());
                    }
                }
            }
        }
    }
    latest
}

#[cfg(test)]
mod tests {
    use super::*;
    use fanta_headless::document::HeadlessDocument;
    use std::collections::BTreeMap;

    #[test]
    fn a_frame_lands_as_png_with_the_page_list() {
        let directory = tempfile::tempdir().expect("tempdir");
        let document = HeadlessDocument::empty().expect("empty document");
        fanta_format::write_project_tree(directory.path(), document.doc(), &BTreeMap::new())
            .expect("write project");
        let out = directory.path().join("frame.png");

        let options = FrameOptions {
            root: directory.path().to_path_buf(),
            out: out.clone(),
            page: None,
            max_dimension: 256,
            interval: Duration::from_millis(1),
            once: true,
        };
        let line = render_frame(&options, 1).expect("render");
        let bytes = std::fs::read(&out).expect("frame file");
        assert_eq!(&bytes[..4], b"\x89PNG", "not a PNG: {:?}", &bytes[..4]);
        assert_eq!(line["event"], json!("frame"), "{line}");
        assert_eq!(line["pages"].as_array().map(Vec::len), Some(1), "{line}");
        assert!(line["width"].as_u64().unwrap_or(0) > 0, "{line}");

        // The frame file lives inside the project here: the change scan must
        // not see its own writes, or the loop would re-render forever.
        let stamp = latest_change(directory.path(), &out);
        std::fs::write(&out, b"newer").expect("touch frame");
        assert_eq!(stamp, latest_change(directory.path(), &out));
    }
}
