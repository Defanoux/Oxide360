//! Sketch Extend and Offset.

use crate::sketch::{Constraint, Entity, EntityId, PointId, Sketch, line_circle};
use glam::DVec2;
use std::f64::consts::TAU;

/// What an offset piece lies on once moved over.
#[derive(Clone, Copy, Debug)]
enum Curve {
    Line { p: DVec2, d: DVec2 },
    Circle { c: DVec2, r: f64 },
}

/// Where two moved-over curves cross.
fn crossings(a: Curve, b: Curve) -> Vec<DVec2> {
    match (a, b) {
        (Curve::Line { p, d }, Curve::Line { p: q, d: e }) => {
            let denom = d.perp_dot(e);
            if denom.abs() < 1e-9 {
                return Vec::new();
            }
            vec![p + d * ((q - p).perp_dot(e) / denom)]
        }
        (Curve::Line { p, d }, Curve::Circle { c, r }) | (Curve::Circle { c, r }, Curve::Line { p, d }) => {
            line_circle(p, d, c, r).into_iter().map(|t| p + d * t).collect()
        }
        (Curve::Circle { c: c1, r: r1 }, Curve::Circle { c: c2, r: r2 }) => {
            let dist = c1.distance(c2);
            if dist < 1e-9 {
                return Vec::new();
            }
            let dir = (c2 - c1) / dist;
            let a = (dist * dist + r1 * r1 - r2 * r2) / (2.0 * dist);
            let h2 = r1 * r1 - a * a;
            if h2 < 0.0 {
                return Vec::new();
            }
            let h = h2.sqrt();
            vec![c1 + dir * a + dir.perp() * h, c1 + dir * a - dir.perp() * h]
        }
    }
}

/// One link of a chain, walked in a direction.
#[derive(Clone, Copy, Debug)]
struct Piece {
    e: EntityId,
    /// Walked from its `a` end to its `b` end.
    forward: bool,
}

impl Sketch {
    /// The end of line `e` nearest `at` runs on to the first other geometry in
    /// its way (a line, circle or arc that really extends that far). The end
    /// moves to a new point, tied onto what it meets (or joined to it, when it
    /// meets another's end). Constraints for the caller to add one by one.
    pub fn extend_line(&mut self, e: EntityId, at: DVec2) -> Result<Vec<Constraint>, String> {
        let Some(&Entity::Line { a, b }) = self.entities.get(e) else {
            return Err("Extend works on lines".into());
        };
        let (pa, pb) = (self.points[a], self.points[b]);
        let d = pb - pa;
        let Some(u) = d.try_normalize() else { return Err("that line has no length".into()) };
        let from_b = (at - pa).dot(d) / d.length_squared() >= 0.5;
        let (start, dir) = if from_b { (pb, u) } else { (pa, -u) };
        let tol = 1e-7;
        // (distance along, point, entity hit, an existing end point there)
        let mut best: Option<(f64, DVec2, EntityId, Option<PointId>)> = None;
        let mut offer = |s: f64, x: DVec2, other: EntityId, join: Option<PointId>| {
            if s > tol && best.is_none_or(|b| s < b.0) {
                best = Some((s, x, other, join));
            }
        };
        let near = |p: PointId, x: DVec2| (self.points[p].distance(x) < 1e-6).then_some(p);
        for (i, ent) in self.entities.iter().enumerate() {
            if i == e {
                continue;
            }
            match *ent {
                Entity::Line { a: c, b: d2 } => {
                    let (pc, pd) = (self.points[c], self.points[d2]);
                    let f = pd - pc;
                    let denom = dir.perp_dot(f);
                    if denom.abs() < 1e-12 * f.length().max(1e-12) {
                        continue;
                    }
                    let s = (pc - start).perp_dot(f) / denom;
                    let w = (pc - start).perp_dot(dir) / denom;
                    let sw = tol / f.length().max(1e-12);
                    if w >= -sw && w <= 1.0 + sw {
                        let x = start + dir * s;
                        offer(s, x, i, near(c, x).or(near(d2, x)));
                    }
                }
                Entity::Circle { center, radius } => {
                    for s in line_circle(start, dir, self.points[center], radius) {
                        offer(s, start + dir * s, i, None);
                    }
                }
                Entity::Arc { a: ea, b: eb, .. } => {
                    let Some((c, r, a0, sweep)) = self.arc_geometry(i) else { continue };
                    for s in line_circle(start, dir, c, r) {
                        let x = start + dir * s;
                        let ang = (x - c).y.atan2((x - c).x);
                        if (ang - a0).rem_euclid(TAU) <= sweep + 1e-9 {
                            offer(s, x, i, near(ea, x).or(near(eb, x)));
                        }
                    }
                }
                Entity::Point { .. } => {}
            }
        }
        let Some((_, x, other, join)) = best else {
            return Err("there is nothing in that direction to extend to".into());
        };
        let mut trial = self.clone();
        // Its length is about to change.
        trial.retain_constraints(|k| match *k {
            Constraint::Length(l, _) => l != e,
            Constraint::Equal(p, q) => p != e && q != e,
            _ => true,
        });
        let (end, tie) = match join {
            Some(p) => (p, None),
            None => {
                trial.points.push(x);
                (trial.points.len() - 1, Some(other))
            }
        };
        trial.entities[e] = if from_b { Entity::Line { a, b: end } } else { Entity::Line { a: end, b } };
        trial.delete(&[]); // the old end, if nothing else uses it
        let mut out = Vec::new();
        if let Some(other) = tie {
            if let Some(p) = trial.points.iter().position(|q| *q == x) {
                out.push(match trial.entities.get(other) {
                    Some(Entity::Line { .. }) => Constraint::OnLine(p, other),
                    _ => Constraint::OnCircle(p, other),
                });
            }
        }
        *self = trial;
        Ok(out)
    }

