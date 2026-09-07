use gpui::SharedString;

/// Read-only page data displayed by [`super::PagesPanel`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PagesPanelItem {
    /// Opaque host identifier. The component returns it unchanged in events.
    pub id: SharedString,
    /// User-facing page name.
    pub title: SharedString,
}

impl PagesPanelItem {
    /// Creates a page row.
    pub fn new(id: impl Into<SharedString>, title: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
        }
    }
}

/// Searchable element categories understood by the Pages panel UI.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum PagesPanelElementKind {
    All,
    Text,
    FrameGroup,
    Component,
    Instance,
    Image,
    Shape,
    Other,
}

impl PagesPanelElementKind {
    /// Stable order used by the filter menu.
    pub const FILTER_ORDER: [Self; 8] = [
        Self::All,
        Self::Text,
        Self::FrameGroup,
        Self::Component,
        Self::Instance,
        Self::Image,
        Self::Shape,
        Self::Other,
    ];

    /// User-facing label.
    pub const fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Text => "Text",
            Self::FrameGroup => "Frame / Group",
            Self::Component => "Component",
            Self::Instance => "Instance",
            Self::Image => "Image",
            Self::Shape => "Shape",
            Self::Other => "Other",
        }
    }
}

/// Optional count displayed next to one element filter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PagesPanelElementCount {
    pub kind: PagesPanelElementKind,
    pub count: usize,
}

impl PagesPanelElementCount {
    pub const fn new(kind: PagesPanelElementKind, count: usize) -> Self {
        Self { kind, count }
    }
}

/// Search scope selected in the Pages panel.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PagesPanelSearchScope {
    #[default]
    CurrentPage,
    AllPages,
}

impl PagesPanelSearchScope {
    pub const fn label(self) -> &'static str {
        match self {
            Self::CurrentPage => "This page",
            Self::AllPages => "All pages",
        }
    }
}

/// Immutable search query sent to the host.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PagesPanelSearchRequest {
    pub query: SharedString,
    pub scope: PagesPanelSearchScope,
    /// Empty means all element kinds.
    pub element_kinds: Vec<PagesPanelElementKind>,
    pub match_case: bool,
    pub whole_words: bool,
}

/// One host-provided search result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PagesPanelSearchResult {
    pub id: SharedString,
    pub title: SharedString,
    pub parent: Option<SharedString>,
    pub kind: PagesPanelElementKind,
}

impl PagesPanelSearchResult {
    pub fn new(
        id: impl Into<SharedString>,
        title: impl Into<SharedString>,
        kind: PagesPanelElementKind,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            parent: None,
            kind,
        }
    }

    pub fn parent(mut self, parent: impl Into<SharedString>) -> Self {
        self.parent = Some(parent.into());
        self
    }
}

/// Host-controlled result data rendered by the search panel.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PagesPanelSearchResults {
    /// Total matches. This may be larger than `items.len()` for paged hosts.
    pub total: usize,
    pub items: Vec<PagesPanelSearchResult>,
    pub element_counts: Vec<PagesPanelElementCount>,
}

/// Direction requested by the previous/next result controls.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PagesPanelResultDirection {
    Previous,
    Next,
}

/// Domain-relevant intent emitted by [`super::PagesPanel`].
///
/// The component updates only transient presentation state. The host remains
/// responsible for applying document operations and feeding updated page and
/// result read models back through the setters on `PagesPanel`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PagesPanelAction {
    ExpansionChanged {
        expanded: bool,
    },
    SelectRequested {
        page_id: SharedString,
    },
    CreateRequested {
        title: SharedString,
    },
    RenameRequested {
        page_id: SharedString,
        title: SharedString,
    },
    DuplicateRequested {
        page_id: SharedString,
    },
    DeleteRequested {
        page_id: SharedString,
    },
    CopyLinkRequested {
        page_id: SharedString,
    },
    SearchRequested(PagesPanelSearchRequest),
    SearchClosed,
    SearchResultSelected {
        result_id: SharedString,
    },
    NavigateResults {
        direction: PagesPanelResultDirection,
        result_id: Option<SharedString>,
    },
    ReplaceRequested {
        request: PagesPanelSearchRequest,
        result_id: Option<SharedString>,
        replacement: SharedString,
    },
    ReplaceAllRequested {
        request: PagesPanelSearchRequest,
        replacement: SharedString,
    },
}
