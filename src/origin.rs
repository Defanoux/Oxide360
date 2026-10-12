//! The origin as Fusion draws it in the 3D view: three short axes and three
//! small translucent planes (the positive quadrant of XY, XZ and YZ), sized to
//! the view so they never run on forever. The planes are also what you click
//! to start a sketch.

use crate::camera::Ray;
use crate::sketch::OriginPlane;
use glam::{DVec2, Vec3};

/// How big the origin planes are for a camera `distance` away: a 1-2-5 size
/// near a fifth of the distance, so they stay a similar size on screen but
/// don't jitter as you zoom.
pub fn size_for(distance: f32) -> f32 {
    let raw = (distance.max(1e-3) * 0.2) as f64;
    let mag = 10f64.powf(raw.log10().floor());
    let norm = raw / mag;
    let nice = if norm < 1.5 {
        1.0
    } else if norm < 3.5 {
        2.0
    } else if norm < 7.5 {
        5.0
    } else {
        10.0
    };
    (nice * mag) as f32
}

/// The four corners of an origin plane's square (positive quadrant), in order.
pub fn quad(plane: OriginPlane, size: f32) -> [Vec3; 4] {
    let p = plane.plane();
    [Vec3::ZERO, p.u * size, p.u * size + p.v * size, p.v * size]
}

/// The origin plane under the cursor ray, nearest first: (plane, distance).
pub fn pick(ray: &Ray, size: f32) -> Option<(OriginPlane, f32)> {
    let s = size as f64;
    OriginPlane::ALL
        .into_iter()
        .filter_map(|op| {
            let (t, DVec2 { x, y }) = op.plane().intersect_t(ray)?;
            ((0.0..=s).contains(&x) && (0.0..=s).contains(&y)).then_some((op, t))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_follows_the_view_in_nice_steps() {
        assert_eq!(size_for(100.0), 20.0);
        assert_eq!(size_for(250.0), 50.0);
        assert_eq!(size_for(500.0), 100.0);
        assert_eq!(size_for(30.0), 5.0);
        assert_eq!(size_for(0.0), 0.0002); // never zero or NaN
        let a = size_for(180.0);
        assert!(a == size_for(190.0), "stable across a small zoom: {a}");
    }

    #[test]
    fn quads_are_the_positive_quadrants() {
        let q = quad(OriginPlane::XZ, 10.0);
        assert_eq!(q, [Vec3::ZERO, Vec3::new(10.0, 0.0, 0.0), Vec3::new(10.0, 0.0, 10.0), Vec3::new(0.0, 0.0, 10.0)]);
        let q = quad(OriginPlane::YZ, 4.0);
        assert_eq!(q[2], Vec3::new(0.0, 4.0, 4.0));
    }

    #[test]
    fn a_ray_picks_the_plane_it_passes_through_and_misses_outside_the_square() {
        // From above and in front, looking down at (3, 3, 0): the XY square.
        let ray = Ray { origin: Vec3::new(3.0, 3.0, 50.0), dir: Vec3::new(0.0, 0.0, -1.0) };
        assert_eq!(pick(&ray, 10.0).map(|p| p.0), Some(OriginPlane::XY));
        // Outside the quadrant (negative x): no plane.
        let ray = Ray { origin: Vec3::new(-3.0, 3.0, 50.0), dir: Vec3::new(0.0, 0.0, -1.0) };
        assert!(pick(&ray, 10.0).is_none());
        // Past the far edge.
        let ray = Ray { origin: Vec3::new(30.0, 3.0, 50.0), dir: Vec3::new(0.0, 0.0, -1.0) };
        assert!(pick(&ray, 10.0).is_none());
        // Along +X toward the origin at height 4, y = 5: crosses YZ (x = 0) first.
        let ray = Ray { origin: Vec3::new(40.0, 5.0, 4.0), dir: Vec3::new(-1.0, 0.0, 0.0) };
        assert_eq!(pick(&ray, 10.0).map(|p| p.0), Some(OriginPlane::YZ));
    }

    #[test]
    fn where_planes_overlap_the_nearer_one_wins() {
        // Looking along -Y from in front at x = 2, z = 2: only XZ (y = 0) is crossed.
        let ray = Ray { origin: Vec3::new(2.0, 40.0, 2.0), dir: Vec3::new(0.0, -1.0, 0.0) };
        assert_eq!(pick(&ray, 10.0).map(|p| p.0), Some(OriginPlane::XZ));
        // A slanted ray crossing XY (z = 0) near x=2 and then... nearest by distance.
        let ray = Ray { origin: Vec3::new(2.0, 2.0, 30.0), dir: Vec3::new(0.0, 0.0, -1.0) };
        assert_eq!(pick(&ray, 10.0).map(|p| p.0), Some(OriginPlane::XY));
    }
}
