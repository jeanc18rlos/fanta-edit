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

fn use_declarations(source: &str) -> Vec<String> {
    let mut declarations = Vec::new();
    let mut declaration = None::<String>;

    for line in source.lines() {
        let line = line.split("//").next().unwrap_or_default().trim();

        if let Some(current) = declaration.as_mut() {
            current.push(' ');
            current.push_str(line);
            if line.contains(';') {
                declarations.push(
                    declaration
                        .take()
                        .expect("an in-progress use declaration must exist"),
                );
            }
            continue;
        }

        let Some(use_start) = line
            .strip_prefix("use ")
            .map(|_| 0)
            .or_else(|| line.find(" use ").map(|index| index + 1))
        else {
            continue;
        };

        let use_statement = line[use_start..].to_owned();
        if use_statement.contains(';') {
            declarations.push(use_statement);
        } else {
            declaration = Some(use_statement);
        }
    }

    declarations
}

fn has_inherent_impl(source: &str, type_name: &str) -> bool {
    let compact = source.split_whitespace().collect::<String>();
    compact.contains(&format!("impl{type_name}{{"))
        || compact.contains(&format!("impl{type_name}where"))
}

fn function_body<'a>(source: &'a str, marker: &str) -> &'a str {
    let start = source
        .find(marker)
        .unwrap_or_else(|| panic!("expected function marker `{marker}`"));
    let open = source[start..]
        .find('{')
        .map(|offset| start + offset)
        .unwrap_or_else(|| panic!("expected `{marker}` to have a body"));
    let mut depth = 0usize;
    for (offset, character) in source[open..].char_indices() {
        match character {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return &source[open + 1..open + offset];
                }
            }
            _ => {}
        }
    }
    panic!("expected `{marker}` to have a balanced body")
}

fn production_source_before_inline_tests(source: &str) -> &str {
    let mut search_start = 0;

    while let Some(relative_start) = source[search_start..].find("#[cfg(test)]") {
        let attribute_start = search_start + relative_start;
        let after_attribute = attribute_start + "#[cfg(test)]".len();
        if source[after_attribute..]
            .trim_start()
            .starts_with("mod tests")
        {
            return &source[..attribute_start];
        }
        search_start = after_attribute;
    }

    source
}

fn bare_numeric_f32_constant(line: &str) -> Option<&str> {
    let declaration = line.split("//").next().unwrap_or_default().trim();
    let declaration = match declaration.strip_prefix("const ") {
        Some(declaration) => declaration,
        None => {
            let (visibility, declaration) = declaration.split_once("const ")?;
            let visibility = visibility.trim();
            if visibility != "pub" && !(visibility.starts_with("pub(") && visibility.ends_with(')'))
            {
                return None;
            }
            declaration
        }
    };

    let (name, remainder) = declaration.split_once(':')?;
    let (type_name, value) = remainder.split_once('=')?;
    if type_name.trim() != "f32" {
        return None;
    }

    let value = value.trim().strip_suffix(';')?;
    let is_bare_literal = value.starts_with(|character: char| character.is_ascii_digit())
        && value
            .chars()
            .all(|character| character.is_ascii_digit() || character == '.');
    if !is_bare_literal {
        return None;
    }

    Some(name.trim())
}

fn bare_numeric_f32_constants(source: &str) -> Vec<&str> {
    source
        .lines()
        .filter_map(bare_numeric_f32_constant)
        .collect()
}

fn raw_pixel_literals(source: &str) -> usize {
    let mut cursor = 0;
    let mut literals = 0;

    while let Some(relative_start) = source[cursor..].find("px(") {
        cursor += relative_start + "px(".len();
        if source[cursor..].starts_with(|character: char| character.is_ascii_digit()) {
            literals += 1;
        }
    }

    literals
}

fn render_function_boundaries(source: &str) -> Vec<(&str, Option<&str>)> {
    let mut functions = Vec::new();
    let mut line_start = 0usize;

    for line in source.split_inclusive('\n') {
        let Some(function_offset) = line.find("fn render") else {
            line_start += line.len();
            continue;
        };
        let prefix = line[..function_offset].trim();
        if !(prefix.is_empty()
            || prefix.starts_with("pub ")
            || prefix.starts_with("pub(")
            || prefix.starts_with("async "))
        {
            line_start += line.len();
            continue;
        }

        let function_start = line_start + function_offset;
        let remainder = &source[function_start..];
        let body_start = remainder.find('{');
        let declaration_end = remainder.find(';');
        if declaration_end.is_some_and(|end| body_start.is_none_or(|start| end < start)) {
            let end = declaration_end.expect("a declaration terminator must exist") + 1;
            functions.push((&remainder[..end], None));
            line_start += line.len();
            continue;
        }

        let open = body_start.expect("a render function must have a body or declaration");
        let signature = &remainder[..open];
        let mut depth = 0usize;
        let mut body_end = None;
        for (offset, character) in remainder[open..].char_indices() {
            match character {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        body_end = Some(open + offset);
                        break;
                    }
                }
                _ => {}
            }
        }
        let close = body_end.expect("a render function body must balance braces");
        functions.push((signature, Some(&remainder[open + 1..close])));
        line_start += line.len();
    }

    functions
}

fn direct_overlay_slot_assignments(source: &str) -> Vec<(usize, String)> {
    fn is_identifier_byte(byte: u8) -> bool {
        byte.is_ascii_alphanumeric() || byte == b'_'
    }

    let bytes = source.as_bytes();
    let mut cursor = 0;
    let mut assignments = Vec::new();

    while let Some(relative_start) = source[cursor..].find("overlays") {
        let overlays_start = cursor + relative_start;
        let overlays_end = overlays_start + "overlays".len();
        cursor = overlays_end;

        if overlays_start
            .checked_sub(1)
            .and_then(|index| bytes.get(index))
            .is_some_and(|byte| is_identifier_byte(*byte))
            || bytes
                .get(overlays_end)
                .is_some_and(|byte| is_identifier_byte(*byte))
        {
            continue;
        }

        let mut probe = overlays_end;
        while bytes.get(probe).is_some_and(u8::is_ascii_whitespace) {
            probe += 1;
        }
        if bytes.get(probe) != Some(&b'.') {
            continue;
        }
        probe += 1;

        while bytes.get(probe).is_some_and(u8::is_ascii_whitespace) {
            probe += 1;
        }
        let slot_start = probe;
        while bytes
            .get(probe)
            .is_some_and(|byte| is_identifier_byte(*byte))
        {
            probe += 1;
        }
        if probe == slot_start {
            continue;
        }
        let slot = &source[slot_start..probe];

        while bytes.get(probe).is_some_and(u8::is_ascii_whitespace) {
            probe += 1;
        }
        if bytes.get(probe) == Some(&b'=') && bytes.get(probe + 1) != Some(&b'=') {
            let line = source[..overlays_start]
                .bytes()
                .filter(|byte| *byte == b'\n')
                .count()
                + 1;
            assignments.push((line, slot.to_owned()));
        }
    }

    assignments
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
    let storybook_manifest =
        fs::read_to_string(&storybook_manifest_path).expect("storybook manifest must be readable");

    assert!(
        dependency_names(&storybook_manifest).contains(&"fanta-gpui"),
        "{} must consume fanta-gpui through the public facade",
        storybook_manifest_path.display()
    );
}

#[test]
fn inspector_molecules_do_not_import_design_domain_types() {
    let molecules_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("molecules");
    let inspector_module = molecules_root.join("inspector.rs");
    let inspector_root = molecules_root.join("inspector");

    let mut forbidden_imports = Vec::new();
    let mut inspect_source = |path: &Path, source: &str| {
        for declaration in use_declarations(source) {
            let compact = declaration.split_whitespace().collect::<String>();
            let imports_design_domain = compact.contains("Design")
                || compact.contains("::design")
                || compact.contains("{design")
                || compact.contains("crate::prelude")
                || compact.contains("fanta_gpui::prelude");
            if imports_design_domain {
                forbidden_imports.push((path.to_path_buf(), declaration));
            }
        }
        if source.contains("DesignPanel")
            || source.contains("crate::design")
            || source.contains("organisms::design")
            || source.contains("fanta_gpui::design")
        {
            forbidden_imports.push((
                path.to_path_buf(),
                "fully qualified Design-domain reference".to_owned(),
            ));
        }
    };

    // The module is introduced incrementally and may use either Rust module
    // layout. Once either path exists, every source below it is covered.
    if inspector_module.exists() {
        let source = fs::read_to_string(&inspector_module)
            .expect("inspector molecule module must be readable");
        inspect_source(&inspector_module, &source);
    }
    if inspector_root.exists() {
        visit_rust_sources(&inspector_root, &mut inspect_source);
    }

    assert!(
        forbidden_imports.is_empty(),
        "molecules::inspector must remain domain-neutral and may not import the Design facade, \
         Design-prefixed domain types, or the crate prelude; found: {forbidden_imports:#?}"
    );
}

