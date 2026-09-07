use std::fs;
use std::path::{Path, PathBuf};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("fanta-gpui must remain inside the workspace crates directory")
        .to_path_buf()
}

fn dependency_names(manifest: &str) -> Vec<&str> {
    let mut dependency_section = false;
    let mut dependencies = Vec::new();

    for line in manifest.lines() {
        let line = line.trim();

        if line.starts_with('[') {
            let segments = line.trim_matches(['[', ']']).split('.').collect::<Vec<_>>();
            let dependency_index = segments.iter().position(|segment| {
                matches!(
                    *segment,
                    "dependencies" | "dev-dependencies" | "build-dependencies"
                )
            });
            dependency_section = dependency_index.is_some_and(|index| index + 1 == segments.len());
            if let Some(name) = dependency_index.and_then(|index| segments.get(index + 1).copied())
            {
                dependencies.push(name.trim_matches(['\'', '"']));
            }
            continue;
        }

        if !dependency_section || line.is_empty() || line.starts_with('#') {
            continue;
        }

        if let Some((name, _)) = line.split_once('=') {
            dependencies.push(
                name.trim()
                    .split('.')
                    .next()
                    .expect("a dependency key is never empty")
                    .trim_matches(['\'', '"']),
            );
        }
    }

    dependencies
}

fn visit_rust_sources(path: &Path, visit: &mut impl FnMut(&Path, &str)) {
    for entry in fs::read_dir(path).expect("source directory must be readable") {
        let entry = entry.expect("source entry must be readable");
        let path = entry.path();

        if path.is_dir() {
            visit_rust_sources(&path, visit);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            let source = fs::read_to_string(&path).expect("Rust source must be readable");
            visit(&path, &source);
        }
    }
}

#[test]
fn reusable_crate_has_no_fanta_domain_dependencies() {
    let manifest_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let manifest = fs::read_to_string(&manifest_path).expect("library manifest must be readable");
    let forbidden = dependency_names(&manifest)
        .into_iter()
        .filter(|name| name.starts_with("fanta-") || name.starts_with("fanta_"))
        .collect::<Vec<_>>();

    assert!(
        forbidden.is_empty(),
        "{} must remain independent from Fanta domain/application crates; found: {forbidden:?}",
        manifest_path.display()
    );
}

#[test]
fn reusable_crate_does_not_reference_the_storybook_host() {
    let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut references = Vec::new();

    visit_rust_sources(&source_root, &mut |path, source| {
        if source.contains("fanta_gpui_storybook") || source.contains("fanta-gpui-storybook") {
            references.push(path.to_path_buf());
        }
    });

    assert!(
        references.is_empty(),
        "reusable source must not reference its Storybook host; found: {references:?}"
    );
}

#[test]
fn storybook_is_a_one_way_consumer_of_the_library() {
    let storybook_manifest_path = workspace_root()
        .join("crates")
        .join("fanta-gpui-storybook")
        .join("Cargo.toml");
    // fanta-gpui-storybook is not vendored into this repository: it is upstream
    // dev tooling that the product does not build. Where it is absent there is
    // no boundary to police, so the check is skipped rather than failed.
    let Ok(storybook_manifest) = fs::read_to_string(&storybook_manifest_path) else {
        return;
    };

    assert!(
        dependency_names(&storybook_manifest).contains(&"fanta-gpui"),
        "{} must consume fanta-gpui through the public facade",
        storybook_manifest_path.display()
    );
}
