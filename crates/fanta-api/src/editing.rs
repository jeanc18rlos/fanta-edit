//! Templates: who may change what. A session acts in a [`Role`], which gives
//! every scope a rule: allow it, deny it, or defer to the node's own editing
//! rules. Edits the rules do not allow are refused, and only a creator
//! changes the rules themselves.

use std::collections::BTreeMap;

use fanta_doc::{AssetId, EditingRules, Fill, NodeData, NodeId, Operation, Placeholder, Scope};
use fanta_headless::edit_operations::replace_data_operation;

use crate::properties::{fill_mut, fills};
use crate::{ApiError, DesignSession, Result};

/// Who is editing the design.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Role {
    /// The template's author: every scope is allowed, and the editing rules
    /// can change.
    #[default]
    Creator,
    /// Someone filling the template in: every scope defers to the node.
    Adopter,
    /// Someone looking at it: no edit is made, whatever the rules say, and
    /// only selection follows its `editor/select` rule.
    Viewer,
}

impl Role {
    fn default_rule(self, scope: Scope) -> ScopeRule {
        match self {
            Role::Creator => ScopeRule::Allow,
            Role::Adopter => ScopeRule::Defer,
            Role::Viewer if scope == Scope::EditorSelect => ScopeRule::Allow,
            Role::Viewer => ScopeRule::Deny,
        }
    }
}

/// The session-wide rule for one scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScopeRule {
    Allow,
    Deny,
    /// Allowed unless the node's editing rules disable the scope.
    Defer,
}

/// The session's role and the rules set over its defaults.
#[derive(Debug, Default)]
pub(crate) struct Permissions {
    role: Role,
    overrides: BTreeMap<Scope, ScopeRule>,
}

impl DesignSession {
    pub fn role(&self) -> Role {
        self.permissions.role
    }

    /// Act as `role`, with its default rule for every scope.
    pub fn set_role(&mut self, role: Role) {
        self.permissions = Permissions {
            role,
            overrides: BTreeMap::new(),
        };
    }

    /// The session-wide rule for `scope`.
    pub fn global_scope(&self, scope: Scope) -> ScopeRule {
        self.permissions
            .overrides
            .get(&scope)
            .copied()
            .unwrap_or_else(|| self.permissions.role.default_rule(scope))
    }

    /// Set the rule for `scope` until the role next changes.
    pub fn set_global_scope(&mut self, scope: Scope, rule: ScopeRule) {
        self.permissions.overrides.insert(scope, rule);
    }

    /// Whether the session may make a change of `scope` to `node`.
    pub fn is_allowed(&self, node: NodeId, scope: Scope) -> Result<bool> {
        Ok(self.allows(self.node(node)?.editing.as_deref(), scope))
    }

    /// Whether `node`'s own rules leave `scope` enabled, whatever the role.
    pub fn is_scope_enabled(&self, node: NodeId, scope: Scope) -> Result<bool> {
        Ok(self
            .node(node)?
            .editing
            .as_ref()
            .is_none_or(|rules| rules.allows(scope)))
    }

    /// Enable or disable `scope` on `node`, for sessions that defer to it.
    pub fn set_scope_enabled(&mut self, node: NodeId, scope: Scope, enabled: bool) -> Result<()> {
        if scope == Scope::Unknown {
            return Err(ApiError::InvalidArgument(
                "`unknown` stands for scopes this build does not know".into(),
            ));
        }
        self.change_rules(node, |rules| {
            if enabled {
                rules.disabled_scopes.remove(&scope);
            } else {
                rules.disabled_scopes.insert(scope);
            }
        })
    }

    pub fn placeholder(&self, node: NodeId) -> Result<Option<Placeholder>> {
        Ok(self
            .node(node)?
            .editing
            .as_ref()
            .and_then(|rules| rules.placeholder))
    }

