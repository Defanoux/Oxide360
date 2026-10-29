#include <Standard_ErrorHandler.hxx>
#include <BRepAdaptor_Curve.hxx>
#include <BRepLib.hxx>
#include <GCPnts_TangentialDeflection.hxx>
#include <HLRAlgo_Projector.hxx>
#include <HLRBRep_Algo.hxx>
#include <HLRBRep_HLRToShape.hxx>
#include <TopExp_Explorer.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Shape.hxx>
#include <gp_Ax2.hxx>
#include <memory>
#include <vector>

// opencad patch: hidden-line removal for 2-D drawings. The shape seen from
// the side `(nx, ny, nz)` points to (the viewer's side), with `(xx, xy, xz)`
// to the right on the paper. Every resulting line as a polyline in paper
// coordinates (mm), flattened: kind, point count, then x y per point. Kinds:
// 0 visible sharp edges, 1 visible outlines (a cylinder's sides), 2 visible
// smooth edges (where a fillet meets a face), 3 hidden sharp edges, 4 hidden
// outlines. `deflection`: largest gap between a curve and its polyline.
// nullptr on any failure.
inline std::unique_ptr<std::vector<double>> opencad_try_hlr(const TopoDS_Shape &shape, double nx, double ny, double nz, double xx, double xy, double xz,
                                                           double deflection) {
  try {
    OCC_CATCH_SIGNALS
    Handle(HLRBRep_Algo) algo = new HLRBRep_Algo();
    algo->Add(shape);
    gp_Ax2 axes(gp_Pnt(0, 0, 0), gp_Dir(nx, ny, nz), gp_Dir(xx, xy, xz));
    algo->Projector(HLRAlgo_Projector(axes));
    algo->Update();
    algo->Hide();
    HLRBRep_HLRToShape out(algo);
    TopoDS_Shape kinds[5] = {out.VCompound(), out.OutLineVCompound(), out.Rg1LineVCompound(), out.HCompound(), out.OutLineHCompound()};
    auto lines = std::unique_ptr<std::vector<double>>(new std::vector<double>());
    for (int k = 0; k < 5; k++) {
      if (kinds[k].IsNull()) {
        continue;
      }
      // The edges come with curves on the paper plane only: give them 3-D ones.
      BRepLib::BuildCurves3d(kinds[k]);
      for (TopExp_Explorer ex(kinds[k], TopAbs_EDGE); ex.More(); ex.Next()) {
        BRepAdaptor_Curve curve(TopoDS::Edge(ex.Current()));
        GCPnts_TangentialDeflection pts(curve, 0.1, deflection);
        if (pts.NbPoints() < 2) {
          continue;
        }
        lines->push_back(k);
        lines->push_back(pts.NbPoints());
        for (int i = 1; i <= pts.NbPoints(); i++) {
          gp_Pnt p = pts.Value(i);
          lines->push_back(p.X());
          lines->push_back(p.Y());
        }
      }
    }
    return lines;
  } catch (...) {
    return nullptr;
  }
}
