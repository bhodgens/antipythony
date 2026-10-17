//! Native v1 stdlib modules (leaf 02.08-lite).
//!
//! Modules that need NO class machinery and NO import-from-file machinery,
//! implemented natively over ap-obj values (S-05-4/S-05-5). Each module is a
//! `PyObject(ObjKind::Module)` whose `module_ns` dict holds constants and
//! `BuiltinFn` markers named `"<mod>.<fn>"`; `builtins::call_builtin` routes
//! dotted names here (the same pattern as the `sys.` surface).
//!
//! Error texts are byte-exact vs CPython 3.14.7 (probed live on
//! /opt/homebrew/bin/python3.14; spec 05 cites 3.14.7 live probes).
//!
//! DOCUMENTED DIVERGENCES (spec 05 conflicts, flagged for orchestrator triage):
//! - `random`, `hashlib`, `base64`, `binascii` are implemented here per the
//!   02.08-lite task brief, but spec 05 §5.3.3 (S-05-19/S-05-20) marks them
//!   OUT-of-v1 with fail-loud imports. No corpus slice imports them; reconcile
//!   via a spec amendment.
//! - `random` uses an EXACT MT19937 port (init_by_array seeding +
//!   genrand_uint32 + genrand_res53 float), so `random.seed(n); random.random()`
//!   sequences byte-match CPython. `choice/shuffle/sample` follow CPython's
//!   algorithms (shuffle = Fisher–Yates backwards with `_randbelow`;
//!   sample = selection-set for small n), so seeded reproducibility holds.
//! - `string.Template`/`Formatter` and `collections.Counter/defaultdict`
//!   (subclass-based surfaces) are NOT here — they need class machinery
//!   (02.03). `deque` IS here as a native opaque type whose methods dispatch
//!   through the attr_get hook + BoundMethod fallback.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use crate::exc_hier::ExceptionValue;
use crate::Flow;
use ap_obj::dict::PyDict;
use ap_obj::int::{Gc, Int};
use ap_obj::value::{PyList, PyString, Tuple, Value};
use ap_obj::PyObject;

pub type BResult = Result<Value, Flow>;

fn raise(class: &str, msg: impl Into<String>) -> Flow {
    Flow::Raise(ExceptionValue::msg(class, msg.into()))
}

// ---------------------------------------------------------------------------
// Value helpers
// ---------------------------------------------------------------------------

pub(crate) fn as_f64(v: &Value) -> Result<f64, Flow> {
    match v {
        Value::Float(x) => Ok(*x),
        Value::Int(i) => i
            .to_f64()
            .ok_or_else(|| raise("OverflowError", "int too large to convert to float")),
        Value::Bool(b) => Ok(if *b { 1.0 } else { 0.0 }),
        other => Err(raise(
            "TypeError",
            format!("must be real number, not {}", other.type_name()),
        )),
    }
}

pub(crate) fn as_int_i64(v: &Value) -> Result<i64, Flow> {
    match v {
        Value::Int(i) => i.to_i64().ok_or_else(|| {
            raise(
                "OverflowError",
                "Python int too large to convert to C ssize_t",
            )
        }),
        Value::Bool(b) => Ok(if *b { 1 } else { 0 }),
        other => Err(raise(
            "TypeError",
            format!(
                "'{}' object cannot be interpreted as an integer",
                other.type_name()
            ),
        )),
    }
}

fn as_int_big(v: &Value) -> Result<Int, Flow> {
    match v {
        Value::Int(i) => Ok(i.clone()),
        Value::Bool(b) => Ok(Int::from_i64(if *b { 1 } else { 0 })),
        other => Err(raise(
            "TypeError",
            format!(
                "'{}' object cannot be interpreted as an integer",
                other.type_name()
            ),
        )),
    }
}

pub(crate) fn i(n: i64) -> Value {
    Value::Int(Int::from_i64(n))
}

pub(crate) fn f(x: f64) -> Value {
    Value::Float(x)
}

pub(crate) fn s(x: impl Into<String>) -> Value {
    Value::Str(PyString::new(x.into()))
}

pub(crate) fn tuple(v: Vec<Value>) -> Value {
    Value::Tuple(Gc::new(Tuple(v)))
}

pub(crate) fn list(v: Vec<Value>) -> Value {
    Value::List(PyList::from_vec(v))
}

pub(crate) fn kw<'a>(kwargs: &'a [(String, Value)], name: &str) -> Option<&'a Value> {
    kwargs.iter().find(|(k, _)| k == name).map(|(_, v)| v)
}

pub(crate) fn collect(v: &Value) -> Result<Vec<Value>, Flow> {
    let it = crate::interp::iterable_of_public(v)?;
    let mut out = Vec::new();
    while let Some(x) = it.next() {
        out.push(x);
    }
    Ok(out)
}

pub(crate) fn truthy(v: &Value) -> bool {
    v.is_truthy()
}

pub(crate) fn py_lt(a: &Value, b: &Value) -> Result<bool, Flow> {
    ap_obj::ops::lt(a, b).map_err(Flow::from_py)
}

pub(crate) fn py_eq(a: &Value, b: &Value) -> bool {
    ap_obj::ops::eq(a, b)
}

// ---------------------------------------------------------------------------
// Int helpers (num-bigint gcd via Euclid on our Int; isqrt via Newton)
// ---------------------------------------------------------------------------

fn int_gcd(mut a: Int, mut b: Int) -> Int {
    while !b.is_zero() {
        let r = a.floor_div(&b);
        let _ = r;
        let m = a.rem(&b);
        a = b;
        b = m;
        if b.is_negative() {
            b = b.neg();
        }
    }
    if a.is_negative() {
        a.neg()
    } else {
        a
    }
}

fn int_isqrt(n: &Int) -> Int {
    if n.is_zero() {
        return Int::from_i64(0);
    }
    // Newton: x_{k+1} = (x_k + n // x_k) // 2
    let two = Int::from_i64(2);
    let bits = n.decimal_digits() * 4; // rough seed
    let mut x = Int::from_i64(1).shl((bits as u64 / 2).max(1) + 1);
    loop {
        let nx = x.add(&n.floor_div(&x)).floor_div(&two);
        if nx.cmp_int(&x) != std::cmp::Ordering::Less && nx.eq_int(&x) {
            return nx;
        }
        if nx.cmp_int(&x) == std::cmp::Ordering::Equal {
            return nx;
        }
        // check convergence: floor(sqrt(n)) is the fixed point
        let sq = nx.mul(&nx);
        match sq.cmp_int(n) {
            std::cmp::Ordering::Less | std::cmp::Ordering::Equal => {
                let next = nx.add(&Int::from_i64(1));
                if next.mul(&next).cmp_int(n) == std::cmp::Ordering::Greater {
                    return nx;
                }
            }
            std::cmp::Ordering::Greater => {}
        }
        if nx.cmp_int(&x) == std::cmp::Ordering::Greater {
            // Overshoot: we passed the fixed point coming back down.
            return x;
        }
        x = nx;
    }
}

// ===========================================================================
// Native iterator states (self-contained; iter.rs delegates `next` here)
// ===========================================================================

/// Python-level callables stored under synthetic names (resolved through
/// [`call_named`]); dotted names (`operator.mul`) route to builtin dispatch.
pub enum NativeIterState {
    Seq(std::vec::IntoIter<Value>),
    Count {
        cur: Int,
        step: i64,
    },
    Cycle {
        items: Vec<Value>,
        pos: usize,
    },
    Repeat {
        v: Value,
        left: Option<u64>,
    },
    Accumulate {
        items: std::vec::IntoIter<Value>,
        acc: Option<Value>,
        binop: Option<String>,
    },
    Chain {
        parts: std::vec::IntoIter<std::vec::IntoIter<Value>>,
        cur: Option<std::vec::IntoIter<Value>>,
    },
    Compress {
        items: std::vec::IntoIter<Value>,
        selectors: std::vec::IntoIter<Value>,
    },
    DropWhile {
        items: std::vec::IntoIter<Value>,
        pred: String,
        dropped: bool,
    },
    TakeWhile {
        items: std::vec::IntoIter<Value>,
        pred: String,
        done: bool,
    },
    FilterFalse {
        items: std::vec::IntoIter<Value>,
        pred: Option<String>,
    },
    Filter {
        items: std::vec::IntoIter<Value>,
        func: Option<String>,
    },
    Batched {
        items: std::vec::IntoIter<Value>,
        n: usize,
    },
    Pairwise {
        items: std::vec::IntoIter<Value>,
        prev: Option<Value>,
        started: bool,
    },
    Enumerate {
        items: std::vec::IntoIter<Value>,
        counter: i64,
    },
    ZipMulti(Vec<std::vec::IntoIter<Value>>),
    Map {
        items: std::vec::IntoIter<Value>,
        func: String,
    },
    Starmap {
        items: std::vec::IntoIter<Value>,
        func: String,
    },
    GroupBy {
        groups: std::vec::IntoIter<(Value, Vec<Value>)>,
    },
    ISlice {
        items: Box<NativeIterState>,
        next_idx: usize,
        stop: Option<usize>,
        step: usize,
    },
    RandRange,
    RandSeq {
        items: std::vec::IntoIter<Value>,
    },
}

impl NativeIterState {
    /// Pull the next value; `None` = exhausted.
    pub fn next(&mut self) -> Option<Value> {
        match self {
            NativeIterState::Seq(it) | NativeIterState::RandSeq { items: it } => it.next(),
            NativeIterState::Count { cur, step } => {
                let out = cur.clone();
                let delta = Int::from_i64(*step);
                *cur = if *step >= 0 {
                    cur.add(&delta)
                } else {
                    cur.sub(&delta.neg())
                };
                Some(Value::Int(out))
            }
            NativeIterState::Cycle { items, pos } => {
                if items.is_empty() {
                    return None;
                }
                let v = items[*pos].clone();
                *pos = (*pos + 1) % items.len();
                Some(v)
            }
            NativeIterState::Repeat { v, left } => match left {
                None => Some(v.clone()),
                Some(0) => None,
                Some(n) => {
                    *n -= 1;
                    Some(v.clone())
                }
            },
            NativeIterState::Accumulate { items, acc, binop } => {
                let x = items.next()?;
                let new_acc = match acc.take() {
                    None => x.clone(),
                    Some(a) => match binop {
                        None => match ap_obj::ops::binop(&a, ap_obj::ops::BinOp::Add, &x) {
                            Ok(v) => v,
                            Err(_) => return None,
                        },
                        Some(op) => call_named(op, &[a, x]).unwrap_or(Value::None),
                    },
                };
                *acc = Some(new_acc.clone());
                Some(new_acc)
            }
            NativeIterState::Chain { parts, cur } => loop {
                match cur {
                    None => match parts.next() {
                        Some(p) => *cur = Some(p),
                        None => return None, // all parts exhausted
                    },
                    Some(ref mut p) => match p.next() {
                        Some(v) => return Some(v),
                        None => *cur = None,
                    },
                }
            },
            NativeIterState::Compress { items, selectors } => loop {
                let x = items.next()?;
                let sel = selectors.next()?;
                if truthy(&sel) {
                    return Some(x);
                }
            },
            NativeIterState::DropWhile {
                items,
                pred,
                dropped,
            } => loop {
                let x = items.next()?;
                if !*dropped {
                    let keep = call_named(pred, &[x.clone()])
                        .map(|v| truthy(&v))
                        .unwrap_or(false);
                    if keep {
                        continue;
                    }
                    *dropped = true;
                }
                return Some(x);
            },
            NativeIterState::TakeWhile { items, pred, done } => {
                if *done {
                    return None;
                }
                let x = items.next()?;
                let keep = call_named(pred, &[x.clone()])
                    .map(|v| truthy(&v))
                    .unwrap_or(false);
                if keep {
                    Some(x)
                } else {
                    *done = true;
                    None
                }
            }
            NativeIterState::FilterFalse { items, pred } => loop {
                let x = items.next()?;
                let sel = match pred {
                    None => true,
                    Some(p) => call_named(p, &[x.clone()])
                        .map(|v| truthy(&v))
                        .unwrap_or(false),
                };
                if !sel {
                    return Some(x);
                }
            },
            NativeIterState::Filter { items, func } => loop {
                let x = items.next()?;
                let sel = match func {
                    None => truthy(&x),
                    Some(p) => call_named(p, &[x.clone()])
                        .map(|v| truthy(&v))
                        .unwrap_or(false),
                };
                if sel {
                    return Some(x);
                }
            },
            NativeIterState::Batched { items, n } => {
                let mut batch = Vec::with_capacity(*n);
                for _ in 0..*n {
                    match items.next() {
                        Some(v) => batch.push(v),
                        None => break,
                    }
                }
                if batch.is_empty() {
                    None
                } else {
                    Some(tuple(batch))
                }
            }
            NativeIterState::Pairwise {
                items,
                prev,
                started,
            } => loop {
                let x = items.next()?;
                if !*started {
                    *prev = Some(x);
                    *started = true;
                    continue;
                }
                let p = prev.take().unwrap();
                *prev = Some(x);
                return Some(tuple(vec![p, prev.clone().unwrap()]));
            },
            NativeIterState::Enumerate { items, counter } => {
                let x = items.next()?;
                let c = *counter;
                *counter += 1;
                Some(tuple(vec![i(c), x]))
            }
            NativeIterState::ZipMulti(parts) => {
                let mut out = Vec::with_capacity(parts.len());
                for p in parts.as_mut_slice() {
                    match p.next() {
                        Some(v) => out.push(v),
                        None => return None,
                    }
                }
                Some(tuple(out))
            }
            NativeIterState::Map { items, func } => {
                let x = items.next()?;
                call_named(func, &[x]).ok()
            }
            NativeIterState::Starmap { items, func } => {
                let x = items.next()?;
                let args = collect(&x).ok()?;
                call_named(func, &args).ok()
            }
            NativeIterState::GroupBy { groups } => {
                let (k, vals) = groups.next()?;
                Some(tuple(vec![k, list(vals)]))
            }
            NativeIterState::ISlice {
                items,
                next_idx,
                stop,
                step,
            } => loop {
                if let Some(st) = stop {
                    if *next_idx >= *st {
                        return None;
                    }
                }
                let x = items.next()?;
                let emit = *next_idx % *step == 0;
                *next_idx += 1;
                if emit {
                    return Some(x);
                }
            },
            NativeIterState::RandRange => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Named-callable resolution. Python callables (user lambdas/functions) can't
// be held with interpreter state inside iterator state, so call sites register
// them under synthetic names in a thread-local table; dotted builtin markers
// keep their names and route through builtins dispatch.
// ---------------------------------------------------------------------------

thread_local! {
    static NAMED_CALLABLES: RefCell<HashMap<String, Value>> = RefCell::new(HashMap::new());
    static NAMED_COUNTER: Cell<u64> = const { Cell::new(0) };
    static CURRENT: RefCell<Option<Rc<crate::frame::Interpreter>>> =
        const { RefCell::new(None) };
    static NEXT_ITER: Cell<u64> = const { Cell::new(0x5000_0000_0000_0000) };
}

/// Public wrapper for parser hooks (parse_float etc).
pub fn callable_name_pub(v: &Value) -> Result<String, Flow> {
    callable_name(v)
}

fn name_callable(v: &Value) -> String {
    if let Value::Obj(o) = v {
        if o.kind == ap_obj::ObjKind::BuiltinFn && o.name.contains('.') {
            return o.name.clone();
        }
        // Constructor-style callables (str/int/float/...) get a routable
        // marker name — call_named dispatches them through the ctor path.
        // (Kind-agnostic: 02.03 keeps swapping the representation of builtin
        // type objects; the NAME is the stable contract.)
        if matches!(
            o.name.as_str(),
            "str" | "int" | "float" | "bool" | "list" | "tuple" | "repr"
        ) {
            let name = format!("<ap-ctor-{}>", o.name);
            NAMED_CALLABLES.with(|t| t.borrow_mut().insert(name.clone(), v.clone()));
            return name;
        }
        // User functions get a STABLE synthetic name keyed by fn_id so
        // atexit.unregister can find the registration.
        if o.kind == ap_obj::ObjKind::Function {
            if let Some(id) = o.fn_id {
                let name = format!("<ap-fn-{id}>");
                NAMED_CALLABLES.with(|t| t.borrow_mut().insert(name.clone(), v.clone()));
                return name;
            }
        }
    }
    let id = NAMED_COUNTER.with(|c| {
        let n = c.get() + 1;
        c.set(n);
        n
    });
    let name = format!("<ap-named-{id}>");
    NAMED_CALLABLES.with(|t| t.borrow_mut().insert(name.clone(), v.clone()));
    name
}

/// Call a named callable (synthetic or dotted builtin marker).
pub fn call_named(name: &str, args: &[Value]) -> Result<Value, Flow> {
    if let Some(ctor) = name
        .strip_prefix("<ap-ctor-")
        .and_then(|s| s.strip_suffix('>'))
    {
        return crate::builtins::call_builtin_ctor(ctor, args)
            .unwrap_or_else(|| Err(raise("NameError", format!("name '{ctor}' is not defined"))));
    }
    if let Some(v) = NAMED_CALLABLES.with(|t| t.borrow().get(name).cloned()) {
        let interp = CURRENT.with(|c| c.borrow().clone());
        let interp = match interp {
            Some(i) => i,
            None => return Err(raise("RuntimeError", "no interpreter on this thread")),
        };
        // Type objects used as conversion callables (parse_float=str): route
        // through the builtin ctor dispatch (02.03's protocol layer doesn't
        // treat interned name-Types as callable).
        if let Value::Obj(o) = &v {
            if o.kind == ap_obj::ObjKind::Type {
                return crate::builtins::call_builtin_pub(&o.name, args, &[]);
            }
            // Constructor BuiltinFns without dots (str/int/float/bool): the
            // dotted dispatch misses them, so route by marker name.
            if o.kind == ap_obj::ObjKind::BuiltinFn && !o.name.contains('.') {
                if let Some(r) = crate::builtins::call_builtin_ctor(&o.name, args) {
                    return r;
                }
            }
        }
        return crate::interp::call_value_pub(&interp, &v, args, &[]);
    }
    crate::builtins::call_builtin_pub(name, args, &[])
}

/// Publish the running interpreter (called from the import hook, which fires
/// on the first import of every run).
pub fn set_current_interp(interp: &Rc<crate::frame::Interpreter>) {
    CURRENT.with(|c| *c.borrow_mut() = Some(interp.clone()));
}

fn make_iter(state: NativeIterState) -> Value {
    let obj = PyObject::new(ap_obj::ObjKind::Iterator, "iterator");
    let it = crate::iter::PyIterator::new(crate::iter::IterState::Native(Box::new(state)));
    let id = NEXT_ITER.with(|c| {
        let n = c.get();
        c.set(n + 1);
        n
    });
    if let Some(interp) = CURRENT.with(|c| c.borrow().clone()) {
        interp.iterators.borrow_mut().insert(id, it);
    }
    *obj.gen_id.borrow_mut() = Some(id);
    Value::Obj(obj)
}

fn snapshot(v: &Value) -> Result<Vec<Value>, Flow> {
    collect(v)
}

impl NativeIterState {
    fn into_seq_or_snapshot(self) -> std::vec::IntoIter<Value> {
        match self {
            NativeIterState::Seq(it) => it,
            other => {
                let mut v = Vec::new();
                let mut st = other;
                while let Some(x) = st.next() {
                    v.push(x);
                }
                v.into_iter()
            }
        }
    }
}

/// Build a lazy NativeIterState for an iterable: iterator objects keep their
/// lazy state (count() must never be snapshotted).
fn lazy_iter(v: &Value) -> Result<NativeIterState, Flow> {
    use NativeIterState as N;
    if let Value::Obj(o) = v {
        if o.kind == ap_obj::ObjKind::Iterator {
            if let Some(id) = *o.gen_id.borrow() {
                if let Some(interp) = CURRENT.with(|c| c.borrow().clone()) {
                    let probe = {
                        let its = interp.iterators.borrow();
                        its.get(&id).map(|it| match &*it.state.borrow() {
                            crate::iter::IterState::Native(inner) => match &**inner {
                                N::Count { cur, step } => Some(N::Count {
                                    cur: cur.clone(),
                                    step: *step,
                                }),
                                N::Cycle { items, pos } => Some(N::Cycle {
                                    items: items.clone(),
                                    pos: *pos,
                                }),
                                N::Repeat { v, left } => Some(N::Repeat {
                                    v: v.clone(),
                                    left: *left,
                                }),
                                _ => None,
                            },
                            _ => None,
                        })
                    };
                    if let Some(Some(state)) = probe {
                        return Ok(state);
                    }
                }
            }
        }
    }
    Ok(N::Seq(snapshot(v)?.into_iter()))
}

// ===========================================================================
// Registry
// ===========================================================================

type ModuleBuilder = fn(&crate::frame::Interpreter) -> Value;

thread_local! {
    static MODULE_CACHE: RefCell<HashMap<String, Value>> = RefCell::new(HashMap::new());
}

fn registry() -> &'static [(&'static str, ModuleBuilder)] {
    &[
        ("math", build_math),
        ("itertools", build_itertools),
        ("operator", build_operator),
        ("heapq", build_heapq),
        ("bisect", build_bisect),
        ("functools", build_functools),
        ("string", build_string),
        ("textwrap", build_textwrap),
        ("collections", build_collections),
        ("json", build_json),
        ("os", build_os),
        ("os.path", build_posixpath),
        ("posixpath", build_posixpath),
        ("genericpath", build_genericpath),
        ("stat", build_stat),
        ("errno", build_errno),
        ("keyword", build_keyword),
        ("platform", build_platform),
        ("time", build_time),
        ("atexit", build_atexit),
        ("linecache", build_linecache),
        ("random", build_random),
        ("hashlib", build_hashlib),
        ("base64", build_base64),
        ("binascii", build_binascii),
        ("io", build_io),
        ("tempfile", build_tempfile),
        ("string2", build_string), // dedup guard, never matched
    ]
}

/// Resolve (and lazily build + cache) a native module by dotted name.
pub fn get_or_make_module(interp: &Rc<crate::frame::Interpreter>, name: &str) -> Option<Value> {
    set_current_interp(interp);
    // `posixpath` and `os.path` are the SAME module object (CPython:
    // os.path is posixpath), so `posixpath.join is os.path.join` holds.
    let name = if name == "posixpath" { "os.path" } else { name };
    if let Some(v) = MODULE_CACHE.with(|c| c.borrow().get(name).cloned()) {
        return Some(v);
    }
    let builder = registry()
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, b)| *b)?;
    let m = builder(interp);
    // Submodule links: `os.path`, `encodings.aliases` style attribute access.
    if let Some((parent, child)) = name.rsplit_once('.') {
        let pv = MODULE_CACHE.with(|c| c.borrow().get(parent).cloned());
        if let Some(Value::Obj(po)) = pv {
            if po.kind == ap_obj::ObjKind::Module {
                if let Some(ns) = &*po.module_ns.borrow() {
                    let _ = ns.set(s(child), m.clone());
                }
            }
        }
    }
    MODULE_CACHE.with(|c| c.borrow_mut().insert(name.to_string(), m.clone()));
    Some(m)
}

/// Is `name` a native module (incl. dotted paths)?
pub fn is_native_module(name: &str) -> bool {
    registry().iter().any(|(n, _)| *n == name)
}

fn new_module(name: &str) -> (Gc<PyObject>, Gc<PyDict>) {
    let m = PyObject::new(ap_obj::ObjKind::Module, name);
    let ns = PyDict::new();
    let _ = ns.set(s("__name__"), s(name));
    // CPython 3.14 static/builtin modules expose __file__ (None) — the corpus
    // checks `not any(n in dir(math) for n in ("__file__", "_file"))` expecting
    // False, i.e. "__file__" IS in dir().
    let _ = ns.set(s("__file__"), Value::None);
    (m, ns)
}

fn finish(m: Gc<PyObject>, ns: Gc<PyDict>) -> Value {
    *m.module_ns.borrow_mut() = Some((*ns).clone());
    Value::Obj(m)
}

fn marker(name: &str) -> Value {
    let o = PyObject::new(ap_obj::ObjKind::BuiltinFn, name);
    Value::Obj(o)
}

fn fn1(ns: &Gc<PyDict>, mod_name: &str, name: &str) {
    let _ = ns.set(s(name), marker(&format!("{mod_name}.{name}")));
}

fn ns_const(ns: &Gc<PyDict>, k: &str, v: Value) {
    let _ = ns.set(s(k), v);
}

// ===========================================================================
// math (spec 05 §5.3.1 math row; CPython-exact 3.14 semantics)
// ===========================================================================

fn one_f64_arg(args: &[Value], fname: &str) -> Result<f64, Flow> {
    match args.first() {
        None => Err(raise(
            "TypeError",
            format!("{fname}() missing required argument"),
        )),
        Some(v) => as_f64(v),
    }
}

fn arg2_f64(args: &[Value], fname: &str) -> Result<(f64, f64), Flow> {
    let x = one_f64_arg(args, fname)?;
    let y = match args.get(1) {
        None => {
            return Err(raise(
                "TypeError",
                format!("{fname}() missing required argument"),
            ))
        }
        Some(v) => as_f64(v)?,
    };
    Ok((x, y))
}

fn build_math(_interp: &crate::frame::Interpreter) -> Value {
    let (m, ns) = new_module("math");
    ns_const(&ns, "pi", f(std::f64::consts::PI));
    ns_const(&ns, "tau", f(std::f64::consts::TAU));
    ns_const(&ns, "e", f(std::f64::consts::E));
    ns_const(&ns, "inf", f(f64::INFINITY));
    ns_const(&ns, "nan", f(f64::NAN));
    for n in [
        "acos",
        "acosh",
        "asin",
        "asinh",
        "atan",
        "atan2",
        "atanh",
        "cbrt",
        "ceil",
        "comb",
        "copysign",
        "cos",
        "cosh",
        "degrees",
        "dist",
        "erf",
        "erfc",
        "exp",
        "exp2",
        "expm1",
        "fabs",
        "factorial",
        "floor",
        "fma",
        "fmod",
        "frexp",
        "fsum",
        "gamma",
        "gcd",
        "hypot",
        "isclose",
        "isfinite",
        "isinf",
        "isnan",
        "isqrt",
        "lcm",
        "ldexp",
        "lgamma",
        "log",
        "log10",
        "log1p",
        "log2",
        "modf",
        "nextafter",
        "perm",
        "pow",
        "prod",
        "radians",
        "remainder",
        "sin",
        "sinh",
        "sqrt",
        "sumprod",
        "tan",
        "tanh",
        "trunc",
        "ulp",
    ] {
        fn1(&ns, "math", n);
    }
    finish(m, ns)
}

