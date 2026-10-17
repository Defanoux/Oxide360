//! Geometric constraint solver.
//!
//! Unknowns: x/y of every free point, plus every circle radius.
//! Each constraint is one equation f_i(x) = 0. We solve with damped
//! Gauss-Newton using the minimum-norm step
//!     dx = -Jᵀ (J Jᵀ + λI)⁻¹ f
//! which moves the geometry as little as possible — unconstrained points stay
//! put, which is what you expect from CAD. Jacobian J is by central
//! differences. Degrees of freedom = unknowns − rank(J).

use crate::sketch::{Constraint, Entity, PointId, Sketch};
use glam::DVec2;

const TOL: f64 = 1e-9; // max residual (mm, or unitless for angles)
const MAX_ITER: usize = 60;
const DAMPING: f64 = 1e-10;
const MIN_GEOM: f64 = 1e-6; // smaller lines/radii count as collapsed
/// Relative "willingness to move" of a dragged point vs everything else.
/// Tiny = the dragged point only moves where the constraints force it to.
const DRAG_WEIGHT: f64 = 1e-8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Solved {
    pub dof: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SolveError {
    /// Constraints conflict (no solution reachable).
    NoConvergence,
    /// A solution exists only by collapsing a line or circle to nothing.
    Degenerate,
    /// A constraint references the wrong kind of entity.
    Invalid,
    /// The new constraint is already implied by existing ones (over-constrained).
    Redundant,
}

impl std::fmt::Display for SolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            SolveError::NoConvergence => "constraints conflict",
            SolveError::Degenerate => "would collapse geometry to zero size",
            SolveError::Invalid => "constraint doesn't fit the selected geometry",
            SolveError::Redundant => "over-constrained — that's already set by other constraints",
        })
    }
}

/// Where each unknown lives in the parameter vector.
struct Layout {
    point_ix: Vec<Option<usize>>,
    radius_ix: Vec<Option<usize>>,
    n: usize,
}

impl Layout {
    fn new(sk: &Sketch, pinned: &[PointId]) -> Self {
        let mut n = 0;
        let point_ix = (0..sk.points.len())
            .map(|p| {
                if sk.is_fixed(p) || pinned.contains(&p) {
                    None
                } else {
                    n += 2;
                    Some(n - 2)
                }
            })
            .collect();
        let radius_ix = sk
            .entities
            .iter()
            .map(|e| match e {
                Entity::Circle { .. } => {
                    n += 1;
                    Some(n - 1)
                }
                _ => None,
            })
            .collect();
        Layout { point_ix, radius_ix, n }
    }

    fn pack(&self, sk: &Sketch) -> Vec<f64> {
        let mut x = vec![0.0; self.n];
        for (p, ix) in self.point_ix.iter().enumerate() {
            if let Some(i) = *ix {
                x[i] = sk.points[p].x;
                x[i + 1] = sk.points[p].y;
            }
        }
        for (e, ix) in self.radius_ix.iter().enumerate() {
            if let (Some(i), Entity::Circle { radius, .. }) = (*ix, sk.entities[e]) {
                x[i] = radius;
            }
        }
        x
    }

    fn unpack(&self, sk: &mut Sketch, x: &[f64]) {
        for (p, ix) in self.point_ix.iter().enumerate() {
            if let Some(i) = *ix {
                sk.points[p] = DVec2::new(x[i], x[i + 1]);
            }
        }
        for (e, ix) in self.radius_ix.iter().enumerate() {
            if let (Some(i), Entity::Circle { radius, .. }) = (*ix, &mut sk.entities[e]) {
                *radius = x[i];
            }
        }
    }
}

struct Eval<'a> {
    sk: &'a Sketch,
    lay: &'a Layout,
    x: &'a [f64],
}

impl Eval<'_> {
    fn pt(&self, p: PointId) -> DVec2 {
        match self.lay.point_ix[p] {
            Some(i) => DVec2::new(self.x[i], self.x[i + 1]),
            None => self.sk.points[p],
        }
    }

    fn line(&self, e: usize) -> Option<(DVec2, DVec2)> {
        match *self.sk.entities.get(e)? {
            Entity::Line { a, b } => Some((self.pt(a), self.pt(b))),
            _ => None,
        }
    }

    fn circle(&self, e: usize) -> Option<(DVec2, f64)> {
        match *self.sk.entities.get(e)? {
            Entity::Circle { center, .. } => Some((self.pt(center), self.x[self.lay.radius_ix[e]?])),
            // An arc's radius is the distance to its first end.
            Entity::Arc { center, a, .. } => {
                let c = self.pt(center);
                Some((c, self.pt(a).distance(c)))
            }
            _ => None,
        }
    }

    /// Equation `k` of constraint `c` (only Midpoint has two: x and y).
    fn residual(&self, c: &Constraint, k: u8) -> Option<f64> {
        use Constraint::*;
        let dir = |e| self.line(e).map(|(a, b)| b - a);
        let unit_pair = |e1, e2| -> Option<(DVec2, DVec2)> {
            let (d1, d2) = (dir(e1)?, dir(e2)?);
            Some((d1 / d1.length().max(1e-12), d2 / d2.length().max(1e-12)))
        };
        Some(match *c {
            Horizontal(e) => {
                let (a, b) = self.line(e)?;
                b.y - a.y
            }
            Vertical(e) => {
                let (a, b) = self.line(e)?;
                b.x - a.x
            }
            Length(e, d) => dir(e)?.length() - d,
            Diameter(e, d) => 2.0 * self.circle(e)?.1 - d,
            Parallel(e1, e2) => {
                let (u, v) = unit_pair(e1, e2)?;
                u.perp_dot(v) // sin of angle between
            }
            Perpendicular(e1, e2) => {
                let (u, v) = unit_pair(e1, e2)?;
                u.dot(v) // cos of angle between
            }
            OnAxis(p, axis) => {
                if p >= self.sk.points.len() {
                    return None;
                }
                let q = self.pt(p);
                if axis == 0 { q.y } else { q.x }
            }
            Radius(e, r) => self.circle(e)?.1 - r,
            ArcEnds(e) => match *self.sk.entities.get(e)? {
                Entity::Ellipse { center, major, minor } => {
                    let c = self.pt(center);
                    let (u, v) = (self.pt(major) - c, self.pt(minor) - c);
                    u.dot(v) / (u.length() * v.length()).max(1e-12)
                }
                Entity::Arc { center, a, b } => {
                    let c = self.pt(center);
                    self.pt(a).distance(c) - self.pt(b).distance(c)
                }
                _ => return None,
            },
            Midpoint(p, a, b) => {
                if p.max(a).max(b) >= self.sk.points.len() {
                    return None;
                }
                let m = self.pt(p) * 2.0 - self.pt(a) - self.pt(b);
                if k == 0 { m.x } else { m.y }
            }
            OnLine(p, l) => {
                if p >= self.sk.points.len() {
                    return None;
                }
                let (a, b) = self.line(l)?;
                let d = b - a;
                d.perp_dot(self.pt(p) - a) / d.length().max(1e-12)
            }
            OnCircle(p, c) => {
                if p >= self.sk.points.len() {
                    return None;
                }
                let (center, r) = self.circle(c)?;
                self.pt(p).distance(center) - r
            }
            Distance(p, q, d) => {
                if p.max(q) >= self.sk.points.len() {
                    return None;
                }
                self.pt(p).distance(self.pt(q)) - d
            }
            Angle(e1, e2, deg) => {
                // sin(φ − target), φ the angle from line 1 to line 2: zero at
                // the target or 180° from it (lines have no direction).
                let (u, v) = unit_pair(e1, e2)?;
                let t = deg.to_radians();
                u.perp_dot(v) * t.cos() - u.dot(v) * t.sin()
            }
            Equal(e1, e2) => match (dir(e1), dir(e2), self.circle(e1), self.circle(e2)) {
                (Some(d1), Some(d2), _, _) => d1.length() - d2.length(),
                (_, _, Some((_, r1)), Some((_, r2))) => r1 - r2,
                _ => return None,
            },
            Tangent(l, c) => {
                // A line and arc sharing an end (a fillet): smooth there, i.e.
                // the radius to that end is square to the line. (The "distance
                // = radius" form below has a double root at a shared end,
                // which the solver can't use.)
                if let (Some(&Entity::Arc { center, a: ea, b: eb }), Some(&Entity::Line { a: la, b: lb })) =
                    (self.sk.entities.get(c), self.sk.entities.get(l))
                {
                    if let Some(q) = [ea, eb].into_iter().find(|&q| q == la || q == lb) {
                        let d = self.pt(lb) - self.pt(la);
                        return Some((self.pt(q) - self.pt(center)).dot(d) / d.length().max(1e-12));
                    }
                }
                let (a, b) = self.line(l)?;
                let (center, r) = self.circle(c)?;
                let d = b - a;
                // distance from centre to the (infinite) line = radius
                d.perp_dot(center - a).abs() / d.length().max(1e-12) - r
            }
            Offset(o, c, d) => match (self.line(o), self.sk.entities.get(c)) {
                // Each end of the copied line (equation k) that far from the original's line.
                (Some((a, b)), Some(&Entity::Line { a: p, b: q })) => {
                    let n = b - a;
                    let x = self.pt(if k == 0 { p } else { q });
                    n.perp_dot(x - a) / n.length().max(1e-12) - d
                }
                // Round: radii that far apart (the centre is shared).
                _ => self.circle(c)?.1 - self.circle(o)?.1 - d,
            },
            Coincident(p, q) => {
                if p.max(q) >= self.sk.points.len() {
                    return None;
                }
                let d = self.pt(p) - self.pt(q);
                if k == 0 { d.x } else { d.y }
            }
            // Each end of the second line (equation k) on the first's line.
            Collinear(l1, l2) => {
                let (a, b) = self.line(l1)?;
                let (p, q) = self.line(l2)?;
                let n = b - a;
                n.perp_dot(if k == 0 { p } else { q } - a) / n.length().max(1e-12)
            }
            Concentric(c1, c2) => {
                let d = self.circle(c1)?.0 - self.circle(c2)?.0;
                if k == 0 { d.x } else { d.y }
            }
            // Equation 0: their middle on the line; 1: the pair square to it.
            Symmetric(p, q, l) => {
                if p.max(q) >= self.sk.points.len() {
                    return None;
                }
                let (a, b) = self.line(l)?;
                let d = (b - a) / (b - a).length().max(1e-12);
                let (x, y) = (self.pt(p), self.pt(q));
                if k == 0 { d.perp_dot((x + y) * 0.5 - a) } else { d.dot(x - y) }
            }
            SymmetricAxis(p, q, axis) => {
                if p.max(q) >= self.sk.points.len() {
                    return None;
                }
                let (x, y) = (self.pt(p), self.pt(q));
                // About X: same x, opposite y; about Y: same y, opposite x.
                match (axis, k) {
                    (0, 0) => x.y + y.y,
                    (0, _) => x.x - y.x,
                    (_, 0) => x.x + y.x,
                    _ => x.y - y.y,
                }
            }
            Fix(_) => return None, // handled by freezing the point, not an equation
        })
    }
}

