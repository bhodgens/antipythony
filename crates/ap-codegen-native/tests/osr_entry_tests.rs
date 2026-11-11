//! 02.05f — OSR entry compile + transfer tests: the entry compiles for a
//! while-loop header, the ABI round-trips a populated transfer buffer
//! (locals + live-ins), and the OSR body runs to completion with the
//! function's boxed result parked in ctx.ret. The buffer-fill/load
//! directions are pinned against the SAME osr::OsrPlan table (the
//! shared-constant-table rule from the leaf's Risk note).

use ap_codegen_native::cfg::lower_fn;
use ap_codegen_native::clif::{compile_baseline, compile_osr_entry};
use ap_codegen_native::osr::{osr_plan, osr_plan_resolved, OsrEligibility, OsrTransferBuffer};
use ap_codegen_native::runtime::{ExecCtx, HostCallbacks};
use ap_ir::expr::{BinOp, CmpOp, Expr};
use ap_ir::item::{FnBody, FnDef, Param, ParamKind, Stmt};
use ap_ir::types::TypeLattice;

fn fn_def(body: Vec<Stmt>) -> FnDef {
    FnDef {
        name: "f".to_string(),
        params: vec![Param {
            name: "n".to_string(),
            kind: ParamKind::PosOrKw,
            default: None,
            default_index: None,
            ann: None,
            ty: TypeLattice::Unknown,
        }],
        return_ann: None,
        decorators: Vec::new(),
        body: FnBody { blocks: body },
        is_generator: false,
        is_async: false,
        ret_ty: TypeLattice::Unknown,
        def_line: 1,
    }
}

/// `def hot(n): total = 0; i = 0; while i < n: total = total + i * 2; i = i + 1; return total`
fn hot_loop_body() -> Vec<Stmt> {
    vec![
        Stmt::Assign(
            vec![ap_ir::item::Target::Name("total".to_string())],
            Expr::LitInt(0),
        ),
        Stmt::Assign(
            vec![ap_ir::item::Target::Name("i".to_string())],
            Expr::LitInt(0),
        ),
        Stmt::While(
            Expr::Compare(
                vec![CmpOp::Lt],
                vec![Expr::Name("i".to_string()), Expr::Name("n".to_string())],
            ),
            vec![
                Stmt::Assign(
                    vec![ap_ir::item::Target::Name("total".to_string())],
                    Expr::BinOp(
                        BinOp::Add,
                        Box::new(Expr::Name("total".to_string())),
                        Box::new(Expr::BinOp(
                            BinOp::Mul,
                            Box::new(Expr::Name("i".to_string())),
                            Box::new(Expr::LitInt(2)),
                        )),
                    ),
                ),
                Stmt::Assign(
                    vec![ap_ir::item::Target::Name("i".to_string())],
                    Expr::BinOp(
                        BinOp::Add,
                        Box::new(Expr::Name("i".to_string())),
                        Box::new(Expr::LitInt(1)),
                    ),
                ),
            ],
            vec![],
        ),
        Stmt::Return(Some(Expr::Name("total".to_string()))),
    ]
}

fn no_host() -> HostCallbacks {
    HostCallbacks {
        load_global: |_c, _s, _n, _d| Err(ap_obj::PyException::new("SystemError", "no host")),
        call: |_c, _s, _cs, _a, _ac, _d| Err(ap_obj::PyException::new("SystemError", "no host")),
        load_attr: |_c, _s, _o, _n, _d| Err(ap_obj::PyException::new("SystemError", "no host")),
        store_attr: |_c, _s, _n, _v| Err(ap_obj::PyException::new("SystemError", "no host")),
    }
}

fn num_slots_of(func: &ap_codegen_native::cfg::Func) -> usize {
    let max_temp = ap_codegen_native::infer::max_temp(func) as usize;
    func.num_locals as usize + max_temp + 1
}

#[test]
fn osr_entry_compiles_for_while_header() {
    let f = fn_def(hot_loop_body());
    let cfg = lower_fn(&f).expect("lower");
    let headers = ap_codegen_native::osr::loop_headers(&cfg);
    assert!(!headers.is_empty(), "while loop must resolve a header");
    let header = headers[0];
    let plan = osr_plan_resolved(&cfg, header).expect("eligible");
    assert!(!plan.locals.is_empty(), "locals transferred");
    let entry = compile_osr_entry(&cfg, header).expect("OSR entry compiles");
    assert!(!entry.ptr.is_null());
    assert_eq!(entry.header, header);
}

