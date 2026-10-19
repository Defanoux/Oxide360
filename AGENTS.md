# Oxide360 – agent notes

Parametric 3D CAD for Linux, modelled on Fusion 360. Rust, OpenCASCADE (exact B-rep), egui + wgpu.
Everything is stored in millimetres (units.rs). The app struct is `OpenCad`; files in src/app/ each add to `impl OpenCad`.

## Rules
- Context is small. Never read a file over 500 lines whole: find the spot with `grep -n`, then read with offset/limit.
- Do only what was asked. Don't refactor, rename, or reformat nearby code.
- After editing: `cargo fmt`, then `cargo check`.
- Run tests filtered: `cargo test <name>`. Never run the full `cargo test`.
- Tests go in `#[cfg(test)] mod tests { use super::*; ... }` at the end of the file. Never give a test the same name as the function it tests.
- Only describe code you have actually read. If unsure, grep first.

## Large files (never read whole)
kernel.rs 6.4k · sketch.rs 5.5k · app/viewport.rs 4.3k · app/panels.rs 2.4k · app/features.rs 2.3k · main.rs 1.9k · solver.rs 1.7k · model.rs 1.6k · doc.rs 1.4k · app/document.rs 1.2k

## Module map
Geometry
- kernel.rs: solid modeling via OpenCASCADE
- model.rs: solid features and their triangle meshes
- sketch.rs: 2D sketches: data model, editing tools, hit testing, deletion
- solver.rs: geometric constraint solver
- profile.rs: closed sketch regions that can be extruded
- offset.rs, pattern.rs, spline.rs, spline_edit.rs, project.rs, text.rs: sketch offset/extend, mirror, splines, trim/break, edge projection, text
- threads.rs: ISO thread sizes for Hole/Thread

Document and files
- doc.rs: everything saved, file I/O, STL export
- export.rs: STL/3MF/OBJ mesh export · sketch_export.rs: sketch to DXF/SVG
- expr.rs: expressions and user parameters · materials.rs: density · assembly.rs: components
- recovery.rs: autosave · settings.rs: preferences (not part of designs) · colors.rs: body colours

App (impl OpenCad, src/app/)
- viewport.rs: 3D/sketch viewport input and drawing
- panels.rs: toolbar, browser, side panels, number box
- features.rs: extrude/revolve/pattern/plane/axis/fillet dialogs and drag arrows
- sketching.rs: starting/editing/finishing sketches, sketch tool actions
- document.rs: documents, files, undo, kernel cache, export
- timeline.rs: timeline bar, renaming, Measure · menus.rs: right-click menu, S-key shortcuts
- control.rs: what the control link asks of a design

Dialogs: *_dialog.rs files, one per feature (hole, shell, sweep, thread, joint, …); params_window.rs
Rendering: gpu.rs (wgpu solids), view.rs (geometry to egui shapes), camera.rs, origin.rs, gizmo.rs, handles.rs, icons.rs
Input: keys.rs, shortcuts.rs, rect_input.rs, inertia.rs
Other: control.rs (drive the app externally), measure.rs, help.rs, ui_shot.rs (test-only UI screenshots)
