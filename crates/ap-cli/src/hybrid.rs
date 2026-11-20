//! `antipythony build --hybrid` — the 04.05 hybrid artifact verb.
//!
//! Shape (the LINK-TIME decision, recorded in the leaf record and the
//! hybrid CRT's module docs): the wasm payload is a SECOND object file
//! emitted here through the same cranelift-object machinery link.rs uses
//! — `include_bytes!` cannot work because the payload bytes are a pure
//! function of (source, invoked file name), known only when the build
//! VERB runs, not when any crate compiles. The object defines three
//! Export data symbols the hybrid CRT (ap-aot-crt-hybrid) references:
//!
//! * `ap_aot_wasm_payload`      — the module bytes (read-only blob);
//! * `ap_aot_wasm_payload_len`  — u64 length + "APWASMEND" sentinel
//!   (the same length-word shape the plain embed uses for
//!   ap_aot_source);
//! * `ap_aot_wasm_payload_name` — NUL-terminated invoked .py file name
//!   (the emitter's source-name seam; `--emit-wasm` names the extracted
//!   module after this stem, and byte-identity vs a fresh `build
//!   --wasm` follows from the emitter being a pure function).
//!
//! Link: `cc -o app prog.o payload.o crt-hybrid.a -Wl,-dead_strip` —
//! one extra input file, nothing else changes vs the 02.06 native link.
//! Plain builds never link the payload object nor the hybrid CRT, so a
//! plain binary carries no payload and no embedded host (the T-16
//! non-hybrid refusal leg: the flag is simply not claimed there).
//!
//! SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

use std::path::Path;

/// Emit the payload object (`ap_aot_wasm_payload.o`) for one program.
///
/// * `wasm_bytes` — the module (produced by `ap_cli::wasm::build_wasm`'s
///   emitter seam for the SAME (source, file name) pair);
/// * `file_name` — the invoked .py file name (`hello.py`);
/// * `out_path` — where the .o lands (the build staging dir).
pub fn emit_wasm_payload_object(
    wasm_bytes: &[u8],
    file_name: &str,
    out_path: &Path,
) -> Result<(), String> {
    use cranelift::codegen::settings::{self, Configurable};
    use cranelift_module::{Linkage, Module};
    use cranelift_object::{ObjectBuilder, ObjectModule};

    // Data-only object: the ISA feeds the builder's format/architecture
    // selection; no code is emitted. PIC so the object links into both
    // PIE (macOS default) and static layouts — the link.rs convention.
    let mut flag_builder = settings::builder();
    flag_builder
        .set("is_pic", "true")
        .map_err(|e| format!("payload object flags: {e}"))?;
    let flags = settings::Flags::new(flag_builder);
    let isa = cranelift_native::builder()
        .map_err(|_| "payload object: native ISA unavailable".to_string())?
        .finish(flags)
        .map_err(|_| "payload object: ISA finish failed".to_string())?;
    let ob = ObjectBuilder::new(
        isa,
        Vec::from("ap_aot_wasm_payload"),
        cranelift_module::default_libcall_names(),
    )
    .map_err(|e| format!("payload object builder: {e}"))?;
    let mut module = ObjectModule::new(ob);

    // ---- the payload blob ----
    let blob_id = module
        .declare_data("ap_aot_wasm_payload", Linkage::Export, false, false)
        .map_err(|e| format!("payload decl: {e}"))?;
    let mut desc = cranelift_module::DataDescription::new();
    // 8-byte alignment: the hybrid link packs __TEXT,__const tighter than
    // the plain link (the payload object is an extra input), and an
    // unaligned blob start shifts every following symbol — the manifest
    // landed on an odd address in the first hybrid link (measured) and
    // the CRT's u64 reads are read_unaligned-proof, but the section data
    // itself should still carry the C-level alignment contract.
    desc.set_align(8);
    desc.define(wasm_bytes.to_vec().into_boxed_slice());
    module
        .define_data(blob_id, &desc)
        .map_err(|e| format!("payload def: {e}"))?;

    // ---- the length word + sentinel ----
    let len_id = module
        .declare_data("ap_aot_wasm_payload_len", Linkage::Export, false, false)
        .map_err(|e| format!("payload len decl: {e}"))?;
    let mut desc = cranelift_module::DataDescription::new();
    let mut b = wasm_bytes.len().to_le_bytes().to_vec();
    b.extend_from_slice(b"APWASMEND");
    desc.define(b.into_boxed_slice());
    module
        .define_data(len_id, &desc)
        .map_err(|e| format!("payload len def: {e}"))?;

    // ---- the recorded file name (NUL-terminated) ----
    let name_id = module
        .declare_data("ap_aot_wasm_payload_name", Linkage::Export, false, false)
        .map_err(|e| format!("payload name decl: {e}"))?;
    let mut desc = cranelift_module::DataDescription::new();
    let mut nb = file_name.as_bytes().to_vec();
    nb.push(0);
    desc.define(nb.into_boxed_slice());
    module
        .define_data(name_id, &desc)
        .map_err(|e| format!("payload name def: {e}"))?;

    let product = module.finish();
    let bytes = product
        .emit()
        .map_err(|e| format!("payload object emit: {e}"))?;
    std::fs::write(out_path, &bytes)
        .map_err(|e| format!("cannot write {}: {e}", out_path.display()))?;
    Ok(())
}
