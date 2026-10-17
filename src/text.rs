//! Sketch text: a font's letter outlines as closed contours, laid out on a
//! baseline. The fonts are the machine's own (from fontconfig's `fc-list`,
//! as Fusion lists the system's) plus two that ship inside egui, so text
//! always has something to draw with.

use ab_glyph::{Font, FontVec, OutlineCurve};
use glam::DVec2;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

/// Bundled fonts (inside egui), always there.
pub const UBUNTU: &str = "Ubuntu Light";
pub const HACK: &str = "Hack";

/// The font new text uses: Arial's metric twin when the machine has it
/// (Fusion's default is Arial), else the bundled one.
pub fn default_family() -> &'static str {
    if families().iter().any(|f| f == "Liberation Sans") { "Liberation Sans" } else { UBUNTU }
}

/// One closed outline: its corners in order, and per segment (from
/// `pts[i]` to `pts[i + 1]`, the last back to the first) 0 for a straight
/// piece or the number of the curve it's part of (equal numbers in a row
/// are one curve, flattened).
#[derive(Clone, Debug, PartialEq)]
pub struct Contour {
    pub pts: Vec<DVec2>,
    pub curve: Vec<u32>,
}

/// Font files by family, then style ("Regular", "Bold", …).
fn installed() -> &'static HashMap<String, HashMap<String, String>> {
    static FONTS: OnceLock<HashMap<String, HashMap<String, String>>> = OnceLock::new();
    FONTS.get_or_init(|| {
        let mut out: HashMap<String, HashMap<String, String>> = HashMap::new();
        let Ok(o) = std::process::Command::new("fc-list").args(["--format", "%{family[0]}\t%{style[0]}\t%{file}\n"]).output() else {
            return out;
        };
        for line in String::from_utf8_lossy(&o.stdout).lines() {
            let mut f = line.split('\t');
            let (Some(family), Some(style), Some(file)) = (f.next(), f.next(), f.next()) else { continue };
            // Plain TrueType/OpenType files only; variable fonts ("[wght]") overlap
            // their own outlines, which would make letters into many profiles.
            let lower = file.to_lowercase();
            if !(lower.ends_with(".ttf") || lower.ends_with(".otf")) || file.contains('[') {
                continue;
            }
            out.entry(family.to_string()).or_default().entry(style.to_string()).or_insert_with(|| file.to_string());
        }
        out
    })
}

/// The families to choose from, sorted (the bundled ones included).
pub fn families() -> &'static Vec<String> {
    static LIST: OnceLock<Vec<String>> = OnceLock::new();
    LIST.get_or_init(|| {
        let mut v: Vec<String> = installed().keys().filter(|f| f.chars().next().is_some_and(|c| c.is_ascii_alphanumeric())).cloned().collect();
        v.extend([UBUNTU.to_string(), HACK.to_string()]);
        v.sort_by_key(|f| f.to_lowercase());
        v.dedup();
        v
    })
}

/// The font for a family and style, loaded once. An unknown family (a file
/// from another machine) falls back to the bundled font.
fn font(family: &str, bold: bool, italic: bool) -> Arc<FontVec> {
    thread_local! {
        static LOADED: std::cell::RefCell<HashMap<String, Arc<FontVec>>> = std::cell::RefCell::new(HashMap::new());
    }
    let want = match (bold, italic) {
        (true, true) => ["Bold Italic", "Bold Oblique", "Bold", "Italic"],
        (true, false) => ["Bold", "Semibold", "Bold", "Bold"],
        (false, true) => ["Italic", "Oblique", "Regular", "Regular"],
        (false, false) => ["Regular", "Book", "Roman", "Normal"],
    };
    let file = installed().get(family).and_then(|styles| want.iter().find_map(|s| styles.get(*s)).or_else(|| styles.values().next())).cloned();
    let key = file.clone().unwrap_or_else(|| family.to_string());
    if let Some(f) = LOADED.with(|c| c.borrow().get(&key).cloned()) {
        return f;
    }
    let bundled = || {
        let bytes = if family == HACK { epaint_default_fonts::HACK_REGULAR } else { epaint_default_fonts::UBUNTU_LIGHT };
        FontVec::try_from_vec(bytes.to_vec()).expect("egui's bundled font loads")
    };
    let f = Arc::new(file.and_then(|p| std::fs::read(p).ok()).and_then(|b| FontVec::try_from_vec(b).ok()).unwrap_or_else(bundled));
    LOADED.with(|c| c.borrow_mut().insert(key, f.clone()));
    f
}

