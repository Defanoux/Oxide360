//! 2-D drawings (Fusion's Drawing): a sheet with the part's standard views,
//! a border and a title block. Views are laid out by ISO first-angle
//! projection (the top view below the front, the right view to its left) at
//! a standard scale that fits. Only the settings are saved with the design;
//! the lines are worked out from the bodies each time (`kernel::hlr_view`).
//!
//! The sheet is a list of lines and texts in paper millimetres (origin at
//! the bottom left, Y up), drawn on screen and written to PDF / DXF alike.

use crate::doc::BodyId;
use crate::kernel::ViewLines;
use glam::{DVec2, DVec3};
use serde::{Deserialize, Serialize};

/// Paper sizes, landscape.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SheetSize {
    A4,
    #[default]
    A3,
    A2,
}

impl SheetSize {
    pub const ALL: [SheetSize; 3] = [SheetSize::A4, SheetSize::A3, SheetSize::A2];

    pub fn name(self) -> &'static str {
        match self {
            SheetSize::A4 => "A4",
            SheetSize::A3 => "A3",
            SheetSize::A2 => "A2",
        }
    }

    /// Width and height (mm).
    pub fn size(self) -> DVec2 {
        match self {
            SheetSize::A4 => DVec2::new(297.0, 210.0),
            SheetSize::A3 => DVec2::new(420.0, 297.0),
            SheetSize::A2 => DVec2::new(594.0, 420.0),
        }
    }
}

/// The standard views.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ViewKind {
    Front,
    Top,
    Right,
    Iso,
}

impl ViewKind {
    pub const ALL: [ViewKind; 4] = [ViewKind::Front, ViewKind::Top, ViewKind::Right, ViewKind::Iso];

    pub fn name(self) -> &'static str {
        match self {
            ViewKind::Front => "Front",
            ViewKind::Top => "Top",
            ViewKind::Right => "Right",
            ViewKind::Iso => "Iso",
        }
    }

    /// Where the viewer is (toward them) and the paper's X, in model space
    /// (Z up, the front faces -Y, as the home view).
    pub fn axes(self) -> (DVec3, DVec3) {
        match self {
            ViewKind::Front => (DVec3::NEG_Y, DVec3::X),
            ViewKind::Top => (DVec3::Z, DVec3::X),
            ViewKind::Right => (DVec3::X, DVec3::Y),
            ViewKind::Iso => (DVec3::new(1.0, -1.0, 1.0).normalize(), DVec3::new(1.0, 1.0, 0.0).normalize()),
        }
    }
}

/// What's saved with the design.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DrawingSettings {
    #[serde(default)]
    pub sheet: SheetSize,
    /// Paper mm per model mm; None picks the largest standard scale that fits.
    #[serde(default)]
    pub scale: Option<f64>,
    /// Hidden lines dashed in the front, top and right views (never the iso).
    #[serde(default = "yes")]
    pub hidden_lines: bool,
    #[serde(default = "all_views")]
    pub views: Vec<ViewKind>,
    /// Title block: the part's name (empty: the design's).
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub drawn_by: String,
    /// What's drawn: these bodies (picked one by one or by component);
    /// empty means the whole design (every visible body).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bodies: Vec<BodyId>,
}

fn yes() -> bool {
    true
}

fn all_views() -> Vec<ViewKind> {
    ViewKind::ALL.to_vec()
}

impl Default for DrawingSettings {
    fn default() -> Self {
        DrawingSettings { sheet: SheetSize::A3, scale: None, hidden_lines: true, views: all_views(), title: String::new(), drawn_by: String::new(), bodies: Vec::new() }
    }
}

/// Tick (`on`) or untick these bodies in what's drawn (a component's
/// bodies go together). Unticking the last leaves the whole design.
pub fn pick_bodies(chosen: &mut Vec<BodyId>, ids: &[BodyId], on: bool) {
    chosen.retain(|b| !ids.contains(b));
    if on {
        chosen.extend_from_slice(ids);
    }
}

