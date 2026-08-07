//! Centralized design tokens.
//!
//! Every color, border style, and reusable style "recipe" (bold key hints,
//! headings, selection highlight, status colors, ...) used anywhere in
//! `src/ui` is defined exactly once here, on [`Theme`]. Views never
//! hardcode a [`Color`] or [`BorderType`] directly — they call a method on
//! `&Theme` instead. This is what makes it possible to ship more than one
//! palette ([`Theme::dark`] and [`Theme::light`]) without touching any
//! rendering code: swapping `App::theme` re-skins the whole application.
//!
//! Token catalogue:
//! - **Color** — `background`, `text`, `text_secondary`, `text_muted`,
//!   `accent`, `accent_alt`, `on_accent`, `success`, `warning`, `danger`,
//!   `danger_text`, `info`.
//! - **Border ("line thickness" / "corner style")** — `border_focused_type`,
//!   `border_unfocused_type`.
//! - **Style recipes** — methods below combine tokens with [`Modifier`]s
//!   (bold, underline) so call sites never write `add_modifier` ad hoc.

use microsandbox::sandbox::SandboxStatus;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::BorderType;

/// Which built-in palette a [`Theme`] represents. Used to label the theme
/// and to implement the toggle keybinding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThemeMode {
    #[default]
    Dark,
    Light,
}

impl ThemeMode {
    /// The other mode — used by the theme-toggle keybinding.
    pub fn toggled(self) -> Self {
        match self {
            ThemeMode::Dark => ThemeMode::Light,
            ThemeMode::Light => ThemeMode::Dark,
        }
    }
}

/// The full set of design tokens used across every view.
#[derive(Debug, Clone, Copy)]
pub struct Theme {
    pub mode: ThemeMode,

    // Base surface -------------------------------------------------------
    /// Fills the whole terminal before anything else is drawn, so every
    /// panel (and every modal) sits on a consistent, theme-correct
    /// background regardless of the user's own terminal color scheme.
    pub background: Color,
    /// Primary, high-emphasis text (values, selected items, sandbox names).
    pub text: Color,
    /// Secondary text — one step down in emphasis from `text` (e.g.
    /// unselected list items).
    pub text_secondary: Color,
    /// Muted/dim text — hints, timestamps, labels, unfocused borders.
    pub text_muted: Color,

    // Accents -------------------------------------------------------------
    /// Primary accent — focused-panel borders, active tab, headings,
    /// selection highlight, links/paths.
    pub accent: Color,
    /// Secondary accent — panel/card titles.
    pub accent_alt: Color,
    /// Text color to use on top of an `accent`-colored background.
    pub on_accent: Color,

    // Semantic status -------------------------------------------------------
    /// Running / success / healthy.
    pub success: Color,
    /// Stopped / caution / key-hint highlight.
    pub warning: Color,
    /// Crashed / error / destructive action.
    pub danger: Color,
    /// A softer variant of `danger` used for body text (e.g. stderr log
    /// lines) where the full-strength `danger` would be reserved for
    /// badges/borders.
    pub danger_text: Color,
    /// Informational accent distinct from status semantics (e.g.
    /// directories, PTY output).
    pub info: Color,

    // Borders ("line thickness" / "corner style") --------------------------
    /// Border style for the panel/card that currently has focus.
    pub border_focused_type: BorderType,
    /// Border style for panels/cards without focus.
    pub border_unfocused_type: BorderType,
}

impl Default for Theme {
    fn default() -> Self {
        Self::dark()
    }
}

impl Theme {
    /// Construct the theme for a given [`ThemeMode`].
    pub fn for_mode(mode: ThemeMode) -> Self {
        match mode {
            ThemeMode::Dark => Self::dark(),
            ThemeMode::Light => Self::light(),
        }
    }

    /// Default palette: high-contrast colors on a black background.
    pub fn dark() -> Self {
        Self {
            mode: ThemeMode::Dark,
            background: Color::Black,
            text: Color::White,
            text_secondary: Color::Gray,
            text_muted: Color::DarkGray,
            accent: Color::Cyan,
            accent_alt: Color::Magenta,
            on_accent: Color::Black,
            success: Color::Green,
            warning: Color::Yellow,
            danger: Color::Red,
            danger_text: Color::LightRed,
            info: Color::Blue,
            border_focused_type: BorderType::Rounded,
            border_unfocused_type: BorderType::Rounded,
        }
    }

    /// Bright palette: dark text on a white background. Uses a few
    /// truecolor tones (instead of the plain ANSI 16) where the named
    /// color would otherwise be low-contrast on a light surface (e.g.
    /// plain `Yellow`/`Gray` nearly disappear on white).
    pub fn light() -> Self {
        Self {
            mode: ThemeMode::Light,
            background: Color::White,
            text: Color::Black,
            text_secondary: Color::Rgb(60, 60, 60),
            text_muted: Color::Rgb(120, 120, 120),
            accent: Color::Blue,
            accent_alt: Color::Magenta,
            on_accent: Color::White,
            success: Color::Rgb(0, 128, 0),
            warning: Color::Rgb(150, 110, 0),
            danger: Color::Rgb(178, 34, 34),
            danger_text: Color::Rgb(200, 40, 40),
            info: Color::Blue,
            border_focused_type: BorderType::Rounded,
            border_unfocused_type: BorderType::Rounded,
        }
    }

