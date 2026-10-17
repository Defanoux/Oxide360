#include <BRepTools.hxx>
#include <BRep_Builder.hxx>
#include <TopoDS_Wire.hxx>
#include <bindings_common.hxx>

inline std::unique_ptr<TopoDS_Wire> outer_wire(const TopoDS_Face &face) {
  return std::unique_ptr<TopoDS_Wire>(new TopoDS_Wire(BRepTools::OuterWire(face)));
}

inline bool write_brep_text(const TopoDS_Shape &shape, rust::String path) {
  return BRepTools::Write(shape, path.c_str());
}

inline std::unique_ptr<TopoDS_Shape> read_brep_text(rust::String path) {
  BRep_Builder builder;
  auto shape = std::unique_ptr<TopoDS_Shape>(new TopoDS_Shape());

  if (BRepTools::Read(*shape, path.c_str(), builder)) {
    return shape;
  } else {
    return std::unique_ptr<TopoDS_Shape>(nullptr);
  }
}

// opencad patch: import and in-document storage. Any OCCT failure gives
// nullptr / false instead of an exception (which would abort the app).
#include <STEPControl_Reader.hxx>
#include <ShapeUpgrade_UnifySameDomain.hxx>
#include <Standard_ErrorHandler.hxx>
#include <BRepBuilderAPI_MakeEdge.hxx>
#include <BRepBuilderAPI_MakeFace.hxx>
#include <BRepGProp.hxx>
#include <GProp_GProps.hxx>
#include <Poly_Triangulation.hxx>
#include <RWStl.hxx>
#include <TopoDS_Shell.hxx>
#include <TopoDS_Solid.hxx>
#include <gp_Pln.hxx>
#include <map>
#include <vector>
#include <TopExp_Explorer.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Compound.hxx>
#include <sstream>

// A STEP file's shapes, in millimetres.
inline std::unique_ptr<TopoDS_Shape> opencad_try_read_step(rust::Str path) {
  try {
    OCC_CATCH_SIGNALS
    STEPControl_Reader reader;
    if (reader.ReadFile(std::string(path).c_str()) != IFSelect_RetDone) {
      return nullptr;
    }
    if (reader.TransferRoots() <= 0) {
      return nullptr;
    }
    TopoDS_Shape shape = reader.OneShape();
    if (shape.IsNull()) {
      return nullptr;
    }
    return std::unique_ptr<TopoDS_Shape>(new TopoDS_Shape(shape));
  } catch (...) {
    return nullptr;
  }
}

