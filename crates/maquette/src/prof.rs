//! Fuel-profiling marks. With the `prof` feature on wasm32, `mark(id)` calls a
//! host import that records the interpreter's fuel counter; otherwise it
//! compiles to nothing.

#[cfg(all(feature = "prof", target_arch = "wasm32"))]
#[link(wasm_import_module = "typst_env")]
extern "C" {
    fn maquette_prof(id: u32);
}

#[inline(always)]
pub fn mark(_id: u32) {
    #[cfg(all(feature = "prof", target_arch = "wasm32"))]
    unsafe {
        maquette_prof(_id)
    }
}