/// Standard scales, largest first (paper per model: 5 is 5:1, 0.5 is 1:2).
pub const SCALES: [f64; 13] = [10.0, 5.0, 2.0, 1.0, 0.5, 0.2, 0.1, 0.05, 0.02, 0.01, 0.005, 0.002, 0.001];

/// "2:1", "1:1", "1:5"…
pub fn scale_text(s: f64) -> String {
    if s >= 1.0 { format!("{}:1", round_text(s)) } else { format!("1:{}", round_text(1.0 / s)) }
}

fn round_text(x: f64) -> String {
    let r = (x * 1000.0).round() / 1000.0;
    if r.fract() == 0.0 { format!("{}", r as i64) } else { format!("{r}") }
}

/// Inside the border, and the gap round views (room for dimensions later).
const BORDER: f64 = 10.0;
const GAP: f64 = 15.0;
/// The title block, bottom right.
pub const TITLE_SIZE: DVec2 = DVec2::new(180.0, 36.0);

/// Where one view goes: paper = origin + model2d * scale.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placed {
    pub kind: ViewKind,
    pub origin: DVec2,
    pub scale: f64,
}

/// Lay the views out (their 1:1 bounds on paper, from `ViewLines::bounds`)
/// by first-angle projection, at `settings.scale` or the largest standard
/// scale that fits. The scale used, and each view placed.
pub fn layout(settings: &DrawingSettings, bounds: &[(ViewKind, (DVec2, DVec2))]) -> (f64, Vec<Placed>) {
    let size = |k: ViewKind| bounds.iter().find(|(v, _)| *v == k).map_or(DVec2::ZERO, |(_, (lo, hi))| *hi - *lo);
    let (front, top, right, iso) = (size(ViewKind::Front), size(ViewKind::Top), size(ViewKind::Right), size(ViewKind::Iso));
    let paper = settings.sheet.size();
    let inner = paper - DVec2::splat(2.0 * BORDER);
    let (left, top_edge) = (BORDER + GAP, paper.y - BORDER - GAP);
    // The iso goes right of the front view, above the title block, at the
    // sheet's scale or smaller (Fusion's can have its own).
    let iso_room = |s: f64| DVec2::new(paper.x - BORDER - GAP - (left + (right.x + front.x) * s + 2.0 * GAP), top_edge - (BORDER + TITLE_SIZE.y + GAP));
    let iso_scale_at = |s: f64| SCALES.into_iter().filter(|&k| k <= s).find(|&k| (iso * k).cmple(iso_room(s)).all()).unwrap_or(SCALES[SCALES.len() - 1]);
    // The front, top and right views share the sheet's scale; it's only
    // taken if the iso then fits at most one standard step smaller.
    let fits = |s: f64| {
        let wide = (right.x + front.x) * s + 3.0 * GAP <= inner.x;
        let left_tall = (front.y + top.y) * s + 3.0 * GAP <= inner.y;
        // The top view, under the front, mustn't run into the title block.
        let clear = right.x * s + front.x.max(top.x) * s + 2.0 * GAP <= inner.x - TITLE_SIZE.x || top.y * s + 2.0 * GAP + TITLE_SIZE.y <= inner.y - front.y * s;
        let one_step_down = SCALES.into_iter().find(|&k| k < s).unwrap_or(s);
        let iso_ok = iso.x == 0.0 || (iso * one_step_down).cmple(iso_room(s)).all();
        wide && left_tall && clear && iso_ok
    };
    let scale = settings.scale.unwrap_or_else(|| SCALES.into_iter().find(|&s| fits(s)).unwrap_or(SCALES[SCALES.len() - 1]));
    let s = scale;
    let iso_scale = iso_scale_at(s);
    // The front, top and right views as one group, centred in the room left
    // of the iso (and above the title block if it would reach it).
    let (right_gap, top_gap) = (if right.x > 0.0 { GAP } else { 0.0 }, if top.y > 0.0 { GAP } else { 0.0 });
    let group = DVec2::new(right.x * s + right_gap + front.x.max(top.x) * s, front.y * s + top_gap + top.y * s);
    let iso_left = if iso.x > 0.0 { paper.x - BORDER - GAP - iso.x * iso_scale - GAP } else { paper.x - BORDER };
    let (x0, x1) = (BORDER + GAP, iso_left.max(BORDER + GAP + group.x));
    let gx = x0 + ((x1 - x0 - group.x) * 0.5).max(0.0);
    let title_left = paper.x - BORDER - TITLE_SIZE.x;
    let y0 = if gx + group.x > title_left { BORDER + TITLE_SIZE.y + GAP } else { BORDER + GAP };
    let gy = (y0 + ((top_edge - y0 - group.y) * 0.5).max(0.0)).min(top_edge - group.y).max(BORDER + GAP);
    // Lower-left corners of each view's box on paper.
    let front_at = DVec2::new(gx + right.x * s + right_gap, gy + top.y * s + top_gap);
    let corner = |k: ViewKind| match k {
        ViewKind::Front => front_at,
        ViewKind::Top => DVec2::new(front_at.x, front_at.y - GAP - top.y * s),
        ViewKind::Right => DVec2::new(gx, front_at.y),
        ViewKind::Iso => DVec2::new(paper.x - BORDER - GAP - iso.x * iso_scale, top_edge - iso.y * iso_scale),
    };
    let placed = bounds
        .iter()
        .filter(|(k, _)| settings.views.contains(k))
        .map(|&(kind, (lo, _))| {
            let scale = if kind == ViewKind::Iso { iso_scale } else { s };
            Placed { kind, origin: corner(kind) - lo * scale, scale }
        })
        .collect();
    (scale, placed)
}

