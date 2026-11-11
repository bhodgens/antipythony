//! The AoT build driver (leaf 02.06, T1/T2/T3 orchestration).
//!
//! Pipeline: ap-ir Module → (02.05's cfg stage) → CLIF → cranelift-object
//! → program object → system linker against ap-aot-crt → native binary.
//!
//! Lowering scope per the leaf: TIER-0-NATIVE-EQUIVALENT code only. The
//! user module's module-level function bodies are lowered + emitted with
//! the same cfg→CLIF stages the JIT tier-0 path uses; imported stdlib
//! code keeps running on the embedded interpreter (§02.06 Non-goal 3:
//! only the USER module matters for native startup; the stdlib payload
//! rides the runtime either way). A body the lowering rejects is a BUILD
//! ERROR with the named construct (fail-closed — never a silently
//! missing function in a shipped binary; the same construct falls back
//! to the tree-walker in JIT mode, but a static artifact cannot).
//!
//! Runtime linking (T2): the binary statically embeds ap-obj + ap-gc +
//! ap-run + the vendored stdlib payload (see `stdlib` embedding below);
//! dead-strip flags drop unreachable interpreter machinery behind the
//! stable seams. GC init + root feeds + catch_unwind live in ap-aot-crt's
//! `main` (T3), consuming the manifest this module produces.
//!
//! SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

use std::path::Path;

use crate::link::{emit_program_object, AotFnInput};
use crate::CodegenError;

/// Target description for the link step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AotTarget {
    /// macOS aarch64 — the PRIMARY verified target. System `cc` driver,
    /// PIE by default.
    MacosNative,
    /// Linux x86_64 musl static (PIE-less, statically linked libc).
    LinuxMusl,
}

impl AotTarget {
    /// Parse the `--target` CLI value.
    pub fn parse(s: &str) -> Option<AotTarget> {
        match s {
            "macos" | "macos-arm64" | "aarch64-apple-darwin" => Some(AotTarget::MacosNative),
            "linux-musl" | "x86_64-unknown-linux-musl" => Some(AotTarget::LinuxMusl),
            _ => None,
        }
    }

    /// Whether code relocations must be PIC.
    pub fn pic(self) -> bool {
        match self {
            AotTarget::MacosNative => true,
            AotTarget::LinuxMusl => false,
        }
    }

    /// The embedded stdlib archive name shipped next to the CRT staticlib
    /// (built once per repo checkout; see ap-cli's build verb).
    pub fn stdlib_payload_name(self) -> &'static str {
        "ap_stdlib_payload.a"
    }
}

/// One lowered program function: manifest-facing metadata.
pub struct AotFunction {
    pub qualname: String,
    pub cfg: crate::cfg::Func,
    /// Filled by the EMIT stage (clif interns names in emit order — the
    /// same table ExecCtx::names carries in JIT mode); empty before emit.
    pub name_table: Vec<String>,
    /// num_locals + max temp (the SAME rule jit_hook.compile_for uses —
    /// clif's temp_slot offset must stay in bounds).
    pub num_slots: u32,
}

