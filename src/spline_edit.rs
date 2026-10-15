//! Trim, Break and Extend for fit-point splines. A spline is cut where other
//! geometry crosses its curve; the pieces kept are new splines through the
//! cut point(s) and the fit points on their side (the curve near a cut moves
//! a little, as the fit points change).

use crate::sketch::{Constraint, Entity, EntityId, PointId, Sketch, line_circle};
use glam::DVec2;

/// A crossing on a spline: (how far along its samples: sample index plus
/// fraction, the point, what crosses it, an existing point of that there).
type Cut = (f64, DVec2, EntityId, Option<PointId>);

/// Where straight piece p→q crosses straight piece a→b: (t along p→q, the point).
fn seg_seg(p: DVec2, q: DVec2, a: DVec2, b: DVec2) -> Option<(f64, DVec2)> {
    let (d, f) = (q - p, b - a);
    let den = d.perp_dot(f);
    if den.abs() < 1e-12 * d.length() * f.length() {
        return None;
    }
    let t = (a - p).perp_dot(f) / den;
    let u = (a - p).perp_dot(d) / den;
    ((0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u)).then(|| (t, p + d * t))
}

impl Sketch {
    /// How many samples a spline's span (between two fit points) has.
    fn per_span(&self, s: usize) -> usize {
        if self.splines.get(s).is_some_and(|l| l.len() >= 3) { crate::spline::PER_SPAN } else { 1 }
    }

    fn spline_closed(&self, s: usize) -> bool {
        crate::spline::is_closed(&self.spline_fit(s))
    }

    /// Where other geometry crosses spline entity `e`, sorted along it (an
    /// open spline's own ends don't count).
    pub(crate) fn spline_cuts(&self, e: EntityId) -> Vec<Cut> {
        let Some(&Entity::Spline { s }) = self.entities.get(e) else { return Vec::new() };
        let curve = self.spline_curve(s);
        let end = (curve.len() - 1) as f64;
        let tol = 1e-6;
        let near = |p: PointId, x: DVec2| (self.points[p].distance(x) < 1e-6).then_some(p);
        let mut cuts: Vec<Cut> = Vec::new();
        for (j, w) in curve.windows(2).enumerate() {
            for (i, ent) in self.entities.iter().enumerate() {
                if i == e {
                    continue;
                }
                let mut found: Vec<(f64, DVec2, Option<PointId>)> = Vec::new();
                match *ent {
                    Entity::Line { a, b } => {
                        if let Some((t, x)) = seg_seg(w[0], w[1], self.points[a], self.points[b]) {
                            found.push((t, x, near(a, x).or(near(b, x))));
                        }
                    }
                    Entity::Circle { .. } | Entity::Arc { .. } => {
                        let (c, r, a0, sweep) = match *ent {
                            Entity::Circle { center, radius } => (self.points[center], radius, 0.0, std::f64::consts::TAU),
                            _ => match self.arc_geometry(i) {
                                Some(g) => g,
                                None => continue,
                            },
                        };
                        let ends: Vec<PointId> = if let Entity::Arc { a, b, .. } = *ent { vec![a, b] } else { vec![] };
                        for t in line_circle(w[0], w[1] - w[0], c, r) {
                            let x = w[0] + (w[1] - w[0]) * t;
                            let on = ((x - c).y.atan2((x - c).x) - a0).rem_euclid(std::f64::consts::TAU) <= sweep + 1e-9;
                            if (0.0..=1.0).contains(&t) && on {
                                found.push((t, x, ends.iter().find_map(|&p| near(p, x))));
                            }
                        }
                    }
                    Entity::Spline { s: o } => {
                        let ends: Vec<PointId> = self.splines.get(o).map_or(Vec::new(), |l| [l.first(), l.last()].into_iter().flatten().copied().collect());
                        for v in self.spline_curve(o).windows(2) {
                            if let Some((t, x)) = seg_seg(w[0], w[1], v[0], v[1]) {
                                found.push((t, x, ends.iter().find_map(|&p| near(p, x))));
                            }
                        }
                    }
                    Entity::Ellipse { .. } => {
                        for v in self.ellipse_curve(i).windows(2) {
                            if let Some((t, x)) = seg_seg(w[0], w[1], v[0], v[1]) {
                                found.push((t, x, None));
                            }
                        }
                    }
                    Entity::Point { .. } => {}
                }
                for (t, x, p) in found {
                    let at = j as f64 + t;
                    let own_end = !self.spline_closed(s) && (at < tol || at > end - tol);
                    if !own_end {
                        cuts.push((at, x, i, p));
                    }
                }
            }
        }
        cuts.sort_by(|a, b| a.0.total_cmp(&b.0));
        cuts.dedup_by(|a, b| a.1.distance(b.1) < 1e-7);
        cuts
    }

