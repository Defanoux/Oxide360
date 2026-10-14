#include <BRepFilletAPI_MakeChamfer.hxx>
#include <BRepFilletAPI_MakeFillet.hxx>
#include <BRepFilletAPI_MakeFillet2d.hxx>
#include <bindings_common.hxx>

inline std::unique_ptr<TopoDS_Edge> BRepFilletAPI_MakeFillet2d_add_fillet(BRepFilletAPI_MakeFillet2d &make_fillet,
                                                                          const TopoDS_Vertex &vertex,
                                                                          Standard_Real radius) {
  return std::unique_ptr<TopoDS_Edge>(new TopoDS_Edge(make_fillet.AddFillet(vertex, radius)));
}

// Chamfers
inline std::unique_ptr<TopoDS_Edge>
BRepFilletAPI_MakeFillet2d_add_chamfer(BRepFilletAPI_MakeFillet2d &make_fillet, const TopoDS_Edge &edge1,
                                       const TopoDS_Edge &edge2, const Standard_Real dist1, const Standard_Real dist2) {
  return std::unique_ptr<TopoDS_Edge>(new TopoDS_Edge(make_fillet.AddChamfer(edge1, edge2, dist1, dist2)));
}

inline std::unique_ptr<TopoDS_Edge>
BRepFilletAPI_MakeFillet2d_add_chamfer_angle(BRepFilletAPI_MakeFillet2d &make_fillet, const TopoDS_Edge &edge,
                                             const TopoDS_Vertex &vertex, const Standard_Real dist,
                                             const Standard_Real angle) {
  return std::unique_ptr<TopoDS_Edge>(new TopoDS_Edge(make_fillet.AddChamfer(edge, vertex, dist, angle)));
}

// opencad patch: Build() throws OCCT exceptions (which are not std::exception)
// when a fillet or chamfer is impossible, e.g. a radius bigger than the face
// next to the edge. An uncaught exception would abort the whole app, so these
// catch it and report failure instead.
inline bool BRepFilletAPI_MakeFillet_try_build(BRepFilletAPI_MakeFillet &make) {
  try {
    make.Build();
    return make.IsDone();
  } catch (...) {
    return false;
  }
}

inline bool BRepFilletAPI_MakeChamfer_try_build(BRepFilletAPI_MakeChamfer &make) {
  try {
    make.Build();
    return make.IsDone();
  } catch (...) {
    return false;
  }
}

// opencad patch: chamfer with two distances; `dist1` is measured on `face`.
// Add() throws when the edge isn't an edge of that face, so report failure.
inline bool BRepFilletAPI_MakeChamfer_try_add_two(BRepFilletAPI_MakeChamfer &make, Standard_Real dist1,
                                                  Standard_Real dist2, const TopoDS_Edge &edge,
                                                  const TopoDS_Face &face) {
  try {
    make.Add(dist1, dist2, edge, face);
    return true;
  } catch (...) {
    return false;
  }
}

// opencad patch: a chamfer `dist` along `face` at `angle` (radians) from it.
inline bool BRepFilletAPI_MakeChamfer_try_add_da(BRepFilletAPI_MakeChamfer &make, Standard_Real dist, Standard_Real angle,
                                                 const TopoDS_Edge &edge, const TopoDS_Face &face) {
  try {
    make.AddDA(dist, angle, edge, face);
    return true;
  } catch (...) {
    return false;
  }
}