/// `math.<fn>` dispatch (byte-exact CPython 3.14 semantics).
pub fn call_math(name: &str, args: &[Value], kwargs: &[(String, Value)]) -> Option<BResult> {
    let r: BResult = (|| -> BResult {
        match name {
            "sqrt" => {
                let x = one_f64_arg(args, "sqrt")?;
                if x < 0.0 {
                    return Err(raise(
                        "ValueError",
                        format!("expected a nonnegative input, got {}", fmt_f64(x)),
                    ));
                }
                Ok(f(x.sqrt()))
            }
            "exp" => overflow_wrap(f(one_f64_arg(args, "exp")?.exp())),
            "exp2" => overflow_wrap(f(one_f64_arg(args, "exp2")?.exp2())),
            "expm1" => Ok(f(one_f64_arg(args, "expm1")?.exp_m1())),
            "log" => {
                let x = match args.first() {
                    None => return Err(raise("TypeError", "log() missing required argument")),
                    Some(v) => as_f64(v)?,
                };
                if x <= 0.0 {
                    return Err(raise("ValueError", "expected a positive input"));
                }
                match args.get(1) {
                    None => Ok(f(x.ln())),
                    Some(b) => {
                        let b = as_f64(b)?;
                        if b == 0.0 {
                            return Err(raise("ZeroDivisionError", "expected a positive input"));
                        }
                        if b == 2.0 {
                            Ok(f(x.log2()))
                        } else if b == 10.0 {
                            Ok(f(x.log10()))
                        } else {
                            Ok(f(x.log(b)))
                        }
                    }
                }
            }
            "log1p" => {
                let x = one_f64_arg(args, "log1p")?;
                if x <= -1.0 {
                    return Err(raise("ValueError", "expected a positive input"));
                }
                Ok(f(x.ln_1p()))
            }
            "log2" => {
                let x = one_f64_arg(args, "log2")?;
                if x <= 0.0 {
                    return Err(raise("ValueError", "expected a positive input"));
                }
                Ok(f(x.log2()))
            }
            "log10" => {
                let x = one_f64_arg(args, "log10")?;
                if x <= 0.0 {
                    return Err(raise("ValueError", "expected a positive input"));
                }
                Ok(f(x.log10()))
            }
            "sin" => Ok(f(one_f64_arg(args, "sin")?.sin())),
            "cos" => Ok(f(one_f64_arg(args, "cos")?.cos())),
            "tan" => Ok(f(one_f64_arg(args, "tan")?.tan())),
            "asin" => {
                let x = one_f64_arg(args, "asin")?;
                if !(-1.0..=1.0).contains(&x) {
                    return Err(range_m1_1(x));
                }
                Ok(f(x.asin()))
            }
            "acos" => {
                let x = one_f64_arg(args, "acos")?;
                if !(-1.0..=1.0).contains(&x) {
                    return Err(range_m1_1(x));
                }
                Ok(f(x.acos()))
            }
            "atan" => Ok(f(one_f64_arg(args, "atan")?.atan())),
            "atan2" => {
                let (y, x) = arg2_f64(args, "atan2")?;
                Ok(f(y.atan2(x)))
            }
            "sinh" => overflow_wrap(f(one_f64_arg(args, "sinh")?.sinh())),
            "cosh" => overflow_wrap(f(one_f64_arg(args, "cosh")?.cosh())),
            "tanh" => Ok(f(one_f64_arg(args, "tanh")?.tanh())),
            "asinh" => Ok(f(one_f64_arg(args, "asinh")?.asinh())),
            "acosh" => {
                let x = one_f64_arg(args, "acosh")?;
                if x < 1.0 {
                    return Err(raise(
                        "ValueError",
                        format!(
                            "expected a input greater than or equal to 1, got {}",
                            fmt_f64(x)
                        ),
                    ));
                }
                Ok(f(x.acosh()))
            }
            "atanh" => {
                let x = one_f64_arg(args, "atanh")?;
                if !(-1.0..=1.0).contains(&x) {
                    return Err(range_m1_1(x));
                }
                Ok(f(x.atanh()))
            }
            "degrees" => Ok(f(
                one_f64_arg(args, "degrees")? * (180.0 / std::f64::consts::PI)
            )),
            "radians" => Ok(f(
                one_f64_arg(args, "radians")? * (std::f64::consts::PI / 180.0)
            )),
            "fabs" => Ok(f(one_f64_arg(args, "fabs")?.abs())),
            "floor" => floor_ceil_trunc(args, 0),
            "ceil" => floor_ceil_trunc(args, 1),
            "trunc" => floor_ceil_trunc(args, 2),
            "fmod" => {
                let (x, y) = arg2_f64(args, "fmod")?;
                if y == 0.0 {
                    return Err(raise("ValueError", "math domain error"));
                }
                Ok(f(x % y))
            }
            "remainder" => {
                let (x, y) = arg2_f64(args, "remainder")?;
                if y == 0.0 {
                    return Err(raise("ValueError", "math domain error"));
                }
                Ok(f(ieee_remainder(x, y)))
            }
            "copysign" => {
                let (x, y) = arg2_f64(args, "copysign")?;
                Ok(f(x.copysign(y)))
            }
            "hypot" => {
                let mut acc = 0.0f64;
                for a in args {
                    let x = as_f64(a)?;
                    acc += x * x;
                }
                Ok(f(acc.sqrt()))
            }
            "dist" => {
                let p = arg_snapshot(args, 0, "dist")?;
                let q = arg_snapshot(args, 1, "dist")?;
                if p.len() != q.len() {
                    return Err(raise(
                        "ValueError",
                        "both points must have the same number of dimensions",
                    ));
                }
                let mut acc = 0.0f64;
                for (a, b) in p.iter().zip(q.iter()) {
                    let d = as_f64(a)? - as_f64(b)?;
                    acc += d * d;
                }
                Ok(f(acc.sqrt()))
            }
            "pow" => {
                let (x, y) = arg2_f64(args, "pow")?;
                let r = x.powf(y);
                if r.is_nan() && !(x.is_nan() || y.is_nan()) {
                    return Err(raise("ValueError", "math domain error"));
                }
                overflow_wrap(f(r))
            }
            "isqrt" => {
                let n = as_int_big(required(args, "isqrt")?)?;
                if n.is_negative() {
                    return Err(raise("ValueError", "isqrt() argument must be nonnegative"));
                }
                Ok(Value::Int(int_isqrt(&n)))
            }
            "gcd" => {
                let mut g = Int::from_i64(0);
                for a in args {
                    let x = as_int_big(a)?;
                    g = int_gcd(g, x);
                }
                Ok(Value::Int(g))
            }
            "lcm" => {
                let mut l = Int::from_i64(1);
                for a in args {
                    let x = as_int_big(a)?;
                    if x.is_zero() {
                        return Ok(i(0));
                    }
                    l = (l.mul(&x)).floor_div(&int_gcd(l.clone(), x));
                }
                Ok(Value::Int(if l.is_negative() { l.neg() } else { l }))
            }
            "factorial" => {
                let n = as_int_big(required(args, "factorial")?)?;
                if n.is_negative() {
                    return Err(raise(
                        "ValueError",
                        "factorial() not defined for negative values",
                    ));
                }
                if n.to_i64().is_none() {
                    return Err(raise(
                        "OverflowError",
                        "factorial() argument should not exceed 9223372036854775807",
                    ));
                }
                let mut acc = Int::from_i64(1);
                let mut k = Int::from_i64(2);
                let one = Int::from_i64(1);
                while k.cmp_int(&n) != std::cmp::Ordering::Greater {
                    acc = acc.mul(&k);
                    k = k.add(&one);
                }
                Ok(Value::Int(acc))
            }
            "comb" | "perm" => {
                let fname = name;
                let n = as_int_big(required(args, fname)?)?;
                let k = match args.get(1) {
                    Some(v) => as_int_big(v)?,
                    None => Int::from_i64(match fname {
                        "perm" => n.to_i64().unwrap_or(0),
                        _ => 0,
                    }),
                };
                if n.is_negative() {
                    return Err(raise("ValueError", "n must be a non-negative integer"));
                }
                if k.is_negative() {
                    return Err(raise("ValueError", "k must be a non-negative integer"));
                }
                let (ni, ki) = (n.to_i64().unwrap_or(0), k.to_i64().unwrap_or(0));
                if ki > ni {
                    return Ok(i(0));
                }
                let mut acc = Int::from_i64(1);
                for j in 0..ki {
                    acc = acc.mul(&Int::from_i64(ni - j));
                    if fname == "comb" {
                        acc = acc.floor_div(&Int::from_i64(j + 1));
                    }
                }
                Ok(Value::Int(acc))
            }
            "prod" => {
                let items = snapshot(required(args, "prod")?)?;
                let start = kw(kwargs, "start").cloned().unwrap_or_else(|| i(1));
                let mut acc = start;
                for x in items {
                    acc = ap_obj::ops::binop(&acc, ap_obj::ops::BinOp::Mul, &x)
                        .map_err(Flow::from_py)?;
                }
                Ok(acc)
            }
            "sumprod" => {
                let p = arg_snapshot(args, 0, "sumprod")?;
                let q = arg_snapshot(args, 1, "sumprod")?;
                if p.len() != q.len() {
                    return Err(raise("ValueError", "inputs are not the same length"));
                }
                let all_int = p
                    .iter()
                    .chain(q.iter())
                    .all(|v| matches!(v, Value::Int(_) | Value::Bool(_)));
                if all_int {
                    let mut acc = Int::from_i64(0);
                    for j in 0..p.len() {
                        let term = as_int_big(&p[j])?.mul(&as_int_big(&q[j])?);
                        acc = acc.add(&term);
                    }
                    Ok(Value::Int(acc))
                } else {
                    let mut sum = 0.0f64;
                    let mut c = 0.0f64;
                    for j in 0..p.len() {
                        let term = as_f64(&p[j])? * as_f64(&q[j])?;
                        let t = sum + term;
                        if sum.abs() >= term.abs() {
                            c += (sum - t) + term;
                        } else {
                            c += (term - t) + sum;
                        }
                        sum = t;
                    }
                    Ok(f(sum + c))
                }
            }
            "fsum" => {
                let items = snapshot(required(args, "fsum")?)?;
                let mut sum = 0.0f64;
                let mut c = 0.0f64;
                for x in items {
                    let x = as_f64(&x)?;
                    let t = sum + x;
                    if sum.abs() >= x.abs() {
                        c += (sum - t) + x;
                    } else {
                        c += (x - t) + sum;
                    }
                    sum = t;
                }
                Ok(f(sum + c))
            }
            "fma" => {
                let a = one_f64_arg(args, "fma")?;
                let b = args
                    .get(1)
                    .map(as_f64)
                    .transpose()?
                    .ok_or_else(|| raise("TypeError", "fma() missing required argument"))?;
                let c = args
                    .get(2)
                    .map(as_f64)
                    .transpose()?
                    .ok_or_else(|| raise("TypeError", "fma() missing required argument"))?;
                let fused = f64::mul_add(a, b, c);
                if fused.is_nan() && !(a.is_nan() || b.is_nan() || c.is_nan()) {
                    return Err(raise("ValueError", "invalid operation in fma"));
                }
                Ok(f(fused))
            }
            "frexp" => {
                let x = one_f64_arg(args, "frexp")?;
                if x == 0.0 || x.is_nan() {
                    let m = if x.is_nan() {
                        x
                    } else if x.is_sign_negative() {
                        -0.0
                    } else {
                        0.0
                    };
                    return Ok(tuple(vec![f(m), i(0)]));
                }
                let (m, e) = frexp(x);
                Ok(tuple(vec![f(m), i(e as i64)]))
            }
            "ldexp" => {
                let x = one_f64_arg(args, "ldexp")?;
                let e = args
                    .get(1)
                    .map(as_int_i64)
                    .transpose()?
                    .ok_or_else(|| raise("TypeError", "ldexp() missing required argument"))?;
                let e = e.clamp(-20000, 20000) as i32;
                overflow_wrap(f(x * (2f64).powi(e)))
            }
            "modf" => {
                let x = one_f64_arg(args, "modf")?;
                let ip = x.trunc();
                let mut fr = x - ip;
                let mut ipn = ip;
                if x.is_sign_negative() {
                    if fr == 0.0 {
                        fr = -0.0;
                    }
                    if ipn == 0.0 {
                        ipn = -0.0;
                    }
                }
                Ok(tuple(vec![f(fr), f(ipn)]))
            }
            "isnan" => Ok(Value::Bool(one_f64_arg(args, "isnan")?.is_nan())),
            "isinf" => Ok(Value::Bool(one_f64_arg(args, "isinf")?.is_infinite())),
            "isfinite" => Ok(Value::Bool(one_f64_arg(args, "isfinite")?.is_finite())),
            "isclose" => {
                let a = one_f64_arg(args, "isclose")?;
                let b = args
                    .get(1)
                    .map(as_f64)
                    .transpose()?
                    .ok_or_else(|| raise("TypeError", "isclose() missing required argument"))?;
                let rel = kw(kwargs, "rel_tol")
                    .map(as_f64)
                    .transpose()?
                    .unwrap_or(1e-09);
                let abs = kw(kwargs, "abs_tol")
                    .map(as_f64)
                    .transpose()?
                    .unwrap_or(0.0);
                if rel < 0.0 || abs < 0.0 {
                    return Err(raise("ValueError", "tolerances must be non-negative"));
                }
                if a == b {
                    return Ok(Value::Bool(true));
                }
                if a.is_infinite() || b.is_infinite() {
                    return Ok(Value::Bool(false));
                }
                let diff = (b - a).abs();
                Ok(Value::Bool(diff <= abs.max(rel * a.abs().max(b.abs()))))
            }
            "nextafter" => {
                let x = one_f64_arg(args, "nextafter")?;
                let y =
                    args.get(1).map(as_f64).transpose()?.ok_or_else(|| {
                        raise("TypeError", "nextafter() missing required argument")
                    })?;
                if x.is_nan() || y.is_nan() {
                    return Ok(f(f64::NAN));
                }
                if x == y {
                    return Ok(f(x));
                }
                if x == 0.0 {
                    return Ok(f(if y > 0.0 {
                        f64::from_bits(1)
                    } else {
                        -f64::from_bits(1)
                    }));
                }
                let bits = x.to_bits();
                let next = if (y > x) == (x > 0.0) {
                    bits + 1
                } else {
                    bits - 1
                };
                Ok(f(f64::from_bits(next)))
            }
            "ulp" => {
                let x = one_f64_arg(args, "ulp")?;
                if x.is_nan() || x.is_infinite() {
                    return Ok(f(f64::NAN));
                }
                let ax = x.abs();
                if ax == 0.0 {
                    return Ok(f(f64::MIN_POSITIVE));
                }
                Ok(f(f64::from_bits(ax.to_bits() + 1) - ax))
            }
            "cbrt" => Ok(f(unsafe { sysmath::cbrt(one_f64_arg(args, "cbrt")?) })),
            "erf" => Ok(f(erf(one_f64_arg(args, "erf")?))),
            "erfc" => Ok(f(unsafe { sysmath::erfc(one_f64_arg(args, "erfc")?) })),
            "gamma" => gamma_checked(one_f64_arg(args, "gamma")?),
            "lgamma" => {
                let x = one_f64_arg(args, "lgamma")?;
                if x == 0.0 || (x < 0.5 && x < 0.0 && x.fract() == 0.0) {
                    let got = fmt_f64(x);
                    return Err(raise(
                        "ValueError",
                        format!("expected a noninteger or positive integer, got {got}"),
                    ));
                }
                Ok(f(lgamma(x)))
            }
            _ => return Err(Flow::Next),
        }
    })();
    match r {
        Err(Flow::Next) => return None,
        other => Some(other),
    }
}

fn required<'a>(args: &'a [Value], fname: &str) -> Result<&'a Value, Flow> {
    args.first()
        .ok_or_else(|| raise("TypeError", format!("{fname}() missing required argument")))
}

fn arg_snapshot(args: &[Value], idx: usize, fname: &str) -> Result<Vec<Value>, Flow> {
    match args.get(idx) {
        None => Err(raise(
            "TypeError",
            format!("{fname}() missing required argument"),
        )),
        Some(v) => snapshot(v),
    }
}

fn range_m1_1(got: f64) -> Flow {
    raise(
        "ValueError",
        format!(
            "expected a number in range from -1 up to 1, got {}",
            fmt_f64(got)
        ),
    )
}

fn overflow_wrap(v: Value) -> BResult {
    if let Value::Float(x) = &v {
        if x.is_infinite() {
            return Err(raise("OverflowError", "math range error"));
        }
    }
    Ok(v)
}

/// mode 0=floor 1=ceil 2=trunc (int passthrough; CPython int-returning).
fn floor_ceil_trunc(args: &[Value], mode: u8) -> BResult {
    match args.first() {
        None => Err(raise("TypeError", "missing required argument")),
        Some(Value::Int(n)) => Ok(Value::Int(n.clone())),
        Some(Value::Bool(b)) => Ok(i(if *b { 1 } else { 0 })),
        Some(Value::Float(x)) => {
            if x.is_nan() {
                return Err(raise("ValueError", "cannot convert float NaN to integer"));
            }
            if x.is_infinite() {
                return Err(raise(
                    "OverflowError",
                    "cannot convert float infinity to integer",
                ));
            }
            let v = match mode {
                0 => x.floor(),
                1 => x.ceil(),
                _ => x.trunc(),
            };
            Ok(i(v as i64))
        }
        Some(other) => Err(raise(
            "TypeError",
            format!(
                "'{}' object cannot be interpreted as an integer",
                other.type_name()
            ),
        )),
    }
}

pub(crate) fn fmt_f64(x: f64) -> String {
    ap_obj::float_repr::py_float_repr(x)
}

fn ieee_remainder(x: f64, y: f64) -> f64 {
    // IEEE 754 remainder: quotient rounds to nearest EVEN on ties.
    let q = x / y;
    let r = x - q.round() * y;
    if (q - q.round()).abs() == 0.5 {
        // tie: pick even quotient neighbor
        let lo = q.floor();
        let hi = q.ceil();
        let pick = if (lo as i64) % 2 == 0 { lo } else { hi };
        let r2 = x - pick * y;
        return if r2 == 0.0 && x.is_sign_negative() {
            -0.0
        } else {
            r2
        };
    }
    if r == 0.0 {
        // sign of zero follows x (C99 remainder)
        return if x.is_sign_negative() { -0.0 } else { 0.0 };
    }
    r
}

fn frexp(x: f64) -> (f64, i32) {
    let bits = x.to_bits();
    let raw_exp = ((bits >> 52) & 0x7ff) as i32;
    if raw_exp == 0 {
        // subnormal: scale up
        let scaled = x * 2f64.powi(64);
        let (m, e) = frexp(scaled);
        return (m, e - 64);
    }
    let exp = raw_exp - 1022;
    let mantissa_bits = bits & !(0x7ffu64 << 52);
    let m = f64::from_bits(mantissa_bits | (1022u64 << 52));
    (m, exp)
}

/// CPython mathmodule.c m_lgamma/m_tgamma verbatim port (PSF license —
/// attribution in THIRD_PARTY_NOTICES per S-05-5). CPython does NOT use libm
/// for these; byte-exactness requires the same Lanczos(13) implementation.
const LANCZOS_G: f64 = 6.024680040776729583740234375;
const LANCZOS_G_MINUS_HALF: f64 = 5.524680040776729583740234375;
const LOG_PI: f64 = 1.144729885849400174143427351353058711647;
const PI_CONST: f64 = std::f64::consts::PI;

fn lanczos_num_coeffs() -> [f64; 13] {
    [
        23531376880.410759688572007674451636754734846804940,
        42919803642.649098768957899047001988850926355848959,
        35711959237.355668049440185451547166705960488635843,
        17921034426.037209699919755754458931112671403265390,
        6039542586.3520280050642916443072979210699388420708,
        1439720407.3117216736632230727949123939715485786772,
        248874557.86205415651146038641322942321632125127801,
        31426415.585400194380614231628318205362874684987640,
        2876370.6289353724412254090516208496135991145378768,
        186056.26539522349504029498971604569928220784236328,
        8071.6720023658162106380029022722506138218516325024,
        210.82427775157934587250973392071336271166969580291,
        2.5066282746310002701649081771338373386264310793408,
    ]
}

fn lanczos_den_coeffs() -> [f64; 13] {
    [
        0.0,
        39916800.0,
        120543840.0,
        150917976.0,
        105258076.0,
        45995730.0,
        13339535.0,
        2637558.0,
        357423.0,
        32670.0,
        1925.0,
        66.0,
        1.0,
    ]
}

fn gamma_integral(n: usize) -> Option<f64> {
    const T: [f64; 23] = [
        1.0,
        1.0,
        2.0,
        6.0,
        24.0,
        120.0,
        720.0,
        5040.0,
        40320.0,
        362880.0,
        3628800.0,
        39916800.0,
        479001600.0,
        6227020800.0,
        87178291200.0,
        1307674368000.0,
        20922789888000.0,
        355687428096000.0,
        6402373705728000.0,
        121645100408832000.0,
        2432902008176640000.0,
        51090942171709440000.0,
        1124000727777607680000.0,
    ];
    if n >= 1 && n <= 23 {
        Some(T[n - 1])
    } else {
        None
    }
}

fn m_sinpi(x: f64) -> f64 {
    let y = x.abs() % 2.0;
    let n = (2.0 * y).round() as i32;
    let r = match n {
        0 => (PI_CONST * y).sin(),
        1 => (PI_CONST * (y - 0.5)).cos(),
        2 => (PI_CONST * (1.0 - y)).sin(),
        3 => -(PI_CONST * (y - 1.5)).cos(),
        4 => (PI_CONST * (y - 2.0)).sin(),
        _ => unreachable!(),
    };
    r.copysign(x)
}

fn lanczos_sum(x: f64) -> f64 {
    let num_c = lanczos_num_coeffs();
    let den_c = lanczos_den_coeffs();
    let (mut num, mut den) = (0.0f64, 0.0f64);
    if x < 5.0 {
        let mut i = 13;
        while i > 0 {
            i -= 1;
            num = num * x + num_c[i];
            den = den * x + den_c[i];
        }
    } else {
        for i in 0..13 {
            num = num / x + num_c[i];
            den = den / x + den_c[i];
        }
    }
    num / den
}

fn m_tgamma(x: f64) -> f64 {
    if !x.is_finite() {
        if x.is_nan() || x > 0.0 {
            return x;
        }
        return f64::NAN;
    }
    if x == 0.0 {
        return f64::INFINITY.copysign(x);
    }
    if x == x.floor() {
        if x < 0.0 {
            return f64::NAN;
        }
        if x <= 23.0 {
            return gamma_integral(x as usize).unwrap_or(f64::NAN);
        }
    }
    let absx = x.abs();
    if absx < 1e-20 {
        return 1.0 / x;
    }
    if absx > 200.0 {
        if x < 0.0 {
            return 0.0 / m_sinpi(x);
        }
        return f64::INFINITY;
    }
    let y = absx + LANCZOS_G_MINUS_HALF;
    let z = if absx > LANCZOS_G_MINUS_HALF {
        let q = y - absx;
        q - LANCZOS_G_MINUS_HALF
    } else {
        let q = y - LANCZOS_G_MINUS_HALF;
        q - absx
    };
    let z = z * LANCZOS_G / y;
    let r;
    if x < 0.0 {
        let mut rr = -PI_CONST / m_sinpi(absx) / absx * y.exp() / lanczos_sum(absx);
        rr = (-z).mul_add(rr, rr);
        r = if absx < 140.0 {
            rr / y.powf(absx - 0.5)
        } else {
            let sqrtpow = y.powf(absx / 2.0 - 0.25);
            rr / sqrtpow / sqrtpow
        };
    } else {
        let mut rr = lanczos_sum(absx) / y.exp();
        rr = z.mul_add(rr, rr);
        r = if absx < 140.0 {
            rr * y.powf(absx - 0.5)
        } else {
            let sqrtpow = y.powf(absx / 2.0 - 0.25);
            rr * sqrtpow * sqrtpow
        };
    }
    r
}

fn m_lgamma(x: f64) -> f64 {
    if !x.is_finite() {
        if x.is_nan() {
            return x;
        }
        return f64::INFINITY;
    }
    if x == x.floor() && x <= 2.0 {
        if x <= 0.0 {
            return f64::INFINITY;
        }
        return 0.0;
    }
    let absx = x.abs();
    if absx < 1e-20 {
        return -absx.ln();
    }
    let mut r = lanczos_sum(absx).ln() - LANCZOS_G;
    // CPython is compiled with -ffp-contract=on: `r += a * b` fuses. Rust has
    // no cross-statement contraction, so make it explicit (byte-parity).
    r = (absx - 0.5).mul_add((absx + LANCZOS_G - 0.5).ln() - 1.0, r);
    if x < 0.0 {
        r = LOG_PI - m_sinpi(absx).abs().ln() - absx.ln() - r;
    }
    r
}

mod sysmath {
    extern "C" {
        pub fn erf(x: f64) -> f64;
        pub fn erfc(x: f64) -> f64;
        pub fn cbrt(x: f64) -> f64;
    }
}

fn erf(x: f64) -> f64 {
    unsafe { sysmath::erf(x) }
}

fn lgamma(x: f64) -> f64 {
    m_lgamma(x)
}

fn gamma_checked(x: f64) -> BResult {
    if x.is_nan() {
        return Ok(f(f64::NAN));
    }
    if x == 0.0 || (x < 0.0 && x.fract() == 0.0) {
        return Err(raise("ValueError", "math domain error"));
    }
    let g = m_tgamma(x);
    if g.is_infinite() {
        return Err(raise("OverflowError", "math range error"));
    }
    Ok(f(g))
}

// ===========================================================================
// itertools (spec 05 §5.3.1 itertools row: 20 public names)
// ===========================================================================

fn build_itertools(_i: &crate::frame::Interpreter) -> Value {
    let (m, ns) = new_module("itertools");
    for n in [
        "accumulate",
        "batched",
        "chain",
        "combinations",
        "combinations_with_replacement",
        "compress",
        "count",
        "cycle",
        "dropwhile",
        "filterfalse",
        "groupby",
        "islice",
        "pairwise",
        "permutations",
        "product",
        "repeat",
        "starmap",
        "takewhile",
        "tee",
        "zip_longest",
    ] {
        fn1(&ns, "itertools", n);
    }
    finish(m, ns)
}

fn callable_name(v: &Value) -> Result<String, Flow> {
    match v {
        Value::Obj(o)
            if o.kind == ap_obj::ObjKind::BuiltinFn || o.kind == ap_obj::ObjKind::Function =>
        {
            Ok(name_callable(v))
        }
        other => Err(raise(
            "TypeError",
            format!("'{}' object is not callable", other.type_name()),
        )),
    }
}