    /// How far along spline `s`'s samples the point nearest `at` is.
    fn spline_position(&self, s: usize, at: DVec2) -> f64 {
        let curve = self.spline_curve(s);
        let mut best = (f64::INFINITY, 0.0);
        for (j, w) in curve.windows(2).enumerate() {
            let d = w[1] - w[0];
            let t = ((at - w[0]).dot(d) / d.length_squared().max(1e-30)).clamp(0.0, 1.0);
            let dist = (w[0] + d * t).distance(at);
            if dist < best.0 {
                best = (dist, j as f64 + t);
            }
        }
        best.1
    }

    /// The fit points of spline `s` strictly between positions `from` and
    /// `to` along its samples (`to` past the end wraps round a closed one).
    fn fit_between(&self, s: usize, from: f64, to: f64) -> Vec<PointId> {
        let list = &self.splines[s];
        let per = self.per_span(s) as f64;
        let closed = self.spline_closed(s);
        let n = if closed { list.len() - 1 } else { list.len() };
        let round = n as f64 * per;
        let gap = 0.05 * per; // a fit point right at a cut is left out
        let mut out = Vec::new();
        for lap in 0..if closed { 2 } else { 1 } {
            for (k, &p) in list.iter().take(n).enumerate() {
                let at = k as f64 * per + lap as f64 * round;
                if at > from + gap && at < to - gap {
                    out.push(p);
                }
            }
        }
        out
    }

    /// The stretch of spline `e` between the nearest cuts either side of the
    /// click: (start position, end position, the cuts there).
    fn spline_span(&self, e: EntityId, at: DVec2) -> Option<(Option<Cut>, Option<Cut>, f64, Vec<Cut>)> {
        let Some(&Entity::Spline { s }) = self.entities.get(e) else { return None };
        let cuts = self.spline_cuts(e);
        let t = self.spline_position(s, at);
        let lo = cuts.iter().rev().find(|c| c.0 <= t).copied();
        let hi = cuts.iter().find(|c| c.0 > t).copied();
        Some((lo, hi, t, cuts))
    }

    /// Points along the stretch Trim would take from spline `e` for a click at `at`.
    pub fn spline_trim_preview(&self, e: EntityId, at: DVec2) -> Option<Vec<DVec2>> {
        let Some(&Entity::Spline { s }) = self.entities.get(e) else { return None };
        let curve = self.spline_curve(s);
        let (lo, hi, _, cuts) = self.spline_span(e, at)?;
        let closed = self.spline_closed(s);
        let end = (curve.len() - 1) as f64;
        let (from, to) = match (lo, hi) {
            (Some(l), Some(h)) => (l.0, h.0),
            (Some(l), None) if closed && !cuts.is_empty() => (l.0, cuts[0].0 + end),
            (None, Some(h)) if closed => (cuts.last().expect("a cut").0 - end, h.0),
            (Some(l), None) => (l.0, end),
            (None, Some(h)) => (0.0, h.0),
            (None, None) => (0.0, end),
        };
        let sample = |x: f64| {
            let x = x.rem_euclid(end.max(1e-12));
            let x = if (x - end).abs() < 1e-12 { end } else { x };
            let j = (x.floor() as usize).min(curve.len() - 2);
            curve[j].lerp(curve[j + 1], x - j as f64)
        };
        let steps = ((to - from).ceil() as usize).max(1);
        Some((0..=steps).map(|k| sample(from + (to - from) * k as f64 / steps as f64)).collect())
    }

    /// A point for cut `c` (the one already there, or a new one tied onto what crosses).
    fn cut_point(&mut self, c: Cut, ties: &mut Vec<Constraint>) -> PointId {
        match c.3 {
            Some(p) => p,
            None => {
                self.points.push(c.1);
                let p = self.points.len() - 1;
                match self.entities.get(c.2) {
                    Some(Entity::Line { .. }) => ties.push(Constraint::OnLine(p, c.2)),
                    Some(Entity::Circle { .. } | Entity::Arc { .. }) => ties.push(Constraint::OnCircle(p, c.2)),
                    _ => {}
                }
                p
            }
        }
    }

