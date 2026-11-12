//! Spec 0009 R10: the hostile-edit corpus under `tests/corpus`. Each case is
//! a project folder with one outside edit applied, and `<case>.expected.json`
//! lists what a checker must report for it.

use fanta_doc::value::VarValue;
use fanta_doc::value::VariableType;
use fanta_doc::variables::{Mode, Variable, VariableCollection};
use fanta_doc::{
    AssetId, BoundProp, CanvasNode, Color, ComponentDef, ComponentId, Doc, Fill, GroupNode,
    InstanceNode, ModeId, NodeData, NodeId, TextNode, Transform2D, VariableCollectionId,
    VariableId, VectorNode,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

fn node(data: NodeData, name: &str, parent: Option<NodeId>, x: f64) -> CanvasNode {
    let mut node = CanvasNode::new(data);
    node.name = name.into();
    node.parent = parent;
    node.transform = Transform2D::translation(x, 0.0);
    node
}

/// One page with a token-bound card, a label, a photo and a component
/// instance, plus the component's own page: every kind of reference the
/// checker follows.
fn base_project() -> (Doc, BTreeMap<AssetId, Vec<u8>>) {
    let mut doc = Doc::new();
    doc.metadata.title = "Corpus".into();
    let page = doc
        .scene
        .insert(node(NodeData::Group(GroupNode::default()), "Home", None, 0.0))
        .unwrap();
    let card = doc
        .scene
        .insert(node(
            NodeData::Vector(VectorNode::rect_solid(0.0, 0.0, 120.0, 80.0, Color::WHITE)),
            "Card",
            Some(page),
            0.0,
        ))
        .unwrap();
    doc.scene
        .insert(node(
            NodeData::Text(TextNode::new("Hello", 100.0, 20.0)),
            "Label",
            Some(page),
            140.0,
        ))
        .unwrap();

    let photo_bytes = b"\x89PNG\r\n\x1a\ncorpus photo".to_vec();
    let asset = fanta_format::asset_id_for_bytes(&photo_bytes);
    let mut photo = VectorNode::rect_solid(0.0, 0.0, 40.0, 40.0, Color::BLACK);
    photo.fills[0] = serde_json::from_value::<Fill>(serde_json::json!({
        "kind": "image", "asset": asset, "mode": "fill",
    }))
    .unwrap();
    doc.scene
        .insert(node(NodeData::Vector(photo), "Photo", Some(page), 260.0))
        .unwrap();

    let component_page = doc
        .scene
        .insert(node(NodeData::Group(GroupNode::default()), "Components", None, 0.0))
        .unwrap();
    let chip_root = doc
        .scene
        .insert(node(
            NodeData::Group(GroupNode {
                clip_size: Some([48.0, 24.0]),
                ..Default::default()
            }),
            "Chip",
            Some(component_page),
            0.0,
        ))
        .unwrap();
    doc.scene
        .insert(node(
            NodeData::Vector(VectorNode::rect_solid(0.0, 0.0, 48.0, 24.0, Color::BLACK)),
            "Chip background",
            Some(chip_root),
            0.0,
        ))
        .unwrap();
    let chip = ComponentId::new();
    doc.components
        .defs
        .insert(chip, ComponentDef::new(chip, chip_root, "Chip"));
    doc.scene
        .insert(node(
            NodeData::Instance(InstanceNode {
                component: chip,
                overrides: Vec::new(),
                prop_values: BTreeMap::new(),
                derived: Vec::new(),
                local_size: [48.0, 24.0],
            }),
            "Chip instance",
            Some(page),
            320.0,
        ))
        .unwrap();
    doc.add_page(page);
    doc.add_page(component_page);

    let collection = VariableCollectionId::new();
    let variable = VariableId::new();
    let mode = ModeId::new();
    doc.variables.collections.insert(
        collection,
        VariableCollection {
            id: collection,
            name: "Theme".into(),
            modes: vec![Mode {
                id: mode,
                name: "Light".into(),
            }],
            default_mode: mode,
            variable_order: Vec::new(),
        },
    );
    doc.variables.variables.insert(
        variable,
        Variable {
            id: variable,
            collection,
            name: "Surface".into(),
            ty: VariableType::Color,
            values_by_mode: BTreeMap::from([(mode, VarValue::Color { value: Color::WHITE })]),
            scopes: Vec::new(),
            options: Vec::new(),
        },
    );
    doc.scene
        .get_mut(card)
        .unwrap()
        .bindings
        .insert(BoundProp::FillColor { index: 0 }, variable);

    (doc, BTreeMap::from([(asset, photo_bytes)]))
}


/// What a case must produce, in the order the checker reports it. Messages
/// are not part of the contract; codes, severities and places are.
fn summary(findings: &[fanta_format::SourceDiagnostic]) -> Value {
    Value::Array(
        findings
            .iter()
            .map(|finding| {
                let mut entry = serde_json::Map::new();
                entry.insert("code".into(), json!(finding.code));
                entry.insert("severity".into(), json!(finding.severity));
                if let Some(file) = &finding.file {
                    entry.insert("file".into(), json!(file));
                }
                if let Some(line) = finding.line {
                    entry.insert("line".into(), json!(line));
                }
                Value::Object(entry)
            })
            .collect(),
    )
}

fn corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus")
}

