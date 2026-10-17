//! A move manipulator (as Fusion's, when inserting or moving): three arrows
//! that move along one axis, and three small squares, shorter than the
//! arrows, that move across two axes at once (the plane they lie in).

use glam::{DVec3, Vec2};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handle {
    /// Along X (0), Y (1) or Z (2).
    Axis(usize),
    /// Across the plane square to X (0: the YZ plane), Y (1: XZ) or Z (2: XY).
    Plane(usize),
}

pub const AXES: [DVec3; 3] = [DVec3::X, DVec3::Y, DVec3::Z];

/// The handles' shapes in the model: an arrow is its two ends, a square its
/// four corners. `len`: how long the arrows are.
pub fn geometry(origin: DVec3, len: f64) -> Vec<(Handle, Vec<DVec3>)> {
    let mut out: Vec<(Handle, Vec<DVec3>)> = (0..3).map(|a| (Handle::Axis(a), vec![origin, origin + AXES[a] * len])).collect();
    // Each square sits between its two axes, from a quarter to half the arrows' length.
    let (near, far) = (0.25 * len, 0.5 * len);
    for n in 0..3 {
        let (u, v) = (AXES[(n + 1) % 3], AXES[(n + 2) % 3]);
        out.push((Handle::Plane(n), vec![origin + u * near + v * near, origin + u * far + v * near, origin + u * far + v * far, origin + u * near + v * far]));
    }
    out
}

/// The handle under `p` (screen), from the handles as drawn (`screen`: the
/// same shapes projected). Arrows win over squares.
pub fn pick(screen: &[(Handle, Vec<Vec2>)], p: Vec2) -> Option<Handle> {
    let near_segment = |a: Vec2, b: Vec2| {
        let ab = b - a;
        let t = ((p - a).dot(ab) / ab.length_squared().max(1e-6)).clamp(0.0, 1.0);
        (a + ab * t).distance(p)
    };
    let arrow = screen
        .iter()
        .filter_map(|(h, pts)| match (h, pts.as_slice()) {
            (Handle::Axis(_), [a, b]) => Some((*h, near_segment(*a, *b))),
            _ => None,
        })
        .filter(|(_, d)| *d < 9.0)
        .min_by(|a, b| a.1.total_cmp(&b.1));
    if let Some((h, _)) = arrow {
        return Some(h);
    }
    screen.iter().find(|(h, pts)| matches!(h, Handle::Plane(_)) && pts.len() == 4 && inside(pts, p)).map(|(h, _)| *h)
}

/// Is `p` inside the convex quad `q` (either winding)?
fn inside(q: &[Vec2], p: Vec2) -> bool {
    let side = |a: Vec2, b: Vec2| (b - a).perp_dot(p - a);
    let s: Vec<f32> = (0..q.len()).map(|k| side(q[k], q[(k + 1) % q.len()])).collect();
    s.iter().all(|v| *v >= 0.0) || s.iter().all(|v| *v <= 0.0)
}

/// Where the mouse ray (`ray_o`, `ray_d`) is on a handle through `origin`:
/// on an arrow, the point of its line nearest the ray; on a square, where
/// the ray meets its plane. None when the ray runs along it.
pub fn grab_point(h: Handle, origin: DVec3, ray_o: DVec3, ray_d: DVec3) -> Option<DVec3> {
    match h {
        Handle::Axis(a) => {
            // Closest points of two lines: origin + d·s and ray_o + ray_d·t.
            let d = AXES[a];
            let w = origin - ray_o;
            let (b, dd, e) = (d.dot(ray_d), d.dot(w), ray_d.dot(w));
            let rr = ray_d.dot(ray_d);
            let denom = rr - b * b; // (|d| = 1)
            if denom.abs() < 1e-9 {
                return None;
            }
            let s = (b * e - rr * dd) / denom;
            Some(origin + d * s)
        }
        Handle::Plane(n) => {
            let normal = AXES[n];
            let along = ray_d.dot(normal);
            if along.abs() < 1e-9 {
                return None;
            }
            let t = (origin - ray_o).dot(normal) / along;
            Some(ray_o + ray_d * t)
        }
    }
}

