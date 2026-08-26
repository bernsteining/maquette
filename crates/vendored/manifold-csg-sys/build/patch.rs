//! Apply `*.patch` files from the carry-patches directory to a cloned
//! manifold source tree.
//!
//! Each patch is checked with `git apply --check` first; only those that
//! apply cleanly are actually applied. This lets a partially-applied
//! cache survive across runs (a stamp mismatch caused a fresh clone, but
//! some patches now match upstream and don't need to apply).
//!
//! No-op when the patches directory doesn't exist, which is the common
//! case after our outstanding carry-patches landed upstream. Called by
//! `super::fetch` after the source tree is on disk.

use std::{path::Path, process::Command};

pub fn patch(patches_dir: &Path, manifold_src: &Path) {
    if !patches_dir.exists() {
        return;
    }

    let Ok(entries) = std::fs::read_dir(patches_dir) else {
        panic!("failed to read patches directory");
    };
    let mut patches: Vec<_> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "patch"))
        .collect();
    patches.sort();
    for patch in &patches {
        let check = Command::new("git")
            .args([
                "apply",
                "--check",
                "--ignore-whitespace",
                "--whitespace=nowarn",
            ])
            .arg(patch)
            .current_dir(manifold_src)
            .output()
            .expect("failed to check patch");
        if check.status.success() {
            let apply = Command::new("git")
                .args(["apply", "--ignore-whitespace", "--whitespace=nowarn"])
                .arg(patch)
                .current_dir(manifold_src)
                .output()
                .expect("failed to apply patch");
            assert!(
                apply.status.success(),
                "failed to apply patch {}: {}",
                patch.display(),
                String::from_utf8_lossy(&apply.stderr)
            );
            println!(
                "cargo:warning=Applied patch: {}",
                patch.file_name().unwrap_or_default().to_string_lossy()
            );
        } else {
            // Log why --check failed so we can diagnose CI issues.
            println!(
                "cargo:warning=Patch skipped (already applied?): {} ({})",
                patch.file_name().unwrap_or_default().to_string_lossy(),
                String::from_utf8_lossy(&check.stderr).trim()
            );
        }
    }
}
