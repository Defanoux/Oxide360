#include <Standard_ErrorHandler.hxx>
#include <BRepOffsetAPI_MakeOffset.hxx>
#include <BRepOffsetAPI_MakePipe.hxx>
#include <BRepOffsetAPI_MakePipeShell.hxx>
#include <BRepOffsetAPI_MakeThickSolid.hxx>
#include <BRepOffsetAPI_ThruSections.hxx>
#include <Law_Function.hxx>
#include <TopTools_ListOfShape.hxx>
#include <TopoDS_Shape.hxx>
#include <bindings_common.hxx>

// opencad patch: a thread's solid (a groove to cut, or a ridge to add): the
// trapezoid profile in the axial plane (widths w_in at radius r_in, w_out at
// r_out, measured along the axis) swept along a helix of `pitch` round the
// axis through `origin` along `dir` (x_dir: where the helix starts), from
// 0 to `length` along the axis. `taper`: the radius change per unit along the
// axis (0 for straight threads; NPT narrows going in). Any OCCT failure gives
// nullptr instead of an exception (which would abort the app).
#include <BRepBuilderAPI_MakeEdge.hxx>
#include <BRepBuilderAPI_MakePolygon.hxx>
#include <BRepBuilderAPI_MakeWire.hxx>
#include <BRepCheck_Analyzer.hxx>
#include <BRep_Builder.hxx>
#include <TopoDS_Solid.hxx>
#include <BRepGProp.hxx>
#include <BRepLib.hxx>
#include <GProp_GProps.hxx>
#include <Geom2d_Line.hxx>
#include <Geom2d_TrimmedCurve.hxx>
#include <GCE2d_MakeSegment.hxx>
#include <Geom_Surface.hxx>
#include <Geom_ConicalSurface.hxx>
#include <Geom_CylindricalSurface.hxx>
#include <gp_Ax3.hxx>
#include <cmath>
#include <algorithm>

inline std::unique_ptr<TopoDS_Shape> opencad_try_thread(double ox, double oy, double oz, double dx, double dy, double dz, double xx, double xy, double xz,
                                                        double r_in, double r_out, double w_in, double w_out, double pitch, double length,
                                                        bool right_hand, double taper) {
  try {
    OCC_CATCH_SIGNALS
    if (!(r_in > 0.0 && r_out > r_in && w_in > 0.0 && w_out > 0.0 && pitch > 0.0 && length > 0.0)) {
      return nullptr;
    }
    gp_Ax3 frame(gp_Pnt(ox, oy, oz), gp_Dir(dx, dy, dz), gp_Dir(xx, xy, xz));
    // The helix runs through the middle of the profile.
    double r_mid = 0.5 * (r_in + r_out);
    double turns = length / pitch;
    double sign = right_hand ? 1.0 : -1.0;
    // The helix in pieces of at most `piece` turns: the sweep makes a face per
    // piece, and a face that winds round many times meets a cylinder along a
    // long helix OCCT's booleans can't follow; short pieces they can.
    const double piece = 0.5;
    Handle(Geom_Surface) surface;
    double dv = pitch; // how far along the surface's v one turn climbs
    if (std::fabs(taper) < 1e-12) {
      surface = new Geom_CylindricalSurface(frame, r_mid);
    } else {
      double angle = std::atan(taper);
      surface = new Geom_ConicalSurface(frame, angle, r_mid);
      dv = pitch / std::cos(angle);
    }
    // On the surface (u: angle, v: along the axis or slant) a helix is a straight line.
    gp_Dir2d dir2(sign * 2.0 * M_PI, dv);
    Handle(Geom2d_Line) line = new Geom2d_Line(gp_Pnt2d(0.0, 0.0), dir2);
    double per_turn = std::sqrt(4.0 * M_PI * M_PI + dv * dv);
    BRepBuilderAPI_MakeWire spine_maker;
    int count = std::max(1, int(std::ceil(turns / piece - 1e-9)));
    for (int i = 0; i < count; i++) {
      double t0 = per_turn * turns * double(i) / double(count);
      double t1 = per_turn * turns * double(i + 1) / double(count);
      TopoDS_Edge e = BRepBuilderAPI_MakeEdge(line, surface, t0, t1).Edge();
      BRepLib::BuildCurves3d(e, 1.0e-7, GeomAbs_C1, 14, 200);
      spine_maker.Add(e);
    }
    if (!spine_maker.IsDone()) {
      return nullptr;
    }
    TopoDS_Wire spine = spine_maker.Wire();
    // The profile, in the plane through the axis and the helix's start.
    gp_Vec x(frame.XDirection()), z(frame.Direction());
    gp_Pnt o = frame.Location();
    auto at = [&](double r, double h) { return o.Translated(x * r + z * h); };
    BRepBuilderAPI_MakePolygon poly(at(r_in, -w_in / 2.0), at(r_out, -w_out / 2.0), at(r_out, w_out / 2.0), at(r_in, w_in / 2.0), true);
    if (!poly.IsDone()) {
      return nullptr;
    }
    BRepOffsetAPI_MakePipeShell sweep(spine);
    sweep.SetMode(true); // Frenet: on a helix that's a rigid screw motion
    // Tight surfaces (the default 1e-4 leaves faces booleans can't intersect reliably).
    sweep.SetTolerance(1.0e-6, 1.0e-6, 1.0e-3);
    sweep.SetMaxDegree(14);
    sweep.SetMaxSegments(500);
    sweep.Add(poly.Wire(), false, false);
    sweep.Build();
    if (!sweep.IsDone() || !sweep.MakeSolid()) {
      return nullptr;
    }
    TopoDS_Shape solid = sweep.Shape();
    GProp_GProps props;
    BRepGProp::VolumeProperties(solid, props);
    if (props.Mass() < 0.0) {
      solid.Reverse();
    }
    if (!BRepCheck_Analyzer(solid).IsValid()) {
      return nullptr;
    }
    return std::unique_ptr<TopoDS_Shape>(new TopoDS_Shape(solid));
  } catch (...) {
    return nullptr;
  }
}

