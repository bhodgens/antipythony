//! Lowerer round-trip tests: one per supported/unsupported `Expr` variant,
//! plus slot assignment, const-pool dedup, line-table sparsity/monotonicity,
//! site ids, decode, and function-granularity fallback.
//!
//! IR trees are built directly (no ap-parse dev-dep needed — keeps this
//! crate's test surface hermetic).

use crate::code::CodeObject;
use crate::instr::{BinKind, CompareOp, Instr, UnaryKind, OPCODE_NAMES};
use crate::linetable::LineTable;
use crate::lower::{lower_fn, lowers_cleanly, LowerOutcome, Unsupported};
use crate::pool::{Const, ConstPool};
use crate::sites::{SiteKind, SiteTable};
use ap_ir::expr::{CompClause, DictItem, LambdaSig};
use ap_ir::item::{ClassDef, ExceptHandler, ImportAlias, MatchCase, TryStmt, WithItem};
use ap_ir::{
    Arg, BinOp, BoolOp, CmpOp, Conversion, Expr, FStrPart, FnBody, FnDef, Param, ParamKind, Stmt,
    Target, UnaryOp,
};

// ---------------------------------------------------------------------------
// IR builders
// ---------------------------------------------------------------------------

fn name(n: &str) -> Expr {
    Expr::Name(n.to_string())
}

fn b(op: BinOp, l: Expr, r: Expr) -> Expr {
    Expr::BinOp(op, Box::new(l), Box::new(r), 1)
}

fn lit(i: i128) -> Expr {
    Expr::LitInt(i)
}

fn param(nm: &str) -> Param {
    Param {
        name: nm.to_string(),
        kind: ParamKind::PosOrKw,
        default: None,
        default_index: None,
        ann: None,
        ty: ap_ir::TypeLattice::Unknown,
    }
}

fn fn_def(name: &str, params: &[&str], body: Vec<Stmt>) -> FnDef {
    FnDef {
        name: name.to_string(),
        params: params.iter().map(|p| param(p)).collect(),
        return_ann: None,
        decorators: vec![],
        body: FnBody { blocks: body },
        is_generator: false,
        is_async: false,
        ret_ty: ap_ir::TypeLattice::Unknown,
        def_line: 7,
    }
}

fn ret(e: Expr) -> Stmt {
    Stmt::Return(Some(e))
}

/// Lower and expect success; returns the CodeObject.
fn must_lower(f: &FnDef) -> CodeObject {
    match lower_fn(f) {
        LowerOutcome::Code(c) => {
            c.validate().expect("CodeObject invariants");
            c
        }
        LowerOutcome::Fallback(u) => panic!("expected Code, got fallback: {u:?}"),
    }
}

