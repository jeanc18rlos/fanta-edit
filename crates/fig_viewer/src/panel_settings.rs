//! Settings for the Fanta design and properties dock panels.

use gpui::Pixels;
use settings::{RegisterSetting, Settings};
use ui::px;
use workspace::dock::DockPosition;

#[derive(Debug, RegisterSetting)]
pub struct FantaDesignPanelSettings {
    pub button: bool,
    pub dock: DockPosition,
    pub default_width: Pixels,
}

impl Settings for FantaDesignPanelSettings {
    fn from_settings(content: &settings::SettingsContent) -> Self {
        let panel = content.fanta_design_panel.as_ref().unwrap();
        Self {
            button: panel.button.unwrap(),
            dock: panel.dock.unwrap().into(),
            default_width: panel.default_width.map(px).unwrap(),
        }
    }
}

#[derive(Debug, RegisterSetting)]
pub struct FantaPropertiesPanelSettings {
    pub button: bool,
    pub dock: DockPosition,
    pub default_width: Pixels,
}

impl Settings for FantaPropertiesPanelSettings {
    fn from_settings(content: &settings::SettingsContent) -> Self {
        let panel = content.fanta_properties_panel.as_ref().unwrap();
        Self {
            button: panel.button.unwrap(),
            dock: panel.dock.unwrap().into(),
            default_width: panel.default_width.map(px).unwrap(),
        }
    }
}