/// Spec 0009 A2: every committed case produces exactly its expected findings.
#[test]
fn every_corpus_case_produces_its_expected_findings() {
    let mut cases = 0;
    for entry in fs::read_dir(corpus_dir()).unwrap() {
        let case = entry.unwrap().path();
        if !case.is_dir() {
            continue;
        }
        let expected: Value =
            serde_json::from_str(&fs::read_to_string(case.with_extension("expected.json")).unwrap())
                .unwrap();
        let findings = fanta_format::check_project(&case).unwrap();
        assert_eq!(summary(&findings), expected, "{}: {findings:#?}", case.display());
        cases += 1;
    }
    assert_eq!(cases, CASES.len(), "a case folder is missing or unlisted");
}

/// Spec 0009 A1: the unedited project has nothing to report.
#[test]
fn the_pristine_case_is_clean() {
    assert_eq!(
        fanta_format::check_project(&corpus_dir().join("pristine")).unwrap(),
        Vec::new()
    );
}

/// Spec 0009 R1: the command's exit status and `--json` output.
#[test]
fn the_command_exits_0_clean_1_on_errors_and_2_outside_a_project() {
    // A test may block on the child it spawns.
    #[allow(clippy::disallowed_methods)]
    let run = |dir: &Path| {
        std::process::Command::new(env!("CARGO_BIN_EXE_fanta-check"))
            .arg(dir)
            .arg("--json")
            .output()
            .unwrap()
    };
    let clean = run(&corpus_dir().join("pristine"));
    assert_eq!(clean.status.code(), Some(0));
    assert_eq!(
        serde_json::from_slice::<Value>(&clean.stdout).unwrap(),
        json!([])
    );
    // Warnings alone do not fail the run.
    assert_eq!(run(&corpus_dir().join("missing-token")).status.code(), Some(0));
    let conflict = run(&corpus_dir().join("merge-conflict"));
    assert_eq!(conflict.status.code(), Some(1));
    let findings: Value = serde_json::from_slice(&conflict.stdout).unwrap();
    assert_eq!(findings[0]["code"], "parse.conflict_marker");
    assert_eq!(findings[0]["file"], "pages/home/page.css");
    assert_eq!(run(&corpus_dir()).status.code(), Some(2));
}