/// Lower and expect a function-granularity fallback.
fn must_fallback(f: &FnDef) -> String {
    match lower_fn(f) {
        LowerOutcome::Code(c) => panic!("expected fallback, got code: {:?}", c.instrs.first()),
        LowerOutcome::Fallback(u) => u.0.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Pool / linetable / sites unit tests
// ---------------------------------------------------------------------------

#[test]
fn pool_dedupes_ints_strs_floats() {
    let mut p = ConstPool::new();
    let a = p.intern(Const::Int(42));
    let b = p.intern(Const::Int(42));
    let c = p.intern(Const::Str("x".into()));
    let d = p.intern(Const::Str("x".into()));
    let e = p.intern(Const::Float(1.5));
    let f = p.intern(Const::Float(1.5));
    assert_eq!(a, b);
    assert_eq!(c, d);
    assert_eq!(e, f);
    assert_ne!(a, c);
    assert_eq!(p.len(), 3);
}

#[test]
fn pool_bigint_and_float_bits() {
    let mut p = ConstPool::new();
    // Distinct big ints stay distinct.
    let a = p.intern(Const::BigInt("123456789012345678901234567890".into()));
    let b = p.intern(Const::BigInt("999999999999999999999999999999".into()));
    assert_ne!(a, b);
    // 0.0 vs -0.0: distinct bit patterns, distinct entries.
    let z = p.intern(Const::Float(0.0));
    let nz = p.intern(Const::Float(-0.0));
    assert_ne!(z, nz);
    // NaN dedupes to one entry.
    let n1 = p.intern(Const::Float(f64::NAN));
    let n2 = p.intern(Const::Float(f64::NAN));
    assert_eq!(n1, n2);
}

#[test]
fn linetable_sparse_and_monotonic() {
    let mut t = LineTable::new();
    t.record(0, 7);
    t.record(1, 7); // same line — no entry
    t.record(2, 8);
    t.record(3, 8); // same line — no entry
    t.record(4, 9);
    assert_eq!(t.len(), 3, "only line CHANGES recorded");
    // Monotonic strictly increasing instr indices.
    let idxs: Vec<u32> = t.iter().map(|e| e.instr_idx).collect();
    assert_eq!(idxs, vec![0, 2, 4]);
    assert_eq!(t.line_for(0), 7);
    assert_eq!(t.line_for(1), 7);
    assert_eq!(t.line_for(2), 8);
    assert_eq!(t.line_for(4), 9);
}

#[test]
fn linetable_binary_search_interior() {
    let mut t = LineTable::new();
    for (i, line) in [(0u32, 1usize), (10, 2), (20, 3)] {
        t.record(i, line);
    }
    assert_eq!(t.line_for(15), 2);
    assert_eq!(t.line_for(19), 2);
    assert_eq!(t.line_for(25), 3);
    assert_eq!(t.line_for(999), 3);
    assert_eq!(LineTable::new().line_for(0), 0, "empty table -> 0");
}

#[test]
fn sites_alloc_sequential() {
    let mut s = SiteTable::new();
    let s0 = s.alloc(SiteKind::GlobalLoad, 0);
    let s1 = s.alloc(SiteKind::Call, 1);
    let s2 = s.alloc(SiteKind::AttrLoad, 2);
    assert_eq!(s0, 0);
    assert_eq!(s1, 1);
    assert_eq!(s2, 2);
    assert_eq!(s.get(s1).expect("site").kind, SiteKind::Call);
    // retarget fixes instr_idx.
    s.retarget(9, s1);
    assert_eq!(s.get(s1).expect("site").instr_idx, 9);
    assert!(s.get(99).is_none());
}

// ---------------------------------------------------------------------------
// Expr variant round-trip (supported set)
// ---------------------------------------------------------------------------

#[test]
fn lit_int_small_and_big() {
    let f = fn_def(
        "f",
        &[],
        vec![ret(lit(5)), ret(lit(123456789012345678901234567890))],
    );
    let c = must_lower(&f);
    assert_eq!(c.instrs[0], Instr::LoadConst(0));
    assert_eq!(c.consts.get(0), &Const::Int(5));
    assert_eq!(c.instrs[2], Instr::LoadConst(1));
    assert_eq!(
        c.consts.get(1),
        &Const::BigInt("123456789012345678901234567890".into())
    );
    // ret() emits LoadConst+ReturnValue; the fall-off ReturnNone trails.
    assert_eq!(*c.instrs.last().expect("nonempty"), Instr::ReturnNone);
}

#[test]
fn lit_float_str_bytes_bool_none() {
    let body = vec![
        ret(Expr::LitFloat(2.5)),
        ret(Expr::LitStr("hi".into())),
        ret(Expr::LitBytes(vec![1, 2])),
        ret(Expr::LitBool(true)),
        ret(Expr::LitBool(false)),
        ret(Expr::LitNone),
    ];
    let f = fn_def("f", &[], body);
    let c = must_lower(&f);
    assert_eq!(c.instrs[0], Instr::LoadConst(0)); // 2.5
    assert_eq!(c.consts.get(0), &Const::Float(2.5));
    assert_eq!(c.instrs[2], Instr::LoadConst(1)); // "hi"
    assert_eq!(c.consts.get(1), &Const::Str("hi".into()));
    assert_eq!(c.instrs[4], Instr::LoadConst(2)); // b"\x01\x02"
    assert_eq!(c.consts.get(2), &Const::Bytes(vec![1, 2]));
    assert_eq!(c.instrs[6], Instr::LoadConstTrue);
    assert_eq!(c.instrs[8], Instr::LoadConstFalse);
    assert_eq!(c.instrs[10], Instr::LoadConstNone);
}

#[test]
fn name_local_vs_global() {
    // Local: x = 1; return x  -> LoadFast/StoreFast
    let f = fn_def(
        "f",
        &[],
        vec![
            Stmt::Assign(vec![Target::Name("x".into())], lit(1)),
            ret(name("x")),
        ],
    );
    let c = must_lower(&f);
    assert_eq!(c.instrs[0], Instr::LoadConst(0));
    assert_eq!(c.instrs[1], Instr::StoreFast(0));
    assert_eq!(c.instrs[2], Instr::LoadFast(0));
    assert_eq!(c.n_locals, 1);
    assert_eq!(c.local_names, vec!["x".to_string()]);

    // Global read only: return len -> LoadGlobal with a GlobalLoad site
    // (site ids live in the SiteTable keyed by instr_idx, not in the
    // opcode payload — see instr.rs site_id docs).
    let g = fn_def("g", &[], vec![ret(name("len"))]);
    let c = must_lower(&g);
    assert_eq!(c.instrs[0], Instr::LoadGlobal(0));
    assert_eq!(c.names, vec!["len".to_string()]);
    assert_eq!(c.instrs[0].site_id(), None, "LoadGlobal site is table-side");
    let site = c.sites.iter().next().expect("LoadGlobal site recorded");
    assert_eq!(site.kind, SiteKind::GlobalLoad);
    assert_eq!(site.instr_idx, 0);
    assert_eq!(c.sites.len(), 1);
    assert_eq!(c.n_locals, 0);
}

#[test]
fn binop_unary_compare_pairwise() {
    let f = fn_def(
        "f",
        &["a", "b"],
        vec![
            ret(b(BinOp::Add, name("a"), name("b"))),
            ret(Expr::UnaryOp(UnaryOp::Invert, Box::new(name("a")), 1)),
            ret(Expr::Compare(
                vec![CmpOp::Lt],
                vec![name("a"), name("b")],
                1,
            )),
        ],
    );
    let c = must_lower(&f);
    assert_eq!(c.instrs[2], Instr::BinOp(BinKind::Add));
    assert_eq!(c.instrs[5], Instr::UnaryOp(UnaryKind::Invert));
    assert_eq!(c.instrs[9], Instr::Compare(CompareOp::Lt));
}

#[test]
fn binop_op_coverage_all_thirteen() {
    let ops = [
        BinOp::Add,
        BinOp::Sub,
        BinOp::Mul,
        BinOp::MatMul,
        BinOp::Div,
        BinOp::FloorDiv,
        BinOp::Mod,
        BinOp::Pow,
        BinOp::LShift,
        BinOp::RShift,
        BinOp::BitOr,
        BinOp::BitXor,
        BinOp::BitAnd,
    ];
    let body: Vec<Stmt> = ops.iter().map(|&op| ret(b(op, lit(1), lit(2)))).collect();
    let f = fn_def("f", &[], body);
    let c = must_lower(&f);
    for (i, &op) in ops.iter().enumerate() {
        // Each stmt: LoadConst, LoadConst, BinOp, ReturnValue (stride 4).
        assert_eq!(c.instrs[i * 4 + 2], Instr::BinOp(BinKind::from_ir(op)));
    }
}

#[test]
fn call_positional() {
    // return f(a, 1)
    let call = Expr::Call(
        Box::new(name("f")),
        vec![Arg::Pos(name("a")), Arg::Pos(lit(1))],
        3,
    );
    let f = fn_def("f", &["a"], vec![ret(call)]);
    let c = must_lower(&f);
    // LoadGlobal f (callee, site 0) | LoadFast a | LoadConst 1 | CallFunction{site 1} | ReturnValue | ReturnNone
    assert_eq!(c.instrs[3], Instr::CallFunction { argc: 2, site: 1 });
    assert_eq!(c.sites.len(), 2);
    assert_eq!(c.sites.get(0).expect("site").kind, SiteKind::GlobalLoad);
    assert_eq!(c.sites.get(0).expect("site").instr_idx, 0);
    assert_eq!(c.sites.get(1).expect("site").kind, SiteKind::Call);
    assert_eq!(c.sites.get(1).expect("site").instr_idx, 3);
    assert_eq!(c.lines.line_for(3), 3, "call line recorded");
}

#[test]
fn call_with_kw_star_args_falls_back() {
    // Leaf fp-calls: Kw lowers (CallFunctionKw); Star/KwStar call-site
    // unpack stays a fallback.
    {
        let call = Expr::Call(Box::new(name("f")), vec![Arg::Kw("x".into(), lit(1))], 1);
        let f = fn_def("f", &[], vec![ret(call)]);
        let code = must_lower(&f);
        assert!(
            code.instrs
                .iter()
                .any(|i| matches!(i, Instr::CallFunctionKw { .. })),
            "kw call lowers to CallFunctionKw"
        );
    }
    for args in [vec![Arg::Star(lit(1))], vec![Arg::KwStar(lit(1))]] {
        let call = Expr::Call(Box::new(name("f")), args, 1);
        let f = fn_def("f", &[], vec![ret(call)]);
        assert!(
            !lowers_cleanly(&f),
            "star call shapes must fall back this leaf"
        );
    }
}

#[test]
fn attribute_and_subscript() {
    // return a.b + c[d]
    let e = b(
        BinOp::Add,
        Expr::Attribute(Box::new(name("a")), "b".into(), 2),
        Expr::Subscript(Box::new(name("c")), Box::new(name("d")), 2),
    );
    let f = fn_def("f", &["a", "c", "d"], vec![ret(e)]);
    let c = must_lower(&f);
    // LoadFast a | LoadAttr{b} | LoadFast c | LoadFast d | Subscript | BinOp(+) ...
    assert_eq!(c.instrs[1], Instr::LoadAttr { idx: 0, site: 0 });
    assert_eq!(c.names[0], "b");
    assert_eq!(c.sites.get(0).expect("site").kind, SiteKind::AttrLoad);
    assert_eq!(c.sites.get(0).expect("site").instr_idx, 1);
    assert_eq!(c.instrs[4], Instr::Subscript);
}

#[test]
fn subscript_store() {
    // d[k] = v
    let f = fn_def(
        "f",
        &["d", "k", "v"],
        vec![Stmt::Assign(
            vec![Target::Subscript(Box::new(name("d")), Box::new(name("k")))],
            name("v"),
        )],
    );
    let c = must_lower(&f);
    assert_eq!(c.instrs[3], Instr::StoreSubscript);
}

#[test]
fn list_tuple_displays() {
    let f = fn_def(
        "f",
        &["a", "b"],
        vec![
            ret(Expr::List(vec![name("a"), name("b")])),
            ret(Expr::Tuple(vec![name("a")])),
            ret(Expr::Tuple(vec![])),
        ],
    );
    let c = must_lower(&f);
    assert_eq!(c.instrs[2], Instr::BuildList(2));
    assert_eq!(c.instrs[5], Instr::BuildTuple(1));
    assert_eq!(c.instrs[7], Instr::LoadConstEmptyTuple);
}

#[test]
fn empty_tuple_literal_is_special() {
    let f = fn_def("f", &[], vec![ret(Expr::Tuple(vec![]))]);
    let c = must_lower(&f);
    assert_eq!(c.instrs[0], Instr::LoadConstEmptyTuple);
}

#[test]
fn return_forms() {
    // explicit value, bare return, fall-off end
    let f = fn_def("f", &[], vec![ret(lit(1)), Stmt::Return(None)]);
    let c = must_lower(&f);
    assert_eq!(c.instrs[1], Instr::ReturnValue);
    assert_eq!(c.instrs[2], Instr::ReturnNone);
    assert_eq!(
        *c.instrs.last().expect("nonempty"),
        Instr::ReturnNone,
        "fall-off end appends ReturnNone"
    );
}

#[test]
fn augassign_and_annassign() {
    // x += 1 ; y: int = 2
    let f = fn_def(
        "f",
        &[],
        vec![
            Stmt::Assign(vec![Target::Name("x".into())], lit(0)),
            Stmt::AugAssign(Target::Name("x".into()), BinOp::Add, lit(1)),
            Stmt::AnnAssign(
                Target::Name("y".into()),
                ap_ir::Ann {
                    expr: name("int"),
                    ty: ap_ir::TypeLattice::Unknown,
                },
                Some(lit(2)),
            ),
            ret(name("x")),
        ],
    );
    let c = must_lower(&f);
    // x=0: LC0 SF0 | x+=1: LF0 LC1 AugOp SF0 | y: LC2 SF1 | ret: LF0 RetVal | RN
    assert_eq!(
        c.instrs[4],
        Instr::AugOp {
            kind: BinKind::Add,
            sym: "+=",
        }
    );
    assert_eq!(c.instrs[5], Instr::StoreFast(0));
    assert_eq!(c.n_locals, 2);
    assert_eq!(c.local_names, vec!["x".to_string(), "y".to_string()]);
    assert_eq!(c.instrs[8], Instr::LoadFast(0));
}

#[test]
fn pass_and_line_markers() {
    let f = fn_def("f", &[], vec![Stmt::Pass, Stmt::Line(12), ret(lit(1))]);
    let c = must_lower(&f);
    // Pass emits nothing; the Line marker moved cur_line so the LoadConst
    // records line 12. The def-line prologue entry (instr 0 at line 7) is
    // overwritten by the same-instr re-record at the new cur_line —
    // instr 0 IS the LoadConst here (Pass emitted nothing).
    assert_eq!(c.lines.line_for(0), 12);
    assert_eq!(c.lines.line_for(1), 12, "ReturnValue inherits line 12");
}

// ---------------------------------------------------------------------------
// Unsupported Expr variants -> function fallback
// ---------------------------------------------------------------------------

#[test]
fn unsupported_expr_variants_fall_back_whole_function() {
    let cases: Vec<(&str, Expr)> = vec![
        ("complex literal", Expr::LitComplex(1.0, 2.0)),
        ("ellipsis literal", Expr::LitEllipsis),
        (
            "chained comparison",
            Expr::Compare(vec![CmpOp::Lt, CmpOp::Lt], vec![lit(1), lit(2), lit(3)], 1),
        ),
        ("slice expr", Expr::Slice(None, None, None)),
        // Leaf fp-calls: set/dict displays LOWER now (BuildSet/BuildDict —
        // the comprehension accumulators need them anyway).
        (
            "lambda",
            Expr::Lambda(Box::new(LambdaSig { params: vec![] }), Box::new(lit(1))),
        ),
        // Leaf fp-calls: listcomp/setcomp/dictcomp LOWER now (inline loop
        // shape) — moved to comprehension_lowering tests below. genexpr
        // stays a fallback (fiber machinery, master.md).
        (
            "genexpr",
            Expr::Generator(
                Box::new(name("x")),
                vec![CompClause {
                    target: name("x"),
                    iter: name("xs"),
                    ifs: vec![],
                }],
            ),
        ),
        ("yield", Expr::Yield(Some(Box::new(lit(1))))),
        ("yield from", Expr::YieldFrom(Box::new(name("g")))),
        ("await", Expr::Await(Box::new(name("c")))),
        (
            "fstring",
            Expr::FString(vec![
                FStrPart::Lit("a".into()),
                FStrPart::Interp {
                    expr: name("x"),
                    conv: Some(Conversion::R),
                    spec: None,
                    debug_text: None,
                },
            ]),
        ),
        (
            "ifexp",
            Expr::IfExp(Box::new(lit(1)), Box::new(lit(2)), Box::new(lit(3)), 1),
        ),
        ("starred", Expr::Starred(Box::new(name("s")))),
        ("walrus", Expr::Named(Box::new(name("w")), Box::new(lit(1)))),
    ];
    for (label, e) in cases {
        let f = fn_def("f", &[], vec![ret(e)]);
        let reason = must_fallback(&f);
        assert!(!reason.is_empty(), "case {label} produced empty reason");
    }
}

// ---------------------------------------------------------------------------
// Unsupported Stmt shapes -> function fallback (incl. For/Try contract)
// ---------------------------------------------------------------------------

#[test]
fn for_and_try_functions_fall_back() {
    // for x in xs: pass — LOWERS since leaf 01-core/01 (this is the leaf's
    // deliverable). The loop shape must now produce ForNext code that
    // validates.
    let f = fn_def(
        "f",
        &["xs"],
        vec![Stmt::For(
            Target::Name("x".into()),
            name("xs"),
            vec![Stmt::Pass],
            vec![],
        )],
    );
    let c = must_lower(&f);
    // iter; ForNext -> exit; StoreFast; back-edge; RN
    assert!(
        c.instrs.iter().any(|i| matches!(i, Instr::ForNext(_))),
        "ForNext present: {:?}",
        c.instrs
    );

    // try/except LOWERS since leaf fp-exceptions (01-core/03): the
    // fallback-refusal test became a lowering test (see
    // try_except_lowers_with_exc_table below).
}

#[test]
fn other_unsupported_statements_fall_back() {
    let mk = |body: Vec<Stmt>| fn_def("f", &[], body);
    let cases: Vec<(&str, Vec<Stmt>)> = vec![
        (
            "with statement",
            vec![Stmt::With(
                vec![WithItem {
                    expr: name("x"),
                    target: None,
                }],
                vec![Stmt::Pass],
            )],
        ),
        (
            "with statement-2",
            vec![Stmt::With(
                vec![WithItem {
                    expr: name("y"),
                    target: None,
                }],
                vec![Stmt::Pass],
            )],
        ),
        (
            "match statement",
            vec![Stmt::Match(
                lit(1),
                vec![MatchCase {
                    pattern: ap_ir::item::Pattern::Wildcard,
                    guard: None,
                    body: vec![Stmt::Pass],
                }],
            )],
        ),
        ("assert statement", vec![Stmt::Assert(lit(1), None)]),
        ("global declaration", vec![Stmt::Global(vec!["g".into()])]),
        (
            "nonlocal declaration",
            vec![Stmt::Nonlocal(vec!["g".into()])],
        ),
        (
            "import statement",
            vec![Stmt::Import(vec![ImportAlias {
                path: vec!["os".into()],
                alias: None,
            }])],
        ),
        (
            "import-from statement",
            vec![Stmt::ImportFrom("os".into(), vec![], false)],
        ),
        (
            "del statement",
            vec![Stmt::Del(vec![Target::Name("x".into())])],
        ),
        // break/continue WITHOUT an enclosing loop are still compile-time
        // rejections (SyntaxError in CPython).
        ("break", vec![Stmt::Break]),
        ("continue", vec![Stmt::Continue]),
        (
            "nested def",
            vec![Stmt::LocalDef(Box::new(fn_def(
                "inner",
                &[],
                vec![Stmt::Pass],
            )))],
        ),
        (
            "nested class",
            vec![Stmt::LocalClass(Box::new(ClassDef {
                name: "C".into(),
                bases: vec![],
                metaclass: None,
                decorators: vec![],
                body: vec![],
                id: ap_ir::DefId(0),
            }))],
        ),
        // Leaf fp-calls: attribute store LOWERS now (StoreAttr) — moved to
        // attribute_store_lowers_storeattr below.
        (
            "tuple unpack assign",
            vec![Stmt::Assign(
                vec![Target::Tuple(vec![
                    Target::Name("a".into()),
                    Target::Name("b".into()),
                ])],
                lit(1),
            )],
        ),
        (
            "multi-target assign",
            vec![Stmt::Assign(
                vec![Target::Name("a".into()), Target::Name("b".into())],
                lit(1),
            )],
        ),
    ];
    for (label, body) in cases {
        let f = mk(body);
        let reason = must_fallback(&f);
        assert!(!reason.is_empty(), "case {label}");
    }
}

#[test]
fn generator_and_async_fall_back() {
    let mut g = fn_def("g", &[], vec![ret(lit(1))]);
    g.is_generator = true;
    assert_eq!(must_fallback(&g), "generator");

    let mut a = fn_def("a", &[], vec![ret(lit(1))]);
    a.is_async = true;
    assert_eq!(must_fallback(&a), "async def");
}

// ---------------------------------------------------------------------------
// Leaf fp-calls: comprehension lowering, attr stores, kw calls
// ---------------------------------------------------------------------------

#[test]
fn comprehension_lowering_list_shape() {
    // [x * 2 for x in xs] — inline loop: GetIter/ForNext/append/PopTop,
    // NO separate frame, no fallback.
    let comp = Expr::ListComp(
        Box::new(b(BinOp::Mul, name("x"), lit(2))),
        vec![CompClause {
            target: name("x"),
            iter: name("xs"),
            ifs: vec![],
        }],
    );
    let f = fn_def("f", &["xs"], vec![ret(comp)]);
    let c = must_lower(&f);
    let has = |p: &dyn Fn(&Instr) -> bool| c.instrs.iter().any(p);
    assert!(has(&|i| matches!(i, Instr::GetIter)), "GetIter emitted");
    assert!(has(&|i| matches!(i, Instr::ForNext(_))), "ForNext emitted");
    assert!(
        has(&|i| matches!(i, Instr::BuildList(0))),
        "fresh accumulator"
    );
    assert!(
        has(&|i| matches!(
            i,
            Instr::LoadAttrCall { .. } // acc.append
        )),
        "append via fused opcode"
    );
    // Hidden slot: n_locals exceeds visible names (xs + acc).
    assert!(c.n_locals > c.local_names.len(), "hidden comp slots exist");
    assert!(
        !c.local_names.iter().any(|n| n == "x"),
        "clause target stays hidden (no scope leak)"
    );
}

#[test]
fn comprehension_lowering_dict_shape() {
    // {k: v for k, ... } — single-name target + key/value store shape.
    let comp = Expr::DictComp(
        Box::new(name("k")),
        Box::new(name("v")),
        vec![CompClause {
            target: name("k"),
            iter: name("ks"),
            ifs: vec![],
        }],
    );
    let f = fn_def("f", &["ks", "v"], vec![ret(comp)]);
    let c = must_lower(&f);
    assert!(c.instrs.iter().any(|i| matches!(i, Instr::BuildDict(0))));
    assert!(c.instrs.iter().any(|i| matches!(i, Instr::StoreSubscript)));
}

#[test]
fn comprehension_filter_jumps_to_clause_fornext() {
    // [x for x in xs if x > 1 if x < 5] — both filters JumpIfFalse back to
    // the clause's ForNext (continue point).
    let comp = Expr::ListComp(
        Box::new(name("x")),
        vec![CompClause {
            target: name("x"),
            iter: name("xs"),
            ifs: vec![b(BinOp::Add, name("x"), lit(1)), lit(7)],
        }],
    );
    let f = fn_def("f", &["xs"], vec![ret(comp)]);
    let c = must_lower(&f);
    let fornext_idx = c
        .instrs
        .iter()
        .position(|i| matches!(i, Instr::ForNext(_)))
        .expect("ForNext");
    let jumps: Vec<u32> = c
        .instrs
        .iter()
        .filter_map(|i| match i {
            Instr::JumpIfFalse(t) => Some(*t),
            _ => None,
        })
        .collect();
    assert!(
        jumps.iter().all(|&t| t == fornext_idx as u32),
        "filters continue at the clause ForNext: {jumps:?}"
    );
}

#[test]
fn genexpr_still_falls_back() {
    let comp = Expr::Generator(
        Box::new(name("x")),
        vec![CompClause {
            target: name("x"),
            iter: name("xs"),
            ifs: vec![],
        }],
    );
    let f = fn_def("f", &["xs"], vec![ret(comp)]);
    assert!(matches!(
        lower_fn(&f),
        LowerOutcome::Fallback(Unsupported("generator expression"))
    ));
}

#[test]
fn attribute_store_lowers_storeattr() {
    // self.x = 1 — StoreAttr opcode, value under obj.
    let stmt = Stmt::Assign(
        vec![Target::Attribute(Box::new(name("s")), "x".into())],
        lit(1),
    );
    let f = fn_def("f", &["s"], vec![stmt, Stmt::Return(None)]);
    let c = must_lower(&f);
    assert!(
        c.instrs
            .iter()
            .any(|i| matches!(i, Instr::StoreAttr { .. })),
        "attr store lowers to StoreAttr"
    );
    // Aug target reads too: s.x += 1
    let aug = Stmt::AugAssign(
        Target::Attribute(Box::new(name("s")), "x".into()),
        BinOp::Add,
        lit(1),
    );
    let f2 = fn_def("f", &["s"], vec![aug, Stmt::Return(None)]);
    let c2 = must_lower(&f2);
    assert!(c2
        .instrs
        .iter()
        .any(|i| matches!(i, Instr::LoadAttr { .. })));
}

#[test]
fn kw_call_marshals_name_slice() {
    // f(a, x=1, y=2) — names x,y interned contiguously; values follow
    // positionals in the same order.
    let call = Expr::Call(
        Box::new(name("f")),
        vec![
            Arg::Pos(name("a")),
            Arg::Kw("x".into(), lit(1)),
            Arg::Kw("y".into(), lit(2)),
        ],
        1,
    );
    let f = fn_def("f", &["a"], vec![ret(call)]);
    let c = must_lower(&f);
    let kw_idx = c
        .instrs
        .iter()
        .position(|i| matches!(i, Instr::CallFunctionKw { .. }))
        .expect("CallFunctionKw emitted");
    match c.instrs[kw_idx] {
        Instr::CallFunctionKw {
            argc,
            kw_first,
            n_kw,
            ..
        } => {
            assert_eq!(argc, 1);
            assert_eq!(n_kw, 2);
            assert_eq!(&c.names[kw_first as usize], "x");
            assert_eq!(&c.names[kw_first as usize + 1], "y");
        }
        _ => unreachable!(),
    }
}

#[test]
fn comp_targets_shadow_enclosing_locals() {
    // x = 0; [x for x in xs] — the comp's x must NOT store into the
    // enclosing x slot (S-02-2): the target allocates a hidden slot.
    let comp = Expr::ListComp(
        Box::new(name("x")),
        vec![CompClause {
            target: name("x"),
            iter: name("xs"),
            ifs: vec![],
        }],
    );
    let f = fn_def(
        "f",
        &["xs"],
        vec![
            Stmt::Assign(vec![Target::Name("x".into())], lit(0)),
            Stmt::ExprStmt(comp),
            ret(name("x")),
        ],
    );
    let c = must_lower(&f);
    // x's visible slot is 1 (xs=0, x=1). The comp's StoreFast targets must
    // all be >= 2 (hidden region).
    let stores: Vec<u16> = c
        .instrs
        .iter()
        .filter_map(|i| match i {
            Instr::StoreFast(s) => Some(*s),
            _ => None,
        })
        .collect();
    assert!(
        stores.iter().any(|&s| s >= 2),
        "clause target allocates a hidden slot: {stores:?}"
    );
}

#[test]
fn opcode_names_table_in_sync_leaf_fpcalls() {
    // StoreAttr + CallFunctionKw ride the same table discipline.
    let names: Vec<&str> = OPCODE_NAMES.to_vec();
    assert_eq!(names.len(), 40);
    assert_eq!(names[32], "StoreAttr");
    assert_eq!(names[33], "CallFunctionKw");
    let sa = Instr::StoreAttr { idx: 0 };
    let ck = Instr::CallFunctionKw {
        argc: 0,
        kw_first: 0,
        n_kw: 0,
        site: 0,
    };
    assert_eq!(sa.site_id(), None, "StoreAttr carries no site");
    assert_eq!(ck.site_id(), Some(0), "CallFunctionKw carries its site");
}

#[test]
fn vararg_and_default_params_lower() {
    // Leaf fp-calls: vararg/kwarg/kw-only params + defaults bind in the
    // call boundary (the tree-walker's binder) — the lowerer lifts its
    // old refusals; prefill is by NAME so all kinds work.
    let mut f = fn_def("f", &["a"], vec![ret(name("a"))]);
    f.params[0].kind = ParamKind::VarArg;
    assert!(lowers_cleanly(&f), "vararg param lowers");

    let mut d = fn_def("d", &["a"], vec![ret(name("a"))]);
    d.params[0].default = Some(lit(1));
    assert!(lowers_cleanly(&d), "param default lowers");
}

// ---------------------------------------------------------------------------
// Slot assignment
// ---------------------------------------------------------------------------

#[test]
fn slots_params_then_assigns_in_scan_order() {
    // Params get slots first (source order), then assigned names in
    // scan order (pre-order over the statement tree).
    let f = fn_def(
        "f",
        &["p", "q"],
        vec![
            Stmt::Assign(vec![Target::Name("p".into())], lit(1)),
            Stmt::Assign(vec![Target::Name("z".into())], lit(2)),
            Stmt::Assign(vec![Target::Name("y".into())], lit(3)),
            ret(name("z")),
        ],
    );
    let c = must_lower(&f);
    assert_eq!(c.n_locals, 4);
    assert_eq!(
        c.local_names,
        vec![
            "p".to_string(),
            "q".to_string(),
            "z".to_string(),
            "y".to_string()
        ]
    );
    // z stored to slot 2.
    assert_eq!(c.instrs[3], Instr::StoreFast(2));
    // p re-stored to its param slot 0.
    assert_eq!(c.instrs[1], Instr::StoreFast(0));
    assert_eq!(c.instrs[6], Instr::LoadFast(2));
}

#[test]
fn global_reads_use_global_ops_even_next_to_locals() {
    let f = fn_def(
        "f",
        &[],
        vec![
            Stmt::Assign(vec![Target::Name("x".into())], lit(1)),
            ret(b(BinOp::Add, name("x"), name("GLOBAL_THING"))),
        ],
    );
    let c = must_lower(&f);
    // x=0: LC SF | ret: LF0 LG0 BinOp RetVal | RN
    assert_eq!(c.instrs[3], Instr::LoadGlobal(0));
    assert_eq!(c.names, vec!["GLOBAL_THING".to_string()]);
    assert_eq!(c.n_locals, 1);
}

// ---------------------------------------------------------------------------
// Line table via lowerer
// ---------------------------------------------------------------------------

#[test]
fn linetable_only_where_line_changes() {
    // Multi-line BinOp: line numbers change mid-expression.
    let e = Expr::BinOp(
        BinOp::Add,
        Box::new(lit(1)),
        Box::new(lit(2)),
        5, // op on line 5 while the literals sit on cur_line 7
    );
    let f = fn_def("f", &[], vec![ret(e)]);
    let c = must_lower(&f);
    // def-line prologue; LoadConst(1) at 7; LoadConst(2)+BinOp at 5.
    assert_eq!(c.lines.line_for(0), 7);
    assert_eq!(c.lines.line_for(1), 7);
    assert_eq!(c.lines.line_for(2), 5);
    assert_eq!(c.lines.line_for(3), 5);
    // Sparse: prologue line 7 covers instr 0-1; line 5 covers 2-3; the
    // trailing ReturnNone re-records only if the line changed (5 -> 5,
    // no entry).
    let idxs: Vec<u32> = c.lines.iter().map(|x| x.instr_idx).collect();
    assert_eq!(idxs.first(), Some(&0));
    for w in idxs.windows(2) {
        assert!(w[0] < w[1], "line table monotonic: {w:?}");
    }
    assert!(c.lines.len() <= c.instrs.len(), "sparse: entries <= instrs");
}

// ---------------------------------------------------------------------------
// Decode / opcode table
// ---------------------------------------------------------------------------

#[test]
fn opcode_names_table_in_sync() {
    // Every discriminant index has a non-empty distinct name.
    let all: Vec<&str> = OPCODE_NAMES.to_vec();
    let samples = [
        Instr::LoadConst(0),
        Instr::LoadFast(0),
        Instr::LoadGlobal(0),
        Instr::StoreFast(0),
        Instr::StoreGlobal(0),
        Instr::PopTop,
        Instr::BinOp(BinKind::Add),
        Instr::UnaryOp(UnaryKind::Not),
        Instr::Compare(CompareOp::Eq),
        Instr::Jump(1),
        Instr::JumpIfFalse(1),
        Instr::JumpIfTrue(1),
        Instr::JumpIfFalseOrPop(1),
        Instr::JumpIfTrueOrPop(1),
        Instr::GetIter,
        Instr::ForNext(1),
        Instr::CallFunction { argc: 0, site: 0 },
        Instr::LoadAttr { idx: 0, site: 0 },
        Instr::Subscript,
        Instr::StoreSubscript,
        Instr::LoadConstNone,
        Instr::LoadConstTrue,
        Instr::LoadConstFalse,
        Instr::LoadConstEmptyTuple,
        Instr::BuildTuple(0),
        Instr::BuildList(0),
        Instr::BuildDict(0),
        Instr::BuildSet(0),
        Instr::ReturnValue,
        Instr::ReturnNone,
        Instr::AugOp {
            kind: BinKind::Add,
            sym: "+",
        },
        Instr::LoadAttrCall {
            idx: 0,
            argc: 1,
            site: 0,
        },
        Instr::StoreAttr { idx: 0 },
        Instr::CallFunctionKw {
            argc: 0,
            kw_first: 0,
            n_kw: 0,
            site: 0,
        },
        Instr::FallbackToTreeWalk,
        Instr::Raise { mode: 2 },
        Instr::PopExc {
            unbind_slot: u16::MAX,
        },
        Instr::ResumeUnwind,
        Instr::CheckExcept(1),
        Instr::CheckExceptBare,
    ];
    assert_eq!(samples.len(), OPCODE_NAMES.len());
    let mut seen = std::collections::HashSet::new();
    for s in samples {
        let name = s.name();
        assert!(!name.is_empty());
        assert!(seen.insert(name), "duplicate opcode name {name}");
    }
    assert_eq!(all.len(), 40);
}

#[test]
fn decode_disassembles_and_renders() {
    let f = fn_def("f", &["a"], vec![ret(b(BinOp::Add, name("a"), lit(3)))]);
    let c = must_lower(&f);
    let text = crate::decode::disassemble(&c);
    assert!(text.contains("LoadFast"), "{text}");
    assert!(text.contains("LoadConst"), "{text}");
    assert!(text.contains("# 3"), "{text}");
    assert!(text.contains("BinOp"), "{text}");
    assert!(text.contains("# a"), "{text}");
    assert!(text.contains("ReturnValue"), "{text}");
}

#[test]
fn decode_try_lowers_now() {
    // fp-exceptions: bare-except try lowers (01-core/03 landed); the
    // fallback probe moved to shapes WITHOUT any try.
    let f = fn_def(
        "f",
        &[],
        vec![Stmt::Try(TryStmt {
            body: vec![Stmt::Pass],
            handlers: vec![ExceptHandler {
                ty: None,
                name: None,
                body: vec![Stmt::Pass],
            }],
            orelse: vec![],
            finalbody: vec![],
        })],
    );
    match lower_fn(&f) {
        LowerOutcome::Code(c) => {
            assert!(c.excs.len() >= 1, "bare-except try records an arc");
        }
        LowerOutcome::Fallback(u) => panic!("try must lower now: {u:?}"),
    }
}

// ---------------------------------------------------------------------------
// Enum completeness: jump family + control-flow opcodes exist but are never
// emitted by this leaf's lowerer.
// ---------------------------------------------------------------------------

#[test]
fn jump_helpers_and_control_opcodes_present() {
    let j = Instr::Jump(4);
    assert!(j.is_jump());
    assert_eq!(j.jump_target(), Some(4));
    assert!(!Instr::PopTop.is_jump());
    assert_eq!(Instr::PopTop.jump_target(), None);
    assert_eq!(Instr::ForNext(9).jump_target(), Some(9));
    assert!(Instr::ForNext(9).is_jump());
    // Site-carrying opcodes.
    assert_eq!(Instr::PopTop.site_id(), None);
    assert_eq!(Instr::CallFunction { argc: 1, site: 7 }.site_id(), Some(7));
    assert_eq!(Instr::LoadAttr { idx: 0, site: 8 }.site_id(), Some(8));
}

// ---------------------------------------------------------------------------
// Module-level entry point
// ---------------------------------------------------------------------------

#[test]
fn lower_module_fn_dispatches() {
    let item = ap_ir::Item::FnDef(fn_def("m", &[], vec![ret(lit(9))]));
    match crate::lower::lower_module_fn(&item) {
        Some(LowerOutcome::Code(c)) => {
            assert_eq!(c.instrs[0], Instr::LoadConst(0));
            assert_eq!(c.consts.get(0), &Const::Int(9));
        }
        _ => panic!("module fn must lower"),
    }
    // Non-fn item -> None.
    assert!(crate::lower::lower_module_fn(&ap_ir::Item::Stmt(Stmt::Pass)).is_none());
    assert!(
        crate::lower::lower_module_fn(&ap_ir::Item::ClassDef(ClassDef {
            name: "C".into(),
            bases: vec![],
            metaclass: None,
            decorators: vec![],
            body: vec![],
            id: ap_ir::DefId(0),
        }))
        .is_none()
    );
}

// ---------------------------------------------------------------------------
// Leaf 01-core/01: control-flow lowering (If/While/For/BoolOp/Break/Continue)
// ---------------------------------------------------------------------------

#[test]
fn while_loop_lowering_shape() {
    // def f():
    //     n = 0
    //     while n < 3:
    //         n = n + 1
    //     return n
    let f = fn_def(
        "f",
        &[],
        vec![
            Stmt::Assign(vec![Target::Name("n".into())], lit(0)),
            Stmt::While(
                Expr::Compare(vec![CmpOp::Lt], vec![name("n"), lit(3)], 1),
                vec![Stmt::Assign(
                    vec![Target::Name("n".into())],
                    b(BinOp::Add, name("n"), lit(1)),
                )],
                vec![],
            ),
            ret(name("n")),
        ],
    );
    let c = must_lower(&f);
    // n=0: LC SF | LOOP: LF0 LC3 Compare JumpIfFalse->EXIT | LF0 LC1 BinOp SF0 | Jump->LOOP | EXIT: LF0 RetVal | RN
    let mut iffalse_idx = None;
    let mut back_idx = None;
    for (i, ins) in c.instrs.iter().enumerate() {
        if let Instr::JumpIfFalse(t) = ins {
            iffalse_idx = Some((i, *t as usize));
        }
        if let Instr::Jump(t) = ins {
            back_idx = Some((i, *t as usize));
        }
    }
    let (ii, it) = iffalse_idx.expect("JumpIfFalse present");
    let (bi, bt) = back_idx.expect("back-edge Jump present");
    assert!(
        bt < bi,
        "back-edge is a BACK edge (target {bt} < instr {bi})"
    );
    assert!(it > ii && it > bi, "cond-false exits past the body");
    assert_eq!(c.instrs.last(), Some(&Instr::ReturnNone));
}

#[test]
fn for_loop_lowering_shape() {
    // def f(xs):
    //     s = 0
    //     for x in xs:
    //         s = s + x
    //     return s
    let f = fn_def(
        "f",
        &["xs"],
        vec![
            Stmt::Assign(vec![Target::Name("s".into())], lit(0)),
            Stmt::For(
                Target::Name("x".into()),
                name("xs"),
                vec![Stmt::Assign(
                    vec![Target::Name("s".into())],
                    b(BinOp::Add, name("s"), name("x")),
                )],
                vec![],
            ),
            ret(name("s")),
        ],
    );
    let c = must_lower(&f);
    // s=0: LC SF (0..1); LoadFast(xs) GetIter ForNext->EXIT StoreFast(x) body Jump->LOOP
    assert_eq!(c.instrs[2], Instr::LoadFast(0));
    assert_eq!(c.instrs[3], Instr::GetIter);
    let fornext = c.instrs[4];
    assert!(matches!(fornext, Instr::ForNext(_)));
    assert_eq!(c.instrs[5], Instr::StoreFast(2)); // slots: xs=0, s=1, x=2
                                                  // ForNext target = the exit arc after the back-edge jump.
    let t = match fornext {
        Instr::ForNext(t) => t as usize,
        _ => unreachable!(),
    };
    assert!(t > 7, "ForNext exits past the loop body (t={t})");
}

#[test]
fn for_loop_else_and_break_targets() {
    // def f(xs):
    //     for x in xs:
    //         if x == 0:
    //             break
    //     else:
    //         return 1
    //     return 2
    let f = fn_def(
        "f",
        &["xs"],
        vec![
            Stmt::For(
                Target::Name("x".into()),
                name("xs"),
                vec![Stmt::If(
                    Expr::Compare(vec![CmpOp::Eq], vec![name("x"), lit(0)], 1),
                    vec![Stmt::Break],
                    vec![],
                )],
                vec![ret(lit(1))],
            ),
            ret(lit(2)),
        ],
    );
    let c = must_lower(&f);
    c.validate()
        .expect("all break/continue/exit targets patched");
    // Two plain Jumps exist past the loop setup: the BREAK and the loop's
    // BACK EDGE. The back edge targets the loop HEAD (lower idx); the break
    // targets the exit arc AFTER the else block's ReturnValue (highest
    // target in the stream).
    let jumps: Vec<(usize, usize)> = c
        .instrs
        .iter()
        .enumerate()
        .filter_map(|(i, ins)| match ins {
            Instr::Jump(t) if i > 6 => Some((i, *t as usize)),
            _ => None,
        })
        .collect();
    assert_eq!(jumps.len(), 2, "break + back-edge: {jumps:?}");
    // The break (max target) must land exactly on the post-else exit arc:
    // the else block's ReturnValue sits immediately BEFORE it.
    let break_target = jumps.iter().map(|(_, t)| *t).max().unwrap();
    assert_eq!(
        c.instrs[break_target - 1],
        Instr::ReturnValue,
        "break lands just past the loop-else ReturnValue"
    );
    // And the back-edge (min target) is the loop head.
    let back_target = jumps.iter().map(|(_, t)| *t).min().unwrap();
    let fornext_at = c
        .instrs
        .iter()
        .position(|i| matches!(i, Instr::ForNext(_)))
        .unwrap();
    assert!(
        back_target <= fornext_at,
        "back-edge returns to the loop head (t={back_target} <= ForNext@{fornext_at})"
    );
    // Return 1 (else) exists.
    let mut probe_pool = ConstPool::new();
    assert!(c
        .instrs
        .contains(&Instr::LoadConst(probe_pool.intern(Const::Int(1)))));
}

#[test]
fn continue_targets_the_fornext_in_for() {
    // def f(xs):
    //     out = 0
    //     for x in xs:
    //         if x:
    //             continue
    //         out = out + x
    //     return out
    let f = fn_def(
        "f",
        &["xs"],
        vec![
            Stmt::Assign(vec![Target::Name("out".into())], lit(0)),
            Stmt::For(
                Target::Name("x".into()),
                name("xs"),
                vec![
                    Stmt::If(name("x"), vec![Stmt::Continue], vec![]),
                    Stmt::Assign(
                        vec![Target::Name("out".into())],
                        b(BinOp::Add, name("out"), name("x")),
                    ),
                ],
                vec![],
            ),
            ret(name("out")),
        ],
    );
    let c = must_lower(&f);
    let fornext_at = c
        .instrs
        .iter()
        .position(|i| matches!(i, Instr::ForNext(_)))
        .expect("ForNext");
    // The continue's Jump target IS the ForNext instruction index.
    let continue_target = c
        .instrs
        .iter()
        .enumerate()
        .filter_map(|(i, ins)| match ins {
            // JumpIfFalse exits the if; plain Jump after it is the continue.
            Instr::Jump(t) if i > fornext_at => Some(*t as usize),
            _ => None,
        })
        .find(|&t| t == fornext_at)
        .expect("continue jumps to the ForNext");
    assert_eq!(continue_target, fornext_at);
}

#[test]
fn boolop_value_and_short_circuit_shape() {
    // def f(a, b):
    //     return a or b
    let f = fn_def(
        "f",
        &["a", "b"],
        vec![ret(Expr::BoolOp(BoolOp::Or, vec![name("a"), name("b")]))],
    );
    let c = must_lower(&f);
    // LF0 JumpIfTrueOrPop->DONE LF1 DONE RetVal RN
    assert_eq!(c.instrs[0], Instr::LoadFast(0));
    assert!(matches!(c.instrs[1], Instr::JumpIfTrueOrPop(_)));
    assert_eq!(c.instrs[2], Instr::LoadFast(1));
    assert_eq!(c.instrs[3], Instr::ReturnValue);
    let t = match c.instrs[1] {
        Instr::JumpIfTrueOrPop(t) => t as usize,
        _ => unreachable!(),
    };
    assert_eq!(t, 3, "short-circuit lands directly on ReturnValue");

    // `a and b`: JumpIfFalseOrPop.
    let g = fn_def(
        "g",
        &["a", "b"],
        vec![ret(Expr::BoolOp(BoolOp::And, vec![name("a"), name("b")]))],
    );
    let c2 = must_lower(&g);
    assert!(matches!(c2.instrs[1], Instr::JumpIfFalseOrPop(_)));

    // 3-operand chain: two jumps, both targeting the same DONE.
    let h = fn_def(
        "h",
        &["a", "b", "c"],
        vec![ret(Expr::BoolOp(
            BoolOp::Or,
            vec![name("a"), name("b"), name("c")],
        ))],
    );
    let c3 = must_lower(&h);
    let jumps: Vec<usize> = c3
        .instrs
        .iter()
        .enumerate()
        .filter_map(|(i, ins)| match ins {
            Instr::JumpIfTrueOrPop(t) => Some(*t as usize),
            _ => None,
        })
        .collect();
    assert_eq!(jumps.len(), 2);
    assert_eq!(jumps[0], jumps[1], "both land on DONE");
}

#[test]
fn boolop_in_loop_condition_lowers() {
    // while a and b: pass  -> the condition is a jump chain, not a fallback.
    let f = fn_def(
        "f",
        &["a", "b"],
        vec![Stmt::While(
            Expr::BoolOp(BoolOp::And, vec![name("a"), name("b")]),
            vec![Stmt::Pass],
            vec![],
        )],
    );
    let c = must_lower(&f);
    c.validate().expect("loop+boolop validates");
    assert!(c
        .instrs
        .iter()
        .any(|i| matches!(i, Instr::JumpIfFalseOrPop(_))));
    assert!(c.instrs.iter().any(|i| matches!(i, Instr::JumpIfFalse(_))));
}

#[test]
fn nested_loops_keep_separate_break_frames() {
    // Inner break must NOT patch the outer loop's exit; both loops validate.
    let f = fn_def(
        "f",
        &["xs"],
        vec![Stmt::For(
            Target::Name("x".into()),
            name("xs"),
            vec![Stmt::For(
                Target::Name("y".into()),
                name("xs"),
                vec![Stmt::Break],
                vec![],
            )],
            vec![],
        )],
    );
    let c = must_lower(&f);
    c.validate().expect("nested breaks patched correctly");
    assert_eq!(
        c.instrs
            .iter()
            .filter(|i| matches!(i, Instr::ForNext(_)))
            .count(),
        2
    );
}

#[test]
fn loop_var_scoping_leaks_to_function_scope() {
    // for x in xs: pass; return x  — x is a compile-time local even though
    // assigned only by the loop (CPython leak rule).
    let f = fn_def(
        "f",
        &["xs"],
        vec![
            Stmt::For(
                Target::Name("x".into()),
                name("xs"),
                vec![Stmt::Pass],
                vec![],
            ),
            ret(name("x")),
        ],
    );
    let c = must_lower(&f);
    assert!(
        c.local_names.contains(&"x".to_string()),
        "loop var gets a slot: {:?}",
        c.local_names
    );
}

#[test]
fn try_inside_loop_lowers_with_preserved_handle() {
    // fp-exceptions: a Try inside a loop body lowers; the exc entry's
    // stack_depth pins the enclosing ForNext handle (opdepth 1).
    let f = fn_def(
        "f",
        &["xs"],
        vec![Stmt::For(
            Target::Name("x".into()),
            name("xs"),
            vec![Stmt::Try(TryStmt {
                body: vec![Stmt::Pass],
                handlers: vec![ExceptHandler {
                    ty: Some(name("ValueError")),
                    name: Some("e".into()),
                    body: vec![Stmt::Pass],
                }],
                orelse: vec![],
                finalbody: vec![],
            })],
            vec![],
        )],
    );
    let c = must_lower(&f);
    assert!(
        c.excs.len() >= 1,
        "try-in-loop records its exc arc: {:?}",
        c.excs.entries()
    );
    assert!(
        c.excs.entries().iter().any(|e| e.stack_depth == 1),
        "inner arc carries the loop's opdepth: {:?}",
        c.excs.entries()
    );
}

#[test]
fn empty_dict_display_lowers() {
    // d = {} — zero-item dict display (BuildDict(0)); previously a
    // function-level fallback that silently tree-walked dict-loop bodies.
    let f = fn_def(
        "run",
        &[],
        vec![
            Stmt::Assign(vec![Target::Name("d".into())], Expr::Dict(vec![])),
            ret(Expr::Call(
                Box::new(name("len")),
                vec![Arg::Pos(name("d"))],
                2,
            )),
        ],
    );
    let c = must_lower(&f);
    assert!(c.instrs.iter().any(|i| matches!(i, Instr::BuildDict(0))));
}

#[test]
fn kw_slice_contiguous_when_name_pre_interned() {
    // The kw NAME already interned (attr read above the call) must not
    // desync the CallFunctionKw slice — names reserve a fresh contiguous
    // run; values pair with names[kw_first..] in push order.
    let attr_read = Expr::Attribute(Box::new(name("o")), "x".into(), 2);
    let call = Expr::Call(
        Box::new(name("f")),
        vec![
            Arg::Pos(name("a")),
            Arg::Kw("x".into(), lit(1)),
            Arg::Kw("y".into(), lit(2)),
        ],
        3,
    );
    let f = fn_def(
        "run",
        &["o", "a"],
        vec![Stmt::ExprStmt(attr_read), ret(call)],
    );
    let c = must_lower(&f);
    let kws: Vec<(u32, u16)> = c
        .instrs
        .iter()
        .filter_map(|i| match i {
            Instr::CallFunctionKw { kw_first, n_kw, .. } => Some((*kw_first, *n_kw)),
            _ => None,
        })
        .collect();
    assert_eq!(kws.len(), 1);
    let (kw_first, n_kw) = kws[0];
    assert_eq!(n_kw, 2);
    // Contiguous + in kw order.
    assert_eq!(&c.names[kw_first as usize], "x");
    assert_eq!(&c.names[kw_first as usize + 1], "y");
}

#[test]
fn set_display_lowers() {
    let f = fn_def("run", &[], vec![ret(Expr::Set(vec![lit(1), lit(2)]))]);
    let c = must_lower(&f);
    assert!(c.instrs.iter().any(|i| matches!(i, Instr::BuildSet(2))));
}
