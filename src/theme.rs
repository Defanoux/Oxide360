//! Colour themes (Settings → Theme): the panels' look, the 3D view's
//! background and grid, and the icons' line colour — so icons always stand
//! out from the bar they're on (dark lines on a light theme).

use eframe::egui::{self, Color32};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Theme {
    /// Graphite (the original look).
    #[default]
    Dark,
    /// Blue-grey, with a blue accent.
    Slate,
    /// Light panels and toolbar; the 3D view stays a soft grey so sketch
    /// lines, dimensions and hints keep their colours.
    Light,
}

impl Theme {
    pub const ALL: [Theme; 3] = [Theme::Dark, Theme::Slate, Theme::Light];

    pub fn name(self) -> &'static str {
        match self {
            Theme::Dark => "Dark",
            Theme::Slate => "Slate",
            Theme::Light => "Light",
        }
    }
}

/// A theme's colours.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    /// The 3D view's background.
    pub view: Color32,
    /// Panels (title bar, browser, timeline).
    pub panel: Color32,
    /// The toolbar's card, floating on the panel colour.
    pub card: Color32,
    /// Windows and menus.
    pub window: Color32,
    pub text: Color32,
    pub weak: Color32,
    /// Icon lines, and greyed-out icons.
    pub ink: Color32,
    pub ink_off: Color32,
    /// Selected tool / tab, the workspace name.
    pub accent: Color32,
    /// The current tab's and a hovered tool's background.
    pub raised: Color32,
    /// The toolbar's tool icons in each workspace: solid, sketch, drawing.
    pub tools: [Color32; 3],
}


pub fn palette(t: Theme) -> Palette {
    match t {
        Theme::Dark => Palette {
            view: Color32::from_gray(38),
            panel: Color32::from_gray(27),
            card: Color32::from_gray(34),
            window: Color32::from_gray(27),
            text: Color32::from_gray(200),
            weak: Color32::from_gray(130),
            ink: Color32::from_gray(220),
            ink_off: Color32::from_gray(95),
            accent: Color32::from_rgb(110, 170, 255),
            raised: Color32::from_gray(58),
            tools: [Color32::from_rgb(240, 160, 50), Color32::from_rgb(90, 200, 110), Color32::from_rgb(235, 95, 85)],
        },
        Theme::Slate => Palette {
            view: Color32::from_rgb(44, 56, 68),
            panel: Color32::from_rgb(24, 29, 35),
            card: Color32::from_rgb(31, 38, 46),
            window: Color32::from_rgb(28, 34, 41),
            text: Color32::from_rgb(205, 214, 224),
            weak: Color32::from_rgb(130, 145, 160),
            ink: Color32::from_rgb(222, 230, 240),
            ink_off: Color32::from_rgb(90, 102, 116),
            accent: Color32::from_rgb(80, 145, 255),
            raised: Color32::from_rgb(52, 64, 78),
            tools: [Color32::from_rgb(245, 165, 60), Color32::from_rgb(95, 210, 125), Color32::from_rgb(240, 100, 90)],
        },
        Theme::Light => Palette {
            view: Color32::from_rgb(92, 97, 104),
            panel: Color32::from_rgb(232, 234, 237),
            card: Color32::from_rgb(248, 249, 250),
            window: Color32::from_rgb(244, 245, 247),
            text: Color32::from_gray(30),
            weak: Color32::from_gray(105),
            ink: Color32::from_gray(45),
            ink_off: Color32::from_gray(170),
            accent: Color32::from_rgb(25, 105, 215),
            raised: Color32::from_rgb(208, 214, 222),
            tools: [Color32::from_rgb(215, 115, 15), Color32::from_rgb(35, 150, 65), Color32::from_rgb(200, 50, 40)],
        },
    }
}

static CURRENT: AtomicU8 = AtomicU8::new(0);

/// Make `t` the theme everything draws with.
pub fn set(t: Theme) {
    CURRENT.store(Theme::ALL.iter().position(|x| *x == t).unwrap_or(0) as u8, Ordering::Relaxed);
}

pub fn current() -> Theme {
    Theme::ALL.get(CURRENT.load(Ordering::Relaxed) as usize).copied().unwrap_or_default()
}

/// The current theme's colours.
pub fn now() -> Palette {
    palette(current())
}

/// The sketch grid's (minor, major) line colours: a little lighter than the view.
pub fn grid() -> ([f32; 3], [f32; 3]) {
    let v = now().view;
    let lift = |c: Color32, d: u8| [c.r().saturating_add(d) as f32 / 255.0, c.g().saturating_add(d) as f32 / 255.0, c.b().saturating_add(d) as f32 / 255.0];
    (lift(v, 22), lift(v, 47))
}

/// egui's look for a theme.
pub fn visuals(t: Theme) -> egui::Visuals {
    let p = palette(t);
    let mut v = if t == Theme::Light { egui::Visuals::light() } else { egui::Visuals::dark() };
    v.panel_fill = p.panel;
    v.window_fill = p.window;
    v.extreme_bg_color = if t == Theme::Light { Color32::WHITE } else { p.panel.gamma_multiply(0.8) };
    v.override_text_color = Some(p.text);
    v.selection.bg_fill = p.accent.gamma_multiply(if t == Theme::Light { 0.35 } else { 0.55 });
    v.hyperlink_color = p.accent;
    v.widgets.noninteractive.bg_fill = p.window;
    v.widgets.inactive.weak_bg_fill = p.raised.gamma_multiply(if t == Theme::Light { 1.0 } else { 0.8 });
    v.widgets.hovered.weak_bg_fill = p.raised;
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Perceived brightness 0..=255.
    fn bright(c: Color32) -> f32 {
        0.299 * c.r() as f32 + 0.587 * c.g() as f32 + 0.114 * c.b() as f32
    }

    #[test]
    fn icons_and_text_stand_out_on_every_theme() {
        for t in Theme::ALL {
            let p = palette(t);
            for (what, fg) in [("icons", p.ink), ("text", p.text)] {
                for (on, bg) in [("panel", p.panel), ("card", p.card), ("window", p.window), ("raised", p.raised)] {
                    let d = (bright(fg) - bright(bg)).abs();
                    assert!(d > 110.0, "{t:?}: {what} on the {on} only {d} apart");
                }
            }
            // Each workspace's tool colour stands out from the card.
            for c in p.tools {
                let d = (bright(c) - bright(p.card)).abs();
                assert!(d > 60.0, "{t:?}: a tool colour only {d} from the card");
            }
            // Greyed-out icons still show, but fainter than live ones.
            let (live, off) = ((bright(p.ink) - bright(p.card)).abs(), (bright(p.ink_off) - bright(p.card)).abs());
            assert!(off > 25.0 && off < live, "{t:?}: {off} vs {live}");
        }
    }

    #[test]
    fn the_theme_in_use_switches() {
        set(Theme::Slate);
        assert_eq!(current(), Theme::Slate);
        set(Theme::Dark);
        assert_eq!(current(), Theme::Dark);
    }
}