/// How a line is drawn (ISO 128 widths, mm).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pen {
    /// Visible edges and outlines, the border.
    Thick,
    /// Smooth edges, title block lines.
    Thin,
    /// Hidden edges.
    Dashed,
}

impl Pen {
    pub fn width(self) -> f64 {
        match self {
            Pen::Thick => 0.5,
            Pen::Thin => 0.25,
            Pen::Dashed => 0.35,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Text {
    pub at: DVec2,
    /// Capital height (mm).
    pub height: f64,
    pub text: String,
}

/// A whole sheet, ready to draw or write out.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sheet {
    pub size: DVec2,
    pub lines: Vec<(Pen, Vec<DVec2>)>,
    pub texts: Vec<Text>,
    pub scale: f64,
}

/// Title block values worked out by the app.
pub struct TitleInfo<'a> {
    pub title: &'a str,
    pub drawn_by: &'a str,
    pub material: &'a str,
    pub date: &'a str,
}

/// The sheet: border, views (their lines, from `kernel::hlr_view` at 1:1)
/// and title block.
pub fn sheet(settings: &DrawingSettings, views: &[(ViewKind, ViewLines)], info: &TitleInfo) -> Sheet {
    let paper = settings.sheet.size();
    let bounds: Vec<(ViewKind, (DVec2, DVec2))> = views.iter().filter(|(k, _)| settings.views.contains(k)).filter_map(|(k, v)| Some((*k, v.bounds()?))).collect();
    let (scale, placed) = layout(settings, &bounds);
    let mut out = Sheet { size: paper, scale, ..Default::default() };
    // Border.
    let (lo, hi) = (DVec2::splat(BORDER), paper - DVec2::splat(BORDER));
    out.lines.push((Pen::Thick, vec![lo, DVec2::new(hi.x, lo.y), hi, DVec2::new(lo.x, hi.y), lo]));
    for p in &placed {
        let Some((_, lines)) = views.iter().find(|(k, _)| *k == p.kind) else { continue };
        let on_paper = |l: &Vec<DVec2>| l.iter().map(|q| p.origin + *q * p.scale).collect::<Vec<_>>();
        for l in lines.visible.iter().chain(&lines.outline) {
            out.lines.push((Pen::Thick, on_paper(l)));
        }
        for l in &lines.smooth {
            out.lines.push((Pen::Thin, on_paper(l)));
        }
        if settings.hidden_lines && p.kind != ViewKind::Iso {
            for l in &lines.hidden {
                out.lines.push((Pen::Dashed, on_paper(l)));
            }
        }
        // A view at a scale of its own says so underneath.
        if p.scale != scale {
            if let Some((lo, hi)) = lines.bounds() {
                let at = p.origin + DVec2::new((lo.x + hi.x) * 0.5 * p.scale, lo.y * p.scale - 6.0);
                out.texts.push(Text { at: at - DVec2::new(10.0, 0.0), height: 2.5, text: format!("Scale {}", scale_text(p.scale)) });
            }
        }
    }
    title_block(&mut out, settings, info, hi.x, lo.y);
    out
}

/// ISO 7200-style title block with its bottom-right corner at (right, bottom).
fn title_block(out: &mut Sheet, settings: &DrawingSettings, info: &TitleInfo, right: f64, bottom: f64) {
    let (w, h) = (TITLE_SIZE.x, TITLE_SIZE.y);
    let x0 = right - w;
    let rect = |a: DVec2, b: DVec2| vec![a, DVec2::new(b.x, a.y), b, DVec2::new(a.x, b.y), a];
    out.lines.push((Pen::Thick, rect(DVec2::new(x0, bottom), DVec2::new(right, bottom + h))));
    // Rows: title (tall) on top, then two rows of small fields.
    let row = h / 3.0;
    for k in 1..3 {
        out.lines.push((Pen::Thin, vec![DVec2::new(x0, bottom + row * k as f64), DVec2::new(right, bottom + row * k as f64)]));
    }
    let cols = [0.0, 45.0, 90.0, 135.0, 180.0];
    for &c in &cols[1..4] {
        out.lines.push((Pen::Thin, vec![DVec2::new(x0 + c, bottom), DVec2::new(x0 + c, bottom + 2.0 * row)]));
    }
    let mut field = |col: usize, r: usize, label: &str, value: &str| {
        let at = DVec2::new(x0 + cols[col] + 1.5, bottom + row * r as f64);
        out.texts.push(Text { at: at + DVec2::new(0.0, row - 3.2), height: 1.8, text: label.to_string() });
        out.texts.push(Text { at: at + DVec2::new(0.0, 2.0), height: 3.5, text: value.to_string() });
    };
    field(0, 2, "Title", info.title);
    field(0, 1, "Drawn by", info.drawn_by);
    field(1, 1, "Date", info.date);
    field(2, 1, "Material", info.material);
    field(3, 1, "Units", "mm");
    field(0, 0, "Scale", &scale_text(out.scale));
    field(1, 0, "Size", settings.sheet.name());
    field(2, 0, "Sheet", "1 / 1");
    field(3, 0, "Projection", "");
    first_angle_symbol(out, DVec2::new(x0 + cols[3] + 30.0, bottom + row * 0.42));
}

/// The ISO first-angle projection symbol (a cone's side and end views).
fn first_angle_symbol(out: &mut Sheet, centre: DVec2) {
    let s = 2.2;
    // Side view of a truncated cone (to the left in first angle: the end view is on the right).
    let side = centre - DVec2::new(6.0, 0.0);
    out.lines.push((Pen::Thin, vec![side + DVec2::new(-3.0, -s), side + DVec2::new(3.0, -2.0 * s), side + DVec2::new(3.0, 2.0 * s), side + DVec2::new(-3.0, s), side + DVec2::new(-3.0, -s)]));
    let end = centre + DVec2::new(5.0, 0.0);
    for r in [s, 2.0 * s] {
        out.lines.push((Pen::Thin, (0..=32).map(|i| end + DVec2::from_angle(i as f64 / 32.0 * std::f64::consts::TAU) * r).collect()));
    }
}

/// Today's date as YYYY-MM-DD (from the clock, UTC).
pub fn today() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    civil_date(secs as i64 / 86400)
}

