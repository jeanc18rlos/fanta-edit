use std::rc::Rc;

use fanta_doc::{
    Doc, Mode, ModeId, ModeScope, NodeData, NodeId, Operation, VariableCollectionId,
    resolve_effective_mode,
};
use gpui::{App, IntoElement, RenderOnce, SharedString, Window, div, px};
use ui::{ContextMenu, ContextMenuEntry, DropdownMenu, DropdownStyle, IconPosition, prelude::*};

#[derive(Clone)]
pub(crate) struct ModeOverrideRow {
    pub(crate) collection: VariableCollectionId,
    pub(crate) collection_name: SharedString,
    pub(crate) explicit: Option<ModeId>,
    pub(crate) inherited: ModeId,
    pub(crate) modes: Vec<Mode>,
}

#[derive(Clone)]
pub(crate) struct ModeOverridesModel {
    pub(crate) scope: ModeScope,
    pub(crate) scope_name: SharedString,
    pub(crate) rows: Vec<ModeOverrideRow>,
}

type ModeOverrideHandler = Rc<dyn Fn(VariableCollectionId, Option<ModeId>, &mut Window, &mut App)>;

#[derive(IntoElement)]
pub(crate) struct ModeOverridesControl {
    id: SharedString,
    model: ModeOverridesModel,
    disabled: bool,
    on_change: ModeOverrideHandler,
}

impl ModeOverridesControl {
    pub(crate) fn new(
        id: impl Into<SharedString>,
        model: ModeOverridesModel,
        on_change: impl Fn(VariableCollectionId, Option<ModeId>, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            model,
            disabled: false,
            on_change: Rc::new(on_change),
        }
    }

    pub(crate) fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

impl RenderOnce for ModeOverridesControl {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let mut rows = v_flex().gap_1();
        let scope = self.model.scope.clone();
        for row in self.model.rows {
            let inherited_name =
                mode_name(&row.modes, row.inherited).unwrap_or_else(|| "Unknown mode".into());
            let current_label: SharedString = row
                .explicit
                .and_then(|mode| mode_name(&row.modes, mode))
                .unwrap_or_else(|| format!("Inherit · {inherited_name}").into());
            let selected = row.explicit;
            let collection = row.collection;
            let modes = row.modes.clone();
            let on_change = self.on_change.clone();
            let menu = ContextMenu::build(window, cx, move |mut menu, _, _| {
                let on_change_inherit = on_change.clone();
                menu.push_item(
                    ContextMenuEntry::new(format!("Inherit ({inherited_name})"))
                        .toggleable(IconPosition::End, selected.is_none())
                        .handler(move |window, cx| on_change_inherit(collection, None, window, cx)),
                );
                for mode in &modes {
                    let on_change = on_change.clone();
                    let mode_id = mode.id;
                    menu.push_item(
                        ContextMenuEntry::new(mode.name.clone())
                            .toggleable(IconPosition::End, selected == Some(mode_id))
                            .handler(move |window, cx| {
                                on_change(collection, Some(mode_id), window, cx)
                            }),
                    );
                }
                menu
            });
            rows = rows.child(
                h_flex()
                    .min_h(px(30.0))
                    .gap_2()
                    .child(
                        div().flex_1().min_w_0().child(
                            Label::new(row.collection_name)
                                .size(LabelSize::Small)
                                .single_line(),
                        ),
                    )
                    .child(
                        div().w(px(150.0)).flex_none().child(
                            DropdownMenu::new(
                                mode_override_row_id(&self.id, &scope, row.collection),
                                current_label,
                                menu,
                            )
                            .style(DropdownStyle::Outlined)
                            .trigger_size(ButtonSize::Compact)
                            .full_width(true)
                            .disabled(self.disabled),
                        ),
                    ),
            );
        }
        rows
    }
}

fn mode_override_row_id(
    base: &str,
    scope: &ModeScope,
    collection: VariableCollectionId,
) -> SharedString {
    let scope = match scope {
        ModeScope::Doc => "doc".to_owned(),
        ModeScope::Frame { node } => format!("frame-{node}"),
    };
    format!("{base}-{scope}-{collection}").into()
}

pub(crate) fn mode_overrides_model(doc: &Doc, node: Option<NodeId>) -> Option<ModeOverridesModel> {
    let node_id = node.or_else(|| doc.active_page())?;
    let node = doc.scene.get(node_id)?;
    let NodeData::Group(group) = &node.data else {
        return None;
    };
    let mut collections: Vec<_> = doc.variables.collections.values().collect();
    collections.sort_by(|left, right| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then(left.id.cmp(&right.id))
    });
    let rows = collections
        .into_iter()
        .filter(|collection| !collection.modes.is_empty())
        .map(|collection| {
            let inherited = node.parent.map_or_else(
                || {
                    doc.active_modes
                        .get(&collection.id)
                        .copied()
                        .filter(|mode| collection.has_mode(*mode))
                        .unwrap_or(collection.default_mode)
                },
                |parent| resolve_effective_mode(&doc.scene, parent, collection, &doc.active_modes),
            );
            ModeOverrideRow {
                collection: collection.id,
                collection_name: collection.name.clone().into(),
                explicit: group
                    .explicit_modes
                    .get(&collection.id)
                    .copied()
                    .filter(|mode| collection.has_mode(*mode)),
                inherited,
                modes: collection.modes.clone(),
            }
        })
        .collect();
    Some(ModeOverridesModel {
        scope: ModeScope::Frame { node: node_id },
        scope_name: if node.name.trim().is_empty() {
            "Untitled container".into()
        } else {
            node.name.clone().into()
        },
        rows,
    })
}

