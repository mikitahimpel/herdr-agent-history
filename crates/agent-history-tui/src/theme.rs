//! Colors for the browser. The palette is supplied by the integration; this
//! crate only knows the terminal default and how to derive widget styles.
pub use ratatui::style::Color;
use ratatui::style::{Modifier, Style};

/// Named color tokens. The token set matches the palette vocabulary used by
/// terminal-multiplexer themes (Catppuccin naming), so a host theme can be
/// injected token for token without this crate knowing where it came from.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Palette {
    /// Focused borders, the active role tab, the title.
    pub accent: Color,
    /// Background painted behind the whole browser.
    pub panel_bg: Color,
    /// Background of the selected result while the results pane is focused.
    pub surface0: Color,
    /// Unfocused borders and separators.
    pub surface1: Color,
    /// Background of the selected result while another pane is focused.
    pub surface_dim: Color,
    /// Muted text: dates, counts, placeholders.
    pub overlay0: Color,
    /// Key-hint labels.
    pub overlay1: Color,
    pub text: Color,
    pub subtext0: Color,
    /// Branch names.
    pub mauve: Color,
    /// Assistant replies.
    pub green: Color,
    /// Matched query terms.
    pub yellow: Color,
    /// Errors.
    pub red: Color,
    /// User messages.
    pub blue: Color,
    /// Codex.
    pub teal: Color,
    /// Claude and recovery prompts.
    pub peach: Color,
}

impl Palette {
    /// ANSI-16 colors plus the terminal's own foreground and background, so
    /// the browser follows whatever scheme the terminal emulator uses.
    pub const fn terminal() -> Self {
        Self {
            accent: Color::Blue,
            panel_bg: Color::Reset,
            surface0: Color::DarkGray,
            surface1: Color::DarkGray,
            surface_dim: Color::DarkGray,
            overlay0: Color::Gray,
            overlay1: Color::Gray,
            text: Color::Reset,
            subtext0: Color::Gray,
            mauve: Color::Magenta,
            green: Color::Green,
            yellow: Color::Yellow,
            red: Color::LightRed,
            blue: Color::Blue,
            teal: Color::Cyan,
            peach: Color::Yellow,
        }
    }

    /// Foreground for text drawn on an `accent` or `yellow` background.
    fn on_color(&self) -> Color {
        match self.panel_bg {
            Color::Reset => Color::Black,
            bg => bg,
        }
    }

    pub(crate) fn base(&self) -> Style {
        Style::new().fg(self.text).bg(self.panel_bg)
    }
    pub(crate) fn muted(&self) -> Style {
        Style::new().fg(self.overlay0)
    }
    pub(crate) fn border(&self, focused: bool) -> Style {
        if focused {
            Style::new().fg(self.accent)
        } else {
            Style::new().fg(self.surface1)
        }
    }
    pub(crate) fn pane_title(&self, focused: bool) -> Style {
        if focused {
            Style::new().fg(self.accent).add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(self.subtext0)
        }
    }
    pub(crate) fn selection(&self, focused: bool) -> Style {
        if focused {
            Style::new().bg(self.surface0).add_modifier(Modifier::BOLD)
        } else {
            Style::new().bg(self.surface_dim)
        }
    }
    pub(crate) fn active_tab(&self) -> Style {
        Style::new()
            .fg(self.on_color())
            .bg(self.accent)
            .add_modifier(Modifier::BOLD)
    }
    pub(crate) fn inactive_tab(&self) -> Style {
        Style::new().fg(self.subtext0)
    }
    pub(crate) fn matched(&self) -> Style {
        Style::new()
            .fg(self.on_color())
            .bg(self.yellow)
            .add_modifier(Modifier::BOLD)
    }
    pub(crate) fn key(&self) -> Style {
        Style::new().fg(self.accent).add_modifier(Modifier::BOLD)
    }
    pub(crate) fn error(&self) -> Style {
        Style::new().fg(self.red).add_modifier(Modifier::BOLD)
    }
}

impl Default for Palette {
    fn default() -> Self {
        Self::terminal()
    }
}