/// The equations to satisfy: one per constraint (Midpoint gives two, Fix
/// none), plus one per arc keeping both its ends on the same circle.
fn equations(sk: &Sketch) -> Vec<(Constraint, u8)> {
    let arcs = sk
        .entities
        .iter()
        .enumerate()
        // (An ellipse's: its two axes kept square.)
        .filter(|(_, e)| matches!(e, Entity::Arc { .. } | Entity::Ellipse { .. }))
        .map(|(i, _)| (Constraint::ArcEnds(i), 0));
    sk.constraints
        .iter()
        .enumerate()
        // Driven (reference) dimensions only report; they're never solved.
        .filter(|&(ci, _)| !sk.is_driven(ci))
        .flat_map(|(_, &c)| match c {
            Constraint::Fix(_) | Constraint::ArcEnds(_) => vec![],
            Constraint::Midpoint(..) | Constraint::Coincident(..) | Constraint::Collinear(..) | Constraint::Concentric(..) | Constraint::Symmetric(..) | Constraint::SymmetricAxis(..) => vec![(c, 0), (c, 1)],
            Constraint::Offset(o, _, _) if sk.is_line(o) => vec![(c, 0), (c, 1)],
            _ => vec![(c, 0)],
        })
        .chain(arcs)
        .collect()
}

fn residuals(sk: &Sketch, lay: &Layout, eqs: &[(Constraint, u8)], x: &[f64]) -> Option<Vec<f64>> {
    let ev = Eval { sk, lay, x };
    eqs.iter()
        .map(|(c, k)| ev.residual(c, *k).filter(|r| r.is_finite()))
        .collect()
}

/// m×n Jacobian, row-major, by central differences.
fn jacobian(sk: &Sketch, lay: &Layout, eqs: &[(Constraint, u8)], x: &[f64]) -> Option<Vec<f64>> {
    let (m, n) = (eqs.len(), x.len());
    let mut j = vec![0.0; m * n];
    let mut xp = x.to_vec();
    for col in 0..n {
        let h = 1e-6 * (1.0 + x[col].abs());
        xp[col] = x[col] + h;
        let fp = residuals(sk, lay, eqs, &xp)?;
        xp[col] = x[col] - h;
        let fm = residuals(sk, lay, eqs, &xp)?;
        xp[col] = x[col];
        for row in 0..m {
            j[row * n + col] = (fp[row] - fm[row]) / (2.0 * h);
        }
    }
    Some(j)
}

/// Solve A y = b (A is m×m, row-major) by Gaussian elimination with partial pivoting.
fn solve_linear(mut a: Vec<f64>, mut b: Vec<f64>) -> Option<Vec<f64>> {
    let m = b.len();
    for k in 0..m {
        let piv = (k..m).max_by(|&i, &j| a[i * m + k].abs().total_cmp(&a[j * m + k].abs()))?;
        if a[piv * m + k].abs() < 1e-300 {
            return None;
        }
        if piv != k {
            for c in 0..m {
                a.swap(k * m + c, piv * m + c);
            }
            b.swap(k, piv);
        }
        for i in k + 1..m {
            let f = a[i * m + k] / a[k * m + k];
            if f != 0.0 {
                for c in k..m {
                    a[i * m + c] -= f * a[k * m + c];
                }
                b[i] -= f * b[k];
            }
        }
    }
    let mut y = vec![0.0; m];
    for k in (0..m).rev() {
        let s: f64 = (k + 1..m).map(|c| a[k * m + c] * y[c]).sum();
        y[k] = (b[k] - s) / a[k * m + k];
    }
    Some(y)
}

/// Numerical rank of an m×n matrix.
fn rank(mut a: Vec<f64>, m: usize, n: usize) -> usize {
    let scale = a.iter().fold(0.0f64, |s, v| s.max(v.abs())).max(1e-300);
    let tol = 1e-7 * scale;
    let mut r = 0;
    for col in 0..n {
        if r == m {
            break;
        }
        let piv = (r..m).max_by(|&i, &j| a[i * n + col].abs().total_cmp(&a[j * n + col].abs())).unwrap();
        if a[piv * n + col].abs() <= tol {
            continue;
        }
        for c in 0..n {
            a.swap(r * n + c, piv * n + c);
        }
        for i in r + 1..m {
            let f = a[i * n + col] / a[r * n + col];
            for c in col..n {
                a[i * n + c] -= f * a[r * n + c];
            }
        }
        r += 1;
    }
    r
}

fn norm2(v: &[f64]) -> f64 {
    v.iter().map(|x| x * x).sum()
}

/// Solve the sketch in place. `pinned` points are held exactly where they are.
/// On error the sketch is left untouched.
pub fn solve(sk: &mut Sketch, pinned: &[PointId]) -> Result<Solved, SolveError> {
    let w = ellipse_weights(sk);
    solve_with(sk, pinned, None, w.as_deref())
}

/// Step weights that keep ellipses from turning: an ellipse's centre and
/// second-axis point barely move, so changing its first axis grows it along
/// that axis instead of sliding the centre and turning the whole ellipse to
/// stay square. None when there are no ellipses.
fn ellipse_weights(sk: &Sketch) -> Option<Vec<f64>> {
    let held: Vec<PointId> = sk.entities.iter().flat_map(|e| if let Entity::Ellipse { center, minor, .. } = *e { vec![center, minor] } else { vec![] }).collect();
    if held.is_empty() {
        return None;
    }
    let mut w = vec![1.0; sk.points.len()];
    for p in held {
        w[p] = 1e-4;
    }
    Some(w)
}

/// Drag point `p` toward `target`, keeping every constraint.
/// First try holding it exactly at the target. If the constraints don't allow
/// that (e.g. a dimension fixes its height), let it move as little as possible
/// off the target instead — so it slides along whatever freedom it has left,
/// like dragging in other CAD tools. On error the sketch is left untouched.
pub fn drag(sk: &mut Sketch, p: PointId, target: DVec2) -> Result<Solved, SolveError> {
    if p >= sk.points.len() {
        return Err(SolveError::Invalid); // point was removed (e.g. undo mid-drag)
    }
    let stiff = stiffness(sk, p);
    // Straight to the target.
    let mut exact = sk.clone();
    exact.points[p] = target;
    if let Ok(s) = solve_with(&mut exact, &[p], None, Some(&stiff)) {
        if !collapsed(sk, &exact) {
            *sk = exact;
            return Ok(s);
        }
    }
    // Too far in one go (a far jump can land on a wrong branch, even a squashed
    // shape): walk there in small steps and stop where the constraints stop it.
    let start = sk.points[p];
    let mut cur = sk.clone();
    let mut last = None;
    for k in 1..=16 {
        let mut step = cur.clone();
        step.points[p] = start.lerp(target, k as f64 / 16.0);
        match solve_with(&mut step, &[p], None, Some(&stiff)) {
            Ok(s) if !collapsed(&cur, &step) => {
                cur = step;
                last = Some(s);
            }
            _ => break,
        }
    }
    if let Some(s) = last {
        *sk = cur;
        return Ok(s);
    }
    // Not reachable exactly (a dimension fixes its height, say): let it move
    // as little as possible off the target, sliding along what freedom is left.
    let mut trial = sk.clone();
    trial.points[p] = target;
    let s = solve_with(&mut trial, &[], Some(p), Some(&stiff))?;
    if collapsed(sk, &trial) {
        return Err(SolveError::Degenerate);
    }
    *sk = trial;
    Ok(s)
}

