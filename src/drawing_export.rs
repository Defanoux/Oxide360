//! A drawing sheet (`drawing::Sheet`) written out: PDF (one vector page at
//! true size, for printing) and DXF (R12, millimetres, a layer per line kind
//! with hidden lines dashed — for LibreCAD, Fusion, laser cutters).

use crate::drawing::{Pen, Sheet};
use std::fmt::Write;

/// PDF points per millimetre.
const PT: f64 = 72.0 / 25.4;

/// The sheet as a one-page PDF, the paper's own size. Text in Helvetica
/// (every reader has it); letters it can't show become "?".
pub fn pdf(sheet: &Sheet) -> Vec<u8> {
    let mut content = String::new();
    // Millimetres from here on; round ends and corners.
    let _ = writeln!(content, "{PT:.6} 0 0 {PT:.6} 0 0 cm 1 J 1 j 0 G");
    for (pen, pts) in &sheet.lines {
        if pts.len() < 2 {
            continue;
        }
        let dash = if *pen == Pen::Dashed { "[3 1.5] 0 d" } else { "[] 0 d" };
        let _ = write!(content, "{} w {dash} {:.4} {:.4} m", pen.width(), pts[0].x, pts[0].y);
        for p in &pts[1..] {
            let _ = write!(content, " {:.4} {:.4} l", p.x, p.y);
        }
        content.push_str(" S\n");
    }
    for t in &sheet.texts {
        // Helvetica's capitals are 0.718 of its size.
        let _ = writeln!(content, "BT /F1 {:.3} Tf {:.4} {:.4} Td ({}) Tj ET", t.height / 0.718, t.at.x, t.at.y, pdf_text(&t.text));
    }
    let (w, h) = (sheet.size.x * PT, sheet.size.y * PT);
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {w:.3} {h:.3}] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>"),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_string(),
        format!("<< /Length {} >>\nstream\n{content}endstream", content.len()),
    ];
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    let mut tail = format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1);
    for o in offsets {
        let _ = writeln!(tail, "{o:010} 00000 n ");
    }
    let _ = write!(tail, "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1);
    out.extend_from_slice(tail.as_bytes());
    out
}

/// Text inside a PDF string: brackets and backslashes escaped, only plain
/// letters kept (others as "?").
fn pdf_text(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '(' | ')' | '\\' => format!("\\{c}"),
            ' '..='~' => c.to_string(),
            _ => "?".to_string(),
        })
        .collect()
}

/// The layer each kind of line goes on.
fn layer(pen: Pen) -> &'static str {
    match pen {
        Pen::Thick => "VISIBLE",
        Pen::Thin => "THIN",
        Pen::Dashed => "HIDDEN",
    }
}

