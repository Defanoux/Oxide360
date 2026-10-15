use super::make_vec;
use crate::primitives::{make_axis_2, make_point};
use cxx::UniquePtr;
use glam::{dvec3, DVec3};
use opencascade_sys as ffi;

#[derive(Debug, Copy, Clone, PartialEq)]
pub enum EdgeType {
    Line,
    Circle,
    Ellipse,
    Hyperbola,
    Parabola,
    BezierCurve,
    BSplineCurve,
    OffsetCurve,
    OtherCurve,
}

impl From<ffi::geom_abs::GeomAbs_CurveType> for EdgeType {
    fn from(curve_type: ffi::geom_abs::GeomAbs_CurveType) -> Self {
        match curve_type {
            ffi::geom_abs::GeomAbs_CurveType::GeomAbs_Line => Self::Line,
            ffi::geom_abs::GeomAbs_CurveType::GeomAbs_Circle => Self::Circle,
            ffi::geom_abs::GeomAbs_CurveType::GeomAbs_Ellipse => Self::Ellipse,
            ffi::geom_abs::GeomAbs_CurveType::GeomAbs_Hyperbola => Self::Hyperbola,
            ffi::geom_abs::GeomAbs_CurveType::GeomAbs_Parabola => Self::Parabola,
            ffi::geom_abs::GeomAbs_CurveType::GeomAbs_BezierCurve => Self::BezierCurve,
            ffi::geom_abs::GeomAbs_CurveType::GeomAbs_BSplineCurve => Self::BSplineCurve,
            ffi::geom_abs::GeomAbs_CurveType::GeomAbs_OffsetCurve => Self::OffsetCurve,
            ffi::geom_abs::GeomAbs_CurveType::GeomAbs_OtherCurve => Self::OtherCurve,
            ffi::geom_abs::GeomAbs_CurveType { repr } => {
                panic!("Unexpected curve type: {repr}")
            },
        }
    }
}

pub struct Edge {
    pub(crate) inner: UniquePtr<ffi::topo_ds::TopoDS_Edge>,
}

impl AsRef<Edge> for Edge {
    fn as_ref(&self) -> &Edge {
        self
    }
}

/// opencad patch: copies share the same underlying edge.
impl Clone for Edge {
    fn clone(&self) -> Self {
        Self::from_edge(&self.inner)
    }
}

impl Edge {
    /// opencad patch: OCCT's hash of the edge (same edge, same hash).
    pub fn hash_code(&self) -> u64 {
        ffi::topo_ds::opencad_shape_hash(ffi::topo_ds::cast_edge_to_shape(&self.inner))
    }

    /// opencad patch: the very same edge.
    pub fn is_same(&self, other: &Edge) -> bool {
        ffi::topo_ds::cast_edge_to_shape(&self.inner).IsEqual(ffi::topo_ds::cast_edge_to_shape(&other.inner))
    }

    pub(crate) fn from_edge(edge: &ffi::topo_ds::TopoDS_Edge) -> Self {
        let inner = ffi::topo_ds::TopoDS_Edge_to_owned(edge);

        Self { inner }
    }

    fn from_make_edge(
        mut make_edge: UniquePtr<ffi::b_rep_builder_api::BRepBuilderAPI_MakeEdge>,
    ) -> Self {
        Self::from_edge(make_edge.pin_mut().Edge())
    }

    pub fn segment(p1: DVec3, p2: DVec3) -> Self {
        let make_edge = ffi::b_rep_builder_api::BRepBuilderAPI_MakeEdge_gp_Pnt_gp_Pnt(
            &make_point(p1),
            &make_point(p2),
        );

        Self::from_make_edge(make_edge)
    }

    pub fn bezier(points: impl IntoIterator<Item = DVec3>) -> Self {
        let points: Vec<_> = points.into_iter().collect();
        let mut array = ffi::t_col_gp::TColgp_HArray1OfPnt_new(1, points.len() as i32);
        for (index, point) in points.into_iter().enumerate() {
            array.pin_mut().SetValue(index as i32 + 1, &make_point(point));
        }

        let bezier = ffi::geom::Geom_BezierCurve_new_points(&array);
        let bezier_handle = ffi::geom::Geom_BezierCurve_to_handle(bezier);
        let curve_handle =
            ffi::geom::new_HandleGeomCurve_from_HandleGeom_BezierCurve(&bezier_handle);

        let mut make_edge =
            ffi::b_rep_builder_api::BRepBuilderAPI_MakeEdge_HandleGeomCurve(&curve_handle);
        let edge = make_edge.pin_mut().Edge();
        Self::from_edge(edge)
    }

