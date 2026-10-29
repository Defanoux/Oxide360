pub use inner::*;

#[cxx::bridge]
mod inner {
    unsafe extern "C++" {
        include!("opencascade-sys/include/hlr.hxx");

        type TopoDS_Shape = crate::topo_ds::TopoDS_Shape;

        /// opencad patch: hidden-line removal for drawings — the lines seen
        /// from (nx, ny, nz), x to the right; flattened as kind, count, x y…
        /// (null on failure).
        #[allow(clippy::too_many_arguments)]
        pub fn opencad_try_hlr(shape: &TopoDS_Shape, nx: f64, ny: f64, nz: f64, xx: f64, xy: f64, xz: f64, deflection: f64) -> UniquePtr<CxxVector<f64>>;
    }
}
