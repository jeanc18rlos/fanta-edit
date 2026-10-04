//! Where does the first save of an opened project spend its memory?
//!
//! Profiles the stages the editor runs to open a project folder and save an
//! edit to its largest page (the first save, then the next autosave), with a
//! counting allocator, on a *copy* of the project (the writes land in the
//! directory you pass):
//!
//! ```sh
//! cargo run --release -p fanta-format --example first_save_profile -- <project-dir>
//! ```
//!
//! Each line reports wall time, the heap still held when the stage ends, and the
//! peak heap reached during it (above what was held when it started).

use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

struct Counting;

static CURRENT: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            let now = CURRENT.fetch_add(layout.size(), Ordering::Relaxed) + layout.size();
            PEAK.fetch_max(now, Ordering::Relaxed);
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) };
        CURRENT.fetch_sub(layout.size(), Ordering::Relaxed);
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

fn mib(bytes: usize) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}

fn stage<T>(name: &str, run: impl FnOnce() -> T) -> T {
    let before = CURRENT.load(Ordering::Relaxed);
    PEAK.store(before, Ordering::Relaxed);
    let started = Instant::now();
    let value = run();
    let elapsed = started.elapsed();
    let after = CURRENT.load(Ordering::Relaxed);
    let peak = PEAK.load(Ordering::Relaxed);
    println!(
        "{name:<34} {:>8.2} s   held {:>8.1} MiB   peak +{:>8.1} MiB",
        elapsed.as_secs_f64(),
        mib(after),
        mib(peak.saturating_sub(before)),
    );
    value
}

fn main() -> anyhow::Result<()> {
    let root: PathBuf = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("usage: first_save_profile <project-dir>"))?
        .into();

    let (mut doc, assets): (fanta_doc::Doc, BTreeMap<fanta_doc::AssetId, Vec<u8>>) =
        stage("open: read_project_tree", || {
            fanta_format::read_project_tree(&root)
        })?;
    let page = doc
        .pages()
        .iter()
        .copied()
        .max_by_key(|page| doc.scene.descendants_of(*page).count())
        .ok_or_else(|| anyhow::anyhow!("the project has no pages"))?;
    println!(
        "  {} nodes, {} assets; editing the largest page ({} nodes)",
        doc.scene.len(),
        assets.len(),
        doc.scene.descendants_of(page).count()
    );
    let target = doc
        .scene
        .children_of(Some(page))
        .first()
        .copied()
        .unwrap_or(page);
    let id = fanta_format::ArtifactId::Page(page);
    let mut cache = fanta_format::ProjectWriteCache::default();
    let mut session = None;

    // The editor's save path (`FigItem::save` in fig_viewer's document.rs),
    // twice: the first save after opening, then the next autosave.
    for (pass, dx) in [("first save", 3.0), ("next save", 5.0), ("third save", 9.0)] {
        println!("{pass}:");
        doc.scene.get_mut(target).expect("edited node").transform =
            fanta_doc::Transform2D::translation(dx, 0.0);
        let persisted = stage("  clone_for_persist", || doc.clone_for_persist());
        if session.is_none() {
            session = Some(stage("  WorkspaceSession::open", || {
                fanta_format::WorkspaceSession::open(&root)
            })?);
        }
        let workspace = session.as_mut().expect("session");
        stage("  open + adopt changed page", || -> anyhow::Result<()> {
            if !workspace.open.contains_key(&id) {
                workspace.open_artifact(id.clone())?;
            }
            workspace.adopt_document_shared(&persisted);
            workspace
                .artifact_mut(&id)
                .expect("open page")
                .adopt_document(&persisted)?;
            Ok(())
        })?;
        let (sources, expected) = stage("  source overrides + preconditions", || {
            anyhow::Ok((
                workspace.validated_source_overrides_for_document(&persisted)?,
                workspace.source_write_preconditions(&persisted)?,
            ))
        })?;
        let report = stage("  write with overrides", || {
            fanta_format::write_project_tree_cached_with_sources_checked(
                &root, &persisted, &assets, &mut cache, &sources, &expected,
            )
        })?;
        let index_hash = report
            .written_hashes
            .get(std::path::Path::new("assets/index.json"))
            .copied()
            .or_else(|| workspace.asset_index_disk_hash())
            .ok_or_else(|| anyhow::anyhow!("no asset index hash"))?;
        stage("  accept_written_sources", || {
            workspace.accept_written_sources(
                &persisted,
                index_hash,
                &sources,
                &report.written_hashes,
            )
        })?;
    }

    Ok(())
}