    pub fn circle(center: DVec3, normal: DVec3, radius: f64) -> Self {
        let axis = make_axis_2(center, normal);

        let make_circle = ffi::gp::gp_Circ_new(&axis, radius);
        let make_edge = ffi::b_rep_builder_api::BRepBuilderAPI_MakeEdge_circle(&make_circle);

        Self::from_make_edge(make_edge)
    }

    /// opencad patch: an ellipse round `center` in the plane square to
    /// `normal`, its `major` semi-axis along `x_dir` (major >= minor): whole
    /// (`span` None) or from parameter u1 to u2 (counter-clockwise about
    /// `normal`). None if OpenCASCADE refuses.
    pub fn try_ellipse(center: DVec3, normal: DVec3, x_dir: DVec3, major: f64, minor: f64, span: Option<(f64, f64)>) -> Option<Self> {
        let (u1, u2) = span.unwrap_or((0.0, 0.0));
        let made = ffi::b_rep_builder_api::opencad_try_ellipse_edge(
            center.x, center.y, center.z, normal.x, normal.y, normal.z, x_dir.x, x_dir.y, x_dir.z, major, minor, u1, u2, span.is_none(),
        );
        (!made.is_null()).then(|| Self::from_edge(&made))
    }

    /// opencad patch: the interpolated spline through `points` (as
    /// `spline_from_points` without tangents), or None if OpenCASCADE can't
    /// make it (it throws, e.g. on repeated points, which would abort).
    pub fn try_spline_through(points: &[DVec3]) -> Option<Self> {
        Self::try_spline(points, false)
    }

    /// opencad patch: as `try_spline_through`; `periodic` closes it smoothly
    /// back to the first point (the points don't repeat it).
    pub fn try_spline(points: &[DVec3], periodic: bool) -> Option<Self> {
        let curve = try_interpolate(points, periodic)?;
        let curve_handle = ffi::geom::new_HandleGeomCurve_from_HandleGeom_BSplineCurve(&curve);
        let mut make_edge = ffi::b_rep_builder_api::BRepBuilderAPI_MakeEdge_HandleGeomCurve(&curve_handle);
        let edge = make_edge.pin_mut().Edge();
        Some(Self::from_edge(edge))
    }

    pub fn spline_from_points(
        points: impl IntoIterator<Item = DVec3>,
        tangents: Option<(DVec3, DVec3)>,
    ) -> Self {
        let points: Vec<_> = points.into_iter().collect();
        let mut array = ffi::t_col_gp::TColgp_HArray1OfPnt_new(1, points.len() as i32);
        for (index, point) in points.into_iter().enumerate() {
            array.pin_mut().SetValue(index as i32 + 1, &make_point(point));
        }
        let array_handle =
            ffi::t_col_gp::new_HandleTColgpHArray1OfPnt_from_TColgpHArray1OfPnt(array);

        let periodic = false;
        let tolerance = 1.0e-7;
        let mut interpolate =
            ffi::geom_api::GeomAPI_Interpolate_new(&array_handle, periodic, tolerance);
        if let Some((t_start, t_end)) = tangents {
            interpolate.pin_mut().Load(&make_vec(t_start), &make_vec(t_end), true);
        }

        interpolate.pin_mut().Perform();
        let bspline_handle = ffi::geom_api::GeomAPI_Interpolate_Curve(&interpolate);
        let curve_handle =
            ffi::geom::new_HandleGeomCurve_from_HandleGeom_BSplineCurve(&bspline_handle);

        let mut make_edge =
            ffi::b_rep_builder_api::BRepBuilderAPI_MakeEdge_HandleGeomCurve(&curve_handle);
        let edge = make_edge.pin_mut().Edge();
        Self::from_edge(edge)
    }

