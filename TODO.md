# opencad TODO

Work top to bottom. Move finished items to **Done** with a one-line note.

## Next

Ordered by how much each makes opencad a usable part modeller (FEATURES.md
has the full Fusion list; this is the slice to do now). Fusion's behaviour is
the spec for each.

**Core solid tools (the gaps you hit modelling a real part)**

**Sketch gaps**
12. **Sketch Text** (font outlines → profiles, extrudable).

**Viewing / selection**
13. **Zoom to fit** (F6 / double-middle-click) and named views (Front, Top,
    Right, Home) on the view cube's menu.
14. **Display modes** — shaded, shaded + edges, wireframe; **Section
    Analysis** (a clipping plane).
15. **Vertex picking** in 3D and **box select** of bodies/faces.

**Construction geometry**
16. **Midplane**, **Plane through 3 points**, **Tangent plane**; **Axis
    through 2 points / through a cylinder**; **Construction point**.

**Shapes Extrude/Revolve can't make**
17. **Sweep** (profile along a path) and **Loft** (between profiles, rails).
18. **Coil** / **Pipe**.

**Files**
19. **Autosave + crash recovery**.
20. **Import** STEP and STL (as bodies), **DXF/SVG export** of a sketch.

**Inspect**
21. Centre of mass, bounding box, **physical materials** (density → mass),
    **Interference**.

Later (big, after the above): components + joints (assemblies), 2D drawings,
sheet metal, surfaces, CAM.

## Polish (do when nearby code is touched)

- New-circle diameter box closes silently on click-away — add a hint.
- Tangent-tool error hints in red, not the preview colour.
- Export only visible bodies (or ask).
- Old files' edge/face references lack `faces` / `follow`; re-derive them on
  load from the built model.

## Done

- Redefine Sketch Plane (right-click a sketch in the browser or timeline): click an origin plane, construction plane or flat face; the sketch keeps its own coordinates there and what's built from it follows; a face that doesn't exist yet at the sketch's place in the timeline is refused (put back with a message).

- Break (sketch MODIFY): lines, circles (two crossings), arcs and splines split where crossed, every piece kept, dots show the split points; Trim, Extend and Offset on splines (pieces respline through the cut and their fit points — close to, not exactly, the original near a cut); lines extend to splines/ellipses, circles are cut by them. Not done: Offset keeping a spline copy dimensioned.

- Dimension lines no longer block picking the sketch line behind them (only the number box does); with the Dimension tool on, clicking another dimension's number edits it.

- Tangent Arc (arc menu; from a line or arc end, Tangent constraint to a line) and the Line tool's drag-off-the-end arc; Ellipse (CREATE): centre, axis end, width; construction axis lines to dimension, axes kept square by the solver; exact OCCT ellipse edges (whole, or the stretch between crossings, corners moved onto the true curve); exact-volume tests. Not done: Trim/Extend/Offset on ellipses, ellipse arcs.

- Symmetric constraint (CONSTRAINTS): two points, two lines (ends paired by nearest mirror image; a shared end goes onto the mirror) or two circles/arcs (centres + Equal), then the line or origin axis to mirror about; Sym glyph at the points; solver test.

- User parameters: expressions with units and functions in every length/angle box (expr.rs); Change Parameters window (fx, in MODIFY and the sketch MODIFY) with rename-follows-through and delete-while-used refused; name suggestions under number boxes (Tab/Enter/click); dimensions and the main feature values (extrude/Press Pull distance, revolve angle, fillet size, offset plane distance/angle) follow parameters (fx:, red when broken). Not linked yet: dialog spin boxes (shell walls, hole sizes, move distances, pattern counts).