/// A case: its folder name, the codes it must produce (sorted), and the edit
/// that makes it from the base project.
type Case = (&'static str, &'static [&'static str], fn(&Path));

const CASES: &[Case] = &[
    ("pristine", &[], |_| {}),
    ("deleted-page-svg", &["manifest.file_missing"], |dir| {
        fs::remove_file(dir.join("pages/home/page.svg")).unwrap();
    }),
    ("deleted-records", &["manifest.file_missing"], |dir| {
        fs::remove_file(dir.join("pages/home/nodes.json")).unwrap();
    }),
    ("renamed-id", &["id.not_a_node", "record.no_element"], |dir| {
        edit(dir, "pages/home/page.svg", |svg| {
            let id = id_of(&svg, "card");
            svg.replace(&format!("id=\"{id}\""), "id=\"card\"")
        });
    }),
    ("pasted-element", &["id.duplicate"], |dir| {
        edit(dir, "pages/home/page.svg", |svg| {
            let card = svg.lines().find(|line| line.contains("class=\"card\"")).unwrap();
            svg.replace(card, &format!("{card}\n{card}"))
        });
    }),
    ("reformatted", &[], |dir| {
        for name in ["pages/home/nodes.json", "pages/home/page.json", "doc/variables.json", "fanta.json"] {
            edit(dir, name, |json| {
                serde_json::from_str::<Value>(&json).unwrap().to_string()
            });
        }
        edit(dir, "pages/home/page.css", |css| {
            css.replace("{\n  ", "\n{\n\t").replace(";\n  ", ";\n\t").replace(": ", ":")
        });
        edit(dir, "pages/home/page.svg", |svg| {
            svg.lines()
                .map(|line| {
                    let indent = line.len() - line.trim_start().len();
                    line.replace("\" ", &format!("\"\n{}", " ".repeat(indent + 4)))
                })
                .collect::<Vec<_>>()
                .join("\n")
        });
    }),
    ("resaved-by-vector-app", &["edit.not_kept", "id.not_a_node"], |dir| {
        // What an SVG application does on save: its own namespaces and
        // metadata, presentation attributes folded into `style`, attribute
        // order changed, the stylesheet instructions dropped.
        edit(dir, "pages/home/page.svg", |svg| {
            let body = svg.split_once("<svg ").unwrap().1;
            let card = body.lines().find(|line| line.contains("class=\"card\"")).unwrap();
            let id = id_of(body, "card");
            let resaved = format!(
                "    <rect style=\"fill:#ff0000;stroke:none\" height=\"80\" width=\"120\" y=\"0\" x=\"0\" id=\"{id}\" inkscape:label=\"Card\"/>"
            );
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"no\"?>\n<svg xmlns:inkscape=\"http://www.inkscape.org/namespaces/inkscape\" xmlns:sodipodi=\"http://sodipodi.sourceforge.net/DTD/sodipodi-0.dtd\" inkscape:version=\"1.3\" sodipodi:docname=\"page.svg\" {}",
                body.replace(card, &resaved).replacen(
                    "  <defs>",
                    "  <sodipodi:namedview id=\"namedview1\" pagecolor=\"#ffffff\"/>\n  <defs>",
                    1
                )
            )
        });
    }),
    ("merge-conflict", &["parse.conflict_marker"], |dir| {
        edit(dir, "pages/home/page.css", |css| {
            let ours = "  fill: var(--theme-surface, #FFFFFF);";
            css.replace(
                ours,
                &format!("<<<<<<< HEAD\n{ours}\n=======\n  fill: #FF0000;\n>>>>>>> feature"),
            )
        });
    }),
    ("missing-token", &["ref.missing_token"], |dir| {
        edit(dir, "pages/home/page.css", |css| {
            css.replace("var(--theme-surface,", "var(--theme-missing,")
        });
    }),
    (
        "deleted-component",
        &["record.missing", "record.missing", "ref.missing_component"],
        |dir| fs::remove_dir_all(dir.join("components/chip")).unwrap(),
    ),
    ("missing-asset", &["ref.missing_asset"], |dir| {
        fs::remove_dir_all(dir.join("assets/images")).unwrap();
    }),
];

fn edit(dir: &Path, name: &str, change: impl FnOnce(String) -> String) {
    let path = dir.join(name);
    let before = fs::read_to_string(&path).unwrap();
    let after = change(before.clone());
    assert_ne!(before, after, "{name}: the edit changed nothing");
    fs::write(path, after).unwrap();
}

/// The `id` of the element with this class.
fn id_of(svg: &str, class: &str) -> String {
    let line = svg
        .lines()
        .find(|line| line.contains(&format!("class=\"{class}\"")))
        .unwrap();
    line.split("id=\"").nth(1).unwrap().split('"').next().unwrap().to_owned()
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let path = entry.unwrap().path();
        let target = to.join(path.file_name().unwrap());
        if path.is_dir() {
            copy_tree(&path, &target);
        } else {
            fs::copy(&path, &target).unwrap();
        }
    }
}

