//! Appearances (Fusion's Appearance, A): how a body looks — colour, how
//! metallic and how glossy it is, and how see-through. Only the look: what a
//! body weighs is its Physical Material (`materials.rs`), chosen separately.
//! A body with an appearance takes its colour from it; one without keeps its
//! own colour and the plain shading.

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

#[cfg(test)]
mod tests {
    use super::*;

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
