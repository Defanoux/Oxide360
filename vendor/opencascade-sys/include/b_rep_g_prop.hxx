#include <BRepGProp.hxx>
#include <BRepGProp_Face.hxx>
#include <bindings_common.hxx>
#include <Standard_ErrorHandler.hxx>
#include <GProp_GProps.hxx>
#include <gp_Pnt.hxx>

// opencad patch: volume and centre of mass by OCCT's adaptive integration to
// relative error `eps` (the plain call is off by ~1% on B-spline faces, as
// a non-uniform Scale makes). out = [volume, cx, cy, cz]. false on failure.
inline bool opencad_try_volume_eps(const TopoDS_Shape &shape, double eps, rust::Slice<double> out) {
  try {
    OCC_CATCH_SIGNALS
    GProp_GProps props;
    BRepGProp::VolumeProperties(shape, props, eps, Standard_False);
    gp_Pnt c = props.CentreOfMass();
    out[0] = props.Mass();
    out[1] = c.X();
    out[2] = c.Y();
    out[3] = c.Z();
    return true;
  } catch (...) {
    return false;
  }
}
