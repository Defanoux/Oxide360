# opencad TODO

Work top to bottom. Move finished items to **Done** with a one-line note.

## Next

9. **Spline** (fit-point), and trimming circles/arcs themselves. (Extend, Offset, Slot done.)

10. **Timeline**: drag to reorder (the Move earlier / later menu items exist); browser: select/delete bodies; Measure faces/edges/angles (point-to-point exists).

11. **Extrude options**: "to object". (Symmetric, Two Sides, Taper done; taper is One Side only.)

## Polish (do when nearby code is touched)

- On-canvas glyphs for OnAxis / Midpoint / Distance constraints.
- New-circle diameter box closes silently on click-away — add a hint.
- Dimension tool: clicking an existing dimension should select/edit it.
- Tangent-tool error hints in red, not the preview colour.
- Export only visible bodies (or ask).
- Old files' edge/face references lack `faces` / `follow`; re-derive them on
  load from the built model.

## Done

- Extrude Taper (degrees, + narrows, − widens; One Side only): lofted between the profile and its offset, holes widen, exact frustum/cone volume tests; new exception-safe `tapered_prism_try` wrapper; `Face::normal_at` no longer aborts when a point has no projection.
- `main.rs` split into `app/{document,sketching,features,panels,timeline,viewport}.rs` (mechanical, no behaviour change; 804 lines left in main.rs).
- Timeline right-click: Rename…, Suppress / Unsuppress (greyed, not built, saved as `Document.suppressed`, follows deletes), Move earlier / later (refused when it would make something fail: `Model::error_count`). Measure tool (Inspect group): two clicks on the model, corners and edge middles snap, shows distance and Δx/Δy/Δz.
- Extrude Extent: One Side / Symmetric (whole length, half each way) / Two Sides (distance each way); `Extent` + `distance2` on `Extrude` (serde default), kernel `Extrude::span`, exact bbox/volume tests; the drag arrow follows the far end.
- Sketch Extend (line to the next line/circle/arc in its way, tied on or joined), Offset (a joined run of lines/arcs or a circle, side + distance from the cursor, live preview, click twice; `offset.rs`), Slot (centre-to-centre, tangent half circles, width dimension); sketch circular pattern's centre starts one Distance from the selection and the arrow drags it (no longer the origin).
- Sketch Circular Pattern has a second blue arrow (distance from the centre, outside the selection) and a Distance box; copies move outward, the original stays.
- N / the Create Sketch button starts a sketch on the plane you already selected (origin or construction), else asks you to click one; P = Project; projected lines are purple; dragging a line snaps it onto the grid (zoom-aware); polygons stay regular when dragged (circumscribed ones get a hidden circumcircle, drags walk in steps, no collapsing); circular pattern's distance arrow stands outside the bodies.
- Offset planes start at 0 mm and are drawn as a square a little bigger than the picked face; polygons get a Diameter dimension on their reference circle (3 DOF left); dragging a line slides it sideways along its normal (`solver::drag_line`); Project on a body takes all its projectable edges; Rectangular / Circular Pattern inside sketches (select geometry, press the button, live preview, blue arrows + × boxes, Enter applies, Esc cancels; constraints carry over; circular centre can be a clicked point).
- Floating tool windows (extrude, revolve, fillet/chamfer, patterns, offset plane) instead of a side panel; pattern axis is click-to-select; blue drag arrows for pattern spacing (both directions), angle and distance-from-axis plus × quantity boxes; Midpoint snap while drawing lines; Ctrl+G grid snap any time; Project (body edge → locked construction geometry; straight, circle, arc); revolve about a construction axis; Offset Plane (from an origin plane or a body face, follows the face) with sketches on it. Not yet: axis through two points / along a cylinder, mirror across a face, sketch patterns, patterning features or faces, planes from planes / through an axis.
- Origin planes set back from the axes (room to pick an axis later); Finish Sketch menu fix; Midpoint constraint (point + line); polygon direction locks to 0/90/180/270° (hold Ctrl to free it); double-click selects the joined chain; drag-select takes everything it touches plus the points in it; sketch Mirror works across origin axes and carries constraints over; bodies named Body1, Body2…; Construction Axis along a straight body edge (timeline feature `Feature::Axis`, used by Circular Pattern). Not yet: axis through two points / along a cylinder, revolve about a construction axis, construction planes.
- Sketch Mirror (select geometry, press Mirror, click the line; copies keep joints and get Equal constraints; `pattern.rs`). Solid Mirror (origin planes), Rectangular Pattern (two directions) and Circular Pattern (axis through the origin) as timeline features (`CopyFeature`, `Feature::Copy`, new bodies, dialog in `copy_dialog.rs`, exact kernel tests). Not yet: sketch patterns, mirroring across a face, patterning features instead of bodies. Also: Origin folder in the browser, depth-tested origin planes, Finish Sketch in the right-click menu, extrude toward the body = Cut.
- Camera glides (1 s) to face a new sketch's plane; ribbon scroll carries on after a flick (`inertia.rs`, slow drags don't glide); right-click a flat body face → Create Sketch; body face beats a coplanar origin plane; planes hidden where a body covers them; origin hides when the first body appears (Create Sketch / browser box show it); extrude defaults to New Body unless it touches an existing body (`model::prism_touches`). FEATURES.md ticked for what exists, decisions logged.
- Origin planes are selectable (click) with a right-click menu → Create Sketch; ribbon scrolls sideways; body double-click rename, per-body default colours + colour picker (right-click → Color…, saved as `body_colors`); timeline shows feature icons underlined in the colour of the body they worked on (`Model.touched`); Cut aims into the body automatically; sketch drag-select (left→right = inside, right→left = touching, Shift adds) and a right-click menu (Delete, Construction, driven toggle).
- Timeline rollback marker (Fusion's): drag the bar (or ⏮ ◀ ▶ ⏭) back along the timeline; later features stay but aren't built and are greyed (click one to roll forward to it). New features insert at the marker. Saved with the file (`ViewState.marker`).
- Body identity: every body has a `BodyId` (feature that made it + piece number) that survives Join/Cut/Intersect/fillets and feature deletes. Browser shows feature names ("Extrude1", "Extrude1 (2)" for split pieces), right-click → Rename (saved in the .ocad as `body_names`), hiding is by id, exports use the names.
- Fixed: editing a chamfer distance slid the rectangle corner (virtual sharp now pinned when a distance dimension is edited).
- Origin like Fusion: no grid in the 3D view (grid only in sketches); short axes + three small translucent origin planes (`origin.rs`), browser Origin checkbox. Create Sketch now = click the button, then click an origin plane or a flat face. Camera type (perspective / orthographic / ortho faces) is persisted in the settings file.
- Persisted settings: touchpad/mouse + view-cube axes in `~/.config/opencad/settings.json` (own small file — eframe storage needs a new crate); hidden bodies, sketch visibility and the Home pose saved in the `.ocad` (`Document.view`, `#[serde(default)]`, always compares equal so it's never an undo step or unsaved change).
- Solid chamfer with two distances: type `2, 5` in the size box (`EdgeFeature.size2`, serde default). First distance is on `EdgeRef.faces[0]`'s face. New patched OCCT wrapper `BRepFilletAPI_MakeChamfer_try_add_two` / `Shape::try_chamfer_edges_two`; exact volume d1·d2/2·L tested.
- Sketch fillet / chamfer: live preview while the size box is open, drag arrow (along the corner bisector), box stays open until Enter/Esc. Sketch chamfer takes two distances: type `2, 5` (first = first-picked line, or the lower-numbered line when clicking the corner point); dragging the arrow keeps the ratio.
- Drag arrows for solid fillet / chamfer (along the edge's face bisector, into the part) and revolve (swings round the axis from the profile), same grab/drag as the extrude arrow. Math in `handles.rs` (tested); main.rs has one generic `Handle`.
- Polygon tools (Circumscribed / Inscribed / Edge), 3–64 sides (toolbar box or ↑/↓): lines + construction reference circle, equal sides, corners on / edges tangent to the circle, 4 DOF left (centre, size, rotation).
- Sketch Trim (lines): click the stretch to remove; end pieces shorten to the cutting line/circle/arc (joined if it ends there, else tied on), middle pieces split the line, uncrossed lines are deleted.
- Construction geometry: per-entity flag (`Sketch.construction`, serde default), X key / toolbar toggle (selection → toggle it; nothing selected → construction mode for new geometry), dashed, ignored by `find_regions`, follows deletes. Center Rectangle now adds construction diagonals.
- Sketch fillet/chamfer also take two lines that meet (click both), same as clicking their corner.
- Driven (reference) dimensions: a dimension that would over-constrain is added greyed in (brackets), read-only, not solved, value follows the geometry; right-click a dimension toggles driving ⇄ driven. `Sketch.driven` is `#[serde(default)]`.
- 3-Point Arc snapping: raw third point (no grid), half-circle centre snap with ½ marker, grid-centre snap on the chord bisector. (Tangent-to-line start snap not done — it was optional.)
- Verified build: `cargo build` clean (no warnings), 151 tests pass.
- Orbit viewport, grid, view cube (with home, flip-to-opposite, axis triad toggle).
- Sketching: line, 2-point / 3-point / center rectangle, center / 2-point /
  3-point / 2-tangent / 3-tangent circle, 3-point and center-point arc, point.
- Constraints (tool-first or selection-first), Dimension tool (length,
  diameter, radius, angle, distance), movable / selectable / deletable
  dimension labels, ties to lines and origin axes, joint dots.
- Sketch fillet / chamfer with virtual sharp.
- Solver with redundancy check and stiffness-weighted dragging.
- Extrude, revolve, fillet, chamfer (OCCT, exact), Join/Cut/Intersect,
  sketch on face (follows its face), end-cap following for extrude/revolve.
- Timeline with errors, undo/redo for everything, save/open, STEP/3MF/STL/OBJ.
- Touchpad mode, zoom toward cursor, notifications.
