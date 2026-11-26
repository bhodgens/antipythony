//! Lowerer round-trip tests: one per supported/unsupported `Expr` variant,
//! plus slot assignment, const-pool dedup, line-table sparsity/monotonicity,
//! site ids, decode, and function-granularity fallback.
//!
//! IR trees are built directly (no ap-parse dev-dep needed — keeps this
//! crate's test surface hermetic).

use crate::code::CodeObject;
use crate::instr::{BinKind, CompareOp, Instr, UnaryKind, OPCODE_NAMES};
use crate::linetable::LineTable;
use crate::lower::{lower_fn, lowers_cleanly, LowerOutcome};
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
    for args in [
        vec![Arg::Kw("x".into(), lit(1))],
        vec![Arg::Star(lit(1))],
        vec![Arg::KwStar(lit(1))],
    ] {
        let call = Expr::Call(Box::new(name("f")), args, 1);
        let f = fn_def("f", &[], vec![ret(call)]);
        assert!(
            !lowers_cleanly(&f),
            "kw/star call shapes must fall back this leaf"
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
    // x=0: LC0 SF0 | x+=1: LF0 LC1 BinOp SF0 | y: LC2 SF1 | ret: LF0 RetVal | RN
    assert_eq!(c.instrs[4], Instr::BinOp(BinKind::Add));
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
        ("boolop", Expr::BoolOp(BoolOp::And, vec![lit(1), lit(2)])),
        (
            "chained comparison",
            Expr::Compare(vec![CmpOp::Lt, CmpOp::Lt], vec![lit(1), lit(2), lit(3)], 1),
        ),
        ("slice expr", Expr::Slice(None, None, None)),
        ("set display", Expr::Set(vec![lit(1)])),
        (
            "dict display",
            Expr::Dict(vec![DictItem {
                key: lit(1),
                value: lit(2),
            }]),
        ),
        ("dict expand", Expr::DictExpand(Box::new(name("d")))),
        (
            "lambda",
            Expr::Lambda(Box::new(LambdaSig { params: vec![] }), Box::new(lit(1))),
        ),
        (
            "listcomp",
            Expr::ListComp(
                Box::new(name("x")),
                vec![CompClause {
                    target: name("x"),
                    iter: name("xs"),
                    ifs: vec![],
                }],
            ),
        ),
        (
            "setcomp",
            Expr::SetComp(
                Box::new(name("x")),
                vec![CompClause {
                    target: name("x"),
                    iter: name("xs"),
                    ifs: vec![],
                }],
            ),
        ),
        (
            "dictcomp",
            Expr::DictComp(
                Box::new(name("k")),
                Box::new(name("v")),
                vec![CompClause {
                    target: name("k"),
                    iter: name("ks"),
                    ifs: vec![],
                }],
            ),
        ),
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
    // for x in xs: pass   (enum carries ForNext; lowerer rejects)
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
    assert_eq!(must_fallback(&f), "for statement");

    // try/except
    let t = fn_def(
        "t",
        &[],
        vec![Stmt::Try(TryStmt {
            body: vec![Stmt::Pass],
            handlers: vec![ExceptHandler {
                ty: Some(name("E")),
                name: Some("e".into()),
                body: vec![Stmt::Pass],
            }],
            orelse: vec![],
            finalbody: vec![],
        })],
    );
    assert_eq!(must_fallback(&t), "try statement");
}

#[test]
fn other_unsupported_statements_fall_back() {
    let mk = |body: Vec<Stmt>| fn_def("f", &[], body);
    let cases: Vec<(&str, Vec<Stmt>)> = vec![
        (
            "if statement",
            vec![Stmt::If(lit(1), vec![Stmt::Pass], vec![])],
        ),
        (
            "while statement",
            vec![Stmt::While(lit(1), vec![Stmt::Pass], vec![])],
        ),
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
        ("raise statement", vec![Stmt::Raise(Some(lit(1)), None)]),
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
        (
            "attribute store",
            vec![Stmt::Assign(
                vec![Target::Attribute(Box::new(name("o")), "p".into())],
                lit(1),
            )],
        ),
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

#[test]
fn vararg_and_default_params_fall_back() {
    let mut f = fn_def("f", &["a"], vec![ret(name("a"))]);
    f.params[0].kind = ParamKind::VarArg;
    assert_eq!(must_fallback(&f), "non-positional param");

    let mut d = fn_def("d", &["a"], vec![ret(name("a"))]);
    d.params[0].default = Some(lit(1));
    assert_eq!(must_fallback(&d), "param default");
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
        Instr::FallbackToTreeWalk,
    ];
    assert_eq!(samples.len(), OPCODE_NAMES.len());
    let mut seen = std::collections::HashSet::new();
    for s in samples {
        let name = s.name();
        assert!(!name.is_empty());
        assert!(seen.insert(name), "duplicate opcode name {name}");
    }
    assert_eq!(all.len(), 31);
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
fn decode_fallback_shape() {
    let f = fn_def(
        "f",
        &[],
        vec![Stmt::For(
            Target::Name("x".into()),
            name("xs"),
            vec![Stmt::Pass],
            vec![],
        )],
    );
    match lower_fn(&f) {
        LowerOutcome::Fallback(_) => {}
        LowerOutcome::Code(_) => panic!("For must not lower this leaf"),
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