#[test]
fn osr_entry_runs_loop_to_completion_from_transfer() {
    let f = fn_def(hot_loop_body());
    let cfg = lower_fn(&f).expect("lower");
    let headers = ap_codegen_native::osr::loop_headers(&cfg);
    let header = headers[0];
    let plan = osr_plan_resolved(&cfg, header).expect("eligible");
    let entry = compile_osr_entry(&cfg, header).expect("compile");

    // Reference: the baseline from the TOP with n=10 returns 90.
    let baseline = compile_baseline(&cfg).expect("baseline");
    let mut ctx_ref = ExecCtx::new(num_slots_of(&cfg), no_host());
    // Param bridging (the 02.05a off-by-one): params live at
    // [num_params..num_params+n_params) — pre-bumped slot 0 is dead.
    // Here num_params == 1, so the param `n` sits at slot 1.
    let n_slot = 1;
    ctx_ref.slots[n_slot] = ap_obj::value::Value::Int(ap_obj::Int::Smi(10));
    // SAFETY: code blob alive (baseline outlives the call); ctx exclusive.
    unsafe { baseline.call(&mut ctx_ref) }.expect("baseline run");
    let expected = ctx_ref.ret.clone();
    match &expected {
        ap_obj::value::Value::Int(ap_obj::Int::Smi(90)) => {}
        other => panic!("baseline reference must be 90, got {other:?}"),
    }

    // OSR entry with the SAME transfer a tier-0 poll would build: locals
    // in LocalId order, then live-ins (the shared table).
    let mut buf = OsrTransferBuffer::zeroed();
    buf.loop_header = header.0;
    let mut ctx = ExecCtx::new(num_slots_of(&cfg), no_host());
    // Populate the buffer through the plan (fill direction), simulating
    // the tier-0 h_osr_fill stream: i = 7, total = 0, n = 10 (locals),
    // then live-in temps (whatever the conservative scan found — None).
    let mut vals: Vec<ap_obj::value::Value> = Vec::new();
    for l in &plan.locals {
        vals.push(
            match cfg.local_names.get(l.0 as usize).and_then(|s| {
                if s == "i" {
                    Some(ap_obj::value::Value::Int(ap_obj::Int::Smi(7)))
                } else if s == "n" {
                    Some(ap_obj::value::Value::Int(ap_obj::Int::Smi(10)))
                } else if s == "total" {
                    // transfer snapshot: total = 0 at i = 7
                    Some(ap_obj::value::Value::Int(ap_obj::Int::Smi(0)))
                } else {
                    None
                }
            }) {
                Some(v) => v,
                None => ap_obj::value::Value::None,
            },
        );
    }
    for t in &plan.live_in {
        eprintln!(
            "OSR-TEST live-in temp t{} (slot {})",
            t.0,
            cfg.num_locals as i64 + t.0 as i64
        );
        vals.push(ap_obj::value::Value::None);
    }
    for (idx, v) in vals.iter().enumerate() {
        buf.slots[idx] = v.clone();
    }
    buf.live_count = vals.len() as u32;
    let status = unsafe { ap_codegen_native::tierup::call_osr_status(entry.ptr, &mut ctx, &buf) };
    if status != 0 {
        eprintln!(
            "OSR-TEST ctx slots: {:?}",
            ctx.slots.iter().take(24).enumerate().collect::<Vec<_>>()
        );
        panic!(
            "OSR body must complete: status={status} pending={:?}",
            ctx.pending_exc
        );
    }
    // The loop continues from i=7: total = 0 + (7..10) * 2 = 42+44+46*… =
    // 7*2+8*2+9*2 = 48; n=10 stops at i=10.
    match &ctx.ret {
        ap_obj::value::Value::Int(ap_obj::Int::Smi(48)) => {}
        other => panic!("OSR entry must resume at i=7 and return 48, got {other:?}"),
    }
    let _ = n_slot;
}

#[test]
fn osr_plan_matches_between_buffer_directions() {
    // The shared-table rule: the fill mapping (tier-0) and the load
    // mapping (entry) must index IDENTICALLY — pinned by resolving the
    // plan once and asserting the canonical order (locals asc, live-ins
    // asc, live-ins start after locals).
    let f = fn_def(hot_loop_body());
    let cfg = lower_fn(&f).expect("lower");
    let header = ap_codegen_native::osr::loop_headers(&cfg)[0];
    let plan = osr_plan_resolved(&cfg, header).expect("eligible");
    for (expect_local, l) in plan.locals.iter().enumerate() {
        assert_eq!(l.0, expect_local as u32, "locals ascending LocalId");
    }
    let mut expect_live = plan.live_in.first().map(|t| t.0).unwrap_or(0);
    for t in &plan.live_in {
        assert!(t.0 >= expect_live, "live-ins ascending TempId");
        expect_live = t.0;
    }
    assert!(
        plan.live_count() <= ap_codegen_native::osr::OSR_MAX_LIVE,
        "eligibility gate holds"
    );
    // And the eligibility enum agrees (the registry's gate reads THIS).
    assert!(matches!(
        osr_plan(&cfg, header),
        OsrEligibility::Eligible { .. }
    ));
}

#[test]
fn dump_cfg_shape() {
    let f = fn_def(hot_loop_body());
    let cfg = lower_fn(&f).expect("lower");
    for (bi, b) in cfg.blocks.iter().enumerate() {
        eprintln!(
            "block {bi}: term={:?} insts={}",
            b.term
                .as_ref()
                .map(|t| format!("{t:?}"))
                .unwrap_or_default(),
            b.insts.len()
        );
        for i in &b.insts {
            eprintln!("   {i:?}");
        }
    }
    eprintln!("headers: {:?}", ap_codegen_native::osr::loop_headers(&cfg));
    let header = ap_codegen_native::osr::loop_headers(&cfg)[0];
    let plan = osr_plan_resolved(&cfg, header).expect("eligible");
    eprintln!("plan: locals={:?} live_in={:?}", plan.locals, plan.live_in);
    eprintln!("local_names: {:?}", cfg.local_names);
}

#[test]
fn h_osr_load_direct() {
    use ap_codegen_native::runtime::h_osr_load;
    let mut buf = OsrTransferBuffer::zeroed();
    buf.slots[0] = ap_obj::value::Value::Int(ap_obj::Int::Smi(111));
    buf.slots[2] = ap_obj::value::Value::Int(ap_obj::Int::Smi(222));
    buf.live_count = 3;
    let mut ctx = ExecCtx::new(8, no_host());
    h_osr_load(&mut ctx, &buf as *const _ as *const u8, 0, 0);
    h_osr_load(&mut ctx, &buf as *const _ as *const u8, 2, 5);
    match (&ctx.slots[0], &ctx.slots[5]) {
        (
            ap_obj::value::Value::Int(ap_obj::Int::Smi(111)),
            ap_obj::value::Value::Int(ap_obj::Int::Smi(222)),
        ) => {}
        other => panic!("h_osr_load direct: {other:?}"),
    }
}