#[test]
fn inspection_context_is_the_only_selected_node_source() {
    let panel_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("organisms")
        .join("design")
        .join("panel");
    let facade_path = panel_root
        .parent()
        .expect("the panel module has a Design parent")
        .join("panel.rs");
    let mut direct_node_cache_reads = Vec::new();

    let mut inspect = |path: &Path, source: &str| {
        let compact = source.split_whitespace().collect::<String>();
        if compact.contains(".host.node") {
            direct_node_cache_reads.push(path.to_path_buf());
        }
    };
    let facade = fs::read_to_string(&facade_path).expect("the Design facade must be readable");
    inspect(&facade_path, &facade);
    visit_rust_sources(&panel_root, &mut inspect);

    assert!(
        direct_node_cache_reads.is_empty(),
        "selected-node reads must resolve through DesignPanelHostState::inspected_node so the inspection context remains authoritative; found: {direct_node_cache_reads:?}",
    );

    let host_state_path = panel_root.join("host_state.rs");
    let host_state = fs::read_to_string(&host_state_path)
        .expect("the grouped Design host state must be readable");
    let compact = host_state.split_whitespace().collect::<String>();
    assert!(
        compact.contains("page_node_fallback:DesignPanelNode")
            && !compact.contains("pub(super)node:DesignPanelNode"),
        "{} may retain a Page-only compatibility fallback, but not a second selected-node cache",
        host_state_path.display()
    );
}

#[test]
fn design_overlay_slots_are_mutated_only_by_the_overlay_coordinator() {
    let design_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("organisms")
        .join("design");
    let panel_root = design_root.join("panel");
    let facade_path = design_root.join("panel.rs");
    let mut direct_assignments = Vec::new();

    let mut inspect = |path: &Path, source: &str| {
        if path
            .file_name()
            .is_some_and(|name| name == "overlay_coordinator.rs" || name == "tests.rs")
        {
            return;
        }

        for (line, slot) in
            direct_overlay_slot_assignments(production_source_before_inline_tests(source))
        {
            direct_assignments.push((path.to_path_buf(), line, slot));
        }
    };

    let facade = fs::read_to_string(&facade_path).expect("the Design facade must be readable");
    inspect(&facade_path, &facade);
    visit_rust_sources(&panel_root, &mut inspect);

    assert!(
        direct_assignments.is_empty(),
        "production Design-panel code must transition overlay state through typed \
         DesignOverlayCoordinator APIs instead of assigning its slots directly; found: \
         {direct_assignments:#?}",
    );
}

#[test]
fn design_storybook_echoes_one_canonical_snapshot() {
    let design_story_root = workspace_root()
        .join("crates")
        .join("fanta-gpui-storybook")
        .join("src")
        .join("screens")
        .join("design");
    let mut compatibility_setter_calls = Vec::new();

    visit_rust_sources(&design_story_root, &mut |path, source| {
        for (index, line) in source.lines().enumerate() {
            let line_without_snapshot = line.replace("panel.set_view_data", "");
            if line_without_snapshot.contains("panel.set_")
                || line_without_snapshot.contains("panel.clear_")
            {
                compatibility_setter_calls.push((path.to_path_buf(), index + 1, line.to_owned()));
            }
        }
    });

    assert!(
        compatibility_setter_calls.is_empty(),
        "the Design specimen must construct Storybook-owned DesignPanelViewData instead of issuing granular component setter batches: {compatibility_setter_calls:#?}",
    );
}

#[test]
fn design_panel_updates_flow_from_snapshot_and_compatibility_wrappers_to_private_apply_helpers() {
    const COMPATIBILITY_UPDATES: &[(&str, &str)] = &[
        ("set_node", "apply_node"),
        ("set_additional_labels", "apply_additional_labels"),
        ("set_nudge_settings", "apply_nudge_settings"),
        ("set_workspace_mode", "apply_workspace_mode"),
        ("set_active_surface", "apply_active_surface"),
        ("set_inspection_context", "apply_inspection_context"),
        ("set_export_view_data", "apply_export_view_data"),
        ("clear_export_view_data", "apply_clear_export_view_data"),
        ("set_color_style_view_data", "apply_color_style_view_data"),
        (
            "set_color_style_sample_view_data",
            "apply_color_style_sample_view_data",
        ),
        (
            "set_color_contrast_view_data",
            "apply_color_contrast_view_data",
        ),
        (
            "set_paint_variable_view_data",
            "apply_paint_variable_view_data",
        ),
        ("set_paint_style_view_data", "apply_paint_style_view_data"),
        ("set_media_paint_view_data", "apply_media_paint_view_data"),
        ("set_shader_view_data", "apply_shader_view_data"),
        (
            "set_typography_style_view_data",
            "apply_typography_style_view_data",
        ),
        ("set_font_view_data", "apply_font_view_data"),
        ("set_effect_style_view_data", "apply_effect_style_view_data"),
        (
            "set_effect_variable_view_data",
            "apply_effect_variable_view_data",
        ),
        (
            "set_property_variable_view_data",
            "apply_property_variable_view_data",
        ),
        (
            "set_component_swap_view_data",
            "apply_component_swap_view_data",
        ),
        (
            "set_layout_grid_style_view_data",
            "apply_layout_grid_style_view_data",
        ),
        (
            "set_layout_grid_variable_view_data",
            "apply_layout_grid_variable_view_data",
        ),
        (
            "set_layout_grid_count_variable_view_data",
            "apply_layout_grid_count_variable_view_data",
        ),
        (
            "set_add_auto_layout_view_data",
            "apply_add_auto_layout_view_data",
        ),
        (
            "clear_add_auto_layout_view_data",
            "apply_clear_add_auto_layout_view_data",
        ),
        (
            "set_draw_appearance_view_data",
            "apply_draw_appearance_view_data",
        ),
        (
            "clear_draw_appearance_view_data",
            "apply_clear_draw_appearance_view_data",
        ),
        ("set_frame_preset_view_data", "apply_frame_preset_view_data"),
        (
            "clear_frame_preset_view_data",
            "apply_clear_frame_preset_view_data",
        ),
        (
            "set_smart_selection_view_data",
            "apply_smart_selection_view_data",
        ),
        (
            "clear_smart_selection_view_data",
            "apply_clear_smart_selection_view_data",
        ),
        ("set_page_view_data", "apply_page_view_data"),
        ("clear_page_view_data", "apply_clear_page_view_data"),
        (
            "set_page_local_styles_view_data",
            "apply_page_local_styles_view_data",
        ),
        (
            "clear_page_local_styles_view_data",
            "apply_clear_page_local_styles_view_data",
        ),
        ("set_variables_entry_point", "apply_variables_entry_point"),
        (
            "set_variable_mode_view_data",
            "apply_variable_mode_view_data",
        ),
        (
            "clear_variable_mode_view_data",
            "apply_clear_variable_mode_view_data",
        ),
        (
            "set_viewer_properties_view_data",
            "apply_viewer_properties_view_data",
        ),
        (
            "clear_viewer_properties_view_data",
            "apply_clear_viewer_properties_view_data",
        ),
        (
            "set_selection_header_view_data",
            "apply_selection_header_view_data",
        ),
        (
            "set_selection_header_view_data_for_target",
            "apply_selection_header_view_data_for_target",
        ),
        (
            "clear_selection_header_view_data",
            "apply_clear_selection_header_view_data",
        ),
        ("set_property_value_states", "apply_property_value_states"),
        ("set_property_value_state", "apply_property_value_state"),
    ];

    let facade_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("organisms")
        .join("design")
        .join("panel.rs");
    let source = fs::read_to_string(&facade_path).expect("the Design facade must be readable");
    let grouped_body = function_body(&source, "pub fn set_view_data");

    assert!(
        !grouped_body.contains("self.set_") && !grouped_body.contains("self.clear_"),
        "DesignPanel::set_view_data must invoke private canonical apply helpers, not fan out through public compatibility setters",
    );

    for (public_name, apply_name) in COMPATIBILITY_UPDATES {
        let body = function_body(&source, &format!("pub fn {public_name}("));
        assert!(
            body.contains(&format!("self.{apply_name}(")),
            "DesignPanel::{public_name} must remain a thin compatibility wrapper around {apply_name}",
        );
    }
}

