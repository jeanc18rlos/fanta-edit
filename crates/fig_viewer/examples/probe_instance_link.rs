//! Diagnostic: does editing a component master change its instances' expansion?
//!
//! Loads a `.fig`, groups every `InstanceNode` by the component it RESOLVES to,
//! and for the component with the most instances performs the exact experiment
//! the inspector does — recolor the master's first vector fill + bump the def
//! rev — then re-expands one instance and reports whether the expanded fill
//! changed. Answers "are these cards live instances of that master, and does a
//! master edit reach them?" on real data.
//!
//! Usage: cargo run -p fig_viewer --example probe_instance_link -- <path.fig> [name-substr]

use std::collections::HashMap;
use std::{env, fs, path::PathBuf};

use anyhow::{Context as _, Result, anyhow};
use fanta_doc::{
    Color, ComponentId, Fill, InstanceNode, NodeData, NodeId, expand_instance,
    resolved_component_rev,
};
use fanta_fig_interop::{fig_to_doc, read_fig};

fn main() -> Result<()> {
    let mut args = env::args().skip(1);
    let fig_path = args
        .next()
        .map(PathBuf::from)
        .ok_or_else(|| anyhow!("usage: probe_instance_link <path.fig> [name-substr]"))?;
    let name_filter = args.next();

    let bytes = fs::read(&fig_path).with_context(|| format!("reading {}", fig_path.display()))?;
    let fig = read_fig(&bytes).context("parsing .fig")?;
    let (mut doc, report, _assets) = fig_to_doc(&fig).context("mapping .fig")?;
    eprintln!(
        "mapped {} nodes | {} components, {} sets, {} instances ({} with derived)",
        report.mapped,
        report.components,
        report.component_sets,
        report.instances,
        report.instances_with_derived,
    );

    // Collect instances and the component each resolves to.
    let mut per_component: HashMap<ComponentId, Vec<(NodeId, InstanceNode)>> = HashMap::new();
    let all_ids: Vec<NodeId> = doc
        .scene
        .roots()
        .to_vec()
        .into_iter()
        .flat_map(|root| doc.scene.descendants_of(root).collect::<Vec<_>>())
        .collect();
    for id in all_ids {
        if let Some(node) = doc.scene.get(id)
            && let NodeData::Instance(inst) = &node.data
        {
            per_component
                .entry(inst.component)
                .or_default()
                .push((id, inst.clone()));
        }
    }
    eprintln!(
        "{} distinct component ids are instanced across {} instance nodes",
        per_component.len(),
        per_component.values().map(Vec::len).sum::<usize>(),
    );

    let name_of = |cid: ComponentId, doc: &fanta_doc::Doc| -> String {
        doc.components
            .def(cid)
            .map(|d| d.name.clone())
            .or_else(|| doc.components.sets.get(&cid).map(|s| s.name.clone()))
            .unwrap_or_else(|| "<unregistered>".into())
    };

    // Pick the target: name filter if given, else the most-instanced component.
    let target = per_component
        .iter()
        .filter(|(cid, _)| {
            name_filter.as_ref().is_none_or(|f| {
                name_of(**cid, &doc)
                    .to_lowercase()
                    .contains(&f.to_lowercase())
            })
        })
        .max_by_key(|(_, v)| v.len())
        .map(|(cid, v)| (*cid, v.len()));

    let Some((cid, count)) = target else {
        eprintln!("no instances match the filter — nothing to probe");
        return Ok(());
    };
    let (inst_id, inst) = per_component[&cid][0].clone();
    let resolved_is_def = doc.components.def(cid).is_some();
    let is_set = doc.components.sets.contains_key(&cid);
    eprintln!(
        "\nTARGET component {:?} ({count} instances) — id maps to: def={resolved_is_def} set={is_set}",
        name_of(cid, &doc)
    );
    eprintln!(
        "  probed instance {inst_id:?}: {} overrides, {} derived entries",
        inst.overrides.len(),
        inst.derived.len(),
    );

    // The master root the instance resolves to, and its rev.
    let rev_before = resolved_component_rev(&doc.components, &inst);
    let master_root = doc.components.def(cid).map(|d| d.root).or_else(|| {
        doc.components
            .sets
            .get(&cid)
            .and_then(|s| doc.components.def(s.default_variant).map(|d| d.root))
    });
    let Some(master_root) = master_root else {
        eprintln!("  component id does not resolve to any registered master — NOT a live link");
        return Ok(());
    };

    // Recolor a SPECIFIC vector under the master, then read back THAT SAME
    // node's clone in the expansion (matched by def-local path) — so we compare
    // like with like instead of guessing which fill is "first".
    let magenta = Color::rgb(255, 0, 255);
    // A "fillable" master node: a Vector (edit its fill) or a Group with a
    // background (edit that — a frame/card body fill, which is what "Card" uses).
    let fillable: Vec<NodeId> = doc
        .scene
        .descendants_of(master_root)
        .filter(|id| match doc.scene.get(*id).map(|n| &n.data) {
            Some(NodeData::Vector(v)) => !v.fills.is_empty(),
            Some(NodeData::Group(g)) => g.background.is_some(),
            _ => false,
        })
        .collect();
    let target_node = fillable.first().copied();
    let Some(target_node) = target_node else {
        eprintln!("  master has no fillable (vector or backgrounded-group) descendant");
        return Ok(());
    };
    let target_path = fanta_doc::def_local_path(&doc.scene, master_root, target_node);
    let had_derived = inst
        .derived
        .iter()
        .any(|d| d.path.as_slice() == target_path.as_slice() && d.fills.is_some());
    let clone_fill = |doc: &fanta_doc::Doc, inst: &InstanceNode| -> Option<Fill> {
        expand_instance(&doc.scene, &doc.components, inst)
            .into_iter()
            .find(|e| e.def_path.as_slice() == target_path.as_slice())
            .and_then(|e| match e.node.data {
                NodeData::Vector(v) => v.fills.first().cloned(),
                NodeData::Group(g) => g.background,
                _ => None,
            })
    };
    eprintln!(
        "  editing master node {target_node:?} (def-path {target_path:?}); \
         instance has a baked derived-fill for it: {had_derived}",
    );
    eprintln!("  --- instance sparse overrides ---");
    for ov in &inst.overrides {
        eprintln!(
            "    path {:?}  value {:?}{}",
            ov.target_path.as_slice(),
            ov.value,
            if ov.target_path.as_slice() == target_path.as_slice() {
                "   <-- TARGETS the edited node"
            } else {
                ""
            }
        );
    }
    eprintln!("  --- instance derived entries ---");
    for d in &inst.derived {
        eprintln!(
            "    path {:?}  fills={} transform={} size={}{}",
            d.path.as_slice(),
            d.fills.is_some(),
            d.transform.is_some(),
            d.size.is_some(),
            if d.path.as_slice() == target_path.as_slice() {
                "   <-- TARGETS the edited node"
            } else {
                ""
            }
        );
    }
    let before = clone_fill(&doc, &inst);
    eprintln!("  that node's clone fill BEFORE: {before:?}  (def.rev={rev_before})");

    let old = doc.scene.get(target_node).unwrap().data.clone();
    let mut new = old.clone();
    match &mut new {
        NodeData::Vector(v) => v.fills = smallvec::smallvec![Fill::solid(magenta)],
        NodeData::Group(g) => g.background = Some(Fill::solid(magenta)),
        _ => {}
    }
    doc.apply(fanta_doc::Operation::ReplaceData {
        id: target_node,
        old: Box::new(old),
        new: Box::new(new),
    })
    .context("recoloring master")?;

    let rev_after = resolved_component_rev(&doc.components, &inst);
    let after = clone_fill(&doc, &inst);
    eprintln!("  that node's clone fill AFTER:  {after:?}  (def.rev={rev_after})");
    eprintln!(
        "\nVERDICT: def rev {} ({} -> {}); instance expansion {} after the master edit.",
        if rev_after > rev_before {
            "BUMPED"
        } else {
            "UNCHANGED"
        },
        rev_before,
        rev_after,
        if before != after {
            "CHANGED (propagates ✓)"
        } else {
            "did NOT change (does not propagate ✗)"
        },
    );
    Ok(())
}