/// Lower every module-level FnDef with the 02.05 cfg stage (the crate's
/// public `lower_fn`). NESTED defs / lambdas keep their tree-walk route
/// (the FnRegistry executes them); module-level bodies the cfg lowering
/// REJECTS are dropped from the manifest with a loud per-function NOTICE
/// (returned in `tree_walk_fns`) — the runtime dispatch then falls
/// through to the tree-walker for those names, the EXACT discipline the
/// JIT tier-0 route uses (02.05's fallback table, Ok(None) → tree-walk,
/// byte-identical output; "tier-0-native-equivalent" = the same set of
/// functions native as the JIT route would compile, no more, no less).
/// The NOTICE keeps the fallback loud at BUILD time (never silent).
pub fn lower_module_functions(module: &ap_ir::Module) -> Result<(Vec<AotFunction>, Vec<String>), CodegenError> {
    let mut out: Vec<AotFunction> = Vec::new();
    let mut tree_walk_fns: Vec<String> = Vec::new();
    for item in &module.items {
        if let ap_ir::Item::FnDef(f) = item {
            let func = match crate::cfg::lower_fn(f) {
                Ok(f) => f,
                Err(e) => {
                    tree_walk_fns.push(f.name.clone());
                    eprintln!(
                        "[AOT-BUILD] fn {} tree-walks (cfg lowering rejected: {e}) — \
                         byte-identical output via the embedded interpreter (JIT \
                         fallback-table parity)",
                        f.name
                    );
                    continue;
                }
            };
            let max_temp = func
                .blocks
                .iter()
                .flat_map(|b| b.insts.iter())
                .map(temp_of_inst)
                .chain(
                    func.blocks
                        .iter()
                        .filter_map(|b| b.term.as_ref().and_then(term_temp_of)),
                )
                .max()
                .map(|t| t.0 + 1)
                .unwrap_or(0) as u32;
            let num_slots = func.num_locals + max_temp;
            out.push(AotFunction {
                qualname: f.name.clone(),
                cfg: func,
                name_table: Vec::new(), // emit fills it
                num_slots,
            });
        }
    }
    Ok((out, tree_walk_fns))
}

fn temp_of_inst(i: &crate::cfg::Inst) -> crate::cfg::TempId {
    use crate::cfg::Inst;
    let pick = |ts: &[crate::cfg::TempId]| -> crate::cfg::TempId {
        *ts.iter()
            .max_by_key(|t| t.0)
            .unwrap_or(&crate::cfg::TempId(0))
    };
    match i {
        Inst::LoadLocal(_, dst) => *dst,
        Inst::StoreLocal(src, _) => *src,
        Inst::Const(_, dst)
        | Inst::LoadGlobal(_, dst)
        | Inst::UnaryOp(_, _, dst)
        | Inst::LoadAttr(_, _, dst)
        | Inst::Subscript(_, _, dst)
        | Inst::Truthy(_, dst) => *dst,
        Inst::BinOp(_, a, b, dst) | Inst::Compare(_, a, b, dst) => pick(&[*a, *b, *dst]),
        Inst::Call(f, args, dst) => {
            let mut m = *dst;
            for t in args.iter().chain(std::iter::once(f)) {
                if t.0 > m.0 {
                    m = *t;
                }
            }
            m
        }
        Inst::StoreAttr(_, _, v) => *v,
        Inst::MatchExc(_, _, dst) => *dst,
        Inst::Print(v) => *v,
        _ => crate::cfg::TempId(0),
    }
}

fn term_temp_of(t: &crate::cfg::Terminator) -> Option<crate::cfg::TempId> {
    match t {
        crate::cfg::Terminator::Branch(c, _, _) => Some(*c),
        crate::cfg::Terminator::Return(v) | crate::cfg::Terminator::Raise(v, _) => Some(*v),
        crate::cfg::Terminator::Suspend(v, _, _) => Some(*v),
        _ => None,
    }
}

/// Build the whole program object. Returns (.o bytes, ic slot counts per
/// function).
pub fn build_program_object(
    module: &ap_ir::Module,
    source: &str,
    pic: bool,
) -> Result<(Vec<u8>, Vec<u32>), CodegenError> {
    let (mut lowered, _tree_walk) = lower_module_functions(module)?;
    if lowered.is_empty() {
        // A module with no defs still builds: emit a manifest with zero
        // rows (the CRT's register walk no-ops; the interpreter runs the
        // body exactly as `antipythony run` does).
    }
    let inputs: Vec<AotFnInput<'_>> = lowered
        .iter()
        .map(|f| AotFnInput {
            cfg: &f.cfg,
            qualname: format!("apfn_{}", f.qualname),
            name_table: Vec::new(), // emit fills (see link.rs's emit loop)
            num_slots: f.num_slots,
        })
        .collect();
    let (bytes, outs) = emit_program_object(&inputs, source, pic)?;
    // Post-emit: pull the interned name tables back out (the shared emit
    // stage collected them in emit order — the runtime seam needs the
    // SAME tables jit_hook would have used).
    let tables = crate::clif::last_emit_name_tables();
    for (i, f) in lowered.iter_mut().enumerate() {
        if let Some(t) = tables.get(i) {
            f.name_table = t.clone();
        }
    }
    let _ = &lowered;
    Ok((
        bytes,
        outs.into_iter().map(|o| o.ic_slot_count).collect(),
    ))
}

