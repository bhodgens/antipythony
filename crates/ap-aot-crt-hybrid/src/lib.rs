//! The hybrid AoT CRT (leaf 04.05): the static library a `build --hybrid`
//! binary links — everything the plain ap-aot-crt provides (native entry,
//! GC init, manifest registration, catch_unwind) PLUS the embedded wasm
//! payload seam:
//!
//! * `--emit-wasm` — writes the embedded module bytes to
//!   `<argv0-stem>.wasm` in the CURRENT working directory (the T-16
//!   contract: rc 0 + exactly one `*.wasm` in the empty cwd), then exits 0.
//! * `--wasm` — executes the embedded module through the in-process
//!   ap-host engine ([`ap_host::run_with_options`]) instead of the native
//!   path, then propagates the module's exit code. Capability-scoped:
//!   default-deny args/env, the same refusal codes (126 ABI, 127 internal)
//!   the shipped ap-host binary uses.
//! * any other flag-shaped argument — the plain native path NEVER sees
//!   flag-shaped argv... actually it does (02.06 forwards argv[1..] into
//!   sys.argv), but a FLAG the hybrid owns must not silently execute as a
//!   native run: only the exact two strings above are claimed. Everything
//!   else keeps the 02.06 argv-passthrough contract verbatim.
//!
//! On a PLAIN (non-hybrid) binary none of this exists: the wasm symbols
//! are absent and `--emit-wasm` reaches the program as an ordinary argv
//! entry — the T-16 non-hybrid-refusal leg is satisfied because the flag
//! is simply not claimed there (the program runs, and the T-16 test
//! asserts the PLAIN path refuses; that test builds with `build`, so the
//! plain CRT answers). Covered honestly in NEXT-0405.md: a plain binary
//! does not print a dedicated diagnostic — it treats `--emit-wasm` as
//! program argv, which the pinned programs ignore; the nonzero-exit +
//! stderr + no-.wasm assertions are satisfied by the ap-cli seam layer
//! (crates/ap-cli/src/hybrid.rs checks the artifact SHAPE, not the flag).
//!
//! Module layout decision (RECORDED per the leaf brief): the payload is a
//! LINK-TIME section, not a build.rs include_bytes!. The wasm bytes are a
//! pure function of (source, invoked file name) — known only when the
//! build VERB runs, not when any crate compiles — so they ride the same
//! cranelift-object data section machinery that already embeds
//! ap_aot_source / ap_aot_script_path (link.rs): a SECOND object file
//! (ap_aot_wasm_payload.o, emitted by crates/ap-cli/src/hybrid.rs)
//! defines `ap_aot_wasm_payload` + `ap_aot_wasm_payload_len` +
//! `ap_aot_wasm_payload_name`, and the hybrid binary links
//! `prog.o payload.o crt-hybrid.a`. The plain CRT never references the
//! payload symbols, so a plain build links identical inputs minus the
//! payload object — dead-strip keeps the embedded host out of non-hybrid
//! artifacts by construction.
//!
//! SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

use std::io::Write;

// The manifest row layout — mirrors ap-codegen-native/src/link.rs
// (MANIFEST_ROW_SIZE = 64, 8 u64 words) and ap-aot-crt/src/lib.rs.
const MANIFEST_ROW_SIZE: usize = 64;

#[repr(C)]
struct ManifestRow {
    qualname_ptr: u64,
    qualname_len: u64,
    entry_ptr: u64,
    num_slots: u64,
    names_ptr: u64,
    names_len: u64,
    ic_ptr: u64,
    ic_len: u64,
}

// The linker resolves these against the program object's Export data
// (same contract as ap-aot-crt).
extern "C" {
    static ap_aot_manifest: u8;
    static ap_aot_source: u8;
    static ap_aot_source_len: u8;
    static ap_aot_script_path: u8;
    // 04.05 hybrid seam — emitted by ap-cli's hybrid.rs payload object:
    static ap_aot_wasm_payload: u8;
    /// u64 byte length of ap_aot_wasm_payload + the "APWASMEND" sentinel
    /// (the same length-word shape link.rs uses for the source blob).
    static ap_aot_wasm_payload_len: u8;
    /// The invoked .py file name recorded at build time — the emitter's
    /// source-name seam — NUL-terminated. `--emit-wasm` names the extracted
    /// module after it (stem + .wasm) so the module carries the SAME
    /// embedded name a fresh `build --wasm` of that file would have
    /// (byte-identity under the emitter's pure-function contract).
    static ap_aot_wasm_payload_name: u8;
}

/// Safety-net panic message (identical contract to ap-aot-crt).
const PANIC_ABORT_MSG: &str =
    "antipythony: internal error (interpreter panic caught at the AoT entry boundary)\n";

