# opencad — Feature Roadmap (Fusion 360 parity)

Target: Fedora 44 KDE, Rust, from scratch. Units: millimetres by default (metric), angles in degrees in the UI, radians internally.

## How to use this file (instructions for Claude Code)

- Work **one phase at a time**, in order. Later phases depend on earlier ones.
- Do not start a phase until the previous phase's **Done when** checks pass.
- Before each phase: write a short plan, list files you'll touch, then implement.
- Keep geometry/kernel code in a crate separate from UI code. UI must never contain modeling logic.
- Every feature is a **command** that can be undone/redone and is stored in the **timeline** (parametric history). Never mutate geometry directly from the UI.
- Add tests for anything in the kernel/solver/file format. UI can be tested manually.
- Tick the boxes below as features land. Note deviations at the bottom under **Decisions log**.

## Architecture (decide in Phase 0, then don't revisit)

| Layer | Responsibility |
|---|---|
| `kernel` | B-rep solids, booleans, fillets, shell, tessellation, STEP/STL I/O |
| `sketch` | 2D entities, constraints, solver, profile detection |
| `model` | Document, timeline/feature tree, parameters, undo/redo, rebuild engine |
| `render` | wgpu viewport, camera, picking, selection highlighting |
| `ui` | Panels, toolbar, browser tree, timeline bar, dialogs |
| `io` | Native file format, import/export |

**Decision needed (Phase 0):** modeling kernel.
- Option A: `opencascade-rs` (bindings to OpenCASCADE). Most complete (booleans, fillets, shell, STEP). Adds a C++ build dependency.
- Option B: `truck` (pure Rust B-rep). Cleaner build, but fillets/booleans/shell are less mature.
- Recommendation: A. Writing a robust B-rep kernel from scratch is a multi-year project on its own.

Suggested crates: `wgpu`, `winit`, `egui`/`eframe` (UI), `glam` or `nalgebra` (math), `serde` + `bincode`/`ron` (save format), `rfd` (file dialogs), `rayon` (parallel tessellation), `stl_io`.

---

## Phase 0 — Foundation
- [ ] Cargo workspace with the crates above
- [x] Window + wgpu surface + egui panels on Fedora (Wayland and X11)
- [x] Document struct, command trait, undo/redo stack — whole-document undo
- [ ] Unit system (mm default, switchable to cm/m/in)
- [x] Kernel chosen and a box can be created and tessellated — OpenCASCADE

**Done when:** app opens, shows a lit box, undo/redo works on a dummy command.

## Phase 1 — Viewport & navigation
- [ ] Orbit, pan, zoom (mouse + trackpad); zoom to fit / selection — orbit/pan/zoom done; zoom-to-fit not yet
- [x] Perspective and orthographic toggle
- [ ] ViewCube (click faces/edges/corners to snap view); standard views (Front, Top, Right, Home) — cube + Home done; named standard-view menu not yet
- [ ] Display modes: shaded, shaded + edges, wireframe, hidden-line
- [x] Origin planes/axes visible, grid on sketch plane
- [ ] Ground shadows / simple lighting, background colour
- [ ] Picking: hover highlight, click select faces/edges/vertices/bodies; box select; selection filters — hover/click faces, edges, planes; no vertex/box select in 3D
- [ ] Section analysis (clipping plane)

**Done when:** you can orbit a body and select a face, edge, and vertex individually.

## Phase 2 — Sketching (the core of everything)
**Entities**
- [ ] Line, polyline, rectangle (2-point, 3-point, centre), circle (centre, 2-pt, 3-pt, tangent), arc (3-pt, centre, tangent), ellipse — all but ellipse; polyline not separate
- [ ] Polygon, slot (centre-to-centre, overall), spline (fit point, control point), conic, point — polygon and point done; slot/spline/conic not yet
- [ ] Text, construction geometry toggle, mirror, circular/rectangular pattern — construction toggle and mirror done
- [ ] Fillet and chamfer (2D), trim, extend, break, offset, project/include (from 3D edges), intersect — fillet, chamfer, trim (lines) done; extend/break/offset/project/intersect not yet

**Constraints**
- [ ] Coincident, collinear, concentric, midpoint, fix/unfix, parallel, perpendicular, horizontal/vertical, tangent, equal, symmetric, curvature (smooth) — most done; collinear/concentric/symmetric/curvature not yet
- [x] Dimensions: distance, angle, radius, diameter; driven (reference) vs driving
- [ ] Dimensions accept expressions and named parameters (`width * 2 + 5`)
- [x] Constraint solver (Newton–Raphson / Levenberg–Marquardt on a sparse system); degrees-of-freedom count; fully-constrained colour feedback
- [x] Conflict/redundant constraint detection with a clear error
- [x] Drag under-constrained geometry live

