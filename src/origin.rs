//! The origin as Fusion draws it in the 3D view: three short axes and three
//! small translucent planes (the positive quadrant of XY, XZ and YZ), sized to
//! the view so they never run on forever. The planes are also what you click
//! to start a sketch.

use crate::camera::Ray;
use crate::sketch::OriginPlane;
use glam::DVec2;

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

/// How close two hits along a ray at distance `t` must be to count as the
/// same spot — a body's face lying in an origin plane, say.
pub fn tie_tolerance(t: f32) -> f32 {
    (t.abs() * 1e-4).max(1e-3)
}

/// Where along each of its two axes a plane's square lies: set back from the
/// origin (so the axes and the origin stay clear to pick) and short of `size`.
pub fn span(size: f32) -> (f32, f32) {
    (size * 0.15, size * 0.75)
}

/// The origin plane under the cursor ray, nearest first: (plane, distance).
pub fn pick(ray: &Ray, size: f32, shown: impl Fn(OriginPlane) -> bool) -> Option<(OriginPlane, f32)> {
    let (lo, hi) = span(size);
    let (lo, hi) = (lo as f64, hi as f64);
    OriginPlane::ALL
        .into_iter()
        .filter(|&op| shown(op))
        .filter_map(|op| {
            let (t, DVec2 { x, y }) = op.plane().intersect_t(ray)?;
            ((lo..=hi).contains(&x) && (lo..=hi).contains(&y)).then_some((op, t))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
}

/// What the browser's Origin folder shows: the folder switch, then each
/// plane and axis on its own.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shown {
    pub all: bool,
    pub planes: [bool; 3],
    pub axes: [bool; 3],
}

impl Default for Shown {
    fn default() -> Self {
        Shown { all: true, planes: [true; 3], axes: [true; 3] }
    }
}

impl Shown {
    pub fn plane(&self, p: OriginPlane) -> bool {
        self.all && self.planes[p as usize]
    }

    pub fn axis(&self, k: usize) -> bool {
        self.all && self.axes[k]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

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
    fn a_ray_picks_the_plane_it_passes_through_and_misses_outside_the_square() {
        // From above and in front, looking down at (3, 3, 0): the XY square.
        let ray = Ray { origin: Vec3::new(3.0, 3.0, 50.0), dir: Vec3::new(0.0, 0.0, -1.0) };
        assert_eq!(pick(&ray, 10.0, |_| true).map(|p| p.0), Some(OriginPlane::XY));
        // Outside the quadrant (negative x): no plane.
        let ray = Ray { origin: Vec3::new(-3.0, 3.0, 50.0), dir: Vec3::new(0.0, 0.0, -1.0) };
        assert!(pick(&ray, 10.0, |_| true).is_none());
        // Past the far edge.
        let ray = Ray { origin: Vec3::new(30.0, 3.0, 50.0), dir: Vec3::new(0.0, 0.0, -1.0) };
        assert!(pick(&ray, 10.0, |_| true).is_none());
        // Along +X toward the origin at height 4, y = 5: crosses YZ (x = 0) first.
        let ray = Ray { origin: Vec3::new(40.0, 5.0, 4.0), dir: Vec3::new(-1.0, 0.0, 0.0) };
        assert_eq!(pick(&ray, 10.0, |_| true).map(|p| p.0), Some(OriginPlane::YZ));
    }

    #[test]
    fn where_planes_overlap_the_nearer_one_wins() {
        // Looking along -Y from in front at x = 2, z = 2: only XZ (y = 0) is crossed.
        let ray = Ray { origin: Vec3::new(2.0, 40.0, 2.0), dir: Vec3::new(0.0, -1.0, 0.0) };
        assert_eq!(pick(&ray, 10.0, |_| true).map(|p| p.0), Some(OriginPlane::XZ));
        // A slanted ray crossing XY (z = 0) near x=2 and then... nearest by distance.
        let ray = Ray { origin: Vec3::new(2.0, 2.0, 30.0), dir: Vec3::new(0.0, 0.0, -1.0) };
        assert_eq!(pick(&ray, 10.0, |_| true).map(|p| p.0), Some(OriginPlane::XY));
    }

    #[test]
    fn hidden_planes_are_not_picked_and_the_folder_switch_hides_all() {
        let ray = Ray { origin: Vec3::new(3.0, 3.0, 50.0), dir: Vec3::new(0.0, 0.0, -1.0) };
        assert!(pick(&ray, 10.0, |p| p != OriginPlane::XY).is_none());
        let mut s = Shown::default();
        assert!(s.plane(OriginPlane::XZ) && s.axis(2));
        s.planes[OriginPlane::XZ as usize] = false;
        assert!(!s.plane(OriginPlane::XZ) && s.plane(OriginPlane::XY));
        s.all = false;
        assert!(!s.plane(OriginPlane::XY) && !s.axis(0));
    }

    #[test]
    fn hits_within_the_tolerance_count_as_the_same_spot() {
        assert!(tie_tolerance(100.0) >= 0.01 - 1e-6);
        assert!(tie_tolerance(0.0) > 0.0);
        // A body face lying in the plane (same distance) is not "in front".
        let (t_plane, t_body) = (250.0f32, 250.0 + 1e-3);
        assert!(!(t_plane < t_body - tie_tolerance(t_body)));
        // A body clearly in front of the plane covers it.
        assert!(!(250.0f32 < 200.0 - tie_tolerance(200.0)));
        // A plane clearly in front of the body wins.
        assert!(200.0f32 < 250.0 - tie_tolerance(250.0));
    }
}
