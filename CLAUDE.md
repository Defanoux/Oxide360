# opencad — instructions for Claude Code

opencad is a Fusion 360-style parametric CAD app written from scratch in Rust,
for Gavin's Fedora 44 KDE laptop (Framework, Wayland). Work through `TODO.md`
one item at a time, keeping everything building and every test green.

## How to work (every task)

1. Read the item in `TODO.md`. If it's ambiguous, make the most Fusion-like
   choice and say what you chose — don't stop to ask unless it would throw
   away work.
2. Put logic in the testable modules (`sketch.rs`, `solver.rs`, `profile.rs`,
   `kernel.rs`, `model.rs`, `doc.rs`, `camera.rs`), not in `main.rs`. `main.rs`
   should only wire input → those functions → drawing.
3. Write tests for the new behaviour. Kernel tests check **exact** volumes
   against OpenCASCADE (e.g. a fillet removes r²(1−π/4)·L). Never loosen a
   tolerance or delete a test to get green — fix the code.
4. Run, in this order, and fix everything they report:
   ```
   cargo build 2>&1 | tail -40
   cargo test 2>&1 | tail -40
   ```
   No new warnings. (The GPU tests use Mesa's lavapipe; they pass headless.)
5. `git add -A && git commit -m "<what changed>"` — one commit per TODO item
   (or per meaningful step of a big one).
6. Tick the item in `TODO.md` (move it to Done with a one-line note).
7. Tell Gavin, in a few short lines: what changed, and **exactly what to try
   in the app** (check it yourself first — see below — then he checks it). Then continue with the
   next item unless he said to stop.

If `cargo build` fails in a way you can't fix in a few tries, stop and show the
error rather than rewriting large areas blind.

## Checking your work in the app

You can drive the real app yourself — do it before telling Gavin what to try:

```
target/debug/opencad --control 47321 &      # (run in the background)
target/debug/opencad ctl '{"cmd":"screenshot","path":"<scratchpad>/s.png"}'   # then Read the PNG
target/debug/opencad ctl '{"cmd":"click","x":41,"y":136}'                   # screenshot pixels
target/debug/opencad ctl '{"cmd":"state"}'   # timeline, errors, bodies + volumes, sketch, dialogs
```
Commands: `screenshot`, `state`, `click` / `double_click` (`button`, `mods`),
`move`, `drag` (`from`, `to`), `scroll` (`dy`), `key` ("Escape", "R", …),
`type`, `open` / `save` (`path`), `new`, `wait` (`frames`). Each answers once
the view has settled and any rebuild is done. In a new session the same
commands are MCP tools (`.mcp.json`, server `opencad mcp`, which starts the
app if needed). Stop the app when done; it opens on Gavin's screen.

## Talking to Gavin

Plain and short. No filler, no praise. One concrete recommendation, not a menu
of options. He pastes terminal output and screenshots; read them carefully.
He compares everything to Fusion 360 — when he describes a behaviour, match
Fusion's. If you claim you read a doc or link, actually read it.

## Commands

```
cargo run                      # the app
cargo test                     # all tests (currently 151)
cargo test kernel              # one module's tests
cargo test -- --nocapture NAME # see prints from one test
```
System packages already installed: `opencascade-devel cmake gcc-c++`.
Rust toolchain: 1.98 (edition 2024).

## Architecture

| file | owns |
|---|---|
| `main.rs` | the `OpenCad` state struct, small types, constants, `eframe::App::ui`, `main` |
| `app/*.rs` | the rest of `impl OpenCad`, by topic: `document` (files, undo, kernel cache), `sketching`, `features` (extrude/revolve/pattern/plane/fillet dialogs + drag arrows), `panels` (toolbar, browser, side panels), `timeline` (timeline bar, rename, Measure), `viewport` (input + drawing) |
| `view.rs` | egui drawing helpers: projection, lines, arcs, grid, dimensions (`linear_`, `diameter_`, `radius_`, `angle_dimension`), dashed lines, view cube. `View.trace` records drawn segments so dimensions can be clicked |
| `icons.rs` | vector-drawn toolbar icons (`Icon`, `paint`, `button`, `dropdown`). No image files — keep it that way |
| `camera.rs` | orbit camera, `zoom_at` (toward cursor), rays, view cube cells/labels |
| `sketch.rs` | 2-D model. `Entity {Line, Circle, Arc, Point}`, `Constraint` (Horizontal, Vertical, Parallel, Perpendicular, Equal, Tangent, Fix, Length, Diameter, Radius, Angle, Distance, OnLine, OnCircle, OnAxis, Midpoint; `ArcEnds` is solver-internal), `Anchor {Point, Free, OnLine, OnAxis}`, `Tool`, `Editor` state machine, hit tests, delete + renumbering, `constraint_options`, tangent circles, `arc_preview`, `rectangle_preview`, `fillet_corner` / `chamfer_corner`, movable dimension labels (`label_shift`) |
| `solver.rs` | Newton–Gauss constraint solver; `add_constraints` rejects redundant constraints; `drag` uses hop-based stiffness so far points (the opposite rectangle corner) stay put |
| `profile.rs` | closed regions from the arrangement of lines/circles/arcs (arcs tagged so the kernel rebuilds them exactly) |
| `model.rs` | `Mesh`, `Extrude`, `Revolve`, `EdgeFeature`, `EdgeRef`, `Follow`, `ray_hit` |
| `kernel.rs` | OpenCASCADE: faces from regions, extrude/revolve, booleans, fillet/chamfer, flat faces, pickable edges, face-sketch resolution, end-cap following, `build()` evaluates the timeline with per-feature errors |
| `doc.rs` | `Document` JSON save/load with validation (old files must keep loading — new fields get `#[serde(default)]`), `History` (undo/redo) |
| `export.rs` | STEP (via kernel), 3MF, STL, OBJ |
| `gpu.rs` | wgpu depth-buffered rendering of bodies + grid |
| `vendor/opencascade-sys`, `vendor/opencascade` | **patched** OCCT bindings — see below |

## Rules that keep it from breaking

- **OCCT exceptions abort the process.** They aren't `std::exception`, so they
  can't cross the cxx bridge. Any new OCCT call that can fail gets a C++
  wrapper with `try { … } catch (...) { return false/nullptr; }` in
  `vendor/opencascade-sys/include/*.hxx`, a declaration in the matching
  `src/*.rs` bridge, and a `try_*` method in `vendor/opencascade` returning
  `Option`. Existing ones: `try_fillet_edges`, `try_chamfer_edges`,
  `try_union/subtract/intersect`, `try_revolve`. Also wrap kernel calls in
  `catch_unwind`, as `build()` does.
- **Never write derived data back into the document.** Face sketches store the
  plane *as picked* (+ `follow`); the kernel re-resolves it every build. Doing
  otherwise polluted undo history.
- **Undo** (`History`) records the document once per frame when the mouse is
  up and no text box is focused. Anything that changes the document every
  frame (drags, a carried dimension label) must be gated so it's one step.
- **Kernel cache** compares sketches with `Sketch::same_shape` so moving a
  dimension label doesn't rebuild the model. Keep view-only state out of
  `same_shape`.
- **Indices shift on delete.** Deleting an entity/constraint/feature renumbers
  everything after it. Use `retain_constraints` / `remove_constraint` (they
  keep `label_shift` in step), `Follow::after_delete`, and the existing
  `delete_*` functions. Clear selections that hold indices.
- **Redundant constraints**: add new constraints through
  `solver::add_constraints`, one at a time when some may be redundant (skip
  the ones that fail). A tangent at a shared line/arc end uses the
  perpendicular form (radius ⟂ line); the distance form has a double root
  there and the solver rejects it.
- Coordinates: Z-up, millimetres; f64 for sketch/kernel geometry, f32 for
  rendering. Front faces −Y; home view is front-right-top.
- Fusion behaviours already copied (keep them): tool-first constraints
  (`Tool::Constrain`), one Dimension tool that picks length / diameter /
  radius / angle / distance from the selection and places by click or by
  typing; double-click edits a dimension, single click selects it, Delete
  removes it; drag moves its label.
- **Linux limitation**: winit doesn't deliver touchpad pinch gestures on
  Linux. Touchpad mode zooms with Ctrl + two-finger scroll. Don't try to
  "fix" pinch.
- Don't add image assets, new heavy dependencies, or network access without
  a good reason.

## Where new code goes

New logic goes in a testable module (`sketch.rs`, `offset.rs`, …). `app/*.rs` only wires input → those functions → drawing; pick the file by topic and add `pub(crate)` to methods other files call.
