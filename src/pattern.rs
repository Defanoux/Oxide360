//! Sketch Mirror: copies of selected geometry reflected across a line.

use crate::sketch::{Constraint, Entity, EntityId, PointId, Sel, Sketch};
use glam::DVec2;
use std::collections::HashMap;

/// `p` reflected across the line through `a` and `b`.
pub fn reflect(p: DVec2, a: DVec2, b: DVec2) -> DVec2 {
    let d = (b - a).normalize_or_zero();
    let v = p - a;
    a + d * (2.0 * v.dot(d)) - v
}

/// Mirror the selected entities across line `line`. Points shared between
/// selected entities stay shared in the copy, and points lying on the mirror
/// line are reused (so the copy stays joined to the original there). Returns
/// the constraints worth adding (equal length / radius to each original) for
/// the caller to add one at a time through the solver, and the new entities.
pub fn mirror(sk: &mut Sketch, objects: &[Sel], line: EntityId) -> Result<(Vec<Constraint>, Vec<EntityId>), String> {
    let Some(&Entity::Line { a, b }) = sk.entities.get(line) else {
        return Err("the mirror line must be a line".into());
    };
    let (pa, pb) = (sk.points[a], sk.points[b]);
    if pa.distance(pb) < 1e-9 {
        return Err("the mirror line has no length".into());
    }
    let mut chosen: Vec<EntityId> = Vec::new();
    for s in objects {
        let found = match *s {
            Sel::Entity(e) => Some(e),
            Sel::Point(p) => sk.entities.iter().position(|e| matches!(e, Entity::Point { p: q } if *q == p)),
        };
        if let Some(e) = found.filter(|&e| e != line && e < sk.entities.len() && !chosen.contains(&e)) {
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
    let mut constraints = Vec::new();
    let mut made = Vec::new();
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
            constraints.push(Constraint::Equal(e, n));
        }
        made.push(n);
    }
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
        let (cs, made) = mirror(&mut sk, &[Sel::Entity(1), Sel::Entity(2)], 0).unwrap();
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
        let (_, made) = mirror(&mut sk, &[Sel::Entity(1)], 0).unwrap();
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
        let (cs, made) = mirror(&mut sk, &[Sel::Entity(1), Sel::Entity(2)], 0).unwrap();
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

    #[test]
    fn bad_input_is_refused() {
        let mut sk = lines(&[((0.0, 0.0), (0.0, 5.0)), ((1.0, 0.0), (2.0, 0.0))]);
        assert!(mirror(&mut sk, &[], 0).is_err());
        assert!(mirror(&mut sk, &[Sel::Entity(0)], 0).is_err(), "the line alone is nothing to mirror");
        sk.entities.push(Entity::Circle { center: 0, radius: 1.0 });
        assert!(mirror(&mut sk, &[Sel::Entity(1)], 2).is_err(), "a circle is not a mirror line");
    }
}
