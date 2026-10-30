//! 3D print checks (INSPECT → Print Check): the printer's build volume and
//! whether the part fits, overhangs that need support (shaded yellow at the
//! limit angle to red for a flat ceiling), and the faces worth laying it on.
//! Worked on the bodies' meshes; all lengths mm, Z up, the bed at the
//! lowest point of the part.

use crate::model::Mesh;
use glam::{DVec3, Vec3};

/// A printer: its name and build volume (X, Y, Z).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Printer {
    pub name: &'static str,
    pub size: [f64; 3],
}

/// Common printers (makers' published build volumes).
pub const PRINTERS: &[Printer] = &[
    Printer { name: "Bambu Lab P1S", size: [256.0, 256.0, 256.0] },
    Printer { name: "Bambu Lab P1P", size: [256.0, 256.0, 256.0] },
    Printer { name: "Bambu Lab X1 Carbon", size: [256.0, 256.0, 256.0] },
    Printer { name: "Bambu Lab A1", size: [256.0, 256.0, 256.0] },
    Printer { name: "Bambu Lab A1 mini", size: [180.0, 180.0, 180.0] },
    Printer { name: "Prusa MK4", size: [250.0, 210.0, 220.0] },
    Printer { name: "Prusa MINI+", size: [180.0, 180.0, 180.0] },
    Printer { name: "Creality Ender-3 V2", size: [220.0, 220.0, 250.0] },
];

/// The printer by name; anything else is the custom size.
pub fn printer(name: &str, custom: [f64; 3]) -> Printer {
    PRINTERS.iter().copied().find(|p| p.name == name).unwrap_or(Printer { name: "Custom", size: custom })
}

/// How far past the overhang limit a face with outward normal `n` hangs:
/// None when it prints without support, else 0 (just past `limit_deg`, the
/// most a face may lean out from upright) to 1 (a flat ceiling).
pub fn overhang(n: Vec3, limit_deg: f64) -> Option<f32> {
    let down = -n.z as f64; // how much it faces the bed
    let lean = down.clamp(-1.0, 1.0).asin().to_degrees(); // 0: upright wall, 90: ceiling
    (lean > limit_deg + 1e-6).then(|| ((lean - limit_deg) / (90.0 - limit_deg).max(1e-6)).clamp(0.0, 1.0) as f32)
}

/// Shade for an overhang: yellow just past the limit to red for a ceiling.
pub fn overhang_color(t: f32) -> [f32; 3] {
    let (a, b) = ([0.95, 0.80, 0.25], [0.86, 0.18, 0.15]);
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

fn area(m: &Mesh, t: usize) -> f64 {
    let [a, b, c] = m.tris[t].map(|i| m.positions[i as usize].as_dvec3());
    (b - a).cross(c - a).length() / 2.0
}

/// Each triangle's overhang (see `overhang`), with those lying on the bed
/// (at `bed_z`) left out, and the total area needing support.
pub fn mesh_overhangs(m: &Mesh, limit_deg: f64, bed_z: f64, tol: f64) -> (Vec<Option<f32>>, f64) {
    let mut support = 0.0;
    let marks = (0..m.tris.len())
        .map(|t| {
            let on_bed = m.tris[t].iter().all(|&i| (m.positions[i as usize].z as f64 - bed_z).abs() <= tol);
            let o = if on_bed { None } else { overhang(m.normal(t), limit_deg) };
            if o.is_some() {
                support += area(m, t);
            }
            o
        })
        .collect();
    (marks, support)
}

/// The bodies' box: (lowest corner, highest corner).
pub fn bounds(meshes: &[&Mesh]) -> Option<(DVec3, DVec3)> {
    let mut it = meshes.iter().flat_map(|m| m.positions.iter().map(|p| p.as_dvec3()));
    let first = it.next()?;
    Some(it.fold((first, first), |(lo, hi), p| (lo.min(p), hi.max(p))))
}

/// Does a part this size (X, Y, Z) fit the printer, turned on the bed if
/// need be? None if it fits, else how much too big it is on each axis.
pub fn too_big(size: DVec3, p: &Printer) -> Option<DVec3> {
    let [x, y, z] = p.size;
    let fits = |a: f64, b: f64| size.x <= a + 1e-9 && size.y <= b + 1e-9 && size.z <= z + 1e-9;
    if fits(x, y) || fits(y, x) {
        return None;
    }
    Some((size - DVec3::new(x, y, z)).max(DVec3::ZERO))
}

/// One way to lay the part on the bed: the face direction that goes down.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Orientation {
    /// The outward normal of the face that goes on the bed.
    pub down: DVec3,
    /// How much of the part touches the bed (mm²).
    pub contact: f64,
    /// Overhang area needing support (mm²).
    pub support: f64,
    /// How tall it stands (mm).
    pub height: f64,
}