pub(crate) fn mode_override_operation(
    doc: &Doc,
    scope: ModeScope,
    collection: VariableCollectionId,
    new: Option<ModeId>,
) -> Result<Option<Operation>, &'static str> {
    let collection_data = doc
        .variables
        .collections
        .get(&collection)
        .ok_or("The collection no longer exists")?;
    if new.is_some_and(|mode| !collection_data.has_mode(mode)) {
        return Err("The selected mode no longer exists");
    }
    let old = match &scope {
        ModeScope::Doc => doc.active_modes.get(&collection).copied(),
        ModeScope::Frame { node } => {
            let node = doc
                .scene
                .get(*node)
                .ok_or("The mode container no longer exists")?;
            let NodeData::Group(group) = &node.data else {
                return Err("Only pages and containers can override modes");
            };
            group.explicit_modes.get(&collection).copied()
        }
    };
    Ok((old != new).then_some(Operation::SetActiveMode {
        scope,
        collection,
        old,
        new,
    }))
}

fn mode_name(modes: &[Mode], mode: ModeId) -> Option<SharedString> {
    modes
        .iter()
        .find(|candidate| candidate.id == mode)
        .map(|candidate| candidate.name.clone().into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use fanta_doc::{CanvasNode, GroupNode, Mode, VariableCollection, VectorNode};

    #[test]
    fn mode_override_row_ids_follow_scope_and_collection_instead_of_row_order() {
        let first = VariableCollectionId::new();
        let second = VariableCollectionId::new();
        let frame = NodeId::new();

        let first_id = mode_override_row_id("modes", &ModeScope::Doc, first);
        assert_eq!(
            first_id,
            mode_override_row_id("modes", &ModeScope::Doc, first)
        );
        assert_ne!(
            first_id,
            mode_override_row_id("modes", &ModeScope::Doc, second)
        );
        assert_ne!(
            first_id,
            mode_override_row_id("modes", &ModeScope::Frame { node: frame }, first)
        );
    }

    #[test]
    fn nested_container_model_reports_inherited_and_explicit_modes() {
        let mut doc = Doc::new();
        let collection = VariableCollectionId::new();
        let light = ModeId::new();
        let dark = ModeId::new();
        doc.variables.collections.insert(
            collection,
            VariableCollection {
                id: collection,
                name: "Theme".into(),
                modes: vec![
                    Mode {
                        id: light,
                        name: "Light".into(),
                    },
                    Mode {
                        id: dark,
                        name: "Dark".into(),
                    },
                ],
                default_mode: light,
                variable_order: Vec::new(),
            },
        );
        let mut page = CanvasNode::new(NodeData::Group(GroupNode::default()));
        page.name = "Page".into();
        let page_id = page.id;
        doc.scene.insert(page).expect("insert page");
        doc.add_page(page_id);
        doc.apply(
            mode_override_operation(
                &doc,
                ModeScope::Frame { node: page_id },
                collection,
                Some(dark),
            )
            .expect("valid page mode")
            .expect("page mode change"),
        )
        .expect("apply page mode");

        let mut section = CanvasNode::new(NodeData::Group(GroupNode::default()));
        section.name = "Section".into();
        section.parent = Some(page_id);
        let section_id = section.id;
        doc.scene.insert(section).expect("insert section");
        let model = mode_overrides_model(&doc, Some(section_id)).expect("section mode model");
        assert_eq!(model.rows.len(), 1);
        assert_eq!(model.rows[0].explicit, None);
        assert_eq!(model.rows[0].inherited, dark);

        doc.apply(
            mode_override_operation(
                &doc,
                ModeScope::Frame { node: section_id },
                collection,
                Some(light),
            )
            .expect("valid section mode")
            .expect("section mode change"),
        )
        .expect("apply section mode");
        let model = mode_overrides_model(&doc, Some(section_id)).expect("section mode model");
        assert_eq!(model.rows[0].explicit, Some(light));
        assert_eq!(model.rows[0].inherited, dark);

        let mut child = CanvasNode::new(NodeData::Vector(VectorNode::default()));
        child.parent = Some(section_id);
        let child_id = child.id;
        doc.scene.insert(child).expect("insert nested child");
        assert_eq!(
            resolve_effective_mode(
                &doc.scene,
                child_id,
                &doc.variables.collections[&collection],
                &doc.active_modes,
            ),
            light,
            "the nearest container override wins for nested content"
        );
    }
}
