//! Profiles: the closed regions of a sketch that can be extruded.
//!
//! 1. Every line and circle is split wherever it meets another curve —
//!    crossings, T-junctions and tangent points (a line tangent to a circle).
//! 2. The pieces form a planar graph (arcs become short segments). Walking
//!    every edge while always taking the sharpest turn finds each enclosed face
//!    exactly once (counter-clockwise, positive area); the outer boundary of
//!    each connected piece comes out clockwise and is discarded.
//! 3. Circles that touch nothing are their own loops. A loop inside another
//!    becomes a hole in it, so a circle inside a rectangle gives two profiles:
//!    the rectangle-with-hole and the disc.

use crate::sketch::{Entity, Sketch};
use glam::DVec2;
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// Segments used to approximate a full circle in a profile.
pub const CIRCLE_SEGMENTS: usize = 96;
const AREA_EPS: f64 = 1e-9;
/// Points closer than this (mm) are the same point; also the tangency tolerance.
const TOL: f64 = 1e-6;

/// What a boundary edge really is. Arcs are drawn and triangulated as short
/// segments, but each segment remembers its circle so the solid-modeling
/// kernel can rebuild the exact arc (true cylinders in STEP, not facets).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Curve {
    Line,
    Arc { center: DVec2, radius: f64 },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Region {
    /// Counter-clockwise.
    pub outer: Vec<DVec2>,
    /// Clockwise.
    pub holes: Vec<Vec<DVec2>>,
    /// `outer_curves[i]` is the edge from `outer[i]` to `outer[i + 1]` (wrapping).
    pub outer_curves: Vec<Curve>,
    pub hole_curves: Vec<Vec<Curve>>,
}

/// Reverse a loop together with its edge tags.
fn reverse_loop(pts: &[DVec2], curves: &[Curve]) -> (Vec<DVec2>, Vec<Curve>) {
    let n = pts.len();
    let rp: Vec<DVec2> = pts.iter().rev().copied().collect();
    // Edge j of the reversed loop is edge (n - 2 - j) of the original, backwards.
    let rc: Vec<Curve> = (0..n).map(|j| curves[(2 * n - 2 - j) % n]).collect();
    (rp, rc)
}

pub fn signed_area(poly: &[DVec2]) -> f64 {
    let n = poly.len();
    (0..n).map(|i| poly[i].perp_dot(poly[(i + 1) % n])).sum::<f64>() * 0.5
}

