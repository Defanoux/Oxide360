//! Project: body edges into a sketch. A straight edge becomes a line, a
//! circular one a circle or arc (as seen straight down onto the sketch plane).

use crate::model::EdgeRef;
use crate::sketch::{Constraint, Entity, EntityId, Sketch};
use glam::{DVec2, DVec3};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Projected {
    Line(DVec2, DVec2),
    Circle(DVec2, f64),
    /// Counter-clockwise from `start` to `end` round `center`.
    Arc { center: DVec2, start: DVec2, end: DVec2 },
}

/// The centre of the circle through three points (None if they're in a line).
fn circle_through(a: DVec2, b: DVec2, c: DVec2) -> Option<DVec2> {
    let d = 2.0 * (a.x * (b.y - c.y) + b.x * (c.y - a.y) + c.x * (a.y - b.y));
    if d.abs() < 1e-12 {
        return None;
    }
    let (a2, b2, c2) = (a.length_squared(), b.length_squared(), c.length_squared());
    Some(DVec2::new(
        (a2 * (b.y - c.y) + b2 * (c.y - a.y) + c2 * (a.y - b.y)) / d,
        (a2 * (c.x - b.x) + b2 * (a.x - c.x) + c2 * (b.x - a.x)) / d,
    ))
}

/// Project an edge (a polyline of points on it) onto the plane through
/// `origin` spanned by `u` and `v`.
pub fn project_edge(origin: DVec3, u: DVec3, v: DVec3, pts: &[DVec3]) -> Result<Projected, String> {
    let flat: Vec<DVec2> = pts.iter().map(|p| DVec2::new((*p - origin).dot(u), (*p - origin).dot(v))).collect();
    let (Some(&first), Some(&last)) = (flat.first(), flat.last()) else { return Err("nothing to project".into()) };
    let size = flat.iter().map(|p| p.distance(first)).fold(0.0, f64::max);
    if size < 1e-9 {
        return Err("that edge is edge-on to the sketch plane".into());
    }
    let tol = 1e-6 * size.max(1.0);
    // Straight: every point on the line through the ends.
    if first.distance(last) > 1e-9 {
        let d = (last - first).normalize();
        if flat.iter().all(|p| (*p - first).perp_dot(d).abs() <= tol) {
            return Ok(Projected::Line(first, last));
        }
    }
    // Circular: all points the same distance from the centre.
    let closed = first.distance(last) <= tol;
    let mid = flat[flat.len() / 2];
    let probe = if closed { flat[flat.len() / 4] } else { last };
    let center = circle_through(first, mid, probe).ok_or("only straight and circular edges can be projected")?;
    let r = first.distance(center);
    if r < 1e-9 || !flat.iter().all(|p| (p.distance(center) - r).abs() <= 1e-5 * r.max(1.0)) {
        return Err("only straight and circular edges can be projected".into());
    }
    if closed {
        return Ok(Projected::Circle(center, r));
    }
    // Which way the edge runs round the centre.
    let ccw = (first - center).perp_dot(mid - center) > 0.0;
    Ok(if ccw { Projected::Arc { center, start: first, end: last } } else { Projected::Arc { center, start: last, end: first } })
}

