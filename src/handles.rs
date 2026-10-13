//! On-canvas drag handles (the arrows beside the extrude / fillet / chamfer /
//! revolve boxes): where they sit and how a cursor ray turns into a value.
//! Pure geometry, so it is tested here; `main.rs` only draws and wires input.

use crate::model::EdgeRef;
use glam::DVec3;

/// An arrow that slides along a line: value = distance from `origin` along `dir`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LineHandle {
    pub origin: DVec3,
    pub dir: DVec3,
}

/// The fillet / chamfer arrow for a picked edge, along the bisector of its
/// two faces: into the part on an outside corner (where it takes material
/// off), out into the gap on an inside corner (where it adds material).
/// `inside` says whether a point is in the part. None when the faces aren't
/// known (older files) or the edge is between faces that face opposite ways.
pub fn edge_handle(e: &EdgeRef, size: f64, inside: impl Fn(DVec3) -> bool) -> Option<LineHandle> {
    if e.faces[0] == DVec3::ZERO || e.faces[1] == DVec3::ZERO {
        return None;
    }
    let inward = -(e.faces[0] + e.faces[1]);
    if inward.length() <= 1e-6 {
        return None;
    }
    let dir = inward.normalize();
    // A step off the edge between the faces (along n1 − n2): empty space
    // beside an outside corner, material beside an inside one.
    let probe = e.mid + (e.faces[0] - e.faces[1]).normalize_or_zero() * (size.abs() * 0.05).max(1e-3);
    let concave = inside(probe);
    Some(LineHandle { origin: e.mid, dir: if concave { -dir } else { dir } })
}

/// An arrow that swings round an axis (the revolve angle).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TurnHandle {
    /// The point of the axis nearest the profile.
    pub pivot: DVec3,
    /// Unit axis; positive angles turn right-handed about it.
    pub axis: DVec3,
    /// Unit vector from the pivot to the profile (the 0° direction).
    pub radial: DVec3,
    /// How far the profile's middle is from the axis.
    pub radius: f64,
}

impl TurnHandle {
    /// `spot`: somewhere on the profile (its middle). None if it is on the axis.
    pub fn new(axis_point: DVec3, axis_dir: DVec3, spot: DVec3) -> Option<Self> {
        let axis = axis_dir.try_normalize()?;
        let pivot = axis_point + axis * (spot - axis_point).dot(axis);
        let r = spot - pivot;
        (r.length() > 1e-9).then(|| Self { pivot, axis, radial: r.normalize(), radius: r.length() })
    }

    fn tangent(&self) -> DVec3 {
        self.axis.cross(self.radial)
    }

    /// Where the handle sits when the revolve is `deg` degrees.
    pub fn position(&self, deg: f64) -> DVec3 {
        let a = deg.to_radians();
        self.pivot + (self.radial * a.cos() + self.tangent() * a.sin()) * self.radius
    }

    /// Which way it moves as the angle grows, at `deg`.
    pub fn direction(&self, deg: f64) -> DVec3 {
        let a = deg.to_radians();
        -self.radial * a.sin() + self.tangent() * a.cos()
    }

    /// The angle (degrees, -180..=180) at which the cursor ray `origin + t·dir`
    /// meets the plane the handle swings in.
    pub fn angle_at(&self, origin: DVec3, dir: DVec3) -> Option<f64> {
        let denom = dir.dot(self.axis);
        if denom.abs() < 1e-9 {
            return None;
        }
        let t = (self.pivot - origin).dot(self.axis) / denom;
        let hit = origin + dir * t - self.pivot;
        Some(hit.dot(self.tangent()).atan2(hit.dot(self.radial)).to_degrees())
    }
}

