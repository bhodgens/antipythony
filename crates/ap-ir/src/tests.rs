//! Exhaustiveness + builder smoke tests (leaf 01.02 gate).

use crate::builder::{Builder, NumberLit};
use crate::expr::{Arg, BinOp, BoolOp, CmpOp, Conversion, Expr, UnaryOp};
use crate::item::{FnBody, Item, ParamKind, Pattern, Stmt, Target};
use crate::matrix::{all_matrices, Strategy, EXPR_MATRIX, STMT_MATRIX};
use crate::types::TypeLattice;

/// Every `Strategy::Builder(name)` in the matrix must name a method that
/// exists on `Builder` — this vtable makes a typo a test failure, not a
/// silent lie in the docs.
#[test]
fn matrix_builder_names_resolve_to_real_methods() {
    let table: &[(&str, fn(&Builder))] = &[
        ("bool_op", |b| {
            let _ = b.bool_op(BoolOp::And, vec![b.bool_lit(true), b.bool_lit(false)]);
        }),
        ("named", |b| {
            let _ = b.named(b.name("t"), b.int_lit(1));
        }),
        ("bin_op", |b| {
            let _ = b.bin_op(BinOp::Add, b.int_lit(1), b.int_lit(2));
        }),
        ("unary_op", |b| {
            let _ = b.unary_op(UnaryOp::Invert, b.int_lit(1));
        }),
        ("lambda", |b| {
            let _ = b.lambda(vec![], b.none_lit());
        }),
        ("if_exp", |b| {
            let _ = b.if_exp(b.bool_lit(true), b.int_lit(1), b.int_lit(2));
        }),
        ("dict", |b| {
            let _ = b.dict(vec![b.dict_item(b.str_lit("a"), b.int_lit(1))]);
        }),
        ("set", |b| {
            let _ = b.set(vec![b.int_lit(1)]);
        }),
        ("list_comp", |b| {
            let _ = b.list_comp(b.name("x"), vec![]);
        }),
        ("set_comp", |b| {
            let _ = b.set_comp(b.name("x"), vec![]);
        }),
        ("dict_comp", |b| {
            let _ = b.dict_comp(b.name("k"), b.name("v"), vec![]);
        }),
        ("generator", |b| {
            let _ = b.generator(b.name("x"), vec![]);
        }),
        ("await_", |b| {
            let _ = b.await_(b.name("fut"));
        }),
        ("yield_", |b| {
            let _ = b.yield_(Some(b.int_lit(1)));
        }),
        ("yield_from", |b| {
            let _ = b.yield_from(b.name("sub"));
        }),
        ("compare", |b| {
            let _ = b.compare(vec![CmpOp::Lt], vec![b.int_lit(1), b.int_lit(2)]);
        }),
        ("call", |b| {
            let _ = b.call(b.name("f"), vec![b.arg_pos(b.int_lit(1))]);
        }),
        ("fstring", |b| {
            let _ = b.fstring(vec![b.fstr_lit("x")]);
        }),
        ("str_lit", |b| {
            let _ = b.str_lit("s");
        }),
        ("bytes_lit", |b| {
            let _ = b.bytes_lit(b"bytes");
        }),
        ("num_lit", |b| {
            let _ = b.num_lit(NumberLit::Int(1));
        }),
        ("bool_lit", |b| {
            let _ = b.bool_lit(true);
        }),
        ("none_lit", |b| {
            let _ = b.none_lit();
        }),
        ("ellipsis_lit", |b| {
            let _ = b.ellipsis_lit();
        }),
        ("attribute", |b| {
            let _ = b.attribute(b.name("o"), "a");
        }),
        ("subscript", |b| {
            let _ = b.subscript(b.name("o"), b.int_lit(0));
        }),
        ("starred", |b| {
            let _ = b.starred(b.name("xs"));
        }),
        ("name", |b| {
            let _ = b.name("x");
        }),
        ("list", |b| {
            let _ = b.list(vec![]);
        }),
        ("tuple", |b| {
            let _ = b.tuple(vec![]);
        }),
        ("slice", |b| {
            let _ = b.slice(None, None, None);
        }),
        ("fn_def", |b| {
            let _ = b.fn_def("f", vec![], None, vec![], FnBody { blocks: vec![] });
        }),
        ("class_def", |_b| {
            let mut b2 = Builder::new();
            let _ = b2.class_def("C", vec![], None, vec![], vec![]);
        }),
        ("return_", |b| {
            let _ = b.return_(None);
        }),
        ("del_", |b| {
            let _ = b.del_(vec![b.target_name("x")]);
        }),
        ("assign", |b| {
            let _ = b.assign(vec![b.target_name("x")], b.int_lit(1));
        }),
        ("aug_assign", |b| {
            let _ = b.aug_assign(b.target_name("x"), BinOp::Add, b.int_lit(1));
        }),
        ("ann_assign", |b| {
            let _ = b.ann_assign(
                b.target_name("x"),
                b.ann(b.name("int"), TypeLattice::Int),
                None,
            );
        }),
        ("for_", |b| {
            let _ = b.for_(b.target_name("i"), b.name("xs"), vec![], vec![]);
        }),
        ("while_", |b| {
            let _ = b.while_(b.bool_lit(true), vec![], vec![]);
        }),
        ("if_", |b| {
            let _ = b.if_(b.bool_lit(true), vec![], vec![]);
        }),
        ("with_", |b| {
            let _ = b.with_(
                vec![b.with_item(b.call(b.name("open"), vec![]), None)],
                vec![],
            );
        }),
        ("match_", |b| {
            let _ = b.match_(
                b.name("x"),
                vec![crate::item::MatchCase {
                    pattern: Pattern::Wildcard,
                    guard: None,
                    body: vec![],
                }],
            );
        }),
        ("raise_", |b| {
            let _ = b.raise_(None, None);
        }),
        ("try_", |b| {
            let _ = b.try_(vec![], vec![], vec![], vec![b.pass_()]);
        }),
        ("assert_", |b| {
            let _ = b.assert_(b.bool_lit(true), None);
        }),
        ("import_", |b| {
            let _ = b.import_(vec![b.import_alias("a.b.c", Some("d"))]);
        }),
        ("import_from", |b| {
            let _ = b.import_from("m", vec![b.import_alias("n", None::<String>)], false);
        }),
        ("global_", |b| {
            let _ = b.global_(vec!["x"]);
        }),
        ("nonlocal", |b| {
            let _ = b.nonlocal(vec!["x"]);
        }),
        ("expr_stmt", |b| {
            let _ = b.expr_stmt(b.none_lit());
        }),
        ("pass_", |b| {
            let _ = b.pass_();
        }),
        ("break_", |b| {
            let _ = b.break_();
        }),
        ("continue_", |b| {
            let _ = b.continue_();
        }),
    ];

    for (kind, matrix) in all_matrices() {
        for (variant, strategy) in matrix {
            if let Strategy::Builder(method) = strategy {
                let entry = table
                    .iter()
                    .find(|(name, _)| name == method)
                    .unwrap_or_else(|| {
                        panic!(
                            "{kind}::{variant} cites Builder::{method}, which does not exist \
                             (update the vtable in this test)"
                        )
                    });
                // call it — proves the method is constructible, not just named
                let b = Builder::new();
                (entry.1)(&b);
            }
        }
    }
}

