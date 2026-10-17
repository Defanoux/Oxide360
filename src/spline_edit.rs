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
    /// Refused for a Control Point Spline (these tools work on fit-point ones).
    fn fit_only(&self, s: usize) -> Result<(), String> {
        match self.control_degree(s) {
            Some(_) => Err("that works on fit point splines (not control point ones yet)".into()),
            None => Ok(()),
        }
    }

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
                    Entity::Point { .. } | Entity::Text { .. } => {}
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
        self.fit_only(s)?;
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
        self.fit_only(s)?;
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
        self.fit_only(s)?;
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

    /// The curve's own direction at fit point index `k` of spline `s` (unit;
    /// as it runs), from its samples.
    fn fit_direction(&self, s: usize, k: usize) -> Option<DVec2> {
        let curve = self.spline_curve(s);
        let n = curve.len();
        let i = (k * self.per_span(s)).min(n - 1);
        let closed = self.spline_closed(s);
        let (a, b) = if closed && (i == 0 || i == n - 1) { (curve[n - 2], curve[1]) } else { (curve[i.saturating_sub(1)], curve[(i + 1).min(n - 1)]) };
        (b - a).try_normalize()
    }

    /// How long a natural-strength handle is drawn on spline `s`: a third of
    /// its average span.
    pub fn handle_base(&self, s: usize) -> f64 {
        let fit = self.spline_fit(s);
        let spans = fit.len().saturating_sub(1).max(1);
        fit.windows(2).map(|w| w[0].distance(w[1])).sum::<f64>() / spans as f64 / 3.0
    }

    /// Fusion's tangent handles on spline `s`, one per fit point: (the fit
    /// point, where the handle's forward end is, whether one is set).
    pub fn spline_handles(&self, s: usize) -> Vec<(PointId, DVec2, bool)> {
        let Some(list) = self.splines.get(s).filter(|_| self.control_degree(s).is_none()) else { return Vec::new() };
        let n = if self.spline_closed(s) { list.len() - 1 } else { list.len() };
        let base = self.handle_base(s);
        (0..n)
            .filter_map(|k| {
                let p = list[k];
                let (t, set) = match self.spline_tangent(p) {
                    Some(t) => (t, true),
                    None => (self.fit_direction(s, k)?, false),
                };
                Some((p, self.points[p] + t * base, set))
            })
            .collect()
    }

    /// The tangent handles to show for what's selected: every handle of a
    /// selected spline, and the handle of a selected fit point. (spline,
    /// fit point, forward end, set.)
    pub fn shown_handles(&self, sel: &[crate::sketch::Sel]) -> Vec<(usize, PointId, DVec2, bool)> {
        use crate::sketch::Sel;
        let mut out: Vec<(usize, PointId, DVec2, bool)> = Vec::new();
        for (e, ent) in self.entities.iter().enumerate() {
            let Entity::Spline { s } = *ent else { continue };
            let whole = sel.contains(&Sel::Entity(e));
            for (p, fwd, set) in self.spline_handles(s) {
                if (whole || sel.contains(&Sel::Point(p))) && !out.iter().any(|h| h.1 == p) {
                    out.push((s, p, fwd, set));
                }
            }
        }
        out
    }

    /// Offset spline `e` to the side `at` is on, `distance` away (or as far
    /// as `at` is): a new spline through its fit points moved square off the
    /// curve, each leaving the same way the original does there — so it has
    /// the same points and follows the original closely.
    pub fn offset_spline(&mut self, e: EntityId, at: DVec2, distance: Option<f64>) -> Result<Vec<EntityId>, String> {
        let Some(&Entity::Spline { s }) = self.entities.get(e) else { return Err("not a spline".into()) };
        self.fit_only(s)?;
        let curve = self.spline_curve(s);
        if curve.len() < 2 {
            return Err("the spline is too short".into());
        }
        let closed = self.spline_closed(s);
        // Which side, and how far: from the nearest point of the curve.
        let pos = self.spline_position(s, at);
        let i = (pos.round() as usize).min(curve.len() - 1);
        let near = |i: usize| (curve[(i + 1).min(curve.len() - 1)] - curve[i.saturating_sub(1)]).try_normalize();
        let nrm = near(i).ok_or("the spline has no direction there")?.perp();
        let side = if nrm.dot(at - curve[i]) >= 0.0 { 1.0 } else { -1.0 };
        let dist = distance.map_or_else(|| at.distance(curve[i]), f64::abs);
        if dist < 1e-9 {
            return Err("move the cursor off the spline to set the distance".into());
        }
        let list = self.splines[s].clone();
        let n = if closed { list.len() - 1 } else { list.len() };
        let per = self.per_span(s);
        let last = curve.len() - 1;
        // Square to the curve at sample i, and the point that far off it.
        let normal_at = |i: usize| {
            let (a, b) = if closed && (i == 0 || i == last) { (curve[last - 1], curve[1]) } else { (curve[i.saturating_sub(1)], curve[(i + 1).min(last)]) };
            (b - a).try_normalize().map(|t| t.perp())
        };
        let target = |i: usize| normal_at(i).map(|nn| curve[i] + nn * (side * dist));
        // Start at the fit points; add a point mid-span wherever the copy
        // strays from the true offset by more than 0.02 (a few rounds).
        let mut at_samples: Vec<usize> = (0..n).map(|k| k * per).collect();
        let build = |idx: &[usize]| -> Option<Vec<DVec2>> {
            let mut pts: Vec<DVec2> = idx.iter().map(|&i| target(i)).collect::<Option<_>>()?;
            if closed {
                pts.push(pts[0]);
            }
            Some(pts)
        };
        // The original's parameter (chord length) at sample i.
        let fit: Vec<DVec2> = list.iter().map(|&p| self.points[p]).collect();
        let mut cum = vec![0.0];
        for w in fit.windows(2) {
            cum.push(cum.last().copied().unwrap_or(0.0) + w[0].distance(w[1]));
        }
        let u_orig = |i: usize| {
            let k = (i / per).min(cum.len().saturating_sub(2));
            cum[k] + (cum[k + 1] - cum[k]) * (i - k * per) as f64 / per as f64
        };
        // Each point leaves the way the true offset curve does there, as fast
        // as it runs against the copy's own (chord-length) parameter.
        let ways = |idx: &[usize]| -> Vec<Option<DVec2>> {
            let pts: Vec<DVec2> = idx.iter().filter_map(|&i| target(i)).collect();
            let m = idx.len();
            let mut v: Vec<Option<DVec2>> = (0..m)
                .map(|j| {
                    let i = idx[j];
                    // How the offset curve moves per unit of the original's parameter.
                    let (a, b) = if closed && i == 0 { (last - 1, 1) } else { (i.saturating_sub(1), (i + 1).min(last)) };
                    let du = if closed && i == 0 { u_orig(last) - u_orig(last - 1) + u_orig(1) } else { u_orig(b) - u_orig(a) };
                    let d_orig = (target(b)? - target(a)?) / du.max(1e-12);
                    // The copy's parameter against the original's, round this point.
                    let mut ratio = (0.0, 0.0);
                    for (x, y) in [(j.checked_sub(1), Some(j)), (Some(j), Some(j + 1))] {
                        let (Some(x), Some(y)) = (x, y) else { continue };
                        let (x, y) = if closed { (x % m, y % m) } else { (x, y) };
                        if y >= m && !closed || x >= m || y >= m {
                            continue;
                        }
                        let (ix, iy) = (idx[x], if closed && y == 0 { last } else { idx[y] });
                        let orig = (u_orig(iy) - u_orig(ix)).abs();
                        let copy = pts[x].distance(pts[y]);
                        if orig > 1e-12 {
                            ratio = (ratio.0 + copy, ratio.1 + orig);
                        }
                    }
                    let rate = if ratio.1 > 0.0 { ratio.1 / ratio.0.max(1e-12) } else { 1.0 };
                    Some(d_orig * rate)
                })
                .collect();
            if closed {
                v.push(v[0]);
            }
            v
        };
        for _ in 0..4 {
            let pts = build(&at_samples).ok_or("the spline has no direction there")?;
            let copy = crate::spline::polyline_with(&pts, &ways(&at_samples));
            let mut ends: Vec<usize> = at_samples.clone();
            ends.push(if closed { last } else { *at_samples.last().unwrap_or(&0) });
            let mut add = Vec::new();
            for w in ends.windows(2).filter(|w| w[1] > w[0] + 1) {
                let worst = (w[0]..=w[1]).step_by(((w[1] - w[0]) / 8).max(1)).filter_map(target).map(|q| crate::sketch::polyline_distance(&copy, q)).fold(0.0, f64::max);
                if worst > 0.02 {
                    add.push((w[0] + w[1]) / 2);
                }
            }
            if add.is_empty() {
                break;
            }
            at_samples.extend(add);
            at_samples.sort_unstable();
            at_samples.dedup();
        }
        let pts = build(&at_samples).ok_or("the spline has no direction there")?;
        let mut made = Vec::new();
        for q in pts.iter().take(at_samples.len()) {
            self.points.push(*q);
            made.push(self.points.len() - 1);
        }
        let handles: Vec<(PointId, DVec2)> = made.iter().copied().zip(ways(&at_samples).into_iter().flatten()).collect();
        if closed {
            made.push(made[0]);
        }
        self.splines.push(made);
        for (p, t) in handles {
            self.set_spline_tangent(p, Some(t));
        }
        self.entities.push(Entity::Spline { s: self.splines.len() - 1 });
        let new = self.entities.len() - 1;
        self.set_construction(new, self.is_construction(e));
        Ok(vec![new])
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
                Entity::Point { .. } | Entity::Text { .. } => {}
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
    fn offset_copies_a_spline_the_same_distance_away() {
        let mut sk = crossed();
        let e = sk.entities.iter().position(|x| matches!(x, Entity::Spline { .. })).unwrap();
        let made = sk.offset_spline(e, DVec2::new(20.0, 10.0), Some(3.0)).unwrap();
        let Entity::Spline { s: copy } = sk.entities[made[0]] else { panic!("a spline") };
        let Entity::Spline { s } = sk.entities[e] else { unreachable!() };
        // Every point of the copy is 3 from the original (to within the sampling).
        let original = sk.spline_fit(s);
        for p in sk.spline_curve(copy).iter().step_by(7) {
            let d = crate::spline::distance(&original, *p);
            assert!((d - 3.0).abs() < 0.02, "{p} is {d} away");
        }
        // A gentle curve (an arch like the one in the app): the copy needs
        // no more points than the original, not a crowd.
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        let mut ed = Editor::default();
        ed.set_tool(Tool::Spline);
        for p in [(0.0, 0.0), (15.0, 12.0), (30.0, 15.0), (45.0, 12.0), (60.0, 0.0)] {
            ed.click(&mut sk, Anchor::Free(DVec2::new(p.0, p.1)));
        }
        ed.click(&mut sk, Anchor::Free(DVec2::new(60.0, 0.0)));
        let made = sk.offset_spline(0, DVec2::new(30.0, 20.0), Some(4.0)).unwrap();
        let Entity::Spline { s: copy } = sk.entities[made[0]] else { panic!("a spline") };
        assert!(sk.splines[copy].len() <= 7, "about as many points as the original (5): {}", sk.splines[copy].len());
        for p in sk.spline_curve(copy).iter().step_by(5) {
            let d = crate::spline::distance(&sk.spline_fit(0), *p);
            assert!((d - 4.0).abs() < 0.02, "{p} is {d} away");
        }
    }

    #[test]
    fn a_tangent_handle_turns_the_curve_and_follows_its_point() {
        let mut sk = crossed();
        let Entity::Spline { s } = sk.entities[0] else { panic!("the spline is first") };
        let handles = sk.spline_handles(s);
        assert_eq!(handles.len(), 4, "one per fit point");
        assert!(handles.iter().all(|h| !h.2), "none set yet");
        // Set the second fit point's handle straight down: the curve leaves it downward.
        let p = sk.splines[s][1];
        sk.set_spline_tangent(p, Some(DVec2::new(0.0, -1.0)));
        let curve = sk.spline_curve(s);
        let i = crate::spline::PER_SPAN;
        assert!((curve[i + 1] - curve[i - 1]).normalize().dot(-DVec2::Y) > 0.99);
        assert!(sk.spline_handles(s)[1].2, "now set");
        // Deleting a line earlier in the list renumbers points: the handle follows its point.
        let at = sk.points[p];
        let l = sk.entities.iter().position(|x| matches!(x, Entity::Line { .. })).unwrap();
        sk.delete(&[crate::sketch::Sel::Entity(l)]);
        let q = sk.points.iter().position(|x| *x == at).unwrap();
        assert_eq!(sk.spline_tangent(q), Some(DVec2::new(0.0, -1.0)));
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

#[cfg(test)]
mod bench {
    use super::*;
    use crate::sketch::{Anchor, Editor, OriginPlane, Tool};

    #[test]
    #[ignore]
    fn time_an_arch_and_its_offset() {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        let mut ed = Editor::default();
        ed.set_tool(Tool::Spline);
        for p in [(0.0, 0.0), (15.0, 12.0), (30.0, 15.0), (45.0, 12.0), (60.0, 0.0)] {
            ed.click(&mut sk, Anchor::Free(DVec2::new(p.0, p.1)));
        }
        ed.click(&mut sk, Anchor::Free(DVec2::new(60.0, 0.0)));
        ed.set_tool(Tool::Line);
        ed.click(&mut sk, Anchor::Free(DVec2::new(-5.0, 3.0)));
        ed.click(&mut sk, Anchor::Free(DVec2::new(65.0, 3.0)));
        let t = std::time::Instant::now();
        sk.offset_spline(0, DVec2::new(30.0, 20.0), Some(4.0)).unwrap();
        eprintln!("offset: {:?}", t.elapsed());
        for _ in 0..2 {
            let t = std::time::Instant::now();
            let r = crate::profile::find_regions(&sk);
            eprintln!("find_regions: {:?} ({} regions)", t.elapsed(), r.len());
            let t = std::time::Instant::now();
            for e in 0..sk.entities.len() {
                if let Entity::Spline { s } = sk.entities[e] {
                    let _ = sk.spline_curve(s);
                }
            }
            eprintln!("curves: {:?}", t.elapsed());
            let t = std::time::Instant::now();
            let _ = sk.hit(DVec2::new(30.0, 10.0), 0.5);
            eprintln!("hit: {:?}", t.elapsed());
            let t = std::time::Instant::now();
            let _ = crate::solver::solve(&mut sk.clone(), &[]);
            eprintln!("solve: {:?}", t.elapsed());
        }
    }
}
