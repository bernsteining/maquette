//! Real `.scad` text ingestion.
//!
//! `openscad-rs` parses OpenSCAD source into an AST but does not evaluate it.
//! This module is the evaluator: it walks the AST — resolving variables, `for`
//! loops, `if`, expressions, and user `module`/`function` definitions — and emits
//! our own geometry DSL ([`crate::json::Json`] trees), which the validated,
//! hardened [`crate::build`] pipeline then turns into a mesh. So OpenSCAD's
//! *language* lives here; its *geometry* reuses everything we already built.
//!
//! Coverage is tracked in `scad/FIDELITY.md`. Supported: all primitives (2D +
//! 3D), transforms (incl. `multmatrix`/`resize`/`offset`), booleans, `hull`/
//! `minkowski`, extrudes (`linear_extrude` incl. twist/scale, `rotate_extrude`,
//! `projection`), control flow (`for`/`intersection_for`/`if`), list
//! comprehensions, user modules & functions (+ closures + `children()`),
//! `use`/`include` and `import`(stl/obj)/`text` via Typst-passed bytes, and the
//! expression language + builtin library. Not yet: `surface`, `import` dxf/svg,
//! `$fa`/`$fs`, matrix·matrix, `!` root modifier.

use crate::json::Json;
use openscad_rs::ast::{Argument, BinaryOp, Expr, ExprKind, Parameter, Statement, UnaryOp};
use rustc_hash::{FxBuildHasher, FxHashMap};
use std::collections::HashMap;
use std::rc::Rc;

/// The variable scope: a persistent map (O(1) clone per call scope) with a fast
/// non-cryptographic (Fx) hasher — keys are the user's own identifiers, never
/// adversarial, so SipHash's DoS resistance isn't needed. `archery::RcK` keeps the
/// single-threaded `Rc` sharing.
type Vars = rpds::HashTrieMap<String, Value, archery::RcK, FxBuildHasher>;

/// A runtime value in the OpenSCAD expression language.
#[derive(Clone, Debug)]
enum Value {
    Num(f64),
    Bool(bool),
    Str(String),
    Vec(Vec<Value>),
    /// A function literal: parameters, body, and captured environment (closure).
    Func(Rc<(Vec<Parameter>, Expr, Env)>),
    Undef,
}

impl Value {
    fn as_num(&self) -> Result<f64, String> {
        match self {
            Value::Num(n) => Ok(*n),
            Value::Bool(b) => Ok(if *b { 1.0 } else { 0.0 }),
            _ => Err(format!("expected a number, got {self:?}")),
        }
    }
    fn truthy(&self) -> bool {
        match self {
            Value::Bool(b) => *b,
            Value::Num(n) => *n != 0.0,
            Value::Undef => false,
            Value::Str(s) => !s.is_empty(),
            Value::Vec(v) => !v.is_empty(),
            Value::Func(_) => true,
        }
    }
    /// Convert to a JSON number/array/bool/string for embedding in the DSL.
    fn to_json(&self) -> Json {
        match self {
            Value::Num(n) => Json::Num(*n),
            Value::Bool(b) => Json::Bool(*b),
            Value::Str(s) => Json::Str(s.clone()),
            Value::Vec(v) => Json::Arr(v.iter().map(Value::to_json).collect()),
            Value::Undef | Value::Func(_) => Json::Null,
        }
    }
}

/// A user `function`: parameters + body expression.
type FuncDef = (Vec<Parameter>, Expr);
/// A user `module`: parameters + body statements.
type ModuleDef = (Vec<Parameter>, Vec<Statement>);

#[derive(Clone)]
struct Env {
    // `vars` is a persistent map: cloning an Env (every call scope) shares it in
    // O(1), and a binding copies only the changed trie path. `funcs`/`modules` are
    // effectively write-once (top-level defs / library imports) and Rc-shared, with
    // copy-on-write on a nested `function`/`module` definition.
    vars: Vars,
    funcs: Rc<FxHashMap<String, FuncDef>>,
    modules: Rc<FxHashMap<String, ModuleDef>>,
    /// Geometry passed as `{ … }` to the current module, for `children()`.
    /// Rc-shared so cloning an Env (every module/function/let/for scope) is a
    /// refcount bump, not a deep copy of the geometry vector.
    children: Rc<Vec<Json>>,
    /// `.scad` files supplied from Typst, for `use <>` / `include <>`.
    files: Rc<HashMap<String, String>>,
    /// Directory of the file currently being evaluated (for relative use/include).
    cur_dir: String,
    /// Guards against `use`/`include` cycles / runaway nesting.
    include_depth: usize,
}

// Manual Debug: the Fx `BuildHasher` in `Vars` isn't `Debug`, and a full var dump
// is noise anyway — summarize sizes instead. (Needed because `Value::Func` holds an
// `Env` closure and `Value` derives `Debug`.)
impl std::fmt::Debug for Env {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Env")
            .field("vars", &self.vars.size())
            .field("funcs", &self.funcs.len())
            .field("modules", &self.modules.len())
            .field("cur_dir", &self.cur_dir)
            .finish_non_exhaustive()
    }
}

impl Env {
    fn new() -> Self {
        Env {
            vars: Vars::new_with_hasher_and_ptr_kind(FxBuildHasher),
            funcs: Rc::new(FxHashMap::default()),
            modules: Rc::new(FxHashMap::default()),
            children: Rc::new(Vec::new()),
            files: Rc::new(HashMap::new()),
            cur_dir: String::new(),
            include_depth: 0,
        }
    }
    fn def_module(&mut self, name: String, params: Vec<Parameter>, body: Vec<Statement>) {
        Rc::make_mut(&mut self.modules).insert(name, (params, body));
    }
    fn def_func(&mut self, name: String, params: Vec<Parameter>, body: Expr) {
        Rc::make_mut(&mut self.funcs).insert(name, (params, body));
    }
    fn fn_default(&self) -> Option<f64> {
        self.vars.get("$fn").and_then(|v| v.as_num().ok()).filter(|n| *n >= 3.0)
    }
}

/// Strip C-style block comments, comment/string-aware, preserving newlines.
///
/// `openscad-rs` mishandles any block comment whose terminator is `**/` (a run of
/// `*` immediately before `*/`): it silently swallows the rest of the file and
/// returns an EMPTY parse (no error). Projects like dotSCAD open every file with a
/// `/** … **/` doc-comment, so their whole libraries parsed to nothing. We remove
/// block comments ourselves before parsing — `//` line comments and string
/// literals are left intact, and newlines are kept so error line numbers stay put.
fn strip_block_comments(src: &str) -> String {
    let b = src.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'"' => {
                out.push(b'"');
                i += 1;
                while i < b.len() {
                    let ch = b[i];
                    out.push(ch);
                    i += 1;
                    if ch == b'\\' && i < b.len() {
                        out.push(b[i]);
                        i += 1;
                    } else if ch == b'"' {
                        break;
                    }
                }
            }
            b'/' if i + 1 < b.len() && b[i + 1] == b'/' => {
                while i < b.len() && b[i] != b'\n' {
                    out.push(b[i]);
                    i += 1;
                }
            }
            b'/' if i + 1 < b.len() && b[i + 1] == b'*' => {
                i += 2;
                while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                    if b[i] == b'\n' {
                        out.push(b'\n');
                    }
                    i += 1;
                }
                i = (i + 2).min(b.len()); // consume the closing */
                out.push(b' ');
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| src.to_string())
}

/// Parse `.scad` source, working around the `openscad-rs` block-comment bug.
fn parse_scad(src: &str) -> Result<openscad_rs::ast::SourceFile, String> {
    let cleaned = strip_block_comments(src);
    openscad_rs::parse(&cleaned).map_err(|e| format!("{e}"))
}

