use crate::app::ProbeConfigFocus;

#[derive(PartialEq)]
pub enum NavigatorView {
    Landing,
    Dashboard,
}

#[derive(PartialEq)]
pub enum LandingSubView {
    ProbeList,
    ProbeConfig,
}

/// Which section of the probe-list view has keyboard focus.
#[derive(PartialEq, Clone, Copy)]
pub enum LandingSection {
    Probes,
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
    pub dash_focus: DashFocus,
    pub history_scroll: usize,
    pub graph_scroll: usize,
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
            dash_focus: DashFocus::Logs,
            history_scroll: 0,
            graph_scroll: 0,
            shutting_down: false,
        }
    }

    /// Move up in the landing view; crosses the section boundary Nodes → Probes.
    pub fn landing_up(&mut self, probe_count: usize) {
        match self.landing_section {
            LandingSection::Probes => {
                if probe_count > 0 {
                    self.probe_list_cursor = self.probe_list_cursor.saturating_sub(1);
                }
            }
            LandingSection::Nodes => {
                if self.node_list_cursor == 0 {
                    self.landing_section = LandingSection::Probes;
                } else {
                    self.node_list_cursor -= 1;
                }
            }
        }
    }

    /// Move down in the landing view; crosses the section boundary Probes → Nodes.
    pub fn landing_down(&mut self, probe_count: usize, node_count: usize) {
        match self.landing_section {
            LandingSection::Probes => {
                if probe_count > 0
                    && self.probe_list_cursor + 1 < probe_count
                {
                    self.probe_list_cursor += 1;
                } else if node_count > 0 {
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
            ProbeConfigFocus::Sf => ProbeConfigFocus::Bw,
            ProbeConfigFocus::Bw => ProbeConfigFocus::Confirm,
            ProbeConfigFocus::Confirm => ProbeConfigFocus::Kp,
        };
    }

    pub fn prev_config_focus(&mut self) {
        self.probe_config_focus = match self.probe_config_focus {
            ProbeConfigFocus::Kp => ProbeConfigFocus::Confirm,
            ProbeConfigFocus::Ki => ProbeConfigFocus::Kp,
            ProbeConfigFocus::SourceId => ProbeConfigFocus::Ki,
            ProbeConfigFocus::Sf => ProbeConfigFocus::SourceId,
            ProbeConfigFocus::Bw => ProbeConfigFocus::Sf,
            ProbeConfigFocus::Confirm => ProbeConfigFocus::Bw,
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
}