/// Days since 1970-01-01 as YYYY-MM-DD.
fn civil_date(days: i64) -> String {
    // Howard Hinnant's days_from_civil, inverted.
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect_view(w: f64, h: f64) -> ViewLines {
        let (a, b, c, d) = (DVec2::ZERO, DVec2::new(w, 0.0), DVec2::new(w, h), DVec2::new(0.0, h));
        ViewLines { visible: vec![vec![a, b], vec![b, c], vec![c, d], vec![d, a]], ..Default::default() }
    }

    /// A box x by y by z seen the four ways (iso as its rough outline).
    fn box_views(x: f64, y: f64, z: f64) -> Vec<(ViewKind, ViewLines)> {
        vec![(ViewKind::Front, rect_view(x, z)), (ViewKind::Top, rect_view(x, y)), (ViewKind::Right, rect_view(y, z)), (ViewKind::Iso, rect_view((x + y) * 0.71, z + (x + y) * 0.41))]
    }

    fn info() -> TitleInfo<'static> {
        TitleInfo { title: "Bracket", drawn_by: "", material: "Steel", date: "2026-10-28" }
    }

    #[test]
    fn the_largest_standard_scale_that_fits_is_picked() {
        let s = DrawingSettings::default();
        assert_eq!(sheet(&s, &box_views(40.0, 30.0, 20.0), &info()).scale, 2.0, "a small part is drawn 2:1 on A3");
        let big = sheet(&s, &box_views(300.0, 200.0, 100.0), &info());
        assert_eq!(big.scale, 0.5, "a big one 1:2");
        assert!(big.texts.iter().any(|t| t.text == "Scale 1:5"), "the iso smaller, labelled");
        let a4 = DrawingSettings { sheet: SheetSize::A4, ..Default::default() };
        assert_eq!(sheet(&a4, &box_views(300.0, 200.0, 100.0), &info()).scale, 0.2, "smaller paper, smaller scale");
        let fixed = DrawingSettings { scale: Some(1.0), ..Default::default() };
        assert_eq!(sheet(&fixed, &box_views(40.0, 30.0, 20.0), &info()).scale, 1.0, "a chosen scale is kept");
        assert_eq!((scale_text(2.0), scale_text(1.0), scale_text(0.2)), ("2:1".to_string(), "1:1".to_string(), "1:5".to_string()));
    }

    #[test]
    fn views_are_in_first_angle_places_and_on_the_sheet() {
        let s = DrawingSettings::default();
        let views = box_views(120.0, 80.0, 50.0);
        let bounds: Vec<_> = views.iter().map(|(k, v)| (*k, v.bounds().unwrap())).collect();
        let (scale, placed) = layout(&s, &bounds);
        let boxed = |k: ViewKind| {
            let p = placed.iter().find(|p| p.kind == k).unwrap();
            let (lo, hi) = bounds.iter().find(|(v, _)| *v == k).unwrap().1;
            (p.origin + lo * p.scale, p.origin + hi * p.scale)
        };
        let (front, top, right, iso) = (boxed(ViewKind::Front), boxed(ViewKind::Top), boxed(ViewKind::Right), boxed(ViewKind::Iso));
        assert!(top.1.y < front.0.y && (top.0.x - front.0.x).abs() < 1e-9, "top view below the front, lined up");
        assert!(right.1.x < front.0.x && (right.0.y - front.0.y).abs() < 1e-9, "right view to the left, lined up");
        assert!(iso.0.x > front.1.x && iso.1.y > front.0.y, "iso to the right, at the top");
        let all = [front, top, right, iso];
        for i in 0..4 {
            for j in i + 1..4 {
                let (a, b) = (all[i], all[j]);
                assert!(!(a.0.cmplt(b.1).all() && a.1.cmpgt(b.0).all()), "views {i} and {j} overlap");
            }
        }
        let iso_scale = placed.iter().find(|p| p.kind == ViewKind::Iso).unwrap().scale;
        assert!(iso_scale <= scale);
        // A small part's views sit in the middle of their room, not in a corner.
        let small = box_views(2.0, 1.5, 1.0);
        let b: Vec<_> = small.iter().map(|(k, v)| (*k, v.bounds().unwrap())).collect();
        let (k, placed) = layout(&s, &b);
        let right_view = placed.iter().find(|p| p.kind == ViewKind::Right).unwrap();
        let front_view = placed.iter().find(|p| p.kind == ViewKind::Front).unwrap();
        assert!(right_view.origin.x > 80.0, "not against the left border: {}", right_view.origin.x);
        assert_eq!(k, 10.0, "as big as drawings go");
        let mid_y = front_view.origin.y + 1.0 * k * 0.5;
        assert!(mid_y > 100.0 && mid_y < 250.0, "about half way up: {mid_y}");
        let paper = s.sheet.size();
        let title = (DVec2::new(paper.x - BORDER - TITLE_SIZE.x, BORDER), DVec2::new(paper.x - BORDER, BORDER + TITLE_SIZE.y));
        for (lo, hi) in [front, top, right, iso] {
            assert!(lo.cmpge(DVec2::splat(BORDER)).all() && hi.cmple(paper - DVec2::splat(BORDER)).all(), "inside the border: {lo} {hi} at {scale}");
            let overlaps = lo.cmplt(title.1).all() && hi.cmpgt(title.0).all();
            assert!(!overlaps, "clear of the title block");
        }
    }

    #[test]
    fn the_sheet_has_border_title_and_dashed_hidden_lines_but_not_in_iso() {
        let mut views = box_views(40.0, 30.0, 20.0);
        for (_, v) in &mut views {
            v.hidden.push(vec![DVec2::new(1.0, 1.0), DVec2::new(2.0, 2.0)]);
        }
        let s = sheet(&DrawingSettings::default(), &views, &info());
        assert_eq!(s.lines.iter().filter(|(p, _)| *p == Pen::Dashed).count(), 3, "front, top and right; not iso");
        assert!(s.texts.iter().any(|t| t.text == "Bracket") && s.texts.iter().any(|t| t.text == "2:1"));
        let none = sheet(&DrawingSettings { hidden_lines: false, ..Default::default() }, &views, &info());
        assert!(none.lines.iter().all(|(p, _)| *p != Pen::Dashed), "hidden lines off");
        let front_only = sheet(&DrawingSettings { views: vec![ViewKind::Front], ..Default::default() }, &views, &info());
        assert_eq!(front_only.lines.iter().filter(|(p, _)| *p == Pen::Dashed).count(), 1);
    }

    #[test]
    fn the_iso_is_at_most_one_scale_step_smaller() {
        // A long flat part: the views at 2:1 would leave the iso a corner.
        let views = box_views(110.0, 60.0, 15.0);
        let bounds: Vec<_> = views.iter().map(|(k, v)| (*k, v.bounds().unwrap())).collect();
        let (scale, placed) = layout(&DrawingSettings::default(), &bounds);
        let iso = placed.iter().find(|p| p.kind == ViewKind::Iso).unwrap().scale;
        let one_down = SCALES.into_iter().find(|&k| k < scale).unwrap();
        assert!(iso >= one_down, "sheet {scale}, iso {iso}");
    }

    #[test]
    fn bodies_and_components_are_picked_and_dropped() {
        use crate::doc::Feature;
        let b = |k| BodyId { feature: Feature::Extrude(k), piece: 0 };
        let mut chosen = Vec::new();
        pick_bodies(&mut chosen, &[b(0), b(1)], true);
        pick_bodies(&mut chosen, &[b(2)], true);
        pick_bodies(&mut chosen, &[b(1)], true);
        assert_eq!(chosen, [b(0), b(2), b(1)], "no body twice");
        pick_bodies(&mut chosen, &[b(0), b(1)], false);
        assert_eq!(chosen, [b(2)]);
        pick_bodies(&mut chosen, &[b(2)], false);
        assert!(chosen.is_empty(), "nothing picked: the whole design");
    }

    #[test]
    fn dates_are_counted_right() {
        assert_eq!(civil_date(0), "1970-01-01");
        assert_eq!(civil_date(11016), "2000-02-29");
        assert_eq!(civil_date(20754), "2026-10-28");
    }
}
