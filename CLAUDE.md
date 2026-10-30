# Oxide360 — instructions for Claude Code

Oxide360 is a Fusion 360-style parametric CAD app written from scratch in
Rust (exact OpenCASCADE geometry, egui + wgpu UI). It was called **opencad**
until 2026-10-17 — you'll still see that name in old commits, the `OpenCad`
state struct, and the `opencad_try_*` C++ wrappers. Read `docs/PROJECT.md`
once for the story so far, the decisions already made and how releases work.

- **Owner:** Gavin (GitHub **Defanoux**, Discord Defanoux,
  oxide360@defanoux.uk). Fedora 44 KDE on a Framework laptop (Wayland).
- **Repo:** https://github.com/Defanoux/Oxide360 (public, GPL-3.0-or-later).
  Branch `main`. CI (`.github/workflows/ci.yml`) builds and runs every test
  on Fedora 44 for each push.
- **App id:** `io.github.Defanoux.Oxide360` (Flatpak name, desktop file,
  Wayland app id — keep them identical). Designs are `.ox360`; old `.ocad`
  still open.
- **Work list:** `TODO.md` (Next → Polish → Done). `FEATURES.md` is the
  original Fusion-parity roadmap.

## How to work (every task)

1. Read the item in `TODO.md`. If it's ambiguous, make the most Fusion-like
   choice and say what you chose — don't stop to ask unless it would throw
   away work.
2. Put logic in the testable modules (`sketch.rs`, `solver.rs`, `profile.rs`,
   `kernel.rs`, `model.rs`, `doc.rs`, `camera.rs`, the `*_dialog.rs` files…),
   not in `main.rs` / `app/`. Those only wire input → those functions → drawing.
3. Write tests for the new behaviour. Kernel tests check **exact** volumes
   against OpenCASCADE (e.g. a fillet removes r²(1−π/4)·L). Never loosen a
   tolerance or delete a test to get green — fix the code.