// opencad patch: Shell. A solid hollowed `offset` thick (negative: the wall
// goes inside), opened at the faces numbered `faces` (in TopExp_Explorer face
// order); corners sharp or rounded. nullptr on any failure.
#include <BRepOffsetAPI_MakeOffsetShape.hxx>
#include <TopExp_Explorer.hxx>
#include <TopoDS.hxx>
#include <vector>

inline bool opencad_good_solid(const TopoDS_Shape &s) {
  if (s.IsNull() || !BRepCheck_Analyzer(s).IsValid()) {
    return false;
  }
  GProp_GProps props;
  BRepGProp::VolumeProperties(s, props);
  return props.Mass() > 1e-9;
}

inline std::unique_ptr<TopoDS_Shape> opencad_try_thick_solid(const TopoDS_Shape &shape, rust::Slice<const int32_t> faces, double offset, bool rounded) {
  try {
    OCC_CATCH_SIGNALS
    std::vector<TopoDS_Shape> all;
    for (TopExp_Explorer ex(shape, TopAbs_FACE); ex.More(); ex.Next()) {
      all.push_back(ex.Current());
    }
    TopTools_ListOfShape closing;
    for (int32_t i : faces) {
      if (i < 0 || size_t(i) >= all.size()) {
        return nullptr;
      }
      closing.Append(all[size_t(i)]);
    }
    BRepOffsetAPI_MakeThickSolid maker;
    maker.MakeThickSolidByJoin(shape, closing, offset, 1.0e-5, BRepOffset_Skin, true, false, rounded ? GeomAbs_Arc : GeomAbs_Intersection);
    maker.Build();
    if (!maker.IsDone()) {
      return nullptr;
    }
    TopoDS_Shape out = maker.Shape();
    if (!opencad_good_solid(out)) {
      return nullptr;
    }
    return std::unique_ptr<TopoDS_Shape>(new TopoDS_Shape(out));
  } catch (...) {
    return nullptr;
  }
}

// opencad patch: the solid grown (or, negative, shrunk) by `offset` all round;
// corners sharp or rounded. nullptr on any failure.
inline std::unique_ptr<TopoDS_Shape> opencad_try_offset_shape(const TopoDS_Shape &shape, double offset, bool rounded) {
  try {
    OCC_CATCH_SIGNALS
    BRepOffsetAPI_MakeOffsetShape maker;
    maker.PerformByJoin(shape, offset, 1.0e-5, BRepOffset_Skin, true, false, rounded ? GeomAbs_Arc : GeomAbs_Intersection);
    if (!maker.IsDone()) {
      return nullptr;
    }
    TopoDS_Shape out = maker.Shape();
    // A solid's offset comes back as a shell sometimes: make it a solid.
    if (out.ShapeType() == TopAbs_SHELL) {
      BRep_Builder b;
      TopoDS_Solid solid;
      b.MakeSolid(solid);
      b.Add(solid, out);
      out = solid;
    }
    if (!opencad_good_solid(out)) {
      return nullptr;
    }
    return std::unique_ptr<TopoDS_Shape>(new TopoDS_Shape(out));
  } catch (...) {
    return nullptr;
  }
}

