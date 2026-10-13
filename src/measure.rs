//! The Measure tool: two points on the model and what lies between them.

use glam::DVec3;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Measure {
    pub first: Option<DVec3>,
    pub second: Option<DVec3>,
}

/// What two points say about each other.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reading {
    pub distance: f64,
    /// Second minus first, along X, Y, Z.
    pub delta: DVec3,
}

impl Measure {
    /// A click: sets the first point, then the second; the next one starts over.
    pub fn click(&mut self, p: DVec3) {
        match (self.first, self.second) {
            (Some(_), None) => self.second = Some(p),
            _ => {
                self.first = Some(p);
                self.second = None;
            }
        }
    }

    /// The reading once there are two points; with only one, to `toward` (the cursor).
    pub fn reading(&self, toward: Option<DVec3>) -> Option<Reading> {
        let a = self.first?;
        let b = self.second.or(toward)?;
        Some(Reading { distance: a.distance(b), delta: b - a })
    }
}

/// `p` moved onto the nearest of `corners` if one is within `tol`.
pub fn snap(p: DVec3, corners: impl IntoIterator<Item = DVec3>, tol: f64) -> DVec3 {
    corners
        .into_iter()
        .map(|c| (c.distance(p), c))
        .filter(|(d, _)| *d <= tol)
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map_or(p, |(_, c)| c)
}

impl Reading {
    pub fn text(&self) -> String {
        format!("{:.3} mm   Δx {:.3}  Δy {:.3}  Δz {:.3}", self.distance, self.delta.x, self.delta.y, self.delta.z)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_clicks_measure_and_the_third_starts_again() {
        let mut m = Measure::default();
        m.click(DVec3::new(1.0, 2.0, 3.0));
        assert_eq!(m.reading(None), None);
        let r = m.reading(Some(DVec3::new(4.0, 6.0, 3.0))).unwrap();
        assert!((r.distance - 5.0).abs() < 1e-12);
        m.click(DVec3::new(1.0, 2.0, 13.0));
        let r = m.reading(None).unwrap();
        assert!((r.distance - 10.0).abs() < 1e-12 && r.delta == DVec3::new(0.0, 0.0, 10.0));
        m.click(DVec3::ZERO);
        assert_eq!((m.first, m.second), (Some(DVec3::ZERO), None));
    }

    #[test]
    fn snapping_takes_the_nearest_corner_in_reach_only() {
        let corners = [DVec3::new(0.0, 0.0, 0.0), DVec3::new(1.0, 0.0, 0.0)];
        assert_eq!(snap(DVec3::new(0.8, 0.1, 0.0), corners, 0.5), DVec3::new(1.0, 0.0, 0.0));
        let far = DVec3::new(5.0, 5.0, 5.0);
        assert_eq!(snap(far, corners, 0.5), far);
    }
}