/// Parse + evaluate `.scad` source into a single DSL tree (union of all
/// top-level geometry).
pub fn scad_to_dsl(src: &str, files: HashMap<String, String>) -> Result<Json, String> {
    let sf = parse_scad(src).map_err(|e| format!("scad parse error: {e}"))?;
    let mut env = Env::new();
    env.files = Rc::new(files);
    // Built-in special variables/constants (OpenSCAD defaults). $fn=0 means
    // "resolve tessellation from $fa/$fs" — libraries like dotSCAD read these
    // directly (e.g. its __frags() helper), so they must be defined. Our own
    // primitive builder still falls back to the caller's `fn` default when a
    // node carries no explicit $fn (fn_default() filters out $fn<3, incl. 0).
    env.vars.insert_mut("PI".into(), Value::Num(std::f64::consts::PI));
    env.vars.insert_mut("$t".into(), Value::Num(0.0)); // animation time (static render)
    env.vars.insert_mut("$preview".into(), Value::Bool(true));
    env.vars.insert_mut("$fn".into(), Value::Num(0.0));
    env.vars.insert_mut("$fa".into(), Value::Num(12.0));
    env.vars.insert_mut("$fs".into(), Value::Num(2.0));
    let nodes = eval_body(&sf.statements, &mut env)?;
    group(nodes).ok_or_else(|| "scad: no geometry produced".into())
}

/// Resolve a `use`/`include` path against the files map. Real projects reference
/// the same library by different relative paths depending on which file does the
/// `use` (e.g. `libs/MCAD/x.scad` from the root, `MCAD/x.scad` from a sibling),
/// so we match leniently: exact first, then the longest key that ends with the
/// requested path, then by basename.
fn dir_of(key: &str) -> String {
    match key.rfind('/') {
        Some(i) => key[..i].to_string(),
        None => String::new(),
    }
}
fn path_join(dir: &str, p: &str) -> String {
    let p = p.trim_start_matches("./");
    if dir.is_empty() {
        p.to_string()
    } else {
        format!("{dir}/{p}")
    }
}

/// Resolve a `use`/`include` path to (key, contents). Tries, in order: relative
/// to `cur_dir` (correct OpenSCAD behavior), exact, longest full-path suffix,
/// then basename — so real projects with per-directory relative refs resolve
/// unambiguously (e.g. `libs/x` sees `use <gears.scad>` as `libs/gears.scad`).
fn resolve_file<'a>(files: &'a HashMap<String, String>, cur_dir: &str, path: &str) -> Option<(String, &'a String)> {
    let p = path.trim_start_matches("./");
    let rel = path_join(cur_dir, p);
    if let Some(v) = files.get(&rel) {
        return Some((rel, v));
    }
    if let Some(v) = files.get(p) {
        return Some((p.to_string(), v));
    }
    let base = p.rsplit('/').next().unwrap_or(p);
    let mut best: Option<(&String, &String, usize)> = None;
    for (k, v) in files {
        let score = if k == p || k.ends_with(&format!("/{p}")) {
            1_000_000 + k.len()
        } else if k == base || k.ends_with(&format!("/{base}")) {
            k.len()
        } else {
            continue;
        };
        if best.is_none_or(|(_, _, s)| score > s) {
            best = Some((k, v, score));
        }
    }
    best.map(|(k, v, _)| (k.clone(), v))
}

/// Parse a `use`/`include` target. Returns the resolved file's directory (for
/// nested relative resolution) and its statements.
fn load_scad_file(path: &str, env: &Env) -> Result<(String, Vec<Statement>), String> {
    if env.include_depth > 64 {
        return Err("scad: use/include nested too deep (cycle?)".into());
    }
    let (key, src) = resolve_file(&env.files, &env.cur_dir, path).ok_or_else(|| {
        format!("scad: file \"{path}\" for use/include not provided — pass it in `files`")
    })?;
    let stmts = parse_scad(src)
        .map_err(|e| format!("scad parse error in \"{path}\": {e}"))?
        .statements;
    Ok((dir_of(&key), stmts))
}

/// Merge a file's definitions into `env`. `with_vars` = include semantics (also
/// import top-level variables); false = use semantics (modules & functions only).
fn import_defs(path: &str, env: &mut Env, with_vars: bool) -> Result<(), String> {
    let (dir, stmts) = load_scad_file(path, env)?;
    let saved_dir = std::mem::replace(&mut env.cur_dir, dir);
    env.include_depth += 1;
    let r = (|| {
        for s in &stmts {
            match s {
                Statement::ModuleDefinition { name, params, body, .. } => {
                    env.def_module(name.clone(), params.clone(), body.clone());
                }
                Statement::FunctionDefinition { name, params, body, .. } => {
                    env.def_func(name.clone(), params.clone(), body.clone());
                }
                Statement::Assignment { name, expr, .. } if with_vars => {
                    let v = eval_expr(expr, env)?;
                    env.vars.insert_mut(name.clone(), v);
                }
                // `use` imports the file's top-level variables too — real
                // libraries rely on their own config globals being visible to
                // their modules (OpenSCAD keeps them in the module's file scope).
                Statement::Use { path, .. } => import_defs(path, env, true)?,
                Statement::Include { path, .. } => import_defs(path, env, with_vars)?,
                _ => {}
            }
        }
        Ok(())
    })();
    env.include_depth -= 1;
    env.cur_dir = saved_dir;
    r
}

/// Evaluate a statement list, threading assignments/definitions into `env` and
/// collecting the geometry nodes produced.
fn eval_body(stmts: &[Statement], env: &mut Env) -> Result<Vec<Json>, String> {
    // OpenSCAD hoists definitions and uses last-assignment-wins within a scope;
    // we approximate with a pre-pass for defs + assignments, then a geometry
    // pass. Good enough for the vast majority of real files.
    for s in stmts {
        match s {
            Statement::Assignment { name, expr, .. } => {
                let v = eval_expr(expr, env)?;
                env.vars.insert_mut(name.clone(), v);
            }
            Statement::ModuleDefinition { name, params, body, .. } => {
                env.def_module(name.clone(), params.clone(), body.clone());
            }
            Statement::FunctionDefinition { name, params, body, .. } => {
                env.def_func(name.clone(), params.clone(), body.clone());
            }
            // `use <lib>` imports modules, functions AND the lib's top-level
            // variables (its modules depend on them); `include <lib>` also runs
            // the lib's top-level geometry (handled in the geometry pass below).
            Statement::Use { path, .. } => import_defs(path, env, true)?,
            Statement::Include { path, .. } => import_defs(path, env, true)?,
            _ => {}
        }
    }
    let mut out = Vec::new();
    for s in stmts {
        match s {
            // `include <lib>` also contributes the library's top-level geometry.
            Statement::Include { path, .. } => {
                let (dir, file_stmts) = load_scad_file(path, env)?;
                let saved_dir = std::mem::replace(&mut env.cur_dir, dir);
                env.include_depth += 1;
                let g = eval_body(&file_stmts, env);
                env.include_depth -= 1;
                env.cur_dir = saved_dir;
                out.extend(g?);
            }
            Statement::ModuleInstantiation { name, args, children, modifiers, .. } => {
                if modifiers.disable {
                    continue; // `*` disables the subtree
                }
                if let Some(node) = instantiate(name, args, children, env)? {
                    // `%` (background) → translucent gray ghost; `#` (highlight) →
                    // translucent red. Mirrors OpenSCAD's preview modifiers.
                    let node = if modifiers.background {
                        ghost(node, [0.6, 0.6, 0.6], 0.22)
                    } else if modifiers.highlight {
                        ghost(node, [0.95, 0.25, 0.25], 0.5)
                    } else {
                        node
                    };
                    out.push(node);
                }
            }
            Statement::IfElse { condition, then_body, else_body, .. } => {
                let branch = if eval_expr(condition, env)?.truthy() {
                    Some(then_body)
                } else {
                    else_body.as_ref()
                };
                if let Some(b) = branch {
                    out.extend(eval_body(b, env)?);
                }
            }
            Statement::Block { body, .. } => out.extend(eval_body(body, env)?),
            _ => {}
        }
    }
    Ok(out)
}