#[test]
fn newly_extracted_design_sections_do_not_extend_the_panel_facade() {
    const LEGACY_INHERENT_IMPL_MODULES: &[&str] = &[];

    let panel_module_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("organisms")
        .join("design")
        .join("panel");
    let mut new_inherent_impls = Vec::new();
    let mut active_legacy_exemptions = Vec::new();

    visit_rust_sources(&panel_module_root, &mut |path, source| {
        let relative_path = path
            .strip_prefix(&panel_module_root)
            .expect("visited Design-panel source must be below its module root");
        let is_legacy_module = relative_path
            .to_str()
            .is_some_and(|path| LEGACY_INHERENT_IMPL_MODULES.contains(&path));

        let extends_design_panel = has_inherent_impl(source, "DesignPanel");
        if is_legacy_module && extends_design_panel {
            active_legacy_exemptions.push(relative_path.to_string_lossy().into_owned());
        } else if extends_design_panel {
            new_inherent_impls.push(relative_path.to_path_buf());
        }
    });

    let stale_legacy_exemptions = LEGACY_INHERENT_IMPL_MODULES
        .iter()
        .filter(|path| {
            !active_legacy_exemptions
                .iter()
                .any(|active| active == **path)
        })
        .collect::<Vec<_>>();

    assert!(
        new_inherent_impls.is_empty(),
        "new Design section modules must be standalone projections/controllers rather than \
         inherent `impl DesignPanel` extensions; only the explicit legacy migration boundary is \
         exempt: {new_inherent_impls:?}"
    );
    assert!(
        stale_legacy_exemptions.is_empty(),
        "remove migrated files from LEGACY_INHERENT_IMPL_MODULES instead of retaining stale \
         architecture exceptions: {stale_legacy_exemptions:?}"
    );
}

#[test]
fn extracted_section_renderers_use_typed_sinks_instead_of_the_full_facade() {
    let sections_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("organisms")
        .join("design")
        .join("panel")
        .join("sections");
    let mut raw_facade_arguments = Vec::new();
    let mut facade_updates = Vec::new();

    visit_rust_sources(&sections_root, &mut |path, source| {
        let production = production_source_before_inline_tests(source);
        for (signature, body) in render_function_boundaries(production) {
            let signature = signature.split_whitespace().collect::<String>();
            if signature.contains("Entity<DesignPanel>")
                || signature.contains("&Entity<DesignPanel>")
            {
                raw_facade_arguments.push((path.to_path_buf(), signature.clone()));
            }
            let Some(body) = body else {
                continue;
            };
            let compact_body = body.split_whitespace().collect::<String>();
            if compact_body.contains(".update(cx,|panel,")
                || compact_body.contains(".update(cx,|this,")
            {
                facade_updates.push((path.to_path_buf(), signature));
            }
        }
    });

    assert!(
        raw_facade_arguments.is_empty(),
        "extracted section renderers must receive projections, narrow chrome, and typed event \
         sinks rather than raw DesignPanel entities; found: {raw_facade_arguments:#?}",
    );
    assert!(
        facade_updates.is_empty(),
        "extracted rendering helpers must emit through typed sinks instead of updating the \
         DesignPanel facade directly; found: {facade_updates:#?}",
    );
}

#[test]
fn migrated_effects_typography_and_options_use_explicit_controller_traits() {
    let panel_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("organisms")
        .join("design")
        .join("panel");

    for (file, controller) in [
        ("effects.rs", "DesignEffectsController"),
        ("typography.rs", "DesignTypographyController"),
        ("options.rs", "DesignOptionsController"),
    ] {
        let path = panel_root.join(file);
        let source = fs::read_to_string(&path).expect("controller source must be readable");
        assert!(
            source.contains(&format!("trait {controller}: Sized"))
                && source.contains(&format!("impl {controller} for DesignPanel")),
            "{} must expose and implement the explicit `{controller}` extension boundary",
            path.display(),
        );
        assert!(
            !has_inherent_impl(&source, "DesignPanel"),
            "{} must not extend the facade through an inherent implementation",
            path.display(),
        );
    }

    let facade_path = panel_root
        .parent()
        .expect("the panel module has a Design parent")
        .join("panel.rs");
    let facade = fs::read_to_string(&facade_path).expect("the Design facade must be readable");
    for controller in [
        "DesignEffectsControllerExt",
        "DesignTypographyControllerExt",
        "DesignOptionsControllerExt",
    ] {
        assert!(
            facade.contains(controller),
            "{} must import `{controller}` so sibling controller calls resolve",
            facade_path.display(),
        );
    }
}

#[test]
fn common_design_inspector_fields_use_owned_projections_and_a_live_event_sink() {
    let panel_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("organisms")
        .join("design")
        .join("panel");
    let fields_path = panel_root.join("inspector_fields.rs");
    let fields = fs::read_to_string(&fields_path)
        .expect("the standalone Design inspector field controller must be readable");

    for projection in [
        "struct DesignStandardValueCellProjection",
        "struct DesignIconValueCellProjection",
        "struct DesignToggleRowProjection",
        "struct DesignCheckboxRowProjection",
    ] {
        assert!(
            fields.contains(projection),
            "{} must retain the owned `{projection}` boundary",
            fields_path.display()
        );
    }
    assert!(
        fields.contains("struct DesignInspectorEventSink")
            && fields.contains("panel: Entity<DesignPanel>")
            && fields.contains("enum DesignInspectorFieldEvent")
            && fields.contains("fn dispatch("),
        "{} must route field interactions through an explicit live entity sink",
        fields_path.display()
    );
    assert!(
        !has_inherent_impl(&fields, "DesignPanel"),
        "{} may implement the compatibility field-renderer trait but must not add another inherent DesignPanel extension",
        fields_path.display()
    );

    let options_path = panel_root.join("options.rs");
    let options = fs::read_to_string(&options_path)
        .expect("the retained option-state controller must be readable");
    for migrated_method in [
        "pub(super) fn render_value_cell(",
        "pub(super) fn render_value_cell_with_left_padding(",
        "pub(super) fn render_icon_value_cell(",
        "pub(super) fn render_toggle_row(",
        "pub(super) fn render_checkbox_row(",
        "pub(super) fn render_group_label(",
    ] {
        assert!(
            !options.contains(migrated_method),
            "{} must not reclaim migrated common field chrome `{migrated_method}`",
            options_path.display()
        );
    }
}

#[test]
fn design_production_consumes_the_shared_inspector_taxonomy() {
    let panel_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("organisms")
        .join("design")
        .join("panel");
    let facade = fs::read_to_string(
        panel_root
            .parent()
            .expect("the panel module has a Design parent")
            .join("panel.rs"),
    )
    .expect("the Design facade must be readable");
    let fields = fs::read_to_string(panel_root.join("inspector_fields.rs"))
        .expect("the common field controller must be readable");
    let options = fs::read_to_string(panel_root.join("options.rs"))
        .expect("the option controller must be readable");
    let edit_controller = fs::read_to_string(panel_root.join("edit_controller.rs"))
        .expect("the edit controller must be readable");
    let appearance = fs::read_to_string(panel_root.join("sections/appearance.rs"))
        .expect("the Appearance section must be readable");
    let effects = fs::read_to_string(panel_root.join("sections/effects.rs"))
        .expect("the Effects section must be readable");
    let type_settings = fs::read_to_string(panel_root.join("sections/typography/type_settings.rs"))
        .expect("the type-settings controller must be readable");
    let typography = fs::read_to_string(panel_root.join("typography.rs"))
        .expect("the retained typography controller must be readable");

    for field in ["InspectorNumberField", "InspectorTextField"] {
        assert!(
            facade.contains(field),
            "the real Design property editor must retain shared `{field}` state"
        );
    }
    assert!(
        options.contains("InspectorPickerField"),
        "real Design option controls must gate choices through InspectorPickerField"
    );
    assert!(
        edit_controller.contains("InspectorSliderField")
            && appearance.contains("InspectorSliderField"),
        "real Design sliders must share one controlled slider transaction"
    );
    for field in ["InspectorToggleField", "InspectorCheckboxField"] {
        assert!(
            fields.contains(field),
            "common real Design boolean rows must use `{field}`"
        );
    }
    assert!(
        fields.contains("InspectorGridLayout::compact(InspectorMetrics::current())")
            && fields.contains("inspector_row_with_layout"),
        "common Design field chrome must consume canonical inspector geometry"
    );
    for recipe in ["inspector_segmented_control", "inspector_segment("] {
        assert!(
            type_settings.contains(recipe),
            "type settings must use the shared `{recipe}` recipe"
        );
    }
    for recipe in [
        "inspector_field_grid_with_layout",
        "inspector_row_with_layout",
        "inspector_field_label",
        "inspector_action_group",
        "inspector_popover_surface",
    ] {
        assert!(
            typography.contains(recipe),
            "type settings must consume shared inspector `{recipe}` production chrome"
        );
    }
    for recipe in [
        "inspector_collection_row",
        "inspector_action_group",
        "InspectorColorSwatch",
    ] {
        assert!(
            effects.contains(recipe),
            "Effects must consume shared inspector `{recipe}` production chrome"
        );
    }
}