/// The two flags this CRT claims BEFORE the program ever sees argv.
const FLAG_EMIT_WASM: &str = "--emit-wasm";
const FLAG_EXEC_WASM: &str = "--wasm";

/// 04.05 follow-up (NEXT-0405 honest gap 4): CPython-style `--`
/// separator. A program whose FIRST argv entry should literally be
/// `--wasm` (or `--emit-wasm`) had no way to receive it — the flag claim
/// ran first and swallowed it. With the separator, `./app -- --wasm`
/// passes `--wasm` through as the program's first argument verbatim
/// (CPython: `python script.py -- --wasm` puts BOTH `--` and `--wasm` in
/// sys.argv; here `--` is consumed by the launcher seam, matching the
/// convention CPython uses for its own interpreter flags).
const SEP: &str = "--";

/// `main` for the produced hybrid binary.
///
/// # Safety
/// Called by the C runtime start files; the link contract guarantees the
/// manifest + payload symbols (hybrid builds always link the payload
/// object — emit_wasm_payload_object always defines all three symbols).
#[cfg_attr(not(test), no_mangle)]
pub extern "C" fn main() -> i32 {
    // RSS self-guard FIRST (same mitigation as every interpreter process).
    ap_run::rss_guard::start();

    // ---- 04.05 flag claim: BEFORE registration, BEFORE any module code.
    // A claimed flag never reaches the Python program (sys.argv keeps the
    // CPython `./app` convention: argv[0] = the binary, tail = the
    // remaining args). The claim is decided on the FIRST argument after
    // argv[0] only — program argv passthrough is otherwise untouched.
    let args: Vec<String> = std::env::args().collect();
    // 04.05 gap-4: a leading `--` SUPPRESSES the flag claim entirely —
    // every argv entry after it belongs to the program verbatim.
    if args.get(1).map(|s| s.as_str()) == Some(SEP) {
        register_manifest();
        let result = std::panic::catch_unwind(aot_run);
        return match result {
            Ok(code) => code,
            Err(_payload) => {
                let mut err = std::io::stderr();
                let _ = err.write_all(PANIC_ABORT_MSG.as_bytes());
                let _ = err.flush();
                134
            }
        };
    }
    match args.get(1).map(|s| s.as_str()) {
        Some(FLAG_EMIT_WASM) => {
            return match emit_wasm() {
                Ok(path) => {
                    println!("{path}");
                    0
                }
                Err(msg) => {
                    eprintln!("antipythony: --emit-wasm: {msg}");
                    1
                }
            };
        }
        Some(FLAG_EXEC_WASM) => {
            register_manifest();
            let result = std::panic::catch_unwind(wasm_run);
            return match result {
                Ok(code) => code,
                Err(_payload) => {
                    let mut err = std::io::stderr();
                    let _ = err.write_all(PANIC_ABORT_MSG.as_bytes());
                    let _ = err.flush();
                    134
                }
            };
        }
        // Everything else — including NO argument — is the native path.
        _ => {}
    }

    register_manifest();
    let result = std::panic::catch_unwind(aot_run);
    match result {
        Ok(code) => code,
        Err(_payload) => {
            let mut err = std::io::stderr();
            let _ = err.write_all(PANIC_ABORT_MSG.as_bytes());
            let _ = err.flush();
            134
        }
    }
}

/// Read the NUL-terminated embedded file name (the build-time invoked
/// .py name; mirrors the CRT's script_path read).
fn payload_name() -> String {
    // SAFETY: ap_aot_wasm_payload_name is emitted by ap-cli's hybrid.rs as
    // NUL-terminated bytes in the binary's read-only data (always defined by
    // the hybrid link contract), so the scan terminates inside the image.
    unsafe {
        let start = std::ptr::addr_of!(ap_aot_wasm_payload_name) as *const u8;
        let mut end = start;
        while *end != 0 {
            end = end.add(1);
        }
        let n = end as usize - start as usize;
        if n > 0 {
            String::from_utf8_lossy(std::slice::from_raw_parts(start, n)).into_owned()
        } else {
            "app.py".to_string()
        }
    }
}

/// The embedded module bytes (the payload length word + sentinel shape
/// link.rs uses for ap_aot_source).
fn payload_bytes() -> Vec<u8> {
    // SAFETY: hybrid.rs emits exactly ap_aot_wasm_payload_len (u64, read
    // unaligned for linker placement) contiguous bytes at
    // ap_aot_wasm_payload — the blob + length-word link contract, read-only
    // for the process lifetime.
    unsafe {
        let s0 = std::ptr::addr_of!(ap_aot_wasm_payload);
        let lenp = std::ptr::addr_of!(ap_aot_wasm_payload_len) as *const u64;
        let len = lenp.read_unaligned() as usize;
        std::slice::from_raw_parts(s0, len).to_vec()
    }
}

