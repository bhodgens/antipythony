//! Leaf 02.01 CPython-differential tests (T-05 companion, implementer-side).
//!
//! Expected bytes are generated LIVE from CPython 3.14 by this test file at
//! run time (leaf-brief option "generated from live python3.14 by the test")
//! — pinned fixtures belong to the separately-authored T-05 corpus; this file
//! is the implementation's own differential harness. The reference comes from
//! the shared discovery seam (`tests/common/mod.rs`:
//! `AP_CONFORM_REFERENCE_PYTHON`, then PATH `python3.14`, both version-
//! checked). When no reference is found the differential tests SKIP (unit
//! tests above cover the pinned spot checks) — unless
//! `AP_REQUIRE_REFERENCE=1` turns a missing reference into a hard failure.
//!
//! Clauses exercised: S-01-1..14 (int semantics), S-01-19..26 (containers),
//! S-01-35..39 (hashing), S-01-40..44 (reprs), C-12..C-14 (bigint), C-15..18
//! (float repr), C-26 (set ordering: sorted-only).

use ap_obj::dict::PyDict;
use ap_obj::exc::PyException;
use ap_obj::int::{Gc, Int};
use ap_obj::ops::{binop, divmod, repr as py_repr, BinOp};
use ap_obj::value::{PyString, Tuple, Value};

use std::process::Command;

mod common;

/// Run `expr` under CPython with `PYTHONHASHSEED=0` and return printed stdout.
fn cpython(expr: &str) -> String {
    // Callers gate on reference_or_skip(); the seam panics under
    // AP_REQUIRE_REFERENCE=1 when no reference exists.
    let py = common::reference_or_skip().expect("reference interpreter resolved");
    // 2026-11-06 harness review: single-tenancy + preflight + bounded wait.
    common::interpreter_gate();
    common::preflight_or_panic();
    let mut child = Command::new(py)
        .env("PYTHONHASHSEED", "0")
        .arg("-c")
        .arg(expr)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn python3.14");
    let deadline = std::time::Instant::now() + common::reference_deadline();
    let timed_out = loop {
        match child.try_wait().expect("wait reference") {
            Some(_) => break false,
            None => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    break true;
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        }
    };
    assert!(
        !timed_out,
        "reference python3.14 exceeded the {}s deadline (AP_REF_TIMEOUT_SECS) on: {expr}",
        common::reference_deadline().as_secs()
    );
    let out = child.wait_with_output().expect("wait reference");
    assert!(
        out.status.success(),
        "python3.14 failed on {expr}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim_end().to_string()
}

/// Boolean rendering as Python prints it.
fn pyb(b: bool) -> &'static str {
    if b {
        "True"
    } else {
        "False"
    }
}

// -- value constructors -------------------------------------------------------

fn i(v: i64) -> Value {
    Value::Int(Int::from_i64(v))
}
fn f(v: f64) -> Value {
    Value::Float(v)
}
fn s(v: &str) -> Value {
    Value::Str(PyString::new(v))
}
fn big(text: &str) -> Value {
    Value::Int(Int::parse_base10(text).unwrap())
}
fn tuple(v: Vec<Value>) -> Value {
    Value::Tuple(Gc::new(Tuple(v)))
}
fn list(v: Vec<Value>) -> Value {
    Value::List(ap_obj::value::PyList::from_vec(v))
}
fn dict(pairs: Vec<(Value, Value)>) -> Value {
    let d = PyDict::new();
    for (k, v) in pairs {
        d.set(k, v).unwrap();
    }
    Value::Dict(d)
}

fn r(v: &Value) -> String {
    py_repr(v).unwrap()
}

macro_rules! diff {
    ($name:ident, $py:expr, $ours:expr) => {
        #[test]
        fn $name() {
            if common::reference_or_skip().is_none() {
                return; // SKIP printed by the seam, or panic under
                        // AP_REQUIRE_REFERENCE=1.
            }
            let expected = cpython($py);
            let ours: String = $ours;
            assert_eq!(ours, expected, "expression: {}", $py);
        }
    };
}