4. Run, in this order, and fix everything they report:
   ```
   cargo build 2>&1 | tail -40
   cargo test 2>&1 | tail -40
   ```
   No new warnings. (The GPU tests use Mesa's lavapipe; they pass headless.)
5. Check it in the running app yourself (below) before telling Gavin.
6. `git add -A && git commit -m "<what changed>"` — one commit per TODO item
   (or per meaningful step of a big one), then `git push`. Commits go out as
   `Defanoux <128421570+Defanoux@users.noreply.github.com>` (set in the
   repo's git config) — never his personal email.
7. Tick the item in `TODO.md` (move it to Done with a one-line note).
8. Tell Gavin, in a few short lines: what changed, and **numbered steps of
   exactly what to try** (he answers "1 works, 2 works, 3 …"). Then continue
   with the next item unless he said to stop.

If `cargo build` fails in a way you can't fix in a few tries, stop and show the
error rather than rewriting large areas blind.

## Talking to Gavin

Plain and short. No filler, no praise. One concrete recommendation, not a
menu of options. He pastes terminal output and screenshots; read them
carefully (screenshots are often cropped — look for the detail he means). He
compares everything to Fusion 360 — when he describes a behaviour, match
Fusion's; when he shows a Fusion screenshot, copy what it does. If you claim
you read a doc or link, actually read it. When he asks you to stop after a
milestone, stop and give him a list of everything changed with steps to try.

Things he has asked for that apply everywhere:
- A one-off tool drops back to Select after it's used; only tools used many
  times in a row (Line, quick constraints) stay on. Multi-step tools show a
  hint at the top of the view ("1 of 3: click …").
- Picking points on a body: hovering a flat face shows snap dots (middle,
  side middles, corners — `kernel::face_snaps`); a round edge snaps to its
  centre and lights up whole.
- Disabled toolbar tools stay right-clickable and draggable.
- Numbers typed in dialogs apply when the box is left (Enter / click away),
  and slow previews (coils, sweeps) wait until a drag is let go.

## Checking your work in the app

You can drive the real app yourself — do it before telling Gavin what to try:

```
target/debug/oxide360 --control 47321 &      # (run in the background)
target/debug/oxide360 ctl '{"cmd":"screenshot","path":"<scratchpad>/s.png"}'   # then Read the PNG
target/debug/oxide360 ctl '{"cmd":"click","x":41,"y":136}'                   # screenshot pixels
target/debug/oxide360 ctl '{"cmd":"state"}'   # timeline, errors, bodies + volumes, sketch, dialogs
kill $(pgrep -f -- "--control 4732[1]")      # stop YOUR copy only
```
Commands: `screenshot`, `state`, `click` / `double_click` (`button`, `mods`:
`["shift"]`), `move`, `drag` (`from`, `to`), `scroll` (`dy`), `key`
("Escape", "R", …), `type`, `open` / `save` / `import` (`path`), `new`,
`wait` (`frames`). Each answers once the view has settled and any rebuild is
done. In a new session the same commands are MCP tools (`.mcp.json`, server
`oxide360 mcp`, which starts the app if needed). The app opens on Gavin's
screen — stop it when done.

Gotchas:
- Screenshots are physical pixels (scale 1.7 on his screen); the image you
  Read is shown smaller — multiply what you see by the factor it states.
- Stop your instance with the `pgrep -f -- "--control 4732[1]"` form: `pgrep
  -x oxide360` also matches Gavin's own window, and `pkill -f` with a plain
  pattern kills your own shell.
- Don't `ctl save` into his real files; `save` also adds to File → Recent.
- Dialogs can be checked without the GUI: `src/ui_shot.rs` renders egui to a
  PNG in a test (see `hole_dialog::tests::shots`, run with
  `SHOT_DIR=<dir> cargo test <name> -- --ignored`).
- The egui UI font has **no arrows (→ ↑) and no ✓** — they draw as boxes.
  Use words or ">" in UI text, and draw ticks with lines. · — × Ø ° … ² are fine.
- The file pickers (rfd, xdg portal) can't be driven by ctl; use `open` /
  `import` instead.

## Commands

```
cargo run                      # the app
cargo test                     # all tests (currently ~480)
cargo test kernel              # one module's tests
cargo test -- --nocapture NAME # see prints from one test
packaging/flatpak/build.sh     # Flatpak + dist/Oxide360.flatpak (see docs/PROJECT.md)
```
System packages already installed: `opencascade-devel cmake gcc-c++ gh
flatpak-builder`. Rust 1.98 from Fedora (edition 2024). `gh` is logged in as
Defanoux.

## Architecture

Bigger picture: the **document** (`doc.rs`) is plain data — sketches and
features in a **timeline**. `kernel::Build` evaluates the timeline into
OpenCASCADE solids a little each frame (per-feature errors, never a crash),
`kernel::Model` is the result (bodies with meshes, edges, faces). The app
keeps two kernel caches (`Slot::Committed`, `Slot::Preview` — the open
dialog's feature added) in `app/document.rs`; `ensure_model` rebuilds only
when its inputs changed (`KernelCache` compares every feature list — add new
lists there).

| file | owns |
|---|---|
| `main.rs` | the `OpenCad` state struct (one per tab), `Tabs`, small enums (`Cmd`, `Pending`, `TabRequest`, `EditTarget`, `HelpPage`, `ConstructPick`), constants (`APP_ID`, colours), `eframe::App::ui`, `main` (also `ctl` / `mcp` / file argument) |
| `app/document.rs` | files (open / save / recent / insert / export / sketch DXF-SVG), undo, kernel caches + `inputs()`, autosave, `is_dirty`, body names/colours/materials |
| `app/sketching.rs` | starting, editing, finishing sketches; sketch tool actions |
| `app/features.rs` | every solid dialog's panel + commit + delete (extrude, revolve, patterns, planes, axes, points, fillet, hole, thread, shell, press pull, combine, move, sweep family, import, joint, interference, properties) and drag arrows |
| `app/panels.rs` | title bar (File menu, Help), toolbar (`tool_group`, `run_sketch_tool`, `run_solid_tool` — tools start by name), browser (bodies, components, sketches, construction), settings, help window |
| `app/menus.rs` | right-click marking menu, S-key Design Shortcuts box, `tool_icon` |
| `app/timeline.rs` | timeline bar, renaming, Measure, `free_view`, `corner_under`, `face_snap_under` |
| `app/viewport.rs` | the 3D / sketch view: all input (picking per tool/dialog) and drawing |
| `app/control.rs` | what the control link reports (`state`) and does (open/save/new) |
| `app/drawing.rs` | the DRAWING workspace: sheet on screen (pan/zoom/fit), its toolbar, Export PDF / DXF |
| `control.rs` | the control link: TCP JSON-lines server, `ctl` client, MCP server |
| `kernel.rs` | OpenCASCADE: faces from regions, every feature, booleans, `Build`/`Model`, edge/face references found again each build, imports, joints (`apply_joint`, `resolve_snap`), `face_snaps`, `interference`, `exact_bounds`, `body_properties` |
| `model.rs` | feature structs (`Extrude`, `Revolve`, `EdgeFeature`, `HoleFeature`, `SweepFeature`, `ImportFeature` + `Blob`, `JointFeature`/`JointSnap`/`JointKind`, `ConstructionPoint`, …), `Mesh`, `EdgeRef`, `Follow`, `ray_hit` |
| `doc.rs` | `Document` (everything saved), `Feature` enum (timeline entries), `BodyId` (+ renumbering on delete), JSON save/load + validation (old files must keep loading — new fields get `#[serde(default)]`), `History` (undo/redo) |
| `sketch.rs` | 2-D model: entities, constraints, `Tool`, `Editor` state machine, hit tests, delete + renumbering, text, projections |
| `solver.rs` | Newton–Gauss constraint solver (`add_constraints` rejects redundant ones) |
| `profile.rs` | closed regions of a sketch (what Extrude picks) |
| `project.rs` | projecting body edges into a sketch, linked so they follow |
| `spline.rs`, `spline_edit.rs`, `offset.rs`, `pattern.rs`, `rect_input.rs`, `text.rs` | sketch geometry helpers |
| `camera.rs`, `view.rs`, `gpu.rs`, `icons.rs`, `origin.rs`, `colors.rs`, `handles.rs`, `gizmo.rs`, `inertia.rs` | camera, drawing helpers, wgpu renderer, vector icons (no image files for icons), origin planes, body colours, drag arrows, move arrows+squares, toolbar scrolling |
| `*_dialog.rs` | one per dialog: its state and egui panel, logic testable without the app |
| `measure.rs`, `materials.rs`, `appearance.rs`, `assembly.rs` | Measure tool, physical materials + mass, appearances (looks), components |
| `units.rs`, `expr.rs`, `params_window.rs`, `threads.rs`, `keys.rs`, `settings.rs`, `shortcuts.rs`, `help.rs` | units + parsing, parameters/expressions, thread tables, key bindings, preferences file, tool names for menus/search, Help text + app icon |
| `export.rs`, `sketch_export.rs`, `recovery.rs` | STL/3MF/OBJ export, sketch DXF/SVG, autosave + crash recovery |
| `sketch_palette.rs` | the Sketch Palette (show switches, Slice) |
| `sketch_import.rs` | Insert SVG / DXF: reading drawings into sketch curves |
| `print3d.rs` | Print Check: printers, overhangs, fit, best orientations (`kernel::lay_flat` turns it) |
| `drawing.rs`, `drawing_export.rs` | 2-D drawing sheet (settings, first-angle layout, scales, title block) from `kernel::hlr_view` lines; PDF + DXF writers |
| `ui_shot.rs` | test-only headless UI screenshots |
| `vendor/opencascade-sys`, `vendor/opencascade` | **patched** OCCT bindings — see below |
| `packaging/` | `flatpak/` (manifest, desktop, metainfo, MIME, build script), `make-bundle.sh` (plain tarball) |
| `assets/` | the logo (`oxide360.svg`, and a 256 px PNG used as the window icon) |

## Rules that keep it from breaking

- **OCCT exceptions abort the process.** They aren't `std::exception`, so they
  can't cross the cxx bridge. Any new OCCT call that can fail gets a C++
  wrapper with `try { OCC_CATCH_SIGNALS … } catch (...) { return false/nullptr; }`
  in `vendor/opencascade-sys/include/*.hxx`, a declaration in the matching
  `src/*.rs` bridge, and a `try_*` method in `vendor/opencascade` returning
  `Option`. Also wrap kernel calls in `catch_unwind`, as `build()` does.
  (The vendor crate uses a different glam version: convert with `kv()` /
  component-wise.)
- **Never write derived data back into the document.** Face references store
  the plane *as picked* (+ `follow`); the kernel re-resolves them every build.
- **Undo** (`History`) records the document once per frame when the mouse is
  up and no text box is focused. Anything that changes the document every
  frame (drags) must be gated so it's one step.
- **Kernel cache**: every feature list must be compared in `ensure_model`
  (a missing one meant edits didn't rebuild). Sketches compare with
  `Sketch::same_shape` so view-only state doesn't rebuild.
- **Indices shift on delete.** Deleting an entity/constraint/feature renumbers
  everything after it. Use the existing `delete_*` functions,
  `BodyId::after_delete` / `after_maker_delete` (+ `shift_body_keys` /
  `shift_maker_keys`, which also fix names, colours, materials, components),
  `retain_constraints`, `Follow::after_delete`. Clear selections that hold
  indices.
- **A new timeline feature** touches: `doc::Feature` + validation,
  `Document` field, `BuildInputs` + `Build` + `Model` errors, the kernel step,
  `KernelCache`, `inputs()` (preview), `feature_label`, `feature_error`,
  `feature_name_mut`, timeline name + icon, delete, double-click edit,
  `is_dirty`, both `reset_with` paths. Grep an existing one (`Joint`) and copy.
- **Redundant constraints**: add new constraints through
  `solver::add_constraints`, one at a time when some may be redundant.
- Coordinates: Z-up, millimetres; f64 for sketch/kernel geometry, f32 for
  rendering. Front faces −Y; home view is front-right-top.
- **Linux limitation**: winit doesn't deliver touchpad pinch on Linux.
  Touchpad mode zooms with Ctrl + two-finger scroll. Don't try to "fix" pinch.
- Don't add new heavy dependencies or network access without a good reason.

## Where new code goes

New logic goes in a testable module. `app/*.rs` only wires input → those
functions → drawing; pick the file by topic and add `pub(crate)` to methods
other files call.