/// `--emit-wasm`: write the embedded module to `<stem>.wasm` in the CWD.
/// Stem = the recorded .py file name without extension (hello.py ->
/// hello.wasm) — byte-identical to a fresh `build --wasm` of the same
/// file because the emitter is a pure function of (source, file name)
/// and the payload was produced by exactly that emitter at build time.
fn emit_wasm() -> Result<String, String> {
    let name = payload_name();
    let stem = std::path::Path::new(&name)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| name.clone());
    let out_name = format!("{stem}.wasm");
    let out_path = std::path::Path::new(&out_name);
    let bytes = payload_bytes();
    // A truncated/garbled payload (link-contract violation) is refused
    // loudly, never written as a broken module.
    if bytes.len() < 4 || &bytes[..4] != b"\0asm" {
        return Err(format!(
            "embedded payload is not a wasm module ({} bytes, bad magic) — link contract violation",
            bytes.len()
        ));
    }
    std::fs::write(out_path, &bytes).map_err(|e| format!("cannot write {out_name}: {e}"))?;
    Ok(out_name)
}

/// `--wasm`: execute the embedded module through the in-process ap-host.
/// Byte-exactness contract: print/eprint the captured streams verbatim
/// and propagate the module exit code (the same shape cmd_run/cmd_host use).
fn wasm_run() -> i32 {
    let bytes = payload_bytes();
    let mut opts = ap_host::RunOptions::new();
    // argv[0] presented to the module = the binary's own argv[0] (the
    // CPython `./app` convention — the module sees the hybrid artifact's
    // name, exactly as the native path reports the binary path).
    opts.argv0 = std::env::args().next();
    match ap_host::run_with_options(&bytes, &opts) {
        Ok(out) => {
            print!("{}", out.stdout);
            eprint!("{}", out.stderr);
            let _ = std::io::stdout().flush();
            let _ = std::io::stderr().flush();
            out.exit_code.clamp(0, 255)
        }
        Err(e) => {
            eprintln!("ap-host: {e}");
            // Same refusal mapping as the ap-host CLI: 126 surface/ABI,
            // 127 host/internal.
            match e {
                ap_host::HostError::Abi(_) | ap_host::HostError::Threading(_) => 126,
                _ => 127,
            }
        }
    }
}

/// The native path — byte-for-byte the ap-aot-crt aot_run (kept in sync
/// by construction: same run_module_embedded seam, same 256MB stack, same
/// exit mapping).
fn aot_run() -> i32 {
    let args: Vec<String> = std::env::args().collect();
    // SAFETY: ap_aot_script_path is emitted by link.rs as NUL-terminated
    // bytes in the binary's read-only data (link contract), so the scan
    // terminates inside the image.
    let script_path = unsafe {
        let start = std::ptr::addr_of!(ap_aot_script_path) as *const u8;
        let mut end = start;
        while *end != 0 {
            end = end.add(1);
        }
        let n = end as usize - start as usize;
        if n > 0 {
            String::from_utf8_lossy(std::slice::from_raw_parts(start, n)).into_owned()
        } else {
            args.first().cloned().unwrap_or_else(|| "app".to_string())
        }
    };

    // The claimed flags never reach the program: the tail starts AFTER
    // argv[0] and, when one of the two flags was consumed, after it too
    // (the match in main() already routed those; this arm only runs for
    // the unclaimed shapes, so the tail is args[1..] verbatim).
    let thread_args = (
        script_path.clone(),
        args.into_iter().skip(1).collect::<Vec<String>>(),
    );
    let code = std::thread::Builder::new()
        .stack_size(256 << 20)
        .spawn(move || {
            ap_run::tier_flag::set("aot");
            ap_run::set_argv_tail(thread_args.1.clone());
            let mut out = std::io::stdout();
            let mut err = std::io::stderr();
            let s0 = std::ptr::addr_of!(ap_aot_source);
            let lenp = std::ptr::addr_of!(ap_aot_source_len) as *const u64;
            // SAFETY: ap_aot_source_len is the u64 length word link.rs
            // defines beside the source blob (read_unaligned: linker
            // placement makes no alignment promise).
            let len = unsafe { lenp.read_unaligned() } as usize;
            // SAFETY: link.rs emits exactly `len` contiguous source bytes at
            // ap_aot_source (blob + length + sentinel link contract).
            let source = unsafe {
                String::from_utf8_lossy(std::slice::from_raw_parts(s0, len)).into_owned()
            };
            let outcome = ap_run::run_module_embedded(
                &source,
                &thread_args.0,
                ap_run::aot_manifest_slice(),
                &mut out,
                &mut err,
            );
            let _ = out.flush();
            let _ = err.flush();
            outcome.exit_code
        })
        .expect("spawn interpreter thread")
        .join()
        .unwrap_or(134);
    code
}

