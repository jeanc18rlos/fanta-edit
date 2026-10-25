//! A document open for editing without the app: the [`Doc`], its image
//! assets, the layout baseline that decides what an edit re-solves, and the
//! journal of committed edits. [`HeadlessProject`](crate::HeadlessProject)
//! pairs one with the project directory it reads and saves; a document can
//! also live in memory only.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, mpsc};

use anyhow::{Context as _, Result};
use fanta_design_ops::DesignOp;
use fanta_doc::{
    AssetId, CanvasNode, Doc, GroupNode, JournalEvent, JournalSink, NodeData, NodeId, Operation,
    Transaction,
};
use fanta_render::{AssetResolver, asset::LazyAssetResolver};
use serde_json::Value;

use crate::apply::{self, BatchProgress};
use crate::assets::{AssetStores, OverlayAssetResolver};
use crate::{layout, render};

pub struct HeadlessDocument {
    pub(crate) doc: Doc,
    pub(crate) raw_assets: Arc<BTreeMap<AssetId, Vec<u8>>>,
    pub(crate) assets_changed: bool,
    pub(crate) asset_resolver: Option<Arc<dyn AssetResolver>>,
    /// Images that applied ops ingested, layered over `asset_resolver`.
    pub(crate) asset_overlay: Option<Arc<OverlayAssetResolver>>,
    pub(crate) layout: layout::LayoutState,
    journal_events: mpsc::Receiver<JournalEvent>,
    transactions: Vec<Transaction>,
    journal: Vec<Operation>,
}

impl HeadlessDocument {
    /// Take `doc` and its encoded image assets for editing: record its
    /// committed edits from here on and take its current layout as solved.
    pub fn new(mut doc: Doc, raw_assets: BTreeMap<AssetId, Vec<u8>>) -> Self {
        let (sender, journal_events) = mpsc::channel();
        doc.set_journal_sink(Some(JournalSink::new(sender)));
        let raw_assets = Arc::new(raw_assets);
        let asset_resolver = (!raw_assets.is_empty()).then(|| {
            Arc::new(LazyAssetResolver::with_size_reader(
                raw_assets.clone(),
                Arc::new(render::decode_embedded_image),
                Arc::new(render::embedded_image_size),
            )) as Arc<dyn AssetResolver>
        });
        let layout = layout::LayoutState::new(&doc);
        Self {
            doc,
            raw_assets,
            assets_changed: false,
            asset_resolver,
            asset_overlay: None,
            layout,
            journal_events,
            transactions: Vec::new(),
            journal: Vec::new(),
        }
    }

    /// A new design with one empty page, "Page 1", and no history.
    pub fn empty() -> Result<Self> {
        let mut doc = Doc::new();
        let mut page = CanvasNode::new(NodeData::Group(GroupNode::default()));
        page.name = "Page 1".into();
        let root = page.id;
        doc.apply(Operation::create_node(page))
            .context("creating the first page")?;
        doc.add_page(root);
        doc.set_active_page(Some(root));
        doc.history = Default::default();
        Ok(Self::new(doc, BTreeMap::new()))
    }

    pub fn doc(&self) -> &Doc {
        &self.doc
    }

    /// The document for editing. Change it through [`Doc::apply`] or
    /// [`transaction`](Self::transaction), so history and the journal record
    /// the change.
    pub fn doc_mut(&mut self) -> &mut Doc {
        &mut self.doc
    }

    /// The encoded bytes of every image asset, keyed by asset id.
    pub fn raw_assets(&self) -> &BTreeMap<AssetId, Vec<u8>> {
        &self.raw_assets
    }

    /// What renders resolve image assets through.
    pub fn asset_resolver(&self) -> Option<Arc<dyn AssetResolver>> {
        self.asset_resolver.clone()
    }

    /// Ingest encoded image bytes as an asset of the document. Returns the
    /// asset, its natural size and whether this call added it.
    pub fn ingest_image(&mut self, bytes: Vec<u8>) -> Result<(AssetId, [u32; 2], bool)> {
        let ingested = AssetStores::new(
            &mut self.raw_assets,
            &mut self.asset_resolver,
            &mut self.asset_overlay,
        )
        .add_image_tracked(bytes)?;
        self.assets_changed |= ingested.2;
        Ok(ingested)
    }

    /// Drop an image [`ingest_image`](Self::ingest_image) added, when the
    /// edit that needed it fails.
    pub fn remove_image(&mut self, asset: AssetId) {
        AssetStores::new(
            &mut self.raw_assets,
            &mut self.asset_resolver,
            &mut self.asset_overlay,
        )
        .remove(asset);
    }