/// Did solving squash something flat: any line, circle or arc now under a
/// twentieth of its size before (that no dimension asked for)?
fn collapsed(before: &Sketch, after: &Sketch) -> bool {
    let size = |sk: &Sketch, e: usize| match sk.entities[e] {
        Entity::Line { .. } => sk.line_length(e).unwrap_or(0.0),
        Entity::Circle { radius, .. } => radius,
        Entity::Arc { .. } => sk.arc_geometry(e).map_or(0.0, |(_, r, _, _)| r),
        Entity::Point { .. } | Entity::Text { .. } => 0.0,
        Entity::Spline { s } => sk.spline_fit(s).windows(2).map(|w| w[0].distance(w[1])).sum(),
        Entity::Ellipse { .. } => sk.ellipse_geometry(e).map_or(0.0, |(_, _, _, b)| b),
    };
    (0..before.entities.len().min(after.entities.len())).any(|e| {
        let (b, a) = (size(before, e), size(after, e));
        b > 1e-6 && a < b * 0.05
    })
}

/// Drag a whole line by `shift`: both ends move together to where `shift`
/// puts them from `from` (their positions when the drag started), keeping every
/// constraint (a horizontal line stays horizontal, a collinear one collinear).
/// If the constraints won't allow the full move, only the part across the line
/// or only the part along it is tried, each as far as it can go (a half, a
/// quarter … of it); never an error for that, the line just stays.
pub fn drag_line(sk: &mut Sketch, e: usize, from: [DVec2; 2], shift: DVec2) -> Result<Solved, SolveError> {
    let Some(&Entity::Line { a, b }) = sk.entities.get(e) else { return Err(SolveError::Invalid) };
    // A line with a fixed end stays put (as a fixed point does when dragged).
    if sk.is_fixed(a) || sk.is_fixed(b) {
        return solve(&mut sk.clone(), &[]);
    }
    let stiff = stiffness(sk, a);
    let along = (from[1] - from[0]).normalize_or_zero();
    let parts = [shift, along.perp() * shift.dot(along.perp()), along * shift.dot(along)];
    for part in parts {
        let mut scale = 1.0;
        for _ in 0..8 {
            let mut trial = sk.clone();
            trial.points[a] = from[0] + part * scale;
            trial.points[b] = from[1] + part * scale;
            if let Ok(s) = solve_with(&mut trial, &[a, b], None, Some(&stiff)) {
                *sk = trial;
                return Ok(s);
            }
            scale *= 0.5;
        }
    }
    solve(&mut sk.clone(), &[])
}

/// How willing each point is to move while `p` is dragged: points joined to
/// it move freely, points further away along the shape barely move. So
/// dragging a rectangle's corner moves the two corners next to it (changing
/// width and height) and leaves the opposite corner where it is.
fn stiffness(sk: &Sketch, p: PointId) -> Vec<f64> {
    let n = sk.points.len();
    let mut next: Vec<Vec<PointId>> = vec![Vec::new(); n];
    let mut join = |a: PointId, b: PointId| {
        if a < n && b < n && a != b {
            next[a].push(b);
            next[b].push(a);
        }
    };
    for (i, e) in sk.entities.iter().enumerate() {
        // Construction lines (a rectangle's diagonals) don't tie points together.
        if let (Entity::Line { a, b }, false) = (*e, sk.is_construction(i)) {
            join(a, b);
        }
    }
    for (ci, c) in sk.constraints.iter().enumerate() {
        if sk.is_driven(ci) {
            continue;
        }
        match *c {
            Constraint::Distance(a, b, _) | Constraint::Coincident(a, b) => join(a, b),
            Constraint::Collinear(l1, l2) => {
                // Each end of the second line to the nearer end of the first, so
                // a drag turns the shared line about the far end.
                if let (Some(&Entity::Line { a, b }), Some(&Entity::Line { a: p, b: q })) = (sk.entities.get(l1), sk.entities.get(l2)) {
                    for x in [p, q] {
                        let near = if sk.points[x].distance(sk.points[a]) <= sk.points[x].distance(sk.points[b]) { a } else { b };
                        join(near, x);
                    }
                }
            }
            Constraint::Concentric(c1, c2) => {
                let centre = |e: usize| match sk.entities.get(e) {
                    Some(Entity::Circle { center, .. } | Entity::Arc { center, .. }) => Some(*center),
                    _ => None,
                };
                if let (Some(a), Some(b)) = (centre(c1), centre(c2)) {
                    join(a, b);
                }
            }
            // An offset copy moves with its original.
            Constraint::Offset(o, c, _) => {
                if let (Some(Entity::Line { a, b }), Some(Entity::Line { a: p, b: q })) = (sk.entities.get(o), sk.entities.get(c)) {
                    join(*a, *p);
                    join(*b, *q);
                }
            }
            // Mirror images move together.
            Constraint::Symmetric(p, q, _) | Constraint::SymmetricAxis(p, q, _) => join(p, q),
            // A centre is joined to each corner (not the corners to each other).
            Constraint::Midpoint(m, a, b) => {
                join(m, a);
                join(m, b);
            }
            Constraint::OnLine(q, l) => {
                if let Some(Entity::Line { a, b }) = sk.entities.get(l) {
                    join(q, *a);
                    join(q, *b);
                }
            }
            Constraint::OnCircle(q, c) => {
                if let Some(Entity::Circle { center, .. }) = sk.entities.get(c) {
                    join(q, *center);
                }
            }
            _ => {}
        }
    }
    // Hops from p (breadth first).
    let mut hops = vec![usize::MAX; n];
    let mut queue = std::collections::VecDeque::new();
    if p < n {
        hops[p] = 0;
        queue.push_back(p);
    }
    while let Some(a) = queue.pop_front() {
        for &b in &next[a] {
            if hops[b] == usize::MAX {
                hops[b] = hops[a] + 1;
                queue.push_back(b);
            }
        }
    }
    hops.iter()
        .map(|&h| match h {
            0 | 1 => 1.0,
            2 => 1e-6,
            _ => 1e-8,
        })
        .collect()
}

/// `soft`: a point whose coordinates get a tiny step weight, so the rest of
/// the sketch adjusts first and it only moves where it must. `stiff`: per
/// point weights (see `stiffness`); all 1 when None.
fn solve_with(sk: &mut Sketch, pinned: &[PointId], soft: Option<PointId>, stiff: Option<&[f64]>) -> Result<Solved, SolveError> {
    let lay = Layout::new(sk, pinned);
    let eqs = equations(sk);
    let (m, n) = (eqs.len(), lay.n);
    let mut x = lay.pack(sk);
    // Weighted minimum-norm step: dx = -W Jᵀ (J W Jᵀ + λI)⁻¹ f
    let mut w = vec![1.0; n];
    if let Some(stiff) = stiff {
        for (q, ix) in lay.point_ix.iter().enumerate() {
            if let (Some(i), Some(&s)) = (*ix, stiff.get(q)) {
                w[i] = s;
                w[i + 1] = s;
            }
        }
    }
    if let Some(Some(ix)) = soft.map(|p| lay.point_ix[p]) {
        w[ix] = DRAG_WEIGHT;
        w[ix + 1] = DRAG_WEIGHT;
    }
    let mut f = residuals(sk, &lay, &eqs, &x).ok_or(SolveError::Invalid)?;

    let mut converged = f.iter().all(|r| r.abs() < TOL);
    for _ in 0..MAX_ITER {
        if converged {
            break;
        }
        let j = jacobian(sk, &lay, &eqs, &x).ok_or(SolveError::NoConvergence)?;
        // A = J W Jᵀ + λI
        let mut a = vec![0.0; m * m];
        for r in 0..m {
            for c in r..m {
                let v: f64 = (0..n).map(|k| j[r * n + k] * w[k] * j[c * n + k]).sum();
                a[r * m + c] = v;
                a[c * m + r] = v;
            }
            a[r * m + r] += DAMPING;
        }
        let y = solve_linear(a, f.clone()).ok_or(SolveError::NoConvergence)?;
        let dx: Vec<f64> = (0..n).map(|k| -w[k] * (0..m).map(|r| j[r * n + k] * y[r]).sum::<f64>()).collect();

        // Backtracking line search: only accept steps that reduce the error.
        let mut step = 1.0;
        loop {
            let xn: Vec<f64> = x.iter().zip(&dx).map(|(a, d)| a + step * d).collect();
            if let Some(fnew) = residuals(sk, &lay, &eqs, &xn) {
                if norm2(&fnew) < norm2(&f) {
                    x = xn;
                    f = fnew;
                    break;
                }
            }
            step *= 0.5;
            if step < 1e-6 {
                return Err(SolveError::NoConvergence);
            }
        }
        converged = f.iter().all(|r| r.abs() < TOL);
    }
    if !converged {
        return Err(SolveError::NoConvergence);
    }

    let mut out = sk.clone();
    lay.unpack(&mut out, &x);
    for (e, ent) in out.entities.iter().enumerate() {
        let size = match *ent {
            Entity::Line { .. } => out.line_length(e).unwrap_or(0.0),
            Entity::Circle { radius, .. } => radius,
            Entity::Arc { .. } => out.arc_geometry(e).map_or(0.0, |(_, r, _, _)| r),
            Entity::Point { .. } | Entity::Text { .. } => continue,
            Entity::Spline { s } => out.spline_fit(s).windows(2).map(|w| w[0].distance(w[1])).sum(),
            Entity::Ellipse { .. } => out.ellipse_geometry(e).map_or(0.0, |(_, _, _, b)| b),
        };
        if size < MIN_GEOM {
            return Err(SolveError::Degenerate);
        }
    }

    let dof = if m == 0 {
        n
    } else {
        let j = jacobian(&out, &lay, &eqs, &x).ok_or(SolveError::NoConvergence)?;
        n - rank(j, m, n)
    };
    // Pinned points are only held temporarily; they still count as free.
    let dof = dof + 2 * pinned.iter().filter(|&&p| !out.is_fixed(p)).count();
    *sk = out;
    Ok(Solved { dof })
}