    /// Make the spline pieces `spans` (start point, the fit points inside, end
    /// point) of spline entity `e`: the first replaces it, the rest are new.
    fn respline(&mut self, e: EntityId, pieces: Vec<Vec<PointId>>) {
        let Entity::Spline { s } = self.entities[e] else { return };
        let construction = self.is_construction(e);
        let mut first = true;
        for list in pieces.into_iter().filter(|l| l.len() >= 2) {
            if first {
                self.splines[s] = list;
                first = false;
            } else {
                self.splines.push(list);
                self.entities.push(Entity::Spline { s: self.splines.len() - 1 });
                let n = self.entities.len() - 1;
                self.set_construction(n, construction);
            }
        }
        if first {
            self.delete(&[crate::sketch::Sel::Entity(e)]);
        }
    }

    /// Trim spline `e` at the click `at`: the stretch between the nearest
    /// crossings goes (nothing crossing: the whole spline).
    pub fn trim_spline(&mut self, e: EntityId, at: DVec2) -> Result<Vec<Constraint>, String> {
        let Some(&Entity::Spline { s }) = self.entities.get(e) else { return Err("not a spline".into()) };
        let (lo, hi, _, cuts) = self.spline_span(e, at).ok_or("not a spline")?;
        let closed = self.spline_closed(s);
        let list = self.splines[s].clone();
        let end = (self.spline_curve(s).len() - 1) as f64;
        if cuts.is_empty() || (closed && cuts.len() < 2) {
            self.delete(&[crate::sketch::Sel::Entity(e)]);
            return Ok(Vec::new());
        }
        let mut ties = Vec::new();
        let mut trial = self.clone();
        let pieces = if closed {
            // What's left runs from the cut after the click round to the one before it.
            let lo = lo.unwrap_or(*cuts.last().expect("cuts"));
            let hi = hi.unwrap_or(cuts[0]);
            let to = if lo.0 > hi.0 { lo.0 } else { lo.0 + end };
            let (ph, pl) = (trial.cut_point(hi, &mut ties), trial.cut_point(lo, &mut ties));
            let mut l = vec![ph];
            l.extend(trial.fit_between(s, hi.0, to));
            l.push(pl);
            vec![l]
        } else {
            let mut out = Vec::new();
            if let Some(l) = lo {
                let p = trial.cut_point(l, &mut ties);
                let mut piece = vec![list[0]];
                piece.extend(trial.fit_between(s, 0.0, l.0));
                piece.push(p);
                out.push(piece);
            }
            if let Some(h) = hi {
                let p = trial.cut_point(h, &mut ties);
                let mut piece = vec![p];
                piece.extend(trial.fit_between(s, h.0, end));
                piece.push(*list.last().expect("a spline has points"));
                out.push(piece);
            }
            out
        };
        trial.respline(e, pieces);
        trial.delete(&[]);
        *self = trial;
        Ok(ties)
    }

    /// Break spline `e` at the nearest crossings either side of the click.
    pub(crate) fn break_spline(&mut self, e: EntityId, at: DVec2) -> Result<Vec<Constraint>, String> {
        let Some(&Entity::Spline { s }) = self.entities.get(e) else { return Err("not a spline".into()) };
        let (lo, hi, _, cuts) = self.spline_span(e, at).ok_or("not a spline")?;
        let closed = self.spline_closed(s);
        let list = self.splines[s].clone();
        let end = (self.spline_curve(s).len() - 1) as f64;
        let nothing = || "nothing crosses it there to break it at".to_string();
        let mut ties = Vec::new();
        let mut trial = self.clone();
        let pieces = if closed {
            if cuts.len() < 2 {
                return Err("a closed spline breaks where two things cross it".into());
            }
            let lo = lo.unwrap_or(*cuts.last().expect("cuts"));
            let hi = hi.unwrap_or(cuts[0]);
            let (pl, ph) = (trial.cut_point(lo, &mut ties), trial.cut_point(hi, &mut ties));
            let mut clicked = vec![pl];
            clicked.extend(trial.fit_between(s, lo.0, if hi.0 > lo.0 { hi.0 } else { hi.0 + end }));
            clicked.push(ph);
            let mut rest = vec![ph];
            rest.extend(trial.fit_between(s, hi.0, if lo.0 > hi.0 { lo.0 } else { lo.0 + end }));
            rest.push(pl);
            vec![clicked, rest]
        } else {
            let mut marks: Vec<(f64, PointId)> = vec![(0.0, list[0])];
            for c in [lo, hi].into_iter().flatten() {
                marks.push((c.0, trial.cut_point(c, &mut ties)));
            }
            marks.push((end, *list.last().expect("a spline has points")));
            if marks.len() == 2 {
                return Err(nothing());
            }
            marks
                .windows(2)
                .map(|w| {
                    let mut piece = vec![w[0].1];
                    piece.extend(trial.fit_between(s, w[0].0, w[1].0));
                    piece.push(w[1].1);
                    piece
                })
                .collect()
        };
        trial.respline(e, pieces);
        trial.delete(&[]);
        *self = trial;
        Ok(ties)
    }

