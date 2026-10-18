# Oxide360 — the project so far

For whoever picks this up next (a person or another Claude). `CLAUDE.md` has
the working rules; this file has the background: what Gavin wants, what
exists, what was decided and why, and how to ship a release.

## What it is and who it's for

Gavin wanted Fusion 360 on Linux. Oxide360 is that, written from scratch in
Rust: a parametric modeller where every step lives in a timeline and can be
edited later, with exact B-rep geometry from OpenCASCADE (not meshes). He
uses Fusion daily and judges every behaviour against it — "match Fusion" is
the spec for anything not otherwise decided.

The name (chosen 2026-10-17): *Oxide* for Rust / oxidising, *360* as a nod to
Fusion 360. The logo (`assets/oxide360.svg`) is a rust-orange cube inside a
patina-green 360° ring with an arrow.

## How it was built

All of it by Claude Code sessions driven by Gavin, from 2026-10-11 on, one
TODO item at a time: write it, test it (exact-volume kernel tests), check it
in the running app through the control link, commit, tell Gavin numbered
things to try; he replies "1 works, 2 doesn't because …" with screenshots and
that becomes the next fix. Expect that rhythm.

Rough order of what landed (see `git log` and TODO.md's Done list for detail):

1. Sketching: lines, rectangles, circles, arcs, polygons, slots, splines,
   ellipses, text; constraint solver; Dimension tool; trim/extend/offset/
   fillet/chamfer/mirror/patterns; projection of body edges (linked).
2. Solids: extrude (to distance / object / all, taper, two sides), revolve,
   fillet/chamfer, booleans, holes (counterbore, countersink, tapped — ISO
   thread tables), threads (cosmetic or modelled), shell, press pull / offset
   face, combine, move/copy, mirror and patterns.
3. Construction planes/axes/points; parameters and expressions; timeline with
   rollback, suppress, reorder; undo/redo; tabs; units (mm default, metric).
4. View: GPU renderer (wgpu), view cube with roll arrows, named views, visual
   styles, section analysis, box select, measure, quick stats.
5. Sweep, loft, pipe, coil; autosave + crash recovery; STEP/STL import (with
   placement gizmo), STEP/STL/3MF/OBJ export, DXF/SVG sketch export.
6. Inspect: physical materials, mass properties, interference.
7. Assemblies (first steps): components, joints (rigid/revolute/slider,
   position + motion, animate, quarter turns), face snap points.
8. UI polish: right-click marking menu, S-key shortcuts box with search,
   File → Recent, Help (Getting Started, About with credits + library licences).
9. Shipping: renamed to Oxide360, GitHub (public, CI), Flatpak, release 0.1.0.

## Decisions already made (don't relitigate without a reason)

- **Kernel**: OpenCASCADE via patched `opencascade-rs` in `vendor/`. Every
  fallible OCCT call goes through a C++ `try/catch` wrapper (OCCT exceptions
  otherwise abort the process).
- **UI**: egui/eframe 0.36 + wgpu. Icons are drawn in code (`icons.rs`), not
  image files. The UI font lacks arrows and ✓.
- **Licence**: GPL-3.0-or-later (OpenCASCADE's LGPL-with-exception allows it).
- **Files**: `.ox360` JSON (`doc.rs`, `"format": "oxide360"`); old `.ocad` /
  `"opencad"` files must keep opening. New document fields always get
  `#[serde(default)]`.
- **Settings**: `~/.config/oxide360/settings.json` (copied over from
  `~/.config/opencad` once). Recovery files in `~/.local/share/oxide360/recovery`.
- **App id**: `io.github.Defanoux.Oxide360` — the Flatpak name, desktop file,
  icon name and Wayland app id must all match or KDE shows the wrong icon.
- **Privacy**: commits use Gavin's GitHub noreply address (history was
  rewritten before the first push to remove his personal Gmail). Public
  contact is `oxide360@defanoux.uk` (Cloudflare Email Routing → his Gmail,
  filtered out of his inbox) and Discord `Defanoux`. Never put his personal
  email anywhere.
- **Joints** are timeline features that move a whole component (Fusion-style),
  not a separate solver; the moving part's joint origin is drawn on top of the
  fixed one, axes aligned.
- **Big STL meshes** (over 50,000 triangles) are refused rather than made into
  solids (it takes minutes); Fusion keeps them as mesh bodies — not built yet.

## Shipping

### A release (version X.Y.Z)

1. Bump `version` in `Cargo.toml` and add a `<release version="X.Y.Z"
   date="…"/>` (newest first) to `packaging/flatpak/io.github.Defanoux.Oxide360.metainfo.xml`.
2. `cargo test`, commit, push.
3. `packaging/flatpak/build.sh` → `dist/Oxide360.flatpak` (the first build on
   a machine compiles OpenCASCADE, ~30–60 min; later ones a few minutes).
4. `git tag -a vX.Y.Z -m "Oxide360 X.Y.Z" && git push origin vX.Y.Z`
5. `gh release create vX.Y.Z dist/Oxide360.flatpak --title "Oxide360 X.Y.Z" --notes "…"`
   (0.1.0's notes are the template: install steps + what's new).

To try a built Flatpak: `flatpak install --user -y dist/Oxide360.flatpak`,
then `flatpak run io.github.Defanoux.Oxide360`. To drive it with the control
link add `--share=network` (the sandbox otherwise blocks even localhost) and
write screenshots under the home folder (its `/tmp` is private).

### Flathub — not possible as things stand (read before trying)

Flathub's requirements (docs.flathub.org/docs/for-app-authors/requirements,
"Generative AI policy", checked 2026-10-18) say: Flathub **manifests must not
contain AI-generated or AI-assisted content** (disclosure doesn't exempt
them); **AI tools must not open or automate the submission PR or write its
description, commits or replies**; and AI-generated code must be disclosed,
with reviewers free to reject based on how much there is. Oxide360 is almost
entirely written by Claude, so a Claude must not prepare or submit a Flathub
PR, and a submission by Gavin (hand-written manifest, full disclosure) would
likely be rejected. `packaging/flathub/` holds an offline-buildable manifest
(crates listed in `cargo-sources.json`, generated from `Cargo.lock` with
flatpak-cargo-generator.py) — usable for our own repository, **not** for a
Flathub PR.

Instead: our own Flatpak repository (see TODO.md) — built by CI on each
release tag, signed, published on GitHub Pages, installed from a
`.flatpakref`; Discover then shows updates from it like any other source.

## Where things stand

See `TODO.md` → Next. As of 2026-10-18: appearances, 2D drawings, joint
limits and motion settings, and our own Flatpak repository are the open items; sheet metal,
surfaces and CAM are the far future.
