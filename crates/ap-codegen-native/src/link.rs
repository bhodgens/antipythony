//! Object-file assembly for the AoT pipeline (leaf 02.06, T1/T2).
//!
//! Design (docs/pon-study-tier1.md §cranelift-object): ALL program
//! functions compile into ONE `ObjectModule` — cranelift lays out
//! sections, emits relocations, and manages the symbol table. The extra
//! link-unit pieces ride the same machinery:
//!
//! * per-function name tables (`ap_aot_names_<n>`) + one string blob each;
//! * the read-only MANIFEST (`ap_aot_manifest`, Linkage::Export): one row
//!   per lowered function — {qualname_ptr, qualname_len, entry_ptr,
//!   num_slots, names_ptr, names_len, ic_ptr, ic_len} — with FUNCTION
//!   address relocations (`DataContext::write_function_addr`) resolved by
//!   the static linker;
//! * one zero-cell IC blob per function (`ap_aot_ic_<n>`): the frozen
//!   layout for tier-0-NATIVE-EQUIVALENT code. AoT ships NO runtime
//!   feedback recording; adaptive tier-1 inside the shipped binary is a
//!   documented NON-GOAL for this leaf. The blob exists so the layout is
//!   addressable for the future in-binary adaptive tier without relinking
//!   the program.
//!
//! `ap_aot_register` is NOT synthesized here: the CRT (ap-aot-crt)
//! exports the register walk; it reads `ap_aot_manifest` through the
//! normal link-time symbol reference. One artifact, one direction of
//! dependency, zero synthesized machine code.
//!
//! Relocation kind: `Reloc::Abs4/Abs8` under a non-PIC ISA (the object
//! builder inherits `is_pic` from the flags — the linker consumes plain
//! absolute relocations for static or static-PIE layouts on both MachO
//! and ELF, which is exactly what cranelift-object emits).

use crate::cfg::Func;
use crate::CodegenError;
use cranelift::codegen::ir::types;

/// Manifest row: 8 u64 words = 64 bytes. Word offsets are THE definition;
/// ap-aot-crt reads the same table (a drift is a link-visible size break,
/// never a silent misread — the CRT asserts `count * 64` layout).
pub const MANIFEST_ROW_WORDS: usize = 8;
pub const MANIFEST_ROW_SIZE: usize = MANIFEST_ROW_WORDS * 8;

/// Name-table row: [ptr, len] u64 pair.
pub const NAME_ROW_SIZE: usize = 16;

/// Frozen IC cell size (must match feedback.rs's cell layout pitch —
/// asserted there with offset checks; here only the SPACING matters
/// because the blob is never dereferenced by tier-0-native code).
pub const IC_CELL_PITCH: usize = 32;

/// Everything the assembler needs for one program function.
pub struct AotFnInput<'a> {
    /// The lowered cfg (the SAME stage the JIT compiles from — no
    /// duplicated lowering, §02.06 review box 2).
    pub cfg: &'a Func,
    /// Registration qualname (the CRT host seam registers under this).
    pub qualname: String,
    /// The cfg's interned name table (becomes ExecCtx::names at runtime).
    pub name_table: Vec<String>,
    /// num_locals + max temp (the slot-count rule jit_hook uses — clif's
    /// temp_slot offset must stay in bounds).
    pub num_slots: u32,
}

/// Per-function outputs the caller (ap-cli build verb) persists for the
/// embed-time registration story.
pub struct AotFnOutputs {
    /// Compile-time-static IC cell payload (all-zero cells), byte length
    /// == SlotPlan::len * IC_CELL_PITCH.
    pub ic_len: usize,
    pub ic_slot_count: u32,
}

