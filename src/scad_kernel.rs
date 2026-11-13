//! An OpenSCAD file's shapes (`scad::Geo`) made into exact OpenCASCADE
//! solids: cubes, spheres and cylinders as true primitives (a cylinder with
//! a few `$fn` sides as that prism), 2-D outlines as faces, extrudes and
//! revolves as sweeps, booleans by the kernel, hulls by their points.

use crate::scad::{self, Geo, Move};
use glam::{DMat3, DMat4, DVec2, DVec3};
use glam24::DVec3 as KVec;
use opencascade::primitives::{Edge, Face, Shape, Wire};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;

fn kv(p: DVec3) -> KVec {
    KVec::new(p.x, p.y, p.z)
}

/// A file's parts as solids: (name, colour, the solid) per part, plus what
/// was left out and why. A part that couldn't be made is an error naming it.
pub fn build(out: &scad::Output) -> Result<Vec<(String, Option<[f32; 4]>, Shape)>, String> {
    let mut made = Vec::new();
    for p in &out.parts {
        let shape = catch_unwind(AssertUnwindSafe(|| solid(&p.geo)))
            .map_err(|_| format!("{}: the geometry kernel failed on it", p.name))?
            .map_err(|e| format!("{}: {e}", p.name))?;
        match shape {
            Some(s) if !s.solids().is_empty() => made.push((p.name.clone(), p.color, s)),
            Some(_) if p.geo.is_2d() => return Err(format!("{} is flat (2-D): extrude it (linear_extrude) to make a solid", p.name)),
            _ => {}
        }
    }
    if made.is_empty() {
        return Err("it draws nothing solid".into());
    }
    Ok(made)
}