// opencad patch: Offset Face / Press Pull — move the faces picked (indices in
// TopExp_Explorer face order) along their normals by `offsets` (out is +),
// the faces round them stretching to meet. nullptr on any failure.
#include <BRepOffset_MakeOffset.hxx>
inline std::unique_ptr<TopoDS_Shape> opencad_try_offset_faces(const TopoDS_Shape &shape, rust::Slice<const int32_t> faces, rust::Slice<const double> offsets, bool intersection) {
  try {
    OCC_CATCH_SIGNALS
    if (faces.size() != offsets.size() || faces.size() == 0) {
      return nullptr;
    }
    std::vector<TopoDS_Shape> all;
    for (TopExp_Explorer ex(shape, TopAbs_FACE); ex.More(); ex.Next()) {
      all.push_back(ex.Current());
    }
    BRepOffset_MakeOffset maker;
    maker.Initialize(shape, 0.0, 1.0e-5, BRepOffset_Skin, intersection, false, GeomAbs_Intersection, false, false);
    for (size_t k = 0; k < faces.size(); ++k) {
      int32_t i = faces[k];
      if (i < 0 || size_t(i) >= all.size()) {
        return nullptr;
      }
      maker.SetOffsetOnFace(TopoDS::Face(all[size_t(i)]), offsets[k]);
    }
    maker.MakeOffsetShape();
    if (!maker.IsDone()) {
      return nullptr;
    }
    TopoDS_Shape out = maker.Shape();
    if (out.ShapeType() == TopAbs_SHELL) {
      BRep_Builder b;
      TopoDS_Solid solid;
      b.MakeSolid(solid);
      b.Add(solid, out);
      out = solid;
    }
    if (!opencad_good_solid(out)) {
      return nullptr;
    }
    return std::unique_ptr<TopoDS_Shape>(new TopoDS_Shape(out));
  } catch (...) {
    return nullptr;
  }
}

// opencad patch: Sweep. The face swept along the path wire (corrected
// Frenet, so it doesn't twist about the path). nullptr on any failure.
#include <GeomFill_Trihedron.hxx>
#include <TopoDS_Iterator.hxx>
inline std::unique_ptr<TopoDS_Shape> opencad_try_sweep(const TopoDS_Face &profile, const TopoDS_Wire &path) {
  try {
    OCC_CATCH_SIGNALS
    BRepOffsetAPI_MakePipe pipe(path, profile, GeomFill_IsCorrectedFrenet, false);
    pipe.Build();
    if (!pipe.IsDone()) {
      return nullptr;
    }
    TopoDS_Shape s = pipe.Shape();
    GProp_GProps props;
    BRepGProp::VolumeProperties(s, props);
    if (props.Mass() < 0.0) {
      s.Reverse();
    }
    if (!opencad_good_solid(s)) {
      return nullptr;
    }
    return std::unique_ptr<TopoDS_Shape>(new TopoDS_Shape(s));
  } catch (...) {
    return nullptr;
  }
}

// opencad patch: Loft through the closed wires in `sections` (a compound,
// in order); `ruled`: straight between sections, else smooth. nullptr on
// any failure.
inline std::unique_ptr<TopoDS_Shape> opencad_try_loft(const TopoDS_Shape &sections, bool ruled) {
  try {
    OCC_CATCH_SIGNALS
    BRepOffsetAPI_ThruSections loft(true, ruled);
    int n = 0;
    for (TopoDS_Iterator it(sections); it.More(); it.Next()) {
      if (it.Value().ShapeType() == TopAbs_WIRE) {
        loft.AddWire(TopoDS::Wire(it.Value()));
        n++;
      }
    }
    if (n < 2) {
      return nullptr;
    }
    loft.CheckCompatibility(true);
    loft.Build();
    if (!loft.IsDone()) {
      return nullptr;
    }
    TopoDS_Shape s = loft.Shape();
    GProp_GProps props;
    BRepGProp::VolumeProperties(s, props);
    if (props.Mass() < 0.0) {
      s.Reverse();
    }
    if (!opencad_good_solid(s)) {
      return nullptr;
    }
    return std::unique_ptr<TopoDS_Shape>(new TopoDS_Shape(s));
  } catch (...) {
    return nullptr;
  }
}