/// How many straight pieces a curve of the font is drawn with.
const QUAD_PIECES: usize = 8;
const CUBIC_PIECES: usize = 12;

/// `text` in `family`, its capital letters `height` tall (the CAD
/// convention), starting at the origin on the baseline; more lines go down.
pub fn layout(text: &str, family: &str, bold: bool, italic: bool, height: f64) -> Vec<Contour> {
    thread_local! {
        static LAID: std::cell::RefCell<Vec<((String, String, bool, bool), Vec<Contour>)>> = const { std::cell::RefCell::new(Vec::new()) };
    }
    let key = (text.to_string(), family.to_string(), bold, italic);
    // Laid out once in font units (it's asked for every frame), then scaled.
    let unit = LAID.with(|c| c.borrow().iter().find(|(k, _)| *k == key).map(|(_, v)| v.clone())).unwrap_or_else(|| {
        let v = layout_units(text, &font(family, bold, italic));
        LAID.with(|c| {
            let mut c = c.borrow_mut();
            if c.len() > 32 {
                c.remove(0);
            }
            c.push((key, v.clone()));
        });
        v
    });
    unit.into_iter().map(|c| Contour { pts: c.pts.iter().map(|p| *p * height).collect(), curve: c.curve }).collect()
}

/// The layout with capitals 1 tall.
fn layout_units(text: &str, f: &FontVec) -> Vec<Contour> {
    // Cap height: how tall an H's outline is (its stored box can be a little bigger).
    let cap = f
        .outline(f.glyph_id('H'))
        .map(|o| {
            let ys = o.curves.iter().flat_map(|c| match *c {
                OutlineCurve::Line(a, b) => vec![a.y, b.y],
                OutlineCurve::Quad(a, _, b) => vec![a.y, b.y],
                OutlineCurve::Cubic(a, _, _, b) => vec![a.y, b.y],
            });
            let (lo, hi) = ys.fold((f32::MAX, f32::MIN), |(lo, hi), y| (lo.min(y), hi.max(y)));
            (hi - lo) as f64
        })
        .filter(|h| *h > 0.0)
        .unwrap_or(0.7 * f.units_per_em().unwrap_or(1000.0) as f64);
    let line = (f.ascent_unscaled() - f.descent_unscaled() + f.line_gap_unscaled()) as f64;
    let mut out = Vec::new();
    let mut curve_no = 0u32;
    for (row, text_line) in text.lines().enumerate() {
        let mut x = 0.0f64;
        let mut prev = None;
        for ch in text_line.chars() {
            let id = f.glyph_id(ch);
            if let Some(p) = prev {
                x += f.kern_unscaled(p, id) as f64;
            }
            let origin = DVec2::new(x, -(row as f64) * line);
            if let Some(o) = f.outline(id) {
                out.extend(contours(&o.curves, origin, &mut curve_no));
            }
            x += f.h_advance_unscaled(id) as f64;
            prev = Some(id);
        }
    }
    for c in &mut out {
        for p in &mut c.pts {
            *p /= cap;
        }
    }
    out
}

/// A glyph's curves (one flat list) split into closed contours, flattened.
fn contours(curves: &[OutlineCurve], origin: DVec2, curve_no: &mut u32) -> Vec<Contour> {
    let pt = |p: ab_glyph::Point| origin + DVec2::new(p.x as f64, p.y as f64);
    let mut out: Vec<Contour> = Vec::new();
    let mut cur = Contour { pts: Vec::new(), curve: Vec::new() };
    let mut last: Option<DVec2> = None;
    let close = |cur: &mut Contour, out: &mut Vec<Contour>| {
        let mut c = std::mem::replace(cur, Contour { pts: Vec::new(), curve: Vec::new() });
        // The last piece ends where the first began: drop the repeat.
        if c.pts.len() > 1 && c.pts[0].distance(*c.pts.last().expect("non-empty")) < 1e-9 {
            c.pts.pop();
        } else if !c.pts.is_empty() {
            c.curve.push(0); // (not closed in the font: close it straight)
        }
        if c.pts.len() >= 3 {
            out.push(c);
        }
    };
    for k in curves {
        let (start, pieces): (DVec2, Vec<DVec2>) = match *k {
            OutlineCurve::Line(a, b) => (pt(a), vec![pt(b)]),
            OutlineCurve::Quad(a, c, b) => {
                let (a, c, b) = (pt(a), pt(c), pt(b));
                (a, (1..=QUAD_PIECES).map(|i| { let t = i as f64 / QUAD_PIECES as f64; a * (1.0 - t) * (1.0 - t) + c * 2.0 * t * (1.0 - t) + b * t * t }).collect())
            }
            OutlineCurve::Cubic(a, c1, c2, b) => {
                let (a, c1, c2, b) = (pt(a), pt(c1), pt(c2), pt(b));
                (a, (1..=CUBIC_PIECES).map(|i| {
                    let t = i as f64 / CUBIC_PIECES as f64;
                    let u = 1.0 - t;
                    a * u * u * u + c1 * 3.0 * u * u * t + c2 * 3.0 * u * t * t + b * t * t * t
                }).collect())
            }
        };
        // A curve that doesn't start where the last ended begins a new contour.
        if last.is_some_and(|l| l.distance(start) > 1e-9) {
            close(&mut cur, &mut out);
        }
        if cur.pts.is_empty() {
            cur.pts.push(start);
        }
        let tag = if matches!(k, OutlineCurve::Line(..)) {
            0
        } else {
            *curve_no += 1;
            *curve_no
        };
        for p in pieces {
            cur.curve.push(tag);
            cur.pts.push(p);
        }
        last = cur.pts.last().copied();
    }
    close(&mut cur, &mut out);
    out
}

