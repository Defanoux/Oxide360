#include <GeomAPI_Interpolate.hxx>
#include <GeomAPI_ProjectPointOnSurf.hxx>
#include <Geom_BSplineCurve.hxx>
#include <bindings_common.hxx>

inline std::unique_ptr<Handle_Geom_BSplineCurve> GeomAPI_Interpolate_Curve(const GeomAPI_Interpolate &interpolate) {
  return std::unique_ptr<Handle_Geom_BSplineCurve>(new opencascade::handle<Geom_BSplineCurve>(interpolate.Curve()));
}

// opencad patch: interpolation throws (e.g. on repeated points) and that would
// abort the app; this reports failure as a null handle instead.
inline std::unique_ptr<Handle_Geom_BSplineCurve> GeomAPI_Interpolate_try_curve(const Handle_TColgp_HArray1OfPnt &points, bool periodic) {
  try {
    GeomAPI_Interpolate interp(points, periodic, 1.0e-7);
    interp.Perform();
    if (!interp.IsDone()) {
      return std::unique_ptr<Handle_Geom_BSplineCurve>(new opencascade::handle<Geom_BSplineCurve>());
    }
    return std::unique_ptr<Handle_Geom_BSplineCurve>(new opencascade::handle<Geom_BSplineCurve>(interp.Curve()));
  } catch (...) {
    return std::unique_ptr<Handle_Geom_BSplineCurve>(new opencascade::handle<Geom_BSplineCurve>());
  }
}

// opencad patch: `out.size() / 3` points (x, y, z) at evenly spaced parameters
// along the curve, ends included. False if it fails.
inline bool Geom_BSplineCurve_try_sample(const Handle_Geom_BSplineCurve &curve, rust::Slice<double> out) {
  try {
    if (curve.IsNull()) {
      return false;
    }
    size_t count = out.size() / 3;
    if (count < 2) {
      return false;
    }
    double a = curve->FirstParameter(), b = curve->LastParameter();
    for (size_t i = 0; i < count; i++) {
      gp_Pnt p = curve->Value(a + (b - a) * double(i) / double(count - 1));
      out[3 * i] = p.X();
      out[3 * i + 1] = p.Y();
      out[3 * i + 2] = p.Z();
    }
    return true;
  } catch (...) {
    return false;
  }
}