// opencad patch: Coil. A section (0 circle, 1 square, 2 triangle, `size`
// across) in the plane through the axis, `radius` out, swept round a helix
// of `pitch` for `turns` (Fusion's Coil). nullptr on any failure.
#include <GC_MakeCircle.hxx>
inline std::unique_ptr<TopoDS_Shape> opencad_try_coil(double ox, double oy, double oz, double dx, double dy, double dz, double xx, double xy, double xz,
                                                      double radius, double pitch, double turns, int section, double size) {
  try {
    OCC_CATCH_SIGNALS
    if (!(radius > 0.0 && pitch > 0.0 && turns > 0.0 && size > 0.0)) {
      return nullptr;
    }
    gp_Ax3 frame(gp_Pnt(ox, oy, oz), gp_Dir(dx, dy, dz), gp_Dir(xx, xy, xz));
    Handle(Geom_Surface) surface = new Geom_CylindricalSurface(frame, radius);
    gp_Dir2d dir2(2.0 * M_PI, pitch);
    Handle(Geom2d_Line) line = new Geom2d_Line(gp_Pnt2d(0.0, 0.0), dir2);
    double per_turn = std::sqrt(4.0 * M_PI * M_PI + pitch * pitch);
    BRepBuilderAPI_MakeWire spine_maker;
    const double piece = 0.5;
    int count = std::max(1, int(std::ceil(turns / piece - 1e-9)));
    for (int i = 0; i < count; i++) {
      double t0 = per_turn * turns * double(i) / double(count);
      double t1 = per_turn * turns * double(i + 1) / double(count);
      TopoDS_Edge e = BRepBuilderAPI_MakeEdge(line, surface, t0, t1).Edge();
      BRepLib::BuildCurves3d(e, 1.0e-7, GeomAbs_C1, 14, 200);
      spine_maker.Add(e);
    }
    if (!spine_maker.IsDone()) {
      return nullptr;
    }
    TopoDS_Wire spine = spine_maker.Wire();
    gp_Vec x(frame.XDirection()), z(frame.Direction());
    gp_Pnt o = frame.Location();
    auto at = [&](double r, double h) { return o.Translated(x * r + z * h); };
    TopoDS_Wire profile;
    if (section == 0) {
      gp_Ax2 ax(at(radius, 0.0), frame.YDirection(), frame.XDirection());
      Handle(Geom_Circle) c = new Geom_Circle(ax, size / 2.0);
      profile = BRepBuilderAPI_MakeWire(BRepBuilderAPI_MakeEdge(c).Edge()).Wire();
    } else if (section == 1) {
      double h = size / 2.0;
      profile = BRepBuilderAPI_MakePolygon(at(radius - h, -h), at(radius + h, -h), at(radius + h, h), at(radius - h, h), true).Wire();
    } else {
      double s3 = std::sqrt(3.0);
      profile = BRepBuilderAPI_MakePolygon(at(radius + size / s3, 0.0), at(radius - size / (2.0 * s3), size / 2.0), at(radius - size / (2.0 * s3), -size / 2.0), true).Wire();
    }
    BRepOffsetAPI_MakePipeShell sweep(spine);
    sweep.SetMode(true); // Frenet: on a helix that's a rigid screw motion
    sweep.SetTolerance(1.0e-6, 1.0e-6, 1.0e-3);
    sweep.SetMaxDegree(14);
    sweep.SetMaxSegments(500);
    sweep.Add(profile, false, false);
    sweep.Build();
    if (!sweep.IsDone() || !sweep.MakeSolid()) {
      return nullptr;
    }
    TopoDS_Shape solid = sweep.Shape();
    GProp_GProps props;
    BRepGProp::VolumeProperties(solid, props);
    if (props.Mass() < 0.0) {
      solid.Reverse();
    }
    if (!opencad_good_solid(solid)) {
      return nullptr;
    }
    return std::unique_ptr<TopoDS_Shape>(new TopoDS_Shape(solid));
  } catch (...) {
    return nullptr;
  }
}

// opencad patch: Emboss on a cylinder. `flat` is a planar face in the XY
// plane whose X is arc length round the cylinder and Y height along its axis
// (0, 0 where the profile touches it). It is wrapped onto the cylinder of
// `radius` round the axis through `origin` along `dir` (`anchor`: the way out
// from the axis to where X = 0), keeping lengths along the surface, then
// thickened radially by `thickness` (out is +). nullptr on any failure.
#include <BRepBuilderAPI_MakeFace.hxx>
#include <BRepTools_WireExplorer.hxx>
#include <BRep_Tool.hxx>
#include <GeomAPI.hxx>
#include <Geom2dConvert.hxx>
#include <Geom2d_BSplineCurve.hxx>
#include <Geom2d_TrimmedCurve.hxx>
#include <ShapeFix_Face.hxx>
#include <ShapeFix_Shape.hxx>
#include <gp_Pln.hxx>