    /// Apply design ops as one undo step, with parentless ops placed in
    /// `default_container`. When `commit` is false the batch is checked and
    /// rolled back. Returns the `batch_design` result: `applied`, per-op
    /// status and created ids.
    pub fn apply_design_ops(
        &mut self,
        default_container: Option<NodeId>,
        label: &str,
        ops: &[DesignOp],
        commit: bool,
    ) -> Result<Value> {
        anyhow::ensure!(!ops.is_empty(), "the ops list is empty");
        let selection: Vec<NodeId> = self.doc.selection.iter().copied().collect();
        let viewport = self.doc.viewport;
        let modified_at = self.doc.metadata.modified_at;
        let mut stores = AssetStores::new(
            &mut self.raw_assets,
            &mut self.asset_resolver,
            &mut self.asset_overlay,
        );
        let mut progress = BatchProgress::new(default_container);
        if !ops.iter().all(apply::design_operation_is_presence) {
            self.doc.history.begin(label, &mut self.doc.scene);
            progress.mark_history_started();
        }
        for (index, operation) in ops.iter().enumerate() {
            progress.apply(&mut self.doc, &mut stores, index, operation);
            if progress.failed() {
                break;
            }
        }
        let outcome = if commit {
            progress.finish(&mut self.doc, &mut stores, ops.len())
        } else {
            let outcome = progress.roll_back(&mut self.doc, &mut stores, ops.len());
            self.doc.selection.replace_with(selection);
            self.doc.viewport = viewport;
            outcome
        };
        // Every applied op stamps the edit time, even one rolled back; a batch
        // that leaves the document as it was must not rewrite its metadata.
        if !commit || outcome.value["applied"] != true {
            self.doc.metadata.modified_at = modified_at;
        }
        self.assets_changed |= !outcome.ingested_assets.is_empty();
        self.settle_layout()?;
        Ok(outcome.value)
    }

    /// Apply `operations` as one undo step, then settle the layout it moved.
    pub fn transaction(&mut self, label: &str, operations: Vec<Operation>) -> Result<()> {
        if operations.is_empty() {
            return Ok(());
        }
        self.doc.history.begin(label, &mut self.doc.scene);
        for operation in operations {
            if let Err(error) = self.doc.apply(operation) {
                self.doc
                    .abort_transaction()
                    .context("rolling back the edit")?;
                return Err(error).context("applying the edit");
            }
        }
        self.doc.history.commit(&mut self.doc.scene);
        self.settle_layout()?;
        Ok(())
    }

    /// Re-solve the auto layout and auto-sized text that edits invalidated.
    /// Returns the nodes the solve moved or resized.
    pub fn settle_layout(&mut self) -> Result<BTreeSet<NodeId>> {
        let result = layout::settle(&mut self.doc, &mut self.layout);
        self.drain_journal();
        result
    }

    /// Every operation committed since the document was opened, in order,
    /// with the geometry each edit's layout solve added: the journal the
    /// editor replays to bring an open document up to date.
    pub fn journal(&self) -> &[Operation] {
        &self.journal
    }

    fn drain_journal(&mut self) {
        let mut changed = false;
        while let Ok(event) = self.journal_events.try_recv() {
            changed = true;
            match event {
                JournalEvent::Commit(transaction) | JournalEvent::Redo(transaction) => {
                    self.transactions.push(transaction);
                }
                JournalEvent::Amend(transaction) => match self.transactions.last_mut() {
                    Some(last) => *last = transaction,
                    None => self.transactions.push(transaction),
                },
                JournalEvent::Undo(_) => {
                    self.transactions.pop();
                }
            }
        }
        if changed {
            self.journal = self
                .transactions
                .iter()
                .flat_map(|transaction| transaction.ops.iter().cloned())
                .collect();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_document_has_one_active_page_and_no_history() {
        let document = HeadlessDocument::empty().expect("empty document");
        let doc = document.doc();
        assert_eq!(doc.pages().len(), 1);
        assert_eq!(doc.active_page(), doc.pages().first().copied());
        assert_eq!(doc.history.undo_depth(), 0);
        assert!(document.journal().is_empty());
        assert!(document.raw_assets().is_empty());
    }

    #[test]
    fn settle_layout_reports_the_nodes_it_moved() {
        let mut document = HeadlessDocument::empty().expect("empty document");
        let page = document.doc().active_page().expect("page");
        let applied = document
            .apply_design_ops(
                Some(page),
                "Row",
                &serde_json::from_value::<Vec<DesignOp>>(serde_json::json!([
                    {"op": "create_node", "node_type": "frame", "x": 0, "y": 0,
                     "width": 10, "height": 10},
                ]))
                .expect("ops"),
                true,
            )
            .expect("apply");
        let row: NodeId = applied["created"][0]
            .as_str()
            .and_then(|id| id.parse().ok())
            .expect("row id");
        document
            .apply_design_ops(
                Some(page),
                "Hug",
                &serde_json::from_value::<Vec<DesignOp>>(serde_json::json!([
                    {"op": "create_node", "node_type": "rectangle", "parent": row.to_string(),
                     "x": 0, "y": 0, "width": 30, "height": 20},
                ]))
                .expect("ops"),
                true,
            )
            .expect("add a child");
        let old = document.doc().scene.get(row).expect("row").data.clone();
        let mut new = old.clone();
        if let NodeData::Group(group) = &mut new {
            group.auto_layout = Some(fanta_doc::AutoLayout {
                primary_sizing: fanta_doc::AxisSizing::Hug,
                counter_sizing: fanta_doc::AxisSizing::Hug,
                ..Default::default()
            });
        }
        // Apply without settling, so this test observes the solve.
        document
            .doc_mut()
            .apply(Operation::ReplaceData {
                id: row,
                old: Box::new(old),
                new: Box::new(new),
            })
            .expect("turn on auto layout");
        let moved = document.settle_layout().expect("settle");
        assert!(moved.contains(&row), "the hugging frame resized: {moved:?}");
    }
}
