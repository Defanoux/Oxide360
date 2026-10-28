//! Help (the ? menu): a short getting-started guide, and About — the
//! version, who made it, and the libraries it's built on with their
//! licences (their licences ask for that credit).

/// Getting Started: (heading, what to do).
pub const GETTING_STARTED: &[(&str, &str)] = &[
    ("Look around", "Drag with the left mouse button to orbit, the right (or middle) to pan, the wheel to zoom toward the cursor. F6 fits everything; the view cube's faces and corners turn the view; its house goes Home."),
    ("Sketch", "CREATE > Create Sketch (N), then click a plane or a flat face. Draw with Line (L), Rectangle (R), Circle (C)… Dimension (D) sets sizes; double-click a dimension to type a new value. FINISH SKETCH (top right) when done."),
    ("Make it solid", "Extrude (E): click a closed area of a sketch, drag the arrow or type the distance. Revolve, Sweep, Loft, Hole and the rest are under CREATE; Fillet (F), Shell, Move/Copy (M), Appearance (A: metal, plastic, wood, glass looks) under MODIFY."),
    ("Change your mind", "Everything you make is in the timeline at the bottom: double-click a step to edit it, right-click to delete or suppress it, drag the marker back to see the part as it was. Ctrl+Z undoes."),
    ("Find a tool", "Press S and type part of its name. Right-click in the view for the marking menu: the common tools round the cursor, and what you clicked on in the list below."),
    ("Several parts", "Right-click bodies in the browser > Create Component, then ASSEMBLE > Joint to put one part onto another (rigid, turning or sliding)."),
    ("Check and share", "INSPECT > Measure, Section Analysis, Interference; right-click a body for its Properties (mass needs a Physical Material). File > Export writes STEP, STL, 3MF or OBJ; File > Insert brings STEP and STL in."),
];

/// What opencad is built on: (name, what it does here, licence).
pub const LIBRARIES: &[(&str, &str, &str)] = &[
    ("Open CASCADE Technology", "the geometry kernel (solids, booleans, fillets, STEP)", "LGPL 2.1 with exception"),
    ("opencascade-rs", "Rust bindings to Open CASCADE (patched here)", "LGPL 2.1"),
    ("egui / eframe", "the windows, buttons and panels", "MIT or Apache 2.0"),
    ("wgpu", "drawing the model on the graphics card", "MIT or Apache 2.0"),
    ("glam", "vectors and matrices", "MIT or Apache 2.0"),
    ("serde / serde_json", "saving and opening designs", "MIT or Apache 2.0"),
    ("rfd", "the open and save file pickers", "MIT"),
    ("ab_glyph", "letters for sketch text", "Apache 2.0"),
    ("png", "screenshots", "MIT or Apache 2.0"),
];

/// The logo (assets/oxide360.svg, drawn at 256 px) as the window's icon.
pub fn app_icon() -> Option<egui::IconData> {
    let mut reader = png::Decoder::new(std::io::Cursor::new(&include_bytes!("../assets/oxide360-256.png")[..])).read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf[..info.buffer_size()].to_vec(),
        png::ColorType::Rgb => buf[..info.buffer_size()].chunks(3).flat_map(|c| [c[0], c[1], c[2], 255]).collect(),
        _ => return None,
    };
    Some(egui::IconData { rgba, width: info.width, height: info.height })
}

pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn about_names_the_kernel_and_the_version() {
        assert!(LIBRARIES.iter().any(|(n, _, l)| n.contains("CASCADE") && l.contains("LGPL")));
        assert!(!version().is_empty());
        let icon = app_icon().expect("the logo decodes");
        assert_eq!((icon.width, icon.height, icon.rgba.len()), (256, 256, 256 * 256 * 4));
        assert!(GETTING_STARTED.iter().all(|(h, t)| !h.is_empty() && t.len() > 40));
    }
}
