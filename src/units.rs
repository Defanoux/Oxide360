//! Length units. Everything is stored in millimetres; a design has units it
//! is shown and typed in (Fusion's Document Settings → Units), and any box
//! that takes a length also takes an explicit unit: "22", "22 mm",
//! "0.5 inches", "3'", "1.5 cm".

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Unit {
    #[default]
    Mm,
    Cm,
    M,
    In,
    Ft,
}

impl Unit {
    pub const ALL: [Unit; 5] = [Unit::Mm, Unit::Cm, Unit::M, Unit::In, Unit::Ft];

    pub fn name(self) -> &'static str {
        match self {
            Unit::Mm => "mm",
            Unit::Cm => "cm",
            Unit::M => "m",
            Unit::In => "in",
            Unit::Ft => "ft",
        }
    }

    /// The name in full, for menus.
    pub fn long_name(self) -> &'static str {
        match self {
            Unit::Mm => "Millimetres (mm)",
            Unit::Cm => "Centimetres (cm)",
            Unit::M => "Metres (m)",
            Unit::In => "Inches (in)",
            Unit::Ft => "Feet (ft)",
        }
    }

    /// How many millimetres one of these is.
    pub fn mm(self) -> f64 {
        match self {
            Unit::Mm => 1.0,
            Unit::Cm => 10.0,
            Unit::M => 1000.0,
            Unit::In => 25.4,
            Unit::Ft => 304.8,
        }
    }

    /// Decimal places lengths are shown with (Fusion's look: 2 for mm, 3 for inches).
    pub fn decimals(self) -> usize {
        match self {
            Unit::Mm => 2,
            Unit::Cm | Unit::In => 3,
            Unit::M | Unit::Ft => 4,
        }
    }

    /// A length (mm) as a plain number in this unit: "0.787".
    pub fn plain(self, mm: f64) -> String {
        format!("{:.*}", self.decimals(), mm / self.mm())
    }

    /// A length (mm) with the unit: "0.787 in".
    pub fn show(self, mm: f64) -> String {
        format!("{} {}", self.plain(mm), self.name())
    }

    /// An area (mm²) with the unit: "1.000 in²".
    pub fn show_area(self, mm2: f64) -> String {
        format!("{:.*} {}²", self.decimals(), mm2 / (self.mm() * self.mm()), self.name())
    }

    /// The unit a word names, if any ("inches", "\"", "mm"…).
    pub fn from_word(w: &str) -> Option<Unit> {
        Some(match w.to_ascii_lowercase().as_str() {
            "mm" | "millimetre" | "millimetres" | "millimeter" | "millimeters" => Unit::Mm,
            "cm" | "centimetre" | "centimetres" | "centimeter" | "centimeters" => Unit::Cm,
            "m" | "metre" | "metres" | "meter" | "meters" => Unit::M,
            "in" | "inch" | "inches" | "\"" => Unit::In,
            "ft" | "foot" | "feet" | "'" => Unit::Ft,
            _ => return None,
        })
    }
}

/// A drag box for a length stored in mm, shown in `unit` (with its name)
/// and typed in it — or in any unit written after the number ("22 mm").
/// `mm_per_px` is how far one pixel of dragging moves it.
pub fn length_drag(value: &mut f64, unit: Unit, mm_per_px: f64) -> eframe::egui::DragValue<'_> {
    eframe::egui::DragValue::new(value)
        .speed(mm_per_px)
        .custom_formatter(move |v, _| unit.plain(v))
        .custom_parser(move |s| parse_length(s, unit))
        .suffix(format!(" {}", unit.name()))
}

