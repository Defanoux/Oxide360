#include <Standard_ErrorHandler.hxx>
#include <BRepFeat_MakeCylindricalHole.hxx>
#include <BRepFeat_MakeDPrism.hxx>
#include <LocOpe_DPrism.hxx>
#include <bindings_common.hxx>

// opencad patch: a prism of `face` along (dx, dy, dz) whose sides lean so the
// far end is the face offset by `offset` (negative = smaller). Lofted between
// the face's wires and the offset ones (straight sides); holes are cut out.
// Null when the kernel can't (instead of aborting the app).
#include <BRepAlgoAPI_Cut.hxx>
#include <BRepBndLib.hxx>
#include <Bnd_Box.hxx>
#include <BRepBuilderAPI_Transform.hxx>
#include <BRepOffsetAPI_MakeOffset.hxx>
#include <BRepOffsetAPI_ThruSections.hxx>
#include <BRepTools.hxx>
#include <GeomAbs_JoinType.hxx>
#include <TopExp_Explorer.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Wire.hxx>
#include <gp_Trsf.hxx>
#include <vector>

inline std::unique_ptr<TopoDS_Shape>
tapered_prism_try(const TopoDS_Face &face, Standard_Real dx, Standard_Real dy, Standard_Real dz, Standard_Real offset) {
  try {
    OCC_CATCH_SIGNALS
    BRepOffsetAPI_MakeOffset make_offset(face, GeomAbs_Intersection);
    make_offset.Perform(offset);
    if (!make_offset.IsDone()) {
      return nullptr;
    }
    gp_Trsf lift;
    lift.SetTranslation(gp_Vec(dx, dy, dz));
    // The offset of a face comes back as wires: the biggest is the outline.
    auto centre_and_size = [](const TopoDS_Shape &shape, gp_Pnt &centre) -> Standard_Real {
      Bnd_Box box;
      BRepBndLib::Add(shape, box);
      Standard_Real x0, y0, z0, x1, y1, z1;
      box.Get(x0, y0, z0, x1, y1, z1);
      centre = gp_Pnt((x0 + x1) / 2, (y0 + y1) / 2, (z0 + z1) / 2);
      return gp_Pnt(x0, y0, z0).Distance(gp_Pnt(x1, y1, z1));
    };
    std::vector<TopoDS_Wire> far_all;
    for (TopExp_Explorer w(make_offset.Shape(), TopAbs_WIRE); w.More(); w.Next()) {
      BRepBuilderAPI_Transform moved(w.Current(), lift, true);
      far_all.push_back(TopoDS::Wire(moved.Shape()));
    }
    if (far_all.empty()) {
      return nullptr;
    }
    size_t outer_at = 0;
    Standard_Real biggest = -1;
    for (size_t k = 0; k < far_all.size(); k++) {
      gp_Pnt c;
      Standard_Real size = centre_and_size(far_all[k], c);
      if (size > biggest) {
        biggest = size;
        outer_at = k;
      }
    }
    TopoDS_Wire far_outer = far_all[outer_at];
    std::vector<TopoDS_Wire> bottom_all;
    for (TopExp_Explorer w(face, TopAbs_WIRE); w.More(); w.Next()) {
      bottom_all.push_back(TopoDS::Wire(w.Current()));
    }
    if (bottom_all.empty()) {
      return nullptr;
    }
    size_t bottom_outer_at = 0;
    biggest = -1;
    for (size_t k = 0; k < bottom_all.size(); k++) {
      gp_Pnt c;
      Standard_Real size = centre_and_size(bottom_all[k], c);
      if (size > biggest) {
        biggest = size;
        bottom_outer_at = k;
      }
    }
    TopoDS_Wire bottom_outer = bottom_all[bottom_outer_at];
    std::vector<TopoDS_Wire> bottom_holes, far_holes;
    for (size_t k = 0; k < bottom_all.size(); k++) {
      if (k != bottom_outer_at) {
        bottom_holes.push_back(bottom_all[k]);
      }
    }
    // Each hole meets the offset wire whose middle is nearest (shifted back).
    std::vector<bool> taken(far_all.size(), false);
    taken[outer_at] = true;
    for (const TopoDS_Wire &hole : bottom_holes) {
      gp_Pnt hc;
      centre_and_size(hole, hc);
      int best = -1;
      Standard_Real best_d = 1e300;
      for (size_t k = 0; k < far_all.size(); k++) {
        if (taken[k]) {
          continue;
        }
        gp_Pnt c;
        centre_and_size(far_all[k], c);
        Standard_Real d = c.Distance(hc.Translated(gp_Vec(dx, dy, dz)));
        if (d < best_d) {
          best_d = d;
          best = (int)k;
        }
      }
      if (best < 0) {
        return nullptr;
      }
      taken[best] = true;
      far_holes.push_back(far_all[best]);
    }
    auto loft = [](const TopoDS_Wire &a, const TopoDS_Wire &b) -> TopoDS_Shape {
      BRepOffsetAPI_ThruSections sections(true, true);
      sections.AddWire(a);
      sections.AddWire(b);
      sections.Build();
      if (!sections.IsDone()) {
        return TopoDS_Shape();
      }
      return sections.Shape();
    };
    TopoDS_Shape result = loft(bottom_outer, far_outer);
    if (result.IsNull()) {
      return nullptr;
    }
    for (size_t k = 0; k < bottom_holes.size(); k++) {
      TopoDS_Shape hole = loft(bottom_holes[k], far_holes[k]);
      if (hole.IsNull()) {
        return nullptr;
      }
      BRepAlgoAPI_Cut cut(result, hole);
      if (!cut.IsDone()) {
        return nullptr;
      }
      result = cut.Shape();
    }
    return std::unique_ptr<TopoDS_Shape>(new TopoDS_Shape(result));
  } catch (...) {
    return nullptr;
  }
}