    /// Mark `node` as a placeholder the template's user fills in, or unmark
    /// it with `None`.
    pub fn set_placeholder(
        &mut self,
        node: NodeId,
        placeholder: Option<Placeholder>,
    ) -> Result<()> {
        self.change_rules(node, |rules| rules.placeholder = placeholder)
    }

    /// The placeholders, in tree order.
    pub fn find_all_placeholders(&self) -> Vec<NodeId> {
        self.find(|_, node| {
            node.editing
                .as_ref()
                .is_some_and(|rules| rules.placeholder.is_some())
        })
    }

    /// Show the image in `bytes` in place of the placeholder `node`'s: a
    /// bitmap's own image, or its first image fill. The node keeps its box
    /// and image fit; a crop is dropped, since it framed the old image.
    pub fn replace_placeholder_image(&mut self, node: NodeId, bytes: Vec<u8>) -> Result<()> {
        if self.placeholder(node)?.is_none() {
            return Err(ApiError::InvalidArgument(format!(
                "{node} is not a placeholder"
            )));
        }
        self.permit(Some(node), Scope::FillChange)?;
        let data = &self.node(node)?.data;
        let shows_image = matches!(data, NodeData::Bitmap(_))
            || fills(data).iter().any(|fill| fill.image_asset().is_some());
        if !shows_image {
            return Err(ApiError::InvalidArgument(format!(
                "{node} shows no image to replace"
            )));
        }
        let (asset, natural_size, inserted) = self.document_mut().ingest_image(bytes)?;
        let operations = replace_data_operation(self.doc(), node, |data| {
            replace_image(data, asset, natural_size)
        });
        let result = self.edit(operations);
        if result.is_err() && inserted {
            self.document_mut().remove_image(asset);
        }
        result
    }

    /// Refuse a change of `scope` to `node`, or to the document when there
    /// is no node, unless the session's rules allow it.
    pub(crate) fn permit(&self, node: Option<NodeId>, scope: Scope) -> Result<()> {
        let rules = match node {
            Some(node) => self.node(node)?.editing.as_deref(),
            None => None,
        };
        if self.allows(rules, scope) {
            Ok(())
        } else {
            Err(ApiError::NotAllowed { scope, node })
        }
    }

    fn allows(&self, rules: Option<&EditingRules>, scope: Scope) -> bool {
        if self.permissions.role == Role::Viewer && scope != Scope::EditorSelect {
            return false;
        }
        match self.global_scope(scope) {
            ScopeRule::Allow => true,
            ScopeRule::Deny => false,
            ScopeRule::Defer => rules.is_none_or(|rules| rules.allows(scope)),
        }
    }

    fn change_rules(&mut self, node: NodeId, change: impl FnOnce(&mut EditingRules)) -> Result<()> {
        if self.permissions.role != Role::Creator {
            return Err(ApiError::CreatorOnly);
        }
        let old = self.node(node)?.editing.clone();
        let mut rules = old.as_deref().cloned().unwrap_or_default();
        change(&mut rules);
        let new = (!rules.is_default()).then(|| Box::new(rules));
        if new == old {
            return Ok(());
        }
        self.edit(vec![Operation::SetEditingRules { id: node, old, new }])
    }
}

