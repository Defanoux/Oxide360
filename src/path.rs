//! A chain of sketch curves as a path to measure along: its length, and the
//! point and direction any distance along it (lines and arcs exactly,
//! splines by their drawn points). Pattern on Path places its copies with it.

use crate::sketch::{Entity, Sketch};
use glam::DVec2;

#[derive(Clone, Debug, PartialEq)]
enum Piece {
    Line { a: DVec2, b: DVec2 },
    /// From angle `a0`, turning `sweep` radians (negative: clockwise).
    Arc { c: DVec2, r: f64, a0: f64, sweep: f64 },
    Poly(Vec<DVec2>),
}

impl Piece {
    fn length(&self) -> f64 {
        match self {
            Piece::Line { a, b } => a.distance(*b),
            Piece::Arc { r, sweep, .. } => r * sweep.abs(),
            Piece::Poly(p) => p.windows(2).map(|w| w[0].distance(w[1])).sum(),
        }
    }

    fn reversed(self) -> Piece {
        match self {
            Piece::Line { a, b } => Piece::Line { a: b, b: a },
            Piece::Arc { c, r, a0, sweep } => Piece::Arc { c, r, a0: a0 + sweep, sweep: -sweep },
            Piece::Poly(mut p) => {
                p.reverse();
                Piece::Poly(p)
            }
        }
    }

