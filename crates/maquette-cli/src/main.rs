use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, ValueEnum};

#[derive(Parser)]
#[command(
    name = "maquette",
    about = "Render 3D models (STL/OBJ/PLY, glTF, OpenSCAD) to PNG or SVG."
)]
struct Cli {
    /// Input model: .stl/.obj/.ply, .glb/.gltf, or .scad
    input: PathBuf,

    /// Output file; its extension picks the format (.png or .svg). Defaults to <input>.png
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// JSON render-config file (the same dict the Typst plugins accept)
    #[arg(short, long)]
    config: Option<PathBuf>,

    /// Override one config key: --set key=value (value parsed as JSON, else string). Repeatable.
    #[arg(short = 's', long = "set", value_name = "KEY=VALUE")]
    set: Vec<String>,

    /// Force the output format regardless of the -o extension
    #[arg(long, value_enum)]
    format: Option<Format>,

    /// Force the plugin instead of detecting it from the input extension
    #[arg(long, value_enum)]
    plugin: Option<Plugin>,

    /// OpenSCAD facet count ($fn) for .scad inputs
    #[arg(long = "fn", default_value_t = 32)]
    scad_fn: usize,
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Format {
    Png,
    Svg,
}

impl Format {
    fn ext(self) -> &'static str {
        match self {
            Format::Png => "png",
            Format::Svg => "svg",
        }
    }
}

#[derive(Clone, Copy, ValueEnum)]
enum Plugin {
    Maquette,
    Gltf,
    Scad,
}

fn ext_of(p: &Path) -> Option<String> {
    p.extension().and_then(|s| s.to_str()).map(|s| s.to_ascii_lowercase())
}

fn detect_plugin(p: &Path) -> Result<Plugin, String> {
    match ext_of(p).as_deref() {
        Some("stl") | Some("obj") | Some("ply") => Ok(Plugin::Maquette),
        Some("glb") | Some("gltf") => Ok(Plugin::Gltf),
        Some("scad") => Ok(Plugin::Scad),
        other => Err(format!(
            "cannot detect plugin from extension {:?}; pass --plugin",
            other.unwrap_or("<none>")
        )),
    }
}

fn build_config(config: Option<&Path>, sets: &[String]) -> Result<Vec<u8>, String> {
    let mut value: serde_json::Value = match config {
        Some(p) => {
            let raw = fs::read(p).map_err(|e| format!("read config {}: {e}", p.display()))?;
            serde_json::from_slice(&raw).map_err(|e| format!("parse config {}: {e}", p.display()))?
        }
        None => serde_json::json!({}),
    };
    let obj = value
        .as_object_mut()
        .ok_or("config JSON must be an object")?;
    for s in sets {
        let (k, v) = s
            .split_once('=')
            .ok_or_else(|| format!("--set expects key=value, got {s:?}"))?;
        let parsed = serde_json::from_str(v).unwrap_or_else(|_| serde_json::Value::String(v.to_string()));
        obj.insert(k.to_string(), parsed);
    }
    serde_json::to_vec(&value).map_err(|e| format!("serialize config: {e}"))
}

fn render_maquette(input: &Path, data: &[u8], cfg: &[u8], fmt: Format) -> Result<Vec<u8>, String> {
    use maquette::native::*;
    match (ext_of(input).as_deref(), fmt) {
        (Some("stl"), Format::Png) => render_stl_png(data, cfg),
        (Some("stl"), Format::Svg) => render_stl(data, cfg),
        (Some("obj"), Format::Png) => render_obj_png(data, cfg),
        (Some("obj"), Format::Svg) => render_obj(data, cfg),
        (Some("ply"), Format::Png) => render_ply_png(data, cfg),
        (Some("ply"), Format::Svg) => render_ply(data, cfg),
        _ => Err(format!("unsupported maquette input: {}", input.display())),
    }
}

fn run(cli: Cli) -> Result<(), String> {
    let data = fs::read(&cli.input).map_err(|e| format!("read {}: {e}", cli.input.display()))?;
    let plugin = match cli.plugin {
        Some(p) => p,
        None => detect_plugin(&cli.input)?,
    };
    let fmt = cli
        .format
        .or_else(|| cli.output.as_deref().and_then(format_of))
        .unwrap_or(Format::Png);
    let output = cli
        .output
        .clone()
        .unwrap_or_else(|| cli.input.with_extension(fmt.ext()));
    let cfg = build_config(cli.config.as_deref(), &cli.set)?;

    let raw = match plugin {
        Plugin::Maquette => render_maquette(&cli.input, &data, &cfg, fmt)?,
        Plugin::Gltf => {
            if fmt == Format::Svg {
                return Err("glTF renders to PNG only".into());
            }
            maquette_gltf::native::render_gltf(&data, &cfg)?
        }
        Plugin::Scad => {
            let text = String::from_utf8(data).map_err(|_| "scad input is not UTF-8".to_string())?;
            let ply = maquette_scad::compile_scad(&text, HashMap::new(), cli.scad_fn, HashMap::new())?;
            match fmt {
                Format::Png => maquette::native::render_ply_png(&ply, &cfg)?,
                Format::Svg => maquette::native::render_ply(&ply, &cfg)?,
            }
        }
    };

    let bytes = match fmt {
        Format::Png => encode_png(&raw)?,
        Format::Svg => raw,
    };

    fs::write(&output, &bytes).map_err(|e| format!("write {}: {e}", output.display()))?;
    eprintln!("wrote {} ({} bytes)", output.display(), bytes.len());
    Ok(())
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
            if marker == 0x02 {
                eprintln!("warning: vector overlay dropped from PNG; use --format svg to keep annotations/grid");
            }
            let mut out = Vec::new();
            {
                let mut enc = png::Encoder::new(&mut out, w, h);
                enc.set_color(png::ColorType::Rgba);
                enc.set_depth(png::BitDepth::Eight);
                let mut writer = enc.write_header().map_err(|e| format!("png: {e}"))?;
                writer.write_image_data(px).map_err(|e| format!("png: {e}"))?;
            }
            Ok(out)
        }
        0x3C => {
            eprintln!("warning: renderer returned SVG for a PNG target; writing SVG bytes");
            Ok(raw.to_vec())
        }
        m => Err(format!("unexpected render marker 0x{m:02x}")),
    }
}

fn format_of(p: &Path) -> Option<Format> {
    match ext_of(p).as_deref() {
        Some("svg") => Some(Format::Svg),
        Some("png") => Some(Format::Png),
        _ => None,
    }
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
