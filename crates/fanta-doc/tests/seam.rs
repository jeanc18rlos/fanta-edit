//! Executable guard for the renderer seam (ARCHITECTURE.md §2, README).
//!
//! The six core crates must never transitively depend on `skia-safe`.
//! This turns the documented rule into a blocking test that fails `cargo test --workspace`
//! if anyone accidentally adds a render dependency to the pure core.

use std::process::Command;

#[test]
// This test shells out to `cargo tree` to assert the dependency seam. clippy.toml
// denies `Command::output` in favour of smol's, but this is a synchronous test with
// no async runtime to schedule on.
#[allow(clippy::disallowed_methods)]
fn skia_seam_is_enforced() {
    // When running as a package test, CARGO_MANIFEST_DIR is the crate dir.
    // Walk up to the workspace root.
    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let crate_dir = std::path::Path::new(&manifest);
    // crates/fanta-doc -> crates -> workspace root
    let workspace_root = crate_dir
        .ancestors()
        .nth(2)
        .expect("could not locate workspace root");

    let output = Command::new("cargo")
        .current_dir(workspace_root)
        .args([
            "tree",
            "-e",
            "normal",
            "-p",
            "fanta-doc",
            "-p",
            "fanta-canvas",
            "-p",
            "fanta-tools",
            "-p",
            "fanta-format",
            "-p",
            "fanta-fnx",
            "-p",
            "fanta-fig-interop",
        ])
        .output()
        .expect("failed to execute `cargo tree` — is cargo in PATH?");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    // cargo tree can succeed even if there are warnings; we care about the output.
    let skia_mentions = stdout
        .lines()
        .filter(|line| line.contains("skia-safe"))
        .count();

    assert_eq!(
        skia_mentions, 0,
        "skia-safe leaked into a core (Skia-free) crate!\n\
         Core crates must stay pure: fanta-doc, fanta-canvas, fanta-tools, \
         fanta-format, fanta-fnx, fanta-fig-interop.\n\
         stderr: {}\n\
         tree output:\n{}",
        stderr, stdout
    );
}