#[test]
fn design_overlay_dismissal_converges_through_the_coordinator() {
    let panel_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("organisms")
        .join("design")
        .join("panel.rs");
    let panel = fs::read_to_string(&panel_path).expect("the Design facade must be readable");
    let compact = panel.split_whitespace().collect::<String>();

    assert!(
        panel.contains("fn dismiss_overlay(")
            && panel.contains("fn dismiss_topmost_overlay(")
            && panel.contains("fn dismiss_overlay_from_outside_click("),
        "{} must keep Escape and outside-click cleanup on one facade path",
        panel_path.display()
    );
    assert!(
        compact.contains("self.overlays.escape_dismissal_intent()")
            && compact.contains("self.overlays.outside_click_dismissal_intent(overlay)"),
        "{} must delegate ordering and covered-ancestor policy to DesignOverlayCoordinator",
        panel_path.display()
    );
    assert!(
        !compact.contains("matchthis.overlays.topmost()"),
        "{} must not duplicate the coordinator's priority table in the Render implementation",
        panel_path.display()
    );
}

#[test]
fn component_authoring_dialog_visibility_belongs_to_the_overlay_coordinator() {
    let panel_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("organisms")
        .join("design")
        .join("panel");
    let overlay_source = fs::read_to_string(panel_root.join("overlay_coordinator.rs"))
        .expect("the Design overlay coordinator must be readable");
    let component_state = fs::read_to_string(panel_root.join("component_props/state.rs"))
        .expect("component authoring state must be readable");

    assert!(
        overlay_source.contains("component_authoring_dialog_open"),
        "native component dialogs must participate in the same overlay priority/dismissal state"
    );
    assert!(
        !component_state.contains("dialog_open"),
        "feature-local component state may retain dialog drafts, but not a second visibility source"
    );
}

#[test]
fn extracted_appearance_uses_grouped_projection_and_live_event_sink() {
    let source_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("organisms")
        .join("design")
        .join("panel")
        .join("sections")
        .join("appearance.rs");
    let source = fs::read_to_string(&source_path)
        .expect("the extracted Appearance section must be readable");
    let compact = source.split_whitespace().collect::<String>();

    for projection in [
        "struct AppearanceProjection",
        "struct AppearanceCapabilities",
        "struct AppearanceValues",
        "struct AppearancePresentation",
    ] {
        assert!(
            source.contains(projection),
            "{} must keep Appearance data in the grouped `{projection}` boundary",
            source_path.display()
        );
    }
    assert!(
        source.contains("enum AppearanceEvent") && source.contains("fn dispatch("),
        "{} must route retained callbacks through a live event controller",
        source_path.display()
    );
    assert!(
        source.contains("live_target_matches") && source.contains("live_property_is_editable"),
        "{} must revalidate captured targets and access before emitting edits",
        source_path.display()
    );
    assert!(
        compact.contains("fnrender(projection:&AppearanceProjection,"),
        "{} must render Layer from AppearanceProjection rather than the complete facade",
        source_path.display()
    );
    assert!(
        compact.contains("fnrender_mask(projection:&AppearanceProjection,"),
        "{} must render Mask from AppearanceProjection rather than the complete facade",
        source_path.display()
    );
    assert!(
        !source.contains("impl DesignPanel"),
        "{} must not add an inherent DesignPanel implementation",
        source_path.display()
    );
}

#[test]
fn extracted_shape_sections_use_a_projection_and_explicit_event_sink() {
    let source_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("organisms")
        .join("design")
        .join("panel")
        .join("sections")
        .join("shape.rs");
    let source =
        fs::read_to_string(&source_path).expect("the extracted shape section must be readable");

    assert!(
        source.contains("struct ShapeProjection"),
        "{} must project immutable shape data before rendering",
        source_path.display()
    );
    assert!(
        source.contains("Entity<DesignPanel>"),
        "{} must route direct actions through an explicit retained event sink",
        source_path.display()
    );
    for forbidden in ["panel: &DesignPanel", "panel.host", ".host.node"] {
        assert!(
            !source.contains(forbidden),
            "{} must not bypass its projection through `{forbidden}`",
            source_path.display()
        );
    }
}

#[test]
fn extracted_header_uses_a_projection_and_live_event_sink() {
    let source_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("organisms")
        .join("design")
        .join("panel")
        .join("sections")
        .join("header.rs");
    let source =
        fs::read_to_string(&source_path).expect("the extracted header section must be readable");
    let compact = source.split_whitespace().collect::<String>();

    assert!(
        source.contains("struct HeaderProjection"),
        "{} must project immutable header data before rendering",
        source_path.display()
    );
    assert!(
        source.contains("Entity<DesignPanel>"),
        "{} must route header interactions through the retained event sink",
        source_path.display()
    );
    assert!(
        source.contains("fn dispatch("),
        "{} must validate events against the live panel snapshot",
        source_path.display()
    );
    assert!(
        compact.contains("fnrender(projection:&HeaderProjection,"),
        "{} must render from HeaderProjection rather than the complete facade",
        source_path.display()
    );
    for forbidden in ["impl DesignPanel", "panel.host", ".host."] {
        assert!(
            !source.contains(forbidden),
            "{} must not bypass its projection through `{forbidden}`",
            source_path.display()
        );
    }
}

#[test]
fn extracted_page_section_uses_a_projection_and_explicit_event_sink() {
    let source_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("organisms")
        .join("design")
        .join("panel")
        .join("sections")
        .join("page.rs");
    let source =
        fs::read_to_string(&source_path).expect("the extracted Page section must be readable");
    let compact = source.split_whitespace().collect::<String>();
    let controller_path = source_path
        .parent()
        .expect("Page section has a module directory")
        .join("page")
        .join("controller.rs");
    let controller = fs::read_to_string(&controller_path)
        .expect("the extracted Page controller must be readable");

    assert!(
        source.contains("struct PageProjection"),
        "{} must project immutable Page data before rendering",
        source_path.display()
    );
    assert!(
        source.contains("struct PageEventSink")
            && source.contains("enum PageEvent")
            && source.contains("fn dispatch("),
        "{} must route direct interactions through its typed live Page event sink",
        source_path.display()
    );
    assert!(
        source.contains("struct PageChrome") && !source.contains("trait PageInspectorChrome"),
        "{} must receive narrow rendered chrome instead of a trait implemented by the full facade",
        source_path.display()
    );
    assert!(
        compact.contains(
            "fnrender(projection:&PageProjection,mutchrome:PageChrome<'_>,events:PageEventSink,"
        ),
        "{} must render from PageProjection, PageChrome, and PageEventSink rather than the full facade",
        source_path.display()
    );
    assert!(
        controller.contains("trait PagePanelController")
            && controller.contains("impl PagePanelController for DesignPanel")
            && controller.contains("fn page_projection(&self) -> PageProjection"),
        "{} must own Page projection, live validation, and facade adaptation",
        controller_path.display()
    );
    for forbidden in [
        "panel: &DesignPanel",
        "panel.host",
        ".host.",
        "&impl PageInspectorChrome",
    ] {
        assert!(
            !source.contains(forbidden),
            "{} must not bypass its projection through `{forbidden}`",
            source_path.display()
        );
    }
}

#[test]
fn design_panel_render_delegates_to_the_extracted_shell() {
    let panel_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("organisms")
        .join("design")
        .join("panel");
    let facade_path = panel_root
        .parent()
        .expect("the panel module has a Design parent")
        .join("panel.rs");
    let facade = fs::read_to_string(&facade_path).expect("the Design facade must be readable");
    let shell_path = panel_root.join("shell.rs");
    let shell = fs::read_to_string(&shell_path).expect("the Design shell must be readable");
    let render_body = function_body(&facade, "impl Render for DesignPanel");

    assert!(
        render_body.contains("self.render_shell(window, cx)"),
        "{} must delegate top-level assembly to DesignPanelShellController",
        facade_path.display()
    );
    assert!(
        shell.contains("trait DesignPanelShellController")
            && shell.contains("impl DesignPanelShellController for DesignPanel")
            && shell.contains("fn render_section_header(")
            && shell.contains("fn render_host_owned_surface("),
        "{} must own section chrome, dispatch, and host-owned surface rendering",
        shell_path.display()
    );
    for migrated_method in [
        "fn header_projection(",
        "fn page_projection(",
        "fn render_section_header(",
        "fn render_host_owned_surface(",
        "fn renders_inspector_projection(",
    ] {
        assert!(
            !facade.contains(migrated_method),
            "{} must not regain migrated Page/shell method `{migrated_method}`",
            facade_path.display()
        );
    }
}

