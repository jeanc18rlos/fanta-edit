use std::{env, fs, path::PathBuf};

use anyhow::{Context as _, Result, anyhow};
use fanta_doc::path::PathData;
use fanta_doc::{
    CanvasNode, Doc, Fill, InstanceNode, NodeData, NodeFlags, NodeId, Stroke, Transform2D,
    expand_instance,
};
use fanta_fig_interop::{fig_to_doc, read_fig};

fn main() -> Result<()> {
    let mut args = env::args().skip(1);
    let fig_path = args
        .next()
        .map(PathBuf::from)
        .ok_or_else(|| anyhow!("missing .fig path"))?;
    let root_filter = args.next().unwrap_or_else(|| "Action Bar".to_string());
    let list_matches = args.any(|arg| arg == "--list");

    let bytes = fs::read(&fig_path).with_context(|| format!("reading {}", fig_path.display()))?;
    let fig = read_fig(&bytes).context("parsing .fig")?;
    let (doc, report, _assets) = fig_to_doc(&fig).context("mapping .fig")?;
    eprintln!(
        "mapped {} nodes, {} components, {} sets, {} instances, {} with props, {} with derived",
        report.mapped,
        report.components,
        report.component_sets,
        report.instances,
        report.instances_with_prop_values,
        report.instances_with_derived,
    );

    if list_matches {
        list_node_paths(&doc, &root_filter);
        return Ok(());
    }

    let root = find_node_by_path(&doc, &root_filter)
        .with_context(|| format!("finding node containing {root_filter:?}"))?;
    println!("root: {}", node_path(&doc, root));
    dump_scene_subtree(&doc, root, 0);
    Ok(())
}

fn find_node_by_path(doc: &Doc, filter: &str) -> Option<NodeId> {
    let filter = filter.to_lowercase();
    doc.scene
        .roots()
        .iter()
        .flat_map(|root| doc.scene.descendants_of(*root))
        .find(|node_id| node_path(doc, *node_id).to_lowercase().contains(&filter))
}

fn list_node_paths(doc: &Doc, filter: &str) {
    let filter = filter.to_lowercase();
    for node_id in doc
        .scene
        .roots()
        .iter()
        .flat_map(|root| doc.scene.descendants_of(*root))
        .filter(|node_id| node_path(doc, *node_id).to_lowercase().contains(&filter))
        .take(200)
    {
        if let Some(node) = doc.scene.get(node_id) {
            println!("{} :: {}", node_kind(node), node_path(doc, node_id));
        }
    }
}

fn dump_scene_subtree(doc: &Doc, root: NodeId, depth: usize) {
    if let Some(node) = doc.scene.get(root) {
        dump_node(doc, node, depth, None);
        if let NodeData::Instance(instance) = &node.data {
            dump_expansion(doc, instance, depth + 1, "expanded");
        }
    }
    for child in doc.scene.children_of(Some(root)) {
        dump_scene_subtree(doc, *child, depth + 1);
    }
}

fn dump_expansion(doc: &Doc, instance: &InstanceNode, depth: usize, label: &str) {
    let expanded = expand_instance(&doc.scene, &doc.components, instance);
    println!("{}{label}: {} nodes", "  ".repeat(depth), expanded.len());
    for node in expanded.iter().take(80) {
        dump_node(doc, &node.node, depth + 1, Some(node.def_path.as_slice()));
        if let NodeData::Instance(inner) = &node.node.data {
            dump_expansion(doc, inner, depth + 2, "nested");
        }
    }
    if expanded.len() > 80 {
        println!("{}... {} more", "  ".repeat(depth + 1), expanded.len() - 80);
    }
}