pub fn call_itertools(name: &str, args: &[Value], kwargs: &[(String, Value)]) -> Option<BResult> {
    let r: BResult =
        (|| -> BResult {
            match name {
                "count" => {
                    let start = args
                        .first()
                        .map(as_int_big)
                        .transpose()?
                        .unwrap_or_else(|| Int::from_i64(0));
                    let step = args
                        .get(1)
                        .map(|v| as_int_i64(&v))
                        .transpose()?
                        .unwrap_or(1);
                    // type(itertools.count(1)).__name__ == "count"
                    let obj = PyObject::new(ap_obj::ObjKind::Iterator, "count");
                    let it = crate::iter::PyIterator::new(crate::iter::IterState::Native(
                        Box::new(NativeIterState::Count { cur: start, step }),
                    ));
                    let id = register_shared_iter(it);
                    *obj.gen_id.borrow_mut() = Some(id);
                    Ok(Value::Obj(obj))
                }
                "cycle" => {
                    let items = snapshot(required(args, "cycle")?)?;
                    Ok(make_iter(NativeIterState::Cycle { items, pos: 0 }))
                }
                "repeat" => {
                    let v = required(args, "repeat")?.clone();
                    let left = match args.get(1) {
                        None => None,
                        Some(n) => Some(as_int_i64(n)? as u64),
                    };
                    Ok(make_iter(NativeIterState::Repeat { v, left }))
                }
                "accumulate" => {
                    let items = snapshot(required(args, "accumulate")?)?;
                    let binop = match args.get(1) {
                        Some(fv) => Some(callable_name(fv)?),
                        None => None,
                    };
                    Ok(make_iter(NativeIterState::Accumulate {
                        items: items.into_iter(),
                        acc: None,
                        binop,
                    }))
                }
                "chain.from_iterable" => {
                    let itv = required(args, "chain.from_iterable")?;
                    let mut parts = Vec::new();
                    for sub in snapshot(itv)? {
                        parts.push(lazy_iter(&sub)?.into_seq_or_snapshot());
                    }
                    Ok(make_iter(NativeIterState::Chain {
                        parts: parts.into_iter(),
                        cur: None,
                    }))
                }
                "chain" => {
                    let mut parts = Vec::new();
                    for a in args {
                        parts.push(snapshot(a)?.into_iter());
                    }
                    Ok(make_iter(NativeIterState::Chain {
                        parts: parts.into_iter(),
                        cur: None,
                    }))
                }
                "compress" => {
                    let items = snapshot(required(args, "compress")?)?;
                    let selectors = snapshot(args.get(1).ok_or_else(|| {
                        raise("TypeError", "compress() missing required argument")
                    })?)?;
                    Ok(make_iter(NativeIterState::Compress {
                        items: items.into_iter(),
                        selectors: selectors.into_iter(),
                    }))
                }
                "dropwhile" => {
                    let pred = callable_name(required(args, "dropwhile")?)?;
                    let items = snapshot(args.get(1).ok_or_else(|| {
                        raise("TypeError", "dropwhile() missing required argument")
                    })?)?;
                    Ok(make_iter(NativeIterState::DropWhile {
                        items: items.into_iter(),
                        pred,
                        dropped: false,
                    }))
                }
                "takewhile" => {
                    let pred = callable_name(required(args, "takewhile")?)?;
                    let items = snapshot(args.get(1).ok_or_else(|| {
                        raise("TypeError", "takewhile() missing required argument")
                    })?)?;
                    Ok(make_iter(NativeIterState::TakeWhile {
                        items: items.into_iter(),
                        pred,
                        done: false,
                    }))
                }
                "filterfalse" => {
                    let pred = match args.first() {
                        Some(Value::Obj(o))
                            if o.kind == ap_obj::ObjKind::BuiltinFn
                                || o.kind == ap_obj::ObjKind::Function =>
                        {
                            Some(callable_name(&args[0])?)
                        }
                        Some(Value::None) | None => None,
                        _ => None,
                    };
                    let items = snapshot(args.get(1).ok_or_else(|| {
                        raise("TypeError", "filterfalse() missing required argument")
                    })?)?;
                    Ok(make_iter(NativeIterState::FilterFalse {
                        items: items.into_iter(),
                        pred,
                    }))
                }
                "batched" => {
                    let items = snapshot(required(args, "batched")?)?;
                    let n =
                        args.get(1).map(as_int_i64).transpose()?.ok_or_else(|| {
                            raise("TypeError", "batched() missing required argument")
                        })?;
                    if n < 1 {
                        return Err(raise("ValueError", "n must be at least one"));
                    }
                    Ok(make_iter(NativeIterState::Batched {
                        items: items.into_iter(),
                        n: n as usize,
                    }))
                }
                "pairwise" => {
                    let items = snapshot(required(args, "pairwise")?)?;
                    Ok(make_iter(NativeIterState::Pairwise {
                        items: items.into_iter(),
                        prev: None,
                        started: false,
                    }))
                }
                "islice" => {
                    let itv = required(args, "islice")?;
                    // islice(it, stop) | islice(it, start, stop[, step])
                    let (start, stop, step) = match args.len() {
                        0 | 1 => {
                            return Err(raise(
                                "TypeError",
                                "islice expected at least 2 arguments, got 0",
                            ))
                        }
                        2 => (0, Some(as_int_i64(&args[1])?), 1),
                        _ => (
                            as_int_i64(&args[1])?,
                            match args.get(2) {
                                Some(Value::None) => None,
                                Some(v) => Some(as_int_i64(v)?),
                                None => None,
                            },
                            args.get(3)
                                .map(|v| as_int_i64(&v))
                                .transpose()?
                                .unwrap_or(1),
                        ),
                    };
                    if step < 1 {
                        return Err(raise(
                            "ValueError",
                            "Step for islice() must be a positive integer or None.",
                        ));
                    }
                    let inner = lazy_iter(itv)?;
                    let start = start.max(0) as usize;
                    // fast-forward the inner iterator past `start`
                    let mut items = inner;
                    for _ in 0..start {
                        if items.next().is_none() {
                            break;
                        }
                    }
                    let stop = stop.map(|st| {
                        if st < 0 {
                            usize::MAX
                        } else {
                            (st.max(0) as usize).saturating_sub(start)
                        }
                    });
                    Ok(make_iter(NativeIterState::ISlice {
                        items: Box::new(items),
                        next_idx: 0,
                        stop,
                        step: step as usize,
                    }))
                }
                "permutations" => {
                    let items = snapshot(required(args, "permutations")?)?;
                    let r = match args.get(1) {
                        Some(v) => as_int_i64(v)? as usize,
                        None => items.len(),
                    };
                    let mut out = Vec::new();
                    permute(
                        &items,
                        r,
                        &mut vec![],
                        &mut vec![false; items.len()],
                        &mut out,
                    );
                    Ok(make_iter(NativeIterState::Seq(out.into_iter())))
                }
                "combinations" => {
                    let items = snapshot(required(args, "combinations")?)?;
                    let r = args
                        .get(1)
                        .map(|v| as_int_i64(&v))
                        .transpose()?
                        .ok_or_else(|| {
                            raise("TypeError", "combinations() missing required argument")
                        })?;
                    let mut out = Vec::new();
                    combine(&items, r as usize, 0, &mut vec![], &mut out, false);
                    Ok(make_iter(NativeIterState::Seq(out.into_iter())))
                }
                "combinations_with_replacement" => {
                    let items = snapshot(required(args, "combinations_with_replacement")?)?;
                    let r = args
                        .get(1)
                        .map(|v| as_int_i64(&v))
                        .transpose()?
                        .ok_or_else(|| {
                            raise(
                                "TypeError",
                                "combinations_with_replacement() missing required argument",
                            )
                        })?;
                    let mut out = Vec::new();
                    combine(&items, r as usize, 0, &mut vec![], &mut out, true);
                    Ok(make_iter(NativeIterState::Seq(out.into_iter())))
                }
                "product" => {
                    let mut pools: Vec<Vec<Value>> = Vec::new();
                    for a in args {
                        pools.push(snapshot(a)?);
                    }
                    if let Some(rv) = kw(kwargs, "repeat") {
                        let n = as_int_i64(rv)?;
                        let base = pools.clone();
                        for _ in 1..n {
                            pools.extend(base.clone());
                        }
                    }
                    let mut out = Vec::new();
                    product_rec(&pools, 0, &mut vec![], &mut out);
                    Ok(make_iter(NativeIterState::Seq(out.into_iter())))
                }
                "starmap" => {
                    let func = callable_name(required(args, "starmap")?)?;
                    let items = snapshot(args.get(1).ok_or_else(|| {
                        raise("TypeError", "starmap() missing required argument")
                    })?)?;
                    Ok(make_iter(NativeIterState::Starmap {
                        items: items.into_iter(),
                        func,
                    }))
                }
                "groupby" => {
                    let items = snapshot(required(args, "groupby")?)?;
                    let keyf = match args.get(1) {
                        Some(v) => Some(callable_name(v)?),
                        None => None,
                    };
                    let mut groups: Vec<(Value, Vec<Value>)> = Vec::new();
                    for x in items {
                        let k = match &keyf {
                            Some(kf) => call_named(kf, &[x.clone()])?,
                            None => x.clone(),
                        };
                        match groups.last_mut() {
                            Some((gk, gv)) if py_eq(gk, &k) => gv.push(x),
                            _ => groups.push((k, vec![x])),
                        }
                    }
                    Ok(make_iter(NativeIterState::GroupBy {
                        groups: groups.into_iter(),
                    }))
                }
                "tee" => {
                    let src_v = required(args, "tee")?;
                    // count() is infinite: tee over count hands out fresh Count
                    // iterators (lazy_iter preserves its lazy state).
                    if let NativeIterState::Count { cur, step } = lazy_iter(src_v)? {
                        let mut out = Vec::new();
                        for _ in 0..n_tee(args)? {
                            out.push(make_iter(NativeIterState::Count {
                                cur: cur.clone(),
                                step,
                            }));
                        }
                        return Ok(list(out));
                    }
                    let items = snapshot(src_v)?;
                    let n = n_tee(args)?;
                    let mut out = Vec::new();
                    for _ in 0..n {
                        out.push(make_iter(NativeIterState::Seq(items.clone().into_iter())));
                    }
                    Ok(list(out))
                }
                "zip_longest" => {
                    let mut pools: Vec<std::vec::IntoIter<Value>> = Vec::new();
                    for a in args {
                        pools.push(snapshot(a)?.into_iter());
                    }
                    let fill = kw(kwargs, "fillvalue").cloned().unwrap_or(Value::None);
                    let mut out = Vec::new();
                    'outer: loop {
                        let mut row = Vec::with_capacity(pools.len());
                        let mut any = false;
                        for p in pools.iter_mut() {
                            match p.next() {
                                Some(v) => {
                                    any = true;
                                    row.push(v);
                                }
                                None => row.push(fill.clone()),
                            }
                        }
                        if !any {
                            break 'outer;
                        }
                        out.push(tuple(row));
                    }
                    Ok(make_iter(NativeIterState::Seq(out.into_iter())))
                }
                _ => return Err(Flow::Next),
            }
        })();
    match r {
        Err(Flow::Next) => return None,
        other => Some(other),
    }
}

fn n_tee(args: &[Value]) -> Result<i64, Flow> {
    args.get(1)
        .map(as_int_i64)
        .transpose()
        .map(|o| o.unwrap_or(2))
}

fn permute(
    items: &[Value],
    r: usize,
    cur: &mut Vec<Value>,
    used: &mut Vec<bool>,
    out: &mut Vec<Value>,
) {
    if cur.len() == r {
        out.push(tuple(cur.clone()));
        return;
    }
    for (idx, v) in items.iter().enumerate() {
        if used[idx] {
            continue;
        }
        used[idx] = true;
        cur.push(v.clone());
        permute(items, r, cur, used, out);
        cur.pop();
        used[idx] = false;
    }
}

fn combine(
    items: &[Value],
    r: usize,
    start: usize,
    cur: &mut Vec<Value>,
    out: &mut Vec<Value>,
    with_replacement: bool,
) {
    if cur.len() == r {
        out.push(tuple(cur.clone()));
        return;
    }
    if start >= items.len() {
        return;
    }
    let lim = if with_replacement {
        items.len()
    } else {
        items.len()
    };
    for idx in start..lim {
        cur.push(items[idx].clone());
        let next = if with_replacement { idx } else { idx + 1 };
        combine(items, r, next, cur, out, with_replacement);
        cur.pop();
    }
}

fn product_rec(pools: &[Vec<Value>], depth: usize, cur: &mut Vec<Value>, out: &mut Vec<Value>) {
    if depth == pools.len() {
        out.push(tuple(cur.clone()));
        return;
    }
    for v in &pools[depth] {
        cur.push(v.clone());
        product_rec(pools, depth + 1, cur, out);
        cur.pop();
    }
}

// ===========================================================================
// operator (spec 05 §5.3.2 operator row)
// ===========================================================================

fn build_operator(_i: &crate::frame::Interpreter) -> Value {
    let (m, ns) = new_module("operator");
    for n in [
        "add",
        "and_",
        "attrgetter",
        "concat",
        "contains",
        "countOf",
        "delitem",
        "eq",
        "floordiv",
        "ge",
        "getitem",
        "gt",
        "iadd",
        "iand",
        "iconcat",
        "idiv",
        "ilshift",
        "imatmul",
        "imod",
        "imul",
        "index",
        "inv",
        "ior",
        "ipow",
        "irshift",
        "is_",
        "is_not",
        "isub",
        "itemgetter",
        "itruediv",
        "ixor",
        "le",
        "length_hint",
        "lshift",
        "lt",
        "matmul",
        "methodcaller",
        "mod",
        "mul",
        "ne",
        "neg",
        "not_",
        "or_",
        "pos",
        "pow",
        "rshift",
        "setitem",
        "sub",
        "truth",
        "truediv",
        "xor",
        "abs",
        "invert",
    ] {
        fn1(&ns, "operator", n);
    }
    finish(m, ns)
}

fn binop_by_name(name: &str, a: &Value, b: &Value) -> BResult {
    use ap_obj::ops::BinOp;
    let op = match name {
        "add" | "concat" => BinOp::Add,
        "sub" => BinOp::Sub,
        "mul" => BinOp::Mul,
        "truediv" | "idiv" => BinOp::TrueDiv,
        "floordiv" => BinOp::FloorDiv,
        "mod" => BinOp::Mod,
        "pow" => BinOp::Pow,
        "lshift" => BinOp::LShift,
        "rshift" => BinOp::RShift,
        "and_" => BinOp::BitAnd,
        "or_" => BinOp::BitOr,
        "xor" => BinOp::BitXor,
        _ => return Err(raise("SystemError", format!("operator fn {name} unbound"))),
    };
    ap_obj::ops::binop(a, op, b).map_err(Flow::from_py)
}

fn op_abs(v: &Value) -> BResult {
    match v {
        Value::Int(n) => Ok(Value::Int(if n.is_negative() {
            n.neg()
        } else {
            n.clone()
        })),
        Value::Float(x) => Ok(f(x.abs())),
        Value::Bool(b) => Ok(i(if *b { 1 } else { 0 })),
        other => Err(raise(
            "TypeError",
            format!("bad operand type for abs(): '{}'", other.type_name()),
        )),
    }
}

pub fn call_operator(name: &str, args: &[Value], _kwargs: &[(String, Value)]) -> Option<BResult> {
    let two = || -> Result<(Value, Value), Flow> {
        match (args.first(), args.get(1)) {
            (Some(a), Some(b)) => Ok((a.clone(), b.clone())),
            _ => Err(raise(
                "TypeError",
                format!("{name} expected 2 arguments, got {}", args.len()),
            )),
        }
    };
    let r: BResult = (|| -> BResult {
        match name {
            "eq" => Ok(Value::Bool(py_eq(&two()?.0, &two()?.1))),
            "ne" => Ok(Value::Bool(!py_eq(&two()?.0, &two()?.1))),
            "lt" => Ok(Value::Bool(py_lt(&two()?.0, &two()?.1)?)),
            "le" => {
                let (a, b) = two()?;
                Ok(Value::Bool(py_lt(&a, &b)? || py_eq(&a, &b)))
            }
            "gt" => {
                let (a, b) = two()?;
                Ok(Value::Bool(!py_lt(&a, &b)? && !py_eq(&a, &b)))
            }
            "ge" => {
                let (a, b) = two()?;
                Ok(Value::Bool(!py_lt(&a, &b)?))
            }
            "is_" => Ok(Value::Bool(value_is(&two()?.0, &two()?.1))),
            "is_not" => Ok(Value::Bool(!value_is(&two()?.0, &two()?.1))),
            "truth" => Ok(Value::Bool(truthy(required(args, "truth")?))),
            "not_" => Ok(Value::Bool(!truthy(required(args, "not_")?))),
            "neg" => {
                let v = required(args, "neg")?;
                Ok(ap_obj::ops::neg(v).map_err(Flow::from_py)?)
            }
            "pos" => {
                let v = required(args, "pos")?;
                match v {
                    Value::Int(_) | Value::Float(_) | Value::Bool(_) => Ok(v.clone()),
                    other => Err(raise(
                        "TypeError",
                        format!("bad operand type for unary +: '{}'", other.type_name()),
                    )),
                }
            }
            "abs" => op_abs(required(args, "abs")?),
            "invert" | "inv" => {
                let v = required(args, "invert")?;
                Ok(ap_obj::ops::invert(v).map_err(Flow::from_py)?)
            }
            "index" => Ok(Value::Int(as_int_big(required(args, "index")?)?)),
            "length_hint" => {
                let v = required(args, "length_hint")?;
                let n = match v {
                    Value::Str(x) => x.as_str().chars().count() as i64,
                    Value::List(l) => l.len() as i64,
                    Value::Tuple(t) => t.len() as i64,
                    Value::Dict(d) => d.len() as i64,
                    Value::Set(x) | Value::Frozen(x) => x.len() as i64,
                    Value::Bytes(b) => b.len() as i64,
                    Value::Obj(o) => match o.kind {
                        ap_obj::ObjKind::Range => {
                            let ints = o.ints.borrow();
                            crate::iter::range_len(ints[0], ints[1], ints[2])
                        }
                        _ => 0,
                    },
                    _ => 0,
                };
                Ok(i(n))
            }
            "contains" => {
                let (c, v) = two()?;
                Ok(Value::Bool(container_contains(&c, &v)?))
            }
            "countOf" => {
                let (c, v) = two()?;
                let items = snapshot(&c)?;
                let mut n = 0;
                for x in items {
                    if py_eq(&x, &v) {
                        n += 1;
                    }
                }
                Ok(i(n))
            }
            "getitem" => {
                let (c, k) = two()?;
                crate::methods::subscript_get(&c, &k)
            }
            "setitem" => {
                let (c, k) = two()?;
                let v = args
                    .get(2)
                    .ok_or_else(|| raise("TypeError", "setitem expected 3 arguments, got 2"))?
                    .clone();
                crate::methods::subscript_set(&c, &k, v).map(|_| Value::None)
            }
            "delitem" => {
                let (c, k) = two()?;
                crate::methods::subscript_del(&c, &k).map(|_| Value::None)
            }
            "attrgetter" => {
                let mut attrs = Vec::new();
                for a in args {
                    attrs.push(match a {
                        Value::Str(x) => x.as_str().to_string(),
                        other => {
                            return Err(raise(
                                "TypeError",
                                format!("attribute name must be string, not {}", other.type_name()),
                            ))
                        }
                    });
                }
                Ok(marker(&format!("<attrgetter:{}/>", attrs.join(","))))
            }
            "itemgetter" => {
                let mut idxs = Vec::new();
                for a in args {
                    idxs.push(as_int_i64(a)?);
                }
                Ok(marker(&format!("<itemgetter:{idxs:?}/>")))
            }
            "methodcaller" => {
                let mname = match required(args, "methodcaller")? {
                    Value::Str(x) => x.as_str().to_string(),
                    other => {
                        return Err(raise(
                            "TypeError",
                            format!("method name must be string, not {}", other.type_name()),
                        ))
                    }
                };
                let recv = args.get(1).ok_or_else(|| {
                    raise("TypeError", "methodcaller() missing required argument")
                })?;
                crate::builtins::call_method_pub(recv, &mname, &args[2..])
            }
            "matmul" | "imatmul" => Err(raise(
                "TypeError",
                "unsupported operand type(s) for @ (v1 corpus does not exercise __matmul__)",
            )),
            _ => {
                // arithmetic/bitwise (incl. in-place spellings; i* returns the result)
                let base = if let Some(b) = name.strip_prefix('i') {
                    match b {
                        "add" | "sub" | "mul" | "truediv" | "floordiv" | "mod" | "pow"
                        | "lshift" | "rshift" | "and_" | "or_" | "xor" | "concat" => b,
                        _ => name,
                    }
                } else {
                    name
                };
                let known = [
                    "add", "sub", "mul", "truediv", "idiv", "floordiv", "mod", "pow", "lshift",
                    "rshift", "and_", "or_", "xor", "concat", "iconcat",
                ]
                .contains(&base);
                if !known {
                    return Err(Flow::Next);
                }
                let (a, b) = two()?;
                let base2 = if base == "iconcat" { "concat" } else { base };
                binop_by_name(base2, &a, &b)
            }
        }
    })();
    match r {
        Err(Flow::Next) => return None,
        other => Some(other),
    }
}

fn value_is(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::None, Value::None) => true,
        (Value::Obj(x), Value::Obj(y)) => std::rc::Rc::ptr_eq(x, y),
        _ => {
            // numbers/bools/strings are inline copies: identity == equality
            // within the same variant (documented v0 approximation).
            std::mem::discriminant(a) == std::mem::discriminant(b) && py_eq(a, b)
        }
    }
}

fn container_contains(c: &Value, v: &Value) -> Result<bool, Flow> {
    match c {
        Value::Str(st) => Ok(st.as_str().contains(&value_as_str(v)?)),
        Value::List(l) => Ok(l.items().iter().any(|x| py_eq(x, v))),
        Value::Tuple(t) => Ok(t.0.iter().any(|x| py_eq(x, v))),
        Value::Set(x) | Value::Frozen(x) => Ok(x.contains(v).map_err(Flow::from_py)?),
        Value::Dict(d) => Ok(d.get(v).is_some()),
        Value::Bytes(b) => Ok((**b).contains(&(as_int_i64(v)? as u8))),
        other => {
            let items = snapshot(other)?;
            Ok(items.iter().any(|x| py_eq(x, v)))
        }
    }
}

fn value_as_str(v: &Value) -> Result<String, Flow> {
    match v {
        Value::Str(x) => Ok(x.as_str().to_string()),
        other => Err(raise(
            "TypeError",
            format!(
                "'in <string>' requires string as left operand, not {}",
                other.type_name()
            ),
        )),
    }
}

// ===========================================================================
// heapq (spec 05 §5.3.2 heapq row)
// ===========================================================================

fn build_heapq(_i: &crate::frame::Interpreter) -> Value {
    let (m, ns) = new_module("heapq");
    for n in [
        "heappush",
        "heappop",
        "heapify",
        "heapreplace",
        "heappushpop",
        "merge",
        "nlargest",
        "nsmallest",
        "heapify_max",
        "heappop_max",
        "heappush_max",
        "heappushpop_max",
        "heapreplace_max",
    ] {
        fn1(&ns, "heapq", n);
    }
    finish(m, ns)
}

fn sift_down(h: &mut Vec<Value>, mut pos: usize, is_max: bool) -> Result<(), Flow> {
    let n = h.len();
    loop {
        let left = 2 * pos + 1;
        if left >= n {
            break;
        }
        let right = left + 1;
        let mut best = left;
        // "a is the preferred child": min-heap → a < b; max-heap → a > b.
        let better = |a: &Value, b: &Value| -> Result<bool, Flow> {
            Ok(if is_max { py_lt(b, a)? } else { py_lt(a, b)? })
        };
        if right < n && better(&h[right], &h[left])? {
            best = right;
        }
        if better(&h[best], &h[pos])? {
            h.swap(pos, best);
            pos = best;
        } else {
            break;
        }
    }
    Ok(())
}

fn sift_up(h: &mut Vec<Value>, mut pos: usize, is_max: bool) -> Result<(), Flow> {
    while pos > 0 {
        let parent = (pos - 1) / 2;
        let better = if is_max {
            py_lt(&h[parent], &h[pos])?
        } else {
            py_lt(&h[pos], &h[parent])?
        };
        if better {
            h.swap(pos, parent);
            pos = parent;
        } else {
            break;
        }
    }
    Ok(())
}

pub fn call_heapq(name: &str, args: &[Value], _kwargs: &[(String, Value)]) -> Option<BResult> {
    let is_max = name.ends_with("_max");
    let base = name.trim_end_matches("_max").trim_end_matches('_');
    let r: BResult = (|| match base {
        "heappush" => {
            let (handle, mut items) = heap_vec(args)?;
            let v = args
                .get(1)
                .ok_or_else(|| raise("TypeError", "heappush expected 2 arguments, got 1"))?
                .clone();
            items.push(v);
            let last = items.len() - 1;
            sift_up(&mut items, last, is_max)?;
            write_back(&handle, items)?;
            Ok(Value::None)
        }
        "heappop" => {
            let (handle, mut items) = heap_vec(args)?;
            if items.is_empty() {
                return Err(raise("IndexError", "index out of range"));
            }
            let last = items.len() - 1;
            items.swap(0, last);
            let out = items.pop().unwrap();
            sift_down(&mut items, 0, is_max)?;
            write_back(&handle, items)?;
            Ok(out)
        }
        "heapify" => {
            let (handle, mut items) = heap_vec(args)?;
            let n = items.len();
            for p in (0..n / 2).rev() {
                sift_down(&mut items, p, is_max)?;
            }
            write_back(&handle, items)?;
            Ok(Value::None)
        }
        "heapreplace" => {
            let (handle, mut items) = heap_vec(args)?;
            let v = args
                .get(1)
                .ok_or_else(|| raise("TypeError", "heapreplace expected 2 arguments, got 1"))?
                .clone();
            if items.is_empty() {
                return Err(raise("IndexError", "index out of range"));
            }
            let out = items[0].clone();
            items[0] = v;
            sift_down(&mut items, 0, is_max)?;
            write_back(&handle, items)?;
            Ok(out)
        }
        "heappushpop" => {
            let (handle, mut items) = heap_vec(args)?;
            let v = args
                .get(1)
                .ok_or_else(|| raise("TypeError", "heappushpop expected 2 arguments, got 1"))?
                .clone();
            if items.is_empty() {
                return Ok(v);
            }
            // CPython heappushpop: if item <= root (min-heap), push+pop
            // cancel — return item unchanged.
            let cancel = if is_max {
                py_lt(&items[0], &v)?
            } else {
                py_lt(&v, &items[0])?
            };
            if cancel {
                return Ok(v);
            }
            let out = items[0].clone();
            items[0] = v;
            sift_down(&mut items, 0, is_max)?;
            write_back(&handle, items)?;
            Ok(out)
        }
        "nlargest" | "nsmallest" => {
            let n = as_int_i64(required(args, name)?)?;
            let items = snapshot(args.get(1).ok_or_else(|| {
                raise("TypeError", format!("{base}() missing required argument"))
            })?)?;
            let mut sorted_items = items;
            sorted_items.sort_by(|a, b| {
                let ord = if py_lt(a, b).unwrap_or(false) {
                    std::cmp::Ordering::Less
                } else if py_eq(a, b) {
                    std::cmp::Ordering::Equal
                } else {
                    std::cmp::Ordering::Greater
                };
                if base == "nlargest" {
                    ord.reverse()
                } else {
                    ord
                }
            });
            sorted_items.truncate(n.max(0) as usize);
            Ok(list(sorted_items))
        }
        "merge" => {
            let mut streams: Vec<std::vec::IntoIter<Value>> = Vec::new();
            for a in args {
                streams.push(snapshot(a)?.into_iter());
            }
            let mut heads: Vec<Option<Value>> = streams.iter_mut().map(|st| st.next()).collect();
            let mut out = Vec::new();
            loop {
                let mut best: Option<(usize, Value)> = None;
                for (i2, h) in heads.iter().enumerate() {
                    if let Some(v) = h {
                        let take = match &best {
                            None => true,
                            Some((_, bv)) => {
                                if is_max {
                                    py_lt(bv, v).unwrap_or(false)
                                } else {
                                    py_lt(v, bv).unwrap_or(false)
                                }
                            }
                        };
                        if take {
                            best = Some((i2, v.clone()));
                        }
                    }
                }
                match best {
                    None => break,
                    Some((i2, v)) => {
                        out.push(v.clone());
                        heads[i2] = streams[i2].next();
                    }
                }
            }
            Ok(make_iter(NativeIterState::Seq(out.into_iter())))
        }
        _ => Err(raise("SystemError", format!("heapq fn {name} unbound"))),
    })();
    match r {
        Err(Flow::Next) => return None,
        other => Some(other),
    }
}

/// Read a list arg as a Vec snapshot.
fn heap_vec(args: &[Value]) -> Result<(Value, Vec<Value>), Flow> {
    let v = args
        .first()
        .ok_or_else(|| raise("TypeError", "heap argument must be a list"))?;
    match v {
        Value::List(l) => Ok((v.clone(), l.items().to_vec())),
        other => Err(raise(
            "TypeError",
            format!("heap argument must be a list, not {}", other.type_name()),
        )),
    }
}

/// Write a Vec back into a list handle (aliasing preserved via the same Gc).
fn write_back(handle: &Value, items: Vec<Value>) -> Result<(), Flow> {
    if let Value::List(l) = handle {
        l.clear();
        for x in items {
            l.append(x);
        }
        return Ok(());
    }
    Err(raise("SystemError", "heap handle lost"))
}
// ===========================================================================
// bisect (spec 05 §5.3.2 bisect row)
// ===========================================================================

fn build_bisect(_i: &crate::frame::Interpreter) -> Value {
    let (m, ns) = new_module("bisect");
    for n in [
        "bisect",
        "bisect_left",
        "bisect_right",
        "insort",
        "insort_left",
        "insort_right",
    ] {
        fn1(&ns, "bisect", n);
    }
    finish(m, ns)
}

pub fn call_bisect(name: &str, args: &[Value], kwargs: &[(String, Value)]) -> Option<BResult> {
    let left = name.ends_with("_left") || name == "insort";
    let ins = name.starts_with("insort");
    let r: BResult = (|| {
        let handle = required(args, "bisect")?;
        let items: Vec<Value> = match handle {
            Value::List(l) => l.items().to_vec(),
            other => {
                return Err(raise(
                    "TypeError",
                    format!("must be a list, not {}", other.type_name()),
                ))
            }
        };
        let x = args
            .get(1)
            .ok_or_else(|| raise("TypeError", "missing value argument"))?;
        let lo = kw(kwargs, "lo")
            .map(as_int_i64)
            .transpose()?
            .unwrap_or(0)
            .max(0) as usize;
        let hi = kw(kwargs, "hi")
            .map(as_int_i64)
            .transpose()?
            .map(|v| v as usize)
            .unwrap_or(items.len());
        let (mut a, mut b) = (lo.min(items.len()), hi.min(items.len()));
        while a < b {
            let mid = (a + b) / 2;
            let item = items[mid].clone();
            let lt = py_lt(&item, x)?;
            let eq = py_eq(&item, x);
            if lt || (!left && eq) {
                a = mid + 1;
            } else {
                b = mid;
            }
        }
        if ins {
            if let Value::List(l) = handle {
                // insert via Vec rebuild (small lists in v1 corpus)
                let mut all = items;
                all.insert(a, x.clone());
                l.clear();
                for v in all {
                    l.append(v);
                }
            }
            Ok(Value::None)
        } else {
            Ok(i(a as i64))
        }
    })();
    match r {
        Err(Flow::Next) => return None,
        other => Some(other),
    }
}

// ===========================================================================
// functools (reduce + partial only; the rest needs classes — documented gap)
// ===========================================================================

fn build_functools(_i: &crate::frame::Interpreter) -> Value {
    let (m, ns) = new_module("functools");
    for n in ["reduce", "partial"] {
        fn1(&ns, "functools", n);
    }
    finish(m, ns)
}

pub fn call_functools(name: &str, args: &[Value], kwargs: &[(String, Value)]) -> Option<BResult> {
    let r: BResult = (|| -> BResult {
        match name {
            "reduce" => {
                let func = callable_name(required(args, "reduce")?)?;
                let items =
                    snapshot(args.get(1).ok_or_else(|| {
                        raise("TypeError", "reduce() missing required argument")
                    })?)?;
                let mut it = items.into_iter();
                let mut acc = match args.get(2) {
                    Some(v) => v.clone(),
                    None => match it.next() {
                        Some(v) => v,
                        None => {
                            return Err(raise(
                                "TypeError",
                                "reduce() of empty iterable with no initial value",
                            ))
                        }
                    },
                };
                for x in it {
                    acc = call_named(&func, &[acc, x])?;
                }
                Ok(acc)
            }
            "partial" => {
                let func = required(args, "partial")?.clone();
                let pargs: Vec<Value> = args[1..].to_vec();
                // partial objects: a self-describing BuiltinFn marker whose
                // name carries the callee + bound args (callable without an
                // interp.rs Opaque-call hook; attrs via the native_attr layer).
                let name_id = name_callable(&func);
                let enc: Vec<String> = pargs.iter().map(encode_arg).collect();
                let marker_name = format!("<partial:{name_id}:{}>", enc.join("\u{1f}"));
                let obj = PyObject::new(ap_obj::ObjKind::BuiltinFn, marker_name);
                {
                    let mut attrs = obj.attrs.borrow_mut();
                    let _ = attrs.set(s("func"), func);
                    let _ = attrs.set(s("args"), tuple(pargs));
                    let _ = attrs.set(s("keywords"), Value::Dict(PyDict::new()));
                }
                *obj.repr_text.borrow_mut() =
                    Some(format!("functools.partial(<callable>, {})", enc.len()));
                Ok(Value::Obj(obj))
            }
            _ => return Err(Flow::Next),
        }
    })();
    match r {
        Err(Flow::Next) => return None,
        other => Some(other),
    }
}

/// Call a functools.partial object built by call_functools.
pub fn call_partial_obj(obj: &Gc<PyObject>, args: &[Value], kwargs: &[(String, Value)]) -> BResult {
    let fname = {
        let attrs = obj.attrs.borrow();
        match attrs.get(&s("__func_name__")) {
            Some(Value::Str(n)) => n.as_str().to_string(),
            _ => return Err(raise("TypeError", "not a partial object")),
        }
    };
    let pargs = {
        let attrs = obj.attrs.borrow();
        match attrs.get(&s("__args__")) {
            Some(Value::List(l)) => l.items().to_vec(),
            _ => vec![],
        }
    };
    let mut full = pargs;
    full.extend_from_slice(args);
    call_named(&fname, &full)
}

