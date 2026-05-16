#[derive(PartialEq)]
pub enum NavigatorView {
    Landing,
    Dashboard,
}

#[derive(PartialEq, Clone, Copy)]
pub enum LandingFocus {
    Kp,
    Ki,
    SourceId,
    Start,
    Save,
}

#[derive(PartialEq)]
pub enum DashFocus {
    Data,
    Logs,
}

pub struct Navigator {
    pub view: NavigatorView,
    pub landing_focus: LandingFocus,
    pub dash_focus: DashFocus,
    pub history_scroll: usize,

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
            landing_focus: LandingFocus::Kp,
            dash_focus: DashFocus::Logs,
            view: NavigatorView::Landing,
            history_scroll: 0,

            shutting_down: false,
        }
    }

    pub fn next_landing_focus(&mut self, can_save: bool) {
        self.landing_focus = match self.landing_focus {
            LandingFocus::Kp => LandingFocus::Ki,
            LandingFocus::Ki => LandingFocus::SourceId,
            LandingFocus::SourceId => LandingFocus::Start,
            LandingFocus::Start => {
                if can_save {
                    LandingFocus::Save
                } else {
                    LandingFocus::Kp
                }
            }
            LandingFocus::Save => LandingFocus::Kp,
        }
    }

    pub fn prev_landing_focus(&mut self, can_save: bool) {
        self.landing_focus = match self.landing_focus {
            LandingFocus::Kp => {
                if can_save {
                    LandingFocus::Save
                } else {
                    LandingFocus::Start
                }
            }
            LandingFocus::Ki => LandingFocus::Kp,
            LandingFocus::SourceId => LandingFocus::Ki,
            LandingFocus::Start => LandingFocus::SourceId,
            LandingFocus::Save => LandingFocus::Start,
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
}