/// The change from angle `prev` to `now` (both -180..=180) taking the short
/// way round, so dragging across ±180° doesn't jump.
pub fn angle_step(prev: f64, now: f64) -> f64 {
    (now - prev + 180.0).rem_euclid(360.0) - 180.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn edge_arrow_points_into_the_part_along_the_bisector() {
        // The top-front edge of a box: faces point up and toward the viewer.
        let e = EdgeRef { mid: DVec3::new(5.0, 0.0, 10.0), dir: DVec3::X, faces: [DVec3::Z, -DVec3::Y], follow: None };
        // The box: y in 0..20, z in 0..10.
        let in_box = |p: DVec3| (0.0..=20.0).contains(&p.y) && (0.0..=10.0).contains(&p.z);
        let h = edge_handle(&e, 2.0, in_box).unwrap();
        assert_eq!(h.origin, e.mid);
        assert!((h.dir - DVec3::new(0.0, 1.0, -1.0).normalize()).length() < 1e-12, "{:?}", h.dir);
        // The same faces at an inside corner (a step: material below z = 10
        // everywhere, and behind y = 0 up to z = 20): the arrow points out into the gap.
        let step = |p: DVec3| p.z <= 10.0 || (p.y >= 0.0 && p.z <= 20.0);
        let h = edge_handle(&e, 2.0, step).unwrap();
        assert!((h.dir - DVec3::new(0.0, -1.0, 1.0).normalize()).length() < 1e-12, "{:?}", h.dir);
        // Unknown faces (old files) or opposite faces: no arrow.
        assert!(edge_handle(&EdgeRef { faces: [DVec3::ZERO; 2], ..e }, 2.0, in_box).is_none());
        assert!(edge_handle(&EdgeRef { faces: [DVec3::Z, -DVec3::Z], ..e }, 2.0, in_box).is_none());
    }

    #[test]
    fn turn_handle_sits_where_the_profile_swings_to() {
        // Axis = the world Y axis, profile middle at x = 5 (z = 0).
        let h = TurnHandle::new(DVec3::ZERO, DVec3::Y, DVec3::new(5.0, 3.0, 0.0)).unwrap();
        assert!(close(h.radius, 5.0));
        assert!((h.pivot - DVec3::new(0.0, 3.0, 0.0)).length() < 1e-12);
        assert!((h.position(0.0) - DVec3::new(5.0, 3.0, 0.0)).length() < 1e-12);
        // Right-handed about +Y: +X turns toward -Z.
        assert!((h.position(90.0) - DVec3::new(0.0, 3.0, -5.0)).length() < 1e-9, "{:?}", h.position(90.0));
        assert!((h.direction(0.0) - DVec3::new(0.0, 0.0, -1.0)).length() < 1e-9);
        assert!(TurnHandle::new(DVec3::ZERO, DVec3::Y, DVec3::new(0.0, 4.0, 0.0)).is_none(), "on the axis");
        assert!(TurnHandle::new(DVec3::ZERO, DVec3::ZERO, DVec3::X).is_none(), "no axis");
    }

    #[test]
    fn cursor_ray_gives_the_angle_the_handle_is_dragged_to() {
        let h = TurnHandle::new(DVec3::ZERO, DVec3::Y, DVec3::new(5.0, 0.0, 0.0)).unwrap();
        // A ray from above (looking down -Y) through where 90° is: (0, *, -5).
        for deg in [-170.0, -90.0, -10.0, 0.0, 45.0, 90.0, 179.0] {
            let p = h.position(deg);
            let origin = p + DVec3::Y * 50.0 + DVec3::new(0.3, 0.0, 0.2);
            let got = h.angle_at(origin, (p - origin).normalize()).unwrap();
            assert!(close(got, deg), "{deg} → {got}");
        }
        assert!(h.angle_at(DVec3::ZERO, DVec3::X).is_none(), "ray parallel to the swing plane");
    }

    #[test]
    fn angle_steps_take_the_short_way_across_180() {
        assert!(close(angle_step(170.0, -170.0), 20.0));
        assert!(close(angle_step(-170.0, 170.0), -20.0));
        assert!(close(angle_step(10.0, 30.0), 20.0));
        assert!(close(angle_step(0.0, 0.0), 0.0));
    }
}