    /// The lines and arcs joined end to end with `e` in a simple run or loop,
    /// in walking order. (A circle is its own run.)
    fn chain_of(&self, e: EntityId) -> Result<(Vec<Piece>, bool), String> {
        let ends = |i: EntityId| match self.entities[i] {
            Entity::Line { a, b } | Entity::Arc { a, b, .. } => Some((a, b)),
            _ => None,
        };
        if ends(e).is_none() {
            return Ok((vec![Piece { e, forward: true }], false));
        }
        let mut members = vec![e];
        let mut grew = true;
        while grew {
            grew = false;
            for i in 0..self.entities.len() {
                let Some((a, b)) = ends(i) else { continue };
                if members.contains(&i) {
                    continue;
                }
                let touches = members.iter().filter_map(|&m| ends(m)).any(|(p, q)| [p, q].iter().any(|x| (*x == a || *x == b) && *x != Sketch::ORIGIN));
                if touches {
                    members.push(i);
                    grew = true;
                }
            }
        }
        let degree = |p: PointId| members.iter().filter_map(|&m| ends(m)).filter(|&(a, b)| a == p || b == p).count();
        if members.iter().filter_map(|&m| ends(m)).any(|(a, b)| degree(a) > 2 || degree(b) > 2) {
            return Err("offset needs a single run of lines and arcs — a branch splits it".into());
        }
        // Start at a loose end if there is one, else anywhere (a loop).
        let loose = members.iter().find_map(|&m| {
            let (a, b) = ends(m)?;
            if degree(a) == 1 {
                Some((m, true))
            } else if degree(b) == 1 {
                Some((m, false))
            } else {
                None
            }
        });
        let closed = loose.is_none();
        let (first, forward) = loose.unwrap_or((e, true));
        let mut pieces = vec![Piece { e: first, forward }];
        loop {
            let last = *pieces.last().expect("not empty");
            let (a, b) = ends(last.e).expect("line or arc");
            let tail = if last.forward { b } else { a };
            let next = members.iter().find_map(|&m| {
                if pieces.iter().any(|p| p.e == m) {
                    return None;
                }
                let (x, y) = ends(m)?;
                if tail == Sketch::ORIGIN {
                    return None;
                }
                if x == tail {
                    Some(Piece { e: m, forward: true })
                } else if y == tail {
                    Some(Piece { e: m, forward: false })
                } else {
                    None
                }
            });
            match next {
                Some(p) => pieces.push(p),
                None => break,
            }
        }
        if pieces.len() != members.len() {
            return Err("offset needs a single run of lines and arcs".into());
        }
        Ok((pieces, closed))
    }

