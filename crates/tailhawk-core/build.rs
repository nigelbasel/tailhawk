//! Compiles the HLSL to DXBC at build time so the bytecode can be embedded.
//!
//! `SPEC.md` §3.2 requires offline compilation: runtime `D3DCompile` would add a
//! `d3dcompiler_47.dll` dependency, which is redistributable-only and defeats the copy-and-run
//! promise in §2. Failing the build when `fxc` is missing is deliberate — silently falling back to
//! runtime compilation is exactly the outcome the rule exists to prevent.
//!
//! `fxc`, not `dxc`: `dxc` emits DXIL, which D3D11 does not accept. DXBC is the D3D11 format.

use std::path::{Path, PathBuf};
use std::process::Command;

const SHADER: &str = "shaders/background.hlsl";

/// Highest-versioned x64 `fxc.exe` from an installed Windows SDK, or `$FXC` if set.
fn find_fxc() -> Option<PathBuf> {
    if let Ok(explicit) = std::env::var("FXC") {
        let p = PathBuf::from(explicit);
        if p.is_file() {
            return Some(p);
        }
    }
    let roots = [
        r"C:\Program Files (x86)\Windows Kits\10\bin",
        r"C:\Program Files\Windows Kits\10\bin",
    ];
    let mut found: Vec<PathBuf> = Vec::new();
    for root in roots {
        let Ok(versions) = std::fs::read_dir(root) else {
            continue;
        };
        for version in versions.flatten() {
            let candidate = version.path().join("x64").join("fxc.exe");
            if candidate.is_file() {
                found.push(candidate);
            }
        }
    }
    // Directory names are SDK versions, so lexical order is close enough to version order for
    // picking a recent one, and any of them compiles shader model 5.
    found.sort();
    found.pop()
}

fn compile(fxc: &Path, out_dir: &Path, entry: &str, profile: &str, output: &str) {
    let target = out_dir.join(output);
    let result = Command::new(fxc)
        .args(["/nologo", "/T", profile, "/E", entry, "/O3", "/WX"])
        .arg("/Fo")
        .arg(&target)
        .arg(SHADER)
        .output()
        .unwrap_or_else(|e| panic!("could not run {}: {e}", fxc.display()));

    if !result.status.success() {
        panic!(
            "fxc failed for {entry} ({profile}):\n{}\n{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
    }
}

fn main() {
    println!("cargo:rerun-if-changed={SHADER}");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=FXC");

    // CARGO_CFG_TARGET_OS, not cfg!(): a build script is compiled for the *host*, so cfg! would
    // report the host OS and get the ARM64-on-x64 cross-build wrong.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR is set by cargo"));
    let fxc = find_fxc().expect(
        "fxc.exe not found. Install the Windows SDK, or set FXC to its path. \
         SPEC.md §3.2 requires shaders to be compiled offline, so there is no runtime fallback.",
    );

    compile(&fxc, &out_dir, "vs_main", "vs_5_0", "background_vs.cso");
    compile(&fxc, &out_dir, "ps_main", "ps_5_0", "background_ps.cso");
}