#[test]
fn extracted_position_renderer_uses_an_explicit_projection_and_event_sink() {
    let source_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("organisms")
        .join("design")
        .join("panel")
        .join("sections")
        .join("position.rs");
    let source =
        fs::read_to_string(&source_path).expect("the extracted Position section must be readable");
    let compact = source.split_whitespace().collect::<String>();

    assert!(
        source.contains("struct PositionProjection"),
        "{} must project immutable Position data before rendering",
        source_path.display()
    );
    assert!(
        source.contains("Entity<DesignPanel>"),
        "{} must route interactions through an explicit retained event sink",
        source_path.display()
    );
    assert!(
        compact.contains("fnrender(projection:&PositionProjection,"),
        "{} must render from PositionProjection rather than the complete facade",
        source_path.display()
    );
    assert!(
        !compact.contains("PositionProjection::from_panel")
            && !compact.contains("fnfrom_panel(panel:&DesignPanel)"),
        "{} must construct its snapshot at the facade boundary",
        source_path.display()
    );
}

#[test]
fn extracted_layout_renderer_uses_grouped_projection_and_live_event_sink() {
    let source_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("organisms")
        .join("design")
        .join("panel")
        .join("sections")
        .join("layout.rs");
    let source =
        fs::read_to_string(&source_path).expect("the extracted Layout section must be readable");
    let compact = source.split_whitespace().collect::<String>();

    assert!(
        source.contains("struct LayoutProjection"),
        "{} must project immutable Layout data before rendering",
        source_path.display()
    );
    assert!(
        source.contains("struct LayoutAutoLayoutProjection")
            && source.contains("struct LayoutGridProjection")
            && source.contains("struct LayoutDimensionProjection")
            && source.contains("struct LayoutPresetProjection"),
        "{} must keep complex Layout concerns in semantic subprojections",
        source_path.display()
    );
    assert!(
        source.contains("Entity<DesignPanel>") && source.contains("fn dispatch("),
        "{} must route direct interactions through a live retained event sink",
        source_path.display()
    );
    assert!(
        compact.contains("fnrender(projection:&LayoutProjection,"),
        "{} must render from LayoutProjection rather than the complete facade",
        source_path.display()
    );
    assert!(
        !compact.contains("LayoutProjection::from_panel")
            && !compact.contains("fnfrom_panel(panel:&DesignPanel)"),
        "{} must construct its snapshot at the facade boundary",
        source_path.display()
    );
    for forbidden in ["panel.host", ".host.node", "impl DesignPanel"] {
        assert!(
            !source.contains(forbidden),
            "{} must not bypass its projection through `{forbidden}`",
            source_path.display()
        );
    }
}

#[test]
fn extracted_export_renderer_uses_semantic_projections_and_live_event_sink() {
    let source_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("organisms")
        .join("design")
        .join("panel")
        .join("sections")
        .join("export.rs");
    let source =
        fs::read_to_string(&source_path).expect("the extracted Export section must be readable");
    let compact = source.split_whitespace().collect::<String>();

    for projection in [
        "struct ExportProjection",
        "struct ExportTargetProjection",
        "struct ExportStaticProjection",
        "struct ExportStaticRowProjection",
        "struct ExportAnimatedProjection",
        "struct ExportPreviewProjection",
        "struct ExportPresentationProjection",
    ] {
        assert!(
            source.contains(projection),
            "{} must keep Export data in the semantic `{projection}` boundary",
            source_path.display()
        );
    }
    assert!(
        source.contains("Entity<DesignPanel>") && source.contains("fn dispatch("),
        "{} must route Export interactions through a live retained event sink",
        source_path.display()
    );
    assert!(
        compact.contains("fnrender(projection:&ExportProjection,"),
        "{} must render Export from ExportProjection rather than the complete facade",
        source_path.display()
    );
    assert!(
        !source.contains("impl DesignPanel"),
        "{} must not add an inherent DesignPanel implementation",
        source_path.display()
    );
}

#[test]
fn extracted_effects_use_semantic_projections_and_live_event_sink() {
    let sections_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("organisms")
        .join("design")
        .join("panel")
        .join("sections")
        .join("effects.rs");
    let sections =
        fs::read_to_string(&sections_path).expect("the extracted Effects section must be readable");
    let compact = sections.split_whitespace().collect::<String>();

    for projection in [
        "struct EffectsProjection",
        "struct EffectsIdentityProjection",
        "struct EffectsAccessProjection",
        "struct EffectsContentProjection",
        "struct EffectsOverlayProjection",
        "struct EffectTargetProjection",
    ] {
        assert!(
            sections.contains(projection),
            "{} must keep Effects data in the semantic `{projection}` boundary",
            sections_path.display()
        );
    }
    assert!(
        sections.contains("Entity<DesignPanel>") && sections.contains("fn dispatch("),
        "{} must route Effects interactions through a live retained event sink",
        sections_path.display()
    );
    assert!(
        compact.contains("fnrender(projection:&EffectsProjection,"),
        "{} must render Effects from EffectsProjection rather than the complete facade",
        sections_path.display()
    );
    assert!(
        !sections.contains("impl DesignPanel"),
        "{} must not add an inherent DesignPanel implementation",
        sections_path.display()
    );

    let facade_path = sections_path
        .parent()
        .expect("sections has a panel parent")
        .parent()
        .expect("panel is the sections parent")
        .join("effects.rs");
    let facade =
        fs::read_to_string(&facade_path).expect("the Effects facade seam must be readable");
    assert!(
        facade.contains("sections::effects::EffectsEventSink::new(cx.entity())")
            && facade.contains("sections::effects::render(&projection(self), self, &events, cx)"),
        "{} must delegate its top-level renderer to the extracted section",
        facade_path.display()
    );
}

#[test]
fn extracted_layout_guides_use_semantic_projections_and_explicit_event_sink() {
    let sections_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("organisms")
        .join("design")
        .join("panel")
        .join("sections")
        .join("layout_guides.rs");
    let sections = fs::read_to_string(&sections_path)
        .expect("the extracted Layout Guides section must be readable");
    let compact = sections.split_whitespace().collect::<String>();

    for projection in [
        "struct LayoutGuidesProjection",
        "struct LayoutGuidesIdentityProjection",
        "struct LayoutGuidesAccessProjection",
        "struct LayoutGuidesContentProjection",
        "struct LayoutGuideTargetProjection",
    ] {
        assert!(
            sections.contains(projection),
            "{} must keep Layout Guides data in the semantic `{projection}` boundary",
            sections_path.display()
        );
    }
    assert!(
        sections.contains("trait LayoutGuidesInspectorChrome")
            && !sections.contains("Entity<DesignPanel>")
            && compact.contains("fnrender(projection:&LayoutGuidesProjection,"),
        "{} must render from a narrow projection and explicit retained chrome boundary",
        sections_path.display()
    );
    assert!(
        !sections.contains("impl DesignPanel"),
        "{} must not add an inherent DesignPanel implementation",
        sections_path.display()
    );

    let facade_path = sections_path
        .parent()
        .expect("sections has a panel parent")
        .parent()
        .expect("panel is the sections parent")
        .join("layout_grids.rs");
    let facade =
        fs::read_to_string(&facade_path).expect("the Layout Guides facade seam must be readable");
    assert!(
        facade.contains("sections::layout_guides::render(&projection(self), self, cx)"),
        "{} must delegate its top-level renderer to the extracted section",
        facade_path.display()
    );
}

#[test]
fn extracted_paint_sections_use_semantic_projections_and_live_event_sink() {
    let sections_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("organisms")
        .join("design")
        .join("panel")
        .join("sections")
        .join("paints.rs");
    let sections =
        fs::read_to_string(&sections_path).expect("the extracted Paint sections must be readable");
    let compact = sections.split_whitespace().collect::<String>();

    for projection in [
        "struct PaintSectionIdentity",
        "struct PaintCollectionProjection",
        "struct FillProjection",
        "struct StrokeProjection",
        "struct SelectionColorsProjection",
    ] {
        assert!(
            sections.contains(projection),
            "{} must keep Paint data in the semantic `{projection}` boundary",
            sections_path.display()
        );
    }
    assert!(
        sections.contains("Entity<DesignPanel>") && sections.contains("fn dispatch_direct_action("),
        "{} must route direct Stroke actions through a live retained event sink",
        sections_path.display()
    );
    for renderer in ["render_fill", "render_stroke", "render_selection_colors"] {
        assert!(
            compact.contains(&format!("fn{renderer}(")),
            "{} must own the `{renderer}` renderer",
            sections_path.display()
        );
    }
    assert!(
        !sections.contains("impl DesignPanel"),
        "{} must not add an inherent DesignPanel implementation",
        sections_path.display()
    );

    let facade_path = sections_path
        .parent()
        .expect("sections has a panel parent")
        .parent()
        .expect("panel is the sections parent")
        .join("paints.rs");
    let facade = fs::read_to_string(&facade_path).expect("the Paint facade seam must be readable");
    for delegation in [
        "sections::paints::render_fill",
        "sections::paints::render_stroke",
        "sections::paints::render_selection_colors",
    ] {
        assert!(
            facade.contains(delegation),
            "{} must delegate through `{delegation}`",
            facade_path.display()
        );
    }
}