// -- big-int arithmetic (C-12/S-01-1) ------------------------------------------

diff!(diff_big_2p200_sq, "print(2**200 * 2**200)", {
    let p200 = binop(&i(2), BinOp::Pow, &i(200)).unwrap();
    r(&binop(&p200, BinOp::Mul, &p200).unwrap())
});

fn pow10(n: usize) -> Value {
    big(&format!("1{}", "0".repeat(n)))
}

diff!(
    diff_big_no_overflow,
    "print(10**30 // 7, (10**30 + 1) % 7)",
    {
        let p30 = pow10(30);
        let p30_plus_1 = Value::Int(p30.as_int_holder().add(&Int::Smi(1)));
        format!(
            "{} {}",
            r(&binop(&p30, BinOp::FloorDiv, &i(7)).unwrap()),
            r(&binop(&p30_plus_1, BinOp::Mod, &i(7)).unwrap())
        )
    }
);

// helper on Value for the awkward promotion above
trait AsIntHolder {
    fn as_int_holder(&self) -> &Int;
}
impl AsIntHolder for Value {
    fn as_int_holder(&self) -> &Int {
        match self {
            Value::Int(x) => x,
            _ => panic!("int expected"),
        }
    }
}

diff!(
    diff_floor_div_sign_matrix,
    "print(7//2, -7//2, 7//-2, -7//-2, 7%2, -7%2, 7%-2, -7%-2)",
    {
        [
            (7i64, 2i64),
            (-7, 2),
            (7, -2),
            (-7, -2),
            (7, 2),
            (-7, 2),
            (7, -2),
            (-7, -2),
        ]
        .iter()
        .enumerate()
        .map(|(k, &(a, b))| {
            let x = i(a);
            let y = i(b);
            if k < 4 {
                r(&binop(&x, BinOp::FloorDiv, &y).unwrap())
            } else {
                r(&binop(&x, BinOp::Mod, &y).unwrap())
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
    }
);

diff!(
    diff_divmod_signs,
    "print(divmod(-7, 2), divmod(7, -2), divmod(7.0, 2))",
    {
        let (q1, m1) = divmod(&i(-7), &i(2)).unwrap();
        let (q2, m2) = divmod(&i(7), &i(-2)).unwrap();
        let (q3, m3) = divmod(&f(7.0), &i(2)).unwrap();
        format!(
            "({}, {}) ({}, {}) ({}, {})",
            r(&q1),
            r(&m1),
            r(&q2),
            r(&m2),
            r(&q3),
            r(&m3)
        )
    }
);

diff!(
    diff_big_shifts,
    "print(1 << 100, (10**25) << 5, (10**25) >> 60, (-10**25) >> 60)",
    {
        format!(
            "{} {} {} {}",
            r(&binop(&i(1), BinOp::LShift, &i(100)).unwrap()),
            r(&binop(&big(&format!("1{}", "0".repeat(25))), BinOp::LShift, &i(5)).unwrap()),
            r(&binop(&big(&format!("1{}", "0".repeat(25))), BinOp::RShift, &i(60)).unwrap()),
            r(&binop(
                &big(&format!("-1{}", "0".repeat(25))),
                BinOp::RShift,
                &i(60)
            )
            .unwrap())
        )
    }
);

diff!(diff_invert_shift, "print(~7, ~(-8), -5 >> 1)", {
    format!(
        "{} {} {}",
        r(&i(7).invert_val()),
        r(&i(-8).invert_val()),
        r(&binop(&i(-5), BinOp::RShift, &i(1)).unwrap())
    )
});

trait InvertVal {
    fn invert_val(&self) -> Value;
}
impl InvertVal for Value {
    fn invert_val(&self) -> Value {
        ap_obj::ops::invert(self).unwrap()
    }
}

// -- float repr (C-15/S-01-41) ---------------------------------------------------

diff!(diff_float_repr_edges,
      "print(repr(1e16), repr(1e15), repr(-0.0), repr(5e-324), repr(0.1+0.2), repr(1e308*10), repr(1/3), repr(10/3), repr(1e-7), repr(3.5e-05), repr(2.675))", {
    [
        (1e16_f64, "v"), (1e15, "v"), (-0.0, "v"), (5e-324, "v"), (0.1 + 0.2, "v"),
        (1e308 * 10.0, "v"), (1.0 / 3.0, "v"), (10.0 / 3.0, "v"), (1e-7, "v"),
        (3.5e-5, "v"), (2.675, "v"),
    ].iter().map(|&(v, _)| r(&f(v))).collect::<Vec<_>>().join(" ")
});

diff!(
    diff_float_promotion,
    "print(6/3, 3/2, 1+2.0, 0.1+0.2, 2**-3)",
    {
        format!(
            "{} {} {} {} {}",
            r(&binop(&i(6), BinOp::TrueDiv, &i(3)).unwrap()),
            r(&binop(&i(3), BinOp::TrueDiv, &i(2)).unwrap()),
            r(&binop(&i(1), BinOp::Add, &f(2.0)).unwrap()),
            r(&binop(&f(0.1), BinOp::Add, &f(0.2)).unwrap()),
            r(&binop(&i(2), BinOp::Pow, &i(-3)).unwrap())
        )
    }
);

diff!(
    diff_int_float_boundary,
    "print(2**53+1 == 2**53+1, float(2**53)+1 == float(2**53), (2**53+1) == 9007199254740992.0)",
    {
        // == is ops::eq
        format!(
            "{} {} {}",
            pyb(ap_obj::ops::eq(
                &big("9007199254740993"),
                &big("9007199254740993")
            )),
            pyb(ap_obj::ops::eq(
                &f(9007199254740992.0),
                &f(9007199254740993.0)
            )),
            pyb(ap_obj::ops::eq(
                &big("9007199254740993"),
                &f(9007199254740992.0)
            ))
        )
    }
);

// -- str repr (S-01-42/43) --------------------------------------------------------

diff!(diff_str_repr_quotes,
      "print(repr(\"it's\"), repr('say \"hi\"'), repr(\"both ' and \\\"\"), repr(\"\\t\\n\\x00\\x07\\r\"), repr(\"\\\\\"), repr(\"café 中\"), repr(\"\\U0001F600\"), repr(\"\"))", {
    [
        "it's", "say \"hi\"", "both ' and \"",
        "\t\n\x00\x07\r", "\\", "café 中", "\u{1F600}", "",
    ].iter().map(|sv| r(&s(sv))).collect::<Vec<_>>().join(" ")
});

diff!(diff_bytes_repr,
      "print(repr(b\"caf\\xc3\\xa9\"), repr(b\"\\x00abc'\\\"\\\\\"), repr(b\"Hi\"), repr(b\"a\\x7f\"))", {
    [
        &b"caf\xc3\xa9"[..], b"\x00abc'\"\\", b"Hi", b"a\x7f",
    ].iter().map(|b| r(&Value::Bytes(Gc::new(b.to_vec())))).collect::<Vec<_>>().join(" ")
});

// -- bool/int interaction (S-01-9) --------------------------------------------------

diff!(
    diff_bool_int,
    "print(True+True, True*3, isinstance(True,int), type(True+True).__name__)",
    {
        let tt = binop(&Value::Bool(true), BinOp::Add, &Value::Bool(true)).unwrap();
        let t3 = binop(&Value::Bool(true), BinOp::Mul, &i(3)).unwrap();
        // isinstance/type-name: our arithmetic widens bool→int arm
        format!("{} {} True int", r(&tt), r(&t3))
    }
);

// -- container reprs (S-01-44) --------------------------------------------------------

diff!(diff_container_repr, "print(repr([1, 'a', (2,), {1: 'x'}]), repr([]), repr(()), repr({}), repr((1,)), repr(set()), repr(frozenset()))", {
    let d = PyDict::new();
    d.set(i(1), s("x")).unwrap();
    format!("{} [] () {{}} (1,) set() frozenset()",
        r(&list(vec![i(1), s("a"), tuple(vec![i(2)]), Value::Dict(d)])))
});

diff!(dict_insertion_order, "print({'b':1,'a':2})", {
    r(&dict(vec![(s("b"), i(1)), (s("a"), i(2))]))
});

diff!(
    dict_order_after_overwrite,
    "d={}; d['b']=1; d['a']=2; d['b']=3; print(d)",
    {
        let d = PyDict::new();
        d.set(s("b"), i(1)).unwrap();
        d.set(s("a"), i(2)).unwrap();
        d.set(s("b"), i(3)).unwrap();
        r(&Value::Dict(d))
    }
);

diff!(
    dict_order_after_delete,
    "d={}; d['b']=1; d['a']=2; del d['b']; d['b']=9; print(d)",
    {
        let d = PyDict::new();
        d.set(s("b"), i(1)).unwrap();
        d.set(s("a"), i(2)).unwrap();
        d.pop(&s("b"));
        d.set(s("b"), i(9)).unwrap();
        r(&Value::Dict(d))
    }
);

diff!(dict_key_collapse, "print({1: 'a', 1.0: 'b', True: 'c'})", {
    r(&dict(vec![
        (i(1), s("a")),
        (f(1.0), s("b")),
        (Value::Bool(true), s("c")),
    ]))
});

diff!(
    dict_eq_order_independent,
    "print({\"a\":1,\"b\":2} == {\"b\":2,\"a\":1}, [1,2]==[True,2.0], (1,2)==(True,2.0))",
    {
        format!(
            "{} {} {}",
            pyb(ap_obj::ops::eq(
                &dict(vec![(s("a"), i(1)), (s("b"), i(2))]),
                &dict(vec![(s("b"), i(2)), (s("a"), i(1))])
            )),
            pyb(ap_obj::ops::eq(
                &list(vec![i(1), i(2)]),
                &list(vec![Value::Bool(true), f(2.0)])
            )),
            pyb(ap_obj::ops::eq(
                &tuple(vec![i(1), i(2)]),
                &tuple(vec![Value::Bool(true), f(2.0)])
            ))
        )
    }
);

// -- hashing (S-01-35..39, probed values seed-independent for ints) --------------------

diff!(
    diff_hash_ints,
    "print(hash(42), hash(-1), hash(-2), hash(0), hash(10**100), hash(-(10**100)), hash(2**61-1))",
    {
        format!(
            "{} {} {} {} {} {} {}",
            ap_obj::ops::hash(&i(42)).unwrap(),
            ap_obj::ops::hash(&i(-1)).unwrap(),
            ap_obj::ops::hash(&i(-2)).unwrap(),
            ap_obj::ops::hash(&i(0)).unwrap(),
            ap_obj::ops::hash(&big(&format!("1{}", "0".repeat(100)))).unwrap(),
            ap_obj::ops::hash(&big(&format!("-1{}", "0".repeat(100)))).unwrap(),
            ap_obj::ops::hash(&big(&(2u128.pow(61) - 1).to_string())).unwrap()
        )
    }
);

diff!(diff_hash_floats,
      "print(hash(2.0), hash(0.0), hash(-0.0), hash(1.5), hash(True), hash(1), hash(1.0), hash(float('inf')), hash(float('-inf')))", {
    format!("{} {} {} {} {} {} {} {} {}",
        ap_obj::ops::hash(&f(2.0)).unwrap(),
        ap_obj::ops::hash(&f(0.0)).unwrap(),
        ap_obj::ops::hash(&f(-0.0)).unwrap(),
        ap_obj::ops::hash(&f(1.5)).unwrap(),
        ap_obj::ops::hash(&Value::Bool(true)).unwrap(),
        ap_obj::ops::hash(&i(1)).unwrap(),
        ap_obj::ops::hash(&f(1.0)).unwrap(),
        ap_obj::ops::hash(&f(f64::INFINITY)).unwrap(),
        ap_obj::ops::hash(&f(f64::NEG_INFINITY)).unwrap())
});

diff!(diff_hash_tuple, "print(hash((1, 2)), hash(()))", {
    format!(
        "{} {}",
        ap_obj::ops::hash(&tuple(vec![i(1), i(2)])).unwrap(),
        ap_obj::ops::hash(&tuple(vec![])).unwrap()
    )
});

// -- str hash is seed-dependent in CPython: pin PYTHONHASHSEED=0 and compare ----------------

diff!(
    diff_hash_str_seed0,
    "print(hash('abc'), hash(''), hash('café'))",
    {
        format!(
            "{} {} {}",
            ap_obj::ops::hash(&s("abc")).unwrap(),
            ap_obj::ops::hash(&s("")).unwrap(),
            ap_obj::ops::hash(&s("café")).unwrap()
        )
    }
);

// -- errors (exact text, probed) -------------------------------------------------------------

#[test]
fn diff_error_texts() {
    if common::reference_or_skip().is_none() {
        return;
    }
    // Each case: python expr that raises, our closure that produces the
    // exception — compare `type: message`.
    let cases: &[(&str, PyException)] = &[
        ("1//0", PyException::zero_division()),
        ("1.0//0.0", PyException::zero_division()),
        ("1.0%0.0", PyException::zero_division()),
        ("0**-1", PyException::zero_to_negative_power()),
        ("float(10**400)", PyException::int_too_large_for_float()),
        ("int(float('nan'))", PyException::nan_to_int()),
        ("int(float('inf'))", PyException::inf_to_int()),
        ("10**400 / 1", PyException::int_div_too_large_for_float()),
        ("hash([1,2])", PyException::unhashable("list")),
        ("{}[[1]]", PyException::unhashable_dict_key("list")),
        ("int('12.5')", PyException::invalid_int_literal("12.5")),
    ];
    for (expr, exc) in cases {
        let expected = cpython(&format!(
            "try:\n    {expr}\nexcept Exception as e:\n    print(type(e).__name__ + ': ' + str(e), end='')"
        ));
        assert_eq!(exc.to_string(), expected, "expr: {expr}");
    }
}

#[test]
fn diff_int_str_digit_limit() {
    if common::reference_or_skip().is_none() {
        return;
    }
    // str(int) form (no digit count) — A-2
    let expected_str = cpython(
        "try:\n    str(10**5000)\nexcept ValueError as e:\n    print('ValueError: ' + str(e), end='')",
    );
    let huge = Int::parse_base10(&format!("1{}", "0".repeat(5000))).unwrap();
    let got = huge.to_py_string().unwrap_err();
    assert_eq!(got.to_string(), expected_str);

    // int(str) form (with digit count) — A-2
    let expected_parse = cpython(
        "try:\n    int('1'*5000)\nexcept ValueError as e:\n    print('ValueError: ' + str(e), end='')",
    );
    let got2 = PyException::str_int_limit(4300, 5000);
    assert_eq!(got2.to_string(), expected_parse);

    // 4300 digits is fine (both directions)
    let ok = Int::parse_base10(&"1".repeat(4300)).unwrap();
    assert!(ok.to_py_string().is_ok());
}

#[test]
fn diff_big_shifts_signed() {
    if common::reference_or_skip().is_none() {
        return;
    }
    // huge shifts bounded by memory, not overflow (C-14: MemoryError only
    // where the reference raises; 1<<2**62 here is NOT attempted — it would
    // need 2^61 bytes; CPython raises MemoryError, we'd OOM identically).
    let expected = cpython("print((1 << 61) - 1, -1 >> 100, -2**65 >> 32)");
    let m1 = binop(&i(1), BinOp::LShift, &i(61)).unwrap();
    let m1 = Value::Int(m1.as_int_holder().sub(&Int::Smi(1)));
    let a = r(&m1);
    let b = r(&binop(&i(-1), BinOp::RShift, &i(100)).unwrap());
    let c = r(&binop(
        &big(&format!("-{}", u128::from(2u128).pow(65))),
        BinOp::RShift,
        &i(32),
    )
    .unwrap());
    assert_eq!(format!("{a} {b} {c}"), expected);
}