/// Wrap 0/1/many geometry nodes: None / the node / an implicit union.
fn group(mut nodes: Vec<Json>) -> Option<Json> {
    match nodes.len() {
        0 => None,
        1 => Some(nodes.pop().unwrap()),
        _ => Some(obj(&[("op", Json::Str("union".into())), ("children", Json::Arr(nodes))])),
    }
}

/// Wrap a node in a translucent color (for `%`/`#` preview modifiers).
fn ghost(node: Json, rgb: [f64; 3], alpha: f64) -> Json {
    obj(&[
        ("op", Json::Str("color".into())),
        ("rgb", jvec3(rgb)),
        ("alpha", jnum(alpha)),
        ("child", node),
    ])
}

/// Build a JSON object from key/value pairs.
fn obj(entries: &[(&str, Json)]) -> Json {
    Json::Obj(entries.iter().map(|(k, v)| (k.to_string(), v.clone())).collect())
}

/// Like `obj` but takes ownership of the values (no clone).
fn obj_owned(entries: Vec<(&str, Json)>) -> Json {
    Json::Obj(entries.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}
fn jnum(x: f64) -> Json {
    Json::Num(x)
}
fn jvec3(v: [f64; 3]) -> Json {
    Json::Arr(vec![jnum(v[0]), jnum(v[1]), jnum(v[2])])
}

/// Resolved call arguments: positional list + named map (both evaluated).
struct Args {
    pos: Vec<Value>,
    named: HashMap<String, Value>,
}
impl Args {
    fn get(&self, i: usize, name: &str) -> Option<&Value> {
        self.named.get(name).or_else(|| self.pos.get(i))
    }
    fn num(&self, i: usize, name: &str) -> Option<f64> {
        self.get(i, name).and_then(|v| v.as_num().ok())
    }
    /// A named-only numeric argument (no positional fallback).
    fn named_num(&self, name: &str) -> Option<f64> {
        self.named.get(name).and_then(|v| v.as_num().ok())
    }
    fn bool(&self, name: &str) -> Option<bool> {
        self.named.get(name).map(Value::truthy)
    }
}

fn eval_args(args: &[Argument], env: &Env) -> Result<Args, String> {
    let mut pos = Vec::new();
    let mut named = HashMap::new();
    for a in args {
        let v = eval_expr(&a.value, env)?;
        match &a.name {
            Some(n) => {
                named.insert(n.clone(), v);
            }
            None => pos.push(v),
        }
    }
    Ok(Args { pos, named })
}

/// A vector value -> [f64; 3] (missing components 0). Scalars broadcast to x.
fn vec3_of(v: &Value) -> Result<[f64; 3], String> {
    match v {
        Value::Vec(items) => {
            let mut out = [0.0; 3];
            for (i, it) in items.iter().take(3).enumerate() {
                out[i] = it.as_num()?;
            }
            Ok(out)
        }
        Value::Num(n) => Ok([*n, 0.0, 0.0]),
        _ => Err(format!("expected a vector, got {v:?}")),
    }
}

/// `r` directly, or `d`/2 (diameter). Returns None if neither present.
fn radius(a: &Args, env: &Env) -> Option<f64> {
    a.num(0, "r").or_else(|| a.named_num("d").map(|d| d / 2.0)).or_else(|| {
        // allow bare positional for r on sphere/circle
        let _ = env;
        None
    })
}
fn seg_arg(a: &Args, env: &Env) -> Option<f64> {
    a.named
        .get("$fn")
        .or_else(|| a.named.get("fn"))
        .and_then(|v| v.as_num().ok())
        .filter(|n| *n >= 3.0)
        .or_else(|| env.fn_default())
}
/// Attach a `fn` key if a facet count is resolvable.
fn with_fn(mut entries: Vec<(&str, Json)>, a: &Args, env: &Env) -> Json {
    if let Some(f) = seg_arg(a, env) {
        entries.push(("fn", jnum(f)));
    }
    obj_owned(entries)
}

/// Instantiate a module call -> an optional DSL geometry node.
fn instantiate(
    name: &str,
    args: &[Argument],
    children: &[Statement],
    env: &mut Env,
) -> Result<Option<Json>, String> {
    // `for`/`if`/`let` bind before their children are evaluated, so they read
    // raw args, not pre-evaluated ones.
    match name {
        "for" => return eval_for(args, children, env),
        "intersection_for" => return eval_intersection_for(args, children, env),
        "if" => {
            // `if` can appear as a module instantiation too.
            let a = eval_args(args, env)?;
            let cond = a.pos.first().map(Value::truthy).unwrap_or(false);
            return Ok(if cond { group(eval_body(children, env)?) } else { None });
        }
        // Diagnostics: no geometry, just render any children (e.g. `echo(x) cube();`).
        "echo" | "assert" => return Ok(group(eval_body(children, env)?)),
        // `let(a=…) { … }` and the deprecated `assign(a=…) { … }` — bind the
        // (evaluated) named args as locals, then render the children in that scope.
        "let" | "assign" => {
            let a = eval_args(args, env)?;
            let mut local = env.clone();
            for (k, v) in &a.named {
                local.vars.insert_mut(k.clone(), v.clone());
            }
            return Ok(group(eval_body(children, &mut local)?));
        }
        // `children()` / `children(i)` / `children([i,j,…])` — the geometry the
        // current module was called with (set by instantiate_user).
        "children" | "child" => {
            let a = eval_args(args, env)?;
            let kids = env.children.as_ref().clone();
            return Ok(match a.pos.first() {
                None => group(kids),
                Some(Value::Num(n)) => kids.get(*n as usize).cloned(),
                Some(Value::Vec(idxs)) => {
                    let sel: Vec<Json> = idxs
                        .iter()
                        .filter_map(|v| v.as_num().ok())
                        .filter_map(|i| kids.get(i as usize).cloned())
                        .collect();
                    group(sel)
                }
                _ => group(kids),
            });
        }
        _ => {}
    }

    let a = eval_args(args, env)?;

    let node = match name {
        // ---- 3D primitives ----
        "cube" => {
            let size = match a.get(0, "size") {
                Some(Value::Vec(_)) => vec3_of(a.get(0, "size").unwrap())?,
                Some(Value::Num(n)) => [*n, *n, *n],
                _ => [1.0, 1.0, 1.0],
            };
            let center = a.bool("center").unwrap_or(false);
            obj(&[("op", Json::Str("cube".into())), ("size", jvec3(size)), ("center", Json::Bool(center))])
        }
        "sphere" => {
            // OpenSCAD defaults an argument-less primitive to size 1.
            let r = radius(&a, env).unwrap_or(1.0);
            with_fn(vec![("op", Json::Str("sphere".into())), ("r", jnum(r))], &a, env)
        }
        "cylinder" => {
            let h = a.num(0, "h").unwrap_or(1.0);
            let center = a.bool("center").unwrap_or(false);
            let mut e = vec![("op", Json::Str("cylinder".into())), ("h", jnum(h)), ("center", Json::Bool(center))];
            let r1 = a.named_num("r1")
                .or_else(|| a.named_num("d1").map(|d| d / 2.0));
            let r2 = a.named_num("r2")
                .or_else(|| a.named_num("d2").map(|d| d / 2.0));
            if r1.is_some() || r2.is_some() {
                e.push(("r1", jnum(r1.unwrap_or(0.0))));
                e.push(("r2", jnum(r2.unwrap_or(0.0))));
            } else {
                let r = a.num(1, "r").or_else(|| a.named_num("d").map(|d| d / 2.0))
                    .unwrap_or(1.0);
                e.push(("r", jnum(r)));
            }
            with_fn(e, &a, env)
        }
        "polyhedron" => {
            let points = a.get(0, "points").ok_or("polyhedron: needs points")?.to_json();
            let faces = a.get(1, "faces").ok_or("polyhedron: needs faces")?.to_json();
            obj(&[("op", Json::Str("polyhedron".into())), ("points", points), ("faces", faces)])
        }

        // ---- 2D primitives ----
        "square" => {
            let size = a.get(0, "size").cloned().unwrap_or(Value::Num(1.0));
            let center = a.bool("center").unwrap_or(false);
            let sj = match size {
                Value::Vec(_) => size.to_json(),
                Value::Num(n) => jnum(n),
                _ => jnum(1.0),
            };
            obj(&[("op", Json::Str("square".into())), ("size", sj), ("center", Json::Bool(center))])
        }
        "circle" => {
            let r = radius(&a, env).unwrap_or(1.0);
            with_fn(vec![("op", Json::Str("circle".into())), ("r", jnum(r))], &a, env)
        }
        "import" => {
            let file = match a.get(0, "file") {
                Some(Value::Str(s)) => s.clone(),
                _ => return Err("import: needs a file name (string)".into()),
            };
            obj(&[("op", Json::Str("import".into())), ("file", Json::Str(file))])
        }
        "text" => {
            let t = match a.get(0, "text") {
                Some(v) => value_to_string(v),
                None => return Err("text: needs a string".into()),
            };
            let size = a.num(1, "size").unwrap_or(10.0);
            obj(&[
                ("op", Json::Str("text".into())),
                ("text", Json::Str(t)),
                ("size", jnum(size)),
            ])
        }
        "polygon" => {
            let points = a.get(0, "points").ok_or("polygon: needs points")?.to_json();
            let mut e = vec![("op", Json::Str("polygon".into())), ("points", points)];
            if let Some(p) = a.get(1, "paths") {
                if !matches!(p, Value::Undef) {
                    e.push(("paths", p.to_json()));
                }
            }
            obj_owned(e)
        }

        // ---- transforms (a transform of nothing is nothing) ----
        "translate" => return wrap_transform("translate", "v", vec3_of(a.get(0, "v").ok_or("translate: v")?)?, group(eval_body(children, env)?)),
        "scale" => {
            let v = match a.get(0, "v").ok_or("scale: v")? {
                Value::Num(n) => [*n, *n, *n],
                other => vec3_of(other)?,
            };
            return wrap_transform("scale", "v", v, group(eval_body(children, env)?));
        }
        "mirror" => return wrap_transform("mirror", "v", vec3_of(a.get(0, "v").ok_or("mirror: v")?)?, group(eval_body(children, env)?)),
        "multmatrix" => {
            let m = a.get(0, "m").ok_or("multmatrix: m")?.to_json();
            let child = match group(eval_body(children, env)?) { Some(c) => c, None => return Ok(None) };
            obj(&[("op", Json::Str("multmatrix".into())), ("m", m), ("child", child)])
        }
        "resize" => {
            let v = vec3_of(a.get(0, "newsize").ok_or("resize: newsize")?)?;
            let child = match group(eval_body(children, env)?) { Some(c) => c, None => return Ok(None) };
            obj(&[("op", Json::Str("resize".into())), ("v", jvec3(v)), ("child", child)])
        }
        "offset" => {
            // offset(r=…) or offset(delta=…) → a single signed distance.
            let dist = a
                .num(0, "r")
                .or_else(|| a.named.get("delta").and_then(|x| x.as_num().ok()))
                .ok_or("offset: needs r or delta")?;
            let child = match group(eval_body(children, env)?) { Some(c) => c, None => return Ok(None) };
            obj(&[("op", Json::Str("offset".into())), ("d", jnum(dist)), ("child", child)])
        }
        "projection" => {
            // Shadow projection onto Z=0 (the `cut` flag is not distinguished).
            let child = match group(eval_body(children, env)?) { Some(c) => c, None => return Ok(None) };
            obj(&[("op", Json::Str("projection".into())), ("child", child)])
        }
        "rotate" => {
            // scalar => rotate about Z; vector => euler xyz.
            let deg = match a.get(0, "a").or_else(|| a.get(0, "v")).ok_or("rotate: a")? {
                Value::Num(n) => [0.0, 0.0, *n],
                other => vec3_of(other)?,
            };
            return wrap_transform("rotate", "deg", deg, group(eval_body(children, env)?));
        }
        "color" => {
            // color(c) where c is [r,g,b], [r,g,b,a], or a name; optional 2nd arg
            // / `alpha=` sets/overrides alpha (OpenSCAD's color(c, alpha) form).
            let cval = a.get(0, "c");
            let rgb = match cval {
                Some(Value::Vec(_)) => vec3_of(cval.unwrap())?,
                Some(Value::Str(s)) => named_color(s),
                _ => [0.85, 0.85, 0.85],
            };
            let alpha = a
                .num(1, "alpha")
                .or_else(|| match cval {
                    // 4th component of an [r,g,b,a] vector
                    Some(Value::Vec(v)) if v.len() >= 4 => v[3].as_num().ok(),
                    _ => None,
                })
                .unwrap_or(1.0);
            let child = match group(eval_body(children, env)?) { Some(c) => c, None => return Ok(None) };
            let mut e = vec![
                ("op", Json::Str("color".into())),
                ("rgb", jvec3(rgb)),
                ("child", child),
            ];
            if alpha < 1.0 {
                e.push(("alpha", jnum(alpha)));
            }
            obj_owned(e)
        }

        // ---- extrudes ----
        "linear_extrude" => {
            let h = a.num(0, "height").or_else(|| a.num(0, "h")).ok_or("linear_extrude: height")?;
            let center = a.bool("center").unwrap_or(false);
            let child = match group(eval_body(children, env)?) { Some(c) => c, None => return Ok(None) };
            let mut e = vec![
                ("op", Json::Str("linear_extrude".into())),
                ("h", jnum(h)),
                ("center", Json::Bool(center)),
                ("child", child),
            ];
            if let Some(t) = a.named_num("twist") {
                if t != 0.0 {
                    e.push(("twist", jnum(t)));
                }
            }
            if let Some(s) = a.named_num("scale") {
                if s != 1.0 {
                    e.push(("scale", jnum(s)));
                }
            }
            if let Some(sl) = a.named_num("slices") {
                e.push(("slices", jnum(sl)));
            }
            obj_owned(e)
        }
        "rotate_extrude" => {
            let angle = a.num(0, "angle").unwrap_or(360.0);
            let child = match group(eval_body(children, env)?) { Some(c) => c, None => return Ok(None) };
            with_fn(vec![("op", Json::Str("rotate_extrude".into())), ("angle", jnum(angle)), ("child", child)], &a, env)
        }

        // ---- booleans / grouping ----
        "union" | "difference" | "intersection" | "hull" | "minkowski" => {
            let kids = eval_body(children, env)?;
            if kids.is_empty() {
                return Ok(None);
            }
            obj(&[("op", Json::Str(name.into())), ("children", Json::Arr(kids))])
        }
        // `render()` (force-CGAL) and bare `{ }` groups are just grouping here.
        "group" | "render" => return Ok(group(eval_body(children, env)?)),

        // ---- user-defined module ----
        other => {
            if let Some((params, body)) = env.modules.get(other).cloned() {
                // The `{ … }` block passed to the module is evaluated in the
                // CALLER's scope and made available via `children()`.
                let child_nodes = eval_body(children, env)?;
                return instantiate_user(&params, &body, &a, env, child_nodes);
            }
            return Err(format!("scad: unsupported module \"{other}\""));
        }
    };
    Ok(Some(node))
}

fn wrap_transform(op: &str, key: &str, v: [f64; 3], child: Option<Json>) -> Result<Option<Json>, String> {
    // A transform of nothing is nothing (OpenSCAD semantics).
    Ok(child.map(|c| obj(&[("op", Json::Str(op.into())), (key, jvec3(v)), ("child", c)])))
}

/// `for (i = range|vector) children` -> union of the expansions.
/// Expand a `for`/`intersection_for` over its (possibly multiple, cartesian)
/// bindings into the flat list of child geometry nodes.
fn for_expand(args: &[Argument], children: &[Statement], env: &mut Env) -> Result<Vec<Json>, String> {
    fn rec(
        args: &[Argument],
        idx: usize,
        children: &[Statement],
        env: &mut Env,
        out: &mut Vec<Json>,
    ) -> Result<(), String> {
        if idx == args.len() {
            out.extend(eval_body(children, env)?);
            return Ok(());
        }
        let a = &args[idx];
        let name = a.name.clone().ok_or("for: binding needs a name")?;
        for item in eval_iterable(&a.value, env)? {
            let mut e = env.clone();
            e.vars.insert_mut(name.clone(), item);
            rec(args, idx + 1, children, &mut e, out)?;
        }
        Ok(())
    }
    let mut out = Vec::new();
    rec(args, 0, children, env, &mut out)?;
    Ok(out)
}

/// `for(...) { ... }` → implicit union of the expansions.
fn eval_for(args: &[Argument], children: &[Statement], env: &mut Env) -> Result<Option<Json>, String> {
    Ok(group(for_expand(args, children, env)?))
}

/// `intersection_for(...) { ... }` → intersection of the expansions.
fn eval_intersection_for(args: &[Argument], children: &[Statement], env: &mut Env) -> Result<Option<Json>, String> {
    let kids = for_expand(args, children, env)?;
    if kids.is_empty() {
        return Ok(None);
    }
    Ok(Some(obj(&[("op", Json::Str("intersection".into())), ("children", Json::Arr(kids))])))
}

/// Evaluate an expression to a list of values (for `for` / ranges / vectors).
/// Evaluate an expression that should yield a sequence (for `for`/comprehension
/// bindings). Ranges and vectors (incl. comprehensions) both materialize to a
/// list via `eval_expr`; a scalar yields a 1-element list.
fn eval_iterable(expr: &Expr, env: &Env) -> Result<Vec<Value>, String> {
    match eval_expr(expr, env)? {
        Value::Vec(v) => Ok(v),
        other => Ok(vec![other]),
    }
}

/// Expand one element of a vector literal into `out`, handling list-comprehension
/// forms (`for`/`if`/`let`/`each`/C-style). A plain expression pushes one value.
fn flatten_lc(expr: &Expr, env: &Env, out: &mut Vec<Value>) -> Result<(), String> {
    match &expr.kind {
        ExprKind::LcFor { assignments, body } => lc_for(assignments, 0, body, env, out),
        ExprKind::LcForC { init, condition, update, body } => {
            let mut local = env.clone();
            for a in init {
                if let Some(n) = &a.name {
                    let v = eval_expr(&a.value, &local)?;
                    local.vars.insert_mut(n.clone(), v);
                }
            }
            let mut guard = 0;
            while eval_expr(condition, &local)?.truthy() && guard < 1_000_000 {
                flatten_lc(body, &local, out)?;
                for a in update {
                    if let Some(n) = &a.name {
                        let v = eval_expr(&a.value, &local)?;
                        local.vars.insert_mut(n.clone(), v);
                    }
                }
                guard += 1;
            }
            Ok(())
        }
        ExprKind::LcIf { condition, then_expr, else_expr } => {
            if eval_expr(condition, env)?.truthy() {
                flatten_lc(then_expr, env, out)
            } else if let Some(e) = else_expr {
                flatten_lc(e, env, out)
            } else {
                Ok(())
            }
        }
        ExprKind::LcLet { assignments, body } => {
            let mut local = env.clone();
            for a in assignments {
                if let Some(n) = &a.name {
                    let v = eval_expr(&a.value, &local)?;
                    local.vars.insert_mut(n.clone(), v);
                }
            }
            flatten_lc(body, &local, out)
        }
        ExprKind::LcEach { body } => {
            match eval_expr(body, env)? {
                Value::Vec(v) => out.extend(v),
                other => out.push(other),
            }
            Ok(())
        }
        _ => {
            out.push(eval_expr(expr, env)?);
            Ok(())
        }
    }
}

/// Cartesian binding for `[for (a=…, b=…) …]`.
fn lc_for(assignments: &[Argument], idx: usize, body: &Expr, env: &Env, out: &mut Vec<Value>) -> Result<(), String> {
    if idx == assignments.len() {
        return flatten_lc(body, env, out);
    }
    let a = &assignments[idx];
    let name = a.name.clone().ok_or("list comprehension `for`: binding needs a name")?;
    for item in eval_iterable(&a.value, env)? {
        let mut e = env.clone();
        e.vars.insert_mut(name.clone(), item);
        lc_for(assignments, idx + 1, body, &e, out)?;
    }
    Ok(())
}

/// Instantiate a user module: bind params (defaults + call args), eval its body.
fn instantiate_user(
    params: &[Parameter],
    body: &[Statement],
    a: &Args,
    env: &Env,
    child_nodes: Vec<Json>,
) -> Result<Option<Json>, String> {
    let mut local = env.clone();
    bind_params(params, a, &mut local)?;
    local.vars.insert_mut("$children".into(), Value::Num(child_nodes.len() as f64));
    local.children = Rc::new(child_nodes);
    let nodes = eval_body(body, &mut local)?;
    Ok(group(nodes))
}

fn bind_params(params: &[Parameter], a: &Args, env: &mut Env) -> Result<(), String> {
    for (i, p) in params.iter().enumerate() {
        let v = if let Some(named) = a.named.get(&p.name) {
            named.clone()
        } else if let Some(pos) = a.pos.get(i) {
            pos.clone()
        } else if let Some(def) = &p.default {
            eval_expr(def, env)?
        } else {
            Value::Undef
        };
        env.vars.insert_mut(p.name.clone(), v);
    }
    Ok(())
}

// ---- expression evaluation ----

fn eval_expr(expr: &Expr, env: &Env) -> Result<Value, String> {
    match &expr.kind {
        ExprKind::Number(n) => Ok(Value::Num(*n)),
        ExprKind::String(s) => Ok(Value::Str(s.clone())),
        ExprKind::BoolTrue => Ok(Value::Bool(true)),
        ExprKind::BoolFalse => Ok(Value::Bool(false)),
        ExprKind::Undef => Ok(Value::Undef),
        // OpenSCAD treats a read of an undefined variable as `undef` (with a
        // warning), not an error — real code and libraries rely on this.
        ExprKind::Identifier(name) => Ok(env.vars.get(name).cloned().unwrap_or(Value::Undef)),
        ExprKind::Vector(items) => {
            // Vectors and list comprehensions share this path: each element is
            // flattened, so `for`/`if`/`let`/`each` elements expand in place.
            let mut out = Vec::new();
            for it in items {
                flatten_lc(it, env, &mut out)?;
            }
            Ok(Value::Vec(out))
        }
        ExprKind::LcFor { .. }
        | ExprKind::LcForC { .. }
        | ExprKind::LcIf { .. }
        | ExprKind::LcLet { .. }
        | ExprKind::LcEach { .. } => {
            let mut out = Vec::new();
            flatten_lc(expr, env, &mut out)?;
            Ok(Value::Vec(out))
        }
        ExprKind::Range { start, step, end } => {
            // A range used as a value materializes to a vector.
            Ok(Value::Vec(expand_range(start, step.as_deref(), end, env)?))
        }
        ExprKind::UnaryOp { op, operand } => {
            let v = eval_expr(operand, env)?;
            match op {
                // Unary -/+ negate/copy a number OR every element of a vector/matrix.
                UnaryOp::Negate => match &v {
                    Value::Vec(a) => Ok(scale_seq(a, -1.0)),
                    _ => Ok(Value::Num(-v.as_num()?)),
                },
                UnaryOp::Plus => match &v {
                    Value::Vec(_) => Ok(v),
                    _ => Ok(Value::Num(v.as_num()?)),
                },
                UnaryOp::Not => Ok(Value::Bool(!v.truthy())),
                UnaryOp::BinaryNot => Ok(Value::Num(!(v.as_num()? as i64) as f64)),
            }
        }
        ExprKind::BinaryOp { op, left, right } => eval_binop(*op, left, right, env),
        ExprKind::Ternary { condition, then_expr, else_expr } => {
            if eval_expr(condition, env)?.truthy() {
                eval_expr(then_expr, env)
            } else {
                eval_expr(else_expr, env)
            }
        }
        ExprKind::Index { object, index } => {
            // OpenSCAD: out-of-range / non-indexable / non-numeric index → undef.
            let o = eval_expr(object, env)?;
            let i = match eval_expr(index, env)? {
                Value::Num(n) if n >= 0.0 && n.is_finite() => n as usize,
                _ => return Ok(Value::Undef),
            };
            Ok(match o {
                Value::Vec(v) => v.get(i).cloned().unwrap_or(Value::Undef),
                Value::Str(s) => s.chars().nth(i).map(|c| Value::Str(c.to_string())).unwrap_or(Value::Undef),
                _ => Value::Undef,
            })
        }
        ExprKind::MemberAccess { object, member } => {
            // `.x/.y/.z` (and `.r/.g/.b`) → index into a vector; else undef.
            let idx = match member.as_str() {
                "x" | "r" => 0,
                "y" | "g" => 1,
                "z" | "b" => 2,
                _ => return Ok(Value::Undef),
            };
            Ok(match eval_expr(object, env)? {
                Value::Vec(v) => v.get(idx).cloned().unwrap_or(Value::Undef),
                _ => Value::Undef,
            })
        }
        ExprKind::FunctionCall { callee, args } => eval_call(callee, args, env),
        ExprKind::Let { assignments, body } => {
            let mut local = env.clone();
            for a in assignments {
                if let Some(n) = &a.name {
                    let v = eval_expr(&a.value, &local)?;
                    local.vars.insert_mut(n.clone(), v);
                }
            }
            eval_expr(body, &local)
        }
        ExprKind::AnonymousFunction { params, body } => {
            // Capture the current environment (closure).
            Ok(Value::Func(Rc::new((params.clone(), (**body).clone(), env.clone()))))
        }
        // `assert(cond) expr` / `echo(...) expr` in expression position: we don't
        // enforce the assertion or print, just evaluate the trailing expression
        // (undef if there is none). This matches how they thread through functions.
        ExprKind::Assert { body, .. } | ExprKind::Echo { body, .. } => match body {
            Some(b) => eval_expr(b, env),
            None => Ok(Value::Undef),
        },
        other => Err(format!("scad: unsupported expression {other:?}")),
    }
}

/// Call a function *value* (from a literal or a variable) with call-site args.
fn call_value(f: Value, args: &[Argument], caller_env: &Env) -> Result<Value, String> {
    match f {
        Value::Func(rc) => {
            let (params, body, captured) = &*rc;
            let a = eval_args(args, caller_env)?; // args evaluated in caller scope
            let mut local = captured.clone();
            bind_params(params, &a, &mut local)?;
            eval_expr(body, &local)
        }
        _ => Err("scad: attempt to call a non-function value".into()),
    }
}

fn expand_range(
    start: &Expr,
    step: Option<&Expr>,
    end: &Expr,
    env: &Env,
) -> Result<Vec<Value>, String> {
    let s = eval_expr(start, env)?.as_num()?;
    let e = eval_expr(end, env)?.as_num()?;
    let st = match step {
        Some(x) => eval_expr(x, env)?.as_num()?,
        None => 1.0,
    };
    if st == 0.0 {
        return Err("range step cannot be 0".into());
    }
    let mut out = Vec::new();
    let mut x = s;
    let mut guard = 0;
    while ((st > 0.0 && x <= e + 1e-9) || (st < 0.0 && x >= e - 1e-9)) && guard < 1_000_000 {
        out.push(Value::Num(x));
        x += st;
        guard += 1;
    }
    Ok(out)
}

fn eval_binop(op: BinaryOp, left: &Expr, right: &Expr, env: &Env) -> Result<Value, String> {
    // Short-circuit logicals.
    if matches!(op, BinaryOp::LogicalAnd) {
        return Ok(Value::Bool(eval_expr(left, env)?.truthy() && eval_expr(right, env)?.truthy()));
    }
    if matches!(op, BinaryOp::LogicalOr) {
        return Ok(Value::Bool(eval_expr(left, env)?.truthy() || eval_expr(right, env)?.truthy()));
    }
    let l = eval_expr(left, env)?;
    let r = eval_expr(right, env)?;
    // ---- OpenSCAD operator overloading ----
    match (op, &l, &r) {
        // Elementwise +/- recurse into nested vectors, so they cover matrices and
        // higher-rank arrays, not just flat vectors.
        (BinaryOp::Add, Value::Vec(a), Value::Vec(b)) => return Ok(elementwise(a, b, &|x, y| x + y)),
        (BinaryOp::Subtract, Value::Vec(a), Value::Vec(b)) => return Ok(elementwise(a, b, &|x, y| x - y)),
        // scalar · vector/matrix (recurse) and vector/matrix / scalar.
        (BinaryOp::Multiply, Value::Num(s), Value::Vec(a)) => return Ok(scale_seq(a, *s)),
        (BinaryOp::Multiply, Value::Vec(a), Value::Num(s)) => return Ok(scale_seq(a, *s)),
        (BinaryOp::Divide, Value::Vec(a), Value::Num(s)) => return Ok(scale_seq(a, 1.0 / *s)),
        // vec·vec = dot; vec×matrix, matrix×vec, matrix×matrix products.
        (BinaryOp::Multiply, Value::Vec(a), Value::Vec(b)) => return vec_mul(a, b),
        // Equality works on any types (vectors, strings, undef).
        (BinaryOp::Equal, _, _) if !both_num(&l, &r) => return Ok(Value::Bool(value_eq(&l, &r))),
        (BinaryOp::NotEqual, _, _) if !both_num(&l, &r) => return Ok(Value::Bool(!value_eq(&l, &r))),
        // Ordered comparison on strings is lexicographic; numbers fall through.
        (
            BinaryOp::Less | BinaryOp::LessEqual | BinaryOp::Greater | BinaryOp::GreaterEqual,
            Value::Str(x),
            Value::Str(y),
        ) => return Ok(Value::Bool(ordered_ok(op, x.cmp(y)))),
        _ => {}
    }
    // Any op reaching here with a non-numeric operand yields undef — OpenSCAD is
    // permissive: undef or type-mismatched operands propagate as undef, they don't
    // error. (Bool coerces to 0/1, so it counts as numeric.)
    if !numlike(&l) || !numlike(&r) {
        return Ok(Value::Undef);
    }
    let a = l.as_num()?;
    let b = r.as_num()?;
    Ok(match op {
        BinaryOp::Add => Value::Num(a + b),
        BinaryOp::Subtract => Value::Num(a - b),
        BinaryOp::Multiply => Value::Num(a * b),
        BinaryOp::Divide => Value::Num(a / b),
        BinaryOp::Modulo => Value::Num(a % b),
        BinaryOp::Exponent => Value::Num(a.powf(b)),
        BinaryOp::Equal => Value::Bool((a - b).abs() < 1e-12),
        BinaryOp::NotEqual => Value::Bool((a - b).abs() >= 1e-12),
        BinaryOp::Less => Value::Bool(a < b),
        BinaryOp::LessEqual => Value::Bool(a <= b),
        BinaryOp::Greater => Value::Bool(a > b),
        BinaryOp::GreaterEqual => Value::Bool(a >= b),
        BinaryOp::BitwiseOr => Value::Num(((a as i64) | (b as i64)) as f64),
        BinaryOp::BitwiseAnd => Value::Num(((a as i64) & (b as i64)) as f64),
        BinaryOp::ShiftLeft => Value::Num(((a as i64) << (b as i64)) as f64),
        BinaryOp::ShiftRight => Value::Num(((a as i64) >> (b as i64)) as f64),
        BinaryOp::LogicalAnd | BinaryOp::LogicalOr => unreachable!(),
    })
}

/// Numeric-ish (coerces to a number): a number or a bool (0/1).
fn numlike(v: &Value) -> bool {
    matches!(v, Value::Num(_) | Value::Bool(_))
}
fn both_num(a: &Value, b: &Value) -> bool {
    matches!((a, b), (Value::Num(_), Value::Num(_)))
}
/// Coerce to f64 (number or bool), else None.
fn to_f(v: &Value) -> Option<f64> {
    match v {
        Value::Num(n) => Some(*n),
        Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        _ => None,
    }
}
/// Recursive elementwise binary op over two sequences (min length). Recurses into
/// nested vectors (so it covers matrices); non-numeric / mismatched elements → undef.
fn elementwise(a: &[Value], b: &[Value], f: &dyn Fn(f64, f64) -> f64) -> Value {
    let n = a.len().min(b.len());
    let out = (0..n)
        .map(|i| match (&a[i], &b[i]) {
            (Value::Vec(x), Value::Vec(y)) => elementwise(x, y, f),
            (x, y) => match (to_f(x), to_f(y)) {
                (Some(p), Some(q)) => Value::Num(f(p, q)),
                _ => Value::Undef,
            },
        })
        .collect();
    Value::Vec(out)
}
/// Recursively scale a sequence by a scalar (number·vector/matrix, vector/number).
fn scale_seq(a: &[Value], s: f64) -> Value {
    Value::Vec(
        a.iter()
            .map(|v| match v {
                Value::Vec(x) => scale_seq(x, s),
                _ => to_f(v).map(|p| Value::Num(p * s)).unwrap_or(Value::Undef),
            })
            .collect(),
    )
}
/// Whether an ordering satisfies an ordered comparison operator.
fn ordered_ok(op: BinaryOp, ord: std::cmp::Ordering) -> bool {
    use std::cmp::Ordering::*;
    match op {
        BinaryOp::Less => ord == Less,
        BinaryOp::LessEqual => ord != Greater,
        BinaryOp::Greater => ord == Greater,
        BinaryOp::GreaterEqual => ord != Less,
        _ => false,
    }
}

/// A flat number vector, if every element is a number.
fn as_num_vec(v: &[Value]) -> Option<Vec<f64>> {
    v.iter().map(|e| if let Value::Num(n) = e { Some(*n) } else { None }).collect()
}
/// A rectangular matrix (rows of numbers), if every element is a number vector.
fn as_matrix(v: &[Value]) -> Option<Vec<Vec<f64>>> {
    let rows: Option<Vec<Vec<f64>>> = v
        .iter()
        .map(|e| if let Value::Vec(r) = e { as_num_vec(r) } else { None })
        .collect();
    let rows = rows?;
    let w = rows.first()?.len();
    if w > 0 && rows.iter().all(|r| r.len() == w) { Some(rows) } else { None }
}
fn numvec(xs: Vec<f64>) -> Value {
    Value::Vec(xs.into_iter().map(Value::Num).collect())
}
/// OpenSCAD `*` between two vectors: dot product (vec·vec), row-vec×matrix,
/// matrix×col-vec, or matrix×matrix. Falls back to a dot product on mismatch.
fn vec_mul(a: &[Value], b: &[Value]) -> Result<Value, String> {
    let (av, bv) = (as_num_vec(a), as_num_vec(b));
    if let (Some(x), Some(y)) = (&av, &bv) {
        let n = x.len().min(y.len());
        return Ok(Value::Num((0..n).map(|i| x[i] * y[i]).sum()));
    }
    let (am, bm) = (as_matrix(a), as_matrix(b));
    match (&av, &am, &bv, &bm) {
        // row-vector (len n) × matrix (n×p) -> vector (len p)
        (Some(x), _, None, Some(m)) if x.len() == m.len() => {
            let p = m[0].len();
            Ok(numvec((0..p).map(|j| (0..x.len()).map(|i| x[i] * m[i][j]).sum()).collect()))
        }
        // matrix (n×k) × column-vector (len k) -> vector (len n)
        (None, Some(m), Some(y), _) if m[0].len() == y.len() => {
            Ok(numvec(m.iter().map(|row| row.iter().zip(y).map(|(a, b)| a * b).sum()).collect()))
        }
        // matrix (n×k) × matrix (k×p) -> matrix (n×p)
        (None, Some(ma), None, Some(mb)) if ma[0].len() == mb.len() => {
            let (k, p) = (mb.len(), mb[0].len());
            let out: Vec<Value> = ma
                .iter()
                .map(|row| numvec((0..p).map(|j| (0..k).map(|t| row[t] * mb[t][j]).sum()).collect()))
                .collect();
            Ok(Value::Vec(out))
        }
        _ => Err(format!(
            "scad: incompatible operands for `*` (shapes {}×? and {}×?)",
            a.len(),
            b.len()
        )),
    }
}

/// Evaluate a function call: built-in math or a user-defined function.
fn eval_call(callee: &Expr, args: &[Argument], env: &Env) -> Result<Value, String> {
    let name = match &callee.kind {
        ExprKind::Identifier(n) => n.as_str(),
        // callee is an expression (e.g. `(function(x) x)(3)`) → call its value
        _ => return call_value(eval_expr(callee, env)?, args, env),
    };
    let d2r = std::f64::consts::PI / 180.0; // OpenSCAD trig is in DEGREES
    let argv = |i: usize| -> Result<Value, String> {
        eval_expr(&args.get(i).ok_or("function: missing argument")?.value, env)
    };
    let argn = |i: usize| -> Result<f64, String> { argv(i)?.as_num() };

    // --- structural / non-numeric builtins ---
    match name {
        "str" => {
            let mut s = String::new();
            for a in args {
                s.push_str(&value_to_string(&eval_expr(&a.value, env)?));
            }
            return Ok(Value::Str(s));
        }
        "chr" => {
            let mut s = String::new();
            let mut push_code = |n: f64| {
                if let Some(c) = char::from_u32(n as u32) {
                    s.push(c);
                }
            };
            match argv(0)? {
                Value::Num(n) => push_code(n),
                Value::Vec(v) => {
                    for e in v {
                        push_code(e.as_num()?);
                    }
                }
                _ => return Err("chr: expects a number or list".into()),
            }
            return Ok(Value::Str(s));
        }
        "ord" => {
            return Ok(match argv(0)? {
                Value::Str(s) => s.chars().next().map(|c| Value::Num(c as u32 as f64)).unwrap_or(Value::Undef),
                _ => Value::Undef,
            });
        }
        "concat" => {
            let mut out = Vec::new();
            for a in args {
                match eval_expr(&a.value, env)? {
                    Value::Vec(v) => out.extend(v),
                    other => out.push(other),
                }
            }
            return Ok(Value::Vec(out));
        }
        "len" => {
            return Ok(Value::Num(match argv(0)? {
                Value::Vec(v) => v.len() as f64,
                Value::Str(s) => s.chars().count() as f64,
                _ => return Err("len: expects a vector or string".into()),
            }));
        }
        "norm" => {
            if let Value::Vec(v) = argv(0)? {
                let mut s = 0.0;
                for e in &v {
                    let n = e.as_num()?;
                    s += n * n;
                }
                return Ok(Value::Num(s.sqrt()));
            }
            return Err("norm: expects a vector".into());
        }
        "cross" => {
            let a = vec3_of(&argv(0)?)?;
            let b = vec3_of(&argv(1)?)?;
            return Ok(Value::Vec(vec![
                Value::Num(a[1] * b[2] - a[2] * b[1]),
                Value::Num(a[2] * b[0] - a[0] * b[2]),
                Value::Num(a[0] * b[1] - a[1] * b[0]),
            ]));
        }
        "sign" => {
            let x = argn(0)?;
            return Ok(Value::Num(if x > 0.0 { 1.0 } else if x < 0.0 { -1.0 } else { 0.0 }));
        }
        "is_undef" => return Ok(Value::Bool(matches!(argv(0)?, Value::Undef))),
        "is_list" => return Ok(Value::Bool(matches!(argv(0)?, Value::Vec(_)))),
        "is_num" => return Ok(Value::Bool(matches!(argv(0)?, Value::Num(_)))),
        "is_bool" => return Ok(Value::Bool(matches!(argv(0)?, Value::Bool(_)))),
        "is_string" => return Ok(Value::Bool(matches!(argv(0)?, Value::Str(_)))),
        "is_function" => return Ok(Value::Bool(matches!(argv(0)?, Value::Func(_)))),
        "min" | "max" => {
            let vals: Vec<Value> = if args.len() == 1 {
                match argv(0)? {
                    Value::Vec(v) => v,
                    other => vec![other],
                }
            } else {
                let mut xs = Vec::new();
                for a in args {
                    xs.push(eval_expr(&a.value, env)?);
                }
                xs
            };
            let mut acc: Option<f64> = None;
            for v in vals {
                let x = v.as_num()?;
                acc = Some(match acc {
                    None => x,
                    Some(a) if name == "min" => a.min(x),
                    Some(a) => a.max(x),
                });
            }
            return Ok(Value::Num(acc.ok_or("min/max: no arguments")?));
        }
        "lookup" => return lookup_fn(argv(0)?, argv(1)?),
        "search" => return search_fn(argv(0)?, argv(1)?),
        "rands" => {
            let min = argn(0)?;
            let max = argn(1)?;
            let count = argn(2)?.max(0.0) as usize;
            // Deterministic LCG (seeded) so renders are reproducible.
            let mut state = args.get(3).map(|_| argn(3)).transpose()?.map(|s| s as u64).unwrap_or(0x2545_F491_4F6C_DD1D);
            let mut out = Vec::with_capacity(count);
            for _ in 0..count {
                state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                let f = ((state >> 11) as f64) / ((1u64 << 53) as f64);
                out.push(Value::Num(min + f * (max - min)));
            }
            return Ok(Value::Vec(out));
        }
        _ => {}
    }

    // --- pure-numeric builtins ---
    let v = match name {
        "sin" => (argn(0)? * d2r).sin(),
        "cos" => (argn(0)? * d2r).cos(),
        "tan" => (argn(0)? * d2r).tan(),
        "asin" => argn(0)?.asin() / d2r,
        "acos" => argn(0)?.acos() / d2r,
        "atan" => argn(0)?.atan() / d2r,
        "atan2" => argn(0)?.atan2(argn(1)?) / d2r,
        "sqrt" => argn(0)?.sqrt(),
        "abs" => argn(0)?.abs(),
        "floor" => argn(0)?.floor(),
        "ceil" => argn(0)?.ceil(),
        "round" => argn(0)?.round(),
        "ln" => argn(0)?.ln(),
        "log" => argn(0)?.log10(),
        "exp" => argn(0)?.exp(),
        "pow" => argn(0)?.powf(argn(1)?),
        // user-defined function
        other => {
            if let Some((params, body)) = env.funcs.get(other) {
                let a = eval_args(args, env)?;
                let mut local = env.clone();
                bind_params(params, &a, &mut local)?;
                return eval_expr(body, &local);
            }
            // A variable holding a function literal.
            if let Some(f @ Value::Func(_)) = env.vars.get(other) {
                return call_value(f.clone(), args, env);
            }
            return Err(format!("scad: unknown function \"{other}\""));
        }
    };
    Ok(Value::Num(v))
}

/// Structural equality for `==`/`!=` on non-numeric values.
fn value_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Num(x), Value::Num(y)) => (x - y).abs() < 1e-12,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Str(x), Value::Str(y)) => x == y,
        (Value::Undef, Value::Undef) => true,
        (Value::Vec(x), Value::Vec(y)) => x.len() == y.len() && x.iter().zip(y).all(|(p, q)| value_eq(p, q)),
        _ => false,
    }
}