/// Add constraints, refusing any that don't remove a degree of freedom.
/// A redundant constraint does nothing now, but makes the sketch conflict as
/// soon as a dimension changes — so, like other CAD tools, we reject it.
/// On error the sketch is left untouched.
pub fn add_constraints(sk: &mut Sketch, cs: &[Constraint]) -> Result<Solved, SolveError> {
    let before = solve(&mut sk.clone(), &[])?.dof;
    let mut trial = sk.clone();
    trial.constraints.extend_from_slice(cs);
    let solved = solve(&mut trial, &[])?;
    // Each added constraint must take away at least one degree of freedom.
    if before.saturating_sub(solved.dof) < cs.len() {
        return Err(SolveError::Redundant);
    }
    *sk = trial;
    Ok(solved)
}

/// Points that are "virtual sharps" left by a fillet or chamfer: a lone point
/// held on two lines where the corner used to be.
fn virtual_sharps(sk: &Sketch) -> Vec<PointId> {
    (0..sk.entities.len())
        .filter_map(|e| match sk.entities[e] {
            Entity::Point { p } => Some(p),
            _ => None,
        })
        .filter(|&p| sk.constraints.iter().filter(|c| matches!(c, Constraint::OnLine(q, _) if *q == p)).count() >= 2)
        .collect()
}

/// Dimensions typed as expressions take their values again (after the
/// design's parameters changed — see `expr::set_scope`); `unit`: the
/// design's. Returns the dimensions that couldn't, with why.
pub fn follow_parameters(sk: &mut Sketch, unit: crate::units::Unit) -> Vec<(usize, String)> {
    let mut failed = Vec::new();
    for ci in 0..sk.constraints.len() {
        let Some(text) = sk.dim_expr(ci).map(str::to_string) else { continue };
        let is_angle = matches!(sk.constraints[ci], Constraint::Angle(..));
        let kind = if is_angle { crate::expr::Kind::Angle } else { crate::expr::Kind::Length };
        let v = match crate::expr::eval(&text, kind, sk.dim_unit(ci).unwrap_or(unit)) {
            Ok(v) if is_angle && !(0.0..180.0).contains(&v) => Err("an angle from 0° up to 180°".to_string()),
            Ok(v) if !is_angle && v <= 0.0 => Err("it must be greater than zero".to_string()),
            other => other,
        };
        match v {
            Ok(v) if sk.constraints[ci].value().is_some_and(|now| (now - v).abs() > 1e-9) => {
                if set_dimension_value(sk, ci, v).is_err() {
                    failed.push((ci, "the sketch can't take that value".to_string()));
                }
            }
            Ok(_) => {}
            Err(e) => failed.push((ci, e)),
        }
    }
    failed
}

/// Change the value of dimension `ci` and re-solve. A chamfer's distances
/// are measured from its virtual sharp, which stays put: only the chamfer's
/// own end moves, instead of the whole corner sliding to make room.
pub fn set_dimension_value(sk: &mut Sketch, ci: usize, v: f64) -> Result<Solved, SolveError> {
    let mut trial = sk.clone();
    // (An offset's pieces all share its one value.)
    for k in trial.offset_group(ci) {
        set_value(trial.constraints.get_mut(k).ok_or(SolveError::Invalid)?, v);
    }
    let pins = match trial.constraints[ci] {
        Constraint::Distance(a, b, _) => virtual_sharps(&trial).into_iter().filter(|&p| p == a || p == b).collect(),
        Constraint::Offset(..) => {
            // Like Fusion, the copy moves and the original stays (if it can):
            // hold the originals' points and their circles' size.
            let mut held = trial.clone();
            let mut pins = Vec::new();
            for k in trial.offset_group(ci) {
                let Constraint::Offset(o, _, _) = trial.constraints[k] else { continue };
                pins.extend(trial.entity_points(o));
                if let Some(&Entity::Circle { radius, .. }) = trial.entities.get(o) {
                    held.constraints.push(Constraint::Diameter(o, 2.0 * radius));
                }
            }
            if solve(&mut held, &pins).is_ok() {
                trial.points = held.points;
                trial.entities = held.entities;
            }
            pins
        }
        _ => Vec::new(),
    };
    let solved = match solve(&mut trial.clone(), &pins) {
        Ok(_) => solve(&mut trial, &pins),
        Err(_) => solve(&mut trial, &[]),
    }?;
    *sk = trial;
    Ok(solved)
}

/// Add a dimension. If it would over-constrain the sketch it is added as a
/// *driven* (reference) dimension instead, holding the measured value.
/// Returns whether it ended up driven.
pub fn add_dimension(sk: &mut Sketch, c: Constraint) -> Result<(Solved, bool), SolveError> {
    match add_constraints(sk, &[c]) {
        Ok(s) => Ok((s, false)),
        Err(SolveError::Redundant) => {
            let mut trial = sk.clone();
            let mut c = c;
            let v = trial.measure(&c).ok_or(SolveError::Redundant)?;
            set_value(&mut c, v);
            trial.constraints.push(c);
            let ci = trial.constraints.len() - 1;
            trial.set_driven(ci, true);
            let solved = solve(&mut trial.clone(), &[])?;
            *sk = trial;
            Ok((solved, true))
        }
        Err(e) => Err(e),
    }
}

/// Make driven dimension `ci` a driving one again, holding what it measures
/// now. Refused (sketch untouched) if the sketch is over-constrained by it.
pub fn make_driving(sk: &mut Sketch, ci: usize) -> Result<Solved, SolveError> {
    let before = solve(&mut sk.clone(), &[])?.dof;
    let mut trial = sk.clone();
    let mut c = *trial.constraints.get(ci).ok_or(SolveError::Redundant)?;
    let v = trial.measure(&c).ok_or(SolveError::Redundant)?;
    set_value(&mut c, v);
    trial.constraints[ci] = c;
    trial.set_driven(ci, false);
    let solved = solve(&mut trial, &[])?;
    if before.saturating_sub(solved.dof) < 1 {
        return Err(SolveError::Redundant);
    }
    *sk = trial;
    Ok(solved)
}

/// Make dimension `ci` a driven one, showing the geometry's value from now on.
pub fn make_driven(sk: &mut Sketch, ci: usize) {
    if let Some(mut c) = sk.constraints.get(ci).copied() {
        if let Some(v) = sk.measure(&c) {
            set_value(&mut c, v);
        }
        sk.constraints[ci] = c;
        sk.set_driven(ci, true);
    }
}