    /// Which side of the walked piece `at` is on (+1 left, −1 right) and how far from it.
    fn side_of(&self, p: Piece, at: DVec2) -> (f64, f64) {
        match self.entities[p.e] {
            Entity::Line { a, b } => {
                let (pa, pb) = (self.points[a], self.points[b]);
                let d = (pb - pa).normalize_or_zero() * if p.forward { 1.0 } else { -1.0 };
                let s = d.perp_dot(at - pa); // positive: left of d
                (if s >= 0.0 { 1.0 } else { -1.0 }, s.abs())
            }
            Entity::Arc { .. } => {
                let (c, r, _, _) = self.arc_geometry(p.e).expect("arc");
                let off = at.distance(c) - r; // positive: outside
                // Walking counter-clockwise keeps the centre on the left.
                let left_is_inside = p.forward;
                let inside = off < 0.0;
                (if inside == left_is_inside { 1.0 } else { -1.0 }, off.abs())
            }
            Entity::Circle { center, radius } => {
                let off = at.distance(self.points[center]) - radius;
                (if off < 0.0 { 1.0 } else { -1.0 }, off.abs())
            }
            Entity::Point { .. } => (1.0, 0.0),
        }
    }

    /// The curve of a piece moved `s` to its left, and the unit left normal at `at`.
    fn moved(&self, p: Piece, s: f64) -> Result<Curve, String> {
        match self.entities[p.e] {
            Entity::Line { a, b } => {
                let (pa, pb) = (self.points[a], self.points[b]);
                let d = (pb - pa).normalize_or_zero() * if p.forward { 1.0 } else { -1.0 };
                Ok(Curve::Line { p: pa + d.perp() * s, d })
            }
            Entity::Arc { .. } => {
                let (c, r, _, _) = self.arc_geometry(p.e).expect("arc");
                let r2 = if p.forward { r - s } else { r + s };
                if r2 < 1e-6 {
                    return Err("the offset is bigger than an arc's radius".into());
                }
                Ok(Curve::Circle { c, r: r2 })
            }
            _ => Err("not offsettable".into()),
        }
    }

    /// Offset like Fusion: the copy (see `offset_chain`) plus the constraints
    /// that keep it there, for the caller to add one by one. Lines get an
    /// Offset constraint each (one shared value, shown as one dimension);
    /// where an arc met a line smoothly the copies get a Tangent there; an
    /// arc or circle without two such joints gets a radius Offset.
    pub fn offset_constrained(&mut self, e: EntityId, at: DVec2, distance: Option<f64>) -> Result<Vec<Constraint>, String> {
        let (pieces, _) = self.chain_of(e)?;
        let made = self.offset_chain(e, at, distance)?;
        let d = self.offset_between(pieces[0].e, made[0]).ok_or("the copy has no size")?;
        let pairs: Vec<(EntityId, EntityId)> = pieces.iter().map(|p| p.e).zip(made.iter().copied()).collect();
        let ends = |sk: &Sketch, e: EntityId| match sk.entities[e] {
            Entity::Line { a, b } | Entity::Arc { a, b, .. } => vec![a, b],
            _ => Vec::new(),
        };
        let mut out: Vec<Constraint> = pairs.iter().filter(|(o, _)| self.is_line(*o)).map(|&(o, c)| Constraint::Offset(o, c, d)).collect();
        for &(oa, ca) in pairs.iter().filter(|(o, _)| !self.is_line(*o)) {
            let mut smooth = 0;
            for &(ol, cl) in pairs.iter().filter(|(o, _)| self.is_line(*o)) {
                // The originals' shared end, and whether the arc was square to the line there.
                let Some(&j) = ends(self, oa).iter().find(|p| ends(self, ol).contains(p)) else { continue };
                let (Some(&Entity::Arc { center, .. }), Some(&Entity::Line { a, b })) = (self.entities.get(oa), self.entities.get(ol)) else { continue };
                let dir = (self.points[b] - self.points[a]).normalize_or_zero();
                let radial = self.points[j] - self.points[center];
                if radial.dot(dir).abs() <= 1e-6 * radial.length().max(1.0) && ends(self, ca).iter().any(|p| ends(self, cl).contains(p)) {
                    out.push(Constraint::Tangent(cl, ca));
                    smooth += 1;
                }
            }
            if smooth < 2 {
                out.push(Constraint::Offset(oa, ca, d));
            }
        }
        Ok(out)
    }

