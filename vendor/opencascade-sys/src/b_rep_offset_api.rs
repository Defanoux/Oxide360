pub use inner::*;

#[cxx::bridge]
mod inner {
    #[derive(Debug)]
    #[repr(u32)]
    pub enum BRepOffset_Mode {
        BRepOffset_Skin,
        BRepOffset_Pipe,
        BRepOffset_RectoVerso,
    }

    unsafe extern "C++" {
        include!("opencascade-sys/include/b_rep_offset_api.hxx");

        type TopoDS_Shape = crate::topo_ds::TopoDS_Shape;
        type TopoDS_Face = crate::topo_ds::TopoDS_Face;
        type TopoDS_Wire = crate::topo_ds::TopoDS_Wire;
        type GeomAbs_JoinType = crate::geom_abs::GeomAbs_JoinType;
        type TopTools_ListOfShape = crate::top_tools::TopTools_ListOfShape;
        type Message_ProgressRange = crate::message::Message_ProgressRange;
        type Handle_Law_Function = crate::law::Handle_Law_Function;

        type BRepOffset_Mode;

        /// opencad patch: a thread's solid, swept along a helix (null on failure).
        #[allow(clippy::too_many_arguments)]
        pub fn opencad_try_thread(
            ox: f64, oy: f64, oz: f64, dx: f64, dy: f64, dz: f64, xx: f64, xy: f64, xz: f64,
            r_in: f64, r_out: f64, w_in: f64, w_out: f64, pitch: f64, length: f64, right_hand: bool, taper: f64,
        ) -> UniquePtr<TopoDS_Shape>;

        /// opencad patch: Shell through the faces numbered `faces` (null on failure).
        pub fn opencad_try_thick_solid(shape: &TopoDS_Shape, faces: &[i32], offset: f64, rounded: bool) -> UniquePtr<TopoDS_Shape>;
        /// opencad patch: a solid grown or shrunk all round (null on failure).
        pub fn opencad_try_offset_shape(shape: &TopoDS_Shape, offset: f64, rounded: bool) -> UniquePtr<TopoDS_Shape>;
        /// opencad patch: chosen faces moved along their normals (null on failure).
        pub fn opencad_try_offset_faces(shape: &TopoDS_Shape, faces: &[i32], offsets: &[f64], intersection: bool) -> UniquePtr<TopoDS_Shape>;

        type BRepOffsetAPI_MakeOffset;
        #[cxx_name = "construct_unique"]
        pub fn BRepOffsetAPI_MakeOffset_face_new(
            face: &TopoDS_Face,
            join: GeomAbs_JoinType,
        ) -> UniquePtr<BRepOffsetAPI_MakeOffset>;
        #[cxx_name = "construct_unique"]
        pub fn BRepOffsetAPI_MakeOffset_wire_new(
            wire: &TopoDS_Wire,
            join: GeomAbs_JoinType,
        ) -> UniquePtr<BRepOffsetAPI_MakeOffset>;
        pub fn Perform(self: Pin<&mut BRepOffsetAPI_MakeOffset>, offset: f64, alt: f64);
        pub fn Shape(self: Pin<&mut BRepOffsetAPI_MakeOffset>) -> &TopoDS_Shape;
        pub fn Build(self: Pin<&mut BRepOffsetAPI_MakeOffset>, progress: &Message_ProgressRange);
        pub fn IsDone(self: &BRepOffsetAPI_MakeOffset) -> bool;

        type BRepOffsetAPI_MakeThickSolid;
        #[cxx_name = "construct_unique"]
        pub fn BRepOffsetAPI_MakeThickSolid_new() -> UniquePtr<BRepOffsetAPI_MakeThickSolid>;
        #[allow(clippy::too_many_arguments)]
        pub fn MakeThickSolidByJoin(
            self: Pin<&mut BRepOffsetAPI_MakeThickSolid>,
            shape: &TopoDS_Shape,
            closing_faces: &TopTools_ListOfShape,
            offset: f64,
            tolerance: f64,
            offset_mode: BRepOffset_Mode,
            intersection: bool,
            self_intersection: bool,
            join_type: GeomAbs_JoinType,
            remove_intersecting_edges: bool,
            progress: &Message_ProgressRange,
        );
        pub fn Shape(self: Pin<&mut BRepOffsetAPI_MakeThickSolid>) -> &TopoDS_Shape;
        pub fn Build(
            self: Pin<&mut BRepOffsetAPI_MakeThickSolid>,
            progress: &Message_ProgressRange,
        );
        pub fn IsDone(self: &BRepOffsetAPI_MakeThickSolid) -> bool;

        type BRepOffsetAPI_MakePipe;
        #[cxx_name = "construct_unique"]
        pub fn BRepOffsetAPI_MakePipe_new(
            spine: &TopoDS_Wire,
            profile: &TopoDS_Shape,
        ) -> UniquePtr<BRepOffsetAPI_MakePipe>;
        pub fn Shape(self: Pin<&mut BRepOffsetAPI_MakePipe>) -> &TopoDS_Shape;

        type BRepOffsetAPI_MakePipeShell;
        #[cxx_name = "construct_unique"]
        pub fn BRepOffsetAPI_MakePipeShell_new(
            spine: &TopoDS_Wire,
        ) -> UniquePtr<BRepOffsetAPI_MakePipeShell>;
        pub fn SetMode(self: Pin<&mut BRepOffsetAPI_MakePipeShell>, is_frenet: bool);
        pub fn Add(
            self: Pin<&mut BRepOffsetAPI_MakePipeShell>,
            profile: &TopoDS_Shape,
            with_contact: bool,
            with_correction: bool,
        );
        pub fn SetLaw(
            self: Pin<&mut BRepOffsetAPI_MakePipeShell>,
            profile: &TopoDS_Shape,
            law: &Handle_Law_Function,
            with_contact: bool,
            with_correction: bool,
        );
        pub fn Build(self: Pin<&mut BRepOffsetAPI_MakePipeShell>, progress: &Message_ProgressRange);
        pub fn MakeSolid(self: Pin<&mut BRepOffsetAPI_MakePipeShell>) -> bool;
        pub fn Shape(self: Pin<&mut BRepOffsetAPI_MakePipeShell>) -> &TopoDS_Shape;

        type BRepOffsetAPI_ThruSections;
        #[cxx_name = "construct_unique"]
        pub fn BRepOffsetAPI_ThruSections_new(
            is_solid: bool,
        ) -> UniquePtr<BRepOffsetAPI_ThruSections>;
        pub fn AddWire(self: Pin<&mut BRepOffsetAPI_ThruSections>, wire: &TopoDS_Wire);
        pub fn CheckCompatibility(self: Pin<&mut BRepOffsetAPI_ThruSections>, check: bool);
        pub fn Shape(self: Pin<&mut BRepOffsetAPI_ThruSections>) -> &TopoDS_Shape;
        pub fn Build(self: Pin<&mut BRepOffsetAPI_ThruSections>, progress: &Message_ProgressRange);
        pub fn IsDone(self: &BRepOffsetAPI_ThruSections) -> bool;
    }
}