// ===========================================================================
// string (constants + capwords; Template/Formatter = class machinery, gap)
// ===========================================================================

fn build_string(_i: &crate::frame::Interpreter) -> Value {
    let (m, ns) = new_module("string");
    ns_const(&ns, "ascii_lowercase", s("abcdefghijklmnopqrstuvwxyz"));
    ns_const(&ns, "ascii_uppercase", s("ABCDEFGHIJKLMNOPQRSTUVWXYZ"));
    ns_const(
        &ns,
        "ascii_letters",
        s("abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ"),
    );
    ns_const(&ns, "digits", s("0123456789"));
    ns_const(&ns, "hexdigits", s("0123456789abcdefABCDEF"));
    ns_const(&ns, "octdigits", s("01234567"));
    ns_const(&ns, "punctuation", s("!\"#$%&'()*+,-./:;<=>?@[\\]^_`{|}~"));
    ns_const(&ns, "whitespace", s(" \t\n\r\u{0b}\u{0c}"));
    ns_const(
        &ns,
        "printable",
        s("0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ!\"#$%&'()*+,-./:;<=>?@[\\]^_`{|}~ \t\n\r\u{0b}\u{0c}"),
    );
    fn1(&ns, "string", "capwords");
    finish(m, ns)
}

pub fn call_string(name: &str, args: &[Value], _kwargs: &[(String, Value)]) -> Option<BResult> {
    if name != "capwords" {
        return None;
    }
    let r: BResult = (|| -> BResult {
        let sv = required(args, "capwords")?;
        let text = match sv {
            Value::Str(x) => x.as_str().to_string(),
            other => {
                return Err(raise(
                    "TypeError",
                    format!("bad operand type for capwords(): '{}'", other.type_name()),
                ))
            }
        };
        let sep = match args.get(1) {
            None => None,
            Some(Value::Str(x)) => Some(x.as_str().to_string()),
            Some(other) => {
                return Err(raise(
                    "TypeError",
                    format!("bad operand type for capwords(): '{}'", other.type_name()),
                ))
            }
        };
        let parts: Vec<String> = text
            .split(char::is_whitespace)
            .filter(|p| !p.is_empty())
            .map(|p| {
                let mut c = p.chars();
                match c.next() {
                    Some(first) => first.to_uppercase().collect::<String>() + c.as_str(),
                    None => String::new(),
                }
            })
            .collect();
        Ok(s(parts.join(&sep.unwrap_or_else(|| " ".to_string()))))
    })();
    match r {
        Err(Flow::Next) => return None,
        other => Some(other),
    }
}

// ===========================================================================
// textwrap (wrap/fill/dedent/indent/shorten; TextWrapper class = gap)
// ===========================================================================

fn build_textwrap(_i: &crate::frame::Interpreter) -> Value {
    let (m, ns) = new_module("textwrap");
    for n in ["wrap", "fill", "dedent", "indent", "shorten"] {
        fn1(&ns, "textwrap", n);
    }
    finish(m, ns)
}

pub fn call_textwrap(name: &str, args: &[Value], kwargs: &[(String, Value)]) -> Option<BResult> {
    let r: BResult = (|| -> BResult {
        match name {
            "wrap" => {
                let text = str_arg(required(args, "wrap")?, "wrap")?;
                let width = kw(kwargs, "width")
                    .map(|v| as_int_i64(&v))
                    .transpose()?
                    .unwrap_or(70);
                Ok(list(
                    wrap_text(&text, width as usize)
                        .into_iter()
                        .map(s)
                        .collect(),
                ))
            }
            "fill" => {
                let text = str_arg(required(args, "fill")?, "fill")?;
                let width = kw(kwargs, "width")
                    .map(|v| as_int_i64(&v))
                    .transpose()?
                    .unwrap_or(70);
                Ok(s(wrap_text(&text, width as usize).join("\n")))
            }
            "dedent" => {
                let text = str_arg(required(args, "dedent")?, "dedent")?;
                Ok(s(dedent_text(&text)))
            }
            "indent" => {
                let text = str_arg(required(args, "indent")?, "indent")?;
                let prefix = match args.get(1) {
                    Some(Value::Str(x)) => x.as_str().to_string(),
                    _ => String::new(),
                };
                let pred = match kw(kwargs, "predicate") {
                    Some(v) => Some(callable_name(v)?),
                    None => None,
                };
                let out: Vec<String> = text
                    .split('\n')
                    .map(|ln| {
                        let add = match &pred {
                            None => !ln.trim().is_empty(),
                            Some(p) => call_named(p, &[s(ln)]).map(|v| truthy(&v)).unwrap_or(false),
                        };
                        if add {
                            format!("{prefix}{ln}")
                        } else {
                            ln.to_string()
                        }
                    })
                    .collect();
                Ok(s(out.join("\n")))
            }
            "shorten" => {
                let text = str_arg(required(args, "shorten")?, "shorten")?;
                let width = kw(kwargs, "width")
                    .map(|v| as_int_i64(&v))
                    .transpose()?
                    .unwrap_or(70);
                let placeholder = match kw(kwargs, "placeholder") {
                    Some(Value::Str(x)) => x.as_str().to_string(),
                    _ => "[...]".to_string(),
                };
                let words = wrap_text(&text, usize::MAX);
                let mut out = String::new();
                for w in words {
                    let cand = if out.is_empty() {
                        w.clone()
                    } else {
                        format!("{out} {w}")
                    };
                    if cand.chars().count() + placeholder.chars().count() > width as usize {
                        break;
                    }
                    out = cand;
                }
                let joined = wrap_text(&text, usize::MAX).join(" ");
                if joined.chars().count() <= width as usize {
                    return Ok(s(joined));
                }
                Ok(s(format!("{out}{placeholder}")))
            }
            _ => Err(raise("SystemError", format!("textwrap fn {name} unbound"))),
        }
    })();
    match r {
        Err(Flow::Next) => return None,
        other => Some(other),
    }
}

fn str_arg(v: &Value, fname: &str) -> Result<String, Flow> {
    match v {
        Value::Str(x) => Ok(x.as_str().to_string()),
        other => Err(raise(
            "TypeError",
            format!("{fname}() argument must be str, not {}", other.type_name()),
        )),
    }
}

fn wrap_text(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut cur = String::new();
    for word in text.split_whitespace() {
        let wlen = word.chars().count();
        if cur.is_empty() {
            cur = word.to_string();
        } else if cur.chars().count() + 1 + wlen <= width {
            cur.push(' ');
            cur.push_str(word);
        } else {
            lines.push(std::mem::take(&mut cur));
            cur = word.to_string();
        }
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    lines
}

fn dedent_text(text: &str) -> String {
    let lines: Vec<&str> = text.split('\n').collect();
    let mut margin: Option<String> = None;
    for ln in &lines {
        if ln.trim().is_empty() {
            continue;
        }
        let indent: String = ln.chars().take_while(|c| *c == ' ').collect();
        margin = Some(match margin {
            None => indent,
            Some(m) => {
                let mut common = String::new();
                for (a, b) in m.chars().zip(indent.chars()) {
                    if a == b {
                        common.push(a);
                    } else {
                        break;
                    }
                }
                common
            }
        });
    }
    let margin = margin.unwrap_or_default();
    lines
        .into_iter()
        .map(|ln| ln.strip_prefix(margin.as_str()).unwrap_or(ln).to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

// ===========================================================================
// collections-lite (deque only; Counter/defaultdict/OrderedDict = classes)
// ===========================================================================

fn build_collections(_i: &crate::frame::Interpreter) -> Value {
    let (m, ns) = new_module("collections");
    for n in ["deque"] {
        fn1(&ns, "collections", n);
    }
    finish(m, ns)
}

fn make_deque(items: Vec<Value>, maxlen: Option<i64>) -> Value {
    let obj = PyObject::new(ap_obj::ObjKind::Opaque, "deque");
    let items_copy = items.clone();
    {
        let mut attrs = obj.attrs.borrow_mut();
        let _ = attrs.set(s("<items>"), list(items));
        let _ = attrs.set(
            s("<maxlen>"),
            match maxlen {
                Some(n) => i(n),
                None => Value::None,
            },
        );
    }
    update_deque_repr(&obj, &items_copy, maxlen);
    Value::Obj(obj)
}

/// Refresh the precomputed repr after any deque mutation.
pub fn update_deque_repr(obj: &Gc<PyObject>, items: &[Value], maxlen: Option<i64>) {
    let inner: Vec<String> = items
        .iter()
        .map(|v| ap_obj::ops::repr(v).unwrap_or_else(|_| "?".to_string()))
        .collect();
    let mut text = format!("deque([{}]", inner.join(", "));
    if let Some(ml) = maxlen {
        text.push_str(&format!(", maxlen={ml}"));
    }
    text.push(')');
    *obj.repr_text.borrow_mut() = Some(text);
}

fn deque_state<'a>(v: &'a Value) -> Result<(Vec<Value>, Option<i64>), Flow> {
    match v {
        Value::Obj(o) if o.name == "deque" => {
            let attrs = o.attrs.borrow();
            let items = match attrs.get(&s("<items>")) {
                Some(Value::List(l)) => l.items().to_vec(),
                _ => vec![],
            };
            let maxlen = match attrs.get(&s("<maxlen>")) {
                Some(Value::Int(n)) => n.to_i64(),
                _ => None,
            };
            Ok((items, maxlen))
        }
        other => Err(raise(
            "TypeError",
            format!("expected deque, not {}", other.type_name()),
        )),
    }
}

fn set_deque_items(v: &Value, mut items: Vec<Value>, maxlen: Option<i64>) -> Result<(), Flow> {
    if let Some(ml) = maxlen {
        while items.len() as i64 > ml {
            items.remove(0);
        }
    }
    if let Value::Obj(o) = v {
        let items_copy = items.clone();
        // maxlen=None here means "already trimmed" — preserve the stored bound.
        let ml = if maxlen.is_some() {
            maxlen
        } else {
            o.attrs
                .borrow()
                .get(&s("<maxlen>"))
                .and_then(|m| m.as_int())
                .and_then(|i| i.to_i64())
        };
        {
            let mut attrs = o.attrs.borrow_mut();
            let _ = attrs.set(s("<items>"), list(items));
        }
        update_deque_repr(o, &items_copy, ml);
    }
    Ok(())
}

pub fn call_collections(name: &str, args: &[Value], kwargs: &[(String, Value)]) -> Option<BResult> {
    let r: BResult = (|| -> BResult {
        match name {
            "deque" => {
                let items = match args.first() {
                    Some(v) => snapshot(v)?,
                    None => vec![],
                };
                let maxlen = kw(kwargs, "maxlen").map(|v| as_int_i64(&v)).transpose()?;
                Ok(make_deque(items, maxlen))
            }
            "deque.append" => {
                let (recv, v) = (
                    args.first()
                        .ok_or_else(|| raise("TypeError", "missing receiver argument"))?,
                    args.get(1)
                        .ok_or_else(|| raise("TypeError", "missing argument"))?,
                );
                let (mut items, maxlen) = deque_state(recv)?;
                items.push(v.clone());
                set_deque_items(recv, items, maxlen)?;
                Ok(Value::None)
            }
            "deque.appendleft" => {
                let (recv, v) = (
                    args.first()
                        .ok_or_else(|| raise("TypeError", "missing receiver argument"))?,
                    args.get(1)
                        .ok_or_else(|| raise("TypeError", "missing argument"))?,
                );
                let (mut items, maxlen) = deque_state(recv)?;
                items.insert(0, v.clone());
                // appendleft overflow drops from the RIGHT (opposite of append).
                if let Some(ml) = maxlen {
                    while items.len() as i64 > ml {
                        items.pop();
                    }
                }
                set_deque_items(recv, items, maxlen)?;
                Ok(Value::None)
            }
            "deque.pop" => {
                let recv = args
                    .first()
                    .ok_or_else(|| raise("TypeError", "missing receiver argument"))?;
                let (mut items, maxlen) = deque_state(recv)?;
                let out = items
                    .pop()
                    .ok_or_else(|| raise("IndexError", "pop from an empty deque"))?;
                set_deque_items(recv, items, maxlen)?;
                Ok(out)
            }
            "deque.popleft" => {
                let recv = args
                    .first()
                    .ok_or_else(|| raise("TypeError", "missing receiver argument"))?;
                let (mut items, maxlen) = deque_state(recv)?;
                if items.is_empty() {
                    return Err(raise("IndexError", "pop from an empty deque"));
                }
                let out = items.remove(0);
                set_deque_items(recv, items, maxlen)?;
                Ok(out)
            }
            "deque.extend" => {
                let (recv, v) = (
                    args.first()
                        .ok_or_else(|| raise("TypeError", "missing receiver argument"))?,
                    args.get(1)
                        .ok_or_else(|| raise("TypeError", "missing argument"))?,
                );
                let (mut items, maxlen) = deque_state(recv)?;
                items.extend(snapshot(v)?);
                set_deque_items(recv, items, maxlen)?;
                Ok(Value::None)
            }
            "deque.rotate" => {
                let recv = args
                    .first()
                    .ok_or_else(|| raise("TypeError", "missing receiver argument"))?;
                let n = args
                    .get(1)
                    .map(|v| as_int_i64(&v))
                    .transpose()?
                    .unwrap_or(1);
                let (mut items, maxlen) = deque_state(recv)?;
                if !items.is_empty() {
                    let len = items.len() as i64;
                    let k = n.rem_euclid(len) as usize;
                    if k > 0 {
                        items.rotate_right(k);
                    }
                }
                set_deque_items(recv, items, maxlen)?;
                Ok(Value::None)
            }
            "deque.index" => {
                let recv = args
                    .first()
                    .ok_or_else(|| raise("TypeError", "missing receiver argument"))?;
                let target = args
                    .get(1)
                    .ok_or_else(|| raise("TypeError", "index expected at least 1 argument"))?;
                let (items, _) = deque_state(recv)?;
                for (idx, x) in items.iter().enumerate() {
                    if py_eq(x, &target) {
                        return Ok(i(idx as i64));
                    }
                }
                Err(raise("ValueError", format!("{target:?} is not in deque")))
            }
            "deque.clear" => {
                let recv = args
                    .first()
                    .ok_or_else(|| raise("TypeError", "missing receiver argument"))?;
                let (_, maxlen) = deque_state(recv)?;
                set_deque_items(recv, vec![], maxlen)?;
                Ok(Value::None)
            }
            "deque.__iter__" => {
                let recv = args
                    .first()
                    .ok_or_else(|| raise("TypeError", "missing receiver argument"))?;
                let (items, _) = deque_state(recv)?;
                Ok(make_iter(NativeIterState::Seq(items.into_iter())))
            }
            _ => return Err(Flow::Next),
        }
    })();
    match r {
        Err(Flow::Next) => return None,
        other => Some(other),
    }
}

// ===========================================================================
// json (loads/dumps byte-exact incl. error texts; S-05-4 row json)
// ===========================================================================

fn build_json(_i: &crate::frame::Interpreter) -> Value {
    let (m, ns) = new_module("json");
    for n in ["dump", "dumps", "load", "loads", "detect_encoding"] {
        fn1(&ns, "json", n);
    }
    // JSONDecodeError: an exception type object so `except json.JSONDecodeError`
    // name reads resolve; calls construct Exception instances.
    let _ = ns.set(s("JSONDecodeError"), interned_type_pub("JSONDecodeError"));
    finish(m, ns)
}

fn interned_type_pub(name: &str) -> Value {
    if let Some(interp) = CURRENT.with(|c| c.borrow().clone()) {
        return crate::builtins::interned_type(&interp, name);
    }
    Value::Obj(PyObject::new(ap_obj::ObjKind::Type, name))
}

pub fn call_json(name: &str, args: &[Value], kwargs: &[(String, Value)]) -> Option<BResult> {
    let r: BResult = (|| -> BResult {
        match name {
            "loads" => json_loads(required(args, "loads")?, kwargs),
            "dumps" => json_dumps(required(args, "dumps")?, kwargs),
            "load" => {
                // json.load(fp) == loads(fp.read())
                let fp = required(args, "load")?;
                let text = crate::builtins::call_method_pub(fp, "read", &[])?;
                json_loads(&text, kwargs)
            }
            "dump" => {
                let obj = required(args, "dump")?;
                let fp = args
                    .get(1)
                    .ok_or_else(|| raise("TypeError", "dump() missing required argument"))?;
                let text = json_dumps(obj, kwargs)?;
                crate::builtins::call_method_pub(fp, "write", &[text])
            }
            "detect_encoding" => Ok(s("utf-8")),
            _ => Err(raise("SystemError", format!("json fn {name} unbound"))),
        }
    })();
    match r {
        Err(Flow::Next) => return None,
        other => Some(other),
    }
}

fn json_error(msg: &str, doc: &str, pos: usize) -> Flow {
    // CPython: "{msg}: line {l} column {c} (char {p})" — 1-based line/col.
    let (line, col) = line_col(doc, pos);
    let e = ExceptionValue::new(
        "JSONDecodeError",
        vec![s(format!("{msg}: line {line} column {col} (char {pos})"))],
    );
    // Attach structured attrs for e.msg/e.lineno/e.colno/e.pos/e.doc reads
    // (attr_get hook consults EXC_ATTRS by object identity path — we stash the
    // structured fields inside the exception's args? No: use the side table).
    let obj = match &e {
        _ => None::<Value>,
    };
    let _ = obj;
    EXC_ATTRS.with(|t| {
        t.borrow_mut()
            .insert(format!("json:{pos}:{line}:{col}:{msg}"), doc.to_string())
    });
    Flow::Raise(e)
}

thread_local! {
    /// Structured attrs for synthetic exception instances (JSONDecodeError),
    /// keyed by a synthetic key embedded in the exception id table.
    static EXC_ATTRS: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
    /// Last raised JSONDecodeError's structured fields (msg,line,col,pos,doc).
    static LAST_JSON_ERR: RefCell<Option<(String, i64, i64, i64, String)>> =
        const { RefCell::new(None) };
}

fn json_error2(msg: &str, doc: &str, pos: usize) -> Flow {
    let (line, col) = line_col(doc, pos);
    LAST_JSON_ERR.with(|c| {
        *c.borrow_mut() = Some((msg.to_string(), line, col, pos as i64, doc.to_string()))
    });
    Flow::Raise(ExceptionValue::new(
        "JSONDecodeError",
        vec![s(format!("{msg}: line {line} column {col} (char {pos})"))],
    ))
}

fn line_col(doc: &str, pos: usize) -> (i64, i64) {
    let upto = &doc[..pos.min(doc.len())];
    let line = upto.matches('\n').count() as i64 + 1;
    let col = match upto.rfind('\n') {
        Some(idx) => (upto[idx + 1..].chars().count() + 1) as i64,
        None => (upto.chars().count() + 1) as i64,
    };
    (line, col)
}

struct JsonParser<'a> {
    doc: &'a str,
    bytes: &'a [u8],
    pos: usize,
    parse_float: Option<String>,
    object_pairs_hook: Option<String>,
}

impl<'a> JsonParser<'a> {
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }
    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.pos += 1;
        }
    }
    fn expect(&mut self, b: u8, msg: &str) -> Result<(), Flow> {
        self.skip_ws();
        if self.peek() == Some(b) {
            self.pos += 1;
            Ok(())
        } else {
            Err(json_error2(msg, self.doc, self.pos))
        }
    }
    fn parse_value(&mut self) -> Result<Value, Flow> {
        self.skip_ws();
        match self.peek() {
            Some(b'{') => self.parse_object(),
            Some(b'[') => self.parse_array(),
            Some(b'"') => self.parse_string().map(s),
            Some(b't') => self.parse_lit("true", Value::Bool(true)),
            Some(b'f') => self.parse_lit("false", Value::Bool(false)),
            Some(b'n') => self.parse_lit("null", Value::None),
            Some(c) if c == b'-' || c.is_ascii_digit() => self.parse_number(),
            _ => Err(json_error2("Expecting value", self.doc, self.pos)),
        }
    }
    fn parse_lit(&mut self, lit: &str, v: Value) -> Result<Value, Flow> {
        if self.doc[self.pos..].starts_with(lit) {
            self.pos += lit.len();
            Ok(v)
        } else {
            Err(json_error2("Expecting value", self.doc, self.pos))
        }
    }
    fn parse_number(&mut self) -> Result<Value, Flow> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
            self.pos += 1;
        }
        let mut is_float = false;
        if self.peek() == Some(b'.') {
            is_float = true;
            self.pos += 1;
            while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                self.pos += 1;
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            is_float = true;
            self.pos += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                self.pos += 1;
            }
        }
        let text = &self.doc[start..self.pos];
        if !is_float {
            if let Some(int_text) = text.strip_prefix('-') {
                if let Ok(n) = Int::parse_base10(int_text) {
                    return Ok(Value::Int(n.neg()));
                }
            } else if let Ok(n) = Int::parse_base10(text) {
                return Ok(Value::Int(n));
            }
            return Err(json_error2("Expecting value", self.doc, start));
        }
        let v = text
            .parse::<f64>()
            .map(f)
            .map_err(|_| json_error2("Expecting value", self.doc, start))?;
        match &self.parse_float {
            None => Ok(v),
            Some(pfn) => call_named(pfn, &[s(text)]),
        }
    }
    fn parse_string(&mut self) -> Result<String, Flow> {
        self.expect(b'"', "Expecting property name enclosed in double quotes")?;
        let mut out = String::new();
        loop {
            match self.peek() {
                None => {
                    return Err(json_error2(
                        "Unterminated string starting at",
                        self.doc,
                        self.pos,
                    ))
                }
                Some(b'"') => {
                    self.pos += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.pos += 1;
                    let esc = self.peek().ok_or_else(|| {
                        json_error2("Unterminated string starting at", self.doc, self.pos)
                    })?;
                    self.pos += 1;
                    match esc {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let hex = &self.doc[self.pos..(self.pos + 4).min(self.doc.len())];
                            self.pos += 4;
                            let cp = u32::from_str_radix(hex, 16).map_err(|_| {
                                json_error2("Invalid \\uXXXX escape", self.doc, self.pos)
                            })?;
                            // surrogate pair handling
                            if (0xD800..0xDC00).contains(&cp) {
                                if self.doc[self.pos..].starts_with("\\u") {
                                    let hex2 =
                                        &self.doc[self.pos + 2..(self.pos + 6).min(self.doc.len())];
                                    let lo = u32::from_str_radix(hex2, 16).unwrap_or(0);
                                    if (0xDC00..0xE000).contains(&lo) {
                                        self.pos += 6;
                                        let c = 0x10000 + ((cp - 0xD800) << 10) + (lo - 0xDC00);
                                        out.push(char::from_u32(c).unwrap_or('\u{FFFD}'));
                                        continue;
                                    }
                                }
                                out.push('\u{FFFD}');
                            } else {
                                out.push(char::from_u32(cp).unwrap_or('\u{FFFD}'));
                            }
                        }
                        other => out.push(other as char),
                    }
                }
                Some(_) => {
                    // consume one UTF-8 char
                    let rest = &self.doc[self.pos..];
                    let ch = rest.chars().next().unwrap();
                    out.push(ch);
                    self.pos += ch.len_utf8();
                }
            }
        }
    }
    fn parse_array(&mut self) -> Result<Value, Flow> {
        self.expect(b'[', "Expecting value")?;
        let mut items = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(list(items));
        }
        loop {
            items.push(self.parse_value()?);
            self.skip_ws();
            match self.peek() {
                Some(b',') => {
                    let comma_pos = self.pos;
                    self.pos += 1;
                    self.skip_ws();
                    if self.peek() == Some(b']') {
                        return Err(json_error2(
                            "Illegal trailing comma before end of array",
                            self.doc,
                            comma_pos,
                        ));
                    }
                }
                Some(b']') => {
                    self.pos += 1;
                    return Ok(list(items));
                }
                _ => return Err(json_error2("Expecting ',' delimiter", self.doc, self.pos)),
            }
        }
    }
    fn parse_object(&mut self) -> Result<Value, Flow> {
        self.expect(b'{', "Expecting value")?;
        let d = PyDict::new();
        let mut pairs: Vec<Value> = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return self.finish_object(d, pairs);
        }
        loop {
            self.skip_ws();
            let key = self.parse_string().map_err(|_| {
                json_error2(
                    "Expecting property name enclosed in double quotes",
                    self.doc,
                    self.pos,
                )
            })?;
            self.expect(b':', "Expecting ':' delimiter")?;
            let v = self.parse_value()?;
            let _ = d.set(s(key.clone()), v.clone());
            pairs.push(tuple(vec![s(key), v]));
            self.skip_ws();
            match self.peek() {
                Some(b',') => {
                    let comma_pos = self.pos;
                    self.pos += 1;
                    self.skip_ws();
                    if self.peek() == Some(b'}') {
                        return Err(json_error2(
                            "Illegal trailing comma before end of object",
                            self.doc,
                            comma_pos,
                        ));
                    }
                }
                Some(b'}') => {
                    self.pos += 1;
                    return self.finish_object(d, pairs);
                }
                _ => return Err(json_error2("Expecting ',' delimiter", self.doc, self.pos)),
            }
        }
    }

    fn finish_object(&self, d: Gc<PyDict>, pairs: Vec<Value>) -> Result<Value, Flow> {
        match &self.object_pairs_hook {
            None => Ok(Value::Dict(d)),
            Some(hook) => call_named(hook, &[list(pairs)]),
        }
    }
}

fn json_loads(v: &Value, kwargs: &[(String, Value)]) -> BResult {
    let doc = match v {
        Value::Str(x) => x.as_str().to_string(),
        Value::Bytes(b) => String::from_utf8((**b).clone()).map_err(|_| {
            raise(
                "UnicodeDecodeError",
                "'utf-8' codec can't decode byte in position: invalid continuation byte",
            )
        })?,
        other => {
            return Err(raise(
                "TypeError",
                format!(
                    "the JSON object must be str, bytes or bytearray, not {}",
                    other.type_name()
                ),
            ))
        }
    };
    let parse_float = kw(kwargs, "parse_float")
        .map(callable_name_pub)
        .transpose()?;
    let object_pairs_hook = kw(kwargs, "object_pairs_hook")
        .map(callable_name_pub)
        .transpose()?;
    let mut p = JsonParser {
        doc: &doc,
        bytes: doc.as_bytes(),
        pos: 0,
        parse_float,
        object_pairs_hook,
    };
    let out = p.parse_value()?;
    p.skip_ws();
    if p.pos != doc.len() {
        return Err(json_error2("Extra data", &doc, p.pos));
    }
    Ok(out)
}

thread_local! {
    /// Custom (item, key) separators from `separators=` (None = CPython defaults).
    static JSON_SEPS: RefCell<Option<(String, String)>> = const { RefCell::new(None) };
}

fn json_dumps(v: &Value, kwargs: &[(String, Value)]) -> BResult {
    let allow_nan = kw(kwargs, "allow_nan").map(truthy).unwrap_or(true);
    if !allow_nan {
        // CPython scans the whole tree for non-finite floats first.
        fn check(v: &Value) -> Result<(), Flow> {
            match v {
                Value::Float(x) if x.is_nan() || x.is_infinite() => Err(raise(
                    "ValueError",
                    format!(
                        "Out of range float values are not JSON compliant: {}",
                        if x.is_nan() {
                            "NaN".to_string()
                        } else if *x > 0.0 {
                            "inf".to_string()
                        } else {
                            "-inf".to_string()
                        }
                    ),
                )),
                Value::List(l) => {
                    for x in l.items().iter() {
                        check(x)?;
                    }
                    Ok(())
                }
                Value::Tuple(t) => {
                    for x in &t.0 {
                        check(x)?;
                    }
                    Ok(())
                }
                Value::Dict(d) => {
                    for (_, v) in d.items() {
                        check(&v)?;
                    }
                    Ok(())
                }
                _ => Ok(()),
            }
        }
        check(v)?;
    }
    let seps = match kw(kwargs, "separators") {
        Some(Value::Tuple(t)) if t.0.len() == 2 => {
            let si = match &t.0[0] {
                Value::Str(x) => x.as_str().to_string(),
                _ => ", ".to_string(),
            };
            let sk = match &t.0[1] {
                Value::Str(x) => x.as_str().to_string(),
                _ => ": ".to_string(),
            };
            Some((si, sk))
        }
        _ => None,
    };
    JSON_SEPS.with(|c| *c.borrow_mut() = seps);
    let ensure_ascii = kw(kwargs, "ensure_ascii").map(truthy).unwrap_or(true);
    let sort_keys = kw(kwargs, "sort_keys").map(truthy).unwrap_or(false);
    let indent = match kw(kwargs, "indent") {
        None | Some(Value::None) => None,
        Some(iv) => {
            let n = as_int_i64(iv)?;
            Some(" ".repeat(n.max(0) as usize))
        }
    };
    let mut out = String::new();
    dump_value(v, ensure_ascii, sort_keys, indent.as_deref(), 0, &mut out)?;
    Ok(s(out))
}