impl Sketch {
    /// Add a projected edge as construction geometry, locked where it lies.
    /// Returns the constraints to add (Fix on each point). Err if it's already there.
    pub fn add_projected(&mut self, pr: Projected) -> Result<Vec<Constraint>, String> {
        const SAME: f64 = 1e-6;
        let exists = self.entities.iter().any(|e| match (*e, pr) {
            (Entity::Line { a, b }, Projected::Line(p, q)) => {
                let (x, y) = (self.points[a], self.points[b]);
                (x.distance(p) < SAME && y.distance(q) < SAME) || (x.distance(q) < SAME && y.distance(p) < SAME)
            }
            (Entity::Circle { center, radius }, Projected::Circle(c, r)) => self.points[center].distance(c) < SAME && (radius - r).abs() < SAME,
            (Entity::Arc { center, a, b }, Projected::Arc { center: c, start, end }) => {
                self.points[center].distance(c) < SAME && self.points[a].distance(start) < SAME && self.points[b].distance(end) < SAME
            }
            _ => false,
        });
        if exists {
            return Err("that edge is already in the sketch".into());
        }
        let add = |sk: &mut Sketch, p: DVec2| {
            sk.points.push(p);
            sk.points.len() - 1
        };
        let mut fix = Vec::new();
        let entity = match pr {
            Projected::Line(p, q) => {
                let (a, b) = (add(self, p), add(self, q));
                fix.extend([a, b]);
                Entity::Line { a, b }
            }
            Projected::Circle(c, radius) => {
                let center = add(self, c);
                fix.push(center);
                Entity::Circle { center, radius }
            }
            Projected::Arc { center, start, end } => {
                let (center, a, b) = (add(self, center), add(self, start), add(self, end));
                fix.extend([center, a, b]);
                Entity::Arc { center, a, b }
            }
        };
        self.entities.push(entity);
        let n = self.entities.len() - 1;
        self.set_construction(n, true);
        Ok(fix.into_iter().map(Constraint::Fix).collect())
    }
}

/// Project every edge that can be (the ones seen edge-on, ellipses and
/// repeats are skipped), locked and as construction geometry. Returns how many
/// were added.
/// With an edge reference, each stays linked to its edge (see `follow_links`).
pub fn project_all(sk: &mut Sketch, origin: DVec3, u: DVec3, v: DVec3, edges: &[(Option<EdgeRef>, Vec<DVec3>)]) -> usize {
    let mut added = 0;
    for (link, pts) in edges {
        let Ok(pr) = project_edge(origin, u, v, pts) else { continue };
        let mut trial = sk.clone();
        let Ok(fixes) = trial.add_projected(pr) else { continue };
        if let Some(r) = link {
            trial.projections.push((trial.entities.len() - 1, *r));
        }
        for c in fixes {
            let mut t = trial.clone();
            if crate::solver::add_constraints(&mut t, &[c]).is_ok() {
                trial = t;
            }
        }
        *sk = trial;
        added += 1;
    }
    added
}

/// Linked projections as the part is now: each link's edge (found by
/// `find`, as points along it) projected again onto the plane through
/// `origin` spanned by `u`, `v`, and the sketch solved so what's constrained
/// to them follows. Returns the moved sketch (None when nothing moved), the
/// entities whose edge is gone (or no longer projects to the same kind of
/// curve; they stay where they were), and an error if the moved sketch
/// can't be solved (then it isn't moved).
pub fn follow_links(
    sk: &Sketch,
    origin: DVec3,
    u: DVec3,
    v: DVec3,
    find: impl Fn(&EdgeRef) -> Option<Vec<DVec3>>,
) -> (Option<Sketch>, Vec<EntityId>, Option<String>) {
    const MOVED: f64 = 1e-9;
    let mut out = sk.clone();
    let mut lost = Vec::new();
    let mut moved = false;
    for &(e, r) in &sk.projections {
        let now = find(&r).and_then(|pts| project_edge(origin, u, v, &pts).ok());
        let mut set = |p: usize, to: DVec2| {
            if out.points[p].distance(to) > MOVED {
                out.points[p] = to;
                moved = true;
            }
        };
        match (sk.entities.get(e).copied(), now) {
            (Some(Entity::Line { a, b }), Some(Projected::Line(p, q))) => {
                // The edge may list its ends the other way round this time.
                let (x, y) = (sk.points[a], sk.points[b]);
                let (p, q) = if x.distance(p) + y.distance(q) <= x.distance(q) + y.distance(p) { (p, q) } else { (q, p) };
                set(a, p);
                set(b, q);
            }
            (Some(Entity::Circle { center, radius }), Some(Projected::Circle(c, r))) => {
                set(center, c);
                if (radius - r).abs() > MOVED {
                    out.entities[e] = Entity::Circle { center, radius: r };
                    moved = true;
                }
            }
            (Some(Entity::Arc { center, a, b }), Some(Projected::Arc { center: c, start, end })) => {
                set(center, c);
                set(a, start);
                set(b, end);
            }
            _ => lost.push(e),
        }
    }
    if !moved {
        return (None, lost, None);
    }
    // A big move solved in one go can start from collapsed geometry (a
    // corner dragged onto the next): then take it in smaller steps.
    let mut last = None;
    for steps in [1, 8, 32] {
        match follow_in_steps(sk, &out, steps) {
            Ok(s) => return (Some(s), lost, None),
            Err(e) => last = Some(e),
        }
    }
    let e = last.expect("tried");
    (None, lost, Some(format!("its projected lines moved with the part and the sketch can't follow: {e}")))
}