/// The sheet as an AutoCAD R12 DXF in millimetres: layers VISIBLE, THIN,
/// HIDDEN (dashed linetype) and TEXT.
pub fn dxf(sheet: &Sheet) -> String {
    let mut s = String::new();
    let mut g = |code: i32, value: &str| {
        let _ = writeln!(s, "{code}\n{value}");
    };
    g(0, "SECTION");
    g(2, "HEADER");
    g(9, "$ACADVER");
    g(1, "AC1009");
    g(9, "$INSUNITS");
    g(70, "4");
    g(0, "ENDSEC");
    g(0, "SECTION");
    g(2, "TABLES");
    g(0, "TABLE");
    g(2, "LTYPE");
    g(70, "2");
    for (name, about, dashes) in [("CONTINUOUS", "Solid line", &[][..]), ("HIDDEN", "Hidden __ __ __", &[3.0, -1.5][..])] {
        g(0, "LTYPE");
        g(2, name);
        g(70, "0");
        g(3, about);
        g(72, "65");
        g(73, &dashes.len().to_string());
        g(40, &format!("{:.3}", dashes.iter().map(|d: &f64| d.abs()).sum::<f64>()));
        for d in dashes {
            g(49, &format!("{d:.3}"));
        }
    }
    g(0, "ENDTAB");
    g(0, "TABLE");
    g(2, "LAYER");
    g(70, "4");
    // Colours: white, grey, dark grey (hidden), white.
    for (name, colour, ltype) in [("VISIBLE", "7", "CONTINUOUS"), ("THIN", "8", "CONTINUOUS"), ("HIDDEN", "9", "HIDDEN"), ("TEXT", "7", "CONTINUOUS")] {
        g(0, "LAYER");
        g(2, name);
        g(70, "0");
        g(62, colour);
        g(6, ltype);
    }
    g(0, "ENDTAB");
    g(0, "ENDSEC");
    g(0, "SECTION");
    g(2, "ENTITIES");
    let f = |x: f64| format!("{x:.6}");
    for (pen, pts) in &sheet.lines {
        if pts.len() < 2 {
            continue;
        }
        g(0, "POLYLINE");
        g(8, layer(*pen));
        g(66, "1");
        g(70, "0");
        for q in pts {
            g(0, "VERTEX");
            g(8, layer(*pen));
            g(10, &f(q.x));
            g(20, &f(q.y));
            g(30, &f(0.0));
        }
        g(0, "SEQEND");
    }
    for t in &sheet.texts {
        g(0, "TEXT");
        g(8, "TEXT");
        g(10, &f(t.at.x));
        g(20, &f(t.at.y));
        g(30, &f(0.0));
        g(40, &f(t.height));
        g(1, &t.text);
    }
    g(0, "ENDSEC");
    g(0, "EOF");
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drawing::Text;
    use glam::DVec2;

    fn sample() -> Sheet {
        Sheet {
            size: DVec2::new(420.0, 297.0),
            lines: vec![(Pen::Thick, vec![DVec2::new(10.0, 10.0), DVec2::new(410.0, 10.0)]), (Pen::Dashed, vec![DVec2::new(20.0, 20.0), DVec2::new(40.0, 20.0)])],
            texts: vec![Text { at: DVec2::new(250.0, 20.0), height: 3.5, text: "Bracket (v2) Ø10".into() }],
            scale: 1.0,
        }
    }

    #[test]
    fn the_pdf_is_a3_and_its_index_points_at_its_objects() {
        let bytes = pdf(&sample());
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.starts_with("%PDF-1.4") && text.trim_end().ends_with("%%EOF"));
        assert!(text.contains("/MediaBox [0 0 1190.551 841.890]"), "A3 in points");
        assert!(text.contains("[3 1.5] 0 d"), "hidden lines dashed");
        assert!(text.contains("(Bracket \\(v2\\) ?10) Tj"), "brackets escaped, other letters as ?");
        // The cross-reference table: every offset lands on its object.
        let start: usize = text.lines().skip_while(|l| *l != "startxref").nth(1).unwrap().parse().unwrap();
        assert!(text[start..].starts_with("xref"));
        let offsets: Vec<usize> = text[start..].lines().skip(3).take(5).map(|l| l[..10].parse().unwrap()).collect();
        for (i, o) in offsets.iter().enumerate() {
            assert!(text[*o..].starts_with(&format!("{} 0 obj", i + 1)), "object {}", i + 1);
        }
        // The stream's stated length is its real length.
        let len: usize = text.split("/Length ").nth(1).unwrap().split(' ').next().unwrap().parse().unwrap();
        let body = text.split(">>\nstream\n").nth(1).unwrap().split("endstream").next().unwrap();
        assert_eq!(body.len(), len);
    }

    #[test]
    fn the_dxf_has_a_dashed_hidden_layer() {
        let d = dxf(&sample());
        assert!(d.contains("LTYPE\n2\nHIDDEN") && d.contains("LAYER\n2\nHIDDEN\n70\n0\n62\n9\n6\nHIDDEN"));
        assert_eq!(d.matches("\nPOLYLINE\n").count(), 2);
        assert!(d.contains("POLYLINE\n8\nHIDDEN") && d.contains("TEXT\n8\nTEXT"));
        assert!(d.trim_end().ends_with("EOF"));
    }
}
