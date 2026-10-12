# opencad TODO

Work top to bottom. Move finished items to **Done** with a one-line note.

## Next

5. **Persist settings**: touchpad/mouse choice and "show XYZ on view cube" in
   eframe storage; body/sketch visibility and the Home pose in the `.ocad`
   file (`#[serde(default)]`), excluded from undo/dirty comparison the way
   `label_shift` is excluded from `same_shape`.

6. **Body identity**: hidden-body checkboxes are by index and shift when
   features change. Name bodies after the feature that created them and
   track them through Join/Cut; browser shows real names; rename via
   right-click.

7. **Sketch Mirror**, then **solid Mirror** and **Rectangular / Circular
   Pattern** (bodies and features).

8. **Project geometry** into a face sketch (body edges → construction lines).

9. **Sketch Extend / Offset**, then **Polygon**, **Slot**, **Spline**. (Trim done for lines; trimming circles/arcs themselves not yet.)

10. **Timeline**: rename, suppress, drag to reorder (with error checking);
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