fn replace_image(data: &mut NodeData, asset: AssetId, natural_size: [u32; 2]) {
    if let NodeData::Bitmap(bitmap) = data {
        bitmap.asset = asset;
        bitmap.natural_size = natural_size;
        bitmap.crop = None;
        return;
    }
    let Some(index) = fills(data)
        .iter()
        .position(|fill| fill.image_asset().is_some())
    else {
        return;
    };
    if let Some(fill) = fill_mut(data, index) {
        fill.set_image_asset(asset);
        if let Fill::Image { crop, .. } = fill {
            *crop = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NodeKind;
    use fanta_doc::BitmapNode;

    fn png(width: u32, height: u32, rgba: [u8; 4]) -> Vec<u8> {
        let mut bytes = Vec::new();
        image::RgbaImage::from_pixel(width, height, image::Rgba(rgba))
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .expect("encode png");
        bytes
    }

    fn bitmap(session: &DesignSession, node: NodeId) -> BitmapNode {
        match &session.node(node).expect("node").data {
            NodeData::Bitmap(bitmap) => bitmap.clone(),
            _ => panic!("a bitmap"),
        }
    }

    #[test]
    fn roles_set_the_default_rule_for_every_scope() {
        let mut session = DesignSession::new().expect("session");
        assert_eq!(session.role(), Role::Creator);
        assert!(
            Scope::ALL
                .iter()
                .all(|scope| session.global_scope(*scope) == ScopeRule::Allow)
        );
        session.set_role(Role::Adopter);
        assert!(
            Scope::ALL
                .iter()
                .all(|scope| session.global_scope(*scope) == ScopeRule::Defer)
        );
        session.set_global_scope(Scope::LayerMove, ScopeRule::Deny);
        assert_eq!(session.global_scope(Scope::LayerMove), ScopeRule::Deny);
        session.set_role(Role::Viewer);
        assert_eq!(session.global_scope(Scope::LayerMove), ScopeRule::Deny);
        assert_eq!(session.global_scope(Scope::EditorSelect), ScopeRule::Allow);
        session.set_role(Role::Adopter);
        assert_eq!(
            session.global_scope(Scope::LayerMove),
            ScopeRule::Defer,
            "a new role starts from its defaults"
        );
    }

    #[test]
    fn an_adopter_is_refused_what_the_node_disables() {
        let mut session = DesignSession::new().expect("session");
        let page = session.active_page().expect("page");
        let logo = session
            .create_node(page, NodeKind::Rectangle)
            .expect("logo");
        session
            .set_scope_enabled(logo, Scope::LayerMove, false)
            .expect("lock position");
        session.set_role(Role::Adopter);
        assert!(!session.is_allowed(logo, Scope::LayerMove).expect("query"));
        assert!(matches!(
            session.set_float(logo, "position/x", 40.0),
            Err(ApiError::NotAllowed {
                scope: Scope::LayerMove,
                node: Some(node),
            }) if node == logo
        ));
        session.flip(logo, true).expect("flipping is its own scope");
        session
            .set_float(logo, "opacity", 0.5)
            .expect("other scopes defer to the node, which allows them");
        session.set_global_scope(Scope::LayerMove, ScopeRule::Allow);
        session
            .set_float(logo, "position/x", 40.0)
            .expect("the session rule wins over the node");
    }

    #[test]
    fn destroying_a_node_needs_every_node_inside_it_to_allow_it() {
        let mut session = DesignSession::new().expect("session");
        let page = session.active_page().expect("page");
        let card = session.create_node(page, NodeKind::Frame).expect("card");
        let badge = session.create_node(card, NodeKind::Ellipse).expect("badge");
        session
            .set_scope_enabled(badge, Scope::LifecycleDestroy, false)
            .expect("keep the badge");
        session.set_role(Role::Adopter);
        assert!(matches!(
            session.destroy(card),
            Err(ApiError::NotAllowed {
                scope: Scope::LifecycleDestroy,
                ..
            })
        ));
        assert!(session.is_valid(card) && session.is_valid(badge));
    }

    #[test]
    fn a_viewer_changes_nothing_but_the_selection() {
        let mut session = DesignSession::new().expect("session");
        let page = session.active_page().expect("page");
        let title = session
            .create_node(page, NodeKind::Text("{{title}}".into()))
            .expect("title");
        session.set_role(Role::Viewer);
        assert!(matches!(
            session.set_name(title, "Headline"),
            Err(ApiError::ViewOnly)
        ));
        assert!(matches!(
            session.set_text_variable("title", "Hello"),
            Err(ApiError::ViewOnly)
        ));
        session.set_global_scope(Scope::TextEdit, ScopeRule::Allow);
        assert!(session.set_string(title, "text/content", "x").is_err());
        assert!(!session.is_allowed(title, Scope::TextEdit).expect("query"));
        session.select(&[title]).expect("select");
        assert_eq!(session.selection(), vec![title]);
    }

    #[test]
    fn only_a_creator_changes_editing_rules() {
        let mut session = DesignSession::new().expect("session");
        let page = session.active_page().expect("page");
        let photo = session
            .create_node(page, NodeKind::Image(png(4, 2, [0, 0, 255, 255])))
            .expect("photo");
        session.set_role(Role::Adopter);
        assert!(matches!(
            session.set_scope_enabled(photo, Scope::LayerMove, false),
            Err(ApiError::CreatorOnly)
        ));
        assert!(matches!(
            session.set_placeholder(photo, Some(Placeholder::default())),
            Err(ApiError::CreatorOnly)
        ));
        assert!(
            session
                .is_scope_enabled(photo, Scope::LayerMove)
                .expect("query")
        );
    }

    #[test]
    fn placeholders_are_found_and_their_images_replaced() {
        let mut session = DesignSession::new().expect("session");
        let page = session.active_page().expect("page");
        let photo = session
            .create_node(page, NodeKind::Image(png(4, 2, [0, 0, 255, 255])))
            .expect("photo");
        let caption = session
            .create_node(page, NodeKind::Rectangle)
            .expect("caption");
        session
            .set_placeholder(
                photo,
                Some(Placeholder {
                    show_overlay: true,
                    show_button: true,
                }),
            )
            .expect("mark placeholder");
        session
            .set_placeholder(caption, Some(Placeholder::default()))
            .expect("mark");
        assert_eq!(session.find_all_placeholders(), vec![photo, caption]);
        let before = bitmap(&session, photo);

        session.set_role(Role::Adopter);
        session
            .replace_placeholder_image(photo, png(8, 8, [255, 0, 0, 255]))
            .expect("replace");
        let after = bitmap(&session, photo);
        assert_ne!(after.asset, before.asset);
        assert_eq!(after.natural_size, [8, 8]);
        assert_eq!(after.local_size, before.local_size, "the box is kept");
        assert_eq!(after.fit, before.fit, "the fit is kept");
        assert!(session.document().raw_assets().contains_key(&after.asset));

        assert!(matches!(
            session.replace_placeholder_image(caption, png(2, 2, [0, 0, 0, 255])),
            Err(ApiError::InvalidArgument(_))
        ));
        session.set_role(Role::Creator);
        session.set_placeholder(photo, None).expect("unmark");
        assert_eq!(session.find_all_placeholders(), vec![caption]);
        assert!(matches!(
            session.replace_placeholder_image(photo, png(2, 2, [0, 0, 0, 255])),
            Err(ApiError::InvalidArgument(_))
        ));
    }

    #[test]
    fn editing_rules_survive_writing_and_reopening_a_project() {
        let mut session = DesignSession::new().expect("session");
        let page = session.active_page().expect("page");
        let logo = session
            .create_node(page, NodeKind::Rectangle)
            .expect("logo");
        session
            .set_scope_enabled(logo, Scope::LifecycleDestroy, false)
            .expect("rule");
        session
            .set_placeholder(logo, Some(Placeholder::default()))
            .expect("placeholder");
        session.add_undo_step("Template").expect("commit");
        let root = tempfile::tempdir().expect("tempdir");
        let project = root.path().join("Template");
        session.write_project(&project).expect("write");

        let mut reopened = DesignSession::open_project(&project).expect("open");
        assert!(
            !reopened
                .is_scope_enabled(logo, Scope::LifecycleDestroy)
                .expect("rule")
        );
        assert_eq!(reopened.find_all_placeholders(), vec![logo]);
        reopened.set_role(Role::Adopter);
        assert!(reopened.destroy(logo).is_err());
    }
}