- Move/Copy (M; MODIFY): bodies from the view or browser; Free Move (X/Y/Z distances and angles about the bodies' box middle, coloured arrows and rings to drag) or Rotate (origin or construction axis, angle); Create Copy makes new bodies (renumbered when the Move is deleted). Timeline feature; tests check the moved boxes. Not done: Point to Point, faces/sketches as objects.

- Combine (MODIFY, after Shell): Target Body, Tool Bodies, Join / Cut / Intersect, Keep Tools; the target keeps its identity, used tools disappear; bodies selected in the browser fill it; exact-volume tests. Not done: editing a Combine can't re-pick a tool it already used up (the × and the names still work).

- Press Pull (Q) / Offset Face (MODIFY, first): click flat faces, type or drag the distance (out is +), the faces round them stretch to meet (OCCT BRepOffset_MakeOffset per-face offsets, new exception-safe wrapper; exact-volume tests). Press Pull on a fillet face opens that Fillet (matched by radius and edge position), on a sketch profile starts Extrude with it. Not done: curved faces (hole walls), dragging a fillet radius in place.

- Small fixes: × clears a selection field (Shell, Thread, Fillet/Chamfer, Extrude/Revolve); a new Extrude/Revolve picks the only profile when the shown sketches have just one; a segfault inside OCCT (thin-wall fillet) is caught as an error (OCC_CATCH_SIGNALS).

- Fillet / Chamfer: Tangent Chain (on by default: one click takes the edges running smoothly on), clicking a face takes all its edges, chamfer types Equal / Two distances / Distance and angle (new exception-safe OCCT AddDA wrapper; exact-volume test).
- Shell (MODIFY): click flat faces to open and/or bodies to hollow closed; Inside / Outside / Both with their thicknesses; Sharp or Rounded offset corners; wall arrow; picked faces found again every build. New exception-safe OCCT wrappers (thick solid, offset shape); a too-thick wall is an error (OCCT returns the part unchanged). Also: rebuilds run a little per frame with a green progress bar; Model all / All cosmetic threads; exports named with the version. Not done: Tangent Chain, opening curved faces.
- Hole (H, CREATE): places = sketch points, sketch circles (hole takes the circle's Ø), face points or face centres (snap), several per hole; Simple / Counterbore / Countersink; Distance / To (plane or face) / All; Flat or Angle tip; tap type Simple / Clearance (ISO 273 / drill fits) / Tapped / Taper Tapped (NPT, 1:16) from `threads.rs` (ISO coarse+fine, UNC/UNF, NPT); counterbore/countersink follow the screw size; Flip; depth arrow. Tapped threads cosmetic (helix drawn) or modeled (cut). Thread (CREATE): cylindrical faces (shafts or holes), modeled or shown, full length or offset+length, size fits the face, class, hand, Remember Size. Modeled threads: exception-safe OCCT helical sweep in half-turn pieces (one long face made OCCT's booleans fail silently); every modeled cut is volume-checked. Not done: Fusion's At-Point edge References, Objects to Cut, modeled taper threads (Fusion doesn't model those either).
- Rectangle sizes typed while drawing (2-point and centre, `rect_input.rs`): after the first corner, typing fills the width box, Tab the height; typed sizes hold while the cursor picks the side, the click (or Enter) places it and each typed size becomes a dimension; nothing typed = free size, no dimensions; Esc drops what was typed.
- Plane at Angle (CONSTRUCT): click a sketch line, a straight body edge, a construction axis or an origin axis, then type or drag the round arrow; `PlaneFeature.angle = PlaneAngle { hinge, degrees }`, the kernel finds the line again every build (edges/sketches move it), 0° lies in the sketch plane / the edge's first face / XY (XZ for Z). Axes and sketches a plane turns about can't be deleted first. The dialog's Line field re-picks the line (angle kept).
- Offset Plane: the distance box opens focused next to the arrow (type straight away, Enter = OK); construction planes drawn orange like the origin planes; a plane gets hidden once a sketch, a To Object extrude or a mirror uses it.
- Extrude To Object (`Extent::ToObject`, `Extrude.target: ExtrudeTarget` = origin plane / construction plane / body face kept as picked and found again every build): the prism is cut by the target plane, so a slanted plane slants the end; errors for planes alongside or not reached; no taper or drag arrow. Splines close smoothly (periodic) when the first point is clicked again.
- Fit-point splines (`Entity::Spline` → `Sketch.splines`, fit points are ordinary sketch points; the curve is OpenCASCADE's own interpolated B-spline, sampled via new exception-safe wrappers `GeomAPI_Interpolate_try_curve` / `Geom_BSplineCurve_try_sample`, cached in `spline.rs`; profiles take spline runs as `Curve::Spline`, rebuilt by the kernel; deleting a fit point shortens it; lines trim at spline crossings). Not yet: trim / extend / offset of splines, tangent handles, (closed splines done). Trim works on circles (→ the arc left) and arcs (shortened or split, red arc preview). Tools drag into another order within their toolbar group; changed keybinds orange with a per-key reset.
- Keybinds (`keys.rs`, Settings → Set keybinds…: click a key, press the new one, Esc for none, reset to defaults; a taken key moves; Trim = X, Construction = K, Measure = I). Group ▾ lists show only tools not on the toolbar, full-width rows, ⋯ → Pin to toolbar; the ribbon's scrollbar is solid under the group names. Measure doesn't snap to edge middles. Tab rename selects the whole name.
- Browser drawn in the view's left strip (full height, scrolls only past it); toolbar favourites (sketch MODIFY and CONSTRAINTS: right-click a tool to add/remove, the rest under the group name's ▾; `Settings.toolbar_hidden`); tab rename (right-click), no unsaved dot; Measure in sketches uses the on-screen size for picking and leaves bodies out; buttons that add several constraints add them one by one (line collinear with an axis); line length follows the typing.
- Tabs: several designs in one window (`Tabs` holds an `OpenCad` per tab; × asks to save, + opens a new one, closing the window asks per unsaved tab; global settings carry across). Fixed top bar like Fusion's: File menu, Save, Undo/Redo with step lists, tabs with name + version (each save is a new version, `Document.version`, not a change), ⚙ Settings window (input device, touchpad zoom direction, units for new designs, camera, view cube, Measure precision). SOLID/SKETCH no longer scroll with the ribbon. Line: typed length kept while the cursor sets the angle. Touchpad zoom flipped (fingers up zoom in).
- Browser floats over the view's top left (rows in their own small boxes, sized to what's open, « / » to fold); sketch inference lines to line middles / circle centres; Line tool takes a typed length (any unit); Measure works in sketches; top-centre status shows the sketch, its degrees of freedom and the tool in use; Measure precision to 8 places.
- Browser like Fusion's: foldable (« / »), a root node for the design (part icon), folders Document Settings / Origin / Analysis (placeholder) / Construction / Bodies / Sketches with eye icons (folder eyes hide/show all). Timeline double-click (or right-click Edit) edits; Enter finishes tool windows; Measure follows the design's units ("Document" choice); dimensions show their unit and keep one typed into them (`Sketch.dim_units`); sketch selection details bottom right (length, diameter, circumference, area / perimeter of a closed selection).
- Units: each design has units (browser → Document Settings; saved as `Document.units`, old files mm; values always stored in mm), new designs start in the default (settings, "Make … the default"). Dimensions, number boxes, tool fields, readouts and the grid (inch fractions) use them; every length box takes "22 mm", "1 in", "0.5\"", "2 feet"…, angle boxes stay degrees (`units.rs`). Dragged sketch lines follow the cursor both ways keeping their constraints. Mirror starts with no plane.
- Mirror: click an origin or construction plane in the view (or pick it in the dialog) and click bodies there to add/drop them (`CopyKind::MirrorPlane`, exact kernel test); deleting a plane a mirror uses is refused. Not yet: mirror across a body face.
- Sketch: constraint glyphs are clickable labels (select, Shift adds, Delete / right-click Delete); point constraints (Fix, Coincident, On axis, Midpoint) show at their point; the right-hand constraint list is gone (as in Fusion) and the sketch name + degrees of freedom sit at the canvas's top left. Coincident / Collinear / Concentric constraints with icons; the sketch X/Y axes can be picked for constraints. Trim / Extend preview what a click does. Fillet/chamfer arrow points into the gap on inside corners.
- Browser bodies: click selects (Ctrl/Shift adds, tinted in the view, Esc clears); right-click Remove (Fusion's Remove: a timeline feature `Feature::Remove` / `RemoveFeature`, errors if its body is gone, follows feature deletes, pruned when empty; Delete key does this) or Delete (deletes the feature that made the body). Offset constraint is signed (keeps its side — fixed a folded offset). Sketch menu says "Look at Sketch"; Trim/Extend/Offset/Fillet/Chamfer return to Select after use.
- Measure to Fusion's reference (help.autodesk.com Measure): Selection Filter (Face/Edge/Vertex, Body — no components here), Precision 0–6, Units + Secondary units, XYZ Delta None/Global (corner pairs, coloured legs on canvas), Show Snap Points (edge ends/middles, circle and round-face centres; Ctrl locks, Shift hides), Clear selection; Selection 1 / 2 each list their fields (point XYZ; line length; arc length/radius/diameter/centre; circle circumference/…; face area + loop length (+ centre when round); cylinder/fillet radius/diameter/area/loop length; body none) plus Results (min distance, angle, centre distance); visible sketch points, curves and profiles (exact area/loop length) are measurable; hovering a result highlights it; distance search is box-pruned (two 7200-triangle meshes < 0.5 s debug).
- Offset constraint: offset copies stay attached (lines a set distance off, arcs/circles share the centre), one editable dimension per offset; editing it moves the copy, not the original.
- Extrude / revolve only pick profiles from visible sketches (plus the sketch of the feature being edited).
- Look At: right-click in a sketch, on an origin or construction plane, or a flat face; glides to face it.
- Measure like Fusion: hover/click corners, edges, faces (hover highlight, faces filled). One pick: corner XYZ; line length; circle circumference/diameter/radius; arc length/angle; flat face area + perimeter; cylinder/fillet face area + diameter/radius. Two picks: minimum distance + ΔX/Y/Z (dashed line between the closest points), angle, circle centre distance. Floating window (bottom right) with a unit and an optional second unit under each value (saved in settings); clicking a value copies it, and every number box accepts pasted units ("0.5 in", "90°"). Exact via the kernel (face areas, true circles); triangles tagged by face (`face_ends` patch in the vendored mesher). Body default colours rank among existing bodies (`colors::rank`).
- Timeline drag to reorder (blue bar shows the drop gap, refused if it breaks a feature; replaces Move earlier / later). Slot: construction centre line, type the width after the second centre. Offset: type the distance. Tool windows sit bottom-right above the timeline. Default body colours never repeat among existing bodies.
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