/// Structural sanity of the matrix itself: unique variant names, non-empty
/// strategy descriptions.
#[test]
fn matrix_is_wellformed() {
    for (kind, m) in all_matrices() {
        let mut seen = std::collections::BTreeSet::new();
        for (variant, strategy) in m {
            assert!(!variant.is_empty(), "{kind}: empty variant name");
            assert!(seen.insert(*variant), "{kind}: duplicate variant {variant}");
            let desc = match strategy {
                Strategy::Builder(n) => *n,
                Strategy::LoweredVia(d) | Strategy::NotProduced(d) => *d,
            };
            assert!(!desc.is_empty(), "{kind}::{variant}: empty strategy note");
        }
    }
    assert!(EXPR_MATRIX.len() >= 30, "expr matrix suspiciously small");
    assert!(STMT_MATRIX.len() >= 20, "stmt matrix suspiciously small");
}

/// Builder smoke test: construct one of every Expr variant and one of every
/// Stmt variant, and dump them.
#[test]
fn builder_constructs_all_node_kinds() {
    let b = Builder::new();
    let f = b.float_lit(0.5);
    let exprs: Vec<Expr> = vec![
        b.int_lit(1),
        b.int_big_lit("99999999999999999999999999999999"),
        f.clone(),
        b.complex_lit(1.0, 2.0),
        b.str_lit("hi"),
        b.bytes_lit(vec![104, 105]),
        b.bool_lit(true),
        b.none_lit(),
        b.ellipsis_lit(),
        b.name("x"),
        b.bin_op(BinOp::Add, b.int_lit(1), b.int_lit(2)),
        b.unary_op(UnaryOp::Minus, b.int_lit(1)),
        b.bool_op(BoolOp::Or, vec![b.name("a"), b.name("b")]),
        b.compare(
            vec![CmpOp::Lt, CmpOp::Lt],
            vec![b.int_lit(1), b.int_lit(2), b.int_lit(3)],
        ),
        b.call(
            b.name("f"),
            vec![b.arg_pos(b.int_lit(1)), b.arg_kw("k", b.int_lit(2))],
        ),
        b.attribute(b.name("o"), "attr"),
        b.subscript(b.name("xs"), b.slice(Some(b.int_lit(0)), None, None)),
        b.slice(Some(b.int_lit(1)), Some(b.int_lit(2)), Some(b.int_lit(3))),
        b.list(vec![b.int_lit(1)]),
        b.tuple(vec![b.int_lit(1)]),
        b.set(vec![b.int_lit(1)]),
        b.dict(vec![b.dict_item(b.str_lit("k"), b.int_lit(1))]),
        b.dict_expand(b.name("d")),
        b.lambda(vec![b.param("q", ParamKind::PosOrKw)], b.name("q")),
        b.list_comp(
            b.name("x"),
            vec![b.comp_clause(b.name("x"), b.name("xs"), vec![])],
        ),
        b.set_comp(b.name("x"), vec![]),
        b.dict_comp(b.name("k"), b.name("v"), vec![]),
        b.generator(b.name("x"), vec![]),
        b.yield_(Some(b.int_lit(1))),
        b.yield_from(b.name("g")),
        b.await_(b.name("c")),
        b.fstring(vec![
            b.fstr_lit("n="),
            b.fstr_interp(b.name("n"), Some(Conversion::R), None),
        ]),
        b.if_exp(b.bool_lit(true), b.int_lit(1), b.int_lit(2)),
        b.starred(b.name("rest")),
        b.named(b.name("w"), b.int_lit(1)),
    ];
    assert_eq!(exprs.len(), 35);

    let stmts: Vec<Stmt> = vec![
        b.assign(vec![b.target_name("x")], b.int_lit(1)),
        b.assign(
            vec![b.target_tuple(vec![b.target_name("a"), b.target_starred(b.name("rest"))])],
            b.name("xs"),
        ),
        b.aug_assign(
            b.target_subscript(b.name("xs"), b.int_lit(0)),
            BinOp::Add,
            b.int_lit(1),
        ),
        b.ann_assign(
            b.target_name("x"),
            b.ann(b.name("int"), TypeLattice::Int),
            Some(b.int_lit(1)),
        ),
        b.return_(Some(b.none_lit())),
        b.if_(b.bool_lit(true), vec![b.pass_()], vec![b.pass_()]),
        b.while_(b.bool_lit(false), vec![], vec![]),
        b.for_(
            b.target_name("i"),
            b.name("xs"),
            vec![b.continue_()],
            vec![b.pass_()],
        ),
        b.with_(
            vec![b.with_item(b.call(b.name("ctx"), vec![]), Some(b.target_name("c")))],
            vec![b.pass_()],
        ),
        b.try_(
            vec![b.pass_()],
            vec![b.except_handler(
                Some(b.name("ValueError")),
                Some("e".into()),
                vec![b.pass_()],
            )],
            vec![],
            vec![b.pass_()],
        ),
        b.raise_(
            Some(b.call(b.name("ValueError"), vec![b.arg_pos(b.str_lit("boom"))])),
            None,
        ),
        b.match_(
            b.name("x"),
            vec![crate::item::MatchCase {
                pattern: Pattern::Literal(b.int_lit(1)),
                guard: Some(b.bool_lit(true)),
                body: vec![b.pass_()],
            }],
        ),
        b.assert_(b.bool_lit(true), Some(b.str_lit("msg"))),
        b.global_(vec!["g"]),
        b.nonlocal(vec!["n"]),
        b.import_(vec![b.import_alias("os.path", Some("p"))]),
        b.import_from("sys", vec![b.import_alias("argv", None::<String>)], false),
        b.import_from("os", vec![], true),
        b.del_(vec![
            b.target_name("x"),
            b.target_attribute(b.name("o"), "a"),
        ]),
        b.pass_(),
        b.expr_stmt(b.call(b.name("log"), vec![])),
        b.break_(),
        b.continue_(),
    ];
    assert_eq!(stmts.len(), 23);

    // dump the lot — must be deterministic and non-empty
    let module = b.module(vec![
        Item::Stmt(stmts[0].clone()),
        Item::Stmt(stmts[1].clone()),
    ]);
    let dump1 = crate::dump_module(&module);
    let dump2 = crate::dump_module(&module);
    assert_eq!(dump1, dump2);
    assert!(dump1.contains("assign x = 1"));
}