/// `g` as a shape (2-D ones as faces on the XY plane); None when it's empty.
fn solid(g: &Geo) -> Result<Option<Shape>, String> {
    Ok(Some(match g {
        Geo::Cube { size, center } => {
            let lo = if *center { -*size / 2.0 } else { DVec3::ZERO };
            Shape::box_from_corners(kv(lo), kv(lo + *size))
        }
        Geo::Sphere { r } => Shape::sphere(*r).build(),
        Geo::Cylinder { h, r1, r2, center, sides } => {
            let z0 = if *center { -h / 2.0 } else { 0.0 };
            match sides {
                Some(_) => return hull(&[g.clone()]).map(Some),
                None if (r1 - r2).abs() < 1e-12 => Shape::cylinder(kv(DVec3::Z * z0), *r1, kv(DVec3::Z), *h),
                None => Shape::cone().at(kv(DVec3::Z * z0)).bottom_radius(*r1).top_radius(*r2).height(*h).build(),
            }
        }
        Geo::Polyhedron { points, faces } => {
            // OpenSCAD lists a face's points clockwise seen from outside.
            let tris: Vec<[DVec3; 3]> = faces.iter().flat_map(|f| (1..f.len().saturating_sub(1)).map(move |k| [points[f[0]], points[f[k + 1]], points[f[k]]])).collect();
            mesh_solid(&tris).ok_or("its polyhedron isn't closed (every edge must join two faces)")?
        }
        Geo::Square { size, center } => {
            let lo = if *center { -*size / 2.0 } else { DVec2::ZERO };
            polygon_face(&[lo, lo + DVec2::new(size.x, 0.0), lo + *size, lo + DVec2::new(0.0, size.y)])?
        }
        Geo::Circle { r, sides } => match sides {
            Some(n) => polygon_face(&(0..*n).map(|k| {
                let a = std::f64::consts::TAU * k as f64 / *n as f64;
                DVec2::new(r * a.cos(), r * a.sin())
            }).collect::<Vec<_>>())?,
            None => Face::from_wire(&Wire::from_edges([&Edge::circle(kv(DVec3::ZERO), kv(DVec3::Z), *r)])).into(),
        },
        Geo::Polygon { points, paths } => {
            let paths: Vec<Vec<usize>> = if paths.is_empty() { vec![(0..points.len()).collect()] } else { paths.clone() };
            // The first path is the outline; the rest are holes in it.
            let mut out: Option<Shape> = None;
            for (k, p) in paths.iter().enumerate() {
                let face = polygon_face(&p.iter().map(|&i| points[i]).collect::<Vec<_>>())?;
                out = Some(match out {
                    None => face,
                    Some(o) if k > 0 => o.try_subtract(&face).ok_or("its paths overlap in a way the kernel can't cut")?,
                    Some(o) => o,
                });
            }
            return Ok(out);
        }
        Geo::Union(kids) => {
            let mut out: Option<Shape> = None;
            for k in kids {
                if let Some(s) = solid(k)? {
                    out = Some(match out {
                        None => s,
                        Some(o) => o.try_union(&s).ok_or("the kernel couldn't join these shapes")?,
                    });
                }
            }
            return Ok(out);
        }
        Geo::Difference(kids) => {
            let Some(mut out) = solid(&kids[0])? else { return Ok(None) };
            for k in &kids[1..] {
                if let Some(s) = solid(k)? {
                    out = out.try_subtract(&s).ok_or("the kernel couldn't cut these shapes")?;
                }
            }
            out
        }
        Geo::Intersection(kids) => {
            let mut out: Option<Shape> = None;
            for k in kids {
                let Some(s) = solid(k)? else { return Ok(None) };
                out = Some(match out {
                    None => s,
                    Some(o) => o.try_intersect(&s).ok_or("the kernel couldn't intersect these shapes")?,
                });
            }
            return Ok(out);
        }
        Geo::Hull(kids) => hull(kids)?,
        Geo::Move(m, child) => {
            let Some(s) = solid(child)? else { return Ok(None) };
            moved(s, m)?
        }
        Geo::Color(_, child) => return solid(child),
        Geo::LinearExtrude { height, center, child } => {
            let Some(flat) = solid(child)? else { return Ok(None) };
            let mut out: Option<Shape> = None;
            for f in flat.faces() {
                let s: Shape = f.extrude(kv(DVec3::Z * *height)).into();
                out = Some(match out {
                    None => s,
                    Some(o) => o.try_union(&s).ok_or("the kernel couldn't join the extruded outline")?,
                });
            }
            let Some(out) = out else { return Ok(None) };
            if *center { out.translated(kv(DVec3::Z * (-height / 2.0))) } else { out }
        }
        Geo::RotateExtrude { angle, child } => {
            // The outline's X is the radius and its Y the height: stood up on XZ, then turned about Z.
            let Some(flat) = solid(child)? else { return Ok(None) };
            let upright = flat.rotated_about(kv(DVec3::ZERO), kv(DVec3::X), std::f64::consts::FRAC_PI_2);
            let mut out: Option<Shape> = None;
            for f in upright.faces() {
                let s = f.try_revolve(kv(DVec3::ZERO), kv(DVec3::Z), angle.to_radians()).ok_or("the kernel couldn't revolve it (does the outline cross the axis?)")?;
                out = Some(match out {
                    None => s,
                    Some(o) => o.try_union(&s).ok_or("the kernel couldn't join the revolved outline")?,
                });
            }
            return Ok(out);
        }
        // (Not drawn grown or shrunk yet: `scad` already said so.)
        Geo::Offset { child, .. } => return solid(child),
        Geo::Import(p) => crate::kernel::import_shape(p)?,
    }))
}

/// A face from a closed outline on the XY plane.
fn polygon_face(pts: &[DVec2]) -> Result<Shape, String> {
    if pts.len() < 3 {
        return Err("an outline needs at least 3 points".into());
    }
    let wire = Wire::from_ordered_points(pts.iter().map(|p| kv(p.extend(0.0)))).map_err(|_| "an outline couldn't be made")?;
    Ok(Face::from_wire(&wire).into())
}