**Sketch plane**
- [ ] Sketch on origin planes, planar faces, and construction planes — origin planes and faces done; construction planes not yet
- [x] Profile detection: closed loops → regions, with nested holes
- [ ] Sketch visibility, edit sketch, re-attach sketch to different plane/face — visibility done; re-attach not yet

**Done when:** you can draw a dimensioned, fully-constrained bracket outline with holes, change a dimension, and the sketch updates correctly.

## Phase 3 — Core solid features
- [ ] **Extrude** — distance, to object, to face, symmetric, two-sided, taper angle; new body / join / cut / intersect — distance + new/join/cut/intersect; symmetric/two-sided/taper/to-object not yet
- [ ] **Revolve** — full or partial angle, around axis/edge — partial angle + axis done
- [ ] **Hole** — simple, counterbore, countersink; tapped (cosmetic thread); depth, tip angle; place by sketch points or face
- [ ] **Fillet** — constant radius, variable radius, chain/tangent edges, rolling-ball setback — constant radius done
- [ ] **Chamfer** — equal distance, two distances, distance + angle — equal and two distances done; distance+angle not yet
- [ ] **Shell** — inside/outside, remove faces, uniform thickness
- [ ] **Draft** — pull direction, angle, by plane/parting line
- [ ] **Combine** — join, cut, intersect, keep tools
- [ ] **Mirror**, **Rectangular pattern**, **Circular pattern**, **Pattern along path** (features, bodies, or faces) — mirror + rectangular + circular done for bodies; path pattern, features/faces not yet
- [ ] **Move/Copy**, **Align**, **Scale** (uniform / non-uniform)
- [ ] **Split body**, **Split face**, **Delete face** (heal)
- [ ] **Press/Pull**, **Offset face**, **Replace face**
- [ ] **Thicken**, **Stitch/Unstitch**, **Boundary fill**
- [ ] **Rib** and **Web**, **Emboss** (text on face), **Thread** (modeled)
- [ ] **Rule fillet**, **Remove** (delete feature without breaking history)

**Done when:** you can model a bracket with holes, fillets, a shell, and a pattern, and each step appears in the timeline.

## Phase 4 — Construction geometry
- [ ] Offset plane (done: from origin planes and faces), plane at angle, tangent plane, midplane, plane through 3 points, plane along path
- [ ] Axis through two points, along edge, perpendicular to face, through cylinder/cone/torus — along a straight edge done
- [ ] Point at vertex, at edge/plane intersection, centre of circle/sphere, along path
- [ ] Visibility toggles in the browser

## Phase 5 — Parametric history & parameters
- [ ] **Timeline** bar: roll back/forward, drag to reorder (with validity check), suspend, delete, edit feature — rollback + edit + delete done; reorder/suspend not yet
- [ ] **Browser tree**: bodies, sketches, planes, origin; rename, show/hide, isolate — bodies, sketches, origin, rename, show/hide, colours done
- [ ] **User parameters** table (name, expression, unit, comment); favourites
- [ ] Parameter expressions with units and functions (`sin`, `sqrt`, `min`, `max`, `if`)
- [ ] Rebuild engine: dependency graph, recompute downstream features on edit
- [x] Failure handling: mark broken features, keep the rest of the model, show reason
- [ ] **Direct modeling** (non-parametric) edits when history is off; capture design history toggle
- [ ] Persistent face/edge naming so features survive upstream edits (hardest problem here; plan it early)

**Done when:** changing a parameter rebuilds the whole model, and a broken feature doesn't crash the app.

## Phase 6 — Assemblies
- [ ] Components (parts) vs bodies; nested components; activate component
- [ ] **Joints**: rigid, revolute, slider, cylindrical, pin-slot, planar, ball
- [ ] Joint origins on faces/edges/vertices; limits and motion preview
- [ ] As-built joints, joint drive, motion study (basic)
- [ ] **Contact sets**, interference check
- [ ] Ground component, align, exploded view
- [ ] External references (link another document), update on change
- [ ] Bill of materials, measure between components

## Phase 7 — Inspect & analyze
- [ ] **Measure**: distance, angle, area, volume, centre of mass, bounding box
- [ ] Interference, section analysis, curvature comb, zebra analysis, draft analysis, accessibility analysis
- [ ] Physical **materials** (density) and appearance (colour, metallic, roughness) — body colours only
- [ ] Mass properties report

## Phase 8 — Surface & sculpt (advanced)
**Surface**
- [ ] Extrude/Revolve surface, **Sweep**, **Loft** (guide rails, centreline), **Patch**, **Ruled**, **Offset**
- [ ] Trim, extend, stitch, unstitch, thicken, reverse normal
- [ ] Surface fillet, blend, merge