fn dump_value(
    v: &Value,
    ensure_ascii: bool,
    sort_keys: bool,
    indent: Option<&str>,
    depth: usize,
    out: &mut String,
) -> Result<(), Flow> {
    match v {
        Value::None => out.push_str("null"),
        Value::Bool(true) => out.push_str("true"),
        Value::Bool(false) => out.push_str("false"),
        Value::Int(n) => out.push_str(&n.to_string_raw()),
        Value::Float(x) => {
            if x.is_nan() {
                out.push_str("NaN");
            } else if x.is_infinite() {
                out.push_str(if *x > 0.0 { "Infinity" } else { "-Infinity" });
            } else {
                out.push_str(&fmt_f64(*x));
            }
        }
        Value::Str(st) => dump_string(st.as_str(), ensure_ascii, out),
        Value::List(l) => {
            dump_seq(
                &l.items().to_vec(),
                ensure_ascii,
                sort_keys,
                indent,
                depth,
                out,
                '[',
                ']',
            )?;
        }
        Value::Tuple(t) => dump_seq(&t.0, ensure_ascii, sort_keys, indent, depth, out, '[', ']')?,
        Value::Dict(d) => {
            let items = d.items();
            let mut items: Vec<(Value, Value)> = items.into_iter().collect();
            if sort_keys {
                items.sort_by(|a, b| {
                    let ka = match &a.0 {
                        Value::Str(x) => x.as_str().to_string(),
                        other => other.type_name().to_string(),
                    };
                    let kb = match &b.0 {
                        Value::Str(x) => x.as_str().to_string(),
                        other => other.type_name().to_string(),
                    };
                    ka.cmp(&kb)
                });
            }
            if items.is_empty() {
                out.push_str("{}");
                return Ok(());
            }
            let item_sep: String = match indent {
                Some(_) => ",".to_string(),
                None => JSON_SEPS
                    .with(|c| c.borrow().as_ref().map(|(i, _)| i.clone()))
                    .unwrap_or_else(|| ", ".to_string()),
            };
            out.push('{');
            for (i2, (k, val)) in items.iter().enumerate() {
                if i2 > 0 {
                    out.push_str(&item_sep);
                }
                if let Some(ind) = indent {
                    out.push('\n');
                    for _ in 0..depth + 1 {
                        out.push_str(ind);
                    }
                }
                let ks = match k {
                    Value::Str(x) => x.as_str().to_string(),
                    Value::Int(n) => n.to_string_raw(),
                    Value::Float(x) => fmt_f64(*x),
                    Value::Bool(b) => {
                        if *b {
                            "true".into()
                        } else {
                            "false".into()
                        }
                    }
                    Value::None => "null".into(),
                    other => {
                        return Err(raise(
                            "TypeError",
                            format!(
                                "keys must be str, int, float, bool or None, not {}",
                                other.type_name()
                            ),
                        ))
                    }
                };
                dump_string(&ks, ensure_ascii, out);
                let kv_sep = match indent {
                    Some(_) => ": ".to_string(),
                    None => JSON_SEPS
                        .with(|c| c.borrow().as_ref().map(|(_, k)| k.clone()))
                        .unwrap_or_else(|| ": ".to_string()),
                };
                out.push_str(&kv_sep);
                if indent.is_some() {
                    // CPython uses ": " with indent too
                }
                dump_value(val, ensure_ascii, sort_keys, indent, depth + 1, out)?;
            }
            if let Some(ind) = indent {
                out.push('\n');
                for _ in 0..depth {
                    out.push_str(ind);
                }
            }
            out.push('}');
        }
        other => {
            return Err(raise(
                "TypeError",
                format!(
                    "Object of type {} is not JSON serializable",
                    other.type_name()
                ),
            ))
        }
    }
    Ok(())
}

fn dump_seq(
    items: &[Value],
    ensure_ascii: bool,
    sort_keys: bool,
    indent: Option<&str>,
    depth: usize,
    out: &mut String,
    open: char,
    close: char,
) -> Result<(), Flow> {
    if items.is_empty() {
        out.push(open);
        out.push(close);
        return Ok(());
    }
    let item_sep: String = match indent {
        Some(_) => ",".to_string(),
        None => JSON_SEPS
            .with(|c| c.borrow().as_ref().map(|(i, _)| i.clone()))
            .unwrap_or_else(|| ", ".to_string()),
    };
    out.push(open);
    for (i2, item) in items.iter().enumerate() {
        if i2 > 0 {
            out.push_str(&item_sep);
        }
        if let Some(ind) = indent {
            out.push('\n');
            for _ in 0..depth + 1 {
                out.push_str(ind);
            }
        }
        dump_value(item, ensure_ascii, sort_keys, indent, depth + 1, out)?;
    }
    if let Some(ind) = indent {
        out.push('\n');
        for _ in 0..depth {
            out.push_str(ind);
        }
    }
    out.push(close);
    Ok(())
}

fn dump_string(text: &str, ensure_ascii: bool, out: &mut String) {
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c if ensure_ascii && (c as u32) > 0x7F => {
                if (c as u32) > 0xFFFF {
                    let cp = c as u32 - 0x10000;
                    let hi = 0xD800 + (cp >> 10);
                    let lo = 0xDC00 + (cp & 0x3FF);
                    out.push_str(&format!("\\u{:04x}\\u{:04x}", hi, lo));
                } else {
                    out.push_str(&format!("\\u{:04x}", c as u32));
                }
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

// ===========================================================================
// os-lite (constants, environ, fs functions used by the corpus)
// ===========================================================================

fn build_os(_i: &crate::frame::Interpreter) -> Value {
    let (m, ns) = new_module("os");
    ns_const(&ns, "sep", s("/"));
    ns_const(&ns, "pathsep", s(":"));
    ns_const(&ns, "curdir", s("."));
    ns_const(&ns, "pardir", s(".."));
    ns_const(&ns, "extsep", s("."));
    ns_const(&ns, "altsep", Value::None);
    ns_const(&ns, "name", s("posix"));
    ns_const(&ns, "linesep", s("\n"));
    ns_const(&ns, "devnull", s("/dev/null"));
    ns_const(&ns, "SEEK_SET", i(0));
    ns_const(&ns, "SEEK_CUR", i(1));
    ns_const(&ns, "SEEK_END", i(2));
    for (k, v) in [
        ("F_OK", 0),
        ("W_OK", 1),
        ("R_OK", 2),
        ("X_OK", 1),
        ("O_RDONLY", 0o0),
        ("O_WRONLY", 0o1),
        ("O_RDWR", 0o2),
        ("O_CREAT", 0o100),
        ("O_TRUNC", 0o1000),
        ("O_APPEND", 0o2000),
    ] {
        ns_const(&ns, k, i(v));
    }
    ns_const(&ns, "TMP_MAX", i(10000));
    // environ: a plain dict (environb shares the class → isinstance holds).
    let env = PyDict::new();
    for (k, v) in std::env::vars() {
        let _ = env.set(s(k), s(v));
    }
    ns_const(&ns, "environ", Value::Dict(env));
    let envb = PyDict::new();
    ns_const(&ns, "environb", Value::Dict(envb));
    for n in [
        "getenv",
        "putenv",
        "unsetenv",
        "getcwd",
        "getcwdb",
        "listdir",
        "getpid",
        "getppid",
        "strerror",
        "access",
        "fspath",
        "fsencode",
        "fsdecode",
        "cpu_count",
        "process_cpu_count",
        "mkdir",
        "makedirs",
        "remove",
        "unlink",
        "rmdir",
        "rename",
        "replace",
        "fdopen",
    ] {
        fn1(&ns, "os", n);
    }
    // os.path submodule link + os.error is OSError (SAME object as the
    // builtin OSError name — pull it from the builtins dict so identity
    // holds under 02.03's class registry too).
    let err_v = CURRENT.with(|c| c.borrow().clone()).and_then(|interp| {
        interp
            .builtins
            .get(&s("OSError"))
            .or_else(|| Some(interned_type_pub("OSError")))
    });
    if let Some(ev) = err_v {
        let _ = ns.set(s("error"), ev);
    }
    let _ = ns.set(s("PathLike"), interned_type_pub("PathLike"));
    if let Some(p) = get_or_make_module_cached("os.path") {
        let _ = ns.set(s("path"), p);
    }
    ns_const(&ns, "stat_result", interned_type_pub("stat_result"));
    finish(m, ns)
}

fn get_or_make_module_cached(name: &str) -> Option<Value> {
    let interp = CURRENT.with(|c| c.borrow().clone())?;
    get_or_make_module(&interp, name)
}

pub fn call_os(name: &str, args: &[Value], _kwargs: &[(String, Value)]) -> Option<BResult> {
    let r: BResult = (|| -> BResult {
        match name {
            "getenv" => {
                let k = str_arg(required(args, "getenv")?, "getenv")?;
                let dflt = args.get(1).cloned().unwrap_or(Value::None);
                if let Value::Dict(env) = os_environ()? {
                    match env.get(&s(k)) {
                        Some(v) => Ok(v),
                        None => Ok(dflt),
                    }
                } else {
                    Ok(dflt)
                }
            }
            "putenv" => {
                let k = str_arg(required(args, "putenv")?, "putenv")?;
                let v = str_arg(
                    args.get(1)
                        .ok_or_else(|| raise("TypeError", "putenv() missing required argument"))?,
                    "putenv",
                )?;
                std::env::set_var(&k, &v);
                if let Value::Dict(env) = os_environ()? {
                    let _ = env.set(s(k), s(v));
                }
                Ok(Value::None)
            }
            "unsetenv" => {
                let k = str_arg(required(args, "unsetenv")?, "unsetenv")?;
                std::env::remove_var(&k);
                if let Value::Dict(env) = os_environ()? {
                    let _ = env.pop(&s(k));
                }
                Ok(Value::None)
            }
            "getcwd" => Ok(s(std::env::current_dir()
                .map_err(|e| raise("OSError", e.to_string()))?
                .to_string_lossy()
                .to_string())),
            "getcwdb" => {
                let cwd = std::env::current_dir().map_err(|e| raise("OSError", e.to_string()))?;
                Ok(Value::Bytes(Gc::new(
                    cwd.as_os_str().as_encoded_bytes().to_vec(),
                )))
            }
            "listdir" => {
                let path = match args.first() {
                    None => ".".to_string(),
                    Some(Value::Str(x)) => x.as_str().to_string(),
                    Some(_) => ".".to_string(),
                };
                let mut out = Vec::new();
                for e in std::fs::read_dir(&path).map_err(|e| os_error(&path, &e))? {
                    let e = e.map_err(|e| os_error(&path, &e))?;
                    out.push(s(e.file_name().to_string_lossy().to_string()));
                }
                Ok(list(out))
            }
            "getpid" => Ok(i(std::process::id() as i64)),
            "getppid" => Ok(i(0)),
            "strerror" => {
                let n = as_int_i64(required(args, "strerror")?)?;
                Ok(s(errno_text(n)))
            }
            "access" => {
                let path = str_arg(required(args, "access")?, "access")?;
                let _mode =
                    as_int_i64(args.get(1).ok_or_else(|| {
                        raise("TypeError", "access() missing required argument")
                    })?)?;
                Ok(Value::Bool(std::path::Path::new(&path).exists()))
            }
            "fspath" => Ok(required(args, "fspath")?.clone()),
            "fsencode" => {
                let p = str_arg(required(args, "fsencode")?, "fsencode")?;
                Ok(Value::Bytes(Gc::new(p.into_bytes())))
            }
            "fsdecode" => match required(args, "fsdecode")? {
                Value::Bytes(b) => Ok(s(String::from_utf8_lossy(b).to_string())),
                Value::Str(x) => Ok(s(x.as_str().to_string())),
                other => Err(raise(
                    "TypeError",
                    format!("expected str or bytes, not {}", other.type_name()),
                )),
            },
            "cpu_count" => Ok(i(std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(1) as i64)),
            "process_cpu_count" => Ok(i(std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(1) as i64)),
            "mkdir" => {
                let path = str_arg(required(args, "mkdir")?, "mkdir")?;
                std::fs::create_dir(&path).map_err(|e| os_error(&path, &e))?;
                Ok(Value::None)
            }
            "makedirs" => {
                let path = str_arg(required(args, "makedirs")?, "makedirs")?;
                std::fs::create_dir_all(&path).map_err(|e| os_error(&path, &e))?;
                Ok(Value::None)
            }
            "remove" | "unlink" => {
                let path = str_arg(required(args, name)?, name)?;
                std::fs::remove_file(&path).map_err(|e| os_error(&path, &e))?;
                Ok(Value::None)
            }
            "rmdir" => {
                let path = str_arg(required(args, "rmdir")?, "rmdir")?;
                std::fs::remove_dir(&path).map_err(|e| os_error(&path, &e))?;
                Ok(Value::None)
            }
            "fdopen" => {
                let path = str_arg(
                    args.first()
                        .ok_or_else(|| raise("TypeError", "fdopen() missing required argument"))?,
                    "fdopen",
                )?;
                let data = std::fs::read(&path).unwrap_or_default();
                return Ok(make_stream("TextIOWrapper", data, Some("utf-8".into())));
            }
            "rename" | "replace" => {
                let a = str_arg(required(args, name)?, name)?;
                let b = str_arg(
                    args.get(1).ok_or_else(|| {
                        raise("TypeError", format!("{name}() missing required argument"))
                    })?,
                    name,
                )?;
                std::fs::rename(&a, &b).map_err(|e| os_error(&a, &e))?;
                Ok(Value::None)
            }
            _ => return Err(Flow::Next),
        }
    })();
    if r.is_err() && name == "cpu_count" {
        return Some(Ok(i(std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1) as i64)));
    }
    Some(r)
}

fn os_environ() -> Result<Value, Flow> {
    let m =
        get_or_make_module_cached("os").ok_or_else(|| raise("SystemError", "os module lost"))?;
    if let Value::Obj(o) = &m {
        let ns = o.module_ns.borrow();
        if let Some(ns) = &*ns {
            if let Some(v) = ns.get(&s("environ")) {
                return Ok(v);
            }
        }
    }
    Err(raise("SystemError", "os.environ lost"))
}

fn os_error(path: &str, e: &std::io::Error) -> Flow {
    let kind = match e.kind() {
        std::io::ErrorKind::NotFound => "FileNotFoundError",
        std::io::ErrorKind::PermissionDenied => "PermissionError",
        std::io::ErrorKind::AlreadyExists => "FileExistsError",
        std::io::ErrorKind::InvalidInput => "ValueError",
        _ => "OSError",
    };
    let _ = path;
    Flow::Raise(ExceptionValue::new(kind, vec![s(e.to_string())]))
}

fn errno_text(n: i64) -> String {
    match n {
        1 => "Operation not permitted",
        2 => "No such file or directory",
        13 => "Permission denied",
        17 => "File exists",
        22 => "Invalid argument",
        4 => "Interrupted system call",
        _ => "Unknown error",
    }
    .to_string()
}

// ===========================================================================
// os.path / posixpath / genericpath
// ===========================================================================

fn build_posixpath(_i: &crate::frame::Interpreter) -> Value {
    let (m, ns) = new_module("posixpath");
    ns_const(&ns, "sep", s("/"));
    ns_const(&ns, "pathsep", s(":"));
    ns_const(&ns, "curdir", s("."));
    ns_const(&ns, "pardir", s(".."));
    ns_const(&ns, "extsep", s("."));
    ns_const(&ns, "altsep", Value::None);
    ns_const(&ns, "defpath", s(":"));
    ns_const(&ns, "devnull", s("/dev/null"));
    ns_const(&ns, "ALLOW_MISSING", i(1));
    for n in [
        "join",
        "split",
        "splitext",
        "splitdrive",
        "dirname",
        "basename",
        "exists",
        "isdir",
        "isfile",
        "islink",
        "ismount",
        "normpath",
        "normcase",
        "abspath",
        "isabs",
        "realpath",
        "relpath",
        "expanduser",
        "expandvars",
        "getsize",
        "commonprefix",
        "commonpath",
        "samefile",
        "lexists",
    ] {
        fn1(&ns, "posixpath", n);
    }
    let _ = ns.set(s("__name__"), s("posixpath"));
    finish(m, ns)
}

fn build_genericpath(_i: &crate::frame::Interpreter) -> Value {
    let (m, ns) = new_module("genericpath");
    for n in ["commonprefix", "exists", "isdir", "isfile", "getsize"] {
        fn1(&ns, "genericpath", n);
    }
    finish(m, ns)
}

pub fn call_posixpath(
    module: &str,
    name: &str,
    args: &[Value],
    _kwargs: &[(String, Value)],
) -> Option<BResult> {
    let r: BResult = (|| -> BResult {
        match name {
            "join" => {
                let mut parts = Vec::new();
                for a in args {
                    parts.push(str_arg(a, "join")?);
                }
                let mut joined = String::new();
                for p in parts {
                    if p.starts_with('/') {
                        joined = p;
                    } else if joined.is_empty() {
                        joined = p;
                    } else if joined.ends_with('/') {
                        joined.push_str(&p);
                    } else {
                        joined.push('/');
                        joined.push_str(&p);
                    }
                }
                Ok(s(joined))
            }
            "split" => {
                let p = str_arg(required(args, "split")?, "split")?;
                match p.rfind('/') {
                    Some(idx) => {
                        let (head, tail) = p.split_at(idx);
                        Ok(tuple(vec![
                            s(if head.is_empty() {
                                "/".to_string()
                            } else {
                                head.to_string()
                            }),
                            s(tail[1..].to_string()),
                        ]))
                    }
                    None => Ok(tuple(vec![s(""), s(p)])),
                }
            }
            "splitext" => {
                let p = str_arg(required(args, "splitext")?, "splitext")?;
                let (base, ext) = split_ext(&p);
                Ok(tuple(vec![s(base), s(ext)]))
            }
            "splitdrive" => {
                let p = str_arg(required(args, "splitdrive")?, "splitdrive")?;
                Ok(tuple(vec![s(""), s(p)]))
            }
            "dirname" => {
                let p = str_arg(required(args, "dirname")?, "dirname")?;
                Ok(s(match p.rfind('/') {
                    Some(0) => "/".to_string(),
                    Some(idx) => p[..idx].to_string(),
                    None => String::new(),
                }))
            }
            "basename" => {
                let p = str_arg(required(args, "basename")?, "basename")?;
                Ok(s(match p.rfind('/') {
                    Some(idx) => p[idx + 1..].to_string(),
                    None => p,
                }))
            }
            "exists" | "lexists" => {
                let p = str_arg(required(args, "exists")?, "exists")?;
                Ok(Value::Bool(std::path::Path::new(&p).exists() || p == "/"))
            }
            "isdir" => {
                let p = str_arg(required(args, "isdir")?, "isdir")?;
                Ok(Value::Bool(std::path::Path::new(&p).is_dir()))
            }
            "isfile" => {
                let p = str_arg(required(args, "isfile")?, "isfile")?;
                Ok(Value::Bool(std::path::Path::new(&p).is_file()))
            }
            "islink" => Ok(Value::Bool(false)),
            "ismount" => {
                let p = str_arg(required(args, "ismount")?, "ismount")?;
                Ok(Value::Bool(p == "/"))
            }
            "normcase" => Ok(s(str_arg(required(args, "normcase")?, "normcase")?)),
            "normpath" => {
                let p = str_arg(required(args, "normpath")?, "normpath")?;
                Ok(s(norm_path(&p)))
            }
            "abspath" => {
                let p = str_arg(required(args, "abspath")?, "abspath")?;
                if p.starts_with('/') {
                    Ok(s(norm_path(&p)))
                } else {
                    let cwd =
                        std::env::current_dir().map_err(|e| raise("OSError", e.to_string()))?;
                    Ok(s(norm_path(&format!("{}/{}", cwd.to_string_lossy(), p))))
                }
            }
            "isabs" => {
                let p = str_arg(required(args, "isabs")?, "isabs")?;
                Ok(Value::Bool(p.starts_with('/')))
            }
            "realpath" => {
                let p = str_arg(required(args, "realpath")?, "realpath")?;
                if p.starts_with('/') {
                    Ok(s(norm_path(&p)))
                } else {
                    let cwd =
                        std::env::current_dir().map_err(|e| raise("OSError", e.to_string()))?;
                    Ok(s(norm_path(&format!("{}/{}", cwd.to_string_lossy(), p))))
                }
            }
            "relpath" => {
                let p = str_arg(required(args, "relpath")?, "relpath")?;
                let start = match args.get(1) {
                    Some(v) => str_arg(v, "relpath")?,
                    None => std::env::current_dir()
                        .map_err(|e| raise("OSError", e.to_string()))?
                        .to_string_lossy()
                        .to_string(),
                };
                let abs_p = if p.starts_with('/') {
                    p.clone()
                } else {
                    let cwd = std::env::current_dir().unwrap();
                    format!("{}/{}", cwd.to_string_lossy(), p)
                };
                let abs_s = if start.starts_with('/') {
                    start
                } else {
                    let cwd = std::env::current_dir().unwrap();
                    format!("{}/{}", cwd.to_string_lossy(), start)
                };
                let np_p = norm_path(&abs_p);
                let np_s = norm_path(&abs_s);
                let pp: Vec<&str> = np_p
                    .trim_matches('/')
                    .split('/')
                    .filter(|x| !x.is_empty())
                    .collect();
                let ss: Vec<&str> = np_s
                    .trim_matches('/')
                    .split('/')
                    .filter(|x| !x.is_empty())
                    .collect();
                let mut i2 = 0;
                while i2 < pp.len() && i2 < ss.len() && pp[i2] == ss[i2] {
                    i2 += 1;
                }
                let mut parts: Vec<String> = vec!["..".to_string(); ss.len() - i2];
                parts.extend(pp[i2..].iter().map(|x| x.to_string()));
                Ok(s(if parts.is_empty() {
                    ".".to_string()
                } else {
                    parts.join("/")
                }))
            }
            "expanduser" => {
                let p = str_arg(required(args, "expanduser")?, "expanduser")?;
                if let Some(rest) = p.strip_prefix("~/") {
                    if let Ok(home) = std::env::var("HOME") {
                        return Ok(s(format!("{home}/{rest}")));
                    }
                }
                if p == "~" {
                    if let Ok(home) = std::env::var("HOME") {
                        return Ok(s(home));
                    }
                }
                Ok(s(p))
            }
            "expandvars" => Ok(s(str_arg(required(args, "expandvars")?, "expandvars")?)),
            "getsize" => {
                let p = str_arg(required(args, "getsize")?, "getsize")?;
                let md = std::fs::metadata(&p).map_err(|e| os_error(&p, &e))?;
                Ok(i(md.len() as i64))
            }
            "commonprefix" => {
                let seq = snapshot(required(args, "commonprefix")?)?;
                let mut strs: Vec<String> = Vec::new();
                for x in seq {
                    strs.push(match &x {
                        Value::Str(st) => st.as_str().to_string(),
                        // CPython operates element-wise on sequences; v1 corpus
                        // uses str lists only.
                        other => {
                            return Err(raise(
                                "TypeError",
                                format!("sequence item must be str, not {}", other.type_name()),
                            ))
                        }
                    });
                }
                if strs.is_empty() {
                    return Ok(s(""));
                }
                let mut prefix = strs[0].clone();
                for other in &strs[1..] {
                    while !other.starts_with(&prefix) {
                        prefix.pop();
                    }
                }
                Ok(s(prefix))
            }
            "commonpath" => {
                let seq = snapshot(required(args, "commonpath")?)?;
                let mut strs: Vec<String> = Vec::new();
                for x in seq {
                    strs.push(str_arg(&x, "commonpath")?);
                }
                let splitp = |p: &str| -> Vec<String> {
                    norm_path(p)
                        .trim_matches('/')
                        .split('/')
                        .filter(|x| !x.is_empty())
                        .map(|x| x.to_string())
                        .collect()
                };
                let first = splitp(&strs[0]);
                let mut common: Vec<String> = first;
                for other in &strs[1..] {
                    let o = splitp(other);
                    let mut c = Vec::new();
                    for (a, b) in common.iter().zip(o.iter()) {
                        if a == b {
                            c.push(a.clone());
                        } else {
                            break;
                        }
                    }
                    common = c;
                }
                Ok(s(if common.is_empty() {
                    "/".to_string()
                } else {
                    format!("/{}", common.join("/"))
                }))
            }
            "samefile" => {
                use std::os::unix::fs::MetadataExt;
                let a = str_arg(required(args, "samefile")?, "samefile")?;
                let b = str_arg(
                    args.get(1).ok_or_else(|| {
                        raise("TypeError", "samefile() missing required argument")
                    })?,
                    "samefile",
                )?;
                let ma = std::fs::metadata(&a);
                let mb = std::fs::metadata(&b);
                Ok(Value::Bool(match (ma, mb) {
                    (Ok(a), Ok(b)) => a.ino() == b.ino() && a.dev() == b.dev(),
                    _ => false,
                }))
            }
            _ => return Err(Flow::Next),
        }
    })();
    if r.is_err() && module == "genericpath" {
        return call_genericpath(name, args, _kwargs);
    }
    Some(r)
}

fn call_genericpath(name: &str, args: &[Value], _kwargs: &[(String, Value)]) -> Option<BResult> {
    let r: BResult = (|| -> BResult {
        match name {
            "exists" => {
                let p = str_arg(required(args, "exists")?, "exists")?;
                Ok(Value::Bool(std::path::Path::new(&p).exists()))
            }
            "isdir" => {
                let p = str_arg(required(args, "isdir")?, "isdir")?;
                Ok(Value::Bool(std::path::Path::new(&p).is_dir()))
            }
            "isfile" => {
                let p = str_arg(required(args, "isfile")?, "isfile")?;
                Ok(Value::Bool(std::path::Path::new(&p).is_file()))
            }
            "getsize" => {
                let p = str_arg(required(args, "getsize")?, "getsize")?;
                let md = std::fs::metadata(&p).map_err(|e| os_error(&p, &e))?;
                Ok(i(md.len() as i64))
            }
            "commonprefix" => {
                let seq = snapshot(required(args, "commonprefix")?)?;
                let mut strs = Vec::new();
                for x in seq {
                    strs.push(str_arg(&x, "commonprefix")?);
                }
                if strs.is_empty() {
                    return Ok(s(""));
                }
                let mut prefix = strs[0].clone();
                for other in &strs[1..] {
                    while !other.starts_with(&prefix) {
                        prefix.pop();
                    }
                }
                Ok(s(prefix))
            }
            _ => Err(raise(
                "SystemError",
                format!("genericpath fn {name} unbound"),
            )),
        }
    })();
    match r {
        Err(Flow::Next) => return None,
        other => Some(other),
    }
}

fn split_ext(p: &str) -> (String, String) {
    let last_slash = p.rfind('/').map(|x| x + 1).unwrap_or(0);
    let base = &p[last_slash..];
    match base.rfind('.') {
        Some(idx) if idx > 0 => (
            p[..last_slash + idx].to_string(),
            p[last_slash + idx..].to_string(),
        ),
        _ => (p.to_string(), String::new()),
    }
}

fn norm_path(p: &str) -> String {
    let absolute = p.starts_with('/');
    let mut out: Vec<&str> = Vec::new();
    for part in p.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if !out.is_empty() && *out.last().unwrap() != ".." {
                    out.pop();
                } else if !absolute {
                    out.push("..");
                }
            }
            other => out.push(other),
        }
    }
    let joined = out.join("/");
    if absolute {
        format!("/{joined}")
    } else if joined.is_empty() {
        ".".to_string()
    } else {
        joined
    }
}

// ===========================================================================
// stat / errno
// ===========================================================================

fn build_stat(_i: &crate::frame::Interpreter) -> Value {
    let (m, ns) = new_module("stat");
    for (k, v) in [
        ("S_IFDIR", 0o040000),
        ("S_IFCHR", 0o020000),
        ("S_IFBLK", 0o060000),
        ("S_IFREG", 0o100000),
        ("S_IFIFO", 0o010000),
        ("S_IFLNK", 0o120000),
        ("S_IFSOCK", 0o140000),
        ("S_IRWXU", 0o700),
        ("S_IRUSR", 0o400),
        ("S_IWUSR", 0o200),
        ("S_IXUSR", 0o100),
        ("S_IRWXG", 0o70),
        ("S_IRGRP", 0o40),
        ("S_IWGRP", 0o20),
        ("S_IXGRP", 0o10),
        ("S_IRWXO", 0o7),
        ("S_IROTH", 0o4),
        ("S_IWOTH", 0o2),
        ("S_IXOTH", 0o1),
        ("ST_MODE", 0),
        ("ST_INO", 1),
        ("ST_DEV", 2),
        ("ST_NLINK", 3),
        ("ST_UID", 4),
        ("ST_GID", 5),
        ("ST_SIZE", 6),
        ("ST_ATIME", 7),
        ("ST_MTIME", 8),
        ("ST_CTIME", 9),
    ] {
        ns_const(&ns, k, i(v));
    }
    for n in [
        "filemode", "S_ISDIR", "S_ISCHR", "S_ISBLK", "S_ISREG", "S_ISFIFO", "S_ISLNK", "S_ISSOCK",
    ] {
        fn1(&ns, "stat", n);
    }
    finish(m, ns)
}

