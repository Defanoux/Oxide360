//! Appearances (Fusion's Appearance, A): how a body looks — colour, how
//! metallic and how glossy it is, and how see-through. Only the look: what a
//! body weighs is its Physical Material (`materials.rs`), chosen separately.
//! A body with an appearance takes its colour from it; one without keeps its
//! own colour and the plain shading. A face can have its own appearance over
//! its body's.

use crate::doc::BodyId;
use crate::model::Mesh;
use glam::{DVec3, Vec3};
use serde::{Deserialize, Serialize};

/// A group in the library, as Fusion's folders.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    Metal,
    Plastic,
    Paint,
    Rubber,
    Glass,
    Wood,
}

impl Category {
    pub const ALL: [Category; 6] = [Category::Metal, Category::Plastic, Category::Paint, Category::Rubber, Category::Glass, Category::Wood];

    pub fn name(self) -> &'static str {
        match self {
            Category::Metal => "Metal",
            Category::Plastic => "Plastic",
            Category::Paint => "Paint",
            Category::Rubber => "Rubber",
            Category::Glass => "Glass",
            Category::Wood => "Wood",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Appearance {
    /// Also how it's saved with a body.
    pub name: &'static str,
    pub category: Category,
    pub color: [u8; 3],
    /// 1 for metals (reflections tinted by the colour), 0 for everything else.
    pub metal: f32,
    /// 0 dull (rubber) … 1 mirror-like (chrome): how small and bright the highlights are.
    pub gloss: f32,
    /// 1 solid … near 0 clear glass.
    pub opacity: f32,
}

const fn a(name: &'static str, category: Category, color: [u8; 3], gloss: f32, opacity: f32) -> Appearance {
    let metal = if matches!(category, Category::Metal) { 1.0 } else { 0.0 };
    Appearance { name, category, color, metal, gloss, opacity }
}

use Category::*;

/// Every appearance, by category.
pub const LIBRARY: &[Appearance] = &[
    a("Steel - Satin", Metal, [168, 170, 174], 0.45, 1.0),
    a("Steel - Polished", Metal, [200, 202, 206], 0.85, 1.0),
    a("Stainless Steel - Brushed", Metal, [184, 186, 190], 0.55, 1.0),
    a("Aluminum - Satin", Metal, [198, 202, 208], 0.5, 1.0),
    a("Aluminum - Polished", Metal, [222, 226, 231], 0.9, 1.0),
    a("Aluminum - Anodized Black", Metal, [52, 54, 60], 0.55, 1.0),
    a("Aluminum - Anodized Red", Metal, [150, 30, 36], 0.55, 1.0),
    a("Aluminum - Anodized Blue", Metal, [36, 70, 150], 0.55, 1.0),
    a("Brass - Polished", Metal, [214, 174, 92], 0.85, 1.0),
    a("Copper - Polished", Metal, [206, 128, 88], 0.85, 1.0),
    a("Gold - Polished", Metal, [236, 192, 84], 0.9, 1.0),
    a("Chrome", Metal, [232, 234, 238], 1.0, 1.0),
    a("Titanium - Satin", Metal, [160, 158, 150], 0.45, 1.0),
    a("Iron - Cast", Metal, [96, 96, 100], 0.15, 1.0),
    a("Plastic - Glossy (White)", Plastic, [236, 236, 232], 0.8, 1.0),
    a("Plastic - Glossy (Black)", Plastic, [38, 38, 42], 0.8, 1.0),
    a("Plastic - Glossy (Red)", Plastic, [200, 36, 36], 0.8, 1.0),
    a("Plastic - Glossy (Blue)", Plastic, [40, 90, 200], 0.8, 1.0),
    a("Plastic - Glossy (Yellow)", Plastic, [240, 200, 40], 0.8, 1.0),
    a("Plastic - Glossy (Green)", Plastic, [50, 160, 70], 0.8, 1.0),
    a("Plastic - Matte (Grey)", Plastic, [128, 130, 134], 0.25, 1.0),
    a("Plastic - Matte (Black)", Plastic, [46, 46, 48], 0.25, 1.0),
    a("ABS (White)", Plastic, [228, 226, 218], 0.45, 1.0),
    a("Paint - Enamel Glossy (Red)", Paint, [190, 24, 28], 0.9, 1.0),
    a("Paint - Enamel Glossy (Blue)", Paint, [24, 64, 170], 0.9, 1.0),
    a("Paint - Enamel Glossy (Black)", Paint, [30, 30, 33], 0.9, 1.0),
    a("Paint - Enamel Glossy (White)", Paint, [240, 240, 238], 0.9, 1.0),
    a("Paint - Enamel Glossy (Yellow)", Paint, [246, 196, 20], 0.9, 1.0),
    a("Paint - Enamel Glossy (Orange)", Paint, [236, 110, 24], 0.9, 1.0),
    a("Powder Coat (Black)", Paint, [42, 42, 44], 0.35, 1.0),
    a("Rubber - Black", Rubber, [44, 44, 46], 0.08, 1.0),
    a("Rubber - Grey", Rubber, [96, 98, 100], 0.08, 1.0),
    a("Silicone (White)", Rubber, [232, 230, 224], 0.2, 1.0),
    a("Glass - Clear", Glass, [214, 232, 238], 1.0, 0.25),
    a("Glass - Smoked", Glass, [60, 64, 70], 1.0, 0.5),
    a("Glass - Blue", Glass, [110, 160, 220], 1.0, 0.35),
    a("Acrylic - Clear", Glass, [226, 236, 240], 0.9, 0.3),
    a("Oak", Wood, [188, 144, 94], 0.3, 1.0),
    a("Walnut", Wood, [108, 70, 44], 0.35, 1.0),
    a("Pine", Wood, [224, 190, 140], 0.25, 1.0),
    a("Maple", Wood, [230, 200, 158], 0.3, 1.0),
    a("Plywood", Wood, [210, 176, 126], 0.2, 1.0),
];

/// The appearance saved under `name` (None for a name this version doesn't know).
pub fn by_name(name: &str) -> Option<&'static Appearance> {
    LIBRARY.iter().find(|a| a.name == name)
}

/// The appearances in one category, in library order.
pub fn in_category(c: Category) -> impl Iterator<Item = &'static Appearance> {
    LIBRARY.iter().filter(move |a| a.category == c)
}