/// A number and the word after it ("22 mm" → (22, "mm")).
fn split(text: &str) -> Option<(f64, &str)> {
    let t = text.trim();
    let at = t.find(|c: char| !(c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E'))).unwrap_or(t.len());
    let (num, word) = t.split_at(at);
    let v: f64 = num.trim().parse().ok().filter(|v: &f64| v.is_finite())?;
    Some((v, word.trim()))
}

/// A typed length in mm. A bare number is in `unit` (the design's units);
/// a number with a unit is in that unit.
/// Anything more is an expression (with the design's parameters): "width * 2".
pub fn parse_length(text: &str, unit: Unit) -> Option<f64> {
    if let Some((v, word)) = split(text) {
        let u = if word.is_empty() { Some(unit) } else { Unit::from_word(word) };
        if let Some(u) = u {
            return Some(v * u.mm());
        }
    }
    crate::expr::eval(text, crate::expr::Kind::Length, unit).ok()
}

/// Is `text` more than a number (with a unit)? Then it's an expression.
pub fn is_expression(text: &str) -> bool {
    split(text).is_none_or(|(_, word)| !word.is_empty() && Unit::from_word(word).is_none() && !matches!(word.to_ascii_lowercase().as_str(), "°" | "deg" | "degree" | "degrees" | "rad" | "radian" | "radians"))
}

/// The unit written after a typed number, if any ("1 in" → In, "1" → None).
pub fn typed_unit(text: &str) -> Option<Unit> {
    let (_, word) = split(text)?;
    Unit::from_word(word)
}

/// A typed angle in degrees: "90", "90°", "90 deg", "1.5708 rad". Never
/// scaled by the design's length units.
pub fn parse_angle(text: &str) -> Option<f64> {
    if let Some((v, word)) = split(text) {
        match word.to_ascii_lowercase().as_str() {
            "" | "°" | "deg" | "degree" | "degrees" => return Some(v),
            "rad" | "radian" | "radians" => return Some(v.to_degrees()),
            _ => {}
        }
    }
    crate::expr::eval(text, crate::expr::Kind::Angle, Unit::Mm).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_number_is_in_the_designs_units_and_a_unit_wins() {
        assert_eq!(parse_length("22", Unit::Mm), Some(22.0));
        assert!((parse_length("22", Unit::In).unwrap() - 22.0 * 25.4).abs() < 1e-12);
        assert_eq!(parse_length("22 mm", Unit::In), Some(22.0));
        assert!((parse_length("22 inches", Unit::Mm).unwrap() - 558.8).abs() < 1e-9);
        assert!((parse_length("0.5\"", Unit::Mm).unwrap() - 12.7).abs() < 1e-12);
        assert_eq!(parse_length("1'", Unit::Mm), Some(304.8));
        assert_eq!(parse_length("2 Feet", Unit::Mm), Some(609.6));
        assert_eq!(parse_length("1.5 cm", Unit::In), Some(15.0));
        assert_eq!(parse_length("3 metres", Unit::Mm), Some(3000.0));
        assert_eq!(parse_length("-2.5mm", Unit::In), Some(-2.5));
        assert_eq!(parse_length("1e1", Unit::Mm), Some(10.0));
        assert_eq!(parse_length("2 parsecs", Unit::Mm), None);
        assert_eq!(parse_length("", Unit::Mm), None);
        assert_eq!(parse_length("90°", Unit::Mm), None, "an angle isn't a length");
    }

    #[test]
    fn the_unit_typed_is_known() {
        assert_eq!(typed_unit("1 in"), Some(Unit::In));
        assert_eq!(typed_unit("25.4mm"), Some(Unit::Mm));
        assert_eq!(typed_unit("3"), None);
        assert_eq!(typed_unit("x"), None);
    }

    #[test]
    fn angles_ignore_the_length_units() {
        assert_eq!(parse_angle("90"), Some(90.0));
        assert_eq!(parse_angle("45 °"), Some(45.0));
        assert_eq!(parse_angle("30deg"), Some(30.0));
        assert!((parse_angle("3.14159265358979 rad").unwrap() - 180.0).abs() < 1e-9);
        assert_eq!(parse_angle("5 mm"), None);
    }

    #[test]
    fn lengths_show_in_the_unit_and_read_back() {
        assert_eq!(Unit::In.show(25.4), "1.000 in");
        assert_eq!(Unit::Mm.show(12.7), "12.70 mm");
        assert_eq!(Unit::Cm.plain(12.7), "1.270");
        for u in Unit::ALL {
            let shown = u.show(123.456);
            let back = parse_length(&shown, Unit::Mm).unwrap();
            assert!((back - 123.456).abs() <= 0.5 * 10f64.powi(-(u.decimals() as i32)) * u.mm() + 1e-9, "{shown}");
        }
    }
}
