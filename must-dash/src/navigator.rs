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

#[derive(PartialEq)]
pub enum DashFocus {
    Data,
    Logs,
}

pub struct Navigator {
    pub view: NavigatorView,
    pub landing_sub_view: LandingSubView,
    /// Cursor position in the available-probes list
    pub probe_list_cursor: usize,
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
            probe_list_cursor: 0,
            probe_config_focus: ProbeConfigFocus::Kp,
            dash_focus: DashFocus::Logs,
            history_scroll: 0,
            graph_scroll: 0,
            shutting_down: false,
        }
    }

    pub fn probe_list_up(&mut self, probe_count: usize) {
        if probe_count > 0 {
            self.probe_list_cursor = self.probe_list_cursor.saturating_sub(1);
        }
    }

    pub fn probe_list_down(&mut self, probe_count: usize) {
        if probe_count > 0 {
            self.probe_list_cursor = (self.probe_list_cursor + 1).min(probe_count - 1);
        }
    }

    pub fn next_config_focus(&mut self) {
        self.probe_config_focus = match self.probe_config_focus {
            ProbeConfigFocus::Kp => ProbeConfigFocus::Ki,
            ProbeConfigFocus::Ki => ProbeConfigFocus::SourceId,
            ProbeConfigFocus::SourceId => ProbeConfigFocus::Confirm,
            ProbeConfigFocus::Confirm => ProbeConfigFocus::Kp,
        };
    }

    pub fn prev_config_focus(&mut self) {
        self.probe_config_focus = match self.probe_config_focus {
            ProbeConfigFocus::Kp => ProbeConfigFocus::Confirm,
            ProbeConfigFocus::Ki => ProbeConfigFocus::Kp,
            ProbeConfigFocus::SourceId => ProbeConfigFocus::Ki,
            ProbeConfigFocus::Confirm => ProbeConfigFocus::SourceId,
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