/// `from` solved again with its points (and circle radii) eased toward
/// `to`'s in `steps` steps: the projected ones are fixed, the rest follow.
fn follow_in_steps(from: &Sketch, to: &Sketch, steps: usize) -> Result<Sketch, crate::solver::SolveError> {
    let moving: Vec<usize> = (0..from.points.len()).filter(|&p| from.points[p] != to.points[p]).collect();
    let mut sk = from.clone();
    for k in 1..=steps {
        let t = k as f64 / steps as f64;
        for &p in &moving {
            sk.points[p] = from.points[p].lerp(to.points[p], t);
        }
        for (e, ent) in to.entities.iter().enumerate() {
            if let (Entity::Circle { radius: r1, .. }, Some(Entity::Circle { center, radius: r0 })) = (*ent, from.entities.get(e).copied()) {
                sk.entities[e] = Entity::Circle { center, radius: r0 + (r1 - r0) * t };
            }
        }
        crate::solver::solve(&mut sk, &[])?;
    }
    Ok(sk)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sketch::OriginPlane;

    fn flat_xy() -> (DVec3, DVec3, DVec3) {
        (DVec3::ZERO, DVec3::X, DVec3::Y)
    }

    #[test]
    fn a_straight_edge_projects_to_a_line_whatever_its_height() {
        let (o, u, v) = flat_xy();
        let pr = project_edge(o, u, v, &[DVec3::new(1.0, 2.0, 7.0), DVec3::new(5.0, 2.0, 7.0)]).unwrap();
        assert_eq!(pr, Projected::Line(DVec2::new(1.0, 2.0), DVec2::new(5.0, 2.0)));
        // Tilted: the line's shadow.
        let pr = project_edge(o, u, v, &[DVec3::new(0.0, 0.0, 0.0), DVec3::new(3.0, 4.0, 9.0)]).unwrap();
        assert_eq!(pr, Projected::Line(DVec2::ZERO, DVec2::new(3.0, 4.0)));
        // Straight up: nothing to see.
        assert!(project_edge(o, u, v, &[DVec3::new(1.0, 1.0, 0.0), DVec3::new(1.0, 1.0, 5.0)]).is_err());
    }

    #[test]
    fn circles_and_arcs_are_recognised_with_the_right_sweep() {
        let (o, u, v) = flat_xy();
        let on = |c: DVec3, r: f64, from: f64, to: f64| -> Vec<DVec3> {
            (0..=24).map(|k| c + DVec3::new((from + (to - from) * k as f64 / 24.0).cos() * r, (from + (to - from) * k as f64 / 24.0).sin() * r, 3.0)).collect()
        };
        let circle = on(DVec3::new(4.0, 5.0, 0.0), 2.0, 0.0, std::f64::consts::TAU);
        match project_edge(o, u, v, &circle).unwrap() {
            Projected::Circle(c, r) => assert!(c.distance(DVec2::new(4.0, 5.0)) < 1e-9 && (r - 2.0).abs() < 1e-9),
            other => panic!("{other:?}"),
        }
        // A quarter arc counter-clockwise, and the same one listed backwards: same arc.
        let arc = on(DVec3::ZERO, 3.0, 0.0, std::f64::consts::FRAC_PI_2);
        let expect = Projected::Arc { center: DVec2::ZERO, start: DVec2::new(3.0, 0.0), end: DVec2::new(0.0, 3.0) };
        let near = |a: Projected, b: Projected| match (a, b) {
            (Projected::Arc { center: c1, start: s1, end: e1 }, Projected::Arc { center: c2, start: s2, end: e2 }) => {
                c1.distance(c2) < 1e-9 && s1.distance(s2) < 1e-9 && e1.distance(e2) < 1e-9
            }
            _ => false,
        };
        assert!(near(project_edge(o, u, v, &arc).unwrap(), expect));
        let mut back = arc.clone();
        back.reverse();
        assert!(near(project_edge(o, u, v, &back).unwrap(), expect));
    }

    #[test]
    fn projecting_a_box_gives_its_outline_once() {
        // A 4 × 3 × 2 box: 12 edges. Seen from above the verticals are points and the
        // top and bottom edges coincide, leaving 4 lines.
        let c = |x: f64, y: f64, z: f64| DVec3::new(x, y, z);
        let mut edges: Vec<Vec<DVec3>> = Vec::new();
        for z in [0.0, 2.0] {
            edges.extend([
                vec![c(0.0, 0.0, z), c(4.0, 0.0, z)],
                vec![c(4.0, 0.0, z), c(4.0, 3.0, z)],
                vec![c(4.0, 3.0, z), c(0.0, 3.0, z)],
                vec![c(0.0, 3.0, z), c(0.0, 0.0, z)],
            ]);
        }
        for (x, y) in [(0.0, 0.0), (4.0, 0.0), (4.0, 3.0), (0.0, 3.0)] {
            edges.push(vec![c(x, y, 0.0), c(x, y, 2.0)]);
        }
        let (o, u, v) = flat_xy();
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        let edges: Vec<(Option<EdgeRef>, Vec<DVec3>)> = edges.into_iter().map(|e| (None, e)).collect();
        assert_eq!(project_all(&mut sk, o, u, v, &edges), 4);
        assert_eq!(sk.entities.len(), 4);
        assert!((0..4).all(|e| sk.is_construction(e)));
        // Doing it again adds nothing.
        assert_eq!(project_all(&mut sk, o, u, v, &edges), 0);
    }

    #[test]
    fn other_curves_are_refused() {
        let (o, u, v) = flat_xy();
        // A circle seen at an angle (here: squashed) is an ellipse.
        let tilted: Vec<DVec3> = (0..=24).map(|k| { let t = k as f64 / 24.0 * std::f64::consts::TAU; DVec3::new(3.0 * t.cos(), 2.0 * t.sin(), 0.0) }).collect();
        assert!(project_edge(o, u, v, &tilted).is_err());
        assert!(project_edge(o, u, v, &[]).is_err());
    }

    #[test]
    fn projected_geometry_is_construction_locked_and_not_added_twice() {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        let line = Projected::Line(DVec2::new(1.0, 1.0), DVec2::new(6.0, 1.0));
        let fixes = sk.add_projected(line).unwrap();
        assert_eq!(fixes.len(), 2);
        assert!(sk.is_construction(0), "construction, so it doesn't make regions");
        assert!(!sk.is_projected(0), "until it's locked");
        for c in fixes {
            sk.constraints.push(c);
        }
        assert!(sk.is_projected(0), "locked construction geometry reads as projected");
        assert!(sk.add_projected(line).is_err());
        assert!(sk.add_projected(Projected::Line(DVec2::new(6.0, 1.0), DVec2::new(1.0, 1.0))).is_err(), "either direction");
        let arc = Projected::Arc { center: DVec2::ZERO, start: DVec2::new(3.0, 0.0), end: DVec2::new(0.0, 3.0) };
        assert_eq!(sk.add_projected(arc).unwrap().len(), 3);
        assert_eq!(sk.entities.len(), 2);
    }

    fn linked_line() -> (Sketch, EdgeRef) {
        let r = EdgeRef { mid: DVec3::new(5.0, 0.0, 0.0), dir: DVec3::X, ..Default::default() };
        let (o, u, v) = flat_xy();
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        let edges = vec![(Some(r), vec![DVec3::ZERO, DVec3::new(10.0, 0.0, 0.0)])];
        assert_eq!(project_all(&mut sk, o, u, v, &edges), 1);
        assert_eq!(sk.projections, vec![(0, r)]);
        (sk, r)
    }

    #[test]
    fn a_link_follows_its_edge_whichever_way_the_edge_runs() {
        let (sk, _) = linked_line();
        let (o, u, v) = flat_xy();
        // Unmoved: nothing to do.
        let (m, lost, err) = follow_links(&sk, o, u, v, |_| Some(vec![DVec3::ZERO, DVec3::new(10.0, 0.0, 0.0)]));
        assert!(m.is_none() && lost.is_empty() && err.is_none());
        // Longer, and listed end first: each end goes to its nearer new end.
        let (m, _, _) = follow_links(&sk, o, u, v, |_| Some(vec![DVec3::new(14.0, 0.0, 3.0), DVec3::ZERO]));
        let m = m.expect("moved");
        let Entity::Line { a, b } = m.entities[0] else { panic!() };
        assert_eq!((m.points[a], m.points[b]), (DVec2::ZERO, DVec2::new(14.0, 0.0)));
        // Gone, or now a circle: lost, left where it was.
        let (m, lost, _) = follow_links(&sk, o, u, v, |_| None);
        assert!(m.is_none());
        assert_eq!(lost, vec![0]);
    }

    #[test]
    fn deleting_geometry_renumbers_the_links() {
        let (mut sk, r) = linked_line();
        // A line drawn after it, another projected one, then the first deleted.
        sk.points.extend([DVec2::new(0.0, 5.0), DVec2::new(5.0, 5.0)]);
        let n = sk.points.len();
        sk.entities.push(Entity::Line { a: n - 2, b: n - 1 });
        let (o, u, v) = flat_xy();
        let edges = vec![(Some(EdgeRef { mid: DVec3::new(0.0, 3.0, 0.0), ..r }), vec![DVec3::new(0.0, 1.0, 0.0), DVec3::new(0.0, 5.0, 0.0)])];
        assert_eq!(project_all(&mut sk, o, u, v, &edges), 1);
        assert_eq!(sk.projections.iter().map(|p| p.0).collect::<Vec<_>>(), vec![0, 2]);
        sk.delete(&[crate::sketch::Sel::Entity(0)]);
        assert_eq!(sk.projections.iter().map(|p| p.0).collect::<Vec<_>>(), vec![1], "the first link went with its line; the other shifted down");
    }

    #[test]
    fn projected_geometry_cant_be_dimensioned_on_its_own() {
        use crate::sketch::Sel;
        let (mut sk, _) = linked_line();
        assert!(sk.is_projected(0), "locked by project_all");
        sk.points.extend([DVec2::new(0.0, 5.0), DVec2::new(8.0, 9.0)]);
        sk.entities.push(Entity::Line { a: 3, b: 4 });
        let dims = |sel: &[Sel]| sk.constraint_options(sel).into_iter().filter(|(l, _)| *l == "Dimension").count();
        assert_eq!(dims(&[Sel::Entity(0)]), 0, "the projected line");
        assert_eq!(dims(&[Sel::Point(1), Sel::Point(2)]), 0, "its two ends");
        assert_eq!(dims(&[Sel::Point(1), Sel::Point(0)]), 0, "its end and the origin");
        assert_eq!(dims(&[Sel::Entity(1)]), 1, "your own line");
        assert_eq!(dims(&[Sel::Point(2), Sel::Point(4)]), 1, "from it to your own point");
        assert_eq!(dims(&[Sel::Entity(0), Sel::Entity(1)]), 1, "between it and your line");
    }
}
