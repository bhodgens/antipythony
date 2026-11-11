//! Leaf 02.06 — the AOT runtime seam: manifest registration + dispatch.
//!
//! A built binary embeds a manifest of pre-compiled module-level function
//! bodies (cranelift-object objects linked at build time, see
//! ap-codegen-native/link.rs). The CRT (ap-aot-crt) registers the linked
//! manifest HERE before the module body runs; `call_function`'s tier
//! dispatch arm consults the registry by fn qualname and, on a hit,
//! executes the AOT body through the SAME ExecCtx contract the JIT
//! baseline uses (jit_hook) — host callbacks, slot bridge, root capture.
//! A miss tree-walks exactly as tier-0.
//!
//! Registry identity: the manifest's `apfn_<name>` qualnames are matched
//! against the MAIN module's registered def names at first dispatch (a
//! per-process OnceLock map def-name → manifest index, built lazily).
//! Imported-module functions are NOT in the manifest (only the user
//! module's module-level defs ship native — §02.06 Non-goal 3); they
//! dispatch through the normal tree-walk/JIT routes.
//!
//! Byte-identity: the AOT object IS the same lowering the JIT baseline
//! compiles (clif's emit_cfg), executed through the same host seams —
//! transparency differential C-31 applies unchanged. The AOT dispatch
//! path reports through APX_TIER_STATS as `aot_execs`.
//!
//! SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::OnceLock;

use ap_codegen_native::runtime::ExecCtx;
use ap_obj::value::Value;

/// One registered AOT function (one manifest row, filled by the CRT).
pub struct AotFnEntry {
    /// `apfn_<def-name>` — matched against the main module's def names.
    pub qualname: String,
    /// The linked function's entry (`extern "C" fn(*mut ExecCtx) -> i32`).
    pub code_ptr: *const u8,
    /// ExecCtx slot count (num_locals + max temp; the jit_hook rule).
    pub num_slots: u32,
    /// The cfg's interned name table (ExecCtx::names).
    pub name_table: Vec<String>,
    /// The static IC payload pointer + slot count (never recorded in
    /// tier-0-native code; the layout rides the row for the future
    /// in-binary adaptive tier).
    pub ic_ptr: *mut u8,
    pub ic_len: u32,
}

// SAFETY (shared with jit_hook's pointer discipline): the manifest lives
// in the binary's __DATA segment for the process lifetime; entries are
// registered once before any module code runs and read-only afterwards.
unsafe impl Send for AotFnEntry {}
unsafe impl Sync for AotFnEntry {}

static MANIFEST: OnceLock<Vec<AotFnEntry>> = OnceLock::new();
static BY_NAME: OnceLock<HashMap<String, usize>> = OnceLock::new();

/// CRT registration (once, before run_module).
pub fn set_manifest(entries: Vec<AotFnEntry>) {
    let mut by_name = HashMap::new();
    for (i, e) in entries.iter().enumerate() {
        by_name.insert(e.qualname.clone(), i);
    }
    let _ = BY_NAME.set(by_name);
    let _ = MANIFEST.set(entries);
}

/// The registered manifest (empty Vec when unset — embedders/tests).
pub fn manifest_slice() -> &'static [AotFnEntry] {
    MANIFEST.get().map(|v| v.as_slice()).unwrap_or(&[])
}

/// AOT stats (APX_TIER_STATS): executions through AOT bodies.
static AOT_EXECS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The one-line AOT report appended to jit_hook's [TIER-STATS] output.
pub fn stats_line() -> Option<String> {
    let n = AOT_EXECS.load(std::sync::atomic::Ordering::Relaxed);
    Some(format!(
        "[AOT-STATS] registered={} execs={n}",
        manifest_slice().len()
    ))
}

/// Resolve the AOT entry for a MAIN-module def by name (the registry's
/// `by_path` name — call_function knows it as entry.def.name).
pub fn entry_for(def_name: &str) -> Option<usize> {
    BY_NAME.get()?.get(&format!("apfn_{def_name}")).copied()
}

/// Execute an AOT body (the AOT dispatch arm's callee — mirrors
/// jit_call_body's slot bridge exactly).
///
/// Ok(None) never happens here (the body IS compiled); Some(v) = return
/// value, Err(Flow) = the body raised. Panics inside the body are caught
/// at the CRT boundary (catch_unwind), not here.
pub fn call_body(
    index: usize,
    interp: &Rc<crate::frame::Interpreter>,
    frame: &Rc<crate::frame::Frame>,
    entry: &crate::frame::FnEntry,
) -> Result<Value, crate::Flow> {
    let manifest = MANIFEST.get().expect("AOT dispatch before registration");
    let aot = &manifest[index];
    let code: extern "C" fn(*mut ExecCtx) -> i32 = unsafe { std::mem::transmute(aot.code_ptr) };

    // Slot bridge (jit_hook parity): params at [n_params..2*n_params) per
    // the cfg ABI's off-by-one convention, then body locals by name.
    let num_params = entry.def.params.len();
    let mut names: Vec<String> = vec![String::new(); num_params];
    names.extend(entry.def.params.iter().map(|p| p.name.clone()));
    for name in aot.name_table.iter().skip(names.len()) {
        names.push(name.clone());
    }

    // Host state + callbacks: the SAME surfaces the JIT route uses
    // (byte-identity by construction — jit_hook owns the impls).
    let host_state = Box::new(crate::jit_hook::HostState::new(
        interp.clone(),
        frame.clone(),
        aot.name_table.clone(),
    ));
    let mut ctx = ExecCtx::new(
        aot.num_slots as usize,
        crate::jit_hook::aot_host_callbacks(),
    );
    ctx.host_data = Box::into_raw(host_state) as *mut u8;
    ctx.names = aot.name_table.clone();
    // The static IC payload is never recorded by tier-0-native code; the
    // FeedbackCell-shaped view keeps the recording helpers' null-check
    // contract intact without ever writing (ic recording is a no-op for
    // these cells by layout — all EMPTY, pitch-frozen).
    ctx.ic_base = aot.ic_ptr as *mut ap_codegen_native::feedback::FeedbackCell;
    ctx.ic_len = aot.ic_len;
    crate::jit_hook::aot_load_slots(&mut ctx, frame, &names);

    // 02.05g root-shim seam (identical to jit_call_body): capture this
    // host frame as the fp-chain walk start for the duration of the call.
    unsafe { ap_gc::set_capture_point(ap_gc::caller_frame_pointer()) };
    let call_r = code(&mut ctx as *mut ExecCtx);
    ap_gc::clear_capture_point();

    AOT_EXECS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

    // Sync locals back BEFORE mapping the result (exception unwind still
    // leaves partially-updated locals observable — jit_hook discipline).
    crate::jit_hook::aot_store_slots(frame, &names, &ctx, frame);

    match call_r {
        0 => Ok(std::mem::replace(&mut ctx.ret, Value::None)),
        _ => match ctx.pending_exc.take() {
            Some(e) => Err(crate::Flow::Raise(crate::jit_hook::aot_rebuild(e))),
            None => Err(crate::Flow::Raise(crate::exc_hier::ExceptionValue::msg(
                "SystemError",
                "AOT body failed without parking an exception",
            ))),
        },
    }
}