/// THE object emitter: compile every program fn into one ObjectModule and
/// append the manifest + data blobs.
pub fn emit_program_object(
    fns: &[AotFnInput<'_>],
    source: &str,
    pic: bool,
) -> Result<(Vec<u8>, Vec<AotFnOutputs>), CodegenError> {
    use cranelift::codegen::settings::{self, Configurable};
    use cranelift_module::{Linkage, Module};
    use cranelift_object::{ObjectBuilder, ObjectModule};

    let mut flag_builder = settings::builder();
    flag_builder
        .set("use_colocated_libcalls", "false")
        .map_err(|e| CodegenError::Verify(format!("flags: {e}")))?;
    flag_builder
        .set("is_pic", if pic { "true" } else { "false" })
        .map_err(|e| CodegenError::Verify(format!("flags: {e}")))?;
    let flags = settings::Flags::new(flag_builder);
    let isa = cranelift_native::builder()
        .map_err(|_| CodegenError::Unsupported("native ISA unavailable"))?
        .finish(flags)
        .map_err(|_| CodegenError::Unsupported("ISA finish failed"))?;
    let mut ob = ObjectBuilder::new(
        isa,
        Vec::from("ap_aot_prog"),
        cranelift_module::default_libcall_names(),
    )
    .map_err(|e| CodegenError::Verify(format!("object builder: {e}")))?;
    ob.per_function_section(true);
    let mut module = ObjectModule::new(ob);

    // Fresh name-table capture for this batch (clif appends per emit).
    crate::clif::reset_emit_name_tables();

    // ---- declare program functions (Export: the manifest references
    //      them by reloc, the linker keeps only manifest-reachable ones
    //      under -dead_strip / --gc-sections) ----
    let sig = {
        let mut s = cranelift::codegen::ir::Signature::new(module.isa().default_call_conv());
        s.params.push(cranelift::codegen::ir::AbiParam::new(
            module.isa().pointer_type(),
        ));
        s.returns
            .push(cranelift::codegen::ir::AbiParam::new(types::I32));
        s
    };
    let mut func_ids = Vec::with_capacity(fns.len());
    for (i, _input) in fns.iter().enumerate() {
        let name = format!("ap_aot_fn_{i}");
        let fid = module
            .declare_function(&name, Linkage::Export, &sig)
            .map_err(|e| CodegenError::Verify(format!("declare {name}: {e}")))?;
        func_ids.push(fid);
    }

    // ---- emit each body through the SHARED lowering ----
    for (i, input) in fns.iter().enumerate() {
        crate::clif::emit_cfg_into_object(&mut module, func_ids[i], input.cfg)?;
    }

    // ---- name tables + string blobs + IC payloads ----
    // The interned name tables come from the EMIT capture (clif fills
    // them during the shared emit — the same tables ExecCtx::names
    // carries in JIT mode). A capture/input count mismatch is a
    // fail-closed build error, never a manifest row with an empty table.
    let captured = crate::clif::last_emit_name_tables();
    assert_eq!(
        captured.len(),
        fns.len(),
        "link: emit captured {} name tables for {} fns",
        captured.len(),
        fns.len()
    );
    let mut outputs = Vec::with_capacity(fns.len());
    let mut name_data_ids = Vec::with_capacity(fns.len());
    let mut ic_data_ids = Vec::with_capacity(fns.len());
    for (i, input) in fns.iter().enumerate() {
        let input = &AotFnInput {
            cfg: input.cfg,
            qualname: input.qualname.clone(),
            name_table: captured[i].clone(),
            num_slots: input.num_slots,
        };
        // String blob (one flat concatenation; rows point at offsets).
        let blob_id = module
            .declare_anonymous_data(false, false)
            .map_err(|e| CodegenError::Verify(format!("nm blob decl: {e}")))?;
        let mut blob = Vec::new();
        for n in &input.name_table {
            blob.extend_from_slice(n.as_bytes());
        }
        let mut desc = cranelift_module::DataDescription::new();
        desc.define(blob.into_boxed_slice());
        module
            .define_data(blob_id, &desc)
            .map_err(|e| CodegenError::Verify(format!("nm blob def: {e}")))?;

        // [ptr, len] rows with data relocations to the blob.
        let rows_id = module
            .declare_data(&format!("ap_aot_names_{i}"), Linkage::Export, false, false)
            .map_err(|e| CodegenError::Verify(format!("nm rows decl: {e}")))?;
        // Rows: [ptr, len] u64 pairs. DataDescription has no byte-poke
        // after define, so literals (the len words) go in first, then the
        // ptr relocations are written onto the SAME description.
        let mut desc = cranelift_module::DataDescription::new();
        let n = input.name_table.len();
        let mut rowbytes = vec![0u8; NAME_ROW_SIZE * n];
        for (k, nm) in input.name_table.iter().enumerate() {
            let base = k * NAME_ROW_SIZE;
            rowbytes[base + 8..base + 16].copy_from_slice(&(nm.len() as u64).to_le_bytes());
        }
        desc.define(rowbytes.into_boxed_slice());
        let blob_gv = module.declare_data_in_data(blob_id, &mut desc);
        for k in 0..n {
            let addend: i64 = input.name_table[..k].iter().map(|s| s.len() as i64).sum();
            desc.write_data_addr((k * NAME_ROW_SIZE) as u32, blob_gv, addend);
        }
        module
            .define_data(rows_id, &desc)
            .map_err(|e| CodegenError::Verify(format!("nm rows def: {e}")))?;
        name_data_ids.push(rows_id);

        // IC payload: SlotPlan::len zero cells (frozen pitch; see module
        // docs). Defined as data so the manifest reloc resolves.
        let slot_count = crate::clif::slot_plan_len(input.cfg);
        let ic_len = slot_count * IC_CELL_PITCH;
        // WRITABLE: the helper sites record type tags into these cells at
        // runtime (FeedbackCell::record does an atomic CAS). Tier-0-native
        // semantics keep them EMPTY-forever in practice, but the memory
        // must be writable or the first recorded site bus-errors.
        let ic_id = module
            .declare_data(&format!("ap_aot_ic_{i}"), Linkage::Export, true, false)
            .map_err(|e| CodegenError::Verify(format!("ic decl: {e}")))?;
        let mut desc = cranelift_module::DataDescription::new();
        desc.define(vec![0u8; ic_len].into_boxed_slice());
        module
            .define_data(ic_id, &desc)
            .map_err(|e| CodegenError::Verify(format!("ic def: {e}")))?;
        ic_data_ids.push(ic_id);
        outputs.push(AotFnOutputs {
            ic_len,
            ic_slot_count: slot_count as u32,
        });
    }

    // ---- the manifest (count + rows) ----
    let mf_id = module
        .declare_data("ap_aot_manifest", Linkage::Export, false, false)
        .map_err(|e| CodegenError::Verify(format!("manifest decl: {e}")))?;
    {
        let mut desc = cranelift_module::DataDescription::new();
        let mut bytes = vec![0u8; 8 + fns.len() * MANIFEST_ROW_SIZE];
        bytes[..8].copy_from_slice(&(fns.len() as u64).to_le_bytes());
        for (i, input) in fns.iter().enumerate() {
            let names = &captured[i];
            let base = 8 + i * MANIFEST_ROW_SIZE;
            // word 1: qualname_len
            bytes[base + 8..base + 16]
                .copy_from_slice(&(input.qualname.len() as u64).to_le_bytes());
            // word 3: num_slots
            bytes[base + 24..base + 32].copy_from_slice(&(input.num_slots as u64).to_le_bytes());
            // word 5: names_len (ROWS, not bytes — the CRT strides by 16)
            bytes[base + 40..base + 48].copy_from_slice(&(names.len() as u64).to_le_bytes());
            // word 7: ic_len (bytes)
            bytes[base + 56..base + 64].copy_from_slice(&(outputs[i].ic_len as u64).to_le_bytes());
        }
        desc.define(bytes.into_boxed_slice());
        // Relocation pass (offsets are layout-fixed: base + word*8).
        for (i, input) in fns.iter().enumerate() {
            let base = (8 + i * MANIFEST_ROW_SIZE) as u32;
            // word 0: qualname_ptr — anonymous text blob per fn
            let qid = module
                .declare_anonymous_data(false, false)
                .map_err(|e| CodegenError::Verify(format!("qualname blob: {e}")))?;
            let mut qdesc = cranelift_module::DataDescription::new();
            qdesc.define(input.qualname.as_bytes().to_vec().into_boxed_slice());
            module
                .define_data(qid, &qdesc)
                .map_err(|e| CodegenError::Verify(format!("qualname def: {e}")))?;
            let qgv = module.declare_data_in_data(qid, &mut desc);
            desc.write_data_addr(base, qgv, 0);
            // word 2: entry_ptr — function address reloc
            let fgv = module.declare_func_in_data(func_ids[i], &mut desc);
            desc.write_function_addr(base + 16, fgv);
            // word 4: names_ptr
            let ngv = module.declare_data_in_data(name_data_ids[i], &mut desc);
            desc.write_data_addr(base + 32, ngv, 0);
            // word 6: ic_ptr
            let igv = module.declare_data_in_data(ic_data_ids[i], &mut desc);
            desc.write_data_addr(base + 48, igv, 0);
        }
        module
            .define_data(mf_id, &desc)
            .map_err(|e| CodegenError::Verify(format!("manifest def: {e}")))?;
    }

    // ---- the embedded program source (read-only blob; the CRT runs THIS
    //      as the module body — a static binary carries its program) ----
    let src_id = module
        .declare_data("ap_aot_source", Linkage::Export, false, false)
        .map_err(|e| CodegenError::Verify(format!("source decl: {e}")))?;
    {
        let mut desc = cranelift_module::DataDescription::new();
        desc.define(source.as_bytes().to_vec().into_boxed_slice());
        module
            .define_data(src_id, &desc)
            .map_err(|e| CodegenError::Verify(format!("source def: {e}")))?;
    }
    let src_len_id = module
        .declare_data("ap_aot_source_len", Linkage::Export, false, false)
        .map_err(|e| CodegenError::Verify(format!("source len decl: {e}")))?;
    {
        let mut desc = cranelift_module::DataDescription::new();
        let mut b = source.len().to_le_bytes().to_vec();
        b.extend_from_slice(b"APSRCEND");
        desc.define(b.into_boxed_slice());
        module
            .define_data(src_len_id, &desc)
            .map_err(|e| CodegenError::Verify(format!("source len def: {e}")))?;
    }

    let product = module.finish();
    let bytes = product
        .emit()
        .map_err(|e| CodegenError::Verify(format!("object emit: {e}")))?;
    Ok((bytes, outputs))
}
