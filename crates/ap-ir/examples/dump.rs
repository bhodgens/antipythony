//! Build a sample module exercising every major IR node kind and print the
//! IR dump. Used as a manual smoke check and by the 01.02 gate:
//! `cargo run -p ap-ir --example dump`.

use ap_ir::{
    Arg, BinOp, BoolOp, Builder, CmpOp, Conversion, Item, ParamKind, Pattern, TypeLattice, UnaryOp,
};

fn main() {
    let mut b = Builder::new();

    // x = 1 + 2
    let assign = b.assign(
        vec![b.target_name("x")],
        b.bin_op(BinOp::Add, b.int_lit(1), b.int_lit(2)),
    );

    // def greet(name: str, punct="!") -> None:
    //     print(f"hello {name!r}{punct}")
    let name_param = Builder::with_ann(
        b.param("name", ParamKind::PosOrKw),
        b.ann(b.name("str"), TypeLattice::Str),
    );
    let punct_param = Builder::with_default(b.param("punct", ParamKind::PosOrKw), b.str_lit("!"));
    let greet = b.fn_def(
        "greet",
        vec![name_param, punct_param],
        Some(b.ann(b.name("None"), TypeLattice::None)),
        vec![],
        ap_ir::FnBody {
            blocks: vec![b.expr_stmt(b.call(
                b.name("print"),
                vec![b.arg_pos(b.fstring(vec![
                    b.fstr_lit("hello "),
                    b.fstr_interp(b.name("name"), Some(Conversion::R), None),
                    b.fstr_interp(b.name("punct"), None, None),
                ]))],
            ))],
        },
    );

    // def gen(n):
    //     for i in range(n):
    //         yield i * 2
    let gen = b.fn_def(
        "gen",
        vec![b.param("n", ParamKind::PosOrKw)],
        None,
        vec![],
        ap_ir::FnBody {
            blocks: vec![b.for_(
                b.target_name("i"),
                b.call(b.name("range"), vec![b.arg_pos(b.name("n"))]),
                vec![b.expr_stmt(b.yield_(Some(b.bin_op(BinOp::Mul, b.name("i"), b.int_lit(2)))))],
                vec![],
            )],
        },
    );

    // class Point(Base):
    //     y: int = 0
    let point = {
        let body = vec![
            b.ann_assign(
                b.target_name("y"),
                b.ann(b.name("int"), TypeLattice::Int),
                Some(b.int_lit(0)),
            ),
            b.if_(
                b.compare(vec![CmpOp::Gt], vec![b.name("y"), b.int_lit(10)]),
                vec![b.raise_(
                    Some(b.call(
                        b.name("ValueError"),
                        vec![b.arg_kw("msg", b.str_lit("too big"))],
                    )),
                    None,
                )],
                vec![],
            ),
        ];
        let class = b.class_def("Point", vec![b.name("Base")], None, vec![], body);
        Item::ClassDef(class)
    };

    // if a and b: xs[1:2] = (3,)
    // else: total += n
    let ctrl = b.if_(
        b.bool_op(BoolOp::And, vec![b.name("a"), b.name("b")]),
        vec![b.assign(
            vec![b.target_subscript(
                b.name("xs"),
                b.slice(Some(b.int_lit(1)), Some(b.int_lit(2)), None),
            )],
            b.tuple(vec![b.int_lit(3)]),
        )],
        vec![b.aug_assign(b.target_name("total"), BinOp::Add, b.name("n"))],
    );

    // try: del d["k"]
    // except KeyError as e: raise RuntimeError("lost") from e
    // finally: pass
    let guarded = b.try_(
        vec![b.del_(vec![b.target_subscript(b.name("d"), b.str_lit("k"))])],
        vec![b.except_handler(
            Some(b.name("KeyError")),
            Some("e".into()),
            vec![b.raise_(
                Some(b.call(b.name("RuntimeError"), vec![b.arg_pos(b.str_lit("lost"))])),
                Some(b.name("e")),
            )],
        )],
        vec![],
        vec![b.pass_()],
    );

    // match pair: case (0, rest): ...
    let matched = b.match_(
        b.name("pair"),
        vec![ap_ir::MatchCase {
            pattern: Pattern::Sequence(
                vec![Pattern::Literal(b.int_lit(0))],
                Some(Box::new(Pattern::Capture("rest".into()))),
            ),
            guard: None,
            body: vec![b.expr_stmt(b.unary_op(UnaryOp::Not, b.name("rest")))],
        }],
    );

    // module-level: import os.path as p; from sys import argv; [*xs] = {x for x in y}
    let imports = vec![
        b.import_(vec![b.import_alias("os.path", Some("p"))]),
        b.import_from("sys", vec![b.import_alias("argv", None::<String>)], false),
        b.assign(
            vec![b.target_tuple(vec![b.target_starred(b.name("head"))])],
            b.set_comp(
                b.name("x"),
                vec![ap_ir::expr::CompClause {
                    target: b.name("x"),
                    iter: b.name("y"),
                    ifs: vec![],
                }],
            ),
        ),
        b.assert_(b.bool_lit(true), Some(b.str_lit("always"))),
        b.expr_stmt(b.yield_from(b.name("g"))),
    ];
    let _ = Item::Stmt(b.global_(vec!["g"])); // smoke: stmt-level item construction
    let _ = Arg::Pos(b.none_lit()); // keep Arg import exercised outside builder

    let module = b.module(vec![
        Item::Stmt(assign),
        Item::FnDef(greet),
        Item::FnDef(gen),
        point,
        Item::Stmt(ctrl),
        Item::Stmt(guarded),
        Item::Stmt(matched),
    ]);
    for i in imports {
        let _ = i; // construction smoke; not added to keep dump focused
    }

    print!("{}", ap_ir::dump_module(&module));
}