    /// Extend spline `e` from the end nearest `at`: on along its direction
    /// there to the next geometry in the way, which becomes its new end (a
    /// fit point added there).
    pub fn extend_spline(&mut self, e: EntityId, at: DVec2) -> Result<Vec<Constraint>, String> {
        let Some(&Entity::Spline { s }) = self.entities.get(e) else { return Err("not a spline".into()) };
        if self.spline_closed(s) {
            return Err("a closed spline has no end to extend".into());
        }
        let (start, dir, from_last) = self.spline_end_ray(s, at).ok_or("the spline has no direction there")?;
        let (x, other, join) = self.first_hit(start, dir, e).ok_or("there is nothing in that direction to extend to")?;
        let mut trial = self.clone();
        let mut ties = Vec::new();
        let p = match join {
            Some(p) => p,
            None => trial.cut_point((0.0, x, other, None), &mut ties),
        };
        let list = &mut trial.splines[s];
        if from_last { list.push(p) } else { list.insert(0, p) }
        *self = trial;
        Ok(ties)
    }

    /// Where spline `s`'s end nearest `at` is, the way out of it, and whether it's the last end.
    pub(crate) fn spline_end_ray(&self, s: usize, at: DVec2) -> Option<(DVec2, DVec2, bool)> {
        let curve = self.spline_curve(s);
        let (first, last) = (curve[0], *curve.last()?);
        let from_last = at.distance(last) < at.distance(first);
        let n = curve.len();
        let (end, before) = if from_last { (last, curve[n - 2]) } else { (first, curve[1]) };
        Some((end, (end - before).try_normalize()?, from_last))
    }