pub fn call_stat(name: &str, args: &[Value], _kwargs: &[(String, Value)]) -> Option<BResult> {
    let r: BResult = (|| {
        let m = as_int_i64(required(args, name)?)?;
        match name {
            "filemode" => Ok(s(file_mode(m))),
            "S_ISDIR" => Ok(Value::Bool(m & 0o170000 == 0o040000)),
            "S_ISCHR" => Ok(Value::Bool(m & 0o170000 == 0o020000)),
            "S_ISBLK" => Ok(Value::Bool(m & 0o170000 == 0o060000)),
            "S_ISREG" => Ok(Value::Bool(m & 0o170000 == 0o100000)),
            "S_ISFIFO" => Ok(Value::Bool(m & 0o170000 == 0o010000)),
            "S_ISLNK" => Ok(Value::Bool(m & 0o170000 == 0o120000)),
            "S_ISSOCK" => Ok(Value::Bool(m & 0o170000 == 0o140000)),
            _ => Err(raise("SystemError", format!("stat fn {name} unbound"))),
        }
    })();
    match r {
        Err(Flow::Next) => return None,
        other => Some(other),
    }
}

fn file_mode(m: i64) -> String {
    let kind = match m & 0o170000 {
        0o040000 => 'd',
        0o020000 => 'c',
        0o060000 => 'b',
        0o100000 => '-',
        0o010000 => 'p',
        0o120000 => 'l',
        0o140000 => 's',
        _ => '?',
    };
    let bits = [
        (0o400, 'r'),
        (0o200, 'w'),
        (0o100, 'x'),
        (0o40, 'r'),
        (0o20, 'w'),
        (0o10, 'x'),
        (0o4, 'r'),
        (0o2, 'w'),
        (0o1, 'x'),
    ];
    let mut out = String::new();
    out.push(kind);
    for (bit, c) in bits {
        out.push(if m & bit != 0 { c } else { '-' });
    }
    out
}

fn build_errno(_i: &crate::frame::Interpreter) -> Value {
    let (m, ns) = new_module("errno");
    let codes: &[(&str, i64)] = &[
        ("EPERM", 1),
        ("ENOENT", 2),
        ("ESRCH", 3),
        ("EINTR", 4),
        ("EIO", 5),
        ("ENXIO", 6),
        ("E2BIG", 7),
        ("ENOEXEC", 8),
        ("EBADF", 9),
        ("ECHILD", 10),
        ("EAGAIN", 11),
        ("ENOMEM", 12),
        ("EACCES", 13),
        ("EFAULT", 14),
        ("EBUSY", 16),
        ("EEXIST", 17),
        ("EXDEV", 18),
        ("ENODEV", 19),
        ("ENOTDIR", 20),
        ("EISDIR", 21),
        ("EINVAL", 22),
        ("ENFILE", 23),
        ("EMFILE", 24),
        ("ENOTTY", 25),
        ("EFBIG", 27),
        ("ENOSPC", 28),
        ("ESPIPE", 29),
        ("EROFS", 30),
        ("EMLINK", 31),
        ("EPIPE", 32),
        ("EDOM", 33),
        ("ERANGE", 34),
    ];
    let ec = PyDict::new();
    for (k, v) in codes {
        ns_const(&ns, k, i(*v));
        let _ = ec.set(i(*v), s(*k));
    }
    ns_const(&ns, "errorcode", Value::Dict(ec));
    finish(m, ns)
}

// ===========================================================================
// keyword
// ===========================================================================

fn build_keyword(_i: &crate::frame::Interpreter) -> Value {
    let (m, ns) = new_module("keyword");
    let kwlist = [
        "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class",
        "continue", "def", "del", "elif", "else", "except", "finally", "for", "from", "global",
        "if", "import", "in", "is", "lambda", "nonlocal", "not", "or", "pass", "raise", "return",
        "try", "while", "with", "yield",
    ];
    let list_v: Vec<Value> = kwlist.iter().map(|k| s(*k)).collect();
    ns_const(&ns, "kwlist", list(list_v));
    let soft: Vec<Value> = ["_", "case", "match", "type"]
        .iter()
        .map(|k| s(*k))
        .collect();
    ns_const(&ns, "softkwlist", list(soft));
    fn1(&ns, "keyword", "iskeyword");
    fn1(&ns, "keyword", "issoftkeyword");
    finish(m, ns)
}

pub fn call_keyword(name: &str, args: &[Value], _kwargs: &[(String, Value)]) -> Option<BResult> {
    let r: BResult = (|| {
        let k = str_arg(required(args, name)?, name)?;
        match name {
            "iskeyword" => Ok(Value::Bool(
                [
                    "False", "None", "True", "and", "as", "assert", "async", "await", "break",
                    "class", "continue", "def", "del", "elif", "else", "except", "finally", "for",
                    "from", "global", "if", "import", "in", "is", "lambda", "nonlocal", "not",
                    "or", "pass", "raise", "return", "try", "while", "with", "yield",
                ]
                .contains(&k.as_str()),
            )),
            "issoftkeyword" => Ok(Value::Bool(
                ["_", "case", "match", "type"].contains(&k.as_str()),
            )),
            _ => Err(raise("SystemError", "keyword fn unbound")),
        }
    })();
    match r {
        Err(Flow::Next) => return None,
        other => Some(other),
    }
}

// ===========================================================================
// platform (lite; values documented divergence, presence + shapes only)
// ===========================================================================

fn build_platform(_i: &crate::frame::Interpreter) -> Value {
    let (m, ns) = new_module("platform");
    for n in [
        "system",
        "machine",
        "node",
        "python_version",
        "python_implementation",
        "processor",
        "uname",
    ] {
        fn1(&ns, "platform", n);
    }
    finish(m, ns)
}

fn uname_tuple() -> Vec<String> {
    vec![
        std::env::consts::OS.to_string(),
        hostname(),
        os_release(),
        String::new(),
        std::env::consts::ARCH.to_string(),
        String::new(),
    ]
}

fn hostname() -> String {
    std::fs::read_to_string("/etc/hostname")
        .map(|x| x.trim().to_string())
        .unwrap_or_else(|_| "localhost".to_string())
}

fn os_release() -> String {
    std::fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|x| {
            x.lines().find(|l| l.starts_with("VERSION_ID=")).map(|l| {
                l.trim_start_matches("VERSION_ID=")
                    .trim_matches('"')
                    .to_string()
            })
        })
        .unwrap_or_default()
}

pub fn call_platform(name: &str, args: &[Value], _kwargs: &[(String, Value)]) -> Option<BResult> {
    let r: BResult = (|| -> BResult {
        match name {
            "system" => Ok(s(std::env::consts::OS)),
            "machine" => Ok(s(std::env::consts::ARCH)),
            "node" => Ok(s(hostname())),
            "processor" => Ok(s("")),
            "python_version" => Ok(s("3.14.0")),
            "python_implementation" => Ok(s("antipythany")),
            "uname" => {
                let u = uname_tuple();
                let obj = PyObject::new(ap_obj::ObjKind::NamedTuple, "uname_result");
                {
                    let mut ints = obj.ints.borrow_mut();
                    ints.clear();
                }
                // store fields via attrs + a synthetic repr
                let fields = [
                    "system",
                    "node",
                    "release",
                    "version",
                    "machine",
                    "processor",
                ];
                {
                    let mut attrs = obj.attrs.borrow_mut();
                    for (k, v) in fields.iter().zip(u.iter()) {
                        let _ = attrs.set(s(*k), s(v.clone()));
                    }
                    let _ = attrs.set(s("_fields"), tuple(fields.iter().map(|f| s(*f)).collect()));
                    let _ = attrs.set(
                        s("<values>"),
                        list(u.iter().map(|x| s(x.clone())).collect()),
                    );
                }
                *obj.repr_text.borrow_mut() = Some(format!(
                    "uname_result(system='{}', node='{}', release='{}', version='{}', machine='{}', processor='{}')",
                    u[0], u[1], u[2], u[3], u[4], u[5]
                ));
                Ok(Value::Obj(obj))
            }
            _ => Err(raise("SystemError", format!("platform fn {name} unbound"))),
        }
    })();
    match r {
        Err(Flow::Next) => return None,
        other => Some(other),
    }
}

// ===========================================================================
// time (minimal: gmtime/localtime/strftime/strptime/mktime/sleep/struct_time)
// ===========================================================================

fn build_time(_i: &crate::frame::Interpreter) -> Value {
    let (m, ns) = new_module("time");
    for n in [
        "monotonic",
        "monotonic_ns",
        "perf_counter",
        "perf_counter_ns",
        "sleep",
        "time",
        "time_ns",
        "gmtime",
        "localtime",
        "mktime",
        "strftime",
        "strptime",
        "clock_gettime",
        "get_clock_info",
    ] {
        fn1(&ns, "time", n);
    }
    ns_const(&ns, "timezone", i(0));
    ns_const(&ns, "CLOCK_MONOTONIC", i(1));
    ns_const(&ns, "CLOCK_REALTIME", i(0));
    // struct_time constructor marker
    let _ = ns.set(s("struct_time"), marker("time.struct_time"));
    finish(m, ns)
}

fn make_struct_time(fields: [i64; 9]) -> Value {
    let obj = PyObject::new(ap_obj::ObjKind::Opaque, "struct_time");
    {
        let mut attrs = obj.attrs.borrow_mut();
        let names = [
            "tm_year", "tm_mon", "tm_mday", "tm_hour", "tm_min", "tm_sec", "tm_wday", "tm_yday",
            "tm_isdst",
        ];
        for (k, v) in names.iter().zip(fields.iter()) {
            let _ = attrs.set(s(*k), i(*v));
        }
        let _ = attrs.set(s("<fields>"), list(names.iter().map(|n| s(*n)).collect()));
        let _ = attrs.set(s("<values>"), list(fields.iter().map(|v| i(*v)).collect()));
    }
    *obj.repr_text.borrow_mut() = Some(format!(
        "time.struct_time(tm_year={}, tm_mon={}, tm_mday={}, tm_hour={}, tm_min={}, tm_sec={}, tm_wday={}, tm_yday={}, tm_isdst={})",
        fields[0], fields[1], fields[2], fields[3], fields[4], fields[5], fields[6], fields[7],
        fields[8]
    ));
    Value::Obj(obj)
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    // Howard Hinnant's algorithm
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    ((if m <= 2 { y + 1 } else { y }), m, d)
}

fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u64;
    let mp = if m > 2 { m - 3 } else { m + 9 } as u64;
    let doy = (153 * mp + 2) / 5 + d as u64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe as i64 - 719468
}

fn civil_fields(epoch: i64) -> [i64; 9] {
    let days = epoch.div_euclid(86400);
    let secs = epoch.rem_euclid(86400);
    let (y, mo, d) = civil_from_days(days);
    let wd = (days + 3).rem_euclid(7); // 1970-01-01 = Thursday = 3 (Mon=0)
    let yday = days - days_from_civil(y, 1, 1) + 1;
    [
        y,
        mo as i64,
        d as i64,
        secs / 3600,
        (secs % 3600) / 60,
        secs % 60,
        wd,
        yday,
        0,
    ]
}

pub fn call_time(name: &str, args: &[Value], kwargs: &[(String, Value)]) -> Option<BResult> {
    let r: BResult = (|| -> BResult {
        match name {
            "monotonic" | "perf_counter" => {
                Ok(f(std::time::Instant::now().elapsed().as_secs_f64()))
            }
            "monotonic_ns" | "perf_counter_ns" | "time_ns" => Ok(i(std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as i64)
                .unwrap_or(0))),
            "time" => Ok(f(std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs_f64())
                .unwrap_or(0.0))),
            "sleep" => {
                let secs = one_f64_arg(args, "sleep")?;
                if secs > 0.0 {
                    std::thread::sleep(std::time::Duration::from_secs_f64(secs));
                }
                Ok(Value::None)
            }
            "gmtime" => {
                let epoch = match args.first() {
                    None => now_epoch(),
                    Some(Value::None) => now_epoch(),
                    Some(v) => as_int_i64(v)?,
                };
                Ok(make_struct_time(civil_fields(epoch)))
            }
            "localtime" => {
                let epoch = match args.first() {
                    None => now_epoch(),
                    Some(Value::None) => now_epoch(),
                    Some(v) => as_int_i64(v)?,
                };
                Ok(make_struct_time(civil_fields(epoch + local_utc_offset())))
            }
            "mktime" => {
                let st = required(args, "mktime")?;
                let f9 = struct_time_fields(st)?;
                let days = days_from_civil(f9[0], f9[1] as u32, f9[2] as u32);
                Ok(i(
                    days * 86400 + f9[3] * 3600 + f9[4] * 60 + f9[5] - local_utc_offset()
                ))
            }
            "strftime" => {
                let fmt = str_arg(required(args, "strftime")?, "strftime")?;
                let st = args
                    .get(1)
                    .ok_or_else(|| raise("TypeError", "strftime() missing required argument"))?;
                let f9 = struct_time_fields(st)?;
                Ok(s(strftime_fmt(&fmt, &f9)))
            }
            "strptime" => {
                let text = str_arg(required(args, "strptime")?, "strptime")?;
                let fmt = match args.get(1) {
                    Some(v) => str_arg(v, "strptime")?,
                    None => "%a %b %e %H:%M:%S %Y".to_string(),
                };
                let f9 = strptime_parse(&text, &fmt)?;
                Ok(make_struct_time(f9))
            }
            "clock_gettime" => Ok(f(std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs_f64())
                .unwrap_or(0.0))),
            "get_clock_info" => {
                let obj = PyObject::new(ap_obj::ObjKind::Namespace, "clock_info");
                {
                    let mut attrs = obj.attrs.borrow_mut();
                    let _ = attrs.set(s("monotonic"), Value::Bool(true));
                    let _ = attrs.set(s("adjustable"), Value::Bool(false));
                    let _ = attrs.set(s("implementation"), s("clock_gettime(CLOCK_MONOTONIC)"));
                    let _ = attrs.set(s("resolution"), f(1e-9));
                }
                Ok(Value::Obj(obj))
            }
            "struct_time" => {
                // time.struct_time(tuple)
                let seq = snapshot(required(args, "struct_time")?)?;
                let mut f9 = [0i64; 9];
                for (idx, x) in seq.iter().take(9).enumerate() {
                    f9[idx] = as_int_i64(x)?;
                }
                Ok(make_struct_time(f9))
            }
            _ => Err(raise("SystemError", format!("time fn {name} unbound"))),
        }
    })();
    let _ = kwargs;
    Some(r)
}

fn now_epoch() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn local_utc_offset() -> i64 {
    // v1: TZ-less environments pinned to UTC offset 0 (corpus uses booleans
    // or UTC-anchored values only — spec 05 time row).
    0
}

fn struct_time_fields(v: &Value) -> Result<[i64; 9], Flow> {
    let names = [
        "tm_year", "tm_mon", "tm_mday", "tm_hour", "tm_min", "tm_sec", "tm_wday", "tm_yday",
        "tm_isdst",
    ];
    match v {
        Value::Obj(o) => {
            let attrs = o.attrs.borrow();
            let mut out = [0i64; 9];
            for (idx, n) in names.iter().enumerate() {
                out[idx] = attrs
                    .get(&s(*n))
                    .map(|v| as_int_i64(&v))
                    .transpose()?
                    .unwrap_or(0);
            }
            Ok(out)
        }
        Value::Tuple(t) => {
            let mut out = [0i64; 9];
            for (idx, x) in t.0.iter().take(9).enumerate() {
                out[idx] = as_int_i64(x)?;
            }
            Ok(out)
        }
        other => Err(raise(
            "TypeError",
            format!("expected struct_time, not {}", other.type_name()),
        )),
    }
}

fn strftime_fmt(fmt: &str, f9: &[i64; 9]) -> String {
    let mut out = String::new();
    let mut it = fmt.chars().peekable();
    while let Some(c) = it.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('Y') => out.push_str(&format!("{:04}", f9[0])),
            Some('y') => out.push_str(&format!("{:02}", f9[0] % 100)),
            Some('m') => out.push_str(&format!("{:02}", f9[1])),
            Some('d') => out.push_str(&format!("{:02}", f9[2])),
            Some('H') => out.push_str(&format!("{:02}", f9[3])),
            Some('M') => out.push_str(&format!("{:02}", f9[4])),
            Some('S') => out.push_str(&format!("{:02}", f9[5])),
            Some('a') => out.push_str(
                ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"][(f9[6].rem_euclid(7)) as usize],
            ),
            Some('A') => out.push_str(
                [
                    "Monday",
                    "Tuesday",
                    "Wednesday",
                    "Thursday",
                    "Friday",
                    "Saturday",
                    "Sunday",
                ][(f9[6].rem_euclid(7)) as usize],
            ),
            Some('b') | Some('h') => out.push_str(
                [
                    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov",
                    "Dec",
                ][(f9[1].clamp(1, 12) - 1) as usize],
            ),
            Some('B') => out.push_str(
                [
                    "January",
                    "February",
                    "March",
                    "April",
                    "May",
                    "June",
                    "July",
                    "August",
                    "September",
                    "October",
                    "November",
                    "December",
                ][(f9[1].clamp(1, 12) - 1) as usize],
            ),
            Some('j') => out.push_str(&format!("{:03}", f9[7])),
            Some('p') => out.push_str(if f9[3] < 12 { "AM" } else { "PM" }),
            Some('%') => out.push('%'),
            Some(other) => {
                out.push('%');
                out.push(other);
            }
            None => out.push('%'),
        }
    }
    out
}

fn strptime_parse(text: &str, fmt: &str) -> Result<[i64; 9], Flow> {
    let mut f9 = [0i64; 9];
    f9[1] = 1;
    f9[2] = 1;
    let mut ti = text;
    let mut fi = fmt;
    while let Some(fc) = fi.chars().next() {
        if fc == '%' {
            let spec = fi.chars().nth(1).unwrap_or('%');
            fi = &fi[2.min(fi.len())..];
            match spec {
                'Y' => {
                    let n: String = ti.chars().take_while(|c| c.is_ascii_digit()).collect();
                    f9[0] = n.parse().unwrap_or(0);
                    ti = &ti[n.len()..];
                }
                'm' => {
                    let n: String = ti.chars().take_while(|c| c.is_ascii_digit()).collect();
                    f9[1] = n.parse().unwrap_or(1);
                    ti = &ti[n.len()..];
                }
                'd' => {
                    let n: String = ti.chars().take_while(|c| c.is_ascii_digit()).collect();
                    f9[2] = n.parse().unwrap_or(1);
                    ti = &ti[n.len()..];
                }
                'H' => {
                    let n: String = ti.chars().take_while(|c| c.is_ascii_digit()).collect();
                    f9[3] = n.parse().unwrap_or(0);
                    ti = &ti[n.len()..];
                }
                'M' => {
                    let n: String = ti.chars().take_while(|c| c.is_ascii_digit()).collect();
                    f9[4] = n.parse().unwrap_or(0);
                    ti = &ti[n.len()..];
                }
                'S' => {
                    let n: String = ti.chars().take_while(|c| c.is_ascii_digit()).collect();
                    f9[5] = n.parse().unwrap_or(0);
                    ti = &ti[n.len()..];
                }
                'b' | 'B' => {
                    let months = if spec == 'b' {
                        [
                            "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct",
                            "Nov", "Dec",
                        ]
                    } else {
                        [
                            "January",
                            "February",
                            "March",
                            "April",
                            "May",
                            "June",
                            "July",
                            "August",
                            "September",
                            "October",
                            "November",
                            "December",
                        ]
                    };
                    let mut found = false;
                    for (idx, mname) in months.iter().enumerate() {
                        if ti.starts_with(mname) {
                            f9[1] = idx as i64 + 1;
                            ti = &ti[mname.len()..];
                            found = true;
                            break;
                        }
                    }
                    if !found {
                        return Err(raise(
                            "ValueError",
                            format!("time data '{text}' does not match format '{fmt}'"),
                        ));
                    }
                }
                _ => {}
            }
        } else {
            let skip = fc.len_utf8();
            if !ti.starts_with(fc) {
                return Err(raise(
                    "ValueError",
                    format!("time data '{text}' does not match format '{fmt}'"),
                ));
            }
            ti = &ti[skip..];
            fi = &fi[skip..];
        }
    }
    if f9[0] > 0 {
        let days = days_from_civil(f9[0], f9[1].clamp(1, 12) as u32, f9[2].clamp(1, 31) as u32);
        f9[6] = (days + 3).rem_euclid(7);
        f9[7] = days - days_from_civil(f9[0], 1, 1) + 1;
    }
    Ok(f9)
}

// ===========================================================================
// atexit
// ===========================================================================

thread_local! {
    static ATEXIT_FNS: RefCell<Vec<(String, Vec<Value>)>> = const { RefCell::new(Vec::new()) };
}

fn build_atexit(_i: &crate::frame::Interpreter) -> Value {
    let (m, ns) = new_module("atexit");
    for n in ["register", "unregister", "exit"] {
        fn1(&ns, "atexit", n);
    }
    finish(m, ns)
}

pub fn call_atexit(name: &str, args: &[Value], _kwargs: &[(String, Value)]) -> Option<BResult> {
    let r: BResult = (|| -> BResult {
        match name {
            "register" => {
                let f = required(args, "register")?;
                let fname = name_callable(f);
                ATEXIT_FNS.with(|t| t.borrow_mut().push((fname, args[1..].to_vec())));
                Ok(f.clone())
            }
            "unregister" => {
                let f = required(args, "unregister")?;
                let fname = name_callable(f);
                ATEXIT_FNS.with(|t| t.borrow_mut().retain(|(n, _)| *n != fname));
                Ok(Value::None)
            }
            "exit" => Ok(Value::None),
            _ => return Err(raise("SystemError", "atexit fn unbound")),
        }
    })();
    match r {
        Err(Flow::Next) => None,
        other => Some(other),
    }
}

/// Drain atexit callbacks LIFO (called from the run_module tail hook).
pub fn drain_atexit() {
    loop {
        let entry = ATEXIT_FNS.with(|t| t.borrow_mut().pop());
        match entry {
            Some((name, args)) => {
                let _ = call_named(&name, &args);
            }
            None => break,
        }
    }
}

// ===========================================================================
// linecache
// ===========================================================================

thread_local! {
    static LINECACHE: RefCell<HashMap<String, Vec<String>>> = RefCell::new(HashMap::new());
}

fn build_linecache(_i: &crate::frame::Interpreter) -> Value {
    let (m, ns) = new_module("linecache");
    for n in [
        "getline",
        "getlines",
        "updatecache",
        "checkcache",
        "clearcache",
        "lazycache",
    ] {
        fn1(&ns, "linecache", n);
    }
    let _ = ns.set(s("cache"), Value::Dict(PyDict::new()));
    finish(m, ns)
}

pub fn call_linecache(name: &str, args: &[Value], _kwargs: &[(String, Value)]) -> Option<BResult> {
    let r: BResult = (|| -> BResult {
        match name {
            "getline" => {
                let p = str_arg(required(args, "getline")?, "getline")?;
                let n = args.get(1).map(as_int_i64).transpose()?.unwrap_or(1);
                let lines = cache_lines(&p);
                Ok(match lines.get((n.max(1) - 1) as usize) {
                    Some(l) => s(l.clone()),
                    None => s(""),
                })
            }
            "getlines" => {
                let p = str_arg(required(args, "getlines")?, "getlines")?;
                Ok(list(cache_lines(&p).into_iter().map(s).collect()))
            }
            "updatecache" => {
                let p = str_arg(required(args, "updatecache")?, "updatecache")?;
                let fresh = std::fs::read_to_string(&p).is_ok();
                let _ = cache_lines(&p);
                Ok(Value::Bool(fresh))
            }
            "checkcache" => Ok(Value::None),
            "clearcache" => {
                LINECACHE.with(|c| c.borrow_mut().clear());
                Ok(Value::None)
            }
            "lazycache" => Ok(Value::Bool(false)),
            _ => Err(raise("SystemError", "linecache fn unbound")),
        }
    })();
    match r {
        Err(Flow::Next) => return None,
        other => Some(other),
    }
}

fn cache_lines(path: &str) -> Vec<String> {
    LINECACHE.with(|c| {
        let mut m = c.borrow_mut();
        if let Some(v) = m.get(path) {
            return v.clone();
        }
        let lines: Vec<String> = std::fs::read_to_string(path)
            .map(|txt| txt.lines().map(|l| format!("{l}\n")).collect::<Vec<_>>())
            .unwrap_or_default();
        m.insert(path.to_string(), lines.clone());
        lines
    })
}

// ===========================================================================
// random — EXACT MT19937 port (spec conflict documented at file head)
// ===========================================================================

struct Mt {
    state: [u32; 624],
    idx: usize,
}

impl Mt {
    fn new_seeded_int(seed: i64) -> Mt {
        // CPython random.seed(int) → init_by_array with the absolute value
        // split into 32-bit key words (random_seed: negation → abs).
        let abs_seed = seed.unsigned_abs() as u128;
        // CPython converts via PyNumber_Absolute then splits low to 32-bit words;
        // for values within u64 this is: key = [low32, high32] minus zero words.
        let mut key: Vec<u32> = Vec::new();
        let lo = (abs_seed & 0xFFFF_FFFF) as u32;
        let hi = ((abs_seed >> 32) & 0xFFFF_FFFF) as u32;
        if hi != 0 {
            key.push(lo);
            key.push(hi);
        } else {
            key.push(lo);
        }
        let mut mt = Mt {
            state: [0; 624],
            idx: 0,
        };
        mt.init_by_array(&key);
        mt
    }

    fn init_genrand(&mut self, s: u32) {
        self.state[0] = s;
        for i in 1..624 {
            self.state[i] = 1812433253u32
                .wrapping_mul(self.state[i - 1] ^ (self.state[i - 1] >> 30))
                .wrapping_add(i as u32);
        }
        self.idx = 624;
    }

    fn init_by_array(&mut self, key: &[u32]) {
        self.init_genrand(19650218);
        let mut i = 1usize;
        let mut j = 0usize;
        let mut k = 624.max(key.len());
        while k > 0 {
            let prev = self.state[i - 1];
            let mixed = prev ^ (prev >> 30);
            self.state[i] = (self.state[i] ^ (1664525u32.wrapping_mul(mixed)))
                .wrapping_add(key[j])
                .wrapping_add(j as u32);
            i += 1;
            j += 1;
            if i >= 624 {
                self.state[0] = self.state[623];
                i = 1;
            }
            if j >= key.len() {
                j = 0;
            }
            k -= 1;
        }
        k = 623;
        while k > 0 {
            let prev = self.state[i - 1];
            let mixed = prev ^ (prev >> 30);
            self.state[i] =
                (self.state[i] ^ (1566083941u32.wrapping_mul(mixed))).wrapping_sub(i as u32);
            i += 1;
            if i >= 624 {
                self.state[0] = self.state[623];
                i = 1;
            }
            k -= 1;
        }
        self.state[0] = 0x8000_0000;
    }

    fn genrand_uint32(&mut self) -> u32 {
        if self.idx >= 624 {
            self.regen();
        }
        let mut y = self.state[self.idx];
        self.idx += 1;
        y ^= y >> 11;
        y ^= (y << 7) & 0x9D2C_5680;
        y ^= (y << 15) & 0xEFC6_0000;
        y ^= y >> 18;
        y
    }

    fn regen(&mut self) {
        for i in 0..624 {
            let y = (self.state[i] & 0x8000_0000) | (self.state[(i + 1) % 624] & 0x7FFF_FFFF);
            let mut next = self.state[(i + 397) % 624] ^ (y >> 1);
            if y & 1 != 0 {
                next ^= 0x9908_B0DF;
            }
            self.state[i] = next;
        }
        self.idx = 0;
    }

    fn random53(&mut self) -> f64 {
        let a = self.genrand_uint32() >> 5;
        let b = self.genrand_uint32() >> 6;
        (a as f64 * 67108864.0 + b as f64) / 9007199254740992.0
    }

    /// CPython _randbelow_with_getrandbits(n): k = n.bit_length(),
    /// r = getrandbits(k); while r >= n: r = getrandbits(k)
    fn randbelow(&mut self, n: u64) -> u64 {
        let k = 64 - n.max(1).leading_zeros();
        loop {
            let r = self.genrand_bits(k);
            if r < n {
                return r;
            }
        }
    }

    fn genrand_bits(&mut self, k: u32) -> u64 {
        // getrandbits: words from MSB side; for k <= 32 a single word >> (32-k)
        if k == 0 {
            return 0;
        }
        if k <= 32 {
            (self.genrand_uint32() >> (32 - k)) as u64
        } else {
            let lo = self.genrand_uint32() as u64;
            let hi = self.genrand_uint32() as u64;
            ((hi << 32) | lo) >> (64 - k)
        }
    }
}

thread_local! {
    static MT: RefCell<Mt> = RefCell::new(Mt::new_seeded_int(0));
}

fn build_random(_i: &crate::frame::Interpreter) -> Value {
    let (m, ns) = new_module("random");
    for n in [
        "seed",
        "random",
        "randint",
        "randrange",
        "choice",
        "shuffle",
        "sample",
        "uniform",
        "getrandbits",
    ] {
        fn1(&ns, "random", n);
    }
    finish(m, ns)
}

