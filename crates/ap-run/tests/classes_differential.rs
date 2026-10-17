//! Differential class-protocol probes (leaf 02.03): each probe runs under
//! CPython 3.14 AND the ap-run tier-0 executor; stdout bytes and exit code
//! must match byte-exactly (S-06-1/S-06-2). Probes cite the S-03 clause they
//! pin (docs/spec/03-classes.md).

use std::io::Write;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static PROBE_SEQ: AtomicU64 = AtomicU64::new(0);

fn ref_python() -> &'static str {
    for cand in ["/opt/homebrew/bin/python3.14", "python3"] {
        if Command::new(cand).arg("--version").output().is_ok() {
            return cand;
        }
    }
    panic!("no reference python3.14 on this host");
}

fn run_reference(src: &str) -> (Vec<u8>, i32) {
    let dir = std::env::temp_dir().join(format!(
        "aprun-cls-{}-{}",
        std::process::id(),
        PROBE_SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("probe.py");
    std::fs::write(&path, src).unwrap();
    let out = Command::new(ref_python())
        .arg(&path)
        .output()
        .expect("spawn reference python");
    let _ = std::fs::remove_dir_all(&dir);
    (out.stdout, out.status.code().unwrap_or(-1))
}

fn run_aprun(src: &str) -> (Vec<u8>, i32) {
    let src = src.to_string();
    let handle = std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(move || {
            let mut out: Vec<u8> = Vec::new();
            let mut err: Vec<u8> = Vec::new();
            let outcome = ap_run::run_module(&src, "probe.py", &mut out, &mut err);
            let _ = Write::flush(&mut out);
            (out, outcome.exit_code)
        })
        .expect("spawn worker thread");
    handle.join().expect("worker thread")
}

fn assert_differential(name: &str, src: &str) {
    let (ref_out, ref_exit) = run_reference(src);
    let (ap_out, ap_exit) = run_aprun(src);
    assert_eq!(
        String::from_utf8_lossy(&ap_out),
        String::from_utf8_lossy(&ref_out),
        "{name}: stdout diverges from CPython 3.14\n--- source ---\n{src}"
    );
    assert_eq!(
        ap_exit, ref_exit,
        "{name}: exit code diverges\n--- source ---\n{src}"
    );
}

// ===========================================================================
// Attribute lookup order (S-03-09/10/11/12)
// ===========================================================================

#[test]
fn lookup_data_descriptor_shadows_instance_dict() {
    assert_differential(
        "data descriptor priority",
        concat!(
            "class C:\n",
            "    @property\n",
            "    def x(self):\n",
            "        return \"prop\"\n",
            "c = C()\n",
            "c.__dict__[\"x\"] = 99\n",
            "print(c.x)\n",
            "print(type(C.__dict__[\"x\"]).__name__)\n",
        ),
    );
}

#[test]
fn lookup_nondata_shadowed_by_instance_dict() {
    assert_differential(
        "non-data descriptor shadowing",
        concat!(
            "class ND:\n",
            "    def __get__(self, obj, objtype=None):\n",
            "        return \"nd\"\n",
            "class C:\n",
            "    x = ND()\n",
            "c = C()\n",
            "print(c.x)\n",
            "c.x = 99\n",
            "print(c.x, c.__dict__[\"x\"] == 99)\n",
            "c2 = C()\n",
            "print(c2.x, \"x\" in c2.__dict__)\n",
        ),
    );
}

#[test]
fn getattr_fallback_and_miss_message() {
    assert_differential(
        "__getattr__ fallback",
        concat!(
            "class C:\n",
            "    def __getattr__(self, name):\n",
            "        return \"gen:\" + name\n",
            "c = C()\n",
            "c.explicit = 1\n",
            "print(c.missing2, c.explicit)\n",
            "class D: pass\n",
            "try:\n",
            "    d = D()\n",
            "    d.nope\n",
            "except AttributeError as e:\n",
            "    print(\"AttributeError:\", e)\n",
        ),
    );
}

#[test]
fn property_setter_and_readonly() {
    assert_differential(
        "property setter routing",
        concat!(
            "class C:\n",
            "    @property\n",
            "    def x(self):\n",
            "        return \"get\"\n",
            "    @x.setter\n",
            "    def x(self, v):\n",
            "        self._stored = v * 2\n",
            "c = C()\n",
            "c.x = 5\n",
            "print(c._stored, c.x)\n",
            "class RO:\n",
            "    @property\n",
            "    def x(self):\n",
            "        return 1\n",
            "try:\n",
            "    RO().x = 2\n",
            "except AttributeError as e:\n",
            "    print(\"AttributeError:\", e)\n",
        ),
    );
}

#[test]
fn delete_rules() {
    assert_differential(
        "del priority",
        concat!(
            "class DD:\n",
            "    def __get__(self, obj, objtype=None):\n",
            "        return \"dd\"\n",
            "    def __set__(self, obj, v):\n",
            "        obj.__dict__[\"_b\"] = v\n",
            "    def __delete__(self, obj):\n",
            "        print(\"dd-delete\")\n",
            "class C:\n",
            "    b = DD()\n",
            "c = C()\n",
            "del c.b\n",
            "c.b = 3\n",
            "print(c.__dict__)\n",
            "class P:\n",
            "    x = 1\n",
            "class C2(P): pass\n",
            "try:\n",
            "    del C2().x\n",
            "except AttributeError as e:\n",
            "    print(\"AttributeError:\", e)\n",
        ),
    );
}

// ===========================================================================
// MRO + super (S-03-15..19)
// ===========================================================================

#[test]
fn mro_diamond() {
    assert_differential(
        "C3 diamond",
        concat!(
            "class A: pass\n",
            "class B(A): pass\n",
            "class C(A): pass\n",
            "class D(B, C): pass\n",
            "print([t.__name__ for t in D.__mro__])\n",
            "print(D.__mro__[-1] is object)\n",
        ),
    );
}

#[test]
fn super_diamond_chase() {
    assert_differential(
        "diamond chase",
        concat!(
            "class A:\n",
            "    def m(self):\n",
            "        print(\"A\")\n",
            "class B(A):\n",
            "    def m(self):\n",
            "        print(\"B\")\n",
            "        super().m()\n",
            "class C(A):\n",
            "    def m(self):\n",
            "        print(\"C\")\n",
            "        super().m()\n",
            "class D(B, C):\n",
            "    def m(self):\n",
            "        print(\"D\")\n",
            "        super().m()\n",
            "D().m()\n",
            "class E(D): pass\n",
            "E().m()\n",
        ),
    );
}

#[test]
fn super_classmethod_chain() {
    assert_differential(
        "classmethod super",
        concat!(
            "class A:\n",
            "    @classmethod\n",
            "    def m(cls):\n",
            "        return cls.__name__\n",
            "class B(A):\n",
            "    @classmethod\n",
            "    def m(cls):\n",
            "        return \"B->\" + super().m()\n",
            "print(B.m())\n",
            "class C(B): pass\n",
            "print(C.m())\n",
        ),
    );
}

#[test]
fn super_validation_errors() {
    assert_differential(
        "super validation",
        concat!(
            "class SA: pass\n",
            "class SB(SA): pass\n",
            "try:\n",
            "    super(SA, 5)\n",
            "except TypeError as e:\n",
            "    print(\"TypeError:\", e)\n",
            "print(isinstance(super(SA, SB()), SA), type(super(SA, SB())).__name__)\n",
            "class Holder(SA):\n",
            "    def probe(self):\n",
            "        def inner():\n",
            "            return super()\n",
            "        return inner()\n",
            "try:\n",
            "    Holder().probe()\n",
            "except RuntimeError as e:\n",
            "    print(\"RuntimeError:\", e)\n",
        ),
    );
}

// ===========================================================================
// Instantiation + __new__/__init__ (S-03-05..08)
// ===========================================================================

#[test]
fn new_skips_init_on_noninstance() {
    assert_differential(
        "__new__ skips __init__",
        concat!(
            "class A:\n",
            "    def __new__(cls):\n",
            "        print(\"new\")\n",
            "        return \"not an A\"\n",
            "    def __init__(self):\n",
            "        print(\"init\")\n",
            "a = A()\n",
            "print(a, type(a).__name__)\n",
        ),
    );
}

#[test]
fn newinit_arity_errors() {
    assert_differential(
        "arity cross-check",
        concat!(
            "class B: pass\n",
            "try:\n",
            "    B(1)\n",
            "except TypeError as e:\n",
            "    print(\"TypeError:\", e)\n",
            "class A7:\n",
            "    def __init__(self, x):\n",
            "        self.x = x\n",
            "try:\n",
            "    A7()\n",
            "except TypeError as e:\n",
            "    print(\"TypeError:\", e)\n",
        ),
    );
}

#[test]
fn init_return_nonnone_raises() {
    assert_differential(
        "__init__ return check",
        concat!(
            "class R8:\n",
            "    def __init__(self):\n",
            "        return 5\n",
            "try:\n",
            "    R8()\n",
            "except TypeError as e:\n",
            "    print(\"TypeError:\", e)\n",
        ),
    );
}

// ===========================================================================
// Operators + reflected (S-03-25/26)
// ===========================================================================

#[test]
fn binop_reflected_priority() {
    assert_differential(
        "reflected priority",
        concat!(
            "class L:\n",
            "    def __add__(self, o):\n",
            "        return \"L.add\"\n",
            "class R(L):\n",
            "    def __radd__(self, o):\n",
            "        return \"R.radd\"\n",
            "print(L() + R(), R() + L())\n",
            "class N25:\n",
            "    def __add__(self, o):\n",
            "        return NotImplemented\n",
            "class M25:\n",
            "    def __radd__(self, o):\n",
            "        return NotImplemented\n",
            "try:\n",
            "    N25() + M25()\n",
            "except TypeError as e:\n",
            "    print(\"TypeError:\", e)\n",
            "try:\n",
            "    1 + \"x\"\n",
            "except TypeError as e:\n",
            "    print(\"TypeError:\", e)\n",
        ),
    );
}

#[test]
fn augmented_rebinds() {
    assert_differential(
        "augmented rebind",
        concat!(
            "class Aug:\n",
            "    def __init__(self, v):\n",
            "        self.v = v\n",
            "    def __iadd__(self, o):\n",
            "        print(\"iadd\")\n",
            "        return Aug(self.v + 1)\n",
            "a = Aug(1)\n",
            "a += 2\n",
            "print(a.v)\n",
            "l1 = [1]\n",
            "orig = l1\n",
            "l1 += [2]\n",
            "print(l1, l1 is orig)\n",
        ),
    );
}

#[test]
fn unary_slots() {
    assert_differential(
        "unary dunders",
        concat!(
            "class Un:\n",
            "    def __neg__(self): return \"neg\"\n",
            "    def __pos__(self): return \"pos\"\n",
            "    def __invert__(self): return \"inv\"\n",
            "    def __abs__(self): return \"abs\"\n",
            "u = Un()\n",
            "print(-u, +u, ~u, abs(u))\n",
        ),
    );
}

// ===========================================================================
// Iteration + contains + call (S-03-24/26)
// ===========================================================================

#[test]
fn iter_in_for_and_in_operator() {
    assert_differential(
        "__iter__ + __contains__",
        concat!(
            "class Box:\n",
            "    def __iter__(self):\n",
            "        return iter([1, 2, 3])\n",
            "print([v for v in Box()])\n",
            "class Cont:\n",
            "    def __contains__(self, item):\n",
            "        return item == \"yes\"\n",
            "print(\"yes\" in Cont(), \"no\" in Cont())\n",
            "class IterOnly:\n",
            "    def __iter__(self):\n",
            "        return iter([1, 2, 3])\n",
            "print(2 in IterOnly(), 5 in IterOnly())\n",
        ),
    );
}

#[test]
fn call_protocol() {
    assert_differential(
        "__call__",
        concat!(
            "class Callable:\n",
            "    def __call__(self, n):\n",
            "        return n * 2\n",
            "print(Callable()(3))\n",
            "class C: pass\n",
            "try:\n",
            "    C()()\n",
            "except TypeError as e:\n",
            "    print(\"TypeError:\", e)\n",
        ),
    );
}

#[test]
fn len_bool_len_fallback() {
    assert_differential(
        "len/bool protocols",
        concat!(
            "class L0:\n",
            "    def __len__(self):\n",
            "        return 0\n",
            "class B:\n",
            "    def __bool__(self):\n",
            "        return False\n",
            "print(len(L0()), bool(L0()), bool(B()), not L0())\n",
        ),
    );
}

// ===========================================================================
// Exceptions (S-03-32..34)
// ===========================================================================

#[test]
fn user_exception_hierarchy_str() {
    assert_differential(
        "user exception hierarchy",
        concat!(
            "class Level1(Exception): pass\n",
            "class Level3(Level1): pass\n",
            "print([t.__name__ for t in Level3.__mro__])\n",
            "try:\n",
            "    raise Level3(\"a\", 2)\n",
            "except Level1 as e:\n",
            "    print(type(e).__name__, str(e), repr(e), e.args)\n",
            "class WithInit(Level1):\n",
            "    def __init__(self, code):\n",
            "        super().__init__(f\"E{code}\")\n",
            "try:\n",
            "    raise WithInit(42)\n",
            "except Level1 as e:\n",
            "    print(type(e).__name__, str(e), repr(e), e.args)\n",
        ),
    );
}

#[test]
fn identity_preserved_at_handler() {
    assert_differential(
        "raise instance identity",
        concat!(
            "class MyErr(Exception): pass\n",
            "inst = MyErr(\"payload\")\n",
            "try:\n",
            "    raise inst\n",
            "except MyErr as e:\n",
            "    print(e is inst, str(e))\n",
        ),
    );
}

#[test]
fn except_tuple_and_nonclass_rejected() {
    assert_differential(
        "except targets",
        concat!(
            "try:\n",
            "    raise ValueError()\n",
            "except (KeyError, ValueError):\n",
            "    print(\"tuple any-element ok\")\n",
            "class NotExc: pass\n",
            "try:\n",
            "    try:\n",
            "        raise KeyError()\n",
            "    except NotExc:\n",
            "        pass\n",
            "except TypeError as e:\n",
            "    print(\"TypeError:\", e)\n",
        ),
    );
}

// ===========================================================================
// Slots (S-03-13)
// ===========================================================================

#[test]
fn slots_restriction_messages() {
    assert_differential(
        "__slots__ messages",
        concat!(
            "class A:\n",
            "    __slots__ = (\"x\",)\n",
            "a = A()\n",
            "a.x = 1\n",
            "try:\n",
            "    a.z = 2\n",
            "except AttributeError as e:\n",
            "    print(\"AttributeError:\", e)\n",
            "print(a.x)\n",
            "try:\n",
            "    a.z\n",
            "except AttributeError as e:\n",
            "    print(\"AttributeError:\", e)\n",
        ),
    );
}

// ===========================================================================
// Dunder bypass + classmethod/staticmethod (S-03-21/22)
// ===========================================================================

#[test]
fn dunder_bypasses_instance_dict() {
    assert_differential(
        "dunder type lookup",
        concat!(
            "class C:\n",
            "    def __len__(self):\n",
            "        return 3\n",
            "c = C()\n",
            "c.__len__ = lambda: 99\n",
            "print(len(c))\n",
        ),
    );
}

#[test]
fn classmethod_staticmethod_binding() {
    assert_differential(
        "descriptor kinds",
        concat!(
            "class C:\n",
            "    @classmethod\n",
            "    def cm(cls):\n",
            "        return cls.__name__\n",
            "    @staticmethod\n",
            "    def sm():\n",
            "        return 1\n",
            "    def f(self):\n",
            "        return 2\n",
            "c = C()\n",
            "print(C.cm(), c.cm())\n",
            "print(C.sm is c.sm, C.sm())\n",
            "print(c.f == c.f, c.f is not c.f, c.f(), c.f.__self__ is c)\n",
        ),
    );
}

#[test]
fn repr_vs_str_semantics() {
    assert_differential(
        "__repr__/__str__ divergence",
        concat!(
            "class Both:\n",
            "    def __str__(self):\n",
            "        return \"STR\"\n",
            "    def __repr__(self):\n",
            "        return \"REPR\"\n",
            "print(str(Both()), repr(Both()), f\"{Both()}\")\n",
            "class OnlyStr:\n",
            "    def __str__(self):\n",
            "        return \"S\"\n",
            "print(str(OnlyStr()))\n",
        ),
    );
}

#[test]
fn classcell_zero_arg_super_validation() {
    assert_differential(
        "__class__ cell",
        concat!(
            "class Base:\n",
            "    def greet(self):\n",
            "        return \"base\"\n",
            "class Derived(Base):\n",
            "    def greet(self):\n",
            "        return \"derived+\" + super().greet()\n",
            "print(Derived().greet())\n",
            "try:\n",
            "    super()\n",
            "except RuntimeError as e:\n",
            "    print(\"RuntimeError:\", e)\n",
        ),
    );
}

#[test]
fn enter_exit_context_manager() {
    assert_differential(
        "__enter__/__exit__",
        concat!(
            "class Ctx:\n",
            "    def __enter__(self):\n",
            "        print(\"enter\")\n",
            "        return 42\n",
            "    def __exit__(self, a, b, c):\n",
            "        print(\"exit\", a is None)\n",
            "with Ctx() as v:\n",
            "    print(\"body\", v)\n",
            "class Suppress:\n",
            "    def __enter__(self):\n",
            "        return self\n",
            "    def __exit__(self, a, b, c):\n",
            "        return True\n",
            "with Suppress():\n",
            "    raise ValueError(\"swallowed\")\n",
            "print(\"after\")\n",
        ),
    );
}

#[test]
fn metaclass_attribute_chain() {
    assert_differential(
        "metaclass attr fallback",
        concat!(
            "class M(type):\n",
            "    mattr = 7\n",
            "class C(metaclass=M):\n",
            "    pass\n",
            "print(C.mattr)\n",
            "print(C.whatever if False else \"skip\")\n",
            "try:\n",
            "    C().mattr\n",
            "except AttributeError as e:\n",
            "    print(\"AttributeError:\", e)\n",
        ),
    );
}

#[test]
fn isinstance_union_and_tuple() {
    assert_differential(
        "isinstance tuple + union",
        concat!(
            "class A: pass\n",
            "class B(A): pass\n",
            "print(isinstance(B(), (str, B)), issubclass(B, (int, A)))\n",
            "class U1: pass\n",
            "class U2: pass\n",
            "print(isinstance(U1(), U1 | U2), isinstance(5, U1 | U2))\n",
            "try:\n",
            "    isinstance(1, 2)\n",
            "except TypeError as e:\n",
            "    print(\"TypeError:\", e)\n",
        ),
    );
}
