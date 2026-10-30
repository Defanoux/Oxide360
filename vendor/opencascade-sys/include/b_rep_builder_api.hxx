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

// opencad patch: the shape scaled about (px, py, pz) by sx, sy, sz. Equal
// factors keep the exact geometry (a similarity); unequal ones go through
// BRepBuilderAPI_GTransform (curves become B-splines). nullptr on failure.
#include <gp_Trsf.hxx>
#include <gp_GTrsf.hxx>
#include <BRepTools.hxx>
inline std::unique_ptr<TopoDS_Shape> opencad_try_scale(const TopoDS_Shape &shape, double px, double py, double pz, double sx, double sy, double sz) {
  try {
    OCC_CATCH_SIGNALS
    if (sx == sy && sy == sz) {
      gp_Trsf t;
      t.SetScale(gp_Pnt(px, py, pz), sx);
      BRepBuilderAPI_Transform mk(shape, t, true);
      if (!mk.IsDone()) {
        return nullptr;
      }
      return std::unique_ptr<TopoDS_Shape>(new TopoDS_Shape(mk.Shape()));
    }
    gp_GTrsf g;
    g.SetValue(1, 1, sx);
    g.SetValue(2, 2, sy);
    g.SetValue(3, 3, sz);
    // The point stays put: x' = s x + (1 - s) p.
    g.SetTranslationPart(gp_XYZ((1.0 - sx) * px, (1.0 - sy) * py, (1.0 - sz) * pz));
    BRepBuilderAPI_GTransform mk(shape, g, true);
    if (!mk.IsDone()) {
      return nullptr;
    }
    // Faces can keep the old shape's triangulation; drop it so they're meshed again.
    TopoDS_Shape out = mk.Shape();
    BRepTools::Clean(out);
    return std::unique_ptr<TopoDS_Shape>(new TopoDS_Shape(out));
  } catch (...) {
    return nullptr;
  }
}

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