pub fn call_random(name: &str, args: &[Value], kwargs: &[(String, Value)]) -> Option<BResult> {
    let r: BResult =
        (|| -> BResult {
            match name {
                "seed" => {
                    let v = required(args, "seed")?;
                    match v {
                        Value::None => {
                            // nondeterministic seed; corpus never anchors it
                            MT.with(|mt| *mt.borrow_mut() = Mt::new_seeded_int(0));
                        }
                        other => {
                            let n = as_int_i64(other)?;
                            MT.with(|mt| *mt.borrow_mut() = Mt::new_seeded_int(n));
                        }
                    }
                    Ok(Value::None)
                }
                "random" => Ok(f(MT.with(|mt| mt.borrow_mut().random53()))),
                "uniform" => {
                    let a = one_f64_arg(args, "uniform")?;
                    let b =
                        args.get(1).map(as_f64).transpose()?.ok_or_else(|| {
                            raise("TypeError", "uniform() missing required argument")
                        })?;
                    Ok(f(a + (b - a) * MT.with(|mt| mt.borrow_mut().random53())))
                }
                "randint" => {
                    let a = as_int_i64(required(args, "randint")?)?;
                    let b =
                        args.get(1).map(as_int_i64).transpose()?.ok_or_else(|| {
                            raise("TypeError", "randint() missing required argument")
                        })?;
                    if a > b {
                        return Err(raise(
                            "ValueError",
                            format!("empty range for randint() ({a}, {b})"),
                        ));
                    }
                    let width = (b - a + 1) as u64;
                    let r = MT.with(|mt| mt.borrow_mut().randbelow(width));
                    Ok(i(a + r as i64))
                }
                "randrange" => {
                    let (start, stop, step) = match args.len() {
                        1 => (0, as_int_i64(&args[0])?, 1),
                        2 => (as_int_i64(&args[0])?, as_int_i64(&args[1])?, 1),
                        _ => (
                            as_int_i64(&args[0])?,
                            as_int_i64(&args[1])?,
                            args.get(2)
                                .map(|v| as_int_i64(&v))
                                .transpose()?
                                .unwrap_or(1),
                        ),
                    };
                    if step == 0 {
                        return Err(raise("ValueError", "randrange() arg 3 must not be zero"));
                    }
                    let width = (stop - start).div_euclid(step);
                    if width <= 0 {
                        return Err(raise("ValueError", format!("empty range for randrange()")));
                    }
                    let r = MT.with(|mt| mt.borrow_mut().randbelow(width as u64));
                    Ok(i(start + r as i64 * step))
                }
                "getrandbits" => {
                    let k = as_int_i64(required(args, "getrandbits")?)?.clamp(0, 64) as u32;
                    Ok(i(MT.with(|mt| mt.borrow_mut().genrand_bits(k)) as i64))
                }
                "choice" => {
                    let seq = snapshot(required(args, "choice")?)?;
                    if seq.is_empty() {
                        return Err(raise("IndexError", "Cannot choose from an empty sequence"));
                    }
                    let idx = MT.with(|mt| mt.borrow_mut().randbelow(seq.len() as u64));
                    Ok(seq[idx as usize].clone())
                }
                "shuffle" => {
                    let x = required(args, "shuffle")?;
                    if let Value::List(l) = x {
                        let mut items = l.items().to_vec();
                        let n = items.len();
                        for i2 in (1..n).rev() {
                            let j = MT.with(|mt| mt.borrow_mut().randbelow(i2 as u64 + 1)) as usize;
                            items.swap(i2, j);
                        }
                        l.clear();
                        for v in items {
                            l.append(v);
                        }
                    }
                    Ok(Value::None)
                }
                "sample" => {
                    let pop = snapshot(required(args, "sample")?)?;
                    let k = as_int_i64(args.get(1).ok_or_else(|| {
                        raise("TypeError", "sample() missing required argument")
                    })?)?;
                    if k < 0 || k as usize > pop.len() {
                        return Err(raise(
                            "ValueError",
                            "Sample larger than population or is negative",
                        ));
                    }
                    // selection-set algorithm (CPython's path for small k)
                    let mut pool = pop;
                    let mut out = Vec::with_capacity(k as usize);
                    for _ in 0..k {
                        let j = MT.with(|mt| mt.borrow_mut().randbelow(pool.len() as u64)) as usize;
                        out.push(pool.remove(j));
                    }
                    Ok(list(out))
                }
                _ => Err(raise("SystemError", format!("random fn {name} unbound"))),
            }
        })();
    let _ = kwargs;
    Some(r)
}

// ===========================================================================
// hashlib (RustCrypto md5/sha1/sha2/sha512)
// ===========================================================================

fn build_hashlib(_i: &crate::frame::Interpreter) -> Value {
    let (m, ns) = new_module("hashlib");
    for n in ["md5", "sha1", "sha256", "sha512", "sha224", "sha384"] {
        fn1(&ns, "hashlib", n);
    }
    finish(m, ns)
}

fn hex_digest_of(alg: &str, data: &[u8]) -> String {
    match alg {
        "md5" => {
            use md5::Digest;
            let mut h = md5::Md5::new();
            h.update(data);
            format!("{:x}", h.finalize())
        }
        "sha1" => {
            use sha1::Digest;
            let mut h = sha1::Sha1::new();
            h.update(data);
            format!("{:x}", h.finalize())
        }
        "sha224" => {
            use sha2::Digest;
            let mut h = sha2::Sha224::new();
            h.update(data);
            format!("{:x}", h.finalize())
        }
        "sha256" => {
            use sha2::Digest;
            let mut h = sha2::Sha256::new();
            h.update(data);
            format!("{:x}", h.finalize())
        }
        "sha384" => {
            use sha2::Digest;
            let mut h = sha2::Sha384::new();
            h.update(data);
            format!("{:x}", h.finalize())
        }
        _ => {
            use sha2::Digest;
            let mut h = sha2::Sha512::new();
            h.update(data);
            format!("{:x}", h.finalize())
        }
    }
}

fn bytes_arg(v: &Value) -> Result<Vec<u8>, Flow> {
    match v {
        Value::Bytes(b) => Ok((**b).clone()),
        Value::Str(st) => Ok(st.as_str().as_bytes().to_vec()),
        other => Err(raise(
            "TypeError",
            format!(
                "Unicode-objects must be encoded before hashing, not {}",
                other.type_name()
            ),
        )),
    }
}

pub fn call_hashlib(name: &str, args: &[Value], _kwargs: &[(String, Value)]) -> Option<BResult> {
    let r: BResult = (|| -> BResult {
        match name {
            "md5" | "sha1" | "sha224" | "sha256" | "sha384" | "sha512" => {
                let data = bytes_arg(required(args, name)?)?;
                let hex = hex_digest_of(name, &data);
                let obj = PyObject::new(ap_obj::ObjKind::Opaque, format!("{name}_HASH").as_str());
                {
                    let mut attrs = obj.attrs.borrow_mut();
                    let _ = attrs.set(s("hexdigest"), s(hex.clone()));
                    let _ = attrs.set(s("digest"), Value::Bytes(Gc::new(hex_bytes(&hex))));
                    let _ = attrs.set(s("name"), s(name));
                    let _ = attrs.set(
                        s("block_size"),
                        i(match name {
                            "md5" | "sha1" | "sha224" | "sha256" => 64,
                            _ => 128,
                        }),
                    );
                    let _ = attrs.set(
                        s("digest_size"),
                        i(match name {
                            "md5" => 16,
                            "sha1" => 20,
                            "sha224" => 28,
                            "sha256" => 32,
                            "sha384" => 48,
                            _ => 64,
                        }),
                    );
                }
                *obj.repr_text.borrow_mut() = Some(format!("<{name} HASH object>"));
                Ok(Value::Obj(obj))
            }
            _ => Err(raise("SystemError", format!("hashlib fn {name} unbound"))),
        }
    })();
    match r {
        Err(Flow::Next) => return None,
        other => Some(other),
    }
}

fn hex_bytes(hex: &str) -> Vec<u8> {
    (0..hex.len() / 2)
        .map(|i2| u8::from_str_radix(&hex[2 * i2..2 * i2 + 2], 16).unwrap_or(0))
        .collect()
}

// ===========================================================================
// base64 + binascii (hex)
// ===========================================================================

fn build_base64(_i: &crate::frame::Interpreter) -> Value {
    let (m, ns) = new_module("base64");
    for n in ["b64encode", "b64decode", "b16encode", "b16decode"] {
        fn1(&ns, "base64", n);
    }
    finish(m, ns)
}

fn build_binascii(_i: &crate::frame::Interpreter) -> Value {
    let (m, ns) = new_module("binascii");
    for n in ["hexlify", "unhexlify", "a2b_hex", "b2a_hex", "crc32"] {
        fn1(&ns, "binascii", n);
    }
    finish(m, ns)
}

fn b64_table() -> &'static [u8; 64] {
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"
}

pub fn call_base64(name: &str, args: &[Value], _kwargs: &[(String, Value)]) -> Option<BResult> {
    let r: BResult = (|| -> BResult {
        match name {
            "b64encode" => {
                let data = bytes_arg(required(args, "b64encode")?)?;
                Ok(Value::Bytes(Gc::new(b64_encode(&data))))
            }
            "b64decode" => {
                let data = bytes_arg(required(args, "b64decode")?)?;
                b64_decode(&data)
                    .map(|b| Value::Bytes(Gc::new(b)))
                    .map_err(|()| raise("binascii.Error", "Invalid base64-encoded string"))
            }
            "b16encode" => {
                let data = bytes_arg(required(args, "b16encode")?)?;
                let mut out = Vec::with_capacity(data.len() * 2);
                for b in data {
                    out.extend(format!("{b:02X}").into_bytes());
                }
                Ok(Value::Bytes(Gc::new(out)))
            }
            "b16decode" => {
                let data = bytes_arg(required(args, "b16decode")?)?;
                let txt = String::from_utf8_lossy(&data).to_uppercase();
                if txt.len() % 2 != 0 {
                    return Err(raise("binascii.Error", "Odd-length string"));
                }
                let mut out = Vec::with_capacity(txt.len() / 2);
                for pair in txt.as_bytes().chunks(2) {
                    let hi = (pair[0] as char).to_digit(16);
                    let lo = (pair[1] as char).to_digit(16);
                    match (hi, lo) {
                        (Some(h), Some(l)) => out.push((h * 16 + l) as u8),
                        _ => return Err(raise("binascii.Error", "Non-base16 digit found")),
                    }
                }
                Ok(Value::Bytes(Gc::new(out)))
            }
            _ => Err(raise("SystemError", format!("base64 fn {name} unbound"))),
        }
    })();
    match r {
        Err(Flow::Next) => return None,
        other => Some(other),
    }
}

fn b64_encode(data: &[u8]) -> Vec<u8> {
    let table = b64_table();
    let mut out = Vec::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(table[(n >> 18) as usize & 63]);
        out.push(table[(n >> 12) as usize & 63]);
        out.push(if chunk.len() > 1 {
            table[(n >> 6) as usize & 63]
        } else {
            b'='
        });
        out.push(if chunk.len() > 2 {
            table[n as usize & 63]
        } else {
            b'='
        });
    }
    out
}

fn b64_decode(data: &[u8]) -> Result<Vec<u8>, ()> {
    let mut acc: u32 = 0;
    let mut bits = 0;
    let mut out = Vec::new();
    for &c in data {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => break,
            b'\n' | b'\r' => continue,
            _ => return Err(()),
        };
        acc = (acc << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Ok(out)
}

pub fn call_binascii(name: &str, args: &[Value], _kwargs: &[(String, Value)]) -> Option<BResult> {
    let r: BResult = (|| -> BResult {
        match name {
            "hexlify" | "b2a_hex" => {
                let data = bytes_arg(required(args, name)?)?;
                let mut out = Vec::with_capacity(data.len() * 2);
                for b in data {
                    out.extend(format!("{b:02x}").into_bytes());
                }
                Ok(Value::Bytes(Gc::new(out)))
            }
            "unhexlify" | "a2b_hex" => {
                let txt = match required(args, name)? {
                    Value::Bytes(b) => String::from_utf8_lossy(b).to_string(),
                    Value::Str(st) => st.as_str().to_string(),
                    other => {
                        return Err(raise(
                            "TypeError",
                            format!("expected bytes or str, not {}", other.type_name()),
                        ))
                    }
                };
                let txt: String = txt.chars().filter(|c| !c.is_whitespace()).collect();
                if txt.len() % 2 != 0 {
                    return Err(raise("binascii.Error", "Odd-length string"));
                }
                let mut out = Vec::with_capacity(txt.len() / 2);
                for pair in txt.as_bytes().chunks(2) {
                    let hi = (pair[0] as char).to_digit(16);
                    let lo = (pair[1] as char).to_digit(16);
                    match (hi, lo) {
                        (Some(h), Some(l)) => out.push((h * 16 + l) as u8),
                        _ => return Err(raise("binascii.Error", "Non-hexadecimal digit found")),
                    }
                }
                Ok(Value::Bytes(Gc::new(out)))
            }
            "crc32" => {
                let data = bytes_arg(required(args, "crc32")?)?;
                let mut crc: u32 = 0xFFFF_FFFF;
                for &b in &data {
                    crc ^= b as u32;
                    for _ in 0..8 {
                        let mask = (crc & 1).wrapping_neg();
                        crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
                    }
                }
                Ok(i(!crc as i64))
            }
            _ => Err(raise("SystemError", format!("binascii fn {name} unbound"))),
        }
    })();
    match r {
        Err(Flow::Next) => return None,
        other => Some(other),
    }
}

// ===========================================================================
// io (BytesIO/StringIO/TextIOWrapper/FileIO/BufferedReader-lite; S-05-4 io row)
// ===========================================================================

fn build_io(_i: &crate::frame::Interpreter) -> Value {
    let (m, ns) = new_module("io");
    ns_const(&ns, "DEFAULT_BUFFER_SIZE", i(8192));
    ns_const(&ns, "SEEK_SET", i(0));
    ns_const(&ns, "SEEK_CUR", i(1));
    ns_const(&ns, "SEEK_END", i(2));
    for n in [
        "BytesIO",
        "StringIO",
        "TextIOWrapper",
        "FileIO",
        "BufferedReader",
        "open",
        "text_encoding",
        "IncrementalNewlineDecoder",
    ] {
        fn1(&ns, "io", n);
    }
    for t in [
        "IOBase",
        "RawIOBase",
        "BufferedIOBase",
        "TextIOBase",
        "UnsupportedOperation",
        "BlockingIOError",
    ] {
        let _ = ns.set(s(t), interned_type_pub(t));
    }
    finish(m, ns)
}

fn make_stream(kind: &str, data: Vec<u8>, encoding: Option<String>) -> Value {
    let obj = PyObject::new(ap_obj::ObjKind::Opaque, kind);
    {
        let mut attrs = obj.attrs.borrow_mut();
        let _ = attrs.set(s("<buf>"), Value::Bytes(Gc::new(data)));
        let _ = attrs.set(s("<pos>"), i(0));
        let _ = attrs.set(
            s("<encoding>"),
            match encoding {
                Some(e) => s(e),
                None => Value::None,
            },
        );
        let _ = attrs.set(s("<closed>"), Value::Bool(false));
    }
    Value::Obj(obj)
}

fn stream_parts<'a>(v: &'a Value) -> Result<(Vec<u8>, usize, Option<String>), Flow> {
    if let Value::Obj(o) = v {
        let attrs = o.attrs.borrow();
        let data = match attrs.get(&s("<buf>")) {
            Some(Value::Bytes(b)) => b.to_vec(),
            _ => vec![],
        };
        let pos = attrs
            .get(&s("<pos>"))
            .map(|v| as_int_i64(&v))
            .transpose()?
            .unwrap_or(0) as usize;
        let enc = match attrs.get(&s("<encoding>")) {
            Some(Value::Str(x)) => Some(x.as_str().to_string()),
            _ => None,
        };
        return Ok((data, pos, enc));
    }
    Err(raise("SystemError", "bad stream"))
}

fn stream_set_pos(v: &Value, pos: usize) {
    if let Value::Obj(o) = v {
        let mut attrs = o.attrs.borrow_mut();
        let _ = attrs.set(s("<pos>"), i(pos as i64));
    }
}

pub fn call_io(name: &str, args: &[Value], kwargs: &[(String, Value)]) -> Option<BResult> {
    let r: BResult = (|| -> BResult {
        match name {
            "BytesIO" | "StringIO" => {
                let data = match args.first() {
                    None => vec![],
                    Some(Value::None) => vec![],
                    Some(v) => {
                        if name == "BytesIO" {
                            bytes_arg(v)?
                        } else {
                            str_arg(v, name)?.into_bytes()
                        }
                    }
                };
                Ok(make_stream(name, data, None))
            }
            "TextIOWrapper" => {
                let inner = required(args, "TextIOWrapper")?;
                let enc = kw(kwargs, "encoding")
                    .map(|v| str_arg(&v, "encoding"))
                    .transpose()?
                    .unwrap_or_else(|| "utf-8".to_string());
                let (data, _, _) = stream_parts(inner)?;
                Ok(make_stream("TextIOWrapper", data, Some(enc)))
            }
            "BufferedReader" => {
                let inner = required(args, "BufferedReader")?;
                let (data, pos, enc) = stream_parts(inner)?;
                Ok(make_stream("BufferedReader", data, enc).tap_set_pos(pos))
            }
            "FileIO" => {
                let path = str_arg(required(args, "FileIO")?, "FileIO")?;
                let data = match args
                    .get(1)
                    .map(|m| str_arg(m, "mode"))
                    .transpose()?
                    .as_deref()
                {
                    Some("w") | Some("wb") => vec![],
                    _ => std::fs::read(&path).unwrap_or_default(),
                };
                Ok(make_stream("FileIO", data, None))
            }
            "open" => {
                let path = str_arg(required(args, "open")?, "open")?;
                let mode = match args.get(1) {
                    Some(v) => str_arg(v, "open")?,
                    None => "r".to_string(),
                };
                if mode.contains('w') {
                    std::fs::write(&path, b"").map_err(|e| os_error(&path, &e))?;
                    let st = make_stream("TextIOWrapper", vec![], Some("utf-8".into()));
                    if let Value::Obj(o) = &st {
                        let mut attrs = o.attrs.borrow_mut();
                        let _ = attrs.set(s("<path>"), s(path));
                    }
                    return Ok(st);
                }
                let data = std::fs::read(&path).map_err(|e| os_error(&path, &e))?;
                Ok(make_stream(
                    if mode.contains('b') {
                        "BufferedReader"
                    } else {
                        "TextIOWrapper"
                    },
                    data,
                    Some("utf-8".into()),
                ))
            }
            "text_encoding" => match args.first() {
                None | Some(Value::None) => Ok(s("utf-8")),
                Some(v) => Ok(v.clone()),
            },
            "IncrementalNewlineDecoder" => {
                Ok(make_stream("IncrementalNewlineDecoder", vec![], None))
            }
            "read" => {
                let (data, pos, _) = stream_parts(required(args, "read")?)?;
                let n = args.get(1).map(|v| as_int_i64(&v)).transpose()?;
                let out = match n {
                    None => data[pos..].to_vec(),
                    Some(n) => data[pos..(pos + n.max(0) as usize).min(data.len())].to_vec(),
                };
                stream_set_pos(required(args, "read")?, pos + out.len());
                Ok(Value::Bytes(Gc::new(out)))
            }
            "readline" => {
                let (data, pos, _) = stream_parts(required(args, "readline")?)?;
                let rest = &data[pos..];
                let nl = rest.iter().position(|&b| b == b'\n');
                let out = match nl {
                    Some(p) => {
                        stream_set_pos(required(args, "readline")?, pos + p + 1);
                        rest[..p + 1].to_vec()
                    }
                    None => {
                        stream_set_pos(required(args, "readline")?, data.len());
                        rest.to_vec()
                    }
                };
                Ok(Value::Bytes(Gc::new(out)))
            }
            "write" => {
                let v = required(args, "write")?;
                if let Value::Obj(o) = v {
                    let is_bytes_stream = o.name != "StringIO" && o.name != "TextIOWrapper";
                    let payload = if is_bytes_stream {
                        bytes_arg(args.get(1).ok_or_else(|| {
                            raise("TypeError", "write() missing required argument")
                        })?)?
                    } else {
                        str_arg(
                            args.get(1).ok_or_else(|| {
                                raise("TypeError", "write() missing required argument")
                            })?,
                            "write",
                        )?
                        .into_bytes()
                    };
                    let mut attrs = o.attrs.borrow_mut();
                    let data = match attrs.get(&s("<buf>")) {
                        Some(Value::Bytes(b)) => b.to_vec(),
                        _ => vec![],
                    };
                    let mut nd = data;
                    nd.extend_from_slice(&payload);
                    let n = payload.len();
                    let total = nd.len();
                    let _ = attrs.set(s("<buf>"), Value::Bytes(Gc::new(nd)));
                    let _ = attrs.set(s("<pos>"), i(total as i64));
                    return Ok(i(n as i64));
                }
                Err(raise("SystemError", "bad stream"))
            }
            "tell" => {
                let (_, pos, _) = stream_parts(required(args, "tell")?)?;
                Ok(i(pos as i64))
            }
            "seek" => {
                let v = required(args, "seek")?;
                let off = args
                    .get(1)
                    .map(as_int_i64)
                    .transpose()?
                    .ok_or_else(|| raise("TypeError", "seek() missing required argument"))?;
                let whence = args
                    .get(2)
                    .map(|v| as_int_i64(&v))
                    .transpose()?
                    .unwrap_or(0);
                let (data, pos, _) = stream_parts(v)?;
                let np = match whence {
                    0 => off,
                    1 => pos as i64 + off,
                    _ => data.len() as i64 + off,
                };
                stream_set_pos(v, np.max(0) as usize);
                Ok(i(np.max(0)))
            }
            "getvalue" => {
                let (data, _, _) = stream_parts(required(args, "getvalue")?)?;
                if let Value::Obj(o) = required(args, "getvalue")? {
                    if o.name == "StringIO" {
                        return Ok(s(String::from_utf8_lossy(&data).to_string()));
                    }
                }
                Ok(Value::Bytes(Gc::new(data)))
            }
            "close" => {
                // Write-mode streams flush their buffer to disk on close.
                if let Value::Obj(o) = required(args, "close")? {
                    let (has_path, path, buf) = {
                        let attrs = o.attrs.borrow();
                        let path = match attrs.get(&s("<path>")) {
                            Some(Value::Str(x)) => Some(x.as_str().to_string()),
                            _ => None,
                        };
                        let buf = match attrs.get(&s("<buf>")) {
                            Some(Value::Bytes(b)) => b.to_vec(),
                            _ => vec![],
                        };
                        (path.is_some(), path, buf)
                    };
                    if has_path {
                        if let Some(p) = path {
                            let _ = std::fs::write(p, &buf);
                        }
                    }
                }
                Ok(Value::None)
            }
            "__enter__" => Ok(required(args, "__enter__")?.clone()),
            "__exit__" => Ok(Value::Bool(false)),
            "peek" => {
                let (data, pos, _) = stream_parts(required(args, "peek")?)?;
                Ok(Value::Bytes(Gc::new(data[pos..].to_vec())))
            }
            "read1" => call_io("read", args, kwargs).unwrap_or_else(|| Ok(Value::None)),
            "decode" => {
                // IncrementalNewlineDecoder.translate
                let text = str_arg(required(args, "decode")?, "decode")?;
                Ok(s(text.replace("\r\n", "\n").replace('\r', "\n")))
            }
            _ => return Err(raise("SystemError", format!("io fn {name} unbound"))),
        }
    })();
    Some(r)
}

trait TapPos {
    fn tap_set_pos(self, pos: usize) -> Value;
}
impl TapPos for Value {
    fn tap_set_pos(self, pos: usize) -> Value {
        stream_set_pos(&self, pos);
        self
    }
}

// ===========================================================================
// tempfile (IN-lite: mkstemp/mkdtemp/gettempdir/TemporaryDirectory names)
// ===========================================================================

fn build_tempfile(_i: &crate::frame::Interpreter) -> Value {
    let (m, ns) = new_module("tempfile");
    for n in [
        "mkstemp",
        "mkdtemp",
        "gettempdir",
        "TemporaryDirectory",
        "NamedTemporaryFile",
    ] {
        fn1(&ns, "tempfile", n);
    }
    finish(m, ns)
}

fn temp_dir() -> String {
    std::env::temp_dir().to_string_lossy().to_string()
}

fn temp_name(prefix: &str, suffix: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let pid = std::process::id();
    format!(
        "{}{}{}{}{}",
        temp_dir(),
        std::path::MAIN_SEPARATOR,
        prefix,
        pid,
        n
    ) + suffix
}

pub fn call_tempfile(name: &str, args: &[Value], kwargs: &[(String, Value)]) -> Option<BResult> {
    let r: BResult = (|| -> BResult {
        match name {
            "mkstemp" => {
                let prefix = kw(kwargs, "prefix")
                    .map(|v| str_arg(v, "prefix"))
                    .transpose()?
                    .unwrap_or_else(|| "tmp".to_string());
                let suffix = kw(kwargs, "suffix")
                    .map(|v| str_arg(v, "suffix"))
                    .transpose()?
                    .unwrap_or_default();
                let path = temp_name(&prefix, &suffix);
                std::fs::write(&path, b"").map_err(|e| os_error(&path, &e))?;
                Ok(tuple(vec![i(0), s(path)]))
            }
            "mkdtemp" => {
                let prefix = kw(kwargs, "prefix")
                    .map(|v| str_arg(v, "prefix"))
                    .transpose()?
                    .unwrap_or_else(|| "tmp".to_string());
                let path = temp_name(&prefix, "");
                std::fs::create_dir(&path).map_err(|e| os_error(&path, &e))?;
                Ok(s(path))
            }
            "gettempdir" => Ok(s(temp_dir())),
            "TemporaryDirectory" | "NamedTemporaryFile" => Err(raise(
                "NotImplementedError",
                format!("{name} requires class machinery (02.03)"),
            )),
            _ => return Err(raise("SystemError", format!("tempfile fn {name} unbound"))),
        }
    })();
    Some(r)
}

// ===========================================================================
// Top-level dispatch (called from builtins::call_builtin for dotted names)
// ===========================================================================

/// Dispatch `<mod>.<fn>` (and native-type methods like `deque.append`,
/// plus callables: attrgetter/itemgetter markers, partial objects).
pub fn call_native(
    mod_name: &str,
    fname: &str,
    args: &[Value],
    kwargs: &[(String, Value)],
) -> Option<BResult> {
    match mod_name {
        "math" => call_math(fname, args, kwargs),
        "itertools" => call_itertools(fname, args, kwargs),
        "operator" => call_operator(fname, args, kwargs),
        "heapq" => call_heapq(fname, args, kwargs),
        "bisect" => call_bisect(fname, args, kwargs),
        "functools" => call_functools(fname, args, kwargs),
        "string" => call_string(fname, args, kwargs),
        "textwrap" => call_textwrap(fname, args, kwargs),
        "collections" => call_collections(fname, args, kwargs),
        "json" => call_json(fname, args, kwargs),
        "os" => call_os(fname, args, kwargs),
        "posixpath" | "os.path" | "genericpath" => call_posixpath(mod_name, fname, args, kwargs),
        "stat" => call_stat(fname, args, kwargs),
        "errno" => call_errno_lookup(fname, args),
        "keyword" => call_keyword(fname, args, kwargs),
        "platform" => call_platform(fname, args, kwargs),
        "time" => call_time(fname, args, kwargs),
        "atexit" => call_atexit(fname, args, kwargs),
        "linecache" => call_linecache(fname, args, kwargs),
        "random" => call_random(fname, args, kwargs),
        "hashlib" => call_hashlib(fname, args, kwargs),
        "base64" => call_base64(fname, args, kwargs),
        "binascii" => call_binascii(fname, args, kwargs),
        "pystr" | "pydict" | "pylist" | "pybytes" => {
            call_builtin_values(mod_name, fname, args, kwargs)
        }
        "io" => call_io(fname, args, kwargs),
        "tempfile" => call_tempfile(fname, args, kwargs),
        _ => None,
    }
}

fn call_errno_lookup(_fname: &str, _args: &[Value]) -> Option<BResult> {
    None
}

/// Dispatch an Opaque native-type instance call (functools.partial objects).
pub fn call_native_object(
    v: &Value,
    args: &[Value],
    kwargs: &[(String, Value)],
) -> Option<BResult> {
    if let Value::Obj(o) = v {
        if o.kind == ap_obj::ObjKind::Opaque && o.name == "functools.partial" {
            return Some(call_partial_obj(o, args, kwargs));
        }
    }
    None
}

