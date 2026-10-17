pub use inner::*;

#[cxx::bridge]
mod inner {
    unsafe extern "C++" {
        include!("opencascade-sys/include/b_rep_tools.hxx");

        type TopoDS_Shape = crate::topo_ds::TopoDS_Shape;
        type TopoDS_Face = crate::topo_ds::TopoDS_Face;
        type TopoDS_Wire = crate::topo_ds::TopoDS_Wire;

        type BRepTools;

        fn outer_wire(face: &TopoDS_Face) -> UniquePtr<TopoDS_Wire>;

        #[cxx_name = "write_brep_text"]
        pub fn write(shape: &TopoDS_Shape, path: String) -> bool;
        #[cxx_name = "read_brep_text"]
        pub fn read(path: String) -> UniquePtr<TopoDS_Shape>;

        /// opencad patch: a STEP file's shapes (null on failure).
        pub fn opencad_try_read_step(path: &str) -> UniquePtr<TopoDS_Shape>;
        /// opencad patch: an STL mesh as solids (null on failure).
        pub fn opencad_try_read_stl(path: &str) -> UniquePtr<TopoDS_Shape>;
        /// opencad patch: a shape as BRep text (empty on failure).
        pub fn opencad_brep_to_string(shape: &TopoDS_Shape) -> String;
        /// opencad patch: a shape back from BRep text (null on failure).
        pub fn opencad_try_brep_from_string(text: &str) -> UniquePtr<TopoDS_Shape>;
    }
}
