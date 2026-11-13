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

/// The tool reference in Help (each shown with its toolbar icon): sections
/// of (tool name as the toolbar has it, a sketch tool?, what it does).
pub const TOOLS: &[(&str, &[(&str, bool, &str)])] = &[
    ("Sketch: draw", &[
        ("Create Sketch", false, "Start a sketch on an origin plane, a construction plane or a flat face. Everything 2-D starts here."),
        ("Line", true, "Click point after point; each click starts the next line. Click and drag from an end for a tangent arc. Esc or double-click ends the chain."),
        ("Rectangle", true, "Two corners, a centre and a corner, or three points for a turned one. Type the width and height while drawing."),
        ("Circle", true, "Centre and radius, two points across, three points, or tangent to lines."),
        ("Arc", true, "Through three points, centre and two ends, or tangent to where a line ends."),
        ("Polygon", true, "A regular polygon around a centre: drawn inside or outside a circle, or by an edge. Type how many sides."),
        ("Slot", true, "A rounded slot: centre to centre, overall, or along an arc."),
        ("Spline", true, "A smooth curve through the points clicked (drag its handles after)."),
        ("Ellipse", true, "Centre, then the end of one axis, then how wide the other is."),
        ("Point", true, "A single point, for holes and for constraints to hold on to."),
        ("Text", true, "Text in any installed font; extrude or emboss it later."),
        ("Insert SVG / DXF", true, "Brings a drawing's lines, arcs and curves into the sketch, at its real size, placed with drag arrows."),
    ]),
    ("Sketch: change", &[
        ("Fillet", true, "Rounds the corner where two sketch lines meet."),
        ("Chamfer", true, "Cuts a straight bevel across the corner where two lines meet."),
        ("Trim", true, "Click a piece of a curve to cut it back to where it crosses others (X)."),
        ("Break", true, "Splits a curve where it crosses others, without removing anything."),
        ("Extend", true, "Lengthens a curve to the next one it would meet."),
        ("Offset", true, "A copy of a chain of curves, a set distance to one side."),
        ("Mirror", true, "Copies picked curves across a line or axis; the copies stay tied to the originals."),
        ("Rectangular Pattern", true, "Copies picked curves in rows and columns."),
        ("Circular Pattern", true, "Copies picked curves round a point."),
        ("Project", true, "Brings a body's edges into the sketch; they follow the body when it changes."),
        ("Construction", true, "Turns picked curves into construction lines (dashed): they guide, but aren't part of any profile (K)."),
        ("Change Parameters", true, "Named values (width = 40 mm) that dimensions and features can use in place of numbers."),
    ]),
    ("Sketch: constraints", &[
        ("Dimension", true, "Sets a length, distance, angle or diameter; type a number or a parameter (D)."),
        ("Coincident", true, "Puts a point on another point or on a curve."),
        ("Collinear", true, "Keeps lines on one straight line."),
        ("Concentric", true, "Gives circles and arcs the same centre."),
        ("Horizontal", true, "Makes a line (or two points) level."),
        ("Vertical", true, "Makes a line (or two points) upright."),
        ("Parallel", true, "Keeps lines pointing the same way."),
        ("Perpendicular", true, "Keeps lines at right angles."),
        ("Equal", true, "Same length lines, or same size circles."),
        ("Tangent", true, "A line or arc touches a curve smoothly."),
        ("Midpoint", true, "Puts a point at the middle of a line."),
        ("Symmetric", true, "Two points mirror each other across a line."),
        ("Fix", true, "Pins geometry where it is."),
    ]),
    ("Create", &[
        ("Extrude", false, "Pulls sketch profiles or faces into a solid, or cuts with them: a distance, to an object, or through all; one side, two sides or symmetric; with a taper (E)."),
        ("Revolve", false, "Turns a profile round an axis into a solid (a bottle, a wheel)."),
        ("Sweep", false, "Moves a profile along a path."),
        ("Loft", false, "Blends between two or more profiles."),
        ("Pipe", false, "A round, square or triangular tube along a path."),
        ("Coil", false, "A spring or coil: diameter, turns, pitch and section size."),
        ("Emboss", false, "Raises or cuts sketch text or profiles into a flat face, or wraps them round a cylinder."),
        ("Hole", false, "Simple, counterbored or countersunk holes, tapped or not, at sketch points or a clicked spot (H)."),
        ("Thread", false, "Adds a real (modeled) thread to a round face, from the standard sizes."),
    ]),
    ("Modify", &[
        ("Press Pull", false, "Drag a face to push it in or pull it out (Q)."),
        ("Fillet", false, "Rounds picked edges (F)."),
        ("Chamfer", false, "Bevels picked edges: one distance, two distances, or distance and angle."),
        ("Shell", false, "Hollows a body to a wall thickness, leaving picked faces open."),
        ("Move/Copy", false, "Moves or turns bodies with arrows, or by numbers; can leave a copy (M)."),
        ("Combine", false, "Joins bodies, cuts one with others, or keeps only where they overlap."),
        ("Split Body", false, "Cuts a body into pieces along a plane or flat face."),
        ("Scale", false, "Makes bodies bigger or smaller, evenly or by X / Y / Z."),
        ("Appearance", false, "How bodies or faces look: metals, plastics, paint, wood, glass (A)."),
        ("Change Parameters", false, "Named values used by dimensions and features; change one and the design follows."),
    ]),
    ("Pattern", &[
        ("Mirror", false, "Copies bodies or features across a plane."),
        ("Rectangular Pattern", false, "Copies bodies or features in rows and columns."),
        ("Circular Pattern", false, "Copies bodies or features round an axis."),
    ]),
    ("Construct", &[
        ("Offset Plane", false, "A plane parallel to a plane or face, a distance away."),
        ("Plane at Angle", false, "A plane through an edge or axis, turned by an angle."),
        ("Midplane", false, "A plane halfway between two faces."),
        ("Plane Through Three Points", false, "A plane through three points or corners."),
        ("Tangent Plane", false, "A plane touching a round face."),
        ("Axis Along Edge", false, "An axis along a straight edge."),
        ("Axis Through Cylinder", false, "An axis through the middle of a round face."),
        ("Axis Through Two Points", false, "An axis from one point to another."),
        ("Point", false, "A construction point on a corner, an edge, a face or in space."),
    ]),
    ("Inspect", &[
        ("Measure", false, "Distances, angles, lengths and areas between whatever you click (I)."),
        ("Print Check", false, "For 3-D printing: does it fit the printer, where it overhangs, and the best face to lay it on."),
    ]),
];

#[cfg(test)]
mod tool_reference_tests {
    #[test]
    fn every_tool_is_in_the_reference_with_an_icon() {
        let has = |name: &str, sketching: bool| super::TOOLS.iter().any(|(_, tools)| tools.iter().any(|(n, s, _)| *n == name && *s == sketching));
        for name in crate::shortcuts::SOLID {
            assert!(has(name, false), "solid tool {name} has no Help entry");
        }
        for name in crate::shortcuts::SKETCH {
            assert!(has(name, true), "sketch tool {name} has no Help entry");
        }
        for (_, tools) in super::TOOLS {
            for (name, sketching, _) in tools.iter() {
                assert!(crate::app::menus::tool_icon(name, *sketching).is_some(), "{name} has no icon");
            }
        }
    }
}