fn dump_node(doc: &Doc, node: &CanvasNode, depth: usize, def_path: Option<&[NodeId]>) {
    let path = def_path
        .map(|path| format!(" def_path={}", format_node_path_ids(path)))
        .unwrap_or_default();
    let transform = format!(" t={}", transform_text(&node.transform));
    let layout = format_layout(node);
    let name = node.name.as_str();
    let hidden = if node.flags.contains(NodeFlags::HIDDEN) {
        " hidden"
    } else {
        ""
    };
    let mask = if node.is_mask { " mask" } else { "" };
    let world = doc
        .scene
        .world_bounds(node.id)
        .map(|bounds| {
            format!(
                " world=[{:.1},{:.1},{:.1},{:.1}]",
                bounds.min_x, bounds.min_y, bounds.max_x, bounds.max_y
            )
        })
        .unwrap_or_default();
    match &node.data {
        NodeData::Group(group) => {
            let bounds = group
                .clip_size
                .map(|size| format!(" clip={:.1}x{:.1}", size[0], size[1]))
                .unwrap_or_default();
            println!(
                "{}{} [group{}{}{}{}{}{} opacity={:.2} blend={:?} strokes={}{} bg={} radius={:?}{}]",
                "  ".repeat(depth),
                name,
                path,
                transform,
                layout,
                hidden,
                mask,
                world,
                node.opacity,
                node.blend_mode,
                group.strokes.len(),
                stroke_details(&group.strokes),
                group.background.is_some() || !group.background_fills.is_empty(),
                group.corner_radius,
                bounds,
            );
        }
        NodeData::Vector(vector) => {
            let bounds = vector
                .path
                .rough_bounds()
                .map(|bounds| format!(" path_bounds={bounds:?}"))
                .unwrap_or_default();
            println!(
                "{}{} [vector{}{}{}{}{}{} opacity={:.2} blend={:?} fills={} strokes={}{} radius={:?} segments={} moves={}{}]",
                "  ".repeat(depth),
                name,
                path,
                transform,
                layout,
                hidden,
                mask,
                world,
                node.opacity,
                node.blend_mode,
                vector.fills.len(),
                vector.strokes.len(),
                stroke_details(&vector.strokes),
                vector.corner_radius,
                vector.path.segments.len(),
                move_count(&vector.path),
                bounds,
            );
        }
        NodeData::Text(text) => {
            let runs = text
                .style_runs
                .iter()
                .map(|run| format!("{}..{}:{}", run.start, run.end, run.style.color.to_hex()))
                .collect::<Vec<_>>()
                .join(",");
            println!(
                "{}{} [text{}{}{}{}{}{} opacity={:.2} blend={:?} {:?} family={:?} weight={} size={:.1} line_height={:.3} color={} align={:?} auto={:?} valign={:?} runs=[{}] box={:.1}x{:.1}]",
                "  ".repeat(depth),
                name,
                path,
                transform,
                layout,
                hidden,
                mask,
                world,
                node.opacity,
                node.blend_mode,
                text.content,
                text.style.font_family,
                text.style.weight,
                text.style.size_px,
                text.style.line_height,
                text.style.color.to_hex(),
                text.align,
                text.auto_resize,
                text.vertical_align,
                runs,
                text.local_size[0],
                text.local_size[1],
            );
        }
        NodeData::Instance(instance) => {
            let component = doc
                .components
                .def(instance.component)
                .map(|definition| definition.name.as_str())
                .or_else(|| {
                    doc.components
                        .sets
                        .get(&instance.component)
                        .map(|set| set.name.as_str())
                })
                .unwrap_or("<missing>");
            println!(
                "{}{} [instance{}{}{}{}{}{} opacity={:.2} blend={:?} component={component:?} box={:.1}x{:.1} props={} overrides={} derived={}]",
                "  ".repeat(depth),
                name,
                path,
                transform,
                layout,
                hidden,
                mask,
                world,
                node.opacity,
                node.blend_mode,
                instance.local_size[0],
                instance.local_size[1],
                instance.prop_values.len(),
                instance.overrides.len(),
                instance.derived.len(),
            );
            for (prop, value) in &instance.prop_values {
                println!("{}prop {prop}: {value:?}", "  ".repeat(depth + 1));
            }
            for override_entry in &instance.overrides {
                println!(
                    "{}override path={} prop={:?} value={:?}",
                    "  ".repeat(depth + 1),
                    format_node_path_ids(override_entry.target_path.as_slice()),
                    override_entry.target_prop,
                    override_entry.value,
                );
            }
            for derived in &instance.derived {
                let path_bounds = derived
                    .path_data
                    .as_ref()
                    .and_then(PathData::rough_bounds)
                    .map(|bounds| format!(" bounds={bounds:?}"))
                    .unwrap_or_default();
                println!(
                    "{}derived path={} size={:?} transform={} fills={} path={}{} stroke_weight={:?}",
                    "  ".repeat(depth + 1),
                    format_node_path_ids(derived.path.as_slice()),
                    derived.size,
                    derived
                        .transform
                        .as_ref()
                        .map(transform_text)
                        .unwrap_or_else(|| "-".to_string()),
                    derived.fills.as_ref().map_or(0, |fills| fills.len()),
                    derived.path_data.is_some(),
                    path_bounds,
                    derived.stroke_weight,
                );
            }
        }
        other => {
            println!(
                "{}{} [{:?}{}{}{}{}{}]",
                "  ".repeat(depth),
                name,
                other,
                path,
                transform,
                hidden,
                mask,
                world
            );
        }
    }
}