inline std::unique_ptr<TopoDS_Shape> opencad_try_wrap_cylinder(const TopoDS_Shape &flat, double ox, double oy, double oz, double dx, double dy, double dz, double ax, double ay, double az,
                                                               double radius, double thickness) {
  try {
    OCC_CATCH_SIGNALS
    if (!(radius > 0.0) || std::fabs(thickness) < 1e-9) {
      return nullptr;
    }
    // The seam (u = 0) opposite the profile: it sits round u = pi.
    gp_Ax3 frame(gp_Pnt(ox, oy, oz), gp_Dir(dx, dy, dz), gp_Dir(-ax, -ay, -az));
    Handle(Geom_CylindricalSurface) surface = new Geom_CylindricalSurface(frame, radius);
    gp_Pln xy(gp::XOY());
    TopExp_Explorer fx(flat, TopAbs_FACE);
    if (!fx.More()) {
      return nullptr;
    }
    TopoDS_Face src = TopoDS::Face(fx.Current());
    TopoDS_Face face;
    bool first = true;
    for (TopExp_Explorer wx(src, TopAbs_WIRE); wx.More(); wx.Next()) {
      TopoDS_Wire w = TopoDS::Wire(wx.Current());
      BRepBuilderAPI_MakeWire wm;
      for (BRepTools_WireExplorer ex(w, src); ex.More(); ex.Next()) {
        TopoDS_Edge e = ex.Current();
        double f, l;
        Handle(Geom_Curve) c3 = BRep_Tool::Curve(e, f, l);
        if (c3.IsNull()) {
          return nullptr;
        }
        Handle(Geom2d_Curve) c2 = GeomAPI::To2d(c3, xy);
        Handle(Geom2d_BSplineCurve) bs = Geom2dConvert::CurveToBSplineCurve(new Geom2d_TrimmedCurve(c2, f, l));
        for (int i = 1; i <= bs->NbPoles(); i++) {
          gp_Pnt2d p = bs->Pole(i);
          double u = M_PI + p.X() / radius;
          if (u <= 0.0 || u >= 2.0 * M_PI) {
            return nullptr; // more than once round
          }
          bs->SetPole(i, gp_Pnt2d(u, p.Y()));
        }
        if (e.Orientation() == TopAbs_REVERSED) {
          bs->Reverse();
        }
        TopoDS_Edge on = BRepBuilderAPI_MakeEdge(bs, surface).Edge();
        BRepLib::BuildCurves3d(on, 1.0e-7, GeomAbs_C1, 14, 200);
        wm.Add(on);
        if (!wm.IsDone()) {
          return nullptr;
        }
      }
      if (first) {
        face = BRepBuilderAPI_MakeFace(surface, wm.Wire(), true).Face();
        first = false;
      } else {
        BRepBuilderAPI_MakeFace add(face);
        add.Add(wm.Wire());
        face = add.Face();
      }
    }
    if (first) {
      return nullptr;
    }
    ShapeFix_Face fix(face);
    fix.Perform();
    face = fix.Face();
    BRepOffset_MakeOffset maker;
    maker.Initialize(face, thickness, 1.0e-7, BRepOffset_Skin, false, false, GeomAbs_Arc, true);
    maker.MakeOffsetShape();
    if (!maker.IsDone()) {
      return nullptr;
    }
    TopoDS_Shape out = maker.Shape();
    if (out.ShapeType() == TopAbs_SHELL) {
      BRep_Builder b;
      TopoDS_Solid solid;
      b.MakeSolid(solid);
      b.Add(solid, out);
      out = solid;
    }
    GProp_GProps props;
    BRepGProp::VolumeProperties(out, props);
    if (props.Mass() < 0.0) {
      out.Reverse();
    }
    // Thickening towards the axis (raised inside a hole) can leave a solid
    // that doesn't check out; shape healing puts it right.
    if (!opencad_good_solid(out)) {
      ShapeFix_Shape fixer(out);
      fixer.Perform();
      out = fixer.Shape();
    }
    if (!opencad_good_solid(out)) {
      return nullptr;
    }
    return std::unique_ptr<TopoDS_Shape>(new TopoDS_Shape(out));
  } catch (...) {
    return nullptr;
  }
}
