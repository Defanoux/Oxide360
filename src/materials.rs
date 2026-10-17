//! Physical materials (Fusion's Physical Material): a body's density, so its
//! mass can be shown. A body with none chosen is Steel, as in Fusion.

/// A material and its density (g/cm³).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Material {
    pub name: &'static str,
    pub density: f64,
}

/// The library, metals then plastics then others.
pub const ALL: &[Material] = &[
    Material { name: "Steel", density: 7.85 },
    Material { name: "Stainless Steel", density: 8.00 },
    Material { name: "Aluminum 6061", density: 2.70 },
    Material { name: "Brass", density: 8.50 },
    Material { name: "Copper", density: 8.96 },
    Material { name: "Titanium", density: 4.43 },
    Material { name: "Cast Iron", density: 7.20 },
    Material { name: "PLA", density: 1.24 },
    Material { name: "PETG", density: 1.27 },
    Material { name: "ABS", density: 1.04 },
    Material { name: "Nylon", density: 1.14 },
    Material { name: "Acrylic", density: 1.18 },
    Material { name: "Polycarbonate", density: 1.20 },
    Material { name: "Oak", density: 0.75 },
    Material { name: "Pine", density: 0.50 },
    Material { name: "Rubber", density: 1.10 },
    Material { name: "Glass", density: 2.50 },
];

pub const DEFAULT: &str = "Steel";

/// The material called `name` (an unknown name, e.g. from a newer file, is Steel).
pub fn by_name(name: &str) -> Material {
    ALL.iter().copied().find(|m| m.name == name).unwrap_or(ALL[0])
}

/// Grams in `volume` mm³ of something `density` g/cm³.
pub fn mass_g(volume_mm3: f64, density: f64) -> f64 {
    volume_mm3 / 1000.0 * density
}

/// A mass for people: grams below a kilogram, else kilograms.
pub fn format_mass(g: f64) -> String {
    if g.abs() < 1000.0 { format!("{g:.2} g") } else { format!("{:.3} kg", g / 1000.0) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_litre_of_water_like_stuff_weighs_a_kilogram() {
        // 100 × 100 × 100 mm = 1 L.
        assert!((mass_g(1.0e6, 1.0) - 1000.0).abs() < 1e-9);
        // A 20 × 10 × 5 steel block: 1 cm³ × 7.85.
        assert!((mass_g(1000.0, by_name("Steel").density) - 7.85).abs() < 1e-12);
        assert_eq!(format_mass(7.85), "7.85 g");
        assert_eq!(format_mass(7850.0), "7.850 kg");
    }

    #[test]
    fn unknown_names_are_steel_and_names_are_unique() {
        assert_eq!(by_name("Unobtainium").name, DEFAULT);
        assert_eq!(by_name("PLA").density, 1.24);
        for (k, m) in ALL.iter().enumerate() {
            assert!(ALL[k + 1..].iter().all(|n| n.name != m.name), "{} twice", m.name);
        }
    }
}