/// Signed area of a closed outline (counter-clockwise positive).
#[cfg(test)]
fn area(pts: &[DVec2]) -> f64 {
    (0..pts.len()).map(|i| pts[i].perp_dot(pts[(i + 1) % pts.len()])).sum::<f64>() / 2.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds(cs: &[Contour]) -> (DVec2, DVec2) {
        cs.iter().flat_map(|c| &c.pts).fold((DVec2::splat(f64::MAX), DVec2::splat(f64::MIN)), |(lo, hi), p| (lo.min(*p), hi.max(*p)))
    }

    #[test]
    fn capitals_are_as_tall_as_asked_and_sit_on_the_baseline() {
        let h = layout("H", UBUNTU, false, false, 10.0);
        let (lo, hi) = bounds(&h);
        assert!(lo.y.abs() < 1e-9 && (hi.y - 10.0).abs() < 1e-9, "{lo} {hi}");
        assert_eq!(h.len(), 1, "an H is one outline");
        assert!(h[0].curve.iter().all(|&c| c == 0), "all straight");
        assert_eq!(h[0].pts.len(), h[0].curve.len(), "one tag per segment, closed");
    }

    #[test]
    fn an_o_is_two_curved_outlines_one_inside_the_other() {
        let o = layout("O", UBUNTU, false, false, 10.0);
        assert_eq!(o.len(), 2);
        // (Ubuntu's O has short straight sides; the rest is curves.)
        let (curved, all) = o.iter().flat_map(|c| &c.curve).fold((0, 0), |(c, a), &k| (c + (k > 0) as usize, a + 1));
        assert!(curved * 10 > all * 8, "mostly curves: {curved} of {all}");
        let (a, b) = (area(&o[0].pts).abs(), area(&o[1].pts).abs());
        let (big, small) = if a > b { (&o[0], &o[1]) } else { (&o[1], &o[0]) };
        let (blo, bhi) = bounds(std::slice::from_ref(big));
        let (slo, shi) = bounds(std::slice::from_ref(small));
        assert!(slo.x > blo.x && slo.y > blo.y && shi.x < bhi.x && shi.y < bhi.y, "the hole is inside");
        // Curves are numbered: pieces of one curve share a number, the next curve has another.
        let c = &o[0].curve;
        assert!(c.windows(QUAD_PIECES).any(|w| w.iter().all(|&k| k == w[0])));
        assert!(c.iter().collect::<std::collections::HashSet<_>>().len() > 1);
    }

    #[test]
    fn letters_follow_on_and_lines_go_down() {
        let one = bounds(&layout("H", UBUNTU, false, false, 10.0));
        let two = bounds(&layout("HH", UBUNTU, false, false, 10.0));
        assert!(two.1.x > one.1.x * 1.8, "the second H comes after the first");
        let rows = bounds(&layout("H\nH", UBUNTU, false, false, 10.0));
        assert!(rows.0.y < -10.0, "the second line is below the first");
        assert!(layout(" ", UBUNTU, false, false, 10.0).is_empty());
    }

    #[test]
    fn an_unknown_font_falls_back_to_the_bundled_one() {
        assert_eq!(layout("A", "No Such Font 123", false, false, 5.0), layout("A", UBUNTU, false, false, 5.0));
        assert!(families().iter().any(|f| f == UBUNTU) && families().iter().any(|f| f == HACK));
    }
}