    // ---------------------------------------------------------------
    // Style recipes
    // ---------------------------------------------------------------

    /// Base style for the whole screen: `text` on `background`. Rendered
    /// once as a full-screen fill so every widget that only sets `fg`
    /// still sits on a theme-correct background.
    pub fn base_style(&self) -> Style {
        Style::default().fg(self.text).bg(self.background)
    }

    /// Primary text.
    pub fn text(&self) -> Style {
        Style::default().fg(self.text)
    }

    /// Primary text, bold.
    pub fn text_bold(&self) -> Style {
        self.text().add_modifier(Modifier::BOLD)
    }

    /// Secondary (dimmer than primary, brighter than muted) text.
    pub fn secondary(&self) -> Style {
        Style::default().fg(self.text_secondary)
    }

    /// Muted/dim text — hints, timestamps, disabled state.
    pub fn muted(&self) -> Style {
        Style::default().fg(self.text_muted)
    }

    /// Primary accent color.
    pub fn accent(&self) -> Style {
        Style::default().fg(self.accent)
    }

    /// Primary accent color, bold.
    pub fn accent_bold(&self) -> Style {
        self.accent().add_modifier(Modifier::BOLD)
    }

    /// Section heading style (e.g. "General", "Timestamps" in the Info tab).
    pub fn heading(&self) -> Style {
        self.accent()
            .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
    }

    /// Panel/card title accent (secondary accent color, bold).
    pub fn title_accent(&self) -> Style {
        Style::default()
            .fg(self.accent_alt)
            .add_modifier(Modifier::BOLD)
    }

    /// Style for a single keyboard-shortcut letter shown in a hint line
    /// (e.g. the `t` in "`t` terminate").
    pub fn key_hint(&self) -> Style {
        Style::default()
            .fg(self.warning)
            .add_modifier(Modifier::BOLD)
    }

    /// Highlight style for the current selection (bold text on an
    /// accent-colored background) — active tab, selected row in a list
    /// dialog, etc.
    pub fn selected(&self) -> Style {
        Style::default()
            .fg(self.on_accent)
            .bg(self.accent)
            .add_modifier(Modifier::BOLD)
    }

    /// A small "badge" style: bold text on an accent-colored background,
    /// used for standalone chips like the header title.
    pub fn badge(&self) -> Style {
        self.selected()
    }

    /// Style for a single label in a horizontal tab bar. Used by every
    /// tabbed view in the app (detail panel's Info/Metrics/Logs/Filesystem
    /// tabs, the create-sandbox dialog's Basic/Advanced tabs, ...) so they
    /// all look identical.
    pub fn tab_style(&self, active: bool) -> Style {
        if active {
            self.selected()
        } else {
            self.secondary()
        }
    }

    /// Build the spans for a full horizontal tab bar from `(label,
    /// is_active)` pairs: each label is padded to `" Label "` and styled
    /// via [`Theme::tab_style`], with a two-space gap between labels.
    /// Alongside the spans, returns the on-screen width (in columns) of
    /// each individual label span (including its padding, excluding the
    /// trailing gap), so callers that need per-tab mouse hit-rects (e.g.
    /// the detail panel) don't have to re-derive the padding/spacing
    /// rules themselves.
    pub fn tab_bar<'a>(&self, labels: &[(&'a str, bool)]) -> (Vec<Span<'a>>, Vec<u16>) {
        let mut spans = Vec::with_capacity(labels.len() * 2);
        let mut widths = Vec::with_capacity(labels.len());
        for (i, &(label, active)) in labels.iter().enumerate() {
            if i > 0 {
                spans.push(Span::raw("  "));
            }
            let text = format!(" {label} ");
            widths.push(text.chars().count() as u16);
            spans.push(Span::styled(text, self.tab_style(active)));
        }
        (spans, widths)
    }

    pub fn success(&self) -> Style {
        Style::default().fg(self.success)
    }

    pub fn success_bold(&self) -> Style {
        self.success().add_modifier(Modifier::BOLD)
    }

    pub fn warning(&self) -> Style {
        Style::default().fg(self.warning)
    }

    pub fn danger(&self) -> Style {
        Style::default().fg(self.danger)
    }

    pub fn danger_bold(&self) -> Style {
        self.danger().add_modifier(Modifier::BOLD)
    }

    /// Softer danger tone for body text (e.g. stderr log lines).
    pub fn danger_text(&self) -> Style {
        Style::default().fg(self.danger_text)
    }

    pub fn info(&self) -> Style {
        Style::default().fg(self.info)
    }

    /// Border line/fg color for a focused or unfocused panel.
    pub fn border_style(&self, focused: bool) -> Style {
        Style::default().fg(if focused {
            self.accent
        } else {
            self.text_muted
        })
    }

    /// Border line style ("thickness"/corner treatment) for a focused or
    /// unfocused panel.
    pub fn border_type(&self, focused: bool) -> BorderType {
        if focused {
            self.border_focused_type
        } else {
            self.border_unfocused_type
        }
    }

    /// Color that represents a sandbox's lifecycle status everywhere it is
    /// shown (list card dot, Info tab, ...).
    pub fn status_color(&self, status: SandboxStatus) -> Color {
        match status {
            SandboxStatus::Running => self.success,
            SandboxStatus::Stopped => self.warning,
            SandboxStatus::Crashed => self.danger,
            _ => self.text_muted,
        }
    }

    /// Threshold-based color for a percentage gauge (CPU/memory/disk usage).
    pub fn gauge_color(&self, pct: f64) -> Color {
        if pct >= 90.0 {
            self.danger
        } else if pct >= 70.0 {
            self.warning
        } else {
            self.success
        }
    }

    /// Build the alternating key/description spans used by every hint line
    /// in the app (footer, sandbox card actions, dialog hints, ...), e.g.
    /// `[("q", "quit"), ("↑↓", "navigate")]` renders as `q quit  ↑↓
    /// navigate`. Centralizing this keeps every hint line visually
    /// identical instead of some using key-highlighting and others plain
    /// text.
    pub fn hint_spans<'a>(&self, pairs: &[(&'a str, &'a str)]) -> Vec<ratatui::text::Span<'a>> {
        let mut spans = Vec::with_capacity(pairs.len() * 3);
        for (i, (key, desc)) in pairs.iter().enumerate() {
            if i > 0 {
                spans.push(ratatui::text::Span::raw("  "));
            }
            spans.push(ratatui::text::Span::styled(*key, self.key_hint()));
            spans.push(ratatui::text::Span::styled(
                format!(" {desc}"),
                self.muted(),
            ));
        }
        spans
    }

    /// Same as [`Theme::hint_spans`], wrapped in a [`Line`](ratatui::text::Line).
    pub fn hint_line<'a>(&self, pairs: &[(&'a str, &'a str)]) -> ratatui::text::Line<'a> {
        ratatui::text::Line::from(self.hint_spans(pairs))
    }
}