/// `s` moved by `m`.
fn moved(s: Shape, m: &Move) -> Result<Shape, String> {
    Ok(match m {
        Move::Translate(v) => s.translated(kv(*v)),
        Move::Rotate(axis, deg) => s.rotated_about(kv(DVec3::ZERO), kv(axis.normalize_or(DVec3::Z)), deg.to_radians()),
        Move::Mirror(n) => s.mirrored_in_plane(kv(DVec3::ZERO), kv(n.normalize_or(DVec3::X))),
        Move::Scale(f) => scaled(s, *f)?,
        Move::Matrix(mat) => {
            // A rotation, scales along the axes and a move (no shear).
            let m3 = DMat3::from_mat4(*mat);
            let mut scale = DVec3::new(m3.x_axis.length(), m3.y_axis.length(), m3.z_axis.length());
            if scale.min_element() < 1e-12 {
                return Err("multmatrix flattens it".into());
            }
            let mut rot = DMat3::from_cols(m3.x_axis / scale.x, m3.y_axis / scale.y, m3.z_axis / scale.z);
            if !(rot.transpose() * rot).abs_diff_eq(DMat3::IDENTITY, 1e-6) {
                return Err("multmatrix with a slant (shear) isn't supported".into());
            }
            if rot.determinant() < 0.0 {
                scale.x = -scale.x;
                rot = DMat3::from_cols(-rot.x_axis, rot.y_axis, rot.z_axis);
            }
            let mut s = scaled(s, scale)?;
            let q = glam::DQuat::from_mat3(&rot);
            let (axis, angle) = q.to_axis_angle();
            if angle.abs() > 1e-12 {
                s = s.rotated_about(kv(DVec3::ZERO), kv(axis), angle);
            }
            let t = mat.w_axis.truncate();
            if t != DVec3::ZERO { s.translated(kv(t)) } else { s }
        }
    })
}

/// Scaled about the origin; a negative factor mirrors.
fn scaled(s: Shape, f: DVec3) -> Result<Shape, String> {
    let mut s = s;
    let mut f = f;
    for (k, axis) in [DVec3::X, DVec3::Y, DVec3::Z].into_iter().enumerate() {
        if f[k] < 0.0 {
            s = s.mirrored_in_plane(kv(DVec3::ZERO), kv(axis));
            f[k] = -f[k];
        }
    }
    if f.min_element() < 1e-12 {
        return Err("scale flattens it".into());
    }
    if f.abs_diff_eq(DVec3::ONE, 1e-12) {
        return Ok(s);
    }
    if (f.x - f.y).abs() < 1e-12 && (f.y - f.z).abs() < 1e-12 {
        return Ok(s.scaled(kv(DVec3::ZERO), f.x));
    }
    s.try_scaled_xyz(kv(DVec3::ZERO), kv(f)).ok_or_else(|| "the kernel couldn't stretch it".into())
}

/// The convex hull of `kids` (2-D: a face; 3-D: a solid with flat faces).
fn hull(kids: &[Geo]) -> Result<Shape, String> {
    let mut pts = Vec::new();
    for k in kids {
        match k.hull_points(DMat4::IDENTITY) {
            Some(p) => pts.extend(p),
            None => {
                // A boolean (or import): the corners of its surface.
                if let Some(s) = solid(k)? {
                    let m = s.mesh().map_err(|_| "the kernel couldn't measure a shape in hull()")?;
                    pts.extend(m.vertices.iter().map(|v| DVec3::new(v.x, v.y, v.z)));
                }
            }
        }
    }
    if kids.first().is_some_and(Geo::is_2d) || pts.iter().all(|p| p.z.abs() < 1e-9) {
        let outline = scad::hull_2d(&pts.iter().map(|p| p.truncate()).collect::<Vec<_>>());
        return polygon_face(&outline);
    }
    let tris = scad::hull_3d(&pts).ok_or("hull() of flat or too few points")?;
    mesh_solid(&tris.iter().map(|t| [pts[t[0]], pts[t[1]], pts[t[2]]]).collect::<Vec<_>>()).ok_or_else(|| "the kernel couldn't make the hull a solid".into())
}