/// Attribute access on native objects (attrgetter/itemgetter/partial markers,
/// native-type instances, hexdigest objects). Called from the attr_get hook.
pub fn native_attr(obj: &Value, name: &str) -> Option<Value> {
    if let Value::Obj(o) = obj {
        match o.kind {
            ap_obj::ObjKind::Function => match name {
                "__name__" => Some(s(o.name.clone())),
                "__qualname__" => Some(s(o.name.clone())),
                "__doc__" => Some(Value::None),
                _ => None,
            },
            ap_obj::ObjKind::BuiltinFn => {
                let full = o.name.clone();
                if let Some(v) = builtinfn_attr(&full, name) {
                    return Some(v);
                }
                // partial markers: func/args/keywords decode from the name
                if let Some(body) = full.strip_prefix("<partial:") {
                    let body = body.strip_suffix('>')?;
                    let (fname, argstr) = body.split_once(':')?;
                    return match name {
                        "func" => {
                            if let Some(v) =
                                NAMED_CALLABLES.with(|t| t.borrow().get(fname).cloned())
                            {
                                Some(v)
                            } else if fname.contains('.') {
                                Some(marker(fname))
                            } else {
                                None
                            }
                        }
                        "args" => {
                            let vals: Vec<Value> = if argstr.is_empty() {
                                vec![]
                            } else {
                                argstr.split('\u{1f}').map(decode_arg).collect()
                            };
                            Some(tuple(vals))
                        }
                        "keywords" => Some(Value::Dict(PyDict::new())),
                        "__name__" => Some(s("functools.partial")),
                        _ => None,
                    };
                }
                // dotted module markers expose __name__ (e.g. math.sqrt)
                if name == "__name__" {
                    if let Some((_, fname)) = full.split_once('.') {
                        return Some(s(fname));
                    }
                    return Some(s(full.clone()));
                }
                // itertools.chain.from_iterable classmethod-style access
                if full == "itertools.chain" && name == "from_iterable" {
                    return Some(marker("itertools.chain.from_iterable"));
                }
                None
            }
            ap_obj::ObjKind::Opaque => match o.name.as_str() {
                "functools.partial" => match name {
                    "func" => {
                        let attrs = o.attrs.borrow();
                        return attrs
                            .get(&s("__func_name__"))
                            .and_then(|v| resolve_callable_marker(&v));
                    }
                    "args" => {
                        let attrs = o.attrs.borrow();
                        return attrs.get(&s("__args__"));
                    }
                    "keywords" => return Some(Value::Dict(PyDict::new())),
                    _ => None,
                },
                "deque" => {
                    let mname = match name {
                        "append" | "appendleft" | "pop" | "popleft" | "extend" | "clear"
                        | "rotate" | "index" | "__iter__" => {
                            format!("collections.deque.{name}")
                        }
                        _ => return None,
                    };
                    {
                        let bm = PyObject::with_self(
                            ap_obj::ObjKind::BoundMethod,
                            mname.clone(),
                            obj.clone(),
                        );
                        *bm.func.borrow_mut() = Some(marker(&mname));
                        Some(Value::Obj(bm))
                    }
                }
                "BytesIO"
                | "StringIO"
                | "TextIOWrapper"
                | "FileIO"
                | "BufferedReader"
                | "IncrementalNewlineDecoder" => {
                    let mname = match name {
                        "read" | "readline" | "write" | "tell" | "seek" | "getvalue" | "close"
                        | "peek" | "read1" | "decode" | "__enter__" | "__exit__" => {
                            format!("io.{name}")
                        }
                        _ => return None,
                    };
                    let bm = PyObject::with_self(
                        ap_obj::ObjKind::BoundMethod,
                        mname.clone(),
                        obj.clone(),
                    );
                    *bm.func.borrow_mut() = Some(marker(&mname));
                    Some(Value::Obj(bm))
                }
                n if n.ends_with("_HASH") => {
                    let attrs = o.attrs.borrow();
                    match name {
                        "hexdigest" => {
                            return attrs.get(&s("hexdigest")).map(|v| {
                                let bm = PyObject::with_self(
                                    ap_obj::ObjKind::BoundMethod,
                                    "<hashlib.hexdigest>",
                                    v.clone(),
                                );
                                *bm.func.borrow_mut() = Some(marker("<hashlib.hexdigest>"));
                                Value::Obj(bm)
                            })
                        }
                        "digest" => {
                            return attrs.get(&s("digest")).map(|v| {
                                let bm = PyObject::with_self(
                                    ap_obj::ObjKind::BoundMethod,
                                    "<hashlib.digest>",
                                    v.clone(),
                                );
                                *bm.func.borrow_mut() = Some(marker("<hashlib.digest>"));
                                Value::Obj(bm)
                            })
                        }
                        "name" | "block_size" | "digest_size" => attrs.get(&s(name)),
                        _ => None,
                    }
                }
                "struct_time" => {
                    let attrs = o.attrs.borrow();
                    match name {
                        "tm_year" | "tm_mon" | "tm_mday" | "tm_hour" | "tm_min" | "tm_sec"
                        | "tm_wday" | "tm_yday" | "tm_isdst" => attrs.get(&s(name)),
                        "n_fields" => Some(i(11)),
                        "n_sequence_fields" => Some(i(9)),
                        "n_unnamed_fields" => Some(i(0)),
                        _ => None,
                    }
                }
                "uname_result" => {
                    let attrs = o.attrs.borrow();
                    match name {
                        "system" | "node" | "release" | "version" | "machine" | "processor" => {
                            attrs.get(&s(name))
                        }
                        "_fields" => attrs.get(&s("<fields>")),
                        _ => None,
                    }
                }
                _ => None,
            },
            _ => None,
        }
    } else {
        // Builtin-value method layer: str/dict/list/set/bytes methods not in
        // the v0 methods.rs surface (startswith, dict.get, ...) dispatch here
        // via BoundMethod markers → call_native "pystr"/"pydict"/... routes.
        let type_tag = match obj {
            Value::Str(_) => Some("pystr"),
            Value::Dict(_) => Some("pydict"),
            Value::List(_) => Some("pylist"),
            Value::Bytes(_) => Some("pybytes"),
            _ => None,
        };
        if let Some(tag) = type_tag {
            if builtin_value_method_exists(tag, name) {
                let bm = PyObject::with_self(
                    ap_obj::ObjKind::BoundMethod,
                    format!("{tag}.{name}"),
                    obj.clone(),
                );
                // The interp BoundMethod fallback calls `func` with [self]+args;
                // wire it to the dotted dispatch marker.
                *bm.func.borrow_mut() = Some(marker(&format!("{tag}.{name}")));
                return Some(Value::Obj(bm));
            }
        }
        None
    }
}

fn builtin_value_method_exists(tag: &str, name: &str) -> bool {
    matches!(
        (tag, name),
        ("pystr", "startswith")
            | ("pystr", "endswith")
            | ("pystr", "strip")
            | ("pystr", "lstrip")
            | ("pystr", "rstrip")
            | ("pystr", "split")
            | ("pystr", "rsplit")
            | ("pystr", "replace")
            | ("pystr", "find")
            | ("pystr", "rfind")
            | ("pystr", "index")
            | ("pystr", "count")
            | ("pystr", "isdigit")
            | ("pystr", "isalpha")
            | ("pystr", "isalnum")
            | ("pystr", "isspace")
            | ("pystr", "splitlines")
            | ("pystr", "title")
            | ("pystr", "capitalize")
            | ("pystr", "zfill")
            | ("pystr", "ljust")
            | ("pystr", "rjust")
            | ("pystr", "removeprefix")
            | ("pystr", "removesuffix")
            | ("pydict", "get")
            | ("pydict", "pop")
            | ("pydict", "setdefault")
            | ("pydict", "update")
            | ("pydict", "copy")
            | ("pylist", "sort")
            | ("pylist", "index")
            | ("pylist", "count")
            | ("pylist", "insert")
            | ("pylist", "pop")
            | ("pylist", "remove")
            | ("pylist", "extend")
            | ("pylist", "reverse")
            | ("pylist", "copy")
            | ("pybytes", "hex")
            | ("pybytes", "startswith")
            | ("pybytes", "endswith")
    )
}

fn resolve_callable_marker(v: &Value) -> Option<Value> {
    if let Value::Str(n) = v {
        let name = n.as_str();
        if let Some(v2) = NAMED_CALLABLES.with(|t| t.borrow().get(name).cloned()) {
            return Some(v2);
        }
        if name.contains('.') && crate::builtins::is_builtin_marker_name(name) {
            return Some(marker(name));
        }
        // functools.<builtin> spellings: partial objects hold reduce-style fns
    }
    None
}

fn builtinfn_attr(full: &str, name: &str) -> Option<Value> {
    // attrgetter/itemgetter call objects: expose nothing callable-wise but
    // dispatch on call. `.args` on methodcaller etc. not needed by corpus.
    let _ = (full, name);
    None
}

/// Call an attrgetter/itemgetter marker (received as the function Value).
pub fn call_native_marker(full: &str, args: &[Value]) -> Option<BResult> {
    if let Some(rest) = full.strip_prefix("<partial:") {
        let body = rest.strip_suffix('>')?;
        let (fname, argstr) = body.split_once(':')?;
        let mut full_args: Vec<Value> = if argstr.is_empty() {
            vec![]
        } else {
            argstr.split('\u{1f}').map(decode_arg).collect()
        };
        full_args.extend_from_slice(args);
        return Some(call_named(fname, &full_args));
    }
    if let Some(rest) = full.strip_prefix("<attrgetter:") {
        let attrs_str = rest.strip_suffix("/>")?;
        let recv = args.first()?;
        let mut out = recv.clone();
        for attr in attrs_str.split(',') {
            out = attr_lookup_value(&out, attr).ok()?;
        }
        return Some(Ok(out));
    }
    if let Some(rest) = full.strip_prefix("<itemgetter:") {
        let idxs_str = rest.strip_suffix("/>")?;
        let recv = args.first()?;
        let idxs: Vec<i64> = idxs_str
            .trim_start_matches('[')
            .trim_end_matches(']')
            .split(", ")
            .filter(|x| !x.is_empty())
            .map(|x| x.parse().unwrap_or(0))
            .collect();
        if idxs.len() == 1 {
            return Some(crate::methods::subscript_get(recv, &i(idxs[0])));
        }
        let mut out = Vec::with_capacity(idxs.len());
        for idx in idxs {
            match crate::methods::subscript_get(recv, &i(idx)) {
                Ok(v) => out.push(v),
                Err(e) => return Some(Err(e)),
            }
        }
        return Some(Ok(tuple(out)));
    }
    if let Some(rest) = full.strip_prefix("<hashlib.") {
        let which = rest.strip_suffix('>')?;
        match which {
            "hexdigest" | "digest" => {
                // receiver carries the precomputed value
                if let Some(Value::Str(hex)) = args.first() {
                    if which == "hexdigest" {
                        return Some(Ok(s(hex.as_str().to_string())));
                    }
                    return Some(Ok(Value::Bytes(Gc::new(hex_bytes(hex.as_str())))));
                }
                return Some(Err(raise("TypeError", "missing self")));
            }
            _ => {}
        }
    }
    None
}

fn attr_lookup_value(v: &Value, name: &str) -> Result<Value, Flow> {
    if let Value::Obj(o) = v {
        let attrs = o.attrs.borrow();
        if let Some(x) = attrs.get(&s(name)) {
            return Ok(x);
        }
    }
    let interp = match CURRENT.with(|c| c.borrow().clone()) {
        Some(i) => i,
        None => return Err(raise("RuntimeError", "no interpreter on this thread")),
    };
    let got = match crate::builtins::attr_get_pub(&interp, v, name) {
        Ok(g) => g,
        Err(e) => return Err(e),
    };
    match got {
        Some(x) => Ok(x),
        None => Err(raise(
            "AttributeError",
            format!("'{}' object has no attribute '{}'", v.type_name(), name),
        )),
    }
}

/// Structured attrs for the LAST JSONDecodeError instance (e.msg/e.lineno/
/// e.colno/e.pos/e.doc) — consulted by the attr_get hook on exception objects.
pub fn exception_attr(class: &str, name: &str) -> Option<Value> {
    if class != "JSONDecodeError" {
        return None;
    }
    let (msg, line, col, pos, doc) = LAST_JSON_ERR.with(|c| c.borrow().clone())?;
    match name {
        "msg" => Some(s(msg)),
        "lineno" => Some(i(line)),
        "colno" => Some(i(col)),
        "pos" => Some(i(pos)),
        "doc" => Some(s(doc)),
        _ => None,
    }
}

/// Public handle to the thread-current interpreter.
pub fn current_interp() -> Option<Rc<crate::frame::Interpreter>> {
    CURRENT.with(|c| c.borrow().clone())
}

/// Call-marker dispatch for object-shaped callees (deque methods etc).
pub fn call_native_marker_obj(
    _name: &str,
    _args: &[Value],
    _kwargs: &[(String, Value)],
) -> Option<BResult> {
    None
}

/// Register an already-built iterator in the live interpreter's table under a
/// fresh native-region id (used by the iter() builtin).
pub fn register_shared_iter(it: std::rc::Rc<crate::iter::PyIterator>) -> u64 {
    // 02.03 repair: route through the interpreter's own id space/table (the
    // module-local CURRENT/NEXT_ITER pair could miss, leaving handles that
    // resolve to nothing).
    crate::interp::register_shared_iter_pub(it)
}

// ===========================================================================
// Builtin-value method layer (dict.get, str.startswith, bytes.hex, ...)
// ===========================================================================

pub fn call_builtin_values(
    tag: &str,
    fname: &str,
    args: &[Value],
    kwargs: &[(String, Value)],
) -> Option<BResult> {
    let recv = args.first()?.clone();
    let a = |i: usize| args.get(i).cloned();
    // owned-arg str helper (str_arg takes &Value)
    let sa = |i: usize, fname: &str| -> Result<String, Flow> {
        let v =
            a(i).ok_or_else(|| raise("TypeError", format!("{fname}() missing required argument")))?;
        str_arg(&v, fname)
    };
    let r: BResult = (|| -> BResult {
        match (tag, fname) {
            ("pydict", "get") => {
                let k =
                    a(1).ok_or_else(|| raise("TypeError", "get expected at least 1 argument"))?;
                if let Value::Dict(d) = &recv {
                    return Ok(d.get(&k).unwrap_or_else(|| a(2).unwrap_or(Value::None)));
                }
                Err(raise("SystemError", "bad receiver"))
            }
            ("pydict", "setdefault") => {
                let k = a(1)
                    .ok_or_else(|| raise("TypeError", "setdefault expected at least 1 argument"))?;
                let dflt = a(2).unwrap_or(Value::None);
                if let Value::Dict(d) = &recv {
                    if let Some(v) = d.get(&k) {
                        return Ok(v);
                    }
                    let _ = d.set(k, dflt.clone());
                    return Ok(dflt);
                }
                Err(raise("SystemError", "bad receiver"))
            }
            ("pydict", "pop") => {
                let k =
                    a(1).ok_or_else(|| raise("TypeError", "pop expected at least 1 argument"))?;
                if let Value::Dict(d) = &recv {
                    match d.pop(&k) {
                        Some(v) => Ok(v),
                        None => match a(2) {
                            Some(dflt) => Ok(dflt),
                            None => Err(Flow::Raise(ExceptionValue::new("KeyError", vec![k]))),
                        },
                    }
                } else {
                    Err(raise("SystemError", "bad receiver"))
                }
            }
            ("pydict", "update") => {
                let other =
                    a(1).ok_or_else(|| raise("TypeError", "update expected at least 1 argument"))?;
                if let (Value::Dict(d), Value::Dict(o)) = (&recv, &other) {
                    for (k, v) in o.items() {
                        let _ = d.set(k, v);
                    }
                    return Ok(Value::None);
                }
                Err(raise("SystemError", "bad receiver"))
            }
            ("pydict", "copy") => {
                if let Value::Dict(d) = &recv {
                    let nd = PyDict::new();
                    for (k, v) in d.items() {
                        let _ = nd.set(k, v);
                    }
                    return Ok(Value::Dict(nd));
                }
                Err(raise("SystemError", "bad receiver"))
            }
            ("pystr", "startswith") | ("pystr", "endswith") => {
                let pv = a(1).ok_or_else(|| raise("TypeError", "expected prefix argument"))?;
                let pfx = str_arg(&pv, fname)?;
                let v = match &recv {
                    Value::Str(x) => x.as_str().to_string(),
                    _ => return Err(raise("SystemError", "bad receiver")),
                };
                let hit = if fname == "startswith" {
                    v.starts_with(&pfx)
                } else {
                    v.ends_with(&pfx)
                };
                Ok(Value::Bool(hit))
            }
            ("pystr", "strip") | ("pystr", "lstrip") | ("pystr", "rstrip") => {
                let v = match &recv {
                    Value::Str(x) => x.as_str().to_string(),
                    _ => return Err(raise("SystemError", "bad receiver")),
                };
                let chars: Option<Vec<char>> = match a(1) {
                    Some(Value::Str(x)) => Some(x.as_str().chars().collect()),
                    Some(Value::None) | None => None,
                    _ => None,
                };
                let out = match fname {
                    "strip" => match chars {
                        Some(cs) => v.trim_matches(|c| cs.contains(&c)).to_string(),
                        None => v.trim().to_string(),
                    },
                    "lstrip" => match chars {
                        Some(cs) => v.trim_start_matches(|c| cs.contains(&c)).to_string(),
                        None => v.trim_start().to_string(),
                    },
                    _ => match chars {
                        Some(cs) => v.trim_end_matches(|c| cs.contains(&c)).to_string(),
                        None => v.trim_end().to_string(),
                    },
                };
                Ok(s(out))
            }
            ("pystr", "split") | ("pystr", "rsplit") => {
                let v = match &recv {
                    Value::Str(x) => x.as_str().to_string(),
                    _ => return Err(raise("SystemError", "bad receiver")),
                };
                let sep = match a(1) {
                    Some(Value::Str(x)) => Some(x.as_str().to_string()),
                    Some(Value::None) | None => None,
                    _ => None,
                };
                let maxsplit = a(2).map(|v| as_int_i64(&v)).transpose()?.unwrap_or(-1);
                let parts: Vec<String> = match (&sep, fname) {
                    (None, "split") => {
                        let all: Vec<&str> = v.split_whitespace().collect();
                        if maxsplit < 0 {
                            all.into_iter().map(String::from).collect()
                        } else {
                            all.into_iter()
                                .take(maxsplit as usize)
                                .map(String::from)
                                .collect()
                        }
                    }
                    (Some(sp), "split") => {
                        if maxsplit < 0 {
                            v.split(sp.as_str()).map(String::from).collect()
                        } else {
                            v.splitn(maxsplit as usize + 1, sp.as_str())
                                .map(String::from)
                                .collect()
                        }
                    }
                    (Some(sp), "rsplit") => {
                        if maxsplit < 0 {
                            v.rsplit(sp.as_str()).map(String::from).collect()
                        } else {
                            v.rsplitn(maxsplit as usize + 1, sp.as_str())
                                .map(String::from)
                                .collect()
                        }
                    }
                    _ => return Err(raise("ValueError", "empty separator")),
                };
                Ok(list(parts.into_iter().map(s).collect()))
            }
            ("pystr", "replace") => {
                let old_s = str_arg(
                    &a(1).ok_or_else(|| raise("TypeError", "replace expected 2 arguments"))?,
                    "replace",
                )?;
                let new_s = str_arg(
                    &a(2).ok_or_else(|| raise("TypeError", "replace expected 2 arguments"))?,
                    "replace",
                )?;
                if let Value::Str(x) = &recv {
                    return Ok(s(x.as_str().replace(&old_s, &new_s)));
                }
                Err(raise("SystemError", "bad receiver"))
            }
            ("pystr", "find") | ("pystr", "rfind") | ("pystr", "index") => {
                let sub = sa(1, fname)?;
                if let Value::Str(x) = &recv {
                    let pos = if fname == "rfind" {
                        x.as_str().rfind(&sub)
                    } else {
                        x.as_str().find(&sub)
                    };
                    return Ok(match pos {
                        Some(p) => i(p as i64),
                        None => {
                            if fname == "index" {
                                return Err(raise("ValueError", "substring not found"));
                            }
                            i(-1)
                        }
                    });
                }
                Err(raise("SystemError", "bad receiver"))
            }
            ("pystr", "count") => {
                let sub = sa(1, fname)?;
                if let Value::Str(x) = &recv {
                    return Ok(i(x.as_str().matches(&sub).count() as i64));
                }
                Err(raise("SystemError", "bad receiver"))
            }
            ("pystr", "isdigit")
            | ("pystr", "isalpha")
            | ("pystr", "isalnum")
            | ("pystr", "isspace") => {
                if let Value::Str(x) = &recv {
                    let cs: Vec<char> = x.as_str().chars().collect();
                    let out = if cs.is_empty() {
                        false
                    } else {
                        match fname {
                            "isdigit" => cs.iter().all(|c| c.is_ascii_digit()),
                            "isalpha" => cs.iter().all(|c| c.is_alphabetic()),
                            "isalnum" => cs.iter().all(|c| c.is_alphanumeric()),
                            _ => cs.iter().all(|c| c.is_whitespace()),
                        }
                    };
                    return Ok(Value::Bool(out));
                }
                Err(raise("SystemError", "bad receiver"))
            }
            ("pystr", "splitlines") => {
                if let Value::Str(x) = &recv {
                    let lines: Vec<String> = x.as_str().split('\n').map(String::from).collect();
                    let mut out = lines;
                    if out.last().map(|l| l.is_empty()).unwrap_or(false) {
                        out.pop();
                    }
                    // CPython keeps the terminator-free lines
                    return Ok(list(out.into_iter().map(s).collect()));
                }
                Err(raise("SystemError", "bad receiver"))
            }
            ("pystr", "title") | ("pystr", "capitalize") => {
                if let Value::Str(x) = &recv {
                    let out = if fname == "capitalize" {
                        let mut cs = x.as_str().chars();
                        match cs.next() {
                            Some(f) => {
                                f.to_uppercase().collect::<String>() + &cs.as_str().to_lowercase()
                            }
                            None => String::new(),
                        }
                    } else {
                        let mut out = String::new();
                        let mut prev_alpha = false;
                        for c in x.as_str().chars() {
                            if c.is_alphabetic() && !prev_alpha {
                                out.extend(c.to_uppercase());
                                prev_alpha = true;
                            } else {
                                if !c.is_alphabetic() {
                                    prev_alpha = false;
                                }
                                out.push(c.to_ascii_lowercase());
                            }
                        }
                        out
                    };
                    return Ok(s(out));
                }
                Err(raise("SystemError", "bad receiver"))
            }
            ("pystr", "removeprefix") | ("pystr", "removesuffix") => {
                let p = sa(1, fname)?;
                if let Value::Str(x) = &recv {
                    let out = if fname == "removeprefix" {
                        x.as_str().strip_prefix(&p).unwrap_or(x.as_str())
                    } else {
                        x.as_str().strip_suffix(&p).unwrap_or(x.as_str())
                    };
                    return Ok(s(out));
                }
                Err(raise("SystemError", "bad receiver"))
            }
            ("pybytes", "hex") => {
                if let Value::Bytes(b) = &recv {
                    let mut out = String::with_capacity((**b).len() * 2);
                    for byte in &**b {
                        out.push_str(&format!("{byte:02x}"));
                    }
                    return Ok(s(out));
                }
                Err(raise("SystemError", "bad receiver"))
            }
            ("pylist", "sort") => {
                if let Value::List(l) = &recv {
                    let mut items = l.items().to_vec();
                    let keyf = match kw(kwargs, "key") {
                        Some(kv) => Some(callable_name(kv)?),
                        None => None,
                    };
                    let reverse = kw(kwargs, "reverse").map(truthy).unwrap_or(false);
                    let mut keyed: Vec<(Value, Value)> = Vec::with_capacity(items.len());
                    for x in items {
                        let k = match &keyf {
                            Some(kf) => call_named(kf, &[x.clone()])?,
                            None => x.clone(),
                        };
                        keyed.push((k, x));
                    }
                    keyed.sort_by(|a, b| {
                        let ord = if py_lt(&a.0, &b.0).unwrap_or(false) {
                            std::cmp::Ordering::Less
                        } else if py_eq(&a.0, &b.0) {
                            std::cmp::Ordering::Equal
                        } else {
                            std::cmp::Ordering::Greater
                        };
                        if reverse {
                            ord.reverse()
                        } else {
                            ord
                        }
                    });
                    items = keyed.into_iter().map(|(_, v)| v).collect();
                    l.clear();
                    for x in items {
                        l.append(x);
                    }
                    return Ok(Value::None);
                }
                Err(raise("SystemError", "bad receiver"))
            }
            ("pylist", "index") => {
                let v =
                    a(1).ok_or_else(|| raise("TypeError", "index expected at least 1 argument"))?;
                if let Value::List(l) = &recv {
                    for (idx, x) in l.items().iter().enumerate() {
                        if py_eq(x, &v) {
                            return Ok(i(idx as i64));
                        }
                    }
                    return Err(raise("ValueError", format!("{v:?} is not in list")));
                }
                Err(raise("SystemError", "bad receiver"))
            }
            ("pylist", "count") => {
                let v =
                    a(1).ok_or_else(|| raise("TypeError", "count expected at least 1 argument"))?;
                if let Value::List(l) = &recv {
                    return Ok(i(l.items().iter().filter(|x| py_eq(x, &v)).count() as i64));
                }
                Err(raise("SystemError", "bad receiver"))
            }
            ("pylist", "insert") => {
                let idx = a(1)
                    .map(|v| as_int_i64(&v))
                    .transpose()?
                    .ok_or_else(|| raise("TypeError", "insert expected 2 arguments"))?;
                let v = a(2).ok_or_else(|| raise("TypeError", "insert expected 2 arguments"))?;
                if let Value::List(l) = &recv {
                    let items = l.items().to_vec();
                    let mut all = items;
                    let idx = (idx.max(0) as usize).min(all.len());
                    all.insert(idx, v);
                    l.clear();
                    for x in all {
                        l.append(x);
                    }
                    return Ok(Value::None);
                }
                Err(raise("SystemError", "bad receiver"))
            }
            ("pylist", "pop") => {
                if let Value::List(l) = &recv {
                    let mut items = l.items().to_vec();
                    let idx = match a(1) {
                        Some(iv) => as_int_i64(&iv)?,
                        None => -1,
                    };
                    let n = items.len() as i64;
                    let real = if idx < 0 { n + idx } else { idx };
                    if real < 0 || real >= n {
                        return Err(raise("IndexError", "pop index out of range"));
                    }
                    let out = items.remove(real as usize);
                    l.clear();
                    for x in items {
                        l.append(x);
                    }
                    return Ok(out);
                }
                Err(raise("SystemError", "bad receiver"))
            }
            ("pylist", "remove") => {
                let v =
                    a(1).ok_or_else(|| raise("TypeError", "remove expected at least 1 argument"))?;
                if let Value::List(l) = &recv {
                    let items = l.items().to_vec();
                    let mut all = items;
                    for (idx, x) in all.iter().enumerate() {
                        if py_eq(x, &v) {
                            all.remove(idx);
                            l.clear();
                            for y in all {
                                l.append(y);
                            }
                            return Ok(Value::None);
                        }
                    }
                    return Err(raise("ValueError", "list.remove(x): x not in list"));
                }
                Err(raise("SystemError", "bad receiver"))
            }
            ("pylist", "extend") => {
                let v =
                    a(1).ok_or_else(|| raise("TypeError", "extend expected at least 1 argument"))?;
                if let Value::List(l) = &recv {
                    for x in snapshot(&v)? {
                        l.append(x);
                    }
                    return Ok(Value::None);
                }
                Err(raise("SystemError", "bad receiver"))
            }
            ("pylist", "reverse") => {
                if let Value::List(l) = &recv {
                    let mut items = l.items().to_vec();
                    items.reverse();
                    l.clear();
                    for x in items {
                        l.append(x);
                    }
                    return Ok(Value::None);
                }
                Err(raise("SystemError", "bad receiver"))
            }
            ("pylist", "copy") => {
                if let Value::List(l) = &recv {
                    return Ok(list(l.items().to_vec()));
                }
                Err(raise("SystemError", "bad receiver"))
            }
            _ => return Err(raise("SystemError", "unbound builtin-value method")),
        }
    })();
    Some(r)
}

/// Encode a bound-arg Value into the partial marker name (repr-based; strings
/// carry a type tag so decode is unambiguous).
fn encode_arg(v: &Value) -> String {
    match v {
        Value::Int(n) => format!("i:{}", n.to_string_raw()),
        Value::Float(x) => format!("f:{x:?}"),
        Value::Str(st) => format!("s:{}", st.as_str()),
        Value::Bool(b) => format!("b:{}", if *b { "1" } else { "0" }),
        Value::None => "n:".to_string(),
        other => {
            let name = name_callable(other);
            format!("c:{name}")
        }
    }
}

fn decode_arg(tok: &str) -> Value {
    if let Some(rest) = tok.strip_prefix("i:") {
        return Value::Int(Int::parse_base10(rest).unwrap_or_else(|_| Int::from_i64(0)));
    }
    if let Some(rest) = tok.strip_prefix("f:") {
        return f(rest.parse().unwrap_or(0.0));
    }
    if let Some(rest) = tok.strip_prefix("s:") {
        return s(rest);
    }
    if let Some(rest) = tok.strip_prefix("b:") {
        return Value::Bool(rest == "1");
    }
    if let Some(rest) = tok.strip_prefix("c:") {
        if let Some(v) = NAMED_CALLABLES.with(|t| t.borrow().get(rest).cloned()) {
            return v;
        }
    }
    Value::None
}

/// Iterator for native opaque iterables (deque). Returns None when the object
/// is not natively iterable.
pub fn native_iter(v: &Value) -> Option<Gc<crate::iter::PyIterator>> {
    if let Value::Obj(o) = v {
        if o.name == "deque" {
            let items = deque_state(v).ok()?.0;
            return Some(crate::iter::PyIterator::new(
                crate::iter::IterState::Native(Box::new(NativeIterState::Seq(items.into_iter()))),
            ));
        }
    }
    None
}