    /// Offset the run of lines / arcs (or the circle) that `e` is part of. The
    /// side is where `at` is; the distance is how far `at` is from `e`, unless
    /// `distance` says. Returns the new entities. Corners are kept sharp where
    /// the moved pieces cross; the originals stay.
    pub fn offset_chain(&mut self, e: EntityId, at: DVec2, distance: Option<f64>) -> Result<Vec<EntityId>, String> {
        if e >= self.entities.len() || matches!(self.entities[e], Entity::Point { .. }) {
            return Err("Offset works on lines, arcs and circles".into());
        }
        let (pieces, closed) = self.chain_of(e)?;
        let at_piece = pieces.iter().copied().find(|p| p.e == e).expect("e is in its chain");
        let (side, away) = self.side_of(at_piece, at);
        let dist = distance.map_or(away, f64::abs);
        if dist < 1e-9 {
            return Err("move the cursor off the line to set the distance".into());
        }
        // Everything moves to the left of the walk by `s`; the first piece decides the side.
        // `at_piece` is rotated to the walk, so its side is relative to its own direction.
        let s = dist * side;
        let first_new = self.entities.len();
        let construction: Vec<bool> = pieces.iter().map(|p| self.is_construction(p.e)).collect();
        if let Entity::Circle { center, radius } = self.entities[e] {
            let r = if side > 0.0 { radius - dist } else { radius + dist };
            if r < 1e-6 {
                return Err("the offset is bigger than the circle".into());
            }
            // Concentric: the copy shares the centre point.
            self.entities.push(Entity::Circle { center, radius: r });
            self.set_construction(first_new, construction[0]);
            return Ok(vec![first_new]);
        }
        let curves: Vec<Curve> = pieces.iter().map(|&p| self.moved(p, s)).collect::<Result<_, _>>()?;
        let n = pieces.len();
        // Joints: after piece k (and, for a loop, from the last back to the first).
        let walk_end = |p: Piece| match self.entities[p.e] {
            Entity::Line { a, b } | Entity::Arc { a, b, .. } => self.points[if p.forward { b } else { a }],
            _ => DVec2::ZERO,
        };
        let walk_start = |p: Piece| match self.entities[p.e] {
            Entity::Line { a, b } | Entity::Arc { a, b, .. } => self.points[if p.forward { a } else { b }],
            _ => DVec2::ZERO,
        };
        // Where a piece's own end lands when moved straight over (also the
        // answer at a tangent joint, where the curves only graze).
        let straight = |k: usize, at_end: bool| -> DVec2 {
            let p = pieces[k];
            let x = if at_end { walk_end(p) } else { walk_start(p) };
            match curves[k] {
                Curve::Line { d, .. } => x + d.perp() * s,
                Curve::Circle { c, .. } => {
                    let dir = (x - c).normalize_or_zero();
                    let r0 = (x - c).length();
                    x + dir * (if p.forward { -s } else { s }) * if r0 > 0.0 { 1.0 } else { 0.0 }
                }
            }
        };
        let joint = |k: usize, l: usize| -> DVec2 {
            let j = walk_end(pieces[k]);
            crossings(curves[k], curves[l])
                .into_iter()
                .min_by(|x, y| x.distance(j).total_cmp(&y.distance(j)))
                .filter(|x| x.distance(j) < 4.0 * dist + 1e-9 + 1e-3 * j.length())
                .unwrap_or_else(|| straight(k, true))
        };
        let mut joints: Vec<DVec2> = Vec::new();
        if !closed {
            joints.push(straight(0, false));
        }
        for k in 0..n {
            if k + 1 < n {
                joints.push(joint(k, k + 1));
            } else if closed {
                joints.insert(0, joint(k, 0));
            } else {
                joints.push(straight(k, true));
            }
        }
        // `joints[k]` starts piece k and `joints[k + 1]` (wrapping for a loop) ends it.
        let base = self.points.len();
        self.points.extend(joints.iter().copied());
        let pt = |i: usize| base + (i % joints.len());
        let mut made = Vec::new();
        for (k, p) in pieces.iter().enumerate() {
            let (from, to) = (pt(k), pt(k + 1));
            let (a, b) = if p.forward { (from, to) } else { (to, from) };
            let ent = match self.entities[p.e] {
                Entity::Line { .. } => Entity::Line { a, b },
                // Concentric: the copy shares the centre point.
                Entity::Arc { center, .. } => Entity::Arc { center, a, b },
                other => other,
            };
            self.entities.push(ent);
            let id = self.entities.len() - 1;
            self.set_construction(id, construction[k]);
            made.push(id);
        }
        Ok(made)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sketch::{Anchor, Editor, OriginPlane, Tool};

    fn lines(segs: &[((f64, f64), (f64, f64))]) -> Sketch {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        let mut ed = Editor::default();
        ed.set_tool(Tool::Line);
        for &(a, b) in segs {
            ed.cancel();
            ed.click(&mut sk, Anchor::Free(DVec2::new(a.0, a.1)));
            ed.click(&mut sk, Anchor::Free(DVec2::new(b.0, b.1)));
        }
        sk
    }

    fn ends(sk: &Sketch, e: EntityId) -> (DVec2, DVec2) {
        let Entity::Line { a, b } = sk.entities[e] else { panic!("not a line") };
        (sk.points[a], sk.points[b])
    }

    #[test]
    fn extend_runs_a_line_up_to_the_next_line() {
        let mut sk = lines(&[((0.0, 0.0), (4.0, 0.0)), ((10.0, -5.0), (10.0, 5.0))]);
        let cs = sk.extend_line(0, DVec2::new(3.5, 0.0)).unwrap();
        let (a, b) = ends(&sk, 0);
        assert!(a.distance(DVec2::ZERO) < 1e-12 && b.distance(DVec2::new(10.0, 0.0)) < 1e-12);
        assert!(matches!(cs.as_slice(), [Constraint::OnLine(_, 1)]));
    }

    #[test]
    fn extend_picks_the_end_nearest_the_click() {
        let mut sk = lines(&[((0.0, 0.0), (4.0, 0.0)), ((-6.0, -5.0), (-6.0, 5.0))]);
        sk.extend_line(0, DVec2::new(0.2, 0.0)).unwrap();
        let (a, b) = ends(&sk, 0);
        assert!(a.distance(DVec2::new(-6.0, 0.0)) < 1e-12 && b.distance(DVec2::new(4.0, 0.0)) < 1e-12);
    }

    #[test]
    fn extend_stops_at_a_circle_and_ignores_lines_that_fall_short() {
        let mut sk = lines(&[((0.0, 0.0), (1.0, 0.0)), ((5.0, 1.0), (5.0, 3.0))]);
        sk.points.push(DVec2::new(8.0, 0.0));
        let c = sk.points.len() - 1;
        sk.entities.push(Entity::Circle { center: c, radius: 2.0 });
        let cs = sk.extend_line(0, DVec2::new(0.9, 0.0)).unwrap();
        assert!(ends(&sk, 0).1.distance(DVec2::new(6.0, 0.0)) < 1e-9);
        assert!(matches!(cs.as_slice(), [Constraint::OnCircle(_, 2)]));
    }

    #[test]
    fn extend_with_nothing_ahead_is_an_error() {
        let mut sk = lines(&[((0.0, 0.0), (1.0, 0.0)), ((5.0, 1.0), (5.0, 3.0))]);
        assert!(sk.extend_line(0, DVec2::new(0.9, 0.0)).is_err());
    }

    #[test]
    fn extend_joins_at_the_far_lines_end() {
        let mut sk = lines(&[((0.0, 0.0), (1.0, 0.0)), ((5.0, 0.0), (5.0, 3.0))]);
        let before = sk.points.len();
        let cs = sk.extend_line(0, DVec2::new(0.9, 0.0)).unwrap();
        assert!(cs.is_empty());
        assert_eq!(sk.points.len(), before - 1); // the old end went, no new point
        let (_, b) = ends(&sk, 0);
        let (c, _) = ends(&sk, 1);
        assert_eq!(b, c);
    }

    #[test]
    fn offset_a_line_moves_it_over_to_the_cursor_side() {
        let mut sk = lines(&[((0.0, 0.0), (10.0, 0.0))]);
        let made = sk.offset_chain(0, DVec2::new(5.0, 3.0), None).unwrap();
        let (a, b) = ends(&sk, made[0]);
        assert!(a.distance(DVec2::new(0.0, 3.0)) < 1e-12 && b.distance(DVec2::new(10.0, 3.0)) < 1e-12);
        let made = sk.offset_chain(0, DVec2::new(5.0, -2.0), Some(1.5)).unwrap();
        let (a, _) = ends(&sk, made[0]);
        assert!((a.y + 1.5).abs() < 1e-12);
    }

    #[test]
    fn offset_a_square_inward_gives_a_smaller_square() {
        let mut sk = lines(&[((0.0, 0.0), (10.0, 0.0)), ((10.0, 0.0), (10.0, 10.0)), ((10.0, 10.0), (0.0, 10.0)), ((0.0, 10.0), (0.0, 0.0))]);
        // Draw lines apart, then join the corners so they share points.
        let mut sq = Sketch::new("S".into(), OriginPlane::XY);
        let mut ed = Editor::default();
        ed.set_tool(Tool::Rectangle);
        ed.click(&mut sq, Anchor::Free(DVec2::new(0.0, 0.0)));
        ed.click(&mut sq, Anchor::Free(DVec2::new(10.0, 10.0)));
        std::mem::swap(&mut sk, &mut sq);
        let before = sk.entities.len();
        let made = sk.offset_chain(0, DVec2::new(5.0, 2.0), None).unwrap();
        assert_eq!(made.len(), 4);
        assert_eq!(sk.entities.len(), before + 4);
        let mut xs: Vec<f64> = Vec::new();
        for &m in &made {
            let (a, b) = ends(&sk, m);
            xs.extend([a.x, b.x]);
            assert!(a.x > 1.999 && a.x < 8.001 && a.y > 1.999 && a.y < 8.001);
        }
        assert!(xs.iter().any(|x| (x - 2.0).abs() < 1e-9) && xs.iter().any(|x| (x - 8.0).abs() < 1e-9));
        // The new square is joined at its corners.
        let (_, b0) = ends(&sk, made[0]);
        let (a1, _) = ends(&sk, made[1]);
        assert_eq!(b0, a1);
    }

    #[test]
    fn offset_a_square_outward_grows_it() {
        let mut sq = Sketch::new("S".into(), OriginPlane::XY);
        let mut ed = Editor::default();
        ed.set_tool(Tool::Rectangle);
        ed.click(&mut sq, Anchor::Free(DVec2::new(0.0, 0.0)));
        ed.click(&mut sq, Anchor::Free(DVec2::new(10.0, 10.0)));
        let made = sq.offset_chain(0, DVec2::new(5.0, -3.0), None).unwrap();
        let (lo, hi) = made.iter().flat_map(|&m| { let (a, b) = ends(&sq, m); [a, b] }).fold((DVec2::splat(1e9), DVec2::splat(-1e9)), |(l, h), p| (l.min(p), h.max(p)));
        assert!(lo.distance(DVec2::splat(-3.0)) < 1e-9 && hi.distance(DVec2::splat(13.0)) < 1e-9);
    }

    #[test]
    fn offset_a_circle_and_an_arc_change_the_radius() {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        sk.points.push(DVec2::new(1.0, 1.0));
        sk.entities.push(Entity::Circle { center: 1, radius: 5.0 });
        let m = sk.offset_chain(0, DVec2::new(10.0, 1.0), None).unwrap();
        assert!(matches!(sk.entities[m[0]], Entity::Circle { radius, .. } if (radius - 9.0).abs() < 1e-12));
        let m = sk.offset_chain(0, DVec2::new(1.0, 1.0 + 2.0), None).unwrap();
        assert!(matches!(sk.entities[m[0]], Entity::Circle { radius, .. } if (radius - 2.0).abs() < 1e-12));

        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        sk.points.extend([DVec2::new(4.0, 0.0), DVec2::new(0.0, 4.0)]);
        sk.entities.push(Entity::Arc { center: 0, a: 1, b: 2 });
        let m = sk.offset_chain(0, DVec2::new(1.0, 1.0), None).unwrap();
        let (c, r, _, sweep) = sk.arc_geometry(m[0]).unwrap();
        assert!(c.distance(DVec2::ZERO) < 1e-12 && (r - 2.0_f64.sqrt()).abs() < 1e-9 && (sweep - std::f64::consts::FRAC_PI_2).abs() < 1e-9);
    }

    #[test]
    fn offset_a_line_and_arc_run_stays_connected_and_tangent() {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        sk.points.extend([DVec2::new(0.0, 0.0), DVec2::new(10.0, 0.0), DVec2::new(10.0, 6.0), DVec2::new(10.0, 3.0)]);
        sk.entities.push(Entity::Line { a: 1, b: 2 });
        sk.entities.push(Entity::Arc { center: 4, a: 2, b: 3 });
        let m = sk.offset_chain(0, DVec2::new(5.0, 1.0), None).unwrap();
        assert_eq!(m.len(), 2);
        let (a, b) = ends(&sk, m[0]);
        assert!(a.distance(DVec2::new(0.0, 1.0)) < 1e-9 && b.distance(DVec2::new(10.0, 1.0)) < 1e-9);
        let (c, r, _, sweep) = sk.arc_geometry(m[1]).unwrap();
        assert!(c.distance(DVec2::new(10.0, 3.0)) < 1e-9 && (r - 2.0).abs() < 1e-9 && (sweep - std::f64::consts::PI).abs() < 1e-9);
        let Entity::Arc { a: arc_a, .. } = sk.entities[m[1]] else { panic!() };
        let Entity::Line { b: line_b, .. } = sk.entities[m[0]] else { panic!() };
        assert_eq!(arc_a, line_b);
    }

    /// Add constraints one at a time as the app does; how many were refused.
    fn add_each(sk: &mut Sketch, cs: Vec<Constraint>) -> usize {
        let mut refused = 0;
        for c in cs {
            let mut t = sk.clone();
            match crate::solver::add_constraints(&mut t, &[c]) {
                Ok(_) => *sk = t,
                Err(_) => refused += 1,
            }
        }
        refused
    }

    fn dof(sk: &Sketch) -> usize {
        crate::solver::solve(&mut sk.clone(), &[]).unwrap().dof
    }

    /// The dimension an offset shows (the first of its Offset constraints).
    fn offset_dim(sk: &Sketch) -> usize {
        let ci = sk.constraints.iter().position(|c| matches!(c, Constraint::Offset(..))).unwrap();
        assert_eq!(sk.offset_group(ci)[0], ci);
        ci
    }

    #[test]
    fn an_offset_rectangle_is_held_by_one_dimension() {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        let mut ed = Editor::default();
        ed.set_tool(Tool::Rectangle);
        ed.click(&mut sk, Anchor::Free(DVec2::new(0.0, 0.0)));
        ed.click(&mut sk, Anchor::Free(DVec2::new(10.0, 6.0)));
        let before = dof(&sk);
        let cs = sk.offset_constrained(0, DVec2::new(5.0, -2.0), None).unwrap();
        assert_eq!(cs.len(), 4);
        assert_eq!(add_each(&mut sk, cs), 0, "every side's offset holds");
        assert_eq!(dof(&sk), before, "the copy moves only with the original");
        let ci = offset_dim(&sk);
        assert_eq!(sk.offset_group(ci).len(), 4);
        assert!((sk.measure(&sk.constraints[ci]).unwrap() - 2.0).abs() < 1e-9);
        // Change the one dimension: every side moves out to 5.
        crate::solver::set_dimension_value(&mut sk, ci, 5.0).unwrap();
        let (lo, hi) = (4..8).flat_map(|m| { let (a, b) = ends(&sk, m); [a, b] }).fold((DVec2::splat(1e9), DVec2::splat(-1e9)), |(l, h), p| (l.min(p), h.max(p)));
        assert!(lo.distance(DVec2::new(-5.0, -5.0)) < 1e-6 && hi.distance(DVec2::new(15.0, 11.0)) < 1e-6, "{lo} {hi}");
    }

    #[test]
    fn an_offset_slot_stays_a_slot_when_its_dimension_changes() {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        let mut ed = Editor::default();
        ed.set_tool(Tool::Slot);
        ed.click(&mut sk, Anchor::Free(DVec2::new(0.0, 0.0)));
        ed.click(&mut sk, Anchor::Free(DVec2::new(20.0, 0.0)));
        let cs = ed.click(&mut sk, Anchor::Free(DVec2::new(5.0, 3.0)));
        add_each(&mut sk, cs);
        let before = dof(&sk);
        // Outward by 1 (the cursor 1 above the top side).
        let cs = sk.offset_constrained(0, DVec2::new(10.0, 4.0), None).unwrap();
        // (The far side's offset is already implied by the near side, the
        // tangents and the shared centres, so the solver skips it.)
        add_each(&mut sk, cs);
        assert_eq!(dof(&sk), before, "the copy is fully held");
        let ci = offset_dim(&sk);
        crate::solver::set_dimension_value(&mut sk, ci, 2.5).unwrap();
        // Copies: half circles of radius 3 + 2.5 round the same centres, sides 5.5 out.
        for e in 5..sk.entities.len() {
            match sk.entities[e] {
                Entity::Arc { .. } => assert!((sk.arc_geometry(e).unwrap().1 - 5.5).abs() < 1e-6),
                Entity::Line { .. } => {
                    let (a, b) = ends(&sk, e);
                    assert!((a.y.abs() - 5.5).abs() < 1e-6 && (b.y.abs() - 5.5).abs() < 1e-6, "{a} {b}");
                }
                _ => {}
            }
        }
    }

    #[test]
    fn an_offset_circle_keeps_its_centre_and_gap() {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        sk.points.push(DVec2::new(1.0, 1.0));
        sk.entities.push(Entity::Circle { center: 1, radius: 5.0 });
        let before = dof(&sk);
        let cs = sk.offset_constrained(0, DVec2::new(1.0, 3.0), Some(1.5)).unwrap();
        assert_eq!(add_each(&mut sk, cs), 0);
        assert_eq!(dof(&sk), before);
        let ci = offset_dim(&sk);
        crate::solver::set_dimension_value(&mut sk, ci, 4.0).unwrap();
        assert!(matches!(sk.entities[1], Entity::Circle { center: 1, radius } if (radius - 1.0).abs() < 1e-9));
    }

    #[test]
    fn deleting_the_original_keeps_the_copys_shared_centre() {
        use crate::sketch::Sel;
        for round in [true, false] {
            let mut sk = Sketch::new("S".into(), OriginPlane::XY);
            sk.points.extend([DVec2::new(0.0, 0.0), DVec2::new(4.0, 0.0), DVec2::new(0.0, 4.0)]);
            sk.entities.push(if round { Entity::Circle { center: 1, radius: 4.0 } } else { Entity::Arc { center: 1, a: 2, b: 3 } });
            let cs = sk.offset_constrained(0, DVec2::new(5.0, 5.0), Some(1.0)).unwrap();
            add_each(&mut sk, cs);
            sk.delete(&[Sel::Entity(0)]);
            assert_eq!(sk.entities.len(), 1);
            assert!(sk.constraints.iter().all(|c| !matches!(c, Constraint::Offset(..))), "the offset went with its original");
            match sk.entities[0] {
                Entity::Circle { center, radius } => assert!(center < sk.points.len() && (radius - 5.0).abs() < 1e-9),
                Entity::Arc { .. } => assert!((sk.arc_geometry(0).unwrap().1 - 5.0).abs() < 1e-9),
                _ => panic!(),
            }
            crate::solver::solve(&mut sk, &[]).unwrap();
        }
    }

    #[test]
    fn a_branch_cannot_be_offset() {
        let mut sk = lines(&[((0.0, 0.0), (5.0, 0.0)), ((0.0, 0.0), (0.0, 5.0)), ((0.0, 0.0), (-5.0, -5.0))]);
        // They don't share the origin point here, so join three at one point by hand.
        sk.points.push(DVec2::new(20.0, 20.0));
        let c = sk.points.len() - 1;
        for q in [DVec2::new(25.0, 20.0), DVec2::new(20.0, 25.0), DVec2::new(15.0, 15.0)] {
            sk.points.push(q);
            sk.entities.push(Entity::Line { a: c, b: sk.points.len() - 1 });
        }
        let e = sk.entities.len() - 1;
        assert!(sk.offset_chain(e, DVec2::new(0.0, 0.0), Some(1.0)).is_err());
    }
}