/// A closed triangle mesh (counter-clockwise from outside) as a solid with
/// flat faces (triangles in one plane merged), through the STL reader.
fn mesh_solid(tris: &[[DVec3; 3]]) -> Option<Shape> {
    use std::io::Write;
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!("oxide360-scad-{}-{}.stl", std::process::id(), N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
    let mut text = String::with_capacity(tris.len() * 200);
    text.push_str("solid scad\n");
    for t in tris {
        let n = (t[1] - t[0]).cross(t[2] - t[0]).normalize_or_zero();
        text.push_str(&format!("facet normal {} {} {}\nouter loop\n", n.x, n.y, n.z));
        for v in t {
            text.push_str(&format!("vertex {:.12} {:.12} {:.12}\n", v.x, v.y, v.z));
        }
        text.push_str("endloop\nendfacet\n");
    }
    text.push_str("endsolid scad\n");
    let ok = std::fs::File::create(&path).and_then(|mut f| f.write_all(text.as_bytes())).is_ok();
    let s = ok.then(|| Shape::try_read_stl(&path)).flatten();
    let _ = std::fs::remove_file(&path);
    s.filter(|s| !s.solids().is_empty())
}

/// What running a script made, ready to keep in the design.
pub struct Made {
    /// All the parts' solids together (each becomes a body).
    pub brep: String,
    pub source: crate::model::ScadSource,
    /// `echo()` lines and what was left out.
    pub messages: Vec<String>,
}

/// Run script `text` (from the file at `path`, for its `include` / `import`)
/// with the parameter `values` typed, and make its parts.
pub fn make(text: &str, path: Option<&Path>, values: &[(String, String)]) -> Result<Made, String> {
    let overrides: Vec<(String, scad::Value)> = values.iter().map(|(k, v)| (k.clone(), scad::parse_value(v))).collect();
    let out = scad::run(text, path, &overrides)?;
    let parts = build(&out)?;
    let compound = opencascade::primitives::Compound::from_shapes(parts.iter().flat_map(|(_, _, s)| s.solids()));
    let brep = Shape::from(compound).to_brep_string();
    if brep.is_empty() {
        return Err("the geometry kernel couldn't store it".into());
    }
    // One name and colour per solid (a part in two pieces names both).
    let mut names = Vec::new();
    let mut colors = Vec::new();
    for (name, color, s) in &parts {
        let n = s.solids().len();
        for k in 0..n {
            names.push(if n > 1 { format!("{name} ({})", k + 1) } else { name.clone() });
            colors.push(*color);
        }
    }
    Ok(Made {
        brep,
        source: crate::model::ScadSource { text: crate::model::Blob::new(text.to_string()), values: values.to_vec(), parts: names, colors },
        messages: out.messages,
    })
}

/// Read an OpenSCAD file at `path` (its parameters set to `overrides`) and
/// make its parts.
#[cfg(test)]
pub fn read(path: &Path, overrides: &[(String, scad::Value)]) -> Result<(Vec<(String, Option<[f32; 4]>, Shape)>, Vec<String>), String> {
    let src = std::fs::read_to_string(path).map_err(|e| format!("couldn't read it ({e})"))?;
    let out = scad::run(&src, Some(path), overrides)?;
    let parts = build(&out)?;
    Ok((parts, out.messages))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn volume(s: &Shape) -> f64 {
        s.volume()
    }

    fn one(src: &str) -> Shape {
        let out = scad::run(src, None, &[]).unwrap();
        let mut parts = build(&out).unwrap();
        assert_eq!(parts.len(), 1, "{:?}", parts.iter().map(|p| &p.0).collect::<Vec<_>>());
        parts.remove(0).2
    }

    fn near(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6 * b.abs().max(1.0)
    }

    #[test]
    fn primitives_are_exact() {
        assert!(near(volume(&one("cube([10, 20, 30]);")), 6000.0));
        let v = volume(&one("$fn = 48; cylinder(h=10, r=5);"));
        assert!(near(v, std::f64::consts::PI * 25.0 * 10.0), "truly round: {v}");
        let v = volume(&one("cylinder(h=3, r1=2, r2=0);"));
        assert!(near(v, std::f64::consts::PI * 4.0 * 3.0 / 3.0), "{v}");
        let v = volume(&one("sphere(d=6);"));
        assert!(near(v, 4.0 / 3.0 * std::f64::consts::PI * 27.0), "{v}");
        // A hex nut: six flat sides. Area of a hexagon of circumradius r: 3√3/2 r².
        let v = volume(&one("cylinder(d=6.9, h=2.7, $fn=6);"));
        assert!(near(v, 1.5 * 3f64.sqrt() * 3.45f64.powi(2) * 2.7), "{v}");
    }

    #[test]
    fn booleans_moves_and_extrudes() {
        // A 10 box with a 4×4 hole through it, moved and turned (volume stays).
        let v = volume(&one("translate([50,0,0]) rotate([0,90,0]) difference() { cube(10, center=true); cube([4,4,20], center=true); }"));
        assert!(near(v, 1000.0 - 160.0), "{v}");
        // An L outline extruded: the two squares joined, 3 high.
        let v = volume(&one("linear_extrude(3) union() { square([10, 2]); square([2, 10]); }"));
        assert!(near(v, (20.0 + 20.0 - 4.0) * 3.0), "{v}");
        let v = volume(&one("linear_extrude(height=4, center=true) difference() { square(10, center=true); circle(r=2); }"));
        assert!(near(v, (100.0 - std::f64::consts::PI * 4.0) * 4.0), "{v}");
        // Intersection: the overlap only.
        let v = volume(&one("intersection() { cube(10); translate([5,5,5]) cube(10); }"));
        assert!(near(v, 125.0), "{v}");
        // Scaled unevenly, and mirrored.
        let v = volume(&one("mirror([1,0,0]) scale([2,1,0.5]) cube(10);"));
        assert!(near(v, 1000.0), "{v}");
        // A ring revolved: torus-like, a 2×2 square at radius 10 to 12.
        let v = volume(&one("rotate_extrude() translate([10,0]) square(2);"));
        assert!(near(v, std::f64::consts::PI * (144.0 - 100.0) * 2.0), "{v}");
    }

    #[test]
    fn made_parts_keep_names_colours_and_values() {
        let src = "n = 2;\nfor (i = [1:n]) color(\"red\") translate([i*20,0,0]) cube(5);\ncube(1);";
        let m = make(src, None, &[("n".into(), "3".into())]).unwrap();
        assert_eq!(m.source.parts, vec!["cube (1)", "cube (2)", "cube (3)", "cube"], "the for loop is one part in 3 pieces, then the cube: a name per body");
        assert_eq!(m.source.colors[0], Some([1.0, 0.0, 0.0, 1.0]));
        let shape = Shape::try_from_brep_string(&m.brep).unwrap();
        assert_eq!(shape.solids().len(), 4);
        assert_eq!(m.source.values, vec![("n".to_string(), "3".to_string())]);
    }

    #[test]
    fn hulls_become_solids() {
        // Two cubes' hull: one long box.
        let v = volume(&one("hull() { cube(2); translate([8,0,0]) cube(2); }"));
        assert!(near(v, 40.0), "{v}");
        // A stadium (two round ends): close to the exact area, as a many-sided polygon.
        let v = volume(&one("hull() for (s=[-1,1]) translate([s*5,0,0]) cylinder(r=2, h=1);"));
        let exact = 10.0 * 4.0 + std::f64::consts::PI * 4.0;
        assert!((v - exact).abs() / exact < 0.005, "{v} vs {exact}");
        // 2-D hull then extruded.
        let v = volume(&one("linear_extrude(1) hull() { square(1); translate([4,0]) square(1); }"));
        assert!(near(v, 5.0), "{v}");
        let p = one("polyhedron(points=[[0,0,0],[1,0,0],[0,1,0],[0,0,1]], faces=[[0,1,2],[0,3,1],[0,2,3],[1,3,2]]);");
        assert!(near(volume(&p), 1.0 / 6.0), "{}", volume(&p));
    }

    /// An OpenSCAD file given by SCAD_FILE, every part made (and timed).
    /// `SCAD_FILE=path cargo test scad_file -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn scad_file() {
        let path = std::env::var("SCAD_FILE").expect("SCAD_FILE");
        let t = std::time::Instant::now();
        let (parts, messages) = read(Path::new(&path), &[]).unwrap();
        for (name, color, s) in &parts {
            println!("{name}: {:?} volume {:.1} mm³, {} solid(s)", color, volume(s), s.solids().len());
        }
        println!("{messages:?}\n{:.1} s", t.elapsed().as_secs_f64());
    }
}
