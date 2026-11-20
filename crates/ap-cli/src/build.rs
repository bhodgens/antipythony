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
    // Candidate roots: the checkout's canonical target dirs PLUS the live
    // CARGO_TARGET_DIR when set (leaf worktrees build into their own
    // target-<name>/ dir — the hardcoded paths miss and the build verb
    // refuses despite the staticlib being fresh on disk).
    let mut roots = Vec::new();
    if let Ok(ctd) = std::env::var("CARGO_TARGET_DIR") {
        if !ctd.is_empty() {
            roots.push(PathBuf::from(ctd));
        }
    }
    // The running CLI lives at <target>/<profile>/<bin> — its own target
    // dir always holds the CRT built from the same tree. This keeps the
    // build verb self-contained when callers sanitize the environment
    // (T-09's _clean_env drops CARGO_TARGET_DIR; leaf worktrees build
    // into target-<name>/ the hardcoded paths never see).
    if let Ok(exe) = std::env::current_exe() {
        if let (Some(profile), Some(root)) = (exe.parent(), exe.parent().and_then(Path::parent)) {
            let _ = profile;
            roots.push(root.to_path_buf());
        }
    }
    roots.push(manifest.join("target-0206"));
    roots.push(manifest.join("target"));
    for root in &roots {
        for prof in profiles {
            let cand = root.join(prof).join("libap_aot_crt.a");
            if cand.exists() {
                return Ok(cand);
            }
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
    script_path: &str,
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
        ap_codegen_native::aot::build_program_object(&module, source, script_path, pic).map_err(
            |e| match e {
                CodegenError::Unsupported(what) => format!(
                "antipythony: build: unsupported construct in a module-level function body: {what} \
                 (the body would tree-walk under `antipythony run`; AoT requires lowering support)"
            ),
                CodegenError::Verify(what) => format!("antipythony: build: codegen verify: {what}"),
            },
        )?;

    // 3. Link against the CRT staticlib (system linker; dead-strip).
    let crt = crt_staticlib(target)?;
    aot::link_binary(&prog_obj, &crt, out_path, target, build_dir)
        .map_err(|e| format!("antipythony: build: {e}"))?;
    Ok(())
}

/// Locate the hybrid CRT staticlib (04.05 — `cargo build -p
/// ap-aot-crt-hybrid` builds it next to the plain CRT in the same target
/// dir; the same root/profile search as `crt_staticlib`).
fn crt_staticlib_hybrid(target: AotTarget) -> Result<PathBuf, String> {
    let _ = target;
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..");
    let profiles = ["release", "debug"];
    let mut roots = Vec::new();
    if let Ok(ctd) = std::env::var("CARGO_TARGET_DIR") {
        if !ctd.is_empty() {
            roots.push(PathBuf::from(ctd));
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let (Some(profile), Some(root)) = (exe.parent(), exe.parent().and_then(Path::parent)) {
            let _ = profile;
            roots.push(root.to_path_buf());
        }
    }
    roots.push(manifest.join("target-0206"));
    roots.push(manifest.join("target"));
    for root in &roots {
        for prof in profiles {
            let cand = root.join(prof).join("libap_aot_crt_hybrid.a");
            if cand.exists() {
                return Ok(cand);
            }
        }
    }
    Err(
        "antipythony: --hybrid: ap-aot-crt-hybrid staticlib not found — build it first with \\\n             `cargo build -p ap-aot-crt-hybrid` (debug or release; CARGO_TARGET_DIR must match this checkout)"
            .to_string(),
    )
}

/// The `build --hybrid` verb body: the SAME native pipeline (parse →
/// lower → AoT program object), PLUS the wasm payload object (the
/// Plan-03 emitter's output for the same (source, file name)), linked
/// against the hybrid CRT — the payload seam + embedded-host binary.
///
/// The wasm lowering is attempted FIRST and fails the build LOUDLY on an
/// unsupported construct (no hybrid artifact that silently lacks the
/// payload; the same fail-closed contract as `build --wasm`).
pub fn build_hybrid_binary(
    source: &str,
    script_path: &str,
    out_path: &Path,
    target: AotTarget,
    build_dir: &Path,
) -> Result<(), String> {
    // 1. The wasm payload: same front end, same emitter as `build --wasm`
    //    (byte-identity between the payload and a fresh `build --wasm`
    //    holds by construction — both are `lower_module_ex` over the same
    //    lowered module with the same embedded file name).
    let parsed = ap_parse::parse(source)
        .map_err(|e| format!("antipythony: SyntaxError: {} (line {})", e.message, e.line))?;
    let (module, _scopes) =
        ap_parse::lower(&parsed).map_err(|e| format!("antipythony: lower: {e}"))?;
    let file_name = Path::new(script_path)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| script_path.to_string());
    let wasm_bytes =
        ap_codegen_wasm::lower_module_ex(&module, Some(&file_name)).map_err(|e| match e {
            ap_codegen_wasm::CodegenError::Unsupported(what) => format!(
                "antipythony: build --hybrid: unsupported construct in the wasm backend: {what} \
                 (the hybrid embeds the wasm module — see docs/run-wasm-anywhere.md)"
            ),
        })?;

    // 2. The native program object — identical to the plain `build` path
    //    (hybrid-vs-native behavioral invisibility, T-16 C-31 leg).
    let pic = target.pic();
    let (prog_obj, _ic_counts) =
        ap_codegen_native::aot::build_program_object(&module, source, script_path, pic).map_err(
            |e| match e {
                CodegenError::Unsupported(what) => format!(
                "antipythony: build: unsupported construct in a module-level function body: {what} \
                 (the body would tree-walk under `antipythony run`; AoT requires lowering support)"
            ),
                CodegenError::Verify(what) => format!("antipythony: build: codegen verify: {what}"),
            },
        )?;

    // 3. The payload object.
    let payload_obj = build_dir.join("ap_aot_wasm_payload.o");
    crate::hybrid::emit_wasm_payload_object(&wasm_bytes, &file_name, &payload_obj)?;

    // 4. Link: prog.o + payload.o + hybrid CRT (one extra object vs the
    //    02.06 link; same dead-strip).
    let crt = crt_staticlib_hybrid(target)?;
    aot::link_binary_extra_objs(&prog_obj, &[payload_obj], &crt, out_path, target, build_dir)
        .map_err(|e| format!("antipythony: build --hybrid: {e}"))?;
    Ok(())
}