/// A drag under way: the handle, the offset when it started, and the point
/// first grabbed (with the handle where it was then).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Drag {
    pub handle: Handle,
    pub origin: DVec3,
    pub start_offset: [f64; 3],
    pub grabbed: DVec3,
}

impl Drag {
    pub fn start(handle: Handle, origin: DVec3, offset: [f64; 3], ray_o: DVec3, ray_d: DVec3) -> Option<Self> {
        Some(Drag { handle, origin, start_offset: offset, grabbed: grab_point(handle, origin, ray_o, ray_d)? })
    }

    /// The offset with the mouse ray now at (`ray_o`, `ray_d`): moved by as
    /// much as the grabbed point has, only along the arrow or across the square.
    pub fn offset(&self, ray_o: DVec3, ray_d: DVec3) -> [f64; 3] {
        match grab_point(self.handle, self.origin, ray_o, ray_d) {
            Some(p) => (DVec3::from(self.start_offset) + (p - self.grabbed)).to_array(),
            None => self.start_offset,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arrows_move_along_one_axis_and_squares_across_two() {
        // Looking straight down (−Z) from above (x, y).
        let down = |x: f64, y: f64| (DVec3::new(x, y, 100.0), DVec3::NEG_Z);
        let o = DVec3::new(10.0, 0.0, 5.0);
        // The X arrow grabbed at x = 12, dragged to (20, 7): moves 8 along X only.
        let (ro, rd) = down(12.0, 0.0);
        let d = Drag::start(Handle::Axis(0), o, [1.0, 2.0, 3.0], ro, rd).unwrap();
        let (ro, rd) = down(20.0, 7.0);
        assert_eq!(d.offset(ro, rd), [9.0, 2.0, 3.0]);
        // The XY square: both X and Y follow, Z stays.
        let (ro, rd) = down(12.0, 1.0);
        let d = Drag::start(Handle::Plane(2), o, [0.0; 3], ro, rd).unwrap();
        let (ro, rd) = down(15.0, -3.0);
        let off = d.offset(ro, rd);
        assert!((off[0] - 3.0).abs() < 1e-9 && (off[1] + 4.0).abs() < 1e-9 && off[2].abs() < 1e-9, "{off:?}");
        // The Z arrow seen end-on can't be dragged; from the side it can.
        let (ro, rd) = down(10.0, 0.0);
        assert!(Drag::start(Handle::Axis(2), o, [0.0; 3], ro, rd).is_none());
        let side = |z: f64| (DVec3::new(10.0, -100.0, z), DVec3::Y);
        let (ro, rd) = side(6.0);
        let d = Drag::start(Handle::Axis(2), o, [0.0; 3], ro, rd).unwrap();
        let (ro, rd) = side(11.0);
        let off = d.offset(ro, rd);
        assert!(off[0].abs() < 1e-9 && off[1].abs() < 1e-9 && (off[2] - 5.0).abs() < 1e-9, "{off:?}");
    }

    #[test]
    fn squares_are_shorter_than_arrows_and_arrows_are_picked_first() {
        let g = geometry(DVec3::ZERO, 10.0);
        assert_eq!(g.len(), 6);
        for (h, pts) in &g {
            if let Handle::Plane(_) = h {
                assert!(pts.iter().all(|p| p.length() < 10.0 * 0.75), "{h:?} reaches past the arrows");
            }
        }
        // On screen: the X arrow along +x, the XY square up and to the right of it.
        let screen = vec![
            (Handle::Axis(0), vec![Vec2::ZERO, Vec2::new(100.0, 0.0)]),
            (Handle::Plane(2), vec![Vec2::new(25.0, -25.0), Vec2::new(50.0, -25.0), Vec2::new(50.0, -50.0), Vec2::new(25.0, -50.0)]),
        ];
        assert_eq!(pick(&screen, Vec2::new(60.0, 4.0)), Some(Handle::Axis(0)));
        assert_eq!(pick(&screen, Vec2::new(40.0, -40.0)), Some(Handle::Plane(2)));
        assert_eq!(pick(&screen, Vec2::new(80.0, -80.0)), None);
    }
}