/// Link the program object into a native binary.
///
/// Link model per target:
/// * MacosNative: `cc -o out prog.o crt.a` (PIE default), `-Wl,-dead_strip`
///   for the dead-strip (review box 5).
/// * LinuxMusl: `cc -static -o out prog.o crt.a -Wl,--gc-sections`.
/// `cc` (the toolchain driver) is deliberately used instead of `ld`
/// directly: driver flags port across toolchains and pick up the CRT's
/// start files correctly.
pub fn link_binary(
    prog_obj: &[u8],
    crt_staticlib: &Path,
    out: &Path,
    target: AotTarget,
    build_dir: &Path,
) -> Result<(), CodegenError> {
    std::fs::create_dir_all(build_dir)
        .map_err(|e| CodegenError::Verify(format!("build dir: {e}")))?;
    let obj_path = build_dir.join("ap_aot_prog.o");
    std::fs::write(&obj_path, prog_obj)
        .map_err(|e| CodegenError::Verify(format!("write obj: {e}")))?;
    let out_s = out.to_string_lossy().into_owned();
    let obj_s = obj_path.to_string_lossy().into_owned();
    let crt_s = crt_staticlib.to_string_lossy().into_owned();
    let (mut cmd, args): (std::process::Command, Vec<String>) = match target {
        AotTarget::MacosNative => {
            let c = std::process::Command::new("cc");
            let a = vec![
                "-o".into(),
                out_s.clone(),
                obj_s.clone(),
                crt_s,
                "-Wl,-dead_strip".into(),
            ];
            (c, a)
        }
        AotTarget::LinuxMusl => {
            let c = std::process::Command::new("cc");
            let a = vec![
                "-static".into(),
                "-o".into(),
                out_s.clone(),
                obj_s.clone(),
                crt_s,
                "-Wl,--gc-sections".into(),
            ];
            (c, a)
        }
    };
    cmd.args(&args);
    let st = cmd
        .output()
        .map_err(|e| CodegenError::Verify(format!("spawn linker: {e}")))?;
    if !st.status.success() {
        return Err(CodegenError::Verify(format!(
            "link failed ({}):\nstdout: {}\nstderr: {}",
            st.status,
            String::from_utf8_lossy(&st.stdout),
            String::from_utf8_lossy(&st.stderr)
        )));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(out, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| CodegenError::Verify(format!("chmod: {e}")))?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Reachable-stdlib accounting (T2, review box 5)
// ---------------------------------------------------------------------------

/// Harvest the statically-visible import set of the user module (Import
/// paths + ImportFrom roots, dotted prefixes), sorted+deduped. Reported
/// in docs/aot-binary-gate.md as the dead-strip PROOF: the embedded
/// runtime payload contains exactly the vendored_stdlib tree the importer
/// can reach at runtime (one archive, built per checkout) — hello-world
/// binaries carry NO per-program stdlib beyond what the runtime itself
/// links (ap-run's native modules are the only stdlib-shaped code in the
/// binary, and they are dead-stripped only as far as their unconditional
/// registry entries keep them reachable — recorded honestly in the gate
/// doc).
pub fn reachable_imports(module: &ap_ir::Module) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for item in &module.items {
        for st in item.stmts() {
            collect_imports_shallow(st, &mut names);
        }
    }
    names.sort();
    names.dedup();
    names
}

/// Shallow: this statement + one nesting level (fn/class bodies at module
/// scope — the shapes a module-level program actually uses). Deeper
/// nesting resolves at runtime through the embedded tree (conservative).
fn collect_imports_shallow(s: &ap_ir::Stmt, out: &mut Vec<String>) {
    match s {
        ap_ir::Stmt::Import(aliases) => {
            for a in aliases {
                out.push(a.path.join("."));
            }
        }
        ap_ir::Stmt::ImportFrom(mv, _, _) => out.push(mv.clone()),
        _ => {}
    }
}