**Solid sweep/loft**
- [ ] **Sweep** (path, guide rail, orientation modes, taper, twist)
- [ ] **Loft** (sections, rails, centreline, tangent/curvature end conditions)
- [ ] **Coil** (spring/thread), **Pipe**

**Form/T-spline (optional, very hard — defer)**
- [ ] Subdivision modeling, box/quad edit, convert to B-rep

## Phase 9 — Import / export
- [x] Native save/load (versioned format, diff-friendly: RON or JSON with binary blobs)
- [ ] Autosave and crash recovery
- [ ] **Export:** STEP, STL (binary + ASCII), 3MF, OBJ, IGES, DXF (sketch), SVG (sketch) — STEP, STL, 3MF, OBJ done
- [ ] **Import:** STEP, STL, OBJ, DXF, SVG
- [ ] Insert canvas/image reference on a plane (calibrate by two points)

## Phase 10 — 2D drawings
- [ ] Drawing sheets (ISO/ANSI sizes), title block
- [ ] Base view, projected, section, detail, auxiliary, broken views
- [ ] Dimensions, hole/thread callouts, centre marks/centerlines, GD&T symbols, notes, balloons, tables
- [ ] Export to PDF / DXF

## Phase 11 — Manufacture & 3D printing
**Print**
- [ ] Printability checks: overhang, wall thickness, manifold/watertight check
- [ ] Orientation, supports (basic), export to slicer (STL/3MF)

**CAM (long-term)**
- [ ] Stock setup, work coordinate system
- [ ] 2D toolpaths: face, contour, pocket, adaptive, drill, bore
- [ ] 3D toolpaths: parallel, pencil, scallop
- [ ] Tool library, feeds & speeds, simulation, G-code post-processor (Grbl/Marlin first)

## Phase 12 — Simulation (long-term / optional)
- [ ] Static stress (linear FEA): mesh, loads, constraints, results visualisation
- [ ] Modal frequencies, thermal, buckling
- [ ] Shape optimization (very late)

## Phase 13 — Generative design / automation (optional)
- [ ] Scripting API (Rhai or embedded Python) over the command system
- [ ] Macros: record and replay commands
- [ ] Plugin system

## Phase 14 — Polish & app-level
- [ ] Keyboard shortcuts (Fusion-compatible defaults: `L` line, `C` circle, `R` rectangle, `E` extrude, `F` fillet, `D` dimension, `M` move, `Q` press/pull) and remapping — several Fusion defaults present; no remapping
- [ ] Marking menu / right-click radial menu, search-to-run command palette
- [ ] Context-sensitive toolbar tabs: Solid, Surface, Sheet Metal, Plastic, Utilities
- [ ] Sheet metal: rules, flange, bend, unfold/flat pattern, export DXF
- [ ] Theme (light/dark), HiDPI, KDE integration (file picker, notifications)
- [ ] Preferences: units, default material, grid, navigation style (Fusion / Blender / SolidWorks-like)
- [ ] Packaging: Flatpak or RPM for Fedora

---

## Suggested build order if time is limited (minimum useful CAD)
1. Phase 0 → 1 → 2 → 3 (extrude, cut, hole, fillet)
2. Phase 5 (timeline + parameters)
3. Phase 9 (STEP/STL export)
4. Phase 4, then Phase 6

That gets you a working parametric part modeler. Everything else is additive.

## Decisions log
<!-- Claude Code: record kernel choice, solver choice, file format version, and any scope changes here -->
- Kernel: OpenCASCADE via patched vendored bindings, all calls exception-safe (`try_*`).
- Layout: one crate with modules (sketch, solver, profile, kernel, model, doc...) instead of a workspace.
- Solver: own Newton–Gauss with redundancy check. File format: JSON `.ocad`, new fields `#[serde(default)]`.
- Settings (touchpad, view-cube axes, projection) live in `~/.config/opencad/settings.json`; eframe persistence would need a new crate.
- Coplanar ambiguity: a body face lying in an origin (or future construction) plane wins over the plane for picking; the part of a plane covered by a body is hidden.
- Origin hides itself when the first body appears; show it again via the browser Origin box or Create Sketch.
- Origin planes sit set back from the axes so axes/origin stay pickable; polygons lock to quarter turns unless Ctrl is held; construction axes are stored as an edge reference and re-found on every rebuild.
- Tool dialogs are floating windows over the view's right side; the constraints list stays a side panel.
- Extrude operation defaults to Join only if the profile's prism touches an existing body, else New Body, until the user picks one.

