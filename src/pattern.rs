//! Sketch Mirror: copies of selected geometry reflected across a line or an
//! origin axis, with the constraints between the selected pieces carried over.

use crate::sketch::{Constraint, Entity, EntityId, PointId, Sel, Sketch};
use glam::DVec2;
use std::collections::HashMap;

/// `p` reflected across the line through `a` and `b`.
pub fn reflect(p: DVec2, a: DVec2, b: DVec2) -> DVec2 {
    let d = (b - a).normalize_or_zero();
    let v = p - a;
    a + d * (2.0 * v.dot(d)) - v
}

/// What the copies are reflected across.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MirrorLine {
    /// A line of the sketch.
    Entity(EntityId),
    /// The sketch's X (0) or Y (1) axis through the origin.
    Axis(usize),
}

/// Mirror the selected entities across line `line`. Points shared between
/// selected entities stay shared in the copy, and points lying on the mirror
/// line are reused (so the copy stays joined to the original there). Returns
/// the constraints worth adding (equal length / radius to each original) for
/// the caller to add one at a time through the solver, and the new entities.
pub fn mirror(sk: &mut Sketch, objects: &[Sel], across: MirrorLine) -> Result<(Vec<Constraint>, Vec<EntityId>), String> {
    let (pa, pb, line) = match across {
        MirrorLine::Entity(line) => {
            let Some(&Entity::Line { a, b }) = sk.entities.get(line) else {
                return Err("the mirror line must be a line".into());
            };
            (sk.points[a], sk.points[b], Some(line))
        }
        MirrorLine::Axis(0) => (DVec2::ZERO, DVec2::X, None),
        MirrorLine::Axis(_) => (DVec2::ZERO, DVec2::Y, None),
    };
    if pa.distance(pb) < 1e-9 {
        return Err("the mirror line has no length".into());
    }
    let mut chosen: Vec<EntityId> = Vec::new();
    for s in objects {
        let found = match *s {
            Sel::Entity(e) => Some(e),
            Sel::Point(p) => sk.entities.iter().position(|e| matches!(e, Entity::Point { p: q } if *q == p)),
        };
        if let Some(e) = found.filter(|&e| Some(e) != line && e < sk.entities.len() && !chosen.contains(&e)) {
            chosen.push(e);
        }
    }
    if chosen.is_empty() {
        return Err("select geometry to mirror first".into());
    }
    let tol = 1e-7;
    let mut map: HashMap<PointId, PointId> = HashMap::new();
    let mut image = |sk: &mut Sketch, p: PointId| -> PointId {
        *map.entry(p).or_insert_with(|| {
            let at = sk.points[p];
            let r = reflect(at, pa, pb);
            if r.distance(at) < tol {
                p
            } else {
                sk.points.push(r);
                sk.points.len() - 1
            }
        })
    };
    let mut equal = Vec::new();
    let mut made = Vec::new();
    let mut emap: HashMap<EntityId, EntityId> = HashMap::new();
    for e in chosen {
        let copy = match sk.entities[e] {
            Entity::Line { a, b } => Entity::Line { a: image(sk, a), b: image(sk, b) },
            Entity::Circle { center, radius } => Entity::Circle { center: image(sk, center), radius },
            Entity::Point { p } => Entity::Point { p: image(sk, p) },
            // Reflection reverses the sweep, so the new arc runs from the image of b to the image of a.
            Entity::Arc { center, a, b } => Entity::Arc { center: image(sk, center), a: image(sk, b), b: image(sk, a) },
        };
        let was_construction = sk.is_construction(e);
        sk.entities.push(copy);
        let n = sk.entities.len() - 1;
        if was_construction {
            sk.set_construction(n, true);
        }
        if matches!(copy, Entity::Line { .. } | Entity::Circle { .. }) {
            equal.push(Constraint::Equal(e, n));
        }
        emap.insert(e, n);
        made.push(n);
    }
    // The constraints among the selected pieces apply to their copies too, so
    // the copy keeps the shape (a rectangle stays a rectangle) …
    let theta = (pb - pa).to_angle();
    // A horizontal line (0°) mirrors to 2θ, a vertical one (90°) to 2θ − 90°.
    let keeps = |deg: f64| -> Option<bool> {
        let r = (2.0 * theta.to_degrees() - deg).rem_euclid(180.0);
        let near = |t: f64| (r - t).abs() < 1e-7 || (r - t - 180.0).abs() < 1e-7;
        if near(0.0) { Some(true) } else if near(90.0) { Some(false) } else { None }
    };
    let e = |x: &EntityId| emap.get(x).copied();
    let q = |x: &PointId| map.get(x).copied();
    let mut constraints = Vec::new();
    for c in sk.constraints.clone() {
        use Constraint::*;
        let copy = match c {
            Horizontal(l) => match (e(&l), keeps(0.0)) {
                (Some(m), Some(true)) => Some(Horizontal(m)),
                (Some(m), Some(false)) => Some(Vertical(m)),
                _ => None,
            },
            Vertical(l) => match (e(&l), keeps(90.0)) {
                (Some(m), Some(true)) => Some(Horizontal(m)),
                (Some(m), Some(false)) => Some(Vertical(m)),
                _ => None,
            },
            Parallel(a, b) => e(&a).zip(e(&b)).map(|(a, b)| Parallel(a, b)),
            Perpendicular(a, b) => e(&a).zip(e(&b)).map(|(a, b)| Perpendicular(a, b)),
            Equal(a, b) => e(&a).zip(e(&b)).map(|(a, b)| Equal(a, b)),
            Tangent(l, c) => e(&l).zip(e(&c)).map(|(l, c)| Tangent(l, c)),
            OnCircle(p, c) => q(&p).zip(e(&c)).map(|(p, c)| OnCircle(p, c)),
            OnLine(p, l) => q(&p).zip(e(&l)).map(|(p, l)| OnLine(p, l)),
            Midpoint(p, a, b) => match (q(&p), q(&a), q(&b)) {
                (Some(p), Some(a), Some(b)) => Some(Midpoint(p, a, b)),
                _ => None,
            },
            // Sizes aren't repeated: the copy is made equal to the original instead.
            _ => None,
        };
        if let Some(c) = copy {
            constraints.push(c);
        }
    }
    // … and each copy is equal to its original.
    constraints.extend(equal);
    Ok((constraints, made))
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

    #[test]
    fn a_point_reflects_across_a_slanted_line() {
        let r = reflect(DVec2::new(1.0, 0.0), DVec2::ZERO, DVec2::new(1.0, 1.0));
        assert!(r.distance(DVec2::new(0.0, 1.0)) < 1e-12);
        let r = reflect(DVec2::new(3.0, 2.0), DVec2::new(0.0, 5.0), DVec2::new(10.0, 5.0));
        assert!(r.distance(DVec2::new(3.0, 8.0)) < 1e-12);
    }

    #[test]
    fn mirrored_lines_are_reflected_and_keep_their_joints() {
        // An L joined at (4,0), right of the vertical mirror line x = 2.
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        sk.points.extend([DVec2::new(2.0, -5.0), DVec2::new(2.0, 5.0), DVec2::new(4.0, 0.0), DVec2::new(6.0, 0.0), DVec2::new(4.0, 3.0)]);
        sk.entities.push(Entity::Line { a: 1, b: 2 });
        sk.entities.push(Entity::Line { a: 3, b: 4 });
        sk.entities.push(Entity::Line { a: 3, b: 5 });
        let before = sk.entities.len();
        let (cs, made) = mirror(&mut sk, &[Sel::Entity(1), Sel::Entity(2)], MirrorLine::Entity(0)).unwrap();
        assert_eq!(made.len(), 2);
        assert_eq!(sk.entities.len(), before + 2);
        let ends: Vec<(DVec2, DVec2)> = made
            .iter()
            .map(|&e| match sk.entities[e] {
                Entity::Line { a, b } => (sk.points[a], sk.points[b]),
                _ => panic!(),
            })
            .collect();
        assert!(ends[0].0.distance(DVec2::new(0.0, 0.0)) < 1e-12 && ends[0].1.distance(DVec2::new(-2.0, 0.0)) < 1e-12);
        assert!(ends[1].0.distance(DVec2::new(0.0, 0.0)) < 1e-12 && ends[1].1.distance(DVec2::new(0.0, 3.0)) < 1e-12);
        // The two copies share their joint, like the originals.
        let joint = |e: EntityId| match sk.entities[e] {
            Entity::Line { a, .. } => a,
            _ => unreachable!(),
        };
        assert_eq!(joint(made[0]), joint(made[1]));
        assert_eq!(cs, vec![Constraint::Equal(1, made[0]), Constraint::Equal(2, made[1])]);
    }

    #[test]
    fn points_on_the_mirror_line_are_reused() {
        let mut sk = lines(&[((0.0, -5.0), (0.0, 5.0)), ((0.0, 1.0), (3.0, 1.0))]);
        let points = sk.points.len();
        let (_, made) = mirror(&mut sk, &[Sel::Entity(1)], MirrorLine::Entity(0)).unwrap();
        assert_eq!(sk.points.len(), points + 1, "only the far end is new");
        let Entity::Line { a, .. } = sk.entities[made[0]] else { panic!() };
        let Entity::Line { a: orig, .. } = sk.entities[1] else { panic!() };
        assert_eq!(a, orig, "joined to the original on the mirror line");
    }

    #[test]
    fn circles_arcs_and_construction_flags_are_mirrored() {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        sk.points.extend([DVec2::new(0.0, -5.0), DVec2::new(0.0, 5.0), DVec2::new(4.0, 0.0), DVec2::new(5.0, 0.0), DVec2::new(4.0, 1.0)]);
        sk.entities.push(Entity::Line { a: 1, b: 2 });
        sk.entities.push(Entity::Circle { center: 3, radius: 2.0 });
        sk.entities.push(Entity::Arc { center: 3, a: 4, b: 5 });
        sk.set_construction(1, true);
        let (cs, made) = mirror(&mut sk, &[Sel::Entity(1), Sel::Entity(2)], MirrorLine::Entity(0)).unwrap();
        assert_eq!(cs, vec![Constraint::Equal(1, made[0])], "arcs get no equal constraint");
        let Entity::Circle { center, radius } = sk.entities[made[0]] else { panic!() };
        assert_eq!(radius, 2.0);
        assert!(sk.points[center].distance(DVec2::new(-4.0, 0.0)) < 1e-12);
        assert!(sk.is_construction(made[0]) && !sk.is_construction(made[1]));
        // The arc ran counter-clockwise from (5,0) to (4,1); mirrored it runs from the image of
        // (4,1) = (-4,1) to the image of (5,0) = (-5,0), still counter-clockwise.
        let Entity::Arc { a, b, .. } = sk.entities[made[1]] else { panic!() };
        assert!(sk.points[a].distance(DVec2::new(-4.0, 1.0)) < 1e-12);
        assert!(sk.points[b].distance(DVec2::new(-5.0, 0.0)) < 1e-12);
    }


    /// A 4×2 rectangle at (2,0): four lines sharing corners, bottom/top horizontal,
    /// sides vertical, opposite sides equal.
    fn rectangle() -> Sketch {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        sk.points.extend([DVec2::new(2.0, 0.0), DVec2::new(6.0, 0.0), DVec2::new(6.0, 2.0), DVec2::new(2.0, 2.0)]);
        for (a, b) in [(1, 2), (2, 3), (3, 4), (4, 1)] {
            sk.entities.push(Entity::Line { a, b });
        }
        sk.constraints.extend([
            Constraint::Horizontal(0),
            Constraint::Horizontal(2),
            Constraint::Vertical(1),
            Constraint::Vertical(3),
            Constraint::Equal(0, 2),
            Constraint::Equal(1, 3),
        ]);
        sk
    }

    #[test]
    fn constraints_between_the_selected_pieces_carry_over_to_the_copy() {
        let mut sk = rectangle();
        let all: Vec<Sel> = (0..4).map(Sel::Entity).collect();
        // Mirror across the Y axis: horizontals stay horizontal, verticals vertical.
        let (cs, made) = mirror(&mut sk, &all, MirrorLine::Axis(1)).unwrap();
        let (h, v, eq) = (
            |m: usize| cs.contains(&Constraint::Horizontal(m)),
            |m: usize| cs.contains(&Constraint::Vertical(m)),
            |a: usize, b: usize| cs.contains(&Constraint::Equal(a, b)),
        );
        assert!(h(made[0]) && h(made[2]) && v(made[1]) && v(made[3]));
        assert!(eq(made[0], made[2]) && eq(made[1], made[3]), "equal sides stay equal");
        assert!(eq(0, made[0]) && eq(3, made[3]), "and each copy equals its original");
        // The copy sits at x −6..−2, and the solver accepts all of it.
        let mut t = sk.clone();
        for c in cs {
            let mut trial = t.clone();
            if crate::solver::add_constraints(&mut trial, &[c]).is_ok() {
                t = trial;
            }
        }
        let Entity::Line { a, .. } = t.entities[made[0]] else { panic!() };
        assert!((t.points[a].x + 2.0).abs() < 1e-9 || (t.points[a].x + 6.0).abs() < 1e-9);
    }

    #[test]
    fn horizontal_and_vertical_follow_the_mirror_lines_angle() {
        // Across a 45° line they swap; across a 30° line there's no H/V left to keep.
        for (deg, swapped, none) in [(45.0f64, true, false), (30.0, false, true), (0.0, false, false), (90.0, false, false)] {
            let mut sk = rectangle();
            let d = DVec2::from_angle(deg.to_radians()) * 10.0;
            sk.points.extend([DVec2::new(0.0, -20.0), DVec2::new(0.0, -20.0) + d]);
            sk.entities.push(Entity::Line { a: 5, b: 6 });
            let (cs, made) = mirror(&mut sk, &[Sel::Entity(0), Sel::Entity(1)], MirrorLine::Entity(4)).unwrap();
            let any_h = cs.contains(&Constraint::Horizontal(made[0])) || cs.contains(&Constraint::Vertical(made[1]));
            let swap = cs.contains(&Constraint::Vertical(made[0])) && cs.contains(&Constraint::Horizontal(made[1]));
            assert_eq!(swap, swapped, "{deg}");
            assert_eq!(!any_h && !swap, none, "{deg}");
            if !swapped && !none {
                assert!(any_h, "{deg}");
            }
        }
    }

    #[test]
    fn mirroring_across_an_origin_axis_reuses_points_on_it() {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        sk.points.extend([DVec2::new(0.0, 3.0), DVec2::new(5.0, 6.0)]);
        sk.entities.push(Entity::Line { a: 1, b: 2 });
        // Across the Y axis (x = 0): the end on the axis is shared, the other lands at x = −5.
        let (_, made) = mirror(&mut sk, &[Sel::Entity(0)], MirrorLine::Axis(1)).unwrap();
        let Entity::Line { a, b } = sk.entities[made[0]] else { panic!() };
        assert_eq!(a, 1);
        assert!(sk.points[b].distance(DVec2::new(-5.0, 6.0)) < 1e-12);
        // Across the X axis (y = 0).
        let (_, made) = mirror(&mut sk, &[Sel::Entity(0)], MirrorLine::Axis(0)).unwrap();
        let Entity::Line { a, b } = sk.entities[made[0]] else { panic!() };
        assert!(sk.points[a].distance(DVec2::new(0.0, -3.0)) < 1e-12 && sk.points[b].distance(DVec2::new(5.0, -6.0)) < 1e-12);
    }

    #[test]
    fn bad_input_is_refused() {
        let mut sk = lines(&[((0.0, 0.0), (0.0, 5.0)), ((1.0, 0.0), (2.0, 0.0))]);
        assert!(mirror(&mut sk, &[], MirrorLine::Entity(0)).is_err());
        assert!(mirror(&mut sk, &[Sel::Entity(0)], MirrorLine::Entity(0)).is_err(), "the line alone is nothing to mirror");
        sk.entities.push(Entity::Circle { center: 0, radius: 1.0 });
        assert!(mirror(&mut sk, &[Sel::Entity(1)], MirrorLine::Entity(2)).is_err(), "a circle is not a mirror line");
    }
}
