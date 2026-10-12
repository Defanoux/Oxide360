# opencad TODO

Work top to bottom. Move finished items to **Done** with a one-line note.

## Next

7. **Sketch Mirror**, then **solid Mirror** and **Rectangular / Circular
   Pattern** (bodies and features).

8. **Project geometry** into a face sketch (body edges → construction lines).

9. **Sketch Extend / Offset**, then **Slot**, **Spline**. (Trim done for lines; trimming circles/arcs themselves not yet.)

10. **Timeline**: rename, suppress, drag to reorder (with error checking) (rollback marker is done);
    browser: select/delete bodies; **Measure** tool.

11. **Extrude options**: symmetric, two sides, taper angle, "to object".

12. **Split `main.rs`** into `app/*` modules (see CLAUDE.md) — do it earlier
    if any item above needs big changes there.

## Polish (do when nearby code is touched)

- On-canvas glyphs for OnAxis / Midpoint / Distance constraints.
- New-circle diameter box closes silently on click-away — add a hint.
- Dimension tool: clicking an existing dimension should select/edit it.
- Tangent-tool error hints in red, not the preview colour.
- Export only visible bodies (or ask).
- Old files' edge/face references lack `faces` / `follow`; re-derive them on
  load from the built model.

## Done

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
