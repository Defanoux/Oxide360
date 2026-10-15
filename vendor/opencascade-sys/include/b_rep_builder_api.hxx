#include <Standard_ErrorHandler.hxx>
#include <BRepBuilderAPI_GTransform.hxx>
#include <BRepBuilderAPI_MakeEdge.hxx>
#include <BRepBuilderAPI_MakeFace.hxx>
#include <BRepBuilderAPI_MakeShapeOnMesh.hxx>
#include <BRepBuilderAPI_MakeSolid.hxx>
#include <BRepBuilderAPI_MakeVertex.hxx>
#include <BRepBuilderAPI_MakeWire.hxx>
#include <BRepBuilderAPI_Transform.hxx>
#include <Geom2d_Curve.hxx>
#include <Geom_Curve.hxx>
#include <Geom_Surface.hxx>
#include <bindings_common.hxx>

// opencad patch: an ellipse edge (whole, or from parameter u1 to u2) in the
// frame at (cx, cy, cz) with normal n and its major axis along x. major >=
// minor. nullptr on any failure.
#include <gp_Elips.hxx>
#include <gp_Ax2.hxx>
inline std::unique_ptr<TopoDS_Edge> opencad_try_ellipse_edge(double cx, double cy, double cz, double nx, double ny, double nz, double xx, double xy, double xz, double major, double minor, double u1, double u2, bool whole) {
  try {
    OCC_CATCH_SIGNALS
    gp_Ax2 ax(gp_Pnt(cx, cy, cz), gp_Dir(nx, ny, nz), gp_Dir(xx, xy, xz));
    gp_Elips el(ax, major, minor);
    BRepBuilderAPI_MakeEdge mk = whole ? BRepBuilderAPI_MakeEdge(el) : BRepBuilderAPI_MakeEdge(el, u1, u2);
    if (!mk.IsDone()) {
      return nullptr;
    }
    return std::unique_ptr<TopoDS_Edge>(new TopoDS_Edge(mk.Edge()));
  } catch (...) {
    return nullptr;
  }
}