/// OpenSCAD `str()`-style rendering of a value.
fn value_to_string(v: &Value) -> String {
    match v {
        Value::Num(n) => {
            if n.fract() == 0.0 && n.abs() < 1e15 {
                format!("{}", *n as i64)
            } else {
                format!("{n}")
            }
        }
        Value::Bool(b) => b.to_string(),
        Value::Str(s) => s.clone(),
        Value::Undef => "undef".into(),
        Value::Vec(items) => {
            let parts: Vec<String> = items.iter().map(value_to_string).collect();
            format!("[{}]", parts.join(", "))
        }
        Value::Func(_) => "function".into(),
    }
}

/// `lookup(key, [[k,v],…])` with linear interpolation between bracketing keys.
fn lookup_fn(key: Value, table: Value) -> Result<Value, String> {
    let k = key.as_num()?;
    let rows = match table {
        Value::Vec(r) => r,
        _ => return Err("lookup: table must be a list of [key, value]".into()),
    };
    let mut pairs: Vec<(f64, f64)> = Vec::new();
    for row in &rows {
        if let Value::Vec(kv) = row {
            if kv.len() >= 2 {
                pairs.push((kv[0].as_num()?, kv[1].as_num()?));
            }
        }
    }
    if pairs.is_empty() {
        return Ok(Value::Undef);
    }
    pairs.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    if k <= pairs[0].0 {
        return Ok(Value::Num(pairs[0].1));
    }
    if k >= pairs[pairs.len() - 1].0 {
        return Ok(Value::Num(pairs[pairs.len() - 1].1));
    }
    for w in pairs.windows(2) {
        let (k0, v0) = w[0];
        let (k1, v1) = w[1];
        if k >= k0 && k <= k1 {
            let t = if (k1 - k0).abs() < 1e-12 { 0.0 } else { (k - k0) / (k1 - k0) };
            return Ok(Value::Num(v0 + t * (v1 - v0)));
        }
    }
    Ok(Value::Num(pairs[pairs.len() - 1].1))
}