/// Ray-casting point-in-polygon test.
pub fn point_in_polygon(p: DVec2, poly: &[DVec2]) -> bool {
    let mut inside = false;
    let n = poly.len();
    let mut j = n - 1;
    for i in 0..n {
        let (a, b) = (poly[i], poly[j]);
        if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// Triangulate polygon-with-holes. Returns (vertices, CCW triangles).
/// Vertex order is outer loop, then each hole in order.
fn triangulate(loops: &[&[DVec2]]) -> (Vec<DVec2>, Vec<[usize; 3]>) {
    let mut verts = Vec::new();
    let mut flat = Vec::new();
    let mut holes = Vec::new();
    for (k, l) in loops.iter().enumerate() {
        if k > 0 {
            holes.push(verts.len());
        }
        for p in l.iter() {
            verts.push(*p);
            flat.push(p.x);
            flat.push(p.y);
        }
    }
    let idx = earcutr::earcut(&flat, &holes, 2).unwrap_or_default();
    let tris = idx
        .chunks_exact(3)
        .map(|t| {
            let (a, b, c) = (verts[t[0]], verts[t[1]], verts[t[2]]);
            if (b - a).perp_dot(c - a) >= 0.0 { [t[0], t[1], t[2]] } else { [t[0], t[2], t[1]] }
        })
        .collect();
    (verts, tris)
}

impl Region {
    pub fn loops(&self) -> Vec<&[DVec2]> {
        std::iter::once(self.outer.as_slice()).chain(self.holes.iter().map(|h| h.as_slice())).collect()
    }

    /// Each loop (outer first) with its edge tags.
    pub fn loops_with_curves(&self) -> Vec<(&[DVec2], &[Curve])> {
        std::iter::once((self.outer.as_slice(), self.outer_curves.as_slice()))
            .chain(self.holes.iter().zip(&self.hole_curves).map(|(h, c)| (h.as_slice(), c.as_slice())))
            .collect()
    }

    pub fn area(&self) -> f64 {
        signed_area(&self.outer) + self.holes.iter().map(|h| signed_area(h)).sum::<f64>()
    }

    pub fn contains(&self, p: DVec2) -> bool {
        point_in_polygon(p, &self.outer) && !self.holes.iter().any(|h| point_in_polygon(p, h))
    }

    /// (vertices, CCW triangles); vertices are outer then holes, matching `loops()`.
    pub fn triangulate(&self) -> (Vec<DVec2>, Vec<[usize; 3]>) {
        triangulate(&self.loops())
    }

    /// Area-weighted centre, guaranteed to be computed from inside triangles.
    pub fn centroid(&self) -> DVec2 {
        let (v, tris) = self.triangulate();
        let (mut sum, mut total) = (DVec2::ZERO, 0.0);
        for t in &tris {
            let a = (v[t[1]] - v[t[0]]).perp_dot(v[t[2]] - v[t[0]]) * 0.5;
            sum += (v[t[0]] + v[t[1]] + v[t[2]]) / 3.0 * a;
            total += a;
        }
        if total > 0.0 { sum / total } else { self.outer[0] }
    }
}

/// A point strictly inside a CCW loop (centroid of its largest triangle).
fn interior_point(l: &[DVec2]) -> DVec2 {
    let (v, tris) = triangulate(&[l]);
    tris.iter()
        .map(|t| ((v[t[1]] - v[t[0]]).perp_dot(v[t[2]] - v[t[0]]), (v[t[0]] + v[t[1]] + v[t[2]]) / 3.0))
        .max_by(|a, b| a.0.total_cmp(&b.0))
        .map_or(l[0], |(_, c)| c)
}

// ---------- Curve intersections ----------

/// Parameters t in [0, 1] on segment p→p+r where it meets segment q→q+u.
fn seg_seg(p: DVec2, r: DVec2, q: DVec2, u: DVec2) -> Vec<f64> {
    let denom = r.perp_dot(u);
    let rr = r.length_squared();
    let in_range = |t: f64, len: f64| t >= -TOL / len && t <= 1.0 + TOL / len;
    if denom.abs() <= 1e-12 * rr.sqrt() * u.length() {
        // Parallel. If collinear, the other segment's ends that lie on this one split it.
        if (q - p).perp_dot(r).abs() > TOL * rr.sqrt() {
            return vec![];
        }
        return [q, q + u]
            .iter()
            .map(|&x| (x - p).dot(r) / rr)
            .filter(|&t| in_range(t, rr.sqrt()))
            .collect();
    }
    let t = (q - p).perp_dot(u) / denom;
    let s = (q - p).perp_dot(r) / denom;
    if in_range(t, rr.sqrt()) && in_range(s, u.length()) { vec![t.clamp(0.0, 1.0)] } else { vec![] }
}

/// Points where segment p→p+r meets circle (c, rad), including tangency.
fn seg_circle(p: DVec2, r: DVec2, c: DVec2, rad: f64) -> Vec<DVec2> {
    let len = r.length();
    if len <= TOL {
        return vec![];
    }
    let dir = r / len;
    let along = (c - p).dot(dir); // foot of the perpendicular from the centre
    let h = (c - p).perp_dot(dir).abs(); // distance from centre to the line
    let ts: Vec<f64> = if (h - rad).abs() <= TOL {
        vec![along]
    } else if h < rad {
        let k = (rad * rad - h * h).sqrt();
        vec![along - k, along + k]
    } else {
        vec![]
    };
    ts.into_iter().filter(|&t| t >= -TOL && t <= len + TOL).map(|t| p + dir * t.clamp(0.0, len)).collect()
}

/// Points where two circles meet, including tangency.
fn circle_circle(c1: DVec2, r1: f64, c2: DVec2, r2: f64) -> Vec<DVec2> {
    let d = c1.distance(c2);
    if d <= TOL {
        return vec![]; // concentric
    }
    let dir = (c2 - c1) / d;
    let a = (d * d + r1 * r1 - r2 * r2) / (2.0 * d); // distance from c1 to the chord
    if a.abs() > r1 + TOL {
        return vec![];
    }
    let h2 = r1 * r1 - a * a;
    if h2 <= (2.0 * r1 * TOL).max(TOL * TOL) {
        return vec![c1 + dir * a]; // tangent
    }
    let h = h2.sqrt();
    vec![c1 + dir * a + dir.perp() * h, c1 + dir * a - dir.perp() * h]
}

// ---------- Planar graph ----------

struct Graph {
    verts: Vec<DVec2>,
    edges: BTreeMap<(usize, usize), Curve>,
}

impl Graph {
    /// Index of the vertex at p, merging with an existing one within TOL.
    fn vertex(&mut self, p: DVec2) -> usize {
        if let Some(i) = self.verts.iter().position(|v| v.distance(p) <= TOL) {
            return i;
        }
        self.verts.push(p);
        self.verts.len() - 1
    }

    fn edge(&mut self, a: usize, b: usize, curve: Curve) {
        if a != b {
            self.edges.entry((a.min(b), a.max(b))).or_insert(curve);
        }
    }
}

/// Split every curve at its intersections. Returns the graph of pieces and
/// the circles that touch nothing (they stay whole loops).
fn arrangement(sk: &Sketch) -> (Graph, Vec<(DVec2, f64)>) {
    let mut lines: Vec<(DVec2, DVec2)> = Vec::new();
    let mut circles: Vec<(DVec2, f64)> = Vec::new();
    // Per circle: None for a whole circle, or an arc's (start angle, sweep).
    let mut spans: Vec<Option<(f64, f64)>> = Vec::new();
    for (i, e) in sk.entities.iter().enumerate() {
        if sk.is_construction(i) {
            continue; // construction geometry never bounds a region
        }
        match *e {
            Entity::Line { a, b } if sk.points[a].distance(sk.points[b]) > TOL => {
                lines.push((sk.points[a], sk.points[b]))
            }
            Entity::Line { .. } => {}
            Entity::Circle { center, radius } if radius > TOL => {
                circles.push((sk.points[center], radius));
                spans.push(None);
            }
            Entity::Arc { .. } => {
                if let Some((c, r, a0, sweep)) = sk.arc_geometry(i).filter(|g| g.1 > TOL) {
                    circles.push((c, r));
                    spans.push(Some((a0, sweep)));
                }
            }
            Entity::Circle { .. } | Entity::Point { .. } => {}
        }
    }
    // Is point x (on circle k) within that circle's arc?
    let on_span = |k: usize, x: DVec2| match spans[k] {
        None => true,
        Some((a0, sweep)) => {
            let (c, r) = circles[k];
            let t = ((x - c).y.atan2((x - c).x) - a0).rem_euclid(std::f64::consts::TAU);
            t <= sweep + TOL / r || t >= std::f64::consts::TAU - TOL / r
        }
    };

    // Split parameters for each line (0..1) and angles for each circle.
    let mut line_ts: Vec<Vec<f64>> = vec![vec![0.0, 1.0]; lines.len()];
    let mut circle_pts: Vec<Vec<DVec2>> = vec![Vec::new(); circles.len()];
    for i in 0..lines.len() {
        let (p, q) = lines[i];
        for j in 0..lines.len() {
            if i != j {
                let (a, b) = lines[j];
                line_ts[i].extend(seg_seg(p, q - p, a, b - a));
            }
        }
        for (k, &(c, r)) in circles.iter().enumerate() {
            for x in seg_circle(p, q - p, c, r) {
                if on_span(k, x) {
                    line_ts[i].push((x - p).dot(q - p) / (q - p).length_squared());
                    circle_pts[k].push(x);
                }
            }
        }
    }
    for k in 0..circles.len() {
        for m in 0..circles.len() {
            if k != m {
                let ((c1, r1), (c2, r2)) = (circles[k], circles[m]);
                let hits = circle_circle(c1, r1, c2, r2);
                circle_pts[k].extend(hits.into_iter().filter(|&x| on_span(k, x) && on_span(m, x)));
            }
        }
    }

    let mut g = Graph { verts: Vec::new(), edges: BTreeMap::new() };
    for (i, ts) in line_ts.iter_mut().enumerate() {
        let (p, q) = lines[i];
        ts.sort_by(|a, b| a.total_cmp(b));
        let ids: Vec<usize> = ts.iter().map(|&t| g.vertex(p + (q - p) * t)).collect();
        for w in ids.windows(2) {
            g.edge(w[0], w[1], Curve::Line);
        }
    }
    let mut whole = Vec::new();
    for (k, pts) in circle_pts.iter().enumerate() {
        let (c, r) = circles[k];
        if let Some((a0, sweep)) = spans[k] {
            // An arc: pieces between its crossings, from end to end (no wrap).
            let mut ts: Vec<f64> = pts
                .iter()
                .map(|x| {
                    let t = ((x - c).y.atan2((x - c).x) - a0).rem_euclid(std::f64::consts::TAU);
                    if t > sweep + TOL / r { 0.0 } else { t.min(sweep) }
                })
                .chain([0.0, sweep])
                .collect();
            ts.sort_by(|a, b| a.total_cmp(b));
            ts.dedup_by(|a, b| (*a - *b).abs() * r <= TOL);
            for w in ts.windows(2) {
                let (t0, t1) = (w[0], w[1]);
                let steps = (((t1 - t0) / std::f64::consts::TAU * CIRCLE_SEGMENTS as f64).ceil() as usize).max(1);
                let at = |t: f64| c + r * DVec2::from_angle(a0 + t);
                let mut prev = g.vertex(at(t0));
                for s in 1..=steps {
                    let next = g.vertex(at(t0 + (t1 - t0) * s as f64 / steps as f64));
                    g.edge(prev, next, Curve::Arc { center: c, radius: r });
                    prev = next;
                }
            }
            continue;
        }
        if pts.is_empty() {
            whole.push((c, r));
            continue;
        }
        let mut angles: Vec<f64> =
            pts.iter().map(|p| (p.y - c.y).atan2(p.x - c.x).rem_euclid(std::f64::consts::TAU)).collect();
        angles.sort_by(|a, b| a.total_cmp(b));
        angles.dedup_by(|a, b| (*a - *b).abs() * r <= TOL);
        let n = angles.len();
        for i in 0..n {
            let a0 = angles[i];
            let mut a1 = angles[(i + 1) % n];
            if a1 <= a0 {
                a1 += std::f64::consts::TAU; // wraps (or a single split point: full turn)
            }
            // Arc a0→a1 as short segments, same density as a full circle.
            let steps = (((a1 - a0) / std::f64::consts::TAU * CIRCLE_SEGMENTS as f64).ceil() as usize).max(1);
            let mut prev = g.vertex(c + r * DVec2::new(a0.cos(), a0.sin()));
            for s in 1..=steps {
                let a = a0 + (a1 - a0) * s as f64 / steps as f64;
                let next = g.vertex(c + r * DVec2::new(a.cos(), a.sin()));
                g.edge(prev, next, Curve::Arc { center: c, radius: r });
                prev = next;
            }
        }
    }
    (g, whole)
}

/// Enclosed faces of the graph, each as a CCW polygon with its edge tags.
fn graph_faces(g: &Graph) -> Vec<(Vec<DVec2>, Vec<Curve>)> {
    let mut edges: BTreeSet<(usize, usize)> = g.edges.keys().copied().collect();
    // Prune dangling edges (they can't bound a face).
    loop {
        let mut degree: BTreeMap<usize, usize> = BTreeMap::new();
        for &(a, b) in &edges {
            *degree.entry(a).or_default() += 1;
            *degree.entry(b).or_default() += 1;
        }
        let before = edges.len();
        edges.retain(|&(a, b)| degree[&a] > 1 && degree[&b] > 1);
        if edges.len() == before {
            break;
        }
    }
    // Neighbours of each vertex sorted counter-clockwise by angle.
    let mut nbrs: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for &(a, b) in &edges {
        nbrs.entry(a).or_default().push(b);
        nbrs.entry(b).or_default().push(a);
    }
    for (v, list) in nbrs.iter_mut() {
        let o = g.verts[*v];
        list.sort_by(|&p, &q| {
            let (dp, dq) = (g.verts[p] - o, g.verts[q] - o);
            dp.y.atan2(dp.x).total_cmp(&dq.y.atan2(dq.x))
        });
    }
    // Walk every directed edge once, keeping the face on the left.
    let mut used: BTreeSet<(usize, usize)> = BTreeSet::new();
    let mut faces = Vec::new();
    for &(a, b) in &edges {
        for start in [(a, b), (b, a)] {
            if used.contains(&start) {
                continue;
            }
            let mut ids = Vec::new();
            let (mut u, mut v) = start;
            while used.insert((u, v)) {
                ids.push(u);
                // At v, the next edge is the one just clockwise of the way we came in.
                let list = &nbrs[&v];
                let k = list.iter().position(|&w| w == u).unwrap();
                let w = list[(k + list.len() - 1) % list.len()];
                (u, v) = (v, w);
            }
            let poly: Vec<DVec2> = ids.iter().map(|&i| g.verts[i]).collect();
            if poly.len() >= 3 && signed_area(&poly) > AREA_EPS {
                let n = ids.len();
                let curves = (0..n)
                    .map(|k| {
                        let (p, q) = (ids[k], ids[(k + 1) % n]);
                        g.edges[&(p.min(q), p.max(q))]
                    })
                    .collect();
                faces.push((poly, curves));
            }
        }
    }
    faces
}

fn circle_loop(c: DVec2, r: f64) -> Vec<DVec2> {
    (0..CIRCLE_SEGMENTS)
        .map(|i| {
            let a = i as f64 / CIRCLE_SEGMENTS as f64 * std::f64::consts::TAU;
            c + r * DVec2::new(a.cos(), a.sin())
        })
        .collect()
}

/// All extrudable regions of a sketch, in a stable order.
pub fn find_regions(sk: &Sketch) -> Vec<Region> {
    let (graph, whole_circles) = arrangement(sk);
    let tagged = {
        let mut t = graph_faces(&graph);
        t.extend(whole_circles.into_iter().map(|(c, r)| {
            (circle_loop(c, r), vec![Curve::Arc { center: c, radius: r }; CIRCLE_SEGMENTS])
        }));
        t
    };
    let loops: Vec<&Vec<DVec2>> = tagged.iter().map(|(p, _)| p).collect();
    let areas: Vec<f64> = loops.iter().map(|l| signed_area(l)).collect();
    let samples: Vec<DVec2> = loops.iter().map(|l| interior_point(l)).collect();

    // Parent = smallest loop that contains this one.
    let parent: Vec<Option<usize>> = (0..loops.len())
        .map(|i| {
            (0..loops.len())
                .filter(|&j| j != i && areas[j] > areas[i] && point_in_polygon(samples[i], &loops[j]))
                .min_by(|&a, &b| areas[a].total_cmp(&areas[b]))
        })
        .collect();

    (0..loops.len())
        .map(|i| {
            let (holes, hole_curves) = (0..loops.len())
                .filter(|&j| parent[j] == Some(i))
                .map(|j| reverse_loop(&tagged[j].0, &tagged[j].1))
                .unzip();
            Region { outer: tagged[i].0.clone(), outer_curves: tagged[i].1.clone(), holes, hole_curves }
        })
        .collect()
}

/// Merge regions into as few regions as possible: edges shared by two
/// selected regions cancel out, so adjacent profiles become one solid outline
/// (no internal walls). Works because neighbouring regions come from the same
/// arrangement and share exactly the same vertices.
pub fn union(regions: &[&Region]) -> Vec<Region> {
    type Key = (u64, u64);
    let key = |p: DVec2| -> Key { (p.x.to_bits(), p.y.to_bits()) };
    let mut pos: HashMap<Key, DVec2> = HashMap::new();
    let mut net: HashMap<(Key, Key), i32> = HashMap::new();
    let mut tag: HashMap<(Key, Key), Curve> = HashMap::new();
    for r in regions {
        for (l, curves) in r.loops_with_curves() {
            for i in 0..l.len() {
                let (a, b) = (l[i], l[(i + 1) % l.len()]);
                pos.insert(key(a), a);
                pos.insert(key(b), b);
                *net.entry((key(a), key(b))).or_default() += 1;
                tag.entry((key(a), key(b))).or_insert(curves[i]);
            }
        }
    }
    // Keep each directed edge as many times as it beats its reverse.
    let mut out_edges: HashMap<Key, Vec<Key>> = HashMap::new();
    let mut remaining = 0;
    for (&(a, b), &n) in &net {
        let keep = n - net.get(&(b, a)).copied().unwrap_or(0);
        for _ in 0..keep.max(0) {
            out_edges.entry(a).or_default().push(b);
            remaining += 1;
        }
    }
    // Trace closed loops, keeping each edge's tag.
    let mut loops: Vec<(Vec<DVec2>, Vec<Curve>)> = Vec::new();
    let mut starts: Vec<Key> = out_edges.keys().copied().collect();
    starts.sort(); // deterministic
    for start in starts {
        while out_edges.get(&start).is_some_and(|v| !v.is_empty()) {
            let (mut lp, mut lc) = (vec![pos[&start]], Vec::new());
            let mut cur = start;
            loop {
                let Some(next) = out_edges.get_mut(&cur).and_then(|v| v.pop()) else { break };
                remaining -= 1;
                lc.push(tag[&(cur, next)]);
                if next == start {
                    break;
                }
                lp.push(pos[&next]);
                cur = next;
            }
            if lp.len() >= 3 && lc.len() == lp.len() {
                loops.push((lp, lc));
            }
        }
    }
    debug_assert_eq!(remaining, 0);
    let (outers, holes): (Vec<_>, Vec<_>) = loops.into_iter().partition(|(l, _)| signed_area(l) > 0.0);
    let mut result: Vec<Region> = outers
        .into_iter()
        .map(|(outer, outer_curves)| Region { outer, outer_curves, holes: Vec::new(), hole_curves: Vec::new() })
        .collect();
    for (h, hc) in holes {
        let ccw: Vec<DVec2> = h.iter().rev().copied().collect();
        let inside = interior_point(&ccw);
        if let Some(r) = result
            .iter_mut()
            .filter(|r| point_in_polygon(inside, &r.outer))
            .min_by(|a, b| signed_area(&a.outer).total_cmp(&signed_area(&b.outer)))
        {
            r.holes.push(h);
            r.hole_curves.push(hc);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sketch::{Anchor, Editor, OriginPlane, Tool};

    fn free(x: f64, y: f64) -> Anchor {
        Anchor::Free(DVec2::new(x, y))
    }

    fn rect(sk: &mut Sketch, x0: f64, y0: f64, x1: f64, y1: f64) {
        let mut ed = Editor::default();
        ed.set_tool(Tool::Line);
        let first = sk.points.len();
        ed.click(sk, free(x0, y0));
        ed.click(sk, free(x1, y0));
        ed.click(sk, free(x1, y1));
        ed.click(sk, free(x0, y1));
        ed.click(sk, Anchor::Point(first));
    }

    fn circle(sk: &mut Sketch, cx: f64, cy: f64, r: f64) {
        let mut ed = Editor::default();
        ed.set_tool(Tool::Circle);
        ed.click(sk, free(cx, cy));
        ed.click(sk, free(cx + r, cy));
    }

    fn new() -> Sketch {
        Sketch::new("S".into(), OriginPlane::XY)
    }

    #[test]
    fn rectangle_is_one_profile() {
        let mut sk = new();
        rect(&mut sk, 0.0, 0.0, 50.0, 30.0);
        let r = find_regions(&sk);
        assert_eq!(r.len(), 1);
        assert!((r[0].area() - 1500.0).abs() < 1e-9);
        assert!(r[0].contains(DVec2::new(10.0, 10.0)));
        assert!(!r[0].contains(DVec2::new(60.0, 10.0)));
    }

    #[test]
    fn rectangle_drawn_clockwise_still_works() {
        let mut sk = new();
        rect(&mut sk, 0.0, 30.0, 50.0, 0.0); // reversed winding
        assert!((find_regions(&sk)[0].area() - 1500.0).abs() < 1e-9);
    }

    #[test]
    fn circle_inside_rectangle_makes_ring_and_disc() {
        let mut sk = new();
        rect(&mut sk, 0.0, 0.0, 50.0, 30.0);
        circle(&mut sk, 25.0, 15.0, 5.0);
        let r = find_regions(&sk);
        assert_eq!(r.len(), 2);
        let disc = signed_area(&circle_loop(DVec2::ZERO, 5.0));
        assert!((r[0].area() - (1500.0 - disc)).abs() < 1e-6, "rectangle minus hole");
        assert!((r[1].area() - disc).abs() < 1e-6);
        assert!(!r[0].contains(DVec2::new(25.0, 15.0)), "hole is not part of the ring");
        assert!(r[1].contains(DVec2::new(25.0, 15.0)));
    }

    #[test]
    fn diagonal_splits_into_two_triangles() {
        let mut sk = new();
        rect(&mut sk, 0.0, 0.0, 40.0, 40.0);
        let mut ed = Editor::default();
        ed.set_tool(Tool::Line);
        ed.click(&mut sk, Anchor::Point(1));
        ed.click(&mut sk, Anchor::Point(3));
        let r = find_regions(&sk);
        assert_eq!(r.len(), 2);
        for reg in &r {
            assert!((reg.area() - 800.0).abs() < 1e-9);
            assert!(reg.holes.is_empty());
        }
    }

    #[test]
    fn open_chain_and_dangling_line_are_ignored() {
        let mut sk = new();
        let mut ed = Editor::default();
        ed.set_tool(Tool::Line);
        ed.click(&mut sk, free(0.0, 0.0));
        ed.click(&mut sk, free(10.0, 0.0));
        ed.click(&mut sk, free(10.0, 10.0));
        assert!(find_regions(&sk).is_empty(), "open chain");
        ed.cancel();
        rect(&mut sk, 20.0, 0.0, 40.0, 20.0);
        ed.click(&mut sk, Anchor::Point(4)); // corner of the rectangle
        ed.click(&mut sk, free(60.0, 0.0)); // dangling tail
        let r = find_regions(&sk);
        assert_eq!(r.len(), 1);
        assert!((r[0].area() - 400.0).abs() < 1e-9);
    }

    fn line(sk: &mut Sketch, a: (f64, f64), b: (f64, f64)) {
        let mut ed = Editor::default();
        ed.set_tool(Tool::Line);
        ed.click(sk, free(a.0, a.1));
        ed.click(sk, free(b.0, b.1));
    }

    #[test]
    fn two_circles_with_tangent_lines_make_a_belt() {
        // Circles r=10 at x=0 and x=40, lines tangent top and bottom, overhanging both ends.
        let mut sk = new();
        circle(&mut sk, 0.0, 0.0, 10.0);
        circle(&mut sk, 40.0, 0.0, 10.0);
        line(&mut sk, (-20.0, 10.0), (60.0, 10.0));
        line(&mut sk, (-20.0, -10.0), (60.0, -10.0));
        let r = find_regions(&sk);
        assert_eq!(r.len(), 3, "two discs + the region between them");
        let disc = signed_area(&circle_loop(DVec2::ZERO, 10.0));
        let mut areas: Vec<f64> = r.iter().map(|x| x.area()).collect();
        areas.sort_by(|a, b| a.total_cmp(b));
        // Between: 40×20 rectangle minus the two half-discs inside it.
        assert!((areas[0] - disc).abs() < 1e-6 && (areas[1] - disc).abs() < 1e-6, "{areas:?}");
        assert!((areas[2] - (800.0 - disc)).abs() < 1e-6, "{areas:?}");
        let mid = r.iter().find(|x| x.contains(DVec2::new(20.0, 0.0))).unwrap();
        assert!(!mid.contains(DVec2::new(0.0, 0.0)), "the discs are separate profiles");
    }

    #[test]
    fn belt_profiles_union_into_one_outline() {
        let mut sk = new();
        circle(&mut sk, 0.0, 0.0, 10.0);
        circle(&mut sk, 40.0, 0.0, 10.0);
        line(&mut sk, (-20.0, 10.0), (60.0, 10.0));
        line(&mut sk, (-20.0, -10.0), (60.0, -10.0));
        let r = find_regions(&sk);
        let all: Vec<&Region> = r.iter().collect();
        let u = union(&all);
        assert_eq!(u.len(), 1, "one solid outline, no internal walls");
        assert!(u[0].holes.is_empty());
        let total: f64 = r.iter().map(|x| x.area()).sum();
        assert!((u[0].area() - total).abs() < 1e-6);
    }

    #[test]
    fn ring_plus_its_disc_unions_to_full_rectangle() {
        let mut sk = new();
        rect(&mut sk, 0.0, 0.0, 50.0, 30.0);
        circle(&mut sk, 25.0, 15.0, 5.0);
        let r = find_regions(&sk);
        let u = union(&[&r[0], &r[1]]);
        assert_eq!(u.len(), 1);
        assert!(u[0].holes.is_empty());
        assert!((u[0].area() - 1500.0).abs() < 1e-6);
        // The ring alone keeps its hole.
        let ring = union(&[&r[0]]);
        assert_eq!(ring[0].holes.len(), 1);
    }

    #[test]
    fn separate_regions_stay_separate() {
        let mut sk = new();
        rect(&mut sk, 0.0, 0.0, 10.0, 10.0);
        rect(&mut sk, 20.0, 0.0, 30.0, 10.0);
        let r = find_regions(&sk);
        assert_eq!(union(&[&r[0], &r[1]]).len(), 2);
    }

    /// Every edge tagged as an arc must have both ends on that circle.
    fn tags_match_geometry(r: &Region) {
        for (pts, curves) in r.loops_with_curves() {
            assert_eq!(pts.len(), curves.len());
            for i in 0..pts.len() {
                if let Curve::Arc { center, radius } = curves[i] {
                    for p in [pts[i], pts[(i + 1) % pts.len()]] {
                        assert!((p.distance(center) - radius).abs() < 1e-6, "arc tag off its circle");
                    }
                }
            }
        }
    }

    #[test]
    fn curve_tags_follow_every_loop() {
        let mut sk = new();
        rect(&mut sk, 0.0, 0.0, 50.0, 30.0);
        circle(&mut sk, 25.0, 15.0, 5.0);
        circle(&mut sk, 80.0, 0.0, 10.0);
        circle(&mut sk, 120.0, 0.0, 10.0);
        line(&mut sk, (70.0, 10.0), (140.0, 10.0));
        line(&mut sk, (70.0, -10.0), (140.0, -10.0));
        let r = find_regions(&sk);
        for reg in &r {
            tags_match_geometry(reg);
        }
        // The ring's hole is the circle, reversed, and still tagged as that circle.
        let ring = r.iter().find(|x| x.holes.len() == 1).unwrap();
        assert!(ring.hole_curves[0].iter().all(|c| *c == Curve::Arc { center: DVec2::new(25.0, 15.0), radius: 5.0 }));
        assert!(ring.outer_curves.iter().all(|c| *c == Curve::Line));
        // Union of the belt keeps arc tags on the outside and drops the inner ones.
        let belt: Vec<&Region> = r.iter().filter(|x| x.outer.iter().all(|p| p.x > 60.0)).collect();
        assert_eq!(belt.len(), 3);
        let u = union(&belt);
        assert_eq!(u.len(), 1);
        tags_match_geometry(&u[0]);
        let arcs = u[0].outer_curves.iter().filter(|c| matches!(c, Curve::Arc { .. })).count();
        let lines = u[0].outer_curves.iter().filter(|c| matches!(c, Curve::Line)).count();
        assert!(arcs > 0 && lines == 2, "two straight sides, the rest arcs: {lines} lines");
    }

    #[test]
    fn reversing_a_loop_keeps_tags_on_their_edges() {
        let a = Curve::Arc { center: DVec2::ZERO, radius: 1.0 };
        let pts = [DVec2::new(0.0, 0.0), DVec2::new(1.0, 0.0), DVec2::new(1.0, 1.0)];
        let curves = [Curve::Line, a, Curve::Line]; // edge 1 is (1,0)→(1,1)
        let (rp, rc) = reverse_loop(&pts, &curves);
        assert_eq!(rp, vec![pts[2], pts[1], pts[0]]);
        assert_eq!(rc[0], a, "reversed edge 0 is (1,1)→(1,0): the arc");
        assert_eq!(rc[1], Curve::Line);
        assert_eq!(rc[2], Curve::Line);
    }

    #[test]
    fn line_crossing_a_rectangle_splits_it() {
        let mut sk = new();
        rect(&mut sk, 0.0, 0.0, 40.0, 20.0);
        line(&mut sk, (10.0, -5.0), (30.0, 25.0)); // no shared points
        let r = find_regions(&sk);
        assert_eq!(r.len(), 2);
        let total: f64 = r.iter().map(|x| x.area()).sum();
        assert!((total - 800.0).abs() < 1e-6);
    }

    #[test]
    fn overlapping_circles_make_three_regions() {
        let mut sk = new();
        circle(&mut sk, 0.0, 0.0, 10.0);
        circle(&mut sk, 10.0, 0.0, 10.0);
        let r = find_regions(&sk);
        assert_eq!(r.len(), 3, "left, lens, right");
        assert!(r.iter().any(|x| x.contains(DVec2::new(5.0, 0.0)) && !x.contains(DVec2::new(-5.0, 0.0))));
    }

    #[test]
    fn line_tangent_to_circle_from_a_corner() {
        // Rectangle whose top edge is tangent to a circle sitting on it: circle
        // touches but does not cut, so we get the rectangle and the disc.
        let mut sk = new();
        rect(&mut sk, 0.0, 0.0, 40.0, 20.0);
        circle(&mut sk, 20.0, 30.0, 10.0);
        let r = find_regions(&sk);
        assert_eq!(r.len(), 2);
    }

    #[test]
    fn nested_rectangles_and_triangulation_cover_area() {
        let mut sk = new();
        rect(&mut sk, 0.0, 0.0, 100.0, 100.0);
        rect(&mut sk, 20.0, 20.0, 80.0, 80.0);
        rect(&mut sk, 40.0, 40.0, 60.0, 60.0);
        let r = find_regions(&sk);
        assert_eq!(r.len(), 3);
        let areas: Vec<f64> = r.iter().map(|x| x.area()).collect();
        assert!((areas[0] - 6400.0).abs() < 1e-9, "outer minus middle");
        assert!((areas[1] - 3200.0).abs() < 1e-9, "middle minus inner");
        assert!((areas[2] - 400.0).abs() < 1e-9);
        for reg in &r {
            let (v, tris) = reg.triangulate();
            let sum: f64 = tris.iter().map(|t| (v[t[1]] - v[t[0]]).perp_dot(v[t[2]] - v[t[0]]) * 0.5).sum();
            assert!((sum - reg.area()).abs() < 1e-6, "triangles are CCW and cover exactly the region");
            assert!(reg.contains(reg.centroid()) || !reg.holes.is_empty());
        }
    }
}