#[test]
fn extracted_component_section_uses_semantic_projections_and_live_event_sink() {
    let sections_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("organisms")
        .join("design")
        .join("panel")
        .join("sections")
        .join("component.rs");
    let sections = fs::read_to_string(&sections_path)
        .expect("the extracted Component section must be readable");
    let compact = sections.split_whitespace().collect::<String>();

    for projection in [
        "struct ComponentProjection",
        "struct ComponentIdentityProjection",
        "struct ComponentRoleProjection",
        "struct ComponentContextProjection",
        "struct ComponentPropertiesProjection",
        "struct ComponentAuthoringProjection",
        "struct ComponentPresentationProjection",
    ] {
        assert!(
            sections.contains(projection),
            "{} must keep Component data in the semantic `{projection}` boundary",
            sections_path.display()
        );
    }
    assert!(
        sections.contains("Entity<DesignPanel>")
            && sections.contains("fn dispatch_direct_action(")
            && sections.contains("component_action_is_enabled")
            && sections.contains("DesignPanelTarget"),
        "{} must route direct actions through a live sink that revalidates access and exact targets",
        sections_path.display()
    );
    assert!(
        compact.contains("fnrender_component(")
            && compact.contains("projection:&ComponentProjection,"),
        "{} must render Component/Instance from ComponentProjection",
        sections_path.display()
    );
    assert!(
        !sections.contains("impl DesignPanel"),
        "{} must not add an inherent DesignPanel implementation",
        sections_path.display()
    );

    let facade_path = sections_path
        .parent()
        .expect("sections has a panel parent")
        .parent()
        .expect("panel is the sections parent")
        .join("component_props.rs");
    let facade =
        fs::read_to_string(&facade_path).expect("the Component facade seam must be readable");
    assert!(
        facade.contains("sections::component::render_component("),
        "{} must delegate its top-level renderer to the extracted section",
        facade_path.display()
    );
}

#[test]
fn extracted_typography_uses_semantic_projections_and_live_event_sink() {
    let sections_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("organisms")
        .join("design")
        .join("panel")
        .join("sections")
        .join("typography")
        .join("mod.rs");
    let sections = fs::read_to_string(&sections_path)
        .expect("the extracted Typography section must be readable");
    let compact = sections.split_whitespace().collect::<String>();

    for projection in [
        "struct TypographyProjection",
        "struct TypographyIdentityProjection",
        "struct TypographyValuesProjection",
        "struct TypographyAccessProjection",
        "struct TypographyResourceProjection",
        "struct TypographyPresentationProjection",
    ] {
        assert!(
            sections.contains(projection),
            "{} must keep Typography data in the semantic `{projection}` boundary",
            sections_path.display()
        );
    }
    assert!(
        sections.contains("Entity<DesignPanel>") && sections.contains("fn dispatch("),
        "{} must route Typography interactions through a live retained event sink",
        sections_path.display()
    );
    assert!(
        compact.contains("fnrender(projection:&TypographyProjection,"),
        "{} must render Typography from TypographyProjection rather than the complete facade",
        sections_path.display()
    );
    assert!(
        !compact.contains("TypographyProjection::from_panel")
            && !compact.contains("fnfrom_panel(panel:&DesignPanel)")
            && !sections.contains("impl DesignPanel"),
        "{} must keep projection construction at the facade boundary and avoid inherent facade extensions",
        sections_path.display()
    );

    let facade_path = sections_path
        .parent()
        .expect("Typography has a section directory")
        .parent()
        .expect("sections is the Typography parent")
        .parent()
        .expect("panel is the sections parent")
        .join("typography.rs");
    let facade =
        fs::read_to_string(&facade_path).expect("the Typography facade seam must be readable");
    assert!(
        facade.contains("sections::typography::render("),
        "{} must delegate its top-level renderer to the extracted section",
        facade_path.display()
    );
}

#[test]
fn extracted_viewer_renderer_uses_an_explicit_projection_and_event_sink() {
    let source_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("organisms")
        .join("design")
        .join("panel")
        .join("sections")
        .join("viewer.rs");
    let source =
        fs::read_to_string(&source_path).expect("the extracted Viewer section must be readable");
    let compact = source.split_whitespace().collect::<String>();

    assert!(
        source.contains("struct ViewerProjection"),
        "{} must project immutable Viewer data before rendering",
        source_path.display()
    );
    assert!(
        source.contains("Entity<DesignPanel>"),
        "{} must route interactions through an explicit retained event sink",
        source_path.display()
    );
    assert!(
        compact.contains("fnrender(projection:ViewerProjection,"),
        "{} must render from ViewerProjection rather than the complete facade",
        source_path.display()
    );
    assert!(
        !compact.contains("ViewerProjection::from_panel")
            && !compact.contains("fnfrom_panel(panel:&DesignPanel)"),
        "{} must construct its snapshot at the facade boundary",
        source_path.display()
    );
}

#[test]
fn extracted_projection_factories_use_semantic_groups_instead_of_flat_argument_lists() {
    let sections_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("organisms")
        .join("design")
        .join("panel")
        .join("sections");
    let mut flat_factories = Vec::new();

    for file in [
        "appearance.rs",
        "component.rs",
        "effects.rs",
        "export.rs",
        "header.rs",
        "layout.rs",
        "layout_guides.rs",
        "page.rs",
        "paints.rs",
        "position.rs",
        "typography/mod.rs",
    ] {
        let path = sections_root.join(file);
        let source = fs::read_to_string(&path).expect("extracted section must be readable");
        let lines = source.lines().collect::<Vec<_>>();
        let suppresses_projection_constructor = lines.iter().enumerate().any(|(index, line)| {
            line.contains("allow(clippy::too_many_arguments)")
                && lines
                    .iter()
                    .skip(index + 1)
                    .take(3)
                    .any(|line| line.contains("fn new("))
        });
        if suppresses_projection_constructor {
            flat_factories.push(path);
        }
    }

    assert!(
        flat_factories.is_empty(),
        "extracted projections must group identity, capability, value, and presentation inputs; \
         do not replace facade coupling with oversized constructors: {flat_factories:?}",
    );
}

#[test]
fn paint_transactions_belong_to_the_design_edit_controller() {
    let panel_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("organisms")
        .join("design")
        .join("panel");
    let feature_state_path = panel_root.join("feature_state.rs");
    let edit_controller_path = panel_root.join("edit_controller.rs");
    let feature_state = fs::read_to_string(&feature_state_path)
        .expect("the grouped feature state must be readable");
    let edit_controller = fs::read_to_string(&edit_controller_path)
        .expect("the Design edit controller must be readable");

    assert!(
        !feature_state.contains("ActivePaintEdit")
            && !feature_state.contains("DesignPaintFeatureState")
            && !feature_state.contains("active_edit"),
        "{} may own paint disclosure and hover state, but not host-facing paint transactions",
        feature_state_path.display()
    );
    assert!(
        edit_controller.contains("struct DesignPaintEditLifecycle")
            && edit_controller.contains("paint_lifecycle: DesignPaintEditLifecycle")
            && edit_controller.contains("fn reconcile_paint_edit_target("),
        "{} must own paint Begin/Preview/terminal guards and host-echo reconciliation",
        edit_controller_path.display()
    );
}

#[test]
fn property_scrub_and_paint_features_use_controller_extensions() {
    let panel_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("organisms")
        .join("design")
        .join("panel");

    for (file, trait_name) in [
        ("properties.rs", "DesignPropertiesController"),
        ("scrub.rs", "DesignScrubController"),
        ("paints.rs", "DesignPaintController"),
    ] {
        let path = panel_root.join(file);
        let source = fs::read_to_string(&path).expect("feature controller must be readable");
        assert!(
            source.contains(&format!("trait {trait_name}"))
                && source.contains(&format!("impl {trait_name} for DesignPanel"))
                && !has_inherent_impl(&source, "DesignPanel"),
            "{} must extend the facade only through {trait_name}",
            path.display()
        );
    }

    let facade_path = panel_root
        .parent()
        .expect("the panel module has a Design parent")
        .join("panel.rs");
    let facade = fs::read_to_string(&facade_path).expect("the Design facade must be readable");
    let wrapper = function_body(&facade, "pub fn active_scrub_speed");
    assert!(
        wrapper.contains("self.active_scrub_speed_from_controller()"),
        "DesignPanel::active_scrub_speed must remain a thin compatibility wrapper"
    );
}

