#include <Standard_ErrorHandler.hxx>
#include <BRepPrimAPI_MakeBox.hxx>
#include <BRepPrimAPI_MakeCone.hxx>
#include <BRepPrimAPI_MakeCylinder.hxx>
#include <BRepPrimAPI_MakePrism.hxx>
#include <BRepPrimAPI_MakeRevol.hxx>
#include <BRepPrimAPI_MakeSphere.hxx>
#include <BRepPrimAPI_MakeTorus.hxx>
#include <bindings_common.hxx>

// opencad patch: revolve that can't abort the app. Construction (which
// builds the solid) can throw an OCCT exception for impossible input; this
// catches it and returns null, as does a build that reports failure.
inline std::unique_ptr<BRepPrimAPI_MakeRevol>
BRepPrimAPI_MakeRevol_try_new(const TopoDS_Shape &shape, const gp_Ax1 &axis, Standard_Real angle) {
  try {
    OCC_CATCH_SIGNALS
    auto make = std::unique_ptr<BRepPrimAPI_MakeRevol>(new BRepPrimAPI_MakeRevol(shape, axis, angle, false));
    if (!make->IsDone()) {
      return nullptr;
    }
    return make;
  } catch (...) {
    return nullptr;
  }
}