    /// The first thing (other than `skip`) the ray from `start` along `dir`
    /// meets: the point, what it is, an existing point of it there.
    pub(crate) fn first_hit(&self, start: DVec2, dir: DVec2, skip: EntityId) -> Option<(DVec2, EntityId, Option<PointId>)> {
        let mut best: Option<(f64, DVec2, EntityId, Option<PointId>)> = None;
        let mut offer = |t: f64, x: DVec2, i: EntityId, p: Option<PointId>| {
            if t > 1e-6 && best.is_none_or(|b| t < b.0) {
                best = Some((t, x, i, p));
            }
        };
        let far = 1e6;
        let ray_end = start + dir * far;
        let near = |p: PointId, x: DVec2| (self.points[p].distance(x) < 1e-6).then_some(p);
        for (i, ent) in self.entities.iter().enumerate() {
            if i == skip {
                continue;
            }
            match *ent {
                Entity::Line { a, b } => {
                    if let Some((t, x)) = seg_seg(start, ray_end, self.points[a], self.points[b]) {
                        offer(t * far, x, i, near(a, x).or(near(b, x)));
                    }
                }
                Entity::Circle { .. } | Entity::Arc { .. } => {
                    let (c, r, a0, sweep) = match *ent {
                        Entity::Circle { center, radius } => (self.points[center], radius, 0.0, std::f64::consts::TAU),
                        _ => match self.arc_geometry(i) {
                            Some(g) => g,
                            None => continue,
                        },
                    };
                    for t in line_circle(start, dir, c, r) {
                        let x = start + dir * t;
                        if ((x - c).y.atan2((x - c).x) - a0).rem_euclid(std::f64::consts::TAU) <= sweep + 1e-9 {
                            offer(t, x, i, None);
                        }
                    }
                }
                Entity::Spline { s } => {
                    for v in self.spline_curve(s).windows(2) {
                        if let Some((t, x)) = seg_seg(start, ray_end, v[0], v[1]) {
                            offer(t * far, x, i, None);
                        }
                    }
                }
                Entity::Ellipse { .. } => {
                    for v in self.ellipse_curve(i).windows(2) {
                        if let Some((t, x)) = seg_seg(start, ray_end, v[0], v[1]) {
                            offer(t * far, x, i, None);
                        }
                    }
                }
                Entity::Point { .. } => {}
            }
        }
        best.map(|(_, x, i, p)| (x, i, p))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sketch::{Anchor, Editor, OriginPlane, Tool};

    /// A wavy spline from (0, 0) to (40, 0) and a vertical line across it at x = 20.
    fn crossed() -> Sketch {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        let mut ed = Editor::default();
        ed.set_tool(Tool::Spline);
        for p in [(0.0, 0.0), (10.0, 5.0), (25.0, -5.0), (40.0, 0.0)] {
            ed.click(&mut sk, Anchor::Free(DVec2::new(p.0, p.1)));
        }
        ed.click(&mut sk, Anchor::Free(DVec2::new(40.0, 0.0))); // the last again: done
        ed.set_tool(Tool::Line);
        ed.click(&mut sk, Anchor::Free(DVec2::new(20.0, -20.0)));
        ed.click(&mut sk, Anchor::Free(DVec2::new(20.0, 20.0)));
        sk
    }

    fn spline_ends(sk: &Sketch) -> Vec<(DVec2, DVec2)> {
        sk.entities
            .iter()
            .filter_map(|e| if let Entity::Spline { s } = *e { Some(s) } else { None })
            .map(|s| (sk.points[sk.splines[s][0]], sk.points[*sk.splines[s].last().unwrap()]))
            .collect()
    }

    #[test]
    fn trim_takes_the_clicked_side_of_a_spline() {
        let mut sk = crossed();
        let e = sk.entities.iter().position(|x| matches!(x, Entity::Spline { .. })).unwrap();
        let cuts = sk.spline_cuts(e);
        assert_eq!(cuts.len(), 1, "the line crosses it once");
        let x = cuts[0].1;
        assert!((x.x - 20.0).abs() < 1e-6);
        let ties = sk.trim_spline(e, DVec2::new(35.0, -2.0)).unwrap();
        assert_eq!(ties.len(), 1, "the new end is held on the line");
        let ends = spline_ends(&sk);
        assert_eq!(ends.len(), 1);
        assert!(ends[0].0.distance(DVec2::ZERO) < 1e-9 && ends[0].1.distance(x) < 1e-9, "{ends:?}");
        // A spline nothing crosses goes.
        let mut lone = crossed();
        lone.delete(&[crate::sketch::Sel::Entity(lone.entities.iter().position(|x| matches!(x, Entity::Line { .. })).unwrap())]);
        let e = lone.entities.iter().position(|x| matches!(x, Entity::Spline { .. })).unwrap();
        lone.trim_spline(e, DVec2::new(5.0, 3.0)).unwrap();
        assert!(spline_ends(&lone).is_empty());
    }

    #[test]
    fn break_splits_a_spline_and_a_line_at_their_crossing() {
        let mut sk = crossed();
        let e = sk.entities.iter().position(|x| matches!(x, Entity::Spline { .. })).unwrap();
        sk.break_spline(e, DVec2::new(5.0, 3.0)).unwrap();
        let ends = spline_ends(&sk);
        assert_eq!(ends.len(), 2, "two splines");
        assert!(ends[0].1.distance(ends[1].0) < 1e-9, "joined at the crossing");
        // The line breaks there too (at the spline's curve).
        let l = sk.entities.iter().position(|x| matches!(x, Entity::Line { .. })).unwrap();
        sk.break_at(l, DVec2::new(20.0, 10.0)).unwrap();
        assert_eq!(sk.entities.iter().filter(|x| matches!(x, Entity::Line { .. })).count(), 2);
    }

    #[test]
    fn extend_runs_a_spline_on_to_the_next_line() {
        let mut sk = crossed();
        let e = sk.entities.iter().position(|x| matches!(x, Entity::Spline { .. })).unwrap();
        // A wall at x = 50: the spline's last end runs on to it.
        let mut ed = Editor::default();
        ed.set_tool(Tool::Line);
        ed.click(&mut sk, Anchor::Free(DVec2::new(50.0, -30.0)));
        ed.click(&mut sk, Anchor::Free(DVec2::new(50.0, 30.0)));
        sk.extend_spline(e, DVec2::new(39.0, 0.0)).unwrap();
        let ends = spline_ends(&sk);
        assert!((ends[0].1.x - 50.0).abs() < 1e-6, "{ends:?}");
    }
}