fn set_value(c: &mut Constraint, v: f64) {
    c.set_value(v);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sketch::{Anchor, Editor, OriginPlane, Sel, Tool};
    use Constraint::*;

    fn free(x: f64, y: f64) -> Anchor {
        Anchor::Free(DVec2::new(x, y))
    }

    /// Two free lines and two free circles, nothing joined.
    fn loose() -> Sketch {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        sk.points.extend([
            DVec2::new(1.0, 1.0), DVec2::new(6.0, 2.0), // line 0: points 1, 2
            DVec2::new(8.0, 5.0), DVec2::new(12.0, 9.0), // line 1: points 3, 4
            DVec2::new(20.0, 3.0), DVec2::new(26.0, 4.0), // circle centres: 5, 6
        ]);
        sk.entities.extend([
            Entity::Line { a: 1, b: 2 },
            Entity::Line { a: 3, b: 4 },
            Entity::Circle { center: 5, radius: 2.0 },
            Entity::Circle { center: 6, radius: 3.0 },
        ]);
        sk
    }

    #[test]
    fn dimensions_typed_as_expressions_follow_the_parameters() {
        use crate::expr::{ParamUnit, UserParameter};
        let param = |e: &str| vec![UserParameter { name: "width".into(), unit: ParamUnit::Mm, expression: e.into(), comment: String::new() }];
        let mut sk = loose();
        add_constraints(&mut sk, &[Length(0, 5.0), Length(1, 4.0)]).unwrap();
        crate::expr::set_scope(&param("20"));
        sk.set_dim_expr(0, Some("width / 2".into()));
        assert!(follow_parameters(&mut sk, crate::units::Unit::Mm).is_empty());
        let len = |sk: &Sketch, e: usize| match sk.entities[e] {
            Entity::Line { a, b } => sk.points[a].distance(sk.points[b]),
            _ => unreachable!(),
        };
        assert!((len(&sk, 0) - 10.0).abs() < 1e-9, "{}", len(&sk, 0));
        crate::expr::set_scope(&param("30"));
        follow_parameters(&mut sk, crate::units::Unit::Mm);
        assert!((len(&sk, 0) - 15.0).abs() < 1e-9);
        assert!((len(&sk, 1) - 4.0).abs() < 1e-9, "a plain dimension stays");
        // A value it can't take: reported, the old one stays.
        crate::expr::set_scope(&param("-4"));
        assert_eq!(follow_parameters(&mut sk, crate::units::Unit::Mm).len(), 1);
        assert!((len(&sk, 0) - 15.0).abs() < 1e-9);
        // Deleting a constraint before it keeps the expression with its dimension.
        sk.retain_constraints(|c| !matches!(c, Length(1, _)));
        assert_eq!(sk.dim_expr(0), Some("width / 2"));
        crate::expr::set_scope(&[]);
    }

    #[test]
    fn coincident_collinear_and_concentric_hold_and_take_freedom() {
        let mut sk = loose();
        let dof0 = solve(&mut sk.clone(), &[]).unwrap().dof;
        add_constraints(&mut sk, &[Coincident(2, 3)]).unwrap();
        assert!(sk.points[2].distance(sk.points[3]) < 1e-9);
        add_constraints(&mut sk, &[Concentric(2, 3)]).unwrap();
        assert!(sk.points[5].distance(sk.points[6]) < 1e-9);
        let mut sk2 = loose();
        add_constraints(&mut sk2, &[Collinear(0, 1)]).unwrap();
        let (a, b) = (sk2.points[1], sk2.points[2]);
        for q in [sk2.points[3], sk2.points[4]] {
            assert!((b - a).perp_dot(q - a).abs() / (b - a).length() < 1e-9, "{q}");
        }
        assert_eq!(solve(&mut sk.clone(), &[]).unwrap().dof, dof0 - 4, "two each");
        // Coincident twice is redundant.
        assert!(add_constraints(&mut sk, &[Coincident(3, 2)]).is_err());
    }

    #[test]
    fn a_horizontal_line_goes_onto_the_x_axis_one_end_at_a_time() {
        // Collinear with the X axis = both ends on it; with Horizontal already
        // there the second end is implied, so it's skipped (the app adds them one by one).
        let mut sk = loose();
        add_constraints(&mut sk, &[Horizontal(0)]).unwrap();
        let both = [OnAxis(1, 0), OnAxis(2, 0)];
        assert!(add_constraints(&mut sk.clone(), &both).is_err(), "together: refused as redundant");
        add_constraints(&mut sk, &both[..1]).unwrap();
        assert!(add_constraints(&mut sk.clone(), &both[1..]).is_err());
        assert!(sk.points[1].y.abs() < 1e-9 && sk.points[2].y.abs() < 1e-9, "on the axis anyway");
    }

    #[test]
    fn the_sketch_axes_can_be_picked_for_constraints() {
        let mut sk = loose();
        // Near the X axis, away from geometry: the axis.
        assert_eq!(sk.hit(DVec2::new(40.0, 0.05), 0.1), Some(Sel::Axis(0)));
        assert_eq!(sk.hit(DVec2::new(-0.05, 40.0), 0.1), Some(Sel::Axis(1)));
        let opts = |sk: &Sketch, sel: &[Sel]| sk.constraint_options(sel);
        let pick = |o: Vec<(&'static str, Vec<Constraint>)>, l: &str| o.into_iter().find(|(x, _)| *x == l).map(|(_, c)| c);
        assert_eq!(pick(opts(&sk, &[Sel::Point(1), Sel::Axis(0)]), "Coincident"), Some(vec![OnAxis(1, 0)]));
        assert_eq!(pick(opts(&sk, &[Sel::Entity(0), Sel::Axis(0)]), "Parallel"), Some(vec![Horizontal(0)]));
        assert_eq!(pick(opts(&sk, &[Sel::Entity(0), Sel::Axis(0)]), "Perpendicular"), Some(vec![Vertical(0)]));
        let along = pick(opts(&sk, &[Sel::Entity(0), Sel::Axis(1)]), "Collinear").unwrap();
        add_constraints(&mut sk, &along).unwrap();
        assert!(sk.points[1].x.abs() < 1e-9 && sk.points[2].x.abs() < 1e-9);
        // Circle centre onto an axis.
        assert_eq!(pick(opts(&sk, &[Sel::Entity(2), Sel::Axis(0)]), "Coincident"), Some(vec![OnAxis(5, 0)]));
        // Two points: coincident; point and circle: onto it.
        assert_eq!(pick(opts(&sk, &[Sel::Point(3), Sel::Point(4)]), "Coincident"), Some(vec![Coincident(3, 4)]));
        assert_eq!(pick(opts(&sk, &[Sel::Point(3), Sel::Entity(2)]), "Coincident"), Some(vec![OnCircle(3, 2)]));
    }

    /// Rough rectangle from origin: lines 0..3, points 0(origin),1,2,3.
    fn rough_rect() -> Sketch {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        let mut ed = Editor::default();
        ed.set_tool(Tool::Line);
        ed.click(&mut sk, Anchor::Point(0));
        ed.click(&mut sk, free(48.0, 3.0));
        ed.click(&mut sk, free(52.0, 28.0));
        ed.click(&mut sk, free(-2.0, 31.0));
        ed.click(&mut sk, Anchor::Point(0));
        sk
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn no_constraints_means_all_free() {
        let mut sk = rough_rect();
        let before = sk.clone();
        let s = solve(&mut sk, &[]).unwrap();
        assert_eq!(s.dof, 6, "3 free points × 2");
        assert_eq!(sk, before, "nothing moves");
    }

    #[test]
    fn fully_constrained_rectangle() {
        let mut sk = rough_rect();
        sk.constraints = vec![
            Horizontal(0),
            Vertical(1),
            Horizontal(2),
            Vertical(3),
            Length(0, 50.0),
            Length(1, 30.0),
        ];
        let s = solve(&mut sk, &[]).unwrap();
        assert_eq!(s.dof, 0);
        let p = &sk.points;
        assert!(close(p[1].x, 50.0) && close(p[1].y, 0.0));
        assert!(close(p[2].x, 50.0) && close(p[2].y, 30.0));
        assert!(close(p[3].x, 0.0) && close(p[3].y, 30.0));
    }

    #[test]
    fn rectangle_with_parallel_perpendicular_equal() {
        let mut sk = rough_rect();
        sk.constraints = vec![
            Horizontal(0),
            Perpendicular(0, 1),
            Parallel(0, 2),
            Parallel(1, 3),
            Equal(0, 1),
            Length(0, 40.0),
        ];
        let s = solve(&mut sk, &[]).unwrap();
        assert_eq!(s.dof, 0);
        for e in 0..4 {
            assert!(close(sk.line_length(e).unwrap(), 40.0), "square side {e}");
        }
    }

    #[test]
    fn diameter_and_tangent() {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        let mut ed = Editor::default();
        ed.set_tool(Tool::Line);
        ed.click(&mut sk, free(-50.0, 12.0));
        ed.click(&mut sk, free(50.0, 14.0)); // line 0
        ed.set_tool(Tool::Circle);
        ed.click(&mut sk, Anchor::Point(0));
        ed.click(&mut sk, free(8.0, 0.0)); // circle 1 at origin
        sk.constraints = vec![Horizontal(0), Diameter(1, 20.0), Tangent(0, 1)];
        solve(&mut sk, &[]).unwrap();
        let Entity::Circle { radius, .. } = sk.entities[1] else { panic!() };
        assert!(close(radius, 10.0));
        let (a, b) = (sk.points[1], sk.points[2]);
        assert!(close(a.y, b.y), "horizontal");
        assert!(close(a.y.abs(), 10.0), "line sits on the circle");
    }

    #[test]
    fn conflicting_constraints_fail_and_leave_sketch_untouched() {
        let mut sk = rough_rect();
        sk.constraints = vec![Horizontal(0), Parallel(0, 1), Perpendicular(0, 1)];
        let before = sk.clone();
        assert_eq!(solve(&mut sk, &[]), Err(SolveError::NoConvergence));
        assert_eq!(sk, before);
    }

    #[test]
    fn horizontal_and_vertical_on_one_line_is_degenerate() {
        let mut sk = rough_rect();
        sk.constraints = vec![Horizontal(0), Vertical(0)];
        let before = sk.clone();
        assert_eq!(solve(&mut sk, &[]), Err(SolveError::Degenerate));
        assert_eq!(sk, before);
    }

    #[test]
    fn fix_holds_point_and_reduces_dof() {
        let mut sk = rough_rect();
        sk.constraints = vec![Fix(2), Horizontal(0)];
        let p2 = sk.points[2];
        let s = solve(&mut sk, &[]).unwrap();
        assert_eq!(sk.points[2], p2);
        assert_eq!(s.dof, 3, "points 1,3 free (4) minus horizontal (1)");
    }

    #[test]
    fn drag_with_pin_keeps_constraints() {
        let mut sk = rough_rect();
        sk.constraints = vec![Horizontal(0), Vertical(1)];
        solve(&mut sk, &[]).unwrap();
        // Drag point 2 somewhere new; lines 0/1 must stay H/V.
        sk.points[2] = DVec2::new(70.0, 40.0);
        solve(&mut sk, &[2]).unwrap();
        assert_eq!(sk.points[2], DVec2::new(70.0, 40.0));
        assert!(close(sk.points[1].x, 70.0) && close(sk.points[1].y, 0.0));
    }

    #[test]
    fn drag_slides_along_remaining_freedom() {
        // Rectangle with height fixed at 30 but width free.
        let mut sk = rough_rect();
        add_constraints(&mut sk, &[Horizontal(0), Vertical(1), Horizontal(2), Vertical(3), Length(1, 30.0)]).unwrap();
        // Cursor is slightly off the only reachable height (no grid snap).
        drag(&mut sk, 2, DVec2::new(70.0, 29.83)).unwrap();
        let p2 = sk.points[2];
        assert!(close(p2.x, 70.0), "follows the cursor horizontally: {p2}");
        assert!(close(p2.y, 30.0), "height dimension still holds: {p2}");
        assert!(close(sk.points[1].x, 70.0) && close(sk.points[1].y, 0.0));
    }

    #[test]
    fn drag_of_fully_constrained_point_does_not_move_it() {
        let mut sk = rough_rect();
        add_constraints(&mut sk, &[Horizontal(0), Vertical(1), Horizontal(2), Vertical(3)]).unwrap();
        add_constraints(&mut sk, &[Length(0, 50.0), Length(1, 30.0)]).unwrap();
        let before = sk.points.clone();
        drag(&mut sk, 2, DVec2::new(80.0, 45.0)).unwrap();
        for (a, b) in before.iter().zip(&sk.points) {
            assert!(a.distance(*b) < 1e-6);
        }
    }

    #[test]
    fn freehand_rectangle_with_inferred_constraints_is_clean() {
        // Draw a slightly wonky rectangle; every edge is within the snap angle.
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        let mut ed = Editor::default();
        ed.set_tool(Tool::Line);
        let clicks = [
            Anchor::Point(0),
            free(50.0, 1.5),
            free(49.0, 30.0),
            free(1.2, 31.0),
            Anchor::Point(0),
        ];
        for at in clicks {
            for c in ed.click(&mut sk, at) {
                add_constraints(&mut sk, &[c]).expect("inferred constraints never conflict here");
            }
        }
        assert_eq!(sk.constraints, vec![Horizontal(0), Vertical(1), Horizontal(2), Vertical(3)]);
        let s = solve(&mut sk, &[]).unwrap();
        assert_eq!(s.dof, 2, "only width and height left, like a drawn rectangle in Fusion");
        assert!(close(sk.points[3].x, 0.0) && close(sk.points[1].y, 0.0));
    }

    #[test]
    fn dragging_a_line_slides_it_sideways_and_stretches_what_is_joined_to_it() {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        let mut ed = Editor::default();
        ed.set_tool(Tool::Rectangle);
        ed.click(&mut sk, free(10.0, 10.0));
        for c in ed.click(&mut sk, free(60.0, 40.0)) {
            add_constraints(&mut sk, &[c]).unwrap();
        }
        // The top line: the one with both ends at y = 40.
        let top = (0..sk.entities.len())
            .find(|&e| matches!(sk.entities[e], Entity::Line { a, b } if close(sk.points[a].y, 40.0) && close(sk.points[b].y, 40.0)))
            .unwrap();
        let Entity::Line { a, b } = sk.entities[top] else { unreachable!() };
        let from = [sk.points[a], sk.points[b]];
        // Slide it 5 up (a sideways pull is ignored by the caller; here only the shift matters).
        drag_line(&mut sk, top, from, DVec2::new(0.0, 5.0)).unwrap();
        assert!(close(sk.points[a].y, 45.0) && close(sk.points[b].y, 45.0), "{:?}", sk.points);
        // Horizontal stays horizontal and the bottom edge hasn't moved: the sides got longer.
        assert!(close(sk.points[a].x, from[0].x) && close(sk.points[b].x, from[1].x));
        let bottom_y: Vec<f64> = sk.points.iter().map(|p| p.y).filter(|y| close(*y, 10.0)).collect();
        assert_eq!(bottom_y.len(), 2, "the two bottom corners stayed at y = 10");
        // A line pinned by a distance can't go: it stays where it is instead of failing.
        let mut tied = sk.clone();
        tied.constraints.push(Constraint::Fix(a));
        let before = tied.points.clone();
        let ends = [before[a], before[b]];
        drag_line(&mut tied, top, ends, DVec2::new(0.0, 20.0)).unwrap();
        assert!(tied.points.iter().zip(&before).all(|(x, y)| x.distance(*y) < 1e-6), "a fixed corner holds the line");
    }

    #[test]
    fn a_dragged_line_follows_the_cursor_both_ways_and_keeps_its_constraints() {
        // A lone horizontal line: it can go left as well as up.
        let mut sk = loose();
        add_constraints(&mut sk, &[Horizontal(0)]).unwrap();
        let from = [sk.points[1], sk.points[2]];
        drag_line(&mut sk, 0, from, DVec2::new(-7.0, 3.0)).unwrap();
        assert!((sk.points[1] - (from[0] + DVec2::new(-7.0, 3.0))).length() < 1e-6, "{:?}", sk.points[1]);
        assert!(close(sk.points[1].y, sk.points[2].y), "still horizontal");
        // Collinear lines: dragging one along keeps the other on its line.
        let mut c = loose();
        add_constraints(&mut c, &[Collinear(0, 1)]).unwrap();
        let from = [c.points[3], c.points[4]];
        let dir = (from[1] - from[0]).normalize();
        drag_line(&mut c, 1, from, dir * 4.0 + dir.perp() * 2.0).unwrap();
        let (a, b) = (c.points[1], c.points[2]);
        for q in [c.points[3], c.points[4]] {
            assert!((b - a).perp_dot(q - a).abs() / (b - a).length() < 1e-6, "collinear kept");
        }
        // A rectangle's top dragged left and up: it rises, and the rectangle goes
        // left with it (the sides stay vertical, the bottom stays horizontal).
        let mut r = Sketch::new("S".into(), OriginPlane::XY);
        let mut ed = Editor::default();
        ed.set_tool(Tool::Rectangle);
        ed.click(&mut r, free(10.0, 10.0));
        for k in ed.click(&mut r, free(60.0, 40.0)) {
            add_constraints(&mut r, &[k]).unwrap();
        }
        let top = (0..r.entities.len())
            .find(|&e| matches!(r.entities[e], Entity::Line { a, b } if close(r.points[a].y, 40.0) && close(r.points[b].y, 40.0)))
            .unwrap();
        let Entity::Line { a, b } = r.entities[top] else { unreachable!() };
        let from = [r.points[a], r.points[b]];
        drag_line(&mut r, top, from, DVec2::new(-5.0, 2.0)).unwrap();
        assert!(close(r.points[a].y, 42.0) && close(r.points[a].y, r.points[b].y));
        assert!(close(r.points[a].x, from[0].x - 5.0) && close(r.points[b].x, from[1].x - 5.0));
    }

    #[test]
    fn rectangle_tool_leaves_width_and_height_free() {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        let mut ed = Editor::default();
        ed.set_tool(Tool::Rectangle);
        ed.click(&mut sk, Anchor::Point(0));
        for c in ed.click(&mut sk, free(50.0, 30.0)) {
            add_constraints(&mut sk, &[c]).unwrap();
        }
        assert_eq!(solve(&mut sk, &[]).unwrap().dof, 2, "just width and height, like Fusion's rectangle");
        add_constraints(&mut sk, &[Length(0, 60.0), Length(1, 20.0)]).unwrap();
        // Point 1 is the corner opposite the origin (the second click).
        assert!(close(sk.points[1].x, 60.0) && close(sk.points[1].y, 20.0), "{}", sk.points[1]);
    }

    #[test]
    fn angle_constraint_sets_the_angle_between_two_lines() {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        let o = Sketch::ORIGIN;
        sk.points.push(DVec2::new(40.0, 0.0));
        sk.points.push(DVec2::new(30.0, 25.0));
        let (a, b) = (1, 2);
        sk.entities.push(Entity::Line { a: o, b: a });
        sk.entities.push(Entity::Line { a: o, b });
        add_constraints(&mut sk, &[Horizontal(0), Length(1, 30.0)]).unwrap();
        let now = sk.angle_between(0, 1).unwrap();
        add_constraints(&mut sk, &[Angle(0, 1, now)]).unwrap(); // adding at its current value moves nothing
        assert!(close(sk.angle_between(0, 1).unwrap(), now));
        // Change it to 60°, then to 135°.
        for target in [60.0, 135.0] {
            if let Constraint::Angle(_, _, d) = sk.constraints.last_mut().unwrap() {
                *d = target;
            }
            solve(&mut sk, &[]).unwrap();
            assert!(close(sk.angle_between(0, 1).unwrap(), target), "{target}: {}", sk.angle_between(0, 1).unwrap());
            assert!(close(sk.points[b].length(), 30.0), "length kept");
        }
        // With Perpendicular already there, an angle on the same lines is redundant.
        let mut sk2 = sk.clone();
        sk2.constraints.pop();
        add_constraints(&mut sk2, &[Perpendicular(0, 1)]).unwrap();
        assert_eq!(add_constraints(&mut sk2, &[Angle(0, 1, 90.0)]), Err(SolveError::Redundant));
    }

    #[test]
    fn distance_between_points_of_two_lines() {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        let mut ed = Editor::default();
        ed.set_tool(Tool::Line);
        for (x, y) in [(0.0, 0.0), (10.0, 0.0), (10.0, 0.0), (20.0, 8.0), (30.0, 8.0)] {
            ed.click(&mut sk, free(x, y));
        }
        // Points 1 and 3 are on different lines; hold them 25 apart.
        add_constraints(&mut sk, &[Horizontal(0), Horizontal(1), Distance(1, 3, 25.0)]).unwrap();
        assert!(close(sk.points[1].distance(sk.points[3]), 25.0));
        if let Some(Constraint::Distance(_, _, d)) = sk.constraints.last_mut() {
            *d = 12.0;
        }
        solve(&mut sk, &[]).unwrap();
        assert!(close(sk.points[1].distance(sk.points[3]), 12.0));
    }

    #[test]
    fn tangent_circles_settle_into_contact() {
        // A right angle from the (fixed) origin, ends fixed: the lines can't
        // move, so the circle has to.
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        let mut ed = Editor::default();
        ed.set_tool(Tool::Line);
        ed.click(&mut sk, Anchor::Point(Sketch::ORIGIN));
        ed.click(&mut sk, free(30.0, 0.0));
        ed.cancel();
        ed.click(&mut sk, Anchor::Point(Sketch::ORIGIN));
        ed.click(&mut sk, free(0.0, 30.0));
        ed.cancel();
        add_constraints(&mut sk, &[Fix(1), Fix(2)]).unwrap();
        ed.set_tool(Tool::Circle2Tangent);
        ed.pick_tangent_line(&sk, 0);
        ed.pick_tangent_line(&sk, 1);
        for c in ed.tangent_click(&mut sk, DVec2::new(7.0, 4.0)).unwrap() {
            add_constraints(&mut sk, &[c]).unwrap();
        }
        let Entity::Circle { center, .. } = sk.entities[2] else { panic!() };
        let (c, r) = (sk.points[center], circle_radius(&sk, 2));
        assert!(close(c.x, r) && close(c.y, r), "touches both lines: centre {c}, r {r}");
        // 3-tangent: add a slanted third line closing a triangle.
        ed.set_tool(Tool::Line);
        ed.click(&mut sk, Anchor::Point(1));
        ed.click(&mut sk, Anchor::Point(2));
        ed.set_tool(Tool::Circle3Tangent);
        for l in [0, 1, 3] {
            ed.pick_tangent_line(&sk, l);
        }
        for c in ed.tangent_click(&mut sk, DVec2::new(5.0, 5.0)).unwrap() {
            add_constraints(&mut sk, &[c]).unwrap();
        }
        let r = circle_radius(&sk, 4);
        let inr = 30.0 * 30.0 / (30.0 + 30.0 + 30.0 * 2f64.sqrt()); // area·2 / perimeter
        assert!(close(r, inr), "{r} vs {inr}");
    }

    fn circle_radius(sk: &Sketch, e: usize) -> f64 {
        match sk.entities[e] {
            Entity::Circle { radius, .. } => radius,
            _ => panic!(),
        }
    }

    #[test]
    fn a_point_tied_to_a_line_slides_along_it() {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        let mut ed = Editor::default();
        ed.set_tool(Tool::Line);
        ed.click(&mut sk, Anchor::Point(Sketch::ORIGIN));
        ed.click(&mut sk, free(40.0, 0.0));
        ed.cancel();
        add_constraints(&mut sk, &[Horizontal(0), Fix(1)]).unwrap();
        ed.click(&mut sk, Anchor::OnLine(0, DVec2::new(15.0, 0.0)));
        for c in ed.click(&mut sk, free(15.0, 20.0)) {
            add_constraints(&mut sk, &[c]).unwrap();
        }
        let p = 2;
        // Drag the tied point up and sideways: it stays on the base line.
        drag(&mut sk, p, DVec2::new(25.0, 7.0)).unwrap();
        assert!(sk.points[p].y.abs() < 1e-9, "still on the line: {}", sk.points[p]);
        assert!(close(sk.points[p].x, 25.0), "slid along it: {}", sk.points[p]);
    }

    fn make(sk: &mut Sketch, tool: Tool, clicks: &[Anchor]) {
        let mut ed = Editor::default();
        ed.set_tool(tool);
        for &at in clicks {
            for c in ed.click(sk, at) {
                add_constraints(sk, &[c]).unwrap();
            }
        }
    }

    #[test]
    fn three_point_rectangle_is_square_at_any_angle() {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        // Side from (0,0) to (30,10), other side reaching (25,25).
        make(&mut sk, Tool::Rectangle3Point, &[free(0.0, 0.0), free(30.0, 10.0), free(25.0, 25.0)]);
        assert_eq!(sk.entities.len(), 4);
        assert_eq!(solve(&mut sk, &[]).unwrap().dof, 5, "position, angle, width, height");
        let corners = |sk: &Sketch| -> Vec<DVec2> { (1..5).map(|k| sk.points[k]).collect() };
        let p = corners(&sk);
        assert!(close((p[1] - p[0]).dot(p[2] - p[1]), 0.0), "square corner");
        assert!(close((p[2] - p[1]).length(), (p[3] - p[0]).length()));
        // Turn it by dragging a corner: it stays a rectangle.
        drag(&mut sk, 2, DVec2::new(28.0, 20.0)).unwrap();
        let p = corners(&sk);
        assert!(close((p[1] - p[0]).dot(p[2] - p[1]), 0.0));
        assert!(close((p[2] - p[1]).dot(p[3] - p[2]), 0.0));
    }

    #[test]
    fn center_rectangle_stays_centred() {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        make(&mut sk, Tool::RectangleCenter, &[free(10.0, 10.0), free(25.0, 18.0)]);
        assert_eq!(sk.entities.len(), 7, "the centre point, four sides and two construction diagonals");
        assert_eq!((0..7).filter(|&e| sk.is_construction(e)).count(), 2);
        assert_eq!(solve(&mut sk, &[]).unwrap().dof, 4, "centre position, width, height");
        // Change the width: the centre is still in the middle.
        add_constraints(&mut sk, &[Length(1, 40.0)]).unwrap();
        let c = sk.points[1];
        let corners: Vec<DVec2> = (2..6).map(|k| sk.points[k]).collect();
        assert!(close(c.x, (corners[0].x + corners[2].x) / 2.0) && close(c.y, (corners[0].y + corners[2].y) / 2.0));
        assert!(close((corners[1] - corners[0]).length(), 40.0));
        // Deleting a side keeps the centre point (it's a point of its own).
        sk.delete(&[crate::sketch::Sel::Entity(1)]);
        assert!(sk.entities.iter().any(|e| matches!(e, Entity::Point { .. })));
    }

    #[test]
    fn circle_edge_on_a_line_where_it_touches_is_tangent() {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        make(&mut sk, Tool::Line, &[Anchor::Point(Sketch::ORIGIN), free(40.0, 0.0)]); // gets Horizontal
        add_constraints(&mut sk, &[Fix(1)]).unwrap();
        // Centre above the line, edge clicked at the foot of the perpendicular.
        let foot = sk.line_foot(0, DVec2::new(12.0, 7.0)).unwrap();
        let mut ed = Editor::default();
        ed.set_tool(Tool::Circle);
        ed.click(&mut sk, free(12.0, 7.0));
        let cs = ed.click(&mut sk, Anchor::OnLine(0, foot));
        assert_eq!(cs, vec![Constraint::Tangent(0, 1)], "tangent, not a point on the line");
        for c in cs {
            add_constraints(&mut sk, &[c]).unwrap();
        }
        // Drag the centre up: the circle grows to keep touching.
        assert_eq!(sk.points.len(), 3, "no extra point at the touch spot");
        drag(&mut sk, 2, DVec2::new(12.0, 11.0)).unwrap();
        let Entity::Circle { center, radius } = sk.entities[1] else { panic!() };
        assert!(close(sk.points[center].y, radius));
    }

    #[test]
    fn dragging_a_rectangle_corner_leaves_the_opposite_corner() {
        let cases: [(Tool, Vec<Anchor>); 3] = [
            (Tool::Rectangle, vec![free(10.0, 10.0), free(40.0, 30.0)]),
            (Tool::Rectangle3Point, vec![free(10.0, 10.0), free(40.0, 10.0), free(40.0, 30.0)]),
            (Tool::RectangleCenter, vec![free(25.0, 20.0), free(40.0, 30.0)]),
        ];
        for (tool, clicks) in cases {
            let mut sk = Sketch::new("S".into(), OriginPlane::XY);
            make(&mut sk, tool, &clicks);
            // The four corners in order around the rectangle.
            let corners: Vec<PointId> = sk
                .entities
                .iter()
                .filter_map(|e| match *e {
                    Entity::Line { a, .. } => Some(a),
                    _ => None,
                })
                .collect();
            let (grab, opposite) = (corners[2], corners[0]);
            let before = sk.points[opposite];
            let to = sk.points[grab] + DVec2::new(6.0, 4.0);
            drag(&mut sk, grab, to).unwrap();
            assert!((sk.points[grab] - to).length() < 1e-6, "{tool:?}: the grabbed corner follows");
            assert!((sk.points[opposite] - before).length() < 1e-4, "{tool:?}: opposite moved {}", (sk.points[opposite] - before).length());
            let p: Vec<DVec2> = corners.iter().map(|&c| sk.points[c]).collect();
            assert!(close((p[1] - p[0]).dot(p[2] - p[1]), 0.0), "{tool:?}: still a rectangle");
        }
    }

    #[test]
    fn a_point_on_an_axis_stays_on_it_when_dragged() {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        make(&mut sk, Tool::Line, &[Anchor::OnAxis(0, DVec2::new(12.0, 0.0)), free(30.0, 20.0)]);
        drag(&mut sk, 1, DVec2::new(18.0, 6.0)).unwrap();
        assert!(sk.points[1].y.abs() < 1e-9, "still on the X axis: {}", sk.points[1]);
        assert!(close(sk.points[1].x, 18.0), "slid along it");
        // The other end, untied, moves freely.
        drag(&mut sk, 2, DVec2::new(5.0, 25.0)).unwrap();
        assert!(close(sk.points[2].x, 5.0) && close(sk.points[2].y, 25.0));
        assert!(sk.points[1].y.abs() < 1e-9);
    }

    #[test]
    fn filleted_rectangle_keeps_its_dimensions_and_rounding() {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        make(&mut sk, Tool::Rectangle, &[Anchor::Point(Sketch::ORIGIN), free(40.0, 30.0)]);
        add_constraints(&mut sk, &[Length(0, 40.0), Length(1, 30.0)]).unwrap();
        let corner = (0..sk.points.len()).find(|&p| sk.points[p] == DVec2::new(40.0, 30.0)).unwrap();
        let cs = sk.fillet_corner(corner, 5.0).unwrap();
        add_constraints(&mut sk, &cs).unwrap();
        // The sharp corner stays put, and the side that ran into it keeps its
        // 30 — now measured to the sharp corner.
        assert!((sk.points[corner] - DVec2::new(40.0, 30.0)).length() < 1e-9);
        assert!(sk.constraints.iter().any(|c| matches!(c, Constraint::Distance(_, p, d) if *p == corner && close(*d, 30.0))));
        // Make it 50 wide and 35 tall, r = 8: everything follows.
        for c in &mut sk.constraints {
            match c {
                Constraint::Length(0, d) => *d = 50.0,
                Constraint::Distance(_, p, d) if *p == corner => *d = 35.0,
                Constraint::Radius(_, r) => *r = 8.0,
                _ => {}
            }
        }
        solve(&mut sk, &[]).unwrap();
        assert!(close(sk.points[corner].x, 50.0) && close(sk.points[corner].y, 35.0), "{}", sk.points[corner]);
        let arc = sk.entities.iter().position(|e| matches!(e, Entity::Arc { .. })).unwrap();
        let (c, r, _, sweep) = sk.arc_geometry(arc).unwrap();
        assert!(close(r, 8.0) && close(sweep, std::f64::consts::FRAC_PI_2));
        assert!(close(c.x, 42.0) && close(c.y, 27.0), "centre 8 in from both sides: {c}");
    }

    #[test]
    fn dragging_a_missing_point_is_an_error_not_a_crash() {
        let mut sk = rough_rect();
        let before = sk.clone();
        assert_eq!(drag(&mut sk, 99, DVec2::new(1.0, 1.0)), Err(SolveError::Invalid));
        assert_eq!(sk, before);
    }

    #[test]
    fn redundant_but_consistent_constraints_still_solve() {
        let mut sk = rough_rect();
        sk.constraints = vec![Horizontal(0), Horizontal(2), Parallel(0, 2), Vertical(1), Vertical(3)];
        assert!(solve(&mut sk, &[]).is_ok());
    }

    #[test]
    fn redundant_dimension_is_refused() {
        let mut sk = rough_rect();
        add_constraints(&mut sk, &[Horizontal(0), Vertical(1), Horizontal(2), Vertical(3)]).unwrap();
        add_constraints(&mut sk, &[Length(0, 50.0)]).unwrap();
        let s = add_constraints(&mut sk, &[Length(1, 30.0)]).unwrap();
        assert_eq!(s.dof, 0);
        let before = sk.clone();
        // Opposite side is already 50 because of the H/V constraints.
        assert_eq!(add_constraints(&mut sk, &[Length(2, 50.0)]), Err(SolveError::Redundant));
        assert_eq!(sk, before);
    }

    #[test]
    fn partially_redundant_batch_is_refused() {
        let mut sk = rough_rect();
        add_constraints(&mut sk, &[Horizontal(0)]).unwrap();
        add_constraints(&mut sk, &[Parallel(0, 2)]).unwrap();
        // Line 2 is already horizontal via parallel, so H on (0, 2) adds nothing new for 2.
        assert_eq!(add_constraints(&mut sk, &[Horizontal(2)]), Err(SolveError::Redundant));
    }

    #[test]
    fn fix_that_removes_last_freedom_is_allowed() {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        let mut ed = Editor::default();
        ed.set_tool(Tool::Line);
        ed.click(&mut sk, Anchor::Point(0));
        ed.click(&mut sk, free(20.0, 0.0)); // line 0, point 1
        add_constraints(&mut sk, &[Horizontal(0)]).unwrap(); // point 1 has 1 dof left
        let s = add_constraints(&mut sk, &[Fix(1)]).unwrap();
        assert_eq!(s.dof, 0);
        assert_eq!(add_constraints(&mut sk, &[Length(0, 20.0)]), Err(SolveError::Redundant));
    }

    #[test]
    fn linear_solver_and_rank() {
        let y = solve_linear(vec![2.0, 1.0, 1.0, 3.0], vec![3.0, 5.0]).unwrap();
        assert!(close(y[0], 0.8) && close(y[1], 1.4));
        assert_eq!(rank(vec![1.0, 2.0, 2.0, 4.0], 2, 2), 1);
        assert_eq!(rank(vec![1.0, 0.0, 0.0, 1.0, 1.0, 1.0], 3, 2), 2);
    }

    #[test]
    fn over_constraining_dimension_becomes_driven_and_follows_the_geometry() {
        let mut sk = rough_rect();
        add_constraints(&mut sk, &[Horizontal(0), Vertical(1), Horizontal(2), Vertical(3), Length(0, 50.0), Length(1, 30.0)]).unwrap();
        // (As in the app, the new dimension holds what the geometry measures.)
        let len = sk.line_length(2).unwrap();
        let (_, driven) = add_dimension(&mut sk, Length(2, len)).unwrap();
        assert!(driven);
        // Length(2) is already fixed by the rectangle (opposite side of Length(0)).
        let ci = sk.constraints.len() - 1;
        assert!(sk.is_driven(ci), "{:?}", sk.constraints);
        assert!(close(sk.measure(&sk.constraints[ci]).unwrap(), 50.0));
        // It doesn't constrain: dimensions still solve and the driven one is stale-proof.
        sk.constraints[4] = Length(0, 60.0);
        let s = solve(&mut sk, &[]).unwrap();
        assert!(close(sk.measure(&sk.constraints[ci]).unwrap(), 60.0), "{s:?}");
        // A driven dimension can't be made driving while something else holds it...
        assert_eq!(make_driving(&mut sk, ci), Err(SolveError::Redundant));
        assert!(sk.is_driven(ci));
        // ...but is fine once the other one goes.
        sk.remove_constraint(4);
        let ci = ci - 1;
        assert!(sk.is_driven(ci), "flag follows its constraint when one before it is removed");
        make_driving(&mut sk, ci).unwrap();
        assert!(!sk.is_driven(ci));
        make_driven(&mut sk, ci);
        assert!(sk.is_driven(ci));
    }

    #[test]
    fn driven_flags_survive_serialisation_and_old_files() {
        let mut sk = rough_rect();
        sk.constraints.push(Length(0, 5.0));
        let old: Sketch = serde_json::from_str(&serde_json::to_string(&sk).unwrap()).unwrap();
        assert!(!old.is_driven(0));
        sk.set_driven(0, true);
        let back: Sketch = serde_json::from_str(&serde_json::to_string(&sk).unwrap()).unwrap();
        assert!(back.is_driven(0));
    }
}


#[cfg(test)]
mod chamfer_edit_tests {
    use super::*;
    use crate::sketch::{Anchor, Editor, Entity, OriginPlane, Tool};

    /// A 50×30 rectangle drawn with the rectangle tool (so it only has the
    /// constraints the tool adds), then its (50,30) corner chamfered 2 / 5.
    fn chamfered() -> (Sketch, usize, usize) {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        let mut ed = Editor::default();
        ed.set_tool(Tool::Rectangle);
        let mut cs = ed.click(&mut sk, Anchor::Free(DVec2::new(0.0, 0.0)));
        cs.extend(ed.click(&mut sk, Anchor::Free(DVec2::new(50.0, 30.0))));
        for c in cs {
            let mut t = sk.clone();
            if add_constraints(&mut t, &[c]).is_ok() {
                sk = t;
            }
        }
        let corner = (0..sk.points.len()).find(|&p| sk.points[p].distance(DVec2::new(50.0, 30.0)) < 1e-9).unwrap();
        let cs = sk.chamfer_corner2(corner, 2.0, 5.0, None).unwrap();
        add_constraints(&mut sk, &cs).unwrap();
        let ci = sk.constraints.iter().position(|c| matches!(c, Constraint::Distance(_, p, d) if *p == corner && (*d - 2.0).abs() < 1e-9)).unwrap();
        (sk, corner, ci)
    }

    #[test]
    fn changing_a_chamfer_distance_only_moves_the_chamfer_end() {
        let (mut sk, corner, ci) = chamfered();
        let before: Vec<DVec2> = sk.points.clone();
        crate::solver::set_dimension_value(&mut sk, ci, 8.0).unwrap();
        // Nothing but the chamfer's own end point moved: the rectangle is the same size.
        let moved: Vec<usize> = (0..before.len()).filter(|&p| before[p].distance(sk.points[p]) > 1e-6).collect();
        let Constraint::Distance(n, _, _) = sk.constraints[ci] else { unreachable!() };
        assert_eq!(moved, vec![n], "moved points: {moved:?}");
        assert!(sk.points[corner].distance(DVec2::new(50.0, 30.0)) < 1e-9, "the virtual sharp stays");
        assert!((sk.points[n].distance(sk.points[corner]) - 8.0).abs() < 1e-6);
        let _ = Entity::Point { p: 0 };
    }

    #[test]
    fn without_the_pin_the_corner_would_slide() {
        // The old behaviour (plain solve), to keep the test above honest.
        let (mut sk, corner, ci) = chamfered();
        let before = sk.points.clone();
        set_value(&mut sk.constraints[ci], 8.0);
        solve(&mut sk, &[]).unwrap();
        let moved: Vec<usize> = (0..before.len()).filter(|&p| before[p].distance(sk.points[p]) > 1e-6).collect();
        eprintln!("moved without pin: {moved:?}, corner {corner}");
        assert!(moved.len() > 1, "the rectangle itself changed: {moved:?}");
    }
}
