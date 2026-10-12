# opencad TODO

Work top to bottom. Move finished items to **Done** with a one-line note.

## Next

2. **3-Point Arc snapping** (Gavin's request, match Fusion):
   - The third click (the point the arc passes through) must **not** be grid
     snapped — use the raw cursor. (Same rule the 2-tangent circle already
     follows.)
   - **Half-circle snap**: when the arc is close to a semicircle (centre near
     the midpoint of the chord between the first two clicks), snap so the
     centre is exactly the chord midpoint. Show a small marker / "½" hint.
   - **Grid-centre snap** (only while grid snap is on): the centre always lies
     on the perpendicular bisector of the chord; if the centre comes within a
     few pixels of a grid intersection along that bisector, snap it there.
   - Optional, Fusion does it: if the arc starts at the end of a line, snap
     to "tangent to that line" when close, and add a Tangent constraint.
   - Tests: semicircle snap gives centre = chord midpoint exactly; grid snap
     gives a centre on a grid point and on the bisector; raw third point is
     unchanged when no snap applies.

3. **Driven (reference) dimensions**: a dimension that would over-constrain
   the sketch should be offered as *driven* (shown, greyed, read-only, not
   solved) instead of being refused as Redundant. Add a `driven` flag (keep
   old files loading), skip driven ones in `solver::equations`, draw them in
   a muted colour with the value in parentheses, and update their value from
   the geometry each frame. Right-click (or a panel toggle) switches
   driving ⇄ driven.

4. **Construction geometry** (Fusion's X key): a per-entity `construction`
   flag. Construction lines/circles/arcs draw dashed, are ignored by
   `find_regions`, still take constraints. Toolbar toggle + X shortcut.
   Then make the Center Rectangle use construction diagonals like Fusion.

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

9. **Sketch Trim / Extend / Offset**, then **Polygon**, **Slot**, **Spline**.

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