#[test]
fn component_authoring_transactions_belong_to_the_edit_controller() {
    let panel_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("organisms")
        .join("design")
        .join("panel");
    let presentation_path = panel_root.join("component_props").join("state.rs");
    let controller_path = panel_root.join("component_authoring_edit_controller.rs");
    let edit_controller_path = panel_root.join("edit_controller.rs");
    let presentation = fs::read_to_string(&presentation_path)
        .expect("component-authoring presentation state must be readable");
    let controller = fs::read_to_string(&controller_path)
        .expect("component-authoring edit controller must be readable");
    let edit_controller =
        fs::read_to_string(&edit_controller_path).expect("Design edit controller must be readable");

    for transaction in ["name_editor", "property_reorder", "variant_option_reorder"] {
        assert!(
            !presentation.contains(transaction),
            "{} must not own the `{transaction}` host transaction",
            presentation_path.display()
        );
        assert!(
            controller.contains(transaction),
            "{} must own the `{transaction}` lifecycle",
            controller_path.display()
        );
    }
    assert!(
        edit_controller.contains("component_authoring: DesignComponentAuthoringEditController")
            && edit_controller.contains("self.component_authoring.has_active_session()"),
        "{} must include component-authoring edits in the shared active-session boundary",
        edit_controller_path.display()
    );
    assert!(
        controller.contains("fn track_action(")
            && controller.contains("fn reconcile_name_edit(")
            && controller.contains("fn reconcile_property_reorder(")
            && controller.contains("fn reconcile_variant_option_reorder("),
        "{} must guard every authoring phase and host-echo transition",
        controller_path.display()
    );
}

#[test]
fn grid_dimensions_lifecycle_belongs_to_the_property_edit_controller() {
    let panel_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("organisms")
        .join("design")
        .join("panel");
    let properties_path = panel_root.join("properties.rs");
    let edit_controller_path = panel_root.join("edit_controller.rs");
    let facade_path = panel_root
        .parent()
        .expect("the panel module has a Design parent")
        .join("panel.rs");
    let properties = fs::read_to_string(&properties_path)
        .expect("the Design properties controller must be readable");
    let edit_controller = fs::read_to_string(&edit_controller_path)
        .expect("the Design edit controller must be readable");
    let facade = fs::read_to_string(&facade_path).expect("the Design facade must be readable");

    let implementation_start = properties
        .rfind("fn emit_grid_dimensions_property_edit(")
        .expect("the Grid dimensions implementation must exist");
    let implementation = function_body(
        &properties[implementation_start..],
        "fn emit_grid_dimensions_property_edit(",
    );
    assert!(
        implementation.contains(".edit_grid_dimensions(")
            && !implementation.contains("match phase"),
        "{} may validate/map Grid values, but phase guarding must remain in the edit controller",
        properties_path.display()
    );
    assert!(
        !properties.contains("self.edit.grid_dimensions =")
            && !properties.contains("GridDimensionsEdit {")
            && !facade.contains("struct GridDimensionsEdit"),
        "Grid transaction state must not return to {} or {}",
        properties_path.display(),
        facade_path.display()
    );
    assert!(
        edit_controller.contains("struct DesignGridDimensionsEditLifecycle")
            && edit_controller
                .contains("grid_dimensions_lifecycle: DesignGridDimensionsEditLifecycle")
            && edit_controller.contains("fn reconcile_grid_dimensions_edit(")
            && edit_controller.contains("fn cancel_grid_dimensions_edit("),
        "{} must own Grid phase guards plus host-echo/context/access reconciliation",
        edit_controller_path.display()
    );
}

#[test]
fn design_panel_facade_delegates_construction_and_canonical_snapshot_ownership() {
    let design_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("organisms")
        .join("design");
    let facade_path = design_root.join("panel.rs");
    let factory_path = design_root.join("panel").join("factory.rs");
    let controller_path = design_root.join("panel").join("view_data_controller.rs");
    let facade = fs::read_to_string(&facade_path).expect("Design facade must be readable");
    let factory = fs::read_to_string(&factory_path).expect("Design panel factory must be readable");
    let controller =
        fs::read_to_string(&controller_path).expect("Design view-data controller must be readable");

    let constructor = function_body(&facade, "pub fn new(");
    let context_constructor = function_body(&facade, "pub fn new_with_context(");
    assert!(
        constructor.contains("DesignPanelFactory::assemble(")
            && context_constructor.contains("DesignPanelFactory::assemble_with_context("),
        "{} must keep its public constructors as thin factory delegates",
        facade_path.display()
    );
    assert!(
        !constructor.contains("cx.new")
            && !constructor.contains("cx.subscribe")
            && factory.contains("impl DesignPanelFactory")
            && factory.contains("cx.subscribe_in(")
            && factory.contains("DesignPanel {"),
        "retained entity and subscription assembly must belong to {}",
        factory_path.display()
    );

    let snapshot = function_body(&facade, "pub fn view_data(");
    let apply_snapshot = function_body(&facade, "pub fn set_view_data(");
    assert!(
        snapshot.contains("DesignPanelViewDataController::canonical_view_data(self)")
            && apply_snapshot.contains("DesignPanelViewDataController::apply_view_data("),
        "{} must delegate canonical snapshot reads and writes exactly once",
        facade_path.display()
    );
    assert!(
        !facade.contains("\n    fn apply_")
            && controller.contains("trait DesignPanelViewDataController")
            && controller.contains("impl DesignPanelViewDataController for DesignPanel")
            && controller.contains("fn canonical_view_data(")
            && controller.contains("fn apply_view_data(")
            && controller.contains("fn apply_node(")
            && controller.contains("fn apply_inspection_context("),
        "canonical snapshot reconciliation must remain in {} instead of returning to {}",
        controller_path.display(),
        facade_path.display()
    );

    let render = function_body(&facade, "fn render(&mut self");
    assert!(
        render.contains("self.render_shell(window, cx)") && !render.contains("match "),
        "the root Render implementation must remain a shell delegate"
    );
}

#[test]
fn design_panel_facade_groups_retained_children_by_ownership_tier() {
    let design_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("organisms")
        .join("design");
    let facade_path = design_root.join("panel.rs");
    let retained_path = design_root.join("panel").join("retained_children.rs");
    let shell_path = design_root.join("panel").join("shell.rs");
    let factory_path = design_root.join("panel").join("factory.rs");
    let facade = fs::read_to_string(&facade_path).expect("Design facade must be readable");
    let retained =
        fs::read_to_string(&retained_path).expect("retained-children module must be readable");
    let shell = fs::read_to_string(&shell_path).expect("Design shell must be readable");
    let factory = fs::read_to_string(&factory_path).expect("Design panel factory must be readable");

    let fields = function_body(&facade, "pub struct DesignPanel {");
    for loose_field in [
        "property_input:",
        "property_variable_search:",
        "component_property_variable_search:",
        "component_swap_search:",
        "font_search:",
        "style_browser_search:",
        "component_multiline_input:",
        "draw_opacity_slider:",
        "draw_corner_radius_slider:",
        "option_states:",
        "option_subscriptions:",
        "option_snapshots:",
        "scroll_handle:",
        "reset_scroll_after_render:",
    ] {
        assert!(
            !fields.contains(loose_field),
            "{} must reach `{loose_field}` through an ownership group instead of declaring it \
             directly on the facade",
            facade_path.display()
        );
    }
    assert!(
        fields.contains("retained: DesignPanelRetainedChildren")
            && fields.contains("shell: DesignPanelShellState"),
        "{} must group retained children and shell scroll continuity",
        facade_path.display()
    );

    for group in [
        "struct DesignPanelRetainedChildren",
        "struct DesignPanelInputStates",
        "struct DesignDrawSliderStates",
        "struct DesignPanelOptionStates",
    ] {
        assert!(
            retained.contains(group),
            "{} must own the `{group}` boundary",
            retained_path.display()
        );
    }
    assert!(
        !has_inherent_impl(&retained, "DesignPanel"),
        "{} must hold retained children without extending the facade",
        retained_path.display()
    );
    assert!(
        retained.contains("_opacity_subscription: Subscription")
            && retained.contains("_corner_radius_subscription: Subscription")
            && retained.contains("fn replace_corner_radius("),
        "{} must keep slider subscriptions private so a rebuilt slider replaces its subscription \
         through one operation",
        retained_path.display()
    );
    assert!(
        shell.contains("struct DesignPanelShellState")
            && shell.contains("scroll_handle: ScrollHandle")
            && shell.contains("reset_after_render: bool"),
        "{} must own top-level scroll continuity",
        shell_path.display()
    );
    assert!(
        factory.contains("DesignPanelRetainedChildren::new(")
            && factory.contains("DesignDrawSliderStates::new(")
            && factory.contains("DesignPanelShellState::default()"),
        "{} must remain the single assembly point for retained children",
        factory_path.display()
    );
}