    pub fn arc(p1: DVec3, p2: DVec3, p3: DVec3) -> Self {
        let make_arc = ffi::gc::GC_MakeArcOfCircle_point_point_point(
            &make_point(p1),
            &make_point(p2),
            &make_point(p3),
        );

        let make_edge = ffi::b_rep_builder_api::BRepBuilderAPI_MakeEdge_HandleGeomCurve(
            &ffi::geom::new_HandleGeomCurve_from_HandleGeom_TrimmedCurve(
                &ffi::gc::GC_MakeArcOfCircle_Value(&make_arc),
            ),
        );

        Self::from_make_edge(make_edge)
    }

    pub fn start_point(&self) -> DVec3 {
        let curve = ffi::b_rep_adaptor::BRepAdaptor_Curve_new(&self.inner);
        let start_param = curve.FirstParameter();
        let point = ffi::b_rep_adaptor::BRepAdaptor_Curve_value(&curve, start_param);

        dvec3(point.X(), point.Y(), point.Z())
    }

    pub fn end_point(&self) -> DVec3 {
        let curve = ffi::b_rep_adaptor::BRepAdaptor_Curve_new(&self.inner);
        let last_param = curve.LastParameter();
        let point = ffi::b_rep_adaptor::BRepAdaptor_Curve_value(&curve, last_param);

        dvec3(point.X(), point.Y(), point.Z())
    }

    pub fn approximation_segments(&self) -> ApproximationSegmentIterator {
        let adaptor_curve = ffi::b_rep_adaptor::BRepAdaptor_Curve_new(&self.inner);
        let approximator = ffi::gc_pnts::TangentialDeflection_new(&adaptor_curve, 0.1, 0.1);

        ApproximationSegmentIterator { count: 1, approximator }
    }

    pub fn tangent_arc(_p1: DVec3, _tangent: DVec3, _p3: DVec3) {}

    pub fn edge_type(&self) -> EdgeType {
        let curve = ffi::b_rep_adaptor::BRepAdaptor_Curve_new(&self.inner);

        EdgeType::from(curve.GetType())
    }
}

/// The interpolated B-spline through `points`, or None when it can't be made.
fn try_interpolate(points: &[DVec3], periodic: bool) -> Option<UniquePtr<ffi::geom::Handle_Geom_BSplineCurve>> {
    if points.len() < if periodic { 3 } else { 2 } {
        return None;
    }
    let mut array = ffi::t_col_gp::TColgp_HArray1OfPnt_new(1, points.len() as i32);
    for (index, point) in points.iter().enumerate() {
        array.pin_mut().SetValue(index as i32 + 1, &make_point(*point));
    }
    let array_handle = ffi::t_col_gp::new_HandleTColgpHArray1OfPnt_from_TColgpHArray1OfPnt(array);
    let curve = ffi::geom_api::GeomAPI_Interpolate_try_curve(&array_handle, periodic);
    (!curve.IsNull()).then_some(curve)
}

/// opencad patch: `count` points along the interpolated spline through
/// `points`, evenly spaced in its parameter (ends included). None on failure.
pub fn try_spline_samples(points: &[DVec3], count: usize, periodic: bool) -> Option<Vec<DVec3>> {
    let curve = try_interpolate(points, periodic)?;
    let mut out = vec![0.0; 3 * count.max(2)];
    ffi::geom_api::Geom_BSplineCurve_try_sample(&curve, &mut out).then(|| out.chunks_exact(3).map(|c| dvec3(c[0], c[1], c[2])).collect())
}

pub struct ApproximationSegmentIterator {
    count: usize,
    approximator: UniquePtr<ffi::gc_pnts::GCPnts_TangentialDeflection>,
}

impl Iterator for ApproximationSegmentIterator {
    type Item = DVec3;

    fn next(&mut self) -> Option<Self::Item> {
        if self.count <= self.approximator.NbPoints() as usize {
            let point = ffi::gc_pnts::GCPnts_TangentialDeflection_Value(
                &self.approximator,
                self.count as i32,
            );

            self.count += 1;
            Some(dvec3(point.X(), point.Y(), point.Z()))
        } else {
            None
        }
    }
}
