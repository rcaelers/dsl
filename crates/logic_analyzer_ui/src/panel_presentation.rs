use panel_layout::PanelIcon;

/// Stable semantic icon identity for one built-in application panel.
///
/// Portable UI and native host menus render this identity using their own
/// drawing APIs, so one panel cannot silently acquire unrelated icons at the
/// two presentation boundaries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApplicationPanelIcon {
    Waveform,
    Network,
    Console,
    Chip,
    Eye,
    Target,
    Table,
}

impl ApplicationPanelIcon {
    pub(crate) const fn panel_icon(self) -> PanelIcon {
        match self {
            Self::Waveform => PanelIcon::Waveform,
            Self::Network => PanelIcon::Network,
            Self::Console => PanelIcon::Console,
            Self::Chip => PanelIcon::Chip,
            Self::Eye => PanelIcon::Eye,
            Self::Target => PanelIcon::Target,
            Self::Table => PanelIcon::Table,
        }
    }
}

pub const LOGIC_ANALYZER_PANEL_ICON: ApplicationPanelIcon = ApplicationPanelIcon::Waveform;
pub const NODE_GRAPH_PANEL_ICON: ApplicationPanelIcon = ApplicationPanelIcon::Network;
pub const LOG_PANEL_ICON: ApplicationPanelIcon = ApplicationPanelIcon::Console;
pub const MEMORY_PANEL_ICON: ApplicationPanelIcon = ApplicationPanelIcon::Chip;
pub const WATCHES_PANEL_ICON: ApplicationPanelIcon = ApplicationPanelIcon::Eye;
pub const TRIGGERS_PANEL_ICON: ApplicationPanelIcon = ApplicationPanelIcon::Target;
pub const DECODER_PANEL_ICON: ApplicationPanelIcon = ApplicationPanelIcon::Table;
