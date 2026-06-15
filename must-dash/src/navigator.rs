use crate::app::{GatewayConfigFocus, ProbeConfigFocus};

#[derive(PartialEq)]
pub enum NavigatorView {
    Landing,
    Dashboard,
}

#[derive(PartialEq)]
pub enum LandingSubView {
    ProbeList,
    ProbeConfig,
    GatewayConfig,
}

/// Which section of the probe-list view has keyboard focus.
#[derive(PartialEq, Clone, Copy)]
pub enum LandingSection {
    Probes,
    Gateway,
    Nodes,
}

#[derive(PartialEq)]
pub enum DashFocus {
    Data,
    Logs,
}

pub struct Navigator {
    pub view: NavigatorView,
    pub landing_sub_view: LandingSubView,
    /// Which section (probes / configured nodes) has focus
    pub landing_section: LandingSection,
    /// Cursor position in the available-probes list
    pub probe_list_cursor: usize,
    /// Cursor position in the configured-nodes list
    pub node_list_cursor: usize,
    /// Which field is active in the ProbeConfig form
    pub probe_config_focus: ProbeConfigFocus,
    /// Which field is active in the GatewayConfig form
    pub gateway_config_focus: GatewayConfigFocus,
    pub dash_focus: DashFocus,
    pub history_scroll: usize,
    pub graph_scroll: usize,
    pub logs_scroll: usize,
    pub shutting_down: bool,
}

impl Default for Navigator {
    fn default() -> Self {
        Self::new()
    }
}

impl Navigator {
    pub fn new() -> Self {
        Self {
            view: NavigatorView::Landing,
            landing_sub_view: LandingSubView::ProbeList,
            landing_section: LandingSection::Probes,
            probe_list_cursor: 0,
            node_list_cursor: 0,
            probe_config_focus: ProbeConfigFocus::Kp,
            gateway_config_focus: GatewayConfigFocus::Sf,
            dash_focus: DashFocus::Logs,
            history_scroll: 0,
            graph_scroll: 0,
            logs_scroll: 0,
            shutting_down: false,
        }
    }

    pub fn reset_scrolls(&mut self) {
        self.history_scroll = 0;
        self.graph_scroll = 0;
        self.logs_scroll = 0;
        self.shutting_down = true;
    }

    /// Move up in the landing view; crosses section boundaries Nodes → Gateway → Probes.
    pub fn landing_up(&mut self, probe_count: usize) {
        match self.landing_section {
            LandingSection::Probes => {
                if probe_count > 0 {
                    self.probe_list_cursor = self.probe_list_cursor.saturating_sub(1);
                }
            }
            LandingSection::Gateway => {
                self.landing_section = LandingSection::Probes;
            }
            LandingSection::Nodes => {
                if self.node_list_cursor == 0 {
                    self.landing_section = LandingSection::Gateway;
                } else {
                    self.node_list_cursor -= 1;
                }
            }
        }
    }

    /// Move down in the landing view; crosses section boundaries Probes → Gateway → Nodes.
    pub fn landing_down(&mut self, probe_count: usize, node_count: usize) {
        match self.landing_section {
            LandingSection::Probes => {
                if probe_count > 0 && self.probe_list_cursor + 1 < probe_count {
                    self.probe_list_cursor += 1;
                } else {
                    self.landing_section = LandingSection::Gateway;
                }
            }
            LandingSection::Gateway => {
                if node_count > 0 {
                    self.landing_section = LandingSection::Nodes;
                    self.node_list_cursor = 0;
                }
            }
            LandingSection::Nodes => {
                if node_count > 0 {
                    self.node_list_cursor = (self.node_list_cursor + 1).min(node_count - 1);
                }
            }
        }
    }

    pub fn clamp_node_cursor(&mut self, node_count: usize) {
        if node_count == 0 {
            self.landing_section = LandingSection::Probes;
            self.node_list_cursor = 0;
        } else {
            self.node_list_cursor = self.node_list_cursor.min(node_count - 1);
        }
    }

    pub fn next_config_focus(&mut self) {
        self.probe_config_focus = match self.probe_config_focus {
            ProbeConfigFocus::Kp => ProbeConfigFocus::Ki,
            ProbeConfigFocus::Ki => ProbeConfigFocus::SourceId,
            ProbeConfigFocus::SourceId => ProbeConfigFocus::Sf,
            ProbeConfigFocus::Sf => ProbeConfigFocus::AltSf,
            ProbeConfigFocus::AltSf => ProbeConfigFocus::Bw,
            ProbeConfigFocus::Bw => ProbeConfigFocus::Tau,
            ProbeConfigFocus::Tau => ProbeConfigFocus::Confirm,
            ProbeConfigFocus::Confirm => ProbeConfigFocus::Kp,
        };
    }

    pub fn prev_config_focus(&mut self) {
        self.probe_config_focus = match self.probe_config_focus {
            ProbeConfigFocus::Kp => ProbeConfigFocus::Confirm,
            ProbeConfigFocus::Ki => ProbeConfigFocus::Kp,
            ProbeConfigFocus::SourceId => ProbeConfigFocus::Ki,
            ProbeConfigFocus::Sf => ProbeConfigFocus::SourceId,
            ProbeConfigFocus::AltSf => ProbeConfigFocus::Sf,
            ProbeConfigFocus::Bw => ProbeConfigFocus::AltSf,
            ProbeConfigFocus::Tau => ProbeConfigFocus::Bw,
            ProbeConfigFocus::Confirm => ProbeConfigFocus::Tau,
        };
    }

    pub fn next_gateway_focus(&mut self) {
        self.gateway_config_focus = match self.gateway_config_focus {
            GatewayConfigFocus::Sf => GatewayConfigFocus::Bw,
            GatewayConfigFocus::Bw => GatewayConfigFocus::Tau,
            GatewayConfigFocus::Tau => GatewayConfigFocus::Confirm,
            GatewayConfigFocus::Confirm => GatewayConfigFocus::Sf,
        };
    }

    pub fn prev_gateway_focus(&mut self) {
        self.gateway_config_focus = match self.gateway_config_focus {
            GatewayConfigFocus::Sf => GatewayConfigFocus::Confirm,
            GatewayConfigFocus::Bw => GatewayConfigFocus::Sf,
            GatewayConfigFocus::Tau => GatewayConfigFocus::Bw,
            GatewayConfigFocus::Confirm => GatewayConfigFocus::Tau,
        };
    }

    pub fn toggle_dash_focus(&mut self) {
        self.dash_focus = match self.dash_focus {
            DashFocus::Data => DashFocus::Logs,
            DashFocus::Logs => DashFocus::Data,
        };
    }

    pub fn scroll_history_up(&mut self) {
        self.history_scroll = self.history_scroll.saturating_add(1);
    }

    pub fn scroll_history_down(&mut self) {
        self.history_scroll = self.history_scroll.saturating_sub(1);
    }

    pub fn scroll_graph_back(&mut self, total_packets: usize) {
        let max_scroll = total_packets.saturating_sub(1);
        self.graph_scroll = (self.graph_scroll + 1).min(max_scroll);
    }

    pub fn scroll_graph_forward(&mut self) {
        self.graph_scroll = self.graph_scroll.saturating_sub(1);
    }

    pub fn scroll_logs_up(&mut self) {
        self.logs_scroll = self.logs_scroll.saturating_add(1);
    }

    pub fn scroll_logs_down(&mut self) {
        self.logs_scroll = self.logs_scroll.saturating_sub(1);
    }
}
