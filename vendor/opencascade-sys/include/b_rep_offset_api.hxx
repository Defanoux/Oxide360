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
#include <Geom_ConicalSurface.hxx>
#include <Geom_CylindricalSurface.hxx>
#include <gp_Ax3.hxx>
#include <cmath>

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
    TopoDS_Edge helix;
    if (std::fabs(taper) < 1e-12) {
      Handle(Geom_CylindricalSurface) surface = new Geom_CylindricalSurface(frame, r_mid);
      // On the surface (u: angle, v: along the axis) a helix is a straight line.
      gp_Dir2d dir2(sign * 2.0 * M_PI, pitch);
      Handle(Geom2d_Line) line = new Geom2d_Line(gp_Pnt2d(0.0, 0.0), dir2);
      double span = turns * std::sqrt(4.0 * M_PI * M_PI + pitch * pitch);
      helix = BRepBuilderAPI_MakeEdge(line, surface, 0.0, span).Edge();
    } else {
      // A cone: v runs along its slant, so a turn climbs pitch / cos(angle) in v.
      double angle = std::atan(taper);
      Handle(Geom_ConicalSurface) surface = new Geom_ConicalSurface(frame, angle, r_mid);
      double dv = pitch / std::cos(angle);
      gp_Dir2d dir2(sign * 2.0 * M_PI, dv);
      Handle(Geom2d_Line) line = new Geom2d_Line(gp_Pnt2d(0.0, 0.0), dir2);
      double span = turns * std::sqrt(4.0 * M_PI * M_PI + dv * dv);
      helix = BRepBuilderAPI_MakeEdge(line, surface, 0.0, span).Edge();
    }
    BRepLib::BuildCurves3d(helix);
    TopoDS_Wire spine = BRepBuilderAPI_MakeWire(helix).Wire();
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