fn format_layout(node: &CanvasNode) -> String {
    let mut out = String::new();
    if let Some(layout_child) = node.layout_child {
        out.push_str(&format!(
            " layout_child={{grow:{:.1},abs:{},align:{:?}}}",
            layout_child.grow, layout_child.absolute, layout_child.align_self
        ));
    }
    if let NodeData::Group(group) = &node.data
        && let Some(auto_layout) = group.auto_layout
    {
        out.push_str(&format!(
            " auto_layout={{mode:{:?},spacing:{:.1},padding:{:?},primary:{:?},counter:{:?},child:{}}}",
            auto_layout.mode,
            auto_layout.spacing,
            auto_layout.padding,
            auto_layout.primary_align,
            auto_layout.counter_align,
            auto_layout.child_layout
        ));
    }
    out
}

fn stroke_details(strokes: &[Stroke]) -> String {
    if strokes.is_empty() {
        return String::new();
    }
    let details = strokes
        .iter()
        .map(|stroke| {
            let paint = match &stroke.paint {
                Fill::Solid { color } => color.to_hex(),
                Fill::Gradient { .. } => "gradient".to_string(),
                Fill::Image { .. } => "image".to_string(),
            };
            format!(
                "{{w={:.2},align={:?},paint={},per_side={:?}}}",
                stroke.width, stroke.align, paint, stroke.per_side
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    format!(" [{details}]")
}

fn node_kind(node: &CanvasNode) -> &'static str {
    match node.data {
        NodeData::Group(_) => "group",
        NodeData::Vector(_) => "vector",
        NodeData::Text(_) => "text",
        NodeData::Instance(_) => "instance",
        NodeData::Bitmap(_) => "bitmap",
        NodeData::Video(_) => "video",
        NodeData::Audio(_) => "audio",
        NodeData::NodeGraph(_) => "node_graph",
        NodeData::Model3d(_) => "model3d",
        NodeData::AiArtifact(_) => "ai_artifact",
        NodeData::Embed(_) => "embed",
    }
}

fn format_node_path_ids(path: &[NodeId]) -> String {
    let ids = path
        .iter()
        .map(|id| id.to_string())
        .collect::<Vec<_>>()
        .join(">");
    format!("{}:[{ids}]", path.len())
}

fn transform_text(transform: &Transform2D) -> String {
    let [a, b, c, d, tx, ty] = transform.to_components();
    format!("[{a:.3},{b:.3},{c:.3},{d:.3},{tx:.3},{ty:.3}]")
}

fn move_count(path: &PathData) -> usize {
    path.segments
        .iter()
        .filter(|segment| matches!(segment, fanta_doc::path::PathSegment::Move { .. }))
        .count()
}

fn node_path(doc: &Doc, node: NodeId) -> String {
    let mut parts = doc
        .scene
        .ancestors_of(node)
        .map(|ancestor| ancestor.name.clone())
        .collect::<Vec<_>>();
    parts.reverse();
    if let Some(node) = doc.scene.get(node) {
        parts.push(node.name.clone());
    }
    parts.join(" / ")
}