//--------------------------------------------------------------------------------------------------
// Tests
//--------------------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_theme_is_dark() {
        let theme = Theme::default();
        assert_eq!(theme.mode, ThemeMode::Dark);
        assert_eq!(theme.background, Color::Black);
    }

    #[test]
    fn test_theme_mode_toggled_round_trips() {
        assert_eq!(ThemeMode::Dark.toggled(), ThemeMode::Light);
        assert_eq!(ThemeMode::Light.toggled(), ThemeMode::Dark);
        assert_eq!(ThemeMode::Dark.toggled().toggled(), ThemeMode::Dark);
    }

    #[test]
    fn test_for_mode_constructs_matching_palette() {
        assert_eq!(Theme::for_mode(ThemeMode::Dark).mode, ThemeMode::Dark);
        assert_eq!(Theme::for_mode(ThemeMode::Light).mode, ThemeMode::Light);
    }

    #[test]
    fn test_light_theme_uses_light_background_and_dark_text() {
        let theme = Theme::light();
        assert_eq!(theme.background, Color::White);
        assert_eq!(theme.text, Color::Black);
    }

    #[test]
    fn test_status_color_maps_every_status() {
        let theme = Theme::dark();
        assert_eq!(theme.status_color(SandboxStatus::Running), theme.success);
        assert_eq!(theme.status_color(SandboxStatus::Stopped), theme.warning);
        assert_eq!(theme.status_color(SandboxStatus::Crashed), theme.danger);
    }

    #[test]
    fn test_gauge_color_thresholds() {
        let theme = Theme::dark();
        assert_eq!(theme.gauge_color(10.0), theme.success);
        assert_eq!(theme.gauge_color(70.0), theme.warning);
        assert_eq!(theme.gauge_color(89.9), theme.warning);
        assert_eq!(theme.gauge_color(90.0), theme.danger);
        assert_eq!(theme.gauge_color(100.0), theme.danger);
    }

    #[test]
    fn test_border_style_and_type_depend_on_focus() {
        let theme = Theme::dark();
        assert_eq!(theme.border_type(true), theme.border_focused_type);
        assert_eq!(theme.border_type(false), theme.border_unfocused_type);
        assert_eq!(theme.border_style(true).fg, Some(theme.accent));
        assert_eq!(theme.border_style(false).fg, Some(theme.text_muted));
    }

    #[test]
    fn test_hint_spans_produces_key_and_description_for_each_pair() {
        let theme = Theme::dark();
        let spans = theme.hint_spans(&[("q", "quit"), ("r", "refresh")]);
        // key + description per pair, plus a separator between pairs.
        assert_eq!(spans.len(), 5);
        assert_eq!(spans[0].content, "q");
        assert_eq!(spans[0].style.fg, Some(theme.warning));
        assert_eq!(spans[1].content, " quit");
        assert_eq!(spans[3].content, "r");
        assert_eq!(spans[4].content, " refresh");
    }

    #[test]
    fn test_hint_spans_empty_for_no_pairs() {
        let theme = Theme::dark();
        assert!(theme.hint_spans(&[]).is_empty());
    }
}
