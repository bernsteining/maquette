use std::collections::HashMap;

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyBytes};

// The plugin crates keep their `#[wasm_func]` export wrappers even off-wasm
// (to keep the shipped wasm byte-identical). In this cdylib those wrappers are
// exported and reference the two Typst host-protocol imports, which don't exist
// natively — provide no-op stand-ins so the module loads. They are never
// called (Python goes through the `native::` API).
#[no_mangle]
pub extern "C" fn wasm_minimal_protocol_write_args_to_buffer(_ptr: *mut u8) {}
#[no_mangle]
pub extern "C" fn wasm_minimal_protocol_send_result_to_host(_ptr: *const u8, _len: usize) {}

fn err(e: String) -> PyErr {
    PyValueError::new_err(e)
}

// A config may be a dict (JSON-serialised via Python's json), a JSON string, or
// None (empty). Reuses Python's json so no serde/pythonize dependency is needed.
fn config_json(py: Python<'_>, config: Option<&Bound<'_, PyAny>>) -> PyResult<Vec<u8>> {
    match config {
        None => Ok(b"{}".to_vec()),
        Some(o) => {
            if let Ok(s) = o.extract::<String>() {
                return Ok(s.into_bytes());
            }
            let json = py.import("json")?;
            let s: String = json.call_method1("dumps", (o,))?.extract()?;
            Ok(s.into_bytes())
        }
    }
}

// The renderer's PNG entry points return a raw framebuffer, not an encoded PNG:
// [0x00|0x02][w u32 LE][h u32 LE][rgba8 …]. Encode it to a real PNG (a 0x02
// vector overlay is dropped — use fmt="svg" to keep annotations). 0x3C is SVG.
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

fn finish<'py>(py: Python<'py>, raw: Vec<u8>, fmt: &str) -> PyResult<Bound<'py, PyBytes>> {
    let out = match fmt {
        "png" => encode_png(&raw).map_err(err)?,
        "svg" => raw,
        _ => return Err(err(format!("fmt must be 'png' or 'svg', got {fmt:?}"))),
    };
    Ok(PyBytes::new(py, &out))
}

#[pyfunction]
#[pyo3(signature = (data, config=None, fmt="png"))]
fn render_stl<'py>(py: Python<'py>, data: &[u8], config: Option<Bound<'py, PyAny>>, fmt: &str) -> PyResult<Bound<'py, PyBytes>> {
    let cfg = config_json(py, config.as_ref())?;
    let raw = match fmt {
        "svg" => maquette::native::render_stl(data, &cfg).map_err(err)?,
        _ => maquette::native::render_stl_png(data, &cfg).map_err(err)?,
    };
    finish(py, raw, fmt)
}

#[pyfunction]
#[pyo3(signature = (data, config=None, fmt="png"))]
fn render_obj<'py>(py: Python<'py>, data: &[u8], config: Option<Bound<'py, PyAny>>, fmt: &str) -> PyResult<Bound<'py, PyBytes>> {
    let cfg = config_json(py, config.as_ref())?;
    let raw = match fmt {
        "svg" => maquette::native::render_obj(data, &cfg).map_err(err)?,
        _ => maquette::native::render_obj_png(data, &cfg).map_err(err)?,
    };
    finish(py, raw, fmt)
}

#[pyfunction]
#[pyo3(signature = (data, config=None, fmt="png"))]
fn render_ply<'py>(py: Python<'py>, data: &[u8], config: Option<Bound<'py, PyAny>>, fmt: &str) -> PyResult<Bound<'py, PyBytes>> {
    let cfg = config_json(py, config.as_ref())?;
    let raw = match fmt {
        "svg" => maquette::native::render_ply(data, &cfg).map_err(err)?,
        _ => maquette::native::render_ply_png(data, &cfg).map_err(err)?,
    };
    finish(py, raw, fmt)
}

#[pyfunction]
#[pyo3(signature = (data, config=None))]
fn render_gltf<'py>(py: Python<'py>, data: &[u8], config: Option<Bound<'py, PyAny>>) -> PyResult<Bound<'py, PyBytes>> {
    let cfg = config_json(py, config.as_ref())?;
    let raw = maquette_gltf::native::render_gltf(data, &cfg).map_err(err)?;
    finish(py, raw, "png")
}

#[pyfunction]
#[pyo3(signature = (src, config=None, facets=32, fmt="png"))]
fn render_scad<'py>(py: Python<'py>, src: &str, config: Option<Bound<'py, PyAny>>, facets: usize, fmt: &str) -> PyResult<Bound<'py, PyBytes>> {
    let ply = maquette_scad::compile_scad(src, HashMap::new(), facets, HashMap::new()).map_err(err)?;
    let cfg = config_json(py, config.as_ref())?;
    let raw = match fmt {
        "svg" => maquette::native::render_ply(&ply, &cfg).map_err(err)?,
        _ => maquette::native::render_ply_png(&ply, &cfg).map_err(err)?,
    };
    finish(py, raw, fmt)
}

#[pyfunction]
#[pyo3(signature = (src, facets=32))]
fn compile_scad<'py>(py: Python<'py>, src: &str, facets: usize) -> PyResult<Bound<'py, PyBytes>> {
    let ply = maquette_scad::compile_scad(src, HashMap::new(), facets, HashMap::new()).map_err(err)?;
    Ok(PyBytes::new(py, &ply))
}

fn info<'py>(py: Python<'py>, json_bytes: Vec<u8>) -> PyResult<Bound<'py, PyAny>> {
    let s = String::from_utf8(json_bytes).map_err(|e| err(e.to_string()))?;
    py.import("json")?.call_method1("loads", (s,))
}

#[pyfunction]
fn info_stl<'py>(py: Python<'py>, data: &[u8]) -> PyResult<Bound<'py, PyAny>> {
    info(py, maquette::native::get_stl_info(data, b"{}").map_err(err)?)
}

#[pyfunction]
fn info_obj<'py>(py: Python<'py>, data: &[u8]) -> PyResult<Bound<'py, PyAny>> {
    info(py, maquette::native::get_obj_info(data, b"{}").map_err(err)?)
}

#[pyfunction]
fn info_ply<'py>(py: Python<'py>, data: &[u8]) -> PyResult<Bound<'py, PyAny>> {
    info(py, maquette::native::get_ply_info(data, b"{}").map_err(err)?)
}

#[pyfunction]
fn info_gltf<'py>(py: Python<'py>, data: &[u8]) -> PyResult<Bound<'py, PyAny>> {
    info(py, maquette_gltf::native::get_gltf_info(data, b"{}").map_err(err)?)
}

#[pymodule]
fn _maquette(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(render_stl, m)?)?;
    m.add_function(wrap_pyfunction!(render_obj, m)?)?;
    m.add_function(wrap_pyfunction!(render_ply, m)?)?;
    m.add_function(wrap_pyfunction!(render_gltf, m)?)?;
    m.add_function(wrap_pyfunction!(render_scad, m)?)?;
    m.add_function(wrap_pyfunction!(compile_scad, m)?)?;
    m.add_function(wrap_pyfunction!(info_stl, m)?)?;
    m.add_function(wrap_pyfunction!(info_obj, m)?)?;
    m.add_function(wrap_pyfunction!(info_ply, m)?)?;
    m.add_function(wrap_pyfunction!(info_gltf, m)?)?;
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}