/// The best ways to lay the part, best first: on flat faces that the
/// whole part sits on (nothing would go through the bed), least support
/// first, then most bed contact, then shortest.
pub fn orientations(meshes: &[&Mesh], limit_deg: f64, count: usize) -> Vec<Orientation> {
    let Some((lo, hi)) = bounds(meshes) else { return Vec::new() };
    let tol = (hi - lo).length() * 1e-5 + 1e-6;
    let verts: Vec<DVec3> = meshes.iter().flat_map(|m| m.positions.iter().map(|p| p.as_dvec3())).collect();
    let tris: Vec<(DVec3, f64, [DVec3; 3])> = meshes
        .iter()
        .flat_map(|m| (0..m.tris.len()).map(move |t| (m.normal(t).as_dvec3(), area(m, t), m.tris[t].map(|i| m.positions[i as usize].as_dvec3()))))
        .filter(|(n, a, _)| *a > 0.0 && n.length() > 0.5)
        .collect();
    // Face directions, grouped (curved faces give many small groups; only the big ones are tried).
    let mut groups: Vec<(DVec3, f64)> = Vec::new();
    for (n, a, _) in &tris {
        match groups.iter_mut().find(|(g, _)| g.dot(*n) > 0.99995) {
            Some(g) => g.1 += a,
            None => groups.push((*n, *a)),
        }
    }
    let total: f64 = tris.iter().map(|t| t.1).sum();
    groups.sort_by(|a, b| b.1.total_cmp(&a.1));
    let mut out: Vec<Orientation> = Vec::new();
    for &(d, _) in groups.iter().take(64).filter(|g| g.1 > total * 1e-3) {
        // The face's plane must have the whole part on one side: it sits on it.
        let top = verts.iter().map(|v| v.dot(d)).fold(f64::MIN, f64::max);
        let bottom = verts.iter().map(|v| v.dot(d)).fold(f64::MAX, f64::min);
        let contact: f64 = tris.iter().filter(|(n, _, c)| n.dot(d) > 0.99995 && c.iter().all(|v| v.dot(d) >= top - tol)).map(|t| t.1).sum();
        if contact <= total * 1e-4 {
            continue; // the plane only touches an edge or a point
        }
        // Laid down that way, a face leans out by asin(n · d) from upright.
        let support: f64 = tris
            .iter()
            .filter(|(n, _, c)| {
                let on_bed = c.iter().all(|v| v.dot(d) >= top - tol);
                let lean = n.dot(d).clamp(-1.0, 1.0).asin().to_degrees();
                !on_bed && lean > limit_deg + 1e-6
            })
            .map(|t| t.1)
            .sum();
        out.push(Orientation { down: d, contact, support, height: top - bottom });
    }
    // Least support (within a few mm²), then most contact, then shortest.
    out.sort_by(|a, b| {
        let s = |o: &Orientation| (o.support / 5.0).round();
        s(a).total_cmp(&s(b)).then(b.contact.total_cmp(&a.contact)).then(a.height.total_cmp(&b.height))
    });
    // Ways that come out the same (a plate's top and bottom) are shown once.
    let same = |a: &Orientation, b: &Orientation| {
        let near = |x: f64, y: f64| (x - y).abs() <= 0.01 * x.abs().max(y.abs()) + 0.5;
        near(a.support, b.support) && near(a.contact, b.contact) && near(a.height, b.height)
    };
    let mut kept: Vec<Orientation> = Vec::new();
    for o in out {
        if !kept.iter().any(|k| same(k, &o)) {
            kept.push(o);
        }
    }
    kept.truncate(count);
    kept
}