// An STL mesh made into solids: triangles joined along their shared edges
// (equal corners merged on reading), each closed shell a solid turned
// outward, coplanar triangles merged into one face. A mesh with holes comes
// back as open shells (no solids), so the caller can say so.
inline std::unique_ptr<TopoDS_Shape> opencad_try_read_stl(rust::Str path) {
  try {
    OCC_CATCH_SIGNALS
    Handle(Poly_Triangulation) tri = RWStl::ReadFile(std::string(path).c_str());
    if (tri.IsNull() || tri->NbTriangles() == 0) {
      return nullptr;
    }
    BRep_Builder builder;
    const int nodes = tri->NbNodes();
    std::vector<TopoDS_Vertex> vertex(nodes + 1);
    for (int i = 1; i <= nodes; i++) {
      builder.MakeVertex(vertex[i], tri->Node(i), 1.0e-7);
    }
    // Each edge once, shared by the triangles on both sides; how many use it.
    std::map<std::pair<int, int>, std::pair<TopoDS_Edge, int>> edges;
    auto edge = [&](int a, int b) -> TopoDS_Edge {
      std::pair<int, int> key(std::min(a, b), std::max(a, b));
      auto found = edges.find(key);
      if (found == edges.end()) {
        found = edges.emplace(key, std::make_pair(TopoDS_Edge(), 0)).first;
      }
      if (found->second.first.IsNull()) {
        found->second.first = BRepBuilderAPI_MakeEdge(vertex[key.first], vertex[key.second]).Edge();
      }
      found->second.second++;
      TopoDS_Edge e = found->second.first;
      return a == key.first ? e : TopoDS::Edge(e.Reversed());
    };
    TopoDS_Shell shell;
    builder.MakeShell(shell);
    int faces = 0;
    for (int t = 1; t <= tri->NbTriangles(); t++) {
      int n1, n2, n3;
      tri->Triangle(t).Get(n1, n2, n3);
      if (n1 == n2 || n2 == n3 || n1 == n3) {
        continue;
      }
      gp_Pnt p1 = tri->Node(n1), p2 = tri->Node(n2), p3 = tri->Node(n3);
      gp_Vec normal = gp_Vec(p1, p2).Crossed(gp_Vec(p1, p3));
      if (normal.Magnitude() < 1.0e-12) {
        // A sliver with no area (corners in a line): no face, but its edges
        // still count as covered, as the triangles round it close up there.
        const int ends[3][2] = {{n1, n2}, {n2, n3}, {n3, n1}};
        for (const auto &ab : ends) {
          std::pair<int, int> key(std::min(ab[0], ab[1]), std::max(ab[0], ab[1]));
          edges.emplace(key, std::make_pair(TopoDS_Edge(), 0)).first->second.second++;
        }
        continue;
      }
      TopoDS_Wire wire;
      builder.MakeWire(wire);
      builder.Add(wire, edge(n1, n2));
      builder.Add(wire, edge(n2, n3));
      builder.Add(wire, edge(n3, n1));
      wire.Closed(true);
      BRepBuilderAPI_MakeFace face(gp_Pln(p1, gp_Dir(normal)), wire, true);
      if (!face.IsDone()) {
        continue;
      }
      builder.Add(shell, face.Face());
      faces++;
    }
    if (faces == 0) {
      return nullptr;
    }
    // Closed: every edge has triangles on both sides — two, or (where parts
    // of the mesh touch along an edge) four or six.
    bool closed = true;
    for (auto &kv : edges) {
      if (kv.second.second % 2 != 0) {
        closed = false;
        break;
      }
    }
    if (!closed) {
      return std::unique_ptr<TopoDS_Shape>(new TopoDS_Shape(shell));
    }
    shell.Closed(true);
    TopoDS_Solid solid;
    builder.MakeSolid(solid);
    builder.Add(solid, shell);
    GProp_GProps props;
    BRepGProp::VolumeProperties(solid, props);
    if (props.Mass() < 0.0) {
      solid.Reverse();
    }
    ShapeUpgrade_UnifySameDomain unify(solid, true, true, false);
    unify.Build();
    return std::unique_ptr<TopoDS_Shape>(new TopoDS_Shape(unify.Shape()));
  } catch (...) {
    return nullptr;
  }
}

// A shape as OCCT's own text format (exact), to keep inside a design file.
inline rust::String opencad_brep_to_string(const TopoDS_Shape &shape) {
  try {
    OCC_CATCH_SIGNALS
    std::ostringstream out;
    BRepTools::Write(shape, out);
    return rust::String(out.str());
  } catch (...) {
    return rust::String();
  }
}

inline std::unique_ptr<TopoDS_Shape> opencad_try_brep_from_string(rust::Str text) {
  try {
    OCC_CATCH_SIGNALS
    std::istringstream in{std::string(text)};
    BRep_Builder builder;
    TopoDS_Shape shape;
    BRepTools::Read(shape, in, builder);
    if (shape.IsNull()) {
      return nullptr;
    }
    return std::unique_ptr<TopoDS_Shape>(new TopoDS_Shape(shape));
  } catch (...) {
    return nullptr;
  }
}

// opencad patch: a shape's exact bounding box (from its geometry, not its
// mesh, with no tolerance added): xmin, ymin, zmin, xmax, ymax, zmax.
#include <BRepBndLib.hxx>
#include <Bnd_Box.hxx>

inline bool opencad_try_exact_bounds(const TopoDS_Shape &shape, rust::Slice<double> out) {
  try {
    OCC_CATCH_SIGNALS
    if (out.size() < 6) {
      return false;
    }
    Bnd_Box box;
    BRepBndLib::AddOptimal(shape, box, false, false);
    if (box.IsVoid()) {
      return false;
    }
    box.Get(out[0], out[1], out[2], out[3], out[4], out[5]);
    return true;
  } catch (...) {
    return false;
  }
}