/// A face given its own appearance (Fusion's Apply To: Faces): where it was
/// clicked on which body. The face is found again every build (`face_under`),
/// so it follows the part as it changes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FaceAppearance {
    pub body: BodyId,
    pub point: [f64; 3],
    /// The surface's outward direction there.
    pub normal: [f64; 3],
    pub appearance: String,
}

/// The face (index into the body's faces, by `tri_face`) a stored face
/// appearance is on now: the nearest triangle facing about the same way. A
/// face that moved (an extrude made taller) is still found; nothing within a
/// quarter of the body's size means it's gone.
pub fn face_under(mesh: &Mesh, tri_face: &[u32], point: DVec3, normal: DVec3) -> Option<u32> {
    let (lo, hi) = mesh.positions.iter().fold((Vec3::MAX, Vec3::MIN), |(lo, hi), p| (lo.min(*p), hi.max(*p)));
    let reach = (hi - lo).length() as f64 * 0.25;
    let mut best: Option<(f64, u32)> = None;
    for (t, tri) in mesh.tris.iter().enumerate() {
        if mesh.normal(t).as_dvec3().dot(normal) < 0.7 {
            continue;
        }
        let [a, b, c] = tri.map(|i| mesh.positions[i as usize].as_dvec3());
        let d = closest_on_triangle(point, a, b, c).distance(point);
        if d <= reach && best.is_none_or(|(e, _)| d < e) {
            best = tri_face.get(t).map(|&f| (d, f));
        }
    }
    best.map(|(_, f)| f)
}

/// Give the face at (`point`, `normal`) on `body` an appearance (None: take
/// its own off). `same_face` tells whether a stored one is on that face now.
pub fn set_face(list: &mut Vec<FaceAppearance>, body: BodyId, point: DVec3, normal: DVec3, appearance: Option<&str>, same_face: impl Fn(&FaceAppearance) -> bool) {
    list.retain(|f| f.body != body || !same_face(f));
    if let Some(name) = appearance {
        list.push(FaceAppearance { body, point: point.to_array(), normal: normal.to_array(), appearance: name.to_string() });
    }
}