    /// The point and unit direction `s` mm along it (0 ..= length).
    fn at(&self, s: f64) -> (DVec2, DVec2) {
        match self {
            Piece::Line { a, b } => {
                let d = (*b - *a).normalize_or_zero();
                (*a + d * s, d)
            }
            Piece::Arc { c, r, a0, sweep } => {
                let turn = sweep.signum();
                let ang = a0 + turn * s / r;
                let out = DVec2::from_angle(ang);
                (*c + out * *r, DVec2::new(-out.y, out.x) * turn)
            }
            Piece::Poly(p) => {
                let mut left = s;
                for (k, w) in p.windows(2).enumerate() {
                    let l = w[0].distance(w[1]);
                    if left <= l || k + 2 == p.len() {
                        let d = (w[1] - w[0]).normalize_or_zero();
                        return (w[0] + d * left.min(l), d);
                    }
                    left -= l;
                }
                (p.first().copied().unwrap_or(DVec2::ZERO), DVec2::X)
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Path {
    pieces: Vec<Piece>,
    /// It ends where it starts (copies carry on round).
    pub closed: bool,
}

impl Path {
    /// The path through `entities` of `sk` (lines, arcs and splines joined
    /// end to end, or one circle), from the same end a Sweep starts at.
    pub fn of(sk: &Sketch, entities: &[usize]) -> Result<Path, String> {
        if entities.is_empty() {
            return Err("no path picked".into());
        }
        if let [e] = entities {
            if let Some(&Entity::Circle { center, radius }) = sk.entities.get(*e) {
                return Ok(Path { pieces: vec![Piece::Arc { c: sk.points[center], r: radius, a0: 0.0, sweep: std::f64::consts::TAU }], closed: true });
            }
        }
        // Each curve with the sketch points it runs between.
        let mut left: Vec<(usize, usize, Piece)> = Vec::new();
        for &e in entities {
            left.push(match sk.entities.get(e) {
                Some(&Entity::Line { a, b }) => (a, b, Piece::Line { a: sk.points[a], b: sk.points[b] }),
                Some(&Entity::Arc { a, b, .. }) => {
                    let (c, r, a0, sweep) = sk.arc_geometry(e).ok_or("an arc of the path has no size")?;
                    (a, b, Piece::Arc { c, r, a0, sweep })
                }
                Some(&Entity::Spline { s }) => {
                    let list = sk.splines.get(s).ok_or("a spline of the path is damaged")?;
                    (list[0], *list.last().ok_or("a spline of the path is damaged")?, Piece::Poly(sk.spline_curve(s)))
                }
                _ => return Err("a path is made of lines, arcs and splines (or one circle)".into()),
            });
        }
        let uses = |p: usize, ps: &[(usize, usize, Piece)]| ps.iter().filter(|q| q.0 == p || q.1 == p).count();
        let start = left.iter().flat_map(|q| [q.0, q.1]).find(|&p| uses(p, &left) == 1).unwrap_or(left[0].0);
        let mut at = start;
        let mut pieces = Vec::new();
        while !left.is_empty() {
            let Some(k) = left.iter().position(|q| q.0 == at || q.1 == at) else { return Err("the path's curves don't join end to end".into()) };
            let (a, b, piece) = left.remove(k);
            pieces.push(if a == at { piece } else { piece.reversed() });
            at = if a == at { b } else { a };
        }
        Ok(Path { closed: at == start && pieces.len() > 1, pieces })
    }

    /// The same path walked from its other end.
    pub fn reversed(self) -> Path {
        Path { pieces: self.pieces.into_iter().rev().map(Piece::reversed).collect(), closed: self.closed }
    }

    pub fn length(&self) -> f64 {
        self.pieces.iter().map(Piece::length).sum()
    }

    /// The point and unit direction `s` mm from its start (round again on a
    /// closed path; held at the ends of an open one).
    pub fn at(&self, s: f64) -> (DVec2, DVec2) {
        let total = self.length();
        let mut s = if self.closed && total > 0.0 { s.rem_euclid(total) } else { s.clamp(0.0, total) };
        for (k, p) in self.pieces.iter().enumerate() {
            let l = p.length();
            if s <= l || k + 1 == self.pieces.len() {
                return p.at(s.min(l));
            }
            s -= l;
        }
        (DVec2::ZERO, DVec2::X)
    }
}

/// How Pattern on Path spreads its copies.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spread {
    /// How many, the original included.
    pub count: u32,
    /// Between one copy and the next (`spacing`), or from the first to the last.
    pub distance: f64,
    pub spacing: bool,
    /// Where on the path the original counts as being (0 its start, 1 its end).
    pub start: f64,
    /// Copies both ways from the original.
    pub symmetric: bool,
}

impl Spread {
    /// The gap from one copy to the next.
    pub fn step(&self) -> f64 {
        if self.spacing || self.count < 2 { self.distance } else { self.distance / (self.count - 1) as f64 }
    }

    /// Where the original is along a path `length` long, and where each
    /// copy goes (distances along the path). An open path too short for
    /// them is an error.
    pub fn stations(&self, length: f64, closed: bool) -> Result<(f64, Vec<f64>), String> {
        if length <= 1e-9 {
            return Err("the path has no length".into());
        }
        let base = self.start.clamp(0.0, 1.0) * length;
        let step = self.step();
        if self.count >= 2 && step.abs() < 1e-9 {
            return Err("the copies would sit on each other — raise the distance".into());
        }
        let mut out = Vec::new();
        for i in 1..self.count {
            out.push(base + step * i as f64);
            if self.symmetric {
                out.push(base - step * i as f64);
            }
        }
        if !closed {
            if let Some(far) = out.iter().copied().find(|s| *s < -1e-6 || *s > length + 1e-6) {
                let room = if far < 0.0 { base } else { length - base };
                return Err(format!("the path is too short: {room:.2} mm from the start point, the copies need {:.2} mm", step.abs() * (self.count - 1) as f64));
            }
        }
        Ok((base, out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::{FRAC_PI_2, PI};

    fn near(a: DVec2, b: DVec2) -> bool {
        a.distance(b) < 1e-9
    }

    /// 10 along +X, a quarter turn of radius 5 up to (15, 5), then 10 along +Y.
    fn hook() -> Path {
        Path {
            pieces: vec![
                Piece::Line { a: DVec2::ZERO, b: DVec2::new(10.0, 0.0) },
                Piece::Arc { c: DVec2::new(10.0, 5.0), r: 5.0, a0: -FRAC_PI_2, sweep: FRAC_PI_2 },
                Piece::Line { a: DVec2::new(15.0, 5.0), b: DVec2::new(15.0, 15.0) },
            ],
            closed: false,
        }
    }

    #[test]
    fn length_and_points_along_lines_and_arcs_are_exact() {
        let p = hook();
        let arc = 5.0 * FRAC_PI_2;
        assert!((p.length() - (20.0 + arc)).abs() < 1e-12);
        let (at, dir) = p.at(4.0);
        assert!(near(at, DVec2::new(4.0, 0.0)) && near(dir, DVec2::X));
        // Half way round the corner: 45 degrees.
        let (at, dir) = p.at(10.0 + arc / 2.0);
        let d = std::f64::consts::FRAC_1_SQRT_2;
        assert!(near(at, DVec2::new(10.0 + 5.0 * d, 5.0 - 5.0 * d)), "{at}");
        assert!(near(dir, DVec2::new(d, d)), "{dir}");
        let (at, dir) = p.at(10.0 + arc + 3.0);
        assert!(near(at, DVec2::new(15.0, 8.0)) && near(dir, DVec2::Y));
        // Past the end of an open path: held there.
        assert!(near(p.at(999.0).0, DVec2::new(15.0, 15.0)));
    }

    #[test]
    fn a_path_walks_the_same_backwards() {
        let p = hook();
        let total = p.length();
        let back = p.clone().reversed();
        for s in [0.0, 3.0, 12.0, 17.0, total] {
            let (a, da) = p.at(s);
            let (b, db) = back.at(total - s);
            assert!(near(a, b) && near(da, -db), "{s}: {a} {b}");
        }
    }

    #[test]
    fn a_circle_goes_round_and_round() {
        let p = Path { pieces: vec![Piece::Arc { c: DVec2::ZERO, r: 10.0, a0: 0.0, sweep: 2.0 * PI }], closed: true };
        let quarter = 10.0 * FRAC_PI_2;
        assert!(near(p.at(quarter).0, DVec2::new(0.0, 10.0)));
        assert!(near(p.at(quarter * 5.0).0, DVec2::new(0.0, 10.0)), "once round and a quarter");
        assert!(near(p.at(-quarter).0, DVec2::new(0.0, -10.0)));
    }

    #[test]
    fn a_sketch_chain_is_found_from_its_end_in_any_order() {
        let mut sk = Sketch::new("S".into(), crate::sketch::OriginPlane::XY);
        sk.points.extend([DVec2::new(10.0, 0.0), DVec2::new(10.0, 7.0)]);
        let (a, b, c) = (0, 1, 2);
        sk.entities.push(Entity::Line { a: c, b }); // (drawn backwards, listed first)
        sk.entities.push(Entity::Line { a, b });
        let p = Path::of(&sk, &[0, 1]).unwrap();
        assert!(!p.closed && (p.length() - 17.0).abs() < 1e-12);
        // It starts at an end, whichever: 10 along is the corner either way... 7 from one end, 10 from the other.
        let ends = [p.at(0.0).0, p.at(17.0).0];
        assert!(ends.iter().any(|e| near(*e, DVec2::ZERO)) && ends.iter().any(|e| near(*e, DVec2::new(10.0, 7.0))), "{ends:?}");
        assert!(Path::of(&sk, &[]).is_err());
    }

    #[test]
    fn copies_are_spaced_or_spread_and_must_fit_an_open_path() {
        let s = Spread { count: 4, distance: 30.0, spacing: false, start: 0.0, symmetric: false };
        assert_eq!(s.stations(100.0, false).unwrap(), (0.0, vec![10.0, 20.0, 30.0]));
        let s = Spread { spacing: true, distance: 10.0, ..s };
        assert_eq!(s.stations(100.0, false).unwrap(), (0.0, vec![10.0, 20.0, 30.0]));
        // From the middle, both ways.
        let s = Spread { count: 2, distance: 10.0, spacing: true, start: 0.5, symmetric: true };
        assert_eq!(s.stations(100.0, false).unwrap(), (50.0, vec![60.0, 40.0]));
        // Too many for the path; fine on a closed one.
        let s = Spread { count: 5, distance: 30.0, spacing: true, start: 0.0, symmetric: false };
        let e = s.stations(100.0, false).unwrap_err();
        assert!(e.contains("too short") && e.contains("120.00"), "{e}");
        assert!(s.stations(100.0, true).is_ok());
        assert!(Spread { distance: 0.0, ..s }.stations(100.0, true).is_err());
    }
}