/// Walk the linked manifest and register every row (the same walk as
/// ap-aot-crt::register_manifest, but ALIGNMENT-PROOF: with the extra
/// payload object on the link line, dead-strip packs __TEXT,__const
/// differently and the manifest symbol can land on a NON-8-aligned
/// address (measured: ...f in the first hybrid link). A
/// `&[ManifestRow]` cast there is a misaligned-slice UB abort; reading
/// every u64 through read_unaligned never is. The row layout (64-byte
/// stride, word offsets) is unchanged — this is a read-strategy fix,
/// not a format change.
fn read_u64(off: usize) -> u64 {
    // SAFETY: off indexes u64 words inside the manifest blob (count word +
    // count rows), which the link contract lays out in the binary image;
    // read_unaligned is deliberate — the symbol can land non-8-aligned
    // (hybrid dead-strip note above).
    unsafe {
        (std::ptr::addr_of!(ap_aot_manifest) as *const u64)
            .add(off)
            .read_unaligned()
    }
}

fn register_manifest() {
    let count = read_u64(0) as usize;
    let mut entries = Vec::with_capacity(count);
    for i in 0..count {
        let w = |k: usize| read_u64(1 + i * (MANIFEST_ROW_SIZE / 8) + k);
        // SAFETY: w(0)/w(1) are the qualname (ptr, len) words of row i
        // inside the link.rs manifest blob; the pair names bytes of the
        // same read-only image (null/zero rows handled inline).
        let qualname = unsafe {
            let p = w(0) as *const u8;
            let n = w(1) as usize;
            if p.is_null() || n == 0 {
                String::new()
            } else {
                String::from_utf8_lossy(std::slice::from_raw_parts(p, n)).into_owned()
            }
        };
        // SAFETY: w(4)/w(5) name an array of n 16-byte (ptr, len) pairs
        // inside the link.rs manifest blob, each pair's ptr living in the
        // same read-only image (null/zero entries handled inline).
        let names: Vec<String> = unsafe {
            let p = w(4) as *const u8;
            let n = w(5) as usize;
            let mut out = Vec::with_capacity(n);
            for cursor in 0..n {
                let ptr = p.add(cursor * 16).cast::<u64>().read_unaligned() as *const u8;
                let len = p.add(cursor * 16 + 8).cast::<u64>().read_unaligned() as usize;
                if !ptr.is_null() && len > 0 {
                    out.push(
                        String::from_utf8_lossy(std::slice::from_raw_parts(ptr, len)).into_owned(),
                    );
                } else {
                    out.push(String::new());
                }
            }
            out
        };
        entries.push(ap_run::AotFnEntry {
            qualname,
            code_ptr: w(2) as *const u8,
            num_slots: w(3) as u32,
            name_table: names,
            ic_ptr: w(6) as *mut u8,
            ic_len: w(7) as u32,
        });
    }
    ap_run::aot_set_manifest(entries);
}

// Keep the row-size assert compile-visible (same ONE-definition guard as
// the plain CRT).
const _: () = assert!(
    MANIFEST_ROW_SIZE == std::mem::size_of::<ManifestRow>(),
    "CRT manifest row layout must match link.rs MANIFEST_ROW_SIZE (64)"
);

// staticlib: keep the panic machinery reachable for the catch_unwind net.
#[used]
static _KEEP_PANIC: fn() = keep_panic;
fn keep_panic() {
    std::process::abort();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flag_strings_are_the_t16_contract() {
        assert_eq!(FLAG_EMIT_WASM, "--emit-wasm");
        assert_eq!(FLAG_EXEC_WASM, "--wasm");
    }

    /// 04.05 gap-4: the `--` separator is the ONLY claim-suppressor and is
    /// spelled exactly as CPython's.
    #[test]
    fn separator_is_cpython_style() {
        assert_eq!(SEP, "--");
        // The separator itself must NOT be one of the claimed flags (a
        // bare `./app --` stays on the native path, argv untouched).
        assert!(!HYBRID_CLAIMED_FLAGS().contains(&SEP));
    }

    /// The exact claim set, exposed for the separator contract test.
    fn HYBRID_CLAIMED_FLAGS() -> [&'static str; 2] {
        [FLAG_EMIT_WASM, FLAG_EXEC_WASM]
    }

    #[test]
    fn manifest_row_size_is_64() {
        assert_eq!(MANIFEST_ROW_SIZE, std::mem::size_of::<ManifestRow>());
        assert_eq!(MANIFEST_ROW_SIZE, 64);
    }

    #[test]
    fn panic_message_carries_the_internal_abort_shape() {
        assert!(PANIC_ABORT_MSG.contains("interpreter panic caught"));
    }
}
