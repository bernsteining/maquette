//! C ABI for maquette. Every function returns 0 on success and 1 on error.
//! `*out`/`*out_len` receive a heap buffer owned by the caller: on success the
//! rendered bytes (PNG/SVG/PLY/JSON), on error a UTF-8 error message. Free it
//! with `maquette_free(ptr, len)`. Pass NULL config for defaults.

use std::collections::HashMap;
use std::ffi::CStr;
use std::os::raw::c_char;
use std::slice;

// The plugin crates keep their `#[wasm_func]` export wrappers off-wasm (to keep
// the shipped wasm byte-identical); in this cdylib they reference the two Typst
// host-protocol imports, absent natively. No-op stand-ins so the library loads.
#[no_mangle]
pub extern "C" fn wasm_minimal_protocol_write_args_to_buffer(_ptr: *mut u8) {}
#[no_mangle]
pub extern "C" fn wasm_minimal_protocol_send_result_to_host(_ptr: *const u8, _len: usize) {}

unsafe fn deliver(res: Result<Vec<u8>, String>, out: *mut *mut u8, out_len: *mut usize) -> i32 {
    let (code, bytes) = match res {
        Ok(b) => (0, b),
        Err(e) => (1, e.into_bytes()),
    };
    let boxed = bytes.into_boxed_slice();
    let len = boxed.len();
    *out = Box::into_raw(boxed) as *mut u8;
    *out_len = len;
    code
}

unsafe fn cfg(config: *const c_char) -> Vec<u8> {
    if config.is_null() {
        b"{}".to_vec()
    } else {
        CStr::from_ptr(config).to_bytes().to_vec()
    }
}

fn encode_png(raw: &[u8]) -> Result<Vec<u8>, String> {
    let marker = *raw.first().ok_or("empty render output")?;
    match marker {
        0x00 | 0x02 => {
            if raw.len() < 9 {
                return Err("truncated raster header".into());
            }
            let w = u32::from_le_bytes(raw[1..5].try_into().unwrap());
            let h = u32::from_le_bytes(raw[5..9].try_into().unwrap());
            let n = (w as usize) * (h as usize) * 4;
            let px = raw.get(9..9 + n).ok_or("truncated raster pixels")?;
            let mut out = Vec::new();
            {
                let mut enc = png::Encoder::new(&mut out, w, h);
                enc.set_color(png::ColorType::Rgba);
                enc.set_depth(png::BitDepth::Eight);
                let mut wr = enc.write_header().map_err(|e| e.to_string())?;
                wr.write_image_data(px).map_err(|e| e.to_string())?;
            }
            Ok(out)
        }
        0x3C => Ok(raw.to_vec()),
        m => Err(format!("unexpected render marker 0x{m:02x}")),
    }
}

macro_rules! render_fn {
    ($name:ident, $inner:path, png) => {
        #[no_mangle]
        pub unsafe extern "C" fn $name(data: *const u8, len: usize, config: *const c_char, out: *mut *mut u8, out_len: *mut usize) -> i32 {
            let res = $inner(slice::from_raw_parts(data, len), &cfg(config)).and_then(|r| encode_png(&r));
            deliver(res, out, out_len)
        }
    };
    ($name:ident, $inner:path, raw) => {
        #[no_mangle]
        pub unsafe extern "C" fn $name(data: *const u8, len: usize, config: *const c_char, out: *mut *mut u8, out_len: *mut usize) -> i32 {
            let res = $inner(slice::from_raw_parts(data, len), &cfg(config));
            deliver(res, out, out_len)
        }
    };
}

render_fn!(maquette_render_stl_png, maquette::native::render_stl_png, png);
render_fn!(maquette_render_stl_svg, maquette::native::render_stl, raw);
render_fn!(maquette_render_obj_png, maquette::native::render_obj_png, png);
render_fn!(maquette_render_obj_svg, maquette::native::render_obj, raw);
render_fn!(maquette_render_ply_png, maquette::native::render_ply_png, png);
render_fn!(maquette_render_ply_svg, maquette::native::render_ply, raw);
render_fn!(maquette_render_gltf_png, maquette_gltf::native::render_gltf, png);
render_fn!(maquette_info_stl, native_info_stl, raw);
render_fn!(maquette_info_obj, native_info_obj, raw);
render_fn!(maquette_info_ply, native_info_ply, raw);
render_fn!(maquette_info_gltf, native_info_gltf, raw);

fn native_info_stl(d: &[u8], c: &[u8]) -> Result<Vec<u8>, String> { let _ = c; maquette::native::get_stl_info(d, b"{}") }
fn native_info_obj(d: &[u8], c: &[u8]) -> Result<Vec<u8>, String> { let _ = c; maquette::native::get_obj_info(d, b"{}") }
fn native_info_ply(d: &[u8], c: &[u8]) -> Result<Vec<u8>, String> { let _ = c; maquette::native::get_ply_info(d, b"{}") }
fn native_info_gltf(d: &[u8], c: &[u8]) -> Result<Vec<u8>, String> { let _ = c; maquette_gltf::native::get_gltf_info(d, b"{}") }

unsafe fn scad_ply(src: *const c_char, facets: usize) -> Result<Vec<u8>, String> {
    let text = CStr::from_ptr(src).to_str().map_err(|_| "scad source is not UTF-8".to_string())?;
    maquette_scad::compile_scad(text, HashMap::new(), facets, HashMap::new())
}

#[no_mangle]
pub unsafe extern "C" fn maquette_compile_scad(src: *const c_char, facets: usize, out: *mut *mut u8, out_len: *mut usize) -> i32 {
    deliver(scad_ply(src, facets), out, out_len)
}

#[no_mangle]
pub unsafe extern "C" fn maquette_render_scad_png(src: *const c_char, config: *const c_char, facets: usize, out: *mut *mut u8, out_len: *mut usize) -> i32 {
    let res = scad_ply(src, facets)
        .and_then(|ply| maquette::native::render_ply_png(&ply, &cfg(config)))
        .and_then(|raw| encode_png(&raw));
    deliver(res, out, out_len)
}

#[no_mangle]
pub unsafe extern "C" fn maquette_render_scad_svg(src: *const c_char, config: *const c_char, facets: usize, out: *mut *mut u8, out_len: *mut usize) -> i32 {
    let res = scad_ply(src, facets).and_then(|ply| maquette::native::render_ply(&ply, &cfg(config)));
    deliver(res, out, out_len)
}

#[no_mangle]
pub unsafe extern "C" fn maquette_free(ptr: *mut u8, len: usize) {
    if !ptr.is_null() {
        drop(Box::from_raw(slice::from_raw_parts_mut(ptr, len)));
    }
}
