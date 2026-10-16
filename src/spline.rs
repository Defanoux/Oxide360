//! Fit-point splines: the smooth curve through a sketch's fit points. It is
//! the kernel's own curve (OpenCASCADE's interpolated B-spline), sampled for
//! drawing, picking and profiles, so the sketch and the solid always agree.
//! Samples are cached by the fit points (dragging changes them every frame,
//! the cache keeps the rest cheap).

use glam::DVec2;
use glam24::DVec3 as KVec;
use std::cell::RefCell;
use std::collections::HashMap;

/// Points per span (between two fit points) when sampling.
pub const PER_SPAN: usize = 48;

thread_local! {
    static CACHE: RefCell<HashMap<Vec<[u64; 2]>, Vec<DVec2>>> = RefCell::new(HashMap::new());
}

/// Points along the spline through `pts`, ends included. Straight lines
/// between the points if the kernel can't make the curve (e.g. two fit
/// points in the same place). A closed spline (its last point is its first)
/// runs smoothly round, with no corner where it closes. Fit point k is
/// sample k * PER_SPAN exactly (for three or more points).
pub fn polyline(pts: &[DVec2]) -> Vec<DVec2> {
    polyline_with(pts, &[])
}

/// As `polyline`, with a set tangent at the fit points that have one
/// (`tangents[k]` for `pts[k]`; missing or None: the curve's own).
pub fn polyline_with(pts: &[DVec2], tangents: &[Option<DVec2>]) -> Vec<DVec2> {
    if pts.len() < 3 && tangents.iter().all(Option::is_none) {
        return pts.to_vec(); // two points: a straight line
    }
    let mut key: Vec<[u64; 2]> = pts.iter().map(|p| [p.x.to_bits(), p.y.to_bits()]).collect();
    key.extend(tangents.iter().map(|t| t.map_or([u64::MAX; 2], |t| [t.x.to_bits(), t.y.to_bits()])));
    if let Some(hit) = CACHE.with(|c| c.borrow().get(&key).cloned()) {
        return hit;
    }
    let closed = is_closed(pts);
    let n = if closed { pts.len() - 1 } else { pts.len() };
    let fit = &pts[..n];
    let k: Vec<KVec> = fit.iter().map(|p| KVec::new(p.x, p.y, 0.0)).collect();
    let t: Vec<Option<KVec>> = (0..n).map(|i| tangents.get(i).copied().flatten().map(|v| KVec::new(v.x, v.y, 0.0))).collect();
    let made = opencascade::primitives::try_spline_samples_with(&k, &t, closed, PER_SPAN)
        .map(|v| v.into_iter().map(|q| DVec2::new(q.x, q.y)).collect::<Vec<_>>())
        .unwrap_or_else(|| pts.to_vec());
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if c.len() > 512 {
            c.clear();
        }
        c.insert(key, made.clone());
    });
    made
}

/// A closed spline: at least three different points, the last one the first again.
pub fn is_closed(pts: &[DVec2]) -> bool {
    pts.len() >= 4 && pts[0].distance(pts[pts.len() - 1]) < 1e-9
}

/// How far `p` is from the spline through `pts`.
pub fn distance(pts: &[DVec2], p: DVec2) -> f64 {
    polyline(pts).windows(2).map(|w| crate::sketch::dist_to_segment(p, w[0], w[1])).fold(f64::INFINITY, f64::min)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PTS: [DVec2; 5] = [DVec2::new(0.0, 0.0), DVec2::new(10.0, 6.0), DVec2::new(20.0, -3.0), DVec2::new(35.0, 4.0), DVec2::new(40.0, 0.0)];

    #[test]
    fn fit_point_k_is_sample_k_per_span_and_a_tangent_turns_the_curve() {
        let line = polyline(&PTS);
        for (k, p) in PTS.iter().enumerate() {
            assert!(line[k * PER_SPAN].distance(*p) < 1e-9, "fit point {k}");
        }
        // A tangent straight up at the middle point: the curve leaves it upward.
        let mut t = vec![None; PTS.len()];
        t[2] = Some(DVec2::new(0.0, 1.0));
        let bent = polyline_with(&PTS, &t);
        let m = 2 * PER_SPAN;
        let dir = (bent[m + 1] - bent[m - 1]).normalize();
        assert!(dir.dot(DVec2::Y) > 0.999, "{dir}");
        assert!(bent[m].distance(PTS[2]) < 1e-9);
    }

    #[test]
    fn it_runs_from_first_to_last_through_every_fit_point() {
        let line = polyline(&PTS);
        assert_eq!(line.len(), PER_SPAN * 4 + 1);
        assert!(line[0].distance(PTS[0]) < 1e-9 && line.last().unwrap().distance(PTS[4]) < 1e-9);
        for p in PTS {
            assert!(distance(&PTS, p) < 0.02, "{p} is {} off", distance(&PTS, p));
        }
        assert!(distance(&PTS, DVec2::new(20.0, 30.0)) > 20.0);
    }

    #[test]
    fn two_points_or_bad_points_fall_back_to_straight_lines() {
        let two = [DVec2::ZERO, DVec2::new(4.0, 2.0)];
        assert_eq!(polyline(&two), two.to_vec());
        let repeated = [DVec2::ZERO, DVec2::ZERO, DVec2::new(5.0, 1.0)];
        assert_eq!(polyline(&repeated), repeated.to_vec(), "no crash on repeated points");
    }

    #[test]
    fn the_sketch_spline_is_the_kernels_spline() {
        let k: Vec<KVec> = PTS.iter().map(|p| KVec::new(p.x, p.y, 0.0)).collect();
        let edge = opencascade::primitives::Edge::try_spline_through(&k).expect("a spline");
        let ours = polyline(&PTS);
        for q in edge.approximation_segments() {
            let d = ours.windows(2).map(|w| crate::sketch::dist_to_segment(DVec2::new(q.x, q.y), w[0], w[1])).fold(f64::INFINITY, f64::min);
            assert!(d < 0.02, "kernel point {q:?} is {d} off the drawn curve");
        }
    }
}