/// Rebuild `tests/corpus` from the base project. Node ids are new on every
/// run, so regenerate only to add or change a case, and review the diff.
/// The declared codes guard against blessing a wrong result.
#[test]
#[ignore = "rewrites tests/corpus; run when adding or changing a case"]
fn regenerate_corpus() {
    let base = tempfile::tempdir().unwrap();
    let (doc, assets) = base_project();
    fanta_format::write_web_project_tree(base.path(), &doc, &assets, None, &[]).unwrap();
    let corpus = corpus_dir();
    for (name, codes, change) in CASES {
        let case = corpus.join(name);
        if case.exists() {
            fs::remove_dir_all(&case).unwrap();
        }
        fs::create_dir_all(&case).unwrap();
        // Only what a load reads: the viewer, guide and editor support files
        // are the same in every project and would bury the case.
        for file in ["fanta.json", "design.css", "tokens.css"] {
            fs::copy(base.path().join(file), case.join(file)).unwrap();
        }
        for folder in ["doc", "pages", "components", "assets"] {
            copy_tree(&base.path().join(folder), &case.join(folder));
        }
        change(&case);
        let findings = fanta_format::check_project(&case).unwrap();
        let mut produced: Vec<&str> = findings.iter().map(|finding| finding.code).collect();
        produced.sort_unstable();
        assert_eq!(&produced, codes, "{name}: {findings:#?}");
        let mut expected = serde_json::to_string_pretty(&summary(&findings)).unwrap();
        expected.push('\n');
        fs::write(case.with_extension("expected.json"), expected).unwrap();
    }
}

fn files_in(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut found = BTreeMap::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(folder) = pending.pop() {
        for entry in fs::read_dir(&folder).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else {
                found.insert(path.clone(), fs::read(&path).unwrap());
            }
        }
    }
    found
}

/// Spec 0009 R11, A10 at the format layer: a design with a file that does
/// not parse is left out of the opened document, a save leaves its folder
/// byte-identical, and repairing the file brings the design back.
#[test]
fn a_design_that_does_not_parse_is_left_out_and_survives_a_save() {
    for (broken, design, pages_left) in [
        ("pages/home/page.svg", "pages/home", 1),
        ("pages/home/page.css", "pages/home", 1),
        ("pages/home/nodes.json", "pages/home", 1),
        ("pages/home/page.json", "pages/home", 1),
        ("components/chip/master.svg", "components/chip", 2),
        ("components/chip/def.json", "components/chip", 2),
    ] {
        let scratch = tempfile::tempdir().unwrap();
        let root = scratch.path().canonicalize().unwrap();
        copy_tree(&corpus_dir().join("pristine"), &root);
        let original = fs::read_to_string(root.join(broken)).unwrap();
        fs::write(
            root.join(broken),
            format!("<<<<<<< HEAD\n{original}=======\n>>>>>>> theirs\n"),
        )
        .unwrap();
        let hostile = files_in(&root.join(design));

        let (doc, assets, session) =
            fanta_format::WorkspaceSession::open_project_document(&root)
                .unwrap_or_else(|error| panic!("{broken}: {error}"));
        assert_eq!(doc.pages().len(), pages_left, "{broken}");
        let found: Vec<_> = session
            .open_warnings
            .iter()
            .filter(|finding| finding.severity == fanta_format::SourceSeverity::Error)
            .map(|finding| (finding.code, finding.file.as_deref().unwrap_or_default()))
            .collect();
        assert_eq!(
            found,
            [
                ("parse.conflict_marker", broken),
                (fanta_format::DESIGN_UNAVAILABLE, design)
            ],
            "{broken}"
        );

        fanta_format::write_web_project_tree(&root, &doc, &assets, None, &session.open_warnings)
            .unwrap_or_else(|error| panic!("{broken}: {error}"));
        assert_eq!(files_in(&root.join(design)), hostile, "{broken}: the save touched the design");

        fs::write(root.join(broken), original).unwrap();
        let (repaired, _, session) =
            fanta_format::WorkspaceSession::open_project_document(&root).unwrap();
        assert_eq!(repaired.pages().len(), 2, "{broken}");
        assert!(
            !session
                .open_warnings
                .iter()
                .any(|finding| finding.severity == fanta_format::SourceSeverity::Error),
            "{broken}: {:#?}",
            session.open_warnings
        );
    }
}

