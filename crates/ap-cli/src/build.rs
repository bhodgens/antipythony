//! `antipythony build` — the 02.06 AoT verb: produce a native binary.
//!
//! Pipeline (leaf T1-T4): ap-parse → ap-ir → (02.05 cfg stage) → CLIF →
//! cranelift-object (one ObjectModule for all module-level defs) →
//! system linker against the ap-aot-crt staticlib → native binary.
//!
//! Artifacts:
//! * `out` — the binary (executable bit set; runs WITHOUT the toolchain);
//! * build staging under the content-addressed cache entry
//!   (ap_aot_prog.o + build log) — the cache key is the same
//!   (source, lock, version, backend, opt) tuple the run verb uses.
//!
//! `--target` flag: `macos` (aarch64-apple-darwin — the PRIMARY verified
//! target) and `linux-musl` (x86_64 static). A requested target whose
//! toolchain is absent fails LOUDLY with the missing piece named (no
//! silent fallback to a different artifact type).
//!
//! SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

use std::path::{Path, PathBuf};

use ap_codegen_native::aot::{self, AotTarget};
use ap_codegen_native::CodegenError;

/// Locate the prebuilt CRT staticlib (built once per checkout by the
/// build verb itself on first use — `cargo build -p ap-aot-crt`).
fn crt_staticlib(target: AotTarget) -> Result<PathBuf, String> {
    let _ = target;
    // Same profile the CLI runs in; the CRT is profile-independent for
    // correctness (debug ships debug asserts — recorded in the gate doc).
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..");
    let profiles = ["release", "debug"];
    for prof in profiles {
        let cand = manifest
            .join("target-0206")
            .join(prof)
            .join("libap_aot_crt.a");
        if cand.exists() {
            return Ok(cand);
        }
        let cand2 = manifest.join("target").join(prof).join("libap_aot_crt.a");
        if cand2.exists() {
            return Ok(cand2);
        }
    }
    Err(
        "antipythony: ap-aot-crt staticlib not found — build it first with \
         `cargo build -p ap-aot-crt` (debug or release; CARGO_TARGET_DIR must match this checkout)"
            .to_string(),
    )
}

/// The build verb body (called from main.rs's cmd_build).
pub fn build_binary(
    source: &str,
    _script_path: &str,
    out_path: &Path,
    target: AotTarget,
    build_dir: &Path,
) -> Result<(), String> {
    // 1. Parse + lower to ap-ir (the same front end `run` uses).
    let parsed = ap_parse::parse(source)
        .map_err(|e| format!("antipythony: SyntaxError: {} (line {})", e.message, e.line))?;
    let (module, _scopes) =
        ap_parse::lower(&parsed).map_err(|e| format!("antipythony: lower: {e}"))?;

    // 2. Lower every module-level def through the 02.05 cfg stage +
    //    emit the program object. Lowering-rejected bodies are dropped
    //    from the manifest with a loud [AOT-BUILD] NOTICE (lower_module_
    //    functions) and tree-walk at runtime — the JIT fallback-table
    //    discipline, byte-identical output. Remaining codegen failures
    //    (object emit / verify) stay fail-closed build errors.
    let pic = target.pic();
    let (prog_obj, _ic_counts) =
        ap_codegen_native::aot::build_program_object(&module, source, pic)
        .map_err(|e| match e {
            CodegenError::Unsupported(what) => format!(
                "antipythony: build: unsupported construct in a module-level function body: {what} \
                 (the body would tree-walk under `antipythony run`; AoT requires lowering support)"
            ),
            CodegenError::Verify(what) => format!("antipythony: build: codegen verify: {what}"),
        })?;

    // 3. Link against the CRT staticlib (system linker; dead-strip).
    let crt = crt_staticlib(target)?;
    aot::link_binary(&prog_obj, &crt, out_path, target, build_dir)
        .map_err(|e| format!("antipythony: build: {e}"))?;
    Ok(())
}
