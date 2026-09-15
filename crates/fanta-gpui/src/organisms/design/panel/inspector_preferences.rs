use super::{DesignNudgeSettings, DesignVariablesEntryPoint};

/// Cross-file inspector presentation preferences supplied by the host.
///
/// These values never represent document state and never emit document
/// mutation intents when they change.
#[derive(Default)]
pub(super) struct DesignInspectorPreferences {
    pub(super) additional_labels: bool,
    pub(super) nudge_settings: DesignNudgeSettings,
    pub(super) variables_entry_point: DesignVariablesEntryPoint,
}
