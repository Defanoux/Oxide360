#include <Standard_ErrorHandler.hxx>
#include <TopTools_ListOfShape.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Compound.hxx>
#include <TopoDS_Wire.hxx>

// opencad patch: OCCT 7.9 turned the `TopoDS` class into a namespace, which
// cxx can't bind as a type. This class forwards the same static casts and is
// bound in its place (see `cxx_name` in src/topo_ds.rs). Works on 7.8 too.
struct TopoDS_Cast {
  static const TopoDS_Vertex& Vertex(const TopoDS_Shape& s) { return TopoDS::Vertex(s); }
  static const TopoDS_Edge& Edge(const TopoDS_Shape& s) { return TopoDS::Edge(s); }
  static const TopoDS_Wire& Wire(const TopoDS_Shape& s) { return TopoDS::Wire(s); }
  static const TopoDS_Face& Face(const TopoDS_Shape& s) { return TopoDS::Face(s); }
  static const TopoDS_Shell& Shell(const TopoDS_Shape& s) { return TopoDS::Shell(s); }
  static const TopoDS_Solid& Solid(const TopoDS_Shape& s) { return TopoDS::Solid(s); }
  static const TopoDS_Compound& Compound(const TopoDS_Shape& s) { return TopoDS::Compound(s); }
};
#include <TopoDS_Builder.hxx>
#include <TopoDS_Edge.hxx>
#include <TopoDS_Face.hxx>
#include <TopoDS_Shape.hxx>
#include <TopoDS_Shell.hxx>
#include <TopoDS_Solid.hxx>
#include <TopoDS_Vertex.hxx>
#include <bindings_common.hxx>

// opencad patch: OCCT's hash of a shape (its underlying shape and placement),
// for caching per-face work. Never throws.
#include <functional>
inline uint64_t opencad_shape_hash(const TopoDS_Shape &shape) {
  try {
    OCC_CATCH_SIGNALS
    return uint64_t(std::hash<TopoDS_Shape>{}(shape));
  } catch (...) {
    return 0;
  }
}