/// The point of triangle abc nearest p.
fn closest_on_triangle(p: DVec3, a: DVec3, b: DVec3, c: DVec3) -> DVec3 {
    let (ab, ac, ap) = (b - a, c - a, p - a);
    let (d1, d2) = (ab.dot(ap), ac.dot(ap));
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }
    let bp = p - b;
    let (d3, d4) = (ab.dot(bp), ac.dot(bp));
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        return a + ab * (d1 / (d1 - d3));
    }
    let cp = p - c;
    let (d5, d6) = (ab.dot(cp), ac.dot(cp));
    if d6 >= 0.0 && d5 <= d6 {
        return c;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        return a + ac * (d2 / (d2 - d6));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && d4 - d3 >= 0.0 && d5 - d6 >= 0.0 {
        return b + (c - b) * ((d4 - d3) / ((d4 - d3) + (d5 - d6)));
    }
    let denom = 1.0 / (va + vb + vc);
    a + ab * (vb * denom) + ac * (vc * denom)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::Feature;

    /// A 10 mm cube from the origin: faces 0..6 as -X, +X, -Y, +Y, -Z, +Z, two triangles each.
    fn cube(top: f32) -> (Mesh, Vec<u32>) {
        let p = |x: f32, y: f32, z: f32| Vec3::new(x * 10.0, y * 10.0, z * top);
        let quads = [
            [p(0., 0., 0.), p(0., 0., 1.), p(0., 1., 1.), p(0., 1., 0.)],
            [p(1., 0., 0.), p(1., 1., 0.), p(1., 1., 1.), p(1., 0., 1.)],
            [p(0., 0., 0.), p(1., 0., 0.), p(1., 0., 1.), p(0., 0., 1.)],
            [p(0., 1., 0.), p(0., 1., 1.), p(1., 1., 1.), p(1., 1., 0.)],
            [p(0., 0., 0.), p(0., 1., 0.), p(1., 1., 0.), p(1., 0., 0.)],
            [p(0., 0., 1.), p(1., 0., 1.), p(1., 1., 1.), p(0., 1., 1.)],
        ];
        let mut m = Mesh { positions: Vec::new(), tris: Vec::new() };
        let mut tri_face = Vec::new();
        for (f, q) in quads.iter().enumerate() {
            let base = m.positions.len() as u32;
            m.positions.extend_from_slice(q);
            m.tris.extend([[base, base + 1, base + 2], [base, base + 2, base + 3]]);
            tri_face.extend([f as u32, f as u32]);
        }
        (m, tri_face)
    }

    #[test]
    fn a_face_is_found_again_after_the_body_changes() {
        let (m, tf) = cube(10.0);
        let top = DVec3::new(3.0, 4.0, 10.0);
        assert_eq!(face_under(&m, &tf, top, DVec3::Z), Some(5));
        assert_eq!(face_under(&m, &tf, DVec3::new(10.0, 5.0, 5.0), DVec3::X), Some(1));
        // Made taller: the top face moved up 4 mm but is still the one.
        let (tall, tf) = cube(14.0);
        assert_eq!(face_under(&tall, &tf, top, DVec3::Z), Some(5), "not the side faces, which face elsewhere");
        // Nothing facing that way anywhere near: gone.
        let far = DVec3::new(3.0, 4.0, 200.0);
        assert_eq!(face_under(&tall, &tf, far, DVec3::Z), None);
    }

    #[test]
    fn a_face_takes_one_appearance_and_can_lose_it() {
        let body = BodyId { feature: Feature::Extrude(0), piece: 0 };
        let other = BodyId { feature: Feature::Extrude(1), piece: 0 };
        let (m, tf) = cube(10.0);
        let mut list = Vec::new();
        let (m, tf) = (&m, &tf);
        let on = |p: DVec3, n: DVec3| move |f: &FaceAppearance| face_under(m, tf, DVec3::from(f.point), DVec3::from(f.normal)) == face_under(m, tf, p, n);
        let top = (DVec3::new(2.0, 2.0, 10.0), DVec3::Z);
        set_face(&mut list, body, top.0, top.1, Some("Oak"), on(top.0, top.1));
        set_face(&mut list, other, top.0, top.1, Some("Oak"), |_| false);
        // Clicked somewhere else on the same face: replaces it.
        let again = (DVec3::new(8.0, 7.0, 10.0), DVec3::Z);
        set_face(&mut list, body, again.0, again.1, Some("Walnut"), on(again.0, again.1));
        assert_eq!(list.iter().filter(|f| f.body == body).map(|f| f.appearance.as_str()).collect::<Vec<_>>(), ["Walnut"]);
        let side = (DVec3::new(0.0, 5.0, 5.0), -DVec3::X);
        set_face(&mut list, body, side.0, side.1, Some("Chrome"), on(side.0, side.1));
        set_face(&mut list, body, top.0, top.1, None, on(top.0, top.1));
        assert_eq!(list.iter().filter(|f| f.body == body).map(|f| f.appearance.as_str()).collect::<Vec<_>>(), ["Chrome"]);
        assert_eq!(list.len(), 2, "the other body's face is left alone");
    }

    #[test]
    fn names_are_unique_and_found_again() {
        for (i, x) in LIBRARY.iter().enumerate() {
            assert!(LIBRARY[..i].iter().all(|y| y.name != x.name), "{} twice", x.name);
            assert_eq!(by_name(x.name), Some(x));
        }
        assert!(by_name("Unobtainium").is_none());
    }

    #[test]
    fn values_are_in_range_and_only_metals_are_metallic() {
        for x in LIBRARY {
            for v in [x.metal, x.gloss, x.opacity] {
                assert!((0.0..=1.0).contains(&v), "{}: {v}", x.name);
            }
            assert!(x.opacity > 0.1, "{} would vanish", x.name);
            assert_eq!(x.metal == 1.0, x.category == Category::Metal, "{}", x.name);
            assert_eq!(x.opacity < 1.0, x.category == Category::Glass, "{}", x.name);
        }
    }

    #[test]
    fn every_category_has_appearances() {
        for c in Category::ALL {
            assert!(in_category(c).count() >= 3, "{}", c.name());
        }
    }
}
