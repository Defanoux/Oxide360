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