/// A new page whose name maps to the unavailable design's folder must not
/// write over it.
#[test]
fn a_save_refuses_to_write_into_an_unavailable_design() {
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path().canonicalize().unwrap();
    copy_tree(&corpus_dir().join("pristine"), &root);
    fs::write(root.join("pages/home/page.svg"), "<<<<<<< HEAD\n").unwrap();
    let hostile = files_in(&root.join("pages/home"));
    let (mut doc, assets, session) =
        fanta_format::WorkspaceSession::open_project_document(&root).unwrap();
    let page = doc
        .scene
        .insert(node(NodeData::Group(GroupNode::default()), "Home", None, 0.0))
        .unwrap();
    doc.add_page(page);
    let error =
        fanta_format::write_web_project_tree(&root, &doc, &assets, None, &session.open_warnings)
            .unwrap_err();
    assert!(error.to_string().contains("could not be loaded"), "{error}");
    assert_eq!(files_in(&root.join("pages/home")), hostile);
}

/// The editor replaces its session after an outside change, and other hosts
/// save with no findings at all. Neither may cost the user the broken design.
#[test]
fn a_save_without_the_load_findings_still_keeps_an_unparsable_design() {
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path().canonicalize().unwrap();
    copy_tree(&corpus_dir().join("pristine"), &root);
    fs::write(root.join("pages/home/page.svg"), "<<<<<<< HEAD\n").unwrap();
    let hostile = files_in(&root.join("pages/home"));
    let (doc, assets, _) = fanta_format::WorkspaceSession::open_project_document(&root).unwrap();

    let reopened = fanta_format::WorkspaceSession::open(&root).unwrap();
    assert!(
        reopened
            .open_warnings
            .iter()
            .any(|finding| finding.code == fanta_format::DESIGN_UNAVAILABLE)
    );
    fanta_format::write_web_project_tree(&root, &doc, &assets, None, &[]).unwrap();
    assert_eq!(files_in(&root.join("pages/home")), hostile);
}

/// Spec 0009 R6: the snapshot store under `.fanta/` is invisible to the
/// check, the loader and the save.
#[test]
fn the_snapshot_store_is_skipped_by_the_check_the_loader_and_the_save() {
    use fanta_format::history::{History, SnapshotReason};
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path().canonicalize().unwrap();
    copy_tree(&corpus_dir().join("pristine"), &root);
    let taken = History::new(&root).record(SnapshotReason::BeforeSave).unwrap();

    assert_eq!(fanta_format::check_project(&root).unwrap(), Vec::new());
    let (doc, assets, session) = fanta_format::WorkspaceSession::open_project_document(&root).unwrap();
    assert_eq!(session.open_warnings, Vec::new());
    fanta_format::write_web_project_tree(&root, &doc, &assets, None, &[]).unwrap();
    assert_eq!(History::new(&root).list().unwrap(), [taken]);
}