#[test]
fn geometry_constants_resolve_to_shared_tokens() {
    const TOKEN_BACKED_SUFFIXES: &[&str] = &[
        "_HEIGHT", "_WIDTH", "_PADDING", "_GAP", "_SIZE", "_RADIUS", "_INDENT",
    ];
    // One-off surface geometry that the shared scale deliberately does not
    // absorb: picker ceilings, anchor insets, and rail/column widths a single
    // feature owns. Entries leave this list as constants migrate; a new local
    // dimension belongs in `atoms::tokens` instead of here.
    const ALLOWED_LOCAL_GEOMETRY: &[(&str, &str)] = &[
        ("organisms/design/paint_picker.rs", "PICKER_MAX_HEIGHT"),
        ("organisms/design/paint_picker.rs", "COLOR_AREA_HEIGHT"),
        (
            "organisms/design/typography_style_picker.rs",
            "PICKER_MAX_HEIGHT",
        ),
        ("organisms/layers/panel.rs", "LAYER_ROW_MIN_CONTENT_WIDTH"),
        ("organisms/layers/panel.rs", "LAYER_ICON_SLOT"),
        (
            "organisms/toolbar/component/overlays.rs",
            "ACTIONS_PALETTE_CHROME_HEIGHT",
        ),
        (
            "organisms/toolbar/component/overlays.rs",
            "ACTIONS_RESULTS_MAX_HEIGHT",
        ),
        (
            "organisms/toolbar/component/overlays.rs",
            "AGENT_COMPOSER_ANCHOR_INSET",
        ),
        (
            "organisms/toolbar/component/overlays.rs",
            "ZOOM_MENU_ANCHOR_INSET",
        ),
        ("organisms/toolbar/component/rows.rs", "ROW_FADE_WIDTH"),
        ("organisms/pages/panel.rs", "PAGE_ROW_GAP"),
        ("organisms/pages/panel.rs", "MAX_PAGE_LIST_HEIGHT"),
        ("organisms/pages/panel.rs", "PAGE_MENU_HEIGHT"),
        ("organisms/pages/panel.rs", "SCOPE_MENU_HEIGHT"),
        ("organisms/timeline/mod.rs", "RULER_GUTTER"),
        ("organisms/timeline/mod.rs", "RAIL_WIDTH"),
        ("organisms/timeline/mod.rs", "ZOOM_WIDTH"),
        ("organisms/timeline/mod.rs", "ZOOM_COMPACT_WIDTH"),
        ("organisms/timeline/mod.rs", "ZOOM_TRACK_CHROME"),
        ("organisms/timeline/mod.rs", "ZOOM_TRACK_WIDTH"),
        ("organisms/timeline/mod.rs", "EMPTY_CARD_WIDTH"),
        ("organisms/timeline/mod.rs", "EMPTY_CARD_HEIGHT"),
        ("screens/variables/mod.rs", "SIDEBAR_WIDTH"),
        ("screens/variables/mod.rs", "HEADER_TOOLS_WIDTH"),
        ("screens/variables/mod.rs", "NAME_COLUMN_WIDTH"),
        ("screens/variables/mod.rs", "VALUE_COLUMN_WIDTH"),
        ("screens/variables/mod.rs", "ACTIONS_COLUMN_WIDTH"),
        ("screens/variables/mod.rs", "TABLE_SCROLLBAR_WIDTH"),
        (
            "layouts/pseudo_editor/mod.rs",
            "PSEUDO_EDITOR_LEFT_RAIL_WIDTH",
        ),
        (
            "layouts/pseudo_editor/mod.rs",
            "PSEUDO_EDITOR_RIGHT_RAIL_WIDTH",
        ),
        (
            "layouts/pseudo_editor/mod.rs",
            "PSEUDO_EDITOR_MIN_CANVAS_WIDTH",
        ),
        (
            "layouts/pseudo_editor/mod.rs",
            "PSEUDO_EDITOR_MIN_CANVAS_HEIGHT",
        ),
        (
            "layouts/pseudo_editor/mod.rs",
            "PSEUDO_EDITOR_TOP_BAR_HEIGHT",
        ),
        (
            "layouts/pseudo_editor/mod.rs",
            "PSEUDO_EDITOR_TIMELINE_HEIGHT",
        ),
    ];

    let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut untokenized_geometry = Vec::new();
    let mut active_exemptions = Vec::new();

    for tier in ["organisms", "layouts", "screens"] {
        visit_rust_sources(&source_root.join(tier), &mut |path, source| {
            let relative_path = path
                .strip_prefix(&source_root)
                .expect("visited source must live below the crate source root")
                .to_string_lossy()
                .into_owned();

            // Exemptions are matched against the whole file so a constant
            // declared past a `#[cfg(test)] mod tests;` submodule line still
            // holds its entry accountable.
            for name in bare_numeric_f32_constants(source) {
                if ALLOWED_LOCAL_GEOMETRY.contains(&(relative_path.as_str(), name)) {
                    active_exemptions.push((relative_path.clone(), name.to_owned()));
                }
            }

            for name in bare_numeric_f32_constants(production_source_before_inline_tests(source)) {
                if name.ends_with("_MIN_WIDTH") || name.ends_with("_MIN_HEIGHT") {
                    continue;
                }
                if !TOKEN_BACKED_SUFFIXES
                    .iter()
                    .any(|suffix| name.ends_with(suffix))
                {
                    continue;
                }
                if ALLOWED_LOCAL_GEOMETRY.contains(&(relative_path.as_str(), name)) {
                    continue;
                }
                untokenized_geometry.push((relative_path.clone(), name.to_owned()));
            }
        });
    }

    let stale_exemptions = ALLOWED_LOCAL_GEOMETRY
        .iter()
        .filter(|(path, name)| {
            !active_exemptions
                .iter()
                .any(|(active_path, active_name)| active_path == path && active_name == name)
        })
        .collect::<Vec<_>>();

    assert!(
        untokenized_geometry.is_empty(),
        "fixed chrome geometry must resolve to the shared `atoms::tokens` scale (ARCHITECTURE.md \
         §16): give each constant below a named token at its exact current value — never round it \
         to a neighbouring step — or record it in ALLOWED_LOCAL_GEOMETRY when the dimension is \
         genuinely feature-local; found: {untokenized_geometry:#?}"
    );
    assert!(
        stale_exemptions.is_empty(),
        "remove migrated constants from ALLOWED_LOCAL_GEOMETRY instead of retaining stale \
         geometry exceptions; the list may only shrink: {stale_exemptions:#?}"
    );
}

/// Whether a source file is an out-of-line test module rather than production
/// source. `production_source_before_inline_tests` only strips a `#[cfg(test)]
/// mod tests` block living inside a file; the crate also keeps whole test
/// modules in their own files (`tests.rs`, `interaction_tests.rs`,
/// `composed_host_flow_tests.rs`, `test_support.rs`), each declared under
/// `#[cfg(test)]` by its parent. Their fixture geometry is not production debt.
fn is_out_of_line_test_module(path: &Path) -> bool {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .is_some_and(|stem| stem == "tests" || stem == "test_support" || stem.ends_with("_tests"))
}

#[test]
fn raw_pixel_literal_budget_ratchets_down() {
    // Measured ceilings on hand-written `px(<number>)` call sites in each
    // feature directory's PRODUCTION source: out-of-line test modules are
    // excluded, so fixture geometry never buys headroom for shipping code.
    // Every token migration must lower the directory it touches.
    const RAW_PIXEL_BUDGETS: &[(&str, usize)] = &[
        ("organisms/design", 577),
        ("organisms/toolbar", 106),
        ("screens/variables", 73),
        ("organisms/pages", 40),
        ("organisms/timeline", 63),
        ("organisms/layers", 15),
        ("organisms/prototype", 40),
        ("layouts", 30),
        ("molecules", 25),
        ("atoms", 7),
    ];
    // Headroom a budget may carry before it is stale rather than generous.
    const BUDGET_SLACK: usize = 20;

    let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");

    for (directory, budget) in RAW_PIXEL_BUDGETS {
        let mut literals = 0;
        visit_rust_sources(&source_root.join(directory), &mut |path, source| {
            if is_out_of_line_test_module(path) {
                return;
            }
            literals += raw_pixel_literals(production_source_before_inline_tests(source));
        });

        assert!(
            literals <= *budget,
            "src/{directory} may spend at most {budget} raw `px(<number>)` literals but spends \
             {literals}; route the new geometry through an `atoms::tokens` constant at its exact \
             value instead of widening the budget"
        );
        assert!(
            literals + BUDGET_SLACK >= *budget,
            "src/{directory} is down to {literals} raw `px(<number>)` literals against a budget of \
             {budget}; lower the budget to {literals} so the cleanup ratchets instead of leaving \
             headroom for new literals"
        );
    }
}