/// `search(value, list)` — returns matching indices (basic single-return form:
/// scalar/string against a list or string; returns a list of the first match).
fn search_fn(needle: Value, haystack: Value) -> Result<Value, String> {
    let mut hits = Vec::new();
    match (&needle, &haystack) {
        (Value::Str(n), Value::Str(h)) => {
            // For each char of needle, first index in haystack (OpenSCAD-ish).
            for nc in n.chars() {
                if let Some(i) = h.chars().position(|c| c == nc) {
                    hits.push(Value::Num(i as f64));
                }
            }
        }
        (_, Value::Vec(h)) => {
            if let Some(i) = h.iter().position(|e| value_eq(e, &needle)
                || matches!(e, Value::Vec(row) if !row.is_empty() && value_eq(&row[0], &needle)))
            {
                hits.push(Value::Num(i as f64));
            }
        }
        _ => {}
    }
    Ok(Value::Vec(hits))
}

/// A tiny table of common OpenSCAD color names -> linear-ish 0..1 rgb.
fn named_color(name: &str) -> [f64; 3] {
    // Hex: #rgb or #rrggbb
    if let Some(hex) = name.strip_prefix('#') {
        let parse = |s: &str| u8::from_str_radix(s, 16).ok().map(|v| v as f64 / 255.0);
        if hex.len() == 6 {
            if let (Some(r), Some(g), Some(b)) = (parse(&hex[0..2]), parse(&hex[2..4]), parse(&hex[4..6])) {
                return [r, g, b];
            }
        } else if hex.len() == 3 {
            let dup = |c: &str| parse(&format!("{c}{c}"));
            if let (Some(r), Some(g), Some(b)) = (dup(&hex[0..1]), dup(&hex[1..2]), dup(&hex[2..3])) {
                return [r, g, b];
            }
        }
    }
    match name.to_ascii_lowercase().as_str() {
        "red" => [0.9, 0.15, 0.15],
        "green" => [0.2, 0.7, 0.2],
        "blue" => [0.2, 0.4, 0.9],
        "yellow" => [0.95, 0.85, 0.2],
        "orange" => [0.95, 0.6, 0.15],
        "purple" | "magenta" => [0.7, 0.25, 0.8],
        "cyan" => [0.2, 0.8, 0.85],
        "white" => [0.95, 0.95, 0.95],
        "black" => [0.1, 0.1, 0.1],
        "gray" | "grey" => [0.55, 0.55, 0.55],
        "steelblue" => [0.27, 0.51, 0.71],
        _ => [0.8, 0.8, 0.8],
    }
}
