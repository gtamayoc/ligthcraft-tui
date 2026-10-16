//! Responsive 4-pane layout engine, focus ring, and geometry calculator.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, BorderType, Borders};

/// Unique identifier for interactive application panes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PaneId {
    Library,
    PreviewCanvas,
    Inspector,
    QueueStatus,
}

impl PaneId {
    /// Cycle focus forward to the next interactive pane.
    pub fn next(&self) -> Self {
        match self {
            Self::Library => Self::PreviewCanvas,
            Self::PreviewCanvas => Self::Inspector,
            Self::Inspector => Self::Library,
            Self::QueueStatus => Self::Library,
        }
    }

    /// Cycle focus backward to the previous interactive pane.
    pub fn prev(&self) -> Self {
        match self {
            Self::Library => Self::Inspector,
            Self::PreviewCanvas => Self::Library,
            Self::Inspector => Self::PreviewCanvas,
            Self::QueueStatus => Self::Inspector,
        }
    }
}

/// Evaluated geometry for all panes across the terminal.
#[derive(Debug, Clone)]
pub struct AppLayout {
    pub total_area: Rect,
    pub header_area: Rect,
    pub library_area: Rect,
    pub preview_area: Rect,
    pub inspector_area: Rect,
    pub status_area: Rect,
    pub is_too_small: bool,
}

impl AppLayout {
    pub const MIN_WIDTH: u16 = 80;
    pub const MIN_HEIGHT: u16 = 24;

    /// Compute responsive pane rects for given terminal bounds.
    pub fn compute(area: Rect) -> Self {
        if area.width < Self::MIN_WIDTH || area.height < Self::MIN_HEIGHT {
            return Self {
                total_area: area,
                header_area: Rect::default(),
                library_area: Rect::default(),
                preview_area: Rect::default(),
                inspector_area: Rect::default(),
                status_area: Rect::default(),
                is_too_small: true,
            };
        }

        // 1. Vertical split: [Header, Middle, Status]
        let header_height = if area.height >= 30 { 3 } else { 1 };
        let status_height = if area.height >= 30 { 3 } else { 2 };
        let middle_height = area.height.saturating_sub(header_height + status_height);

        let vert_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(header_height),
                Constraint::Min(middle_height),
                Constraint::Length(status_height),
            ])
            .split(area);

        let header_area = vert_chunks[0];
        let middle_area = vert_chunks[1];
        let status_area = vert_chunks[2];

        // 2. Middle horizontal split: [Library, Preview Canvas, Inspector]
        let horiz_constraints = if area.width >= 160 {
            [
                Constraint::Percentage(20), // Library: 20%
                Constraint::Percentage(55), // Canvas: 55%
                Constraint::Percentage(25), // Inspector: 25%
            ]
        } else if area.width >= 120 {
            [
                Constraint::Percentage(22),
                Constraint::Percentage(52),
                Constraint::Percentage(26),
            ]
        } else {
            [
                Constraint::Length(22), // Compact fixed library
                Constraint::Min(36),    // Flexible canvas
                Constraint::Length(26), // Compact fixed inspector
            ]
        };

        let middle_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(horiz_constraints)
            .split(middle_area);

        Self {
            total_area: area,
            header_area,
            library_area: middle_chunks[0],
            preview_area: middle_chunks[1],
            inspector_area: middle_chunks[2],
            status_area,
            is_too_small: false,
        }
    }

    /// Hit-test column and row coordinates against interactive panes.
    pub fn pane_at(&self, col: u16, row: u16) -> Option<PaneId> {
        let pt = (col, row);
        if contains(self.library_area, pt) {
            Some(PaneId::Library)
        } else if contains(self.preview_area, pt) {
            Some(PaneId::PreviewCanvas)
        } else if contains(self.inspector_area, pt) {
            Some(PaneId::Inspector)
        } else if contains(self.status_area, pt) {
            Some(PaneId::QueueStatus)
        } else {
            None
        }
    }

    /// Return styled Block border depending on focus state.
    pub fn block_for(&self, pane: PaneId, focused_pane: PaneId, title: &'static str) -> Block<'static> {
        let is_focused = pane == focused_pane;
        if is_focused {
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Thick)
                .border_style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
                .title(format!(" ❯ [ACTIVO] {} ❮ ", title))
        } else {
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::DarkGray))
                .title(format!(" {} ", title))
        }
    }
}

fn contains(rect: Rect, (x, y): (u16, u16)) -> bool {
    x >= rect.x
        && x < rect.x.saturating_add(rect.width)
        && y >= rect.y
        && y < rect.y.saturating_add(rect.height)
}