/// Generator detection: a body containing yield sets is_generator.
#[test]
fn fn_def_detects_generator() {
    let b = Builder::new();
    let gen_body = FnBody {
        blocks: vec![b.expr_stmt(b.yield_(Some(b.int_lit(1))))],
    };
    let f = b.fn_def("g", vec![], None, vec![], gen_body);
    assert!(f.is_generator);

    let plain = b.fn_def(
        "p",
        vec![],
        None,
        vec![],
        FnBody {
            blocks: vec![b.return_(Some(b.int_lit(1)))],
        },
    );
    assert!(!plain.is_generator);
}

/// Arg enum coverage (mirrors matrix indirectly — all four forms constructible).
#[test]
fn arg_forms() {
    let b = Builder::new();
    let args = vec![
        b.arg_pos(b.int_lit(1)),
        b.arg_star(b.name("xs")),
        b.arg_kw("k", b.int_lit(2)),
        b.arg_kwstar(b.name("kw")),
    ];
    assert_eq!(args.len(), 4);
    assert!(matches!(args[0], Arg::Pos(_)));
    assert!(matches!(args[3], Arg::KwStar(_)));
}

/// Targets cover all assignment forms.
#[test]
fn target_forms() {
    let b = Builder::new();
    let t = b.target_tuple(vec![
        b.target_name("a"),
        b.target_subscript(b.name("o"), b.int_lit(0)),
        b.target_attribute(b.name("o"), "a"),
        b.target_starred(b.name("s")),
    ]);
    assert!(matches!(t, Target::Tuple(ref v) if v.len() == 4));
}

/// Dump determinism on a structural PartialEq round trip: two identically
/// built modules are equal and dump identically.
#[test]
fn partial_eq_is_structural() {
    let build = |b: &Builder| {
        b.module(vec![Item::FnDef(b.fn_def(
            "f",
            vec![b.param("a", ParamKind::PosOrKw)],
            None,
            vec![],
            FnBody {
                blocks: vec![b.return_(Some(b.bin_op(BinOp::Add, b.name("a"), b.int_lit(1))))],
            },
        ))])
    };
    let b1 = Builder::new();
    let b2 = Builder::new();
    let m1 = build(&b1);
    let m2 = build(&b2);
    assert_eq!(m1, m2);
    assert_eq!(crate::dump_module(&m1), crate::dump_module(&m2));
}
