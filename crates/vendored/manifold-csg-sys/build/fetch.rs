//! Clone and check out the pinned upstream manifold source.
//!
//! Caches the clone in `$OUT_DIR/manifold-src/`. Stamp files
//! (`.version-stamp`, `.patch-stamp`) track the pinned commit SHA and the
//! hash of the carry-patches directory; when either changes between runs,
//! the cached source tree is wiped and re-cloned so a stale checkout
//! never lingers across pin bumps. Delegates patch application to
//! `super::patch`. Called from the host/emscripten build paths; the
//! wasm32-unknown-unknown path has its own FetchContent-based flow inside
//! `super::wasm`.

use std::fs;
use std::path::Path;
use std::process::Command;

use crate::MANIFOLD_VERSION;

// Compute a hash of the patches directory so we can detect when patches
// change and invalidate the cached (possibly stale) source checkout.
fn compute_patch_stamp(patches_dir: &Path) -> String {
    let mut entries: Vec<String> = Vec::new();
    if let Ok(dir) = std::fs::read_dir(patches_dir) {
        for e in dir.flatten() {
            let path = e.path();
            if path.extension().is_none_or(|ext| ext != "patch") {
                continue;
            }
            if let Ok(contents) = std::fs::read_to_string(&path) {
                entries.push(format!("{}:{}", path.display(), contents.len()));
            }
        }
    }
    entries.sort();
    entries.join(";")
}

fn clone_manifold3d(manifold_src: &Path, commit_stamp_file: &Path) {
    let status = Command::new("git")
        .args([
            "-c",
            "core.autocrlf=false",
            "clone",
            "https://github.com/elalish/manifold.git",
            manifold_src.to_str().unwrap(),
        ])
        .status()
        .expect("failed to run git clone for manifold3d");
    assert!(status.success(), "git clone manifold3d failed");

    // Checkout pinned commit.
    let status = Command::new("git")
        .args(["checkout", MANIFOLD_VERSION])
        .current_dir(manifold_src)
        .status()
        .expect("failed to checkout manifold3d commit");
    assert!(status.success(), "git checkout manifold3d failed");

    let _ = std::fs::write(commit_stamp_file, MANIFOLD_VERSION);
}

pub fn fetch(out_dir: &Path, manifold_src: &Path, build_dir: &Path, patches_dir: &Path) {
    let commit_stamp_file = out_dir.join(".version-stamp");

    let old_commit = fs::read_to_string(&commit_stamp_file).unwrap_or_default();

    if old_commit.trim() != MANIFOLD_VERSION && manifold_src.exists() {
        if fs::remove_dir_all(manifold_src).is_err() {
            let _ = Command::new("git")
                .args(["checkout", "."])
                .current_dir(manifold_src)
                .status();
        }
        let _ = fs::remove_dir_all(build_dir);
    }
    let current_stamp = compute_patch_stamp(patches_dir);
    let patch_stamp = out_dir.join(".patch-stamp");

    let old_patch_stamp = std::fs::read_to_string(&patch_stamp).unwrap_or_default();
    if current_stamp != old_patch_stamp && manifold_src.exists() {
        // Patches changed — delete cached source so it gets re-cloned and re-patched.
        // If removal fails (e.g. read-only files on Windows), reset via git instead.
        if std::fs::remove_dir_all(manifold_src).is_err() {
            let _ = Command::new("git")
                .args(["checkout", "."])
                .current_dir(manifold_src)
                .status();
        }
        let _ = std::fs::remove_dir_all(build_dir);
    }

    if !manifold_src.join("CMakeLists.txt").exists() {
        clone_manifold3d(manifold_src, &commit_stamp_file)
    }

    super::patch::patch(patches_dir, manifold_src);

    // Record current patch state so we can detect changes on next build.
    let _ = std::fs::write(&patch_stamp, &current_stamp);
}