/// The X, Y, Z turns (degrees, applied in that order, as Move's Free Move)
/// that point `down` straight down.
pub fn turn_down(down: DVec3) -> [f64; 3] {
    let d = down.normalize();
    // About X: the direction into the XZ plane, pointing down…
    let a = (-d.y).atan2(-d.z);
    let r = (d.y * d.y + d.z * d.z).sqrt();
    // …then about Y: straight down.
    let b = d.x.atan2(r);
    [a.to_degrees(), b.to_degrees(), 0.0]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A closed box mesh (outward triangles) from lo to hi.
    fn cube(lo: Vec3, hi: Vec3) -> Mesh {
        let p = |x: bool, y: bool, z: bool| Vec3::new(if x { hi.x } else { lo.x }, if y { hi.y } else { lo.y }, if z { hi.z } else { lo.z });
        let positions = vec![p(false, false, false), p(true, false, false), p(true, true, false), p(false, true, false), p(false, false, true), p(true, false, true), p(true, true, true), p(false, true, true)];
        let quads = [[0, 3, 2, 1], [4, 5, 6, 7], [0, 1, 5, 4], [1, 2, 6, 5], [2, 3, 7, 6], [3, 0, 4, 7]];
        let tris = quads.iter().flat_map(|q| [[q[0], q[1], q[2]], [q[0], q[2], q[3]]]).collect();
        Mesh { positions, tris }
    }

    #[test]
    fn overhangs_by_angle() {
        assert_eq!(overhang(Vec3::X, 45.0), None, "an upright wall");
        assert_eq!(overhang(Vec3::Z, 45.0), None, "a top face");
        assert_eq!(overhang(-Vec3::Z, 45.0), Some(1.0), "a ceiling");
        // Leaning out 30° from upright: fine at 45, not at 20.
        let n = Vec3::new(30f32.to_radians().cos(), 0.0, -30f32.to_radians().sin());
        assert_eq!(overhang(n, 45.0), None);
        let t = overhang(n, 20.0).unwrap();
        assert!((t - 10.0 / 70.0).abs() < 1e-4, "{t}");
    }

    #[test]
    fn a_floating_block_needs_support_under_it_but_not_on_the_bed() {
        let m = cube(Vec3::ZERO, Vec3::new(10.0, 20.0, 5.0));
        let (marks, support) = mesh_overhangs(&m, 45.0, 0.0, 1e-4);
        assert_eq!(support, 0.0, "its bottom is on the bed");
        assert!(marks.iter().all(Option::is_none));
        let (_, support) = mesh_overhangs(&m, 45.0, -1.0, 1e-4);
        assert!((support - 200.0).abs() < 1e-6, "lifted: the bottom hangs: {support}");
    }

    #[test]
    fn fits_the_printer_turned_if_need_be() {
        let p = printer("Prusa MK4", [0.0; 3]);
        assert_eq!(too_big(DVec3::new(240.0, 200.0, 100.0), &p), None);
        assert_eq!(too_big(DVec3::new(205.0, 245.0, 100.0), &p), None, "turned 90° on the bed");
        assert_eq!(too_big(DVec3::new(260.0, 100.0, 230.0), &p), Some(DVec3::new(10.0, 0.0, 10.0)));
        assert_eq!(printer("nobody's", [100.0, 90.0, 80.0]).size, [100.0, 90.0, 80.0]);
    }

    #[test]
    fn an_l_bracket_lies_on_its_long_back() {
        // An L: a 60 × 10 × 10 bar with a 10 × 10 × 40 post up from one end.
        let mut m = cube(Vec3::ZERO, Vec3::new(60.0, 10.0, 10.0));
        m.append(&cube(Vec3::new(0.0, 0.0, 10.0), Vec3::new(10.0, 10.0, 50.0)));
        let best = orientations(&[&m], 45.0, 3);
        assert!(!best.is_empty());
        // Every suggestion sits flat: nothing below the face put down.
        for o in &best {
            let lowest = m.positions.iter().map(|p| p.as_dvec3().dot(o.down)).fold(f64::MIN, f64::max);
            assert!(m.positions.iter().all(|p| p.as_dvec3().dot(o.down) <= lowest + 1e-6));
        }
        // Lying on its side (±Y): the whole L is on the bed, no support.
        assert!(best[0].down.y.abs() > 0.999 && best[0].support < 1e-6, "{best:?}");
        // The post's top (+Z) is a face, but the bar would go through the bed: never offered.
        assert!(best.iter().all(|o| o.down.z < 0.999));
        // Its two sides (±Y) come out the same: offered once.
        assert_eq!(best.iter().filter(|o| o.down.y.abs() > 0.999).count(), 1, "{best:?}");
    }

    #[test]
    fn turning_down_points_the_face_at_the_bed() {
        let rot = |a: [f64; 3], v: DVec3| {
            let v = glam::DQuat::from_rotation_x(a[0].to_radians()) * v;
            let v = glam::DQuat::from_rotation_y(a[1].to_radians()) * v;
            glam::DQuat::from_rotation_z(a[2].to_radians()) * v
        };
        for d in [DVec3::X, -DVec3::X, DVec3::Y, -DVec3::Y, DVec3::Z, -DVec3::Z, DVec3::new(1.0, -2.0, 0.5).normalize()] {
            let got = rot(turn_down(d), d);
            assert!((got + DVec3::Z).length() < 1e-9, "{d} > {got}");
        }
        assert_eq!(turn_down(-DVec3::Z), [0.0, 0.0, 0.0], "already down: left alone");
    }
}
