#include <Standard_ErrorHandler.hxx>
#include <GeomAPI_Interpolate.hxx>
#include <GeomAPI_ProjectPointOnSurf.hxx>
#include <Geom_BSplineCurve.hxx>
#include <TColStd_HArray1OfReal.hxx>
#include <TColStd_HArray1OfBoolean.hxx>
#include <TColgp_Array1OfVec.hxx>
#include <TColgp_HArray1OfPnt.hxx>
#include <cmath>
#include <bindings_common.hxx>

inline std::unique_ptr<Handle_Geom_BSplineCurve> GeomAPI_Interpolate_Curve(const GeomAPI_Interpolate &interpolate) {
  return std::unique_ptr<Handle_Geom_BSplineCurve>(new opencascade::handle<Geom_BSplineCurve>(interpolate.Curve()));
}

// opencad patch: interpolation throws (e.g. on repeated points) and that would
// abort the app; this reports failure as a null handle instead.
inline std::unique_ptr<Handle_Geom_BSplineCurve> GeomAPI_Interpolate_try_curve(const Handle_TColgp_HArray1OfPnt &points, bool periodic) {
  try {
    OCC_CATCH_SIGNALS
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
    OCC_CATCH_SIGNALS
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

// opencad patch: the spline through `pts` (x, y, z each), each point at its
// chord-length parameter, with a set tangent where `has` says (`tangents`,
// x, y, z each); closed smoothly round when `periodic`. Samples each span
// between two points evenly, `per_span` to a span, into `out` (x, y, z
// each; ends included), so point k is sample k * per_span. False on failure.
inline bool opencad_spline_samples(rust::Slice<const double> pts, rust::Slice<const double> tangents, rust::Slice<const uint8_t> has, bool periodic, size_t per_span, rust::Slice<double> out) {
  try {
    OCC_CATCH_SIGNALS
    size_t n = pts.size() / 3;
    if (n < (periodic ? 3u : 2u) || tangents.size() != 3 * n || has.size() != n || per_span < 1) {
      return false;
    }
    size_t spans = periodic ? n : n - 1;
    if (out.size() != 3 * (spans * per_span + 1)) {
      return false;
    }
    Handle(TColgp_HArray1OfPnt) points = new TColgp_HArray1OfPnt(1, int(n));
    for (size_t i = 0; i < n; i++) {
      points->SetValue(int(i) + 1, gp_Pnt(pts[3 * i], pts[3 * i + 1], pts[3 * i + 2]));
    }
    // Chord-length parameters (a closed one's last is back at the start).
    size_t np = periodic ? n + 1 : n;
    Handle(TColStd_HArray1OfReal) params = new TColStd_HArray1OfReal(1, int(np));
    double u = 0.0;
    params->SetValue(1, 0.0);
    for (size_t i = 1; i < np; i++) {
      size_t a = i - 1, b = i % n;
      double dx = pts[3 * b] - pts[3 * a], dy = pts[3 * b + 1] - pts[3 * a + 1], dz = pts[3 * b + 2] - pts[3 * a + 2];
      double d = std::sqrt(dx * dx + dy * dy + dz * dz);
      if (d < 1e-9) {
        return false;
      }
      u += d;
      params->SetValue(int(i) + 1, u);
    }
    GeomAPI_Interpolate interp(points, params, periodic, 1.0e-7);
    bool any = false;
    for (size_t i = 0; i < n; i++) {
      any = any || has[i] != 0;
    }
    if (any) {
      TColgp_Array1OfVec tv(1, int(n));
      Handle(TColStd_HArray1OfBoolean) flags = new TColStd_HArray1OfBoolean(1, int(n));
      for (size_t i = 0; i < n; i++) {
        tv.SetValue(int(i) + 1, gp_Vec(tangents[3 * i], tangents[3 * i + 1], tangents[3 * i + 2]));
        flags->SetValue(int(i) + 1, has[i] != 0);
      }
      interp.Load(tv, flags, false);
    }
    interp.Perform();
    if (!interp.IsDone()) {
      return false;
    }
    Handle(Geom_BSplineCurve) curve = interp.Curve();
    size_t k = 0;
    for (size_t s = 0; s < spans; s++) {
      double u0 = params->Value(int(s) + 1), u1 = params->Value(int(s) + 2);
      for (size_t j = 0; j < per_span; j++) {
        gp_Pnt p = curve->Value(u0 + (u1 - u0) * double(j) / double(per_span));
        out[3 * k] = p.X();
        out[3 * k + 1] = p.Y();
        out[3 * k + 2] = p.Z();
        k++;
      }
    }
    gp_Pnt last = curve->Value(params->Value(int(np)));
    out[3 * k] = last.X();
    out[3 * k + 1] = last.Y();
    out[3 * k + 2] = last.Z();
    return true;
  } catch (...) {
    return false;
  }
}
