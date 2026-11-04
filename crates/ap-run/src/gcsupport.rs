//! gcsupport — ap-run's wiring of the ap-gc collector (leaf 02.04 STAGE 1).
//!
//! Owns:
//! - the `gc` module surface (collect/get_count/get_threshold/set_threshold/
//!   disable/enable/isenabled/get_count/callbacks/garbage/freeze/unfreeze/
//!   is_tracked/get_objects/get_stats) with CPython-3.14-verified shapes
//!   (gc006/gc011/gc016 corpus pins);
//! - the `weakref` + `_weakref` module surfaces (ref/proxy/finalize/
//!   WeakSet basics; TypeError texts byte-exact vs 3.14.7 for the
//!   non-weakrefable builtin values — gc005);
//! - the `__del__` bridge (bridge_del): calls the object's finalizer through
//!   the interpreter with unraisable-exception reporting (C-21/gc009);
//! - the weakref callback bridge (fires once per dead target, C-22);
//! - root registration + the weakbridge install at run entry.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::frame::Interpreter;
use crate::Flow;
use ap_obj::dict::PyDict;
use ap_obj::int::{Gc, Int};
use ap_obj::value::{PyList, PyString, Tuple, Value};
use ap_obj::PyObject;

use crate::builtins_modules::{i, s, BResult};
use crate::exc_hier::ExceptionValue;

pub type EResult = Result<Value, Flow>;

fn raise(class: &str, msg: impl Into<String>) -> Flow {
    Flow::Raise(ExceptionValue::msg(class, msg.into()))
}

// ---------------------------------------------------------------------------
// Weakref registry (Python-visible ref tokens)
// ---------------------------------------------------------------------------

/// One live weakref token: the `_weakref_token` PyObject address is the key;
/// the target payload address + callback + dead-flag ride alongside.
struct WeakToken {
    token: Gc<PyObject>,
    target_addr: usize,
    callback: Option<Value>,
    /// Registration index of the ap-gc WeakEntry (for dead-flag queries).
    entry_index: usize,
}

thread_local! {
    static WEAK_TOKENS: RefCell<Vec<WeakToken>> = RefCell::new(Vec::new());
    /// Finalizer registry for weakref.finalize (obj-addr -> finalize token).
    static FINALIZERS: RefCell<Vec<FinalizerEntry>> = RefCell::new(Vec::new());
}

struct FinalizerEntry {
    token: Gc<PyObject>, // the finalize() object (alive bookkeeping)
    target_addr: usize,
    func: Value,
    args: Vec<Value>,
    kwargs: Vec<(String, Value)>,
    atexit: bool,
    index: u64,
    detached: bool,
}

static mut FINALIZER_SEQ: u64 = 0;

fn next_finalizer_index() -> u64 {
    // Single-threaded v0: static mut under an exclusive interpreter thread.
    unsafe {
        FINALIZER_SEQ += 1;
        FINALIZER_SEQ
    }
}

/// Can this value be weak-referenced? (gc005 matrix: int/str/float/bytes/
/// tuple/list/dict NO; set/frozenset + user instances + classes YES.)
fn weakrefable(v: &Value) -> bool {
    match v {
        Value::Int(_)
        | Value::Str(_)
        | Value::Float(_)
        | Value::Bytes(_)
        | Value::Bool(_)
        | Value::None
        | Value::Tuple(_)
        | Value::List(_)
        | Value::Dict(_)
        | Value::Ellipsis => false,
        Value::Set(_) | Value::Frozen(_) => true,
        // Slotted instances (no __dict__, no __weakref__ slot) are NOT
        // weakrefable in CPython — gc005/s034's NonRef probe.
        // W112: __slots__ DECLARING "__weakref__" makes the instance
        // weakrefable (scrapy Request declares it; trackref's
        // WeakKeyDictionary depends on it). An instance-dict or a
        // dict-bearing base also weakrefables (existing behavior).
        Value::Obj(o) if o.kind == ap_obj::ObjKind::UserInstance => {
            if let Some(t) = o.type_ref.borrow().as_ref().and_then(|w| w.upgrade()) {
                if t.has_instance_dict() {
                    return true;
                }
                return t.mro.borrow().iter().any(|c| {
                    c.flags
                        .slots
                        .as_ref()
                        .is_some_and(|s: &Vec<String>| s.iter().any(|x| x == "__weakref__"))
                });
            }
            true
        }
        Value::Obj(o) => !matches!(o.kind, ap_obj::ObjKind::Iterator),
    }
}

/// weakref.ref(ob[, callback]) — creates a token.
pub fn weakref_ref(args: &[Value], kwargs: &[(String, Value)]) -> BResult {
    let target = args
        .first()
        .ok_or_else(|| raise("TypeError", "weakref.ref() needs a target"))?;
    if !weakrefable(target) {
        return Err(raise(
            "TypeError",
            format!(
                "cannot create weak reference to '{}' object",
                crate::classes::display_type_name(target)
            ),
        ));
    }
    let callback = args
        .get(1)
        .cloned()
        .or_else(|| kwarg_value(kwargs, "callback").cloned());
    let target_addr = target_addr_of(target);
    // s016: interned Type objects (list, dict, ...) are NOT GC-tracked
    // (interp.type_objects holds them), so a weak token reports dead at the
    // first prune and ABC registries (WeakSet of classes) silently empty —
    // MutableSequence.register(list) vanished. They are immortal for the
    // run: pin a strong clone and skip the ap-gc entry (no callback to
    // mis-fire); token_alive/weakref_call answer from the pin table.
    let untracked_immortal = matches!(target, Value::Obj(o) if o.kind == ap_obj::ObjKind::Type);
    if untracked_immortal {
        INTERNED_TYPE_PINS.with(|p| p.borrow_mut().push(target.clone()));
    }
    // arg 2 (internal) names the token kind: "ref" or "_weakref_proxy".
    let token_name = match args.get(2) {
        Some(Value::Str(x)) if x.as_str() == "_weakref_proxy" => "_weakref_proxy",
        _ => "_weakref_token",
    };
    let token = PyObject::new(ap_obj::ObjKind::Opaque, token_name);
    let entry_index = ap_gc::weakref_entry_count();
    {
        let mut tattrs = token.attrs.borrow_mut();
        let _ = tattrs.set(s("<target_addr>"), i(target_addr as i64));
    }
    if !untracked_immortal {
        ap_gc::weakref_add(ap_gc::WeakEntry {
            target: target_addr,
            token: Value::Obj(token.clone()),
            callback,
            dead: false,
            fired: false,
        });
        WEAK_TOKENS.with(|w| {
            w.borrow_mut().push(WeakToken {
                token: token.clone(),
                target_addr,
                callback: None, // callback lives in the ap-gc entry
                entry_index,
            });
        });
    }
    Ok(Value::Obj(token))
}

fn kwarg_value<'a>(kwargs: &'a [(String, Value)], name: &str) -> Option<&'a Value> {
    kwargs.iter().find(|(k, _)| k == name).map(|(_, v)| v)
}

/// weakref.proxy(ob[, callback]) — a transparent stand-in. The token is an
/// opaque carrying only `<target_addr>`; the interp attr/call layers forward
/// to the live target (dead target -> ReferenceError, C-22).
pub fn weakref_proxy(args: &[Value], kwargs: &[(String, Value)]) -> BResult {
    let target = args
        .first()
        .ok_or_else(|| raise("TypeError", "weakref.proxy() needs a target"))?;
    if !weakrefable(target) {
        return Err(raise(
            "TypeError",
            format!(
                "cannot create weak reference to '{}' object",
                crate::classes::display_type_name(target)
            ),
        ));
    }
    let callback = args
        .get(1)
        .cloned()
        .or_else(|| kwarg_value(kwargs, "callback").cloned());
    let target_addr = target_addr_of(target);
    let token = PyObject::new(ap_obj::ObjKind::Opaque, "_weakref_proxy");
    {
        let mut tattrs = token.attrs.borrow_mut();
        let _ = tattrs.set(s("<target_addr>"), i(target_addr as i64));
    }
    ap_gc::weakref_add(ap_gc::WeakEntry {
        target: target_addr,
        token: Value::Obj(token.clone()),
        callback,
        dead: false,
        fired: false,
    });
    WEAK_TOKENS.with(|w| {
        w.borrow_mut().push(WeakToken {
            token: token.clone(),
            target_addr,
            callback: None,
            entry_index: ap_gc::weakref_entry_count(),
        });
    });
    Ok(Value::Obj(token))
}

fn target_addr_of(v: &Value) -> usize {
    match v {
        Value::Str(x) => Rc::as_ptr(x) as *const u8 as usize,
        Value::Bytes(x) => Rc::as_ptr(x) as *const u8 as usize,
        Value::List(x) => Rc::as_ptr(x) as *const u8 as usize,
        Value::Tuple(x) => Rc::as_ptr(x) as *const u8 as usize,
        Value::Dict(x) => Rc::as_ptr(x) as *const u8 as usize,
        Value::Set(x) | Value::Frozen(x) => Rc::as_ptr(x) as *const u8 as usize,
        Value::Obj(x) => Rc::as_ptr(x) as *const u8 as usize,
        _ => 0,
    }
}

/// Look up the target VALUE for a weak address (registry projection; the
/// registry clone keeps tracked objects alive, so membership == liveness).
pub fn target_value_by_addr(addr: usize) -> Option<Value> {
    ap_gc::get_objects()
        .into_iter()
        .find(|v| target_addr_of(v) == addr)
}

/// `ref()` — return the target while alive, None when dead (C-22).
pub fn weakref_call(token: &Gc<PyObject>) -> Value {
    let addr = target_addr_of_token(token);
    let pinned = INTERNED_TYPE_PINS.with(|p| p.borrow().iter().any(|v| target_addr_of(v) == addr));
    if addr == 0 || (!ap_gc::weakref_alive(addr) && !pinned) {
        return Value::None;
    }
    target_value_by_addr(addr).unwrap_or_else(|| {
        // Pinned (non-GC-tracked) targets: recover from the pin table.
        INTERNED_TYPE_PINS.with(|p| {
            p.borrow()
                .iter()
                .find(|v| target_addr_of(v) == addr)
                .cloned()
                .unwrap_or(Value::None)
        })
    })
}

/// The `<target_addr>` recorded on a token object (0 when absent).
fn target_addr_of_token(token: &Gc<PyObject>) -> usize {
    token
        .attrs
        .borrow()
        .get(&s("<target_addr>"))
        .and_then(|v| v.as_int())
        .and_then(|iv| iv.to_i64())
        .map(|n| n.max(0) as usize)
        .unwrap_or(0)
}

/// Drain dead weakrefs: fire callbacks exactly once (ap-gc bridge).
pub fn fire_dead_weakrefs() {
    let dead = ap_gc::weakref_take_dead();
    for (target_addr, callback) in dead {
        let Some(cb) = callback else { continue };
        // weakref.finalize entries carry a native dispatch marker as the
        // callback; their tokens live in FINALIZERS, not WEAK_TOKENS, so
        // resolve the token from the finalize table first (else the
        // WEAK_TOKENS miss below silently skips the finalize — observed:
        // gc015 "after collect: [] alive: True").
        if let Value::Obj(m) = &cb {
            if m.kind == ap_obj::ObjKind::BuiltinFn && m.name == "__ap_finalize_dispatch__" {
                let tok = FINALIZERS.with(|f| {
                    f.borrow()
                        .iter()
                        .find(|e| e.target_addr == target_addr)
                        .map(|e| Value::Obj(e.token.clone()))
                });
                if let Some(Value::Obj(tok)) = tok {
                    finalize_call(&tok);
                }
                continue;
            }
        }
        // The callback receives the ref token (CPython passes the ref).
        let token_val = WEAK_TOKENS.with(|w| {
            w.borrow()
                .iter()
                .find(|t| t.target_addr == target_addr)
                .map(|t| Value::Obj(t.token.clone()))
        });
        let Some(token_val) = token_val else { continue };
        let _ = invoke_callback(&cb, &[token_val]);
    }
}

/// Call a stored callback Value (user fn or named callable) with args.
fn invoke_callback(f: &Value, args: &[Value]) -> EResult {
    let Some(interp) = crate::builtins_modules::current_interp() else {
        return Ok(Value::None);
    };
    crate::interp::call_value_pub(&interp, f, args, &[])
}

// ---------------------------------------------------------------------------
// Finalizers (__del__)
// ---------------------------------------------------------------------------

/// ap-gc bridge: call `__del__` on a garbage object exactly once. Errors are
/// unraisable: reported to stderr in the C-21 shape, never propagated.
pub fn bridge_del(v: &Value) {
    let Some(interp) = crate::builtins_modules::current_interp() else {
        return;
    };
    let del_f = match lookup_del(&interp, v) {
        Some(f) => f,
        None => return,
    };
    let args = [v.clone()];
    let r = crate::interp::call_value_pub(&interp, &del_f, &args, &[]);
    if let Err(Flow::Raise(exc)) = r {
        report_unraisable(&interp, &del_f, &exc);
    }
}

/// Look up `__del__` on a UserInstance (class MRO).
fn lookup_del(interp: &Rc<Interpreter>, v: &Value) -> Option<Value> {
    match v {
        Value::Obj(o) if o.kind == ap_obj::ObjKind::UserInstance => {
            let t = o.type_ref.borrow().as_ref().and_then(|w| w.upgrade())?;
            let (f, _owner) = t.lookup_with_owner("__del__")?;
            Some(f)
        }
        _ => None,
    }
}

/// The C-21 unraisable report: `Exception ignored while calling deallocator
/// <function Bad.__del__ at 0xADDR>:` + traceback + `ExcType: msg`, to the
/// script's sys.unraisablehook when one is set (gc009), else real stderr.
fn report_unraisable(interp: &Rc<Interpreter>, del_f: &Value, exc: &ExceptionValue) {
    // sys.unraisablehook override (gc009's structured path). The attribute
    // is seeded as None (native default); only a real callable swaps to the
    // hook path — None must fall through to the stderr renderer.
    let hook = sys_attr(interp, "unraisablehook").filter(|h| !matches!(h, Value::None));
    if let Some(hook) = hook {
        // args object: exc_type, exc_value, exc_traceback, object
        let args_obj = PyObject::new(ap_obj::ObjKind::Namespace, "unraisable_args");
        let exc_type = {
            let t = ap_obj::PyObject::new(ap_obj::ObjKind::Type, exc.class.clone());
            Value::Obj(t)
        };
        // W112: exc_value must be the exception INSTANCE (CPython
        // sys.unraisablehook contract). The first constructor ARG is a
        // plain value (often a str); pytest's unraisable collector
        // stores it and later reads traceback attrs off it, crashing
        // with AttributeError on _rawentry after every failing test.
        let exc_value = exc
            .instance
            .clone()
            .unwrap_or_else(|| crate::interp::exception_instance_value(exc));
        {
            let mut a = args_obj.attrs.borrow_mut();
            let _ = a.set(s("exc_type"), exc_type);
            let _ = a.set(s("exc_value"), exc_value);
            // W112: real tb chain (was a BuiltinFn marker object -
            // pytest's unraisable collector stores it and later walks
            // it as a traceback, AttributeError-ing on missing attrs).
            let exc_tb = crate::interp::build_tb_chain(
                &interp.script_path,
                &exc.tb,
            );
            let _ = a.set(s("exc_traceback"), exc_tb.clone());
            // Also pin __traceback__ on the instance itself: class-tree
            // exception instances route missing-attr lookups to the
            // CLASS (a type object), and pytest reads
            // exc_value.__traceback__ - without the pinned attr the
            // lookup failed on the class.
            if let Some(Value::Obj(iov)) = exc.instance.clone() {
                let _ = iov
                    .attrs
                    .borrow_mut()
                    .set(Value::Str(PyString::new("__traceback__")), exc_tb);
            }
            let _ = a.set(s("object"), Value::None);
        }
        let _ = crate::interp::call_value_pub(interp, &hook, &[Value::Obj(args_obj)], &[]);
        return;
    }

    // Default: render the C-21 stderr shape into the interpreter's stderr
    // buffer (sys.stderr may be swapped; CPython writes unraisables through
    // sys.stderr — gc009 captures them via the StringIO swap).
    let fname = match del_f {
        Value::Obj(o) => o.name.clone(),
        _ => "__del__".to_string(),
    };
    // Function repr with address: <function Bad.__del__ at 0x...>. CPython
    // prints the qualified name; look it up via the fn registry (fn_id).
    let addr = match del_f {
        Value::Obj(o) => Rc::as_ptr(o) as *const u8 as usize,
        _ => 0,
    };
    let qual = match del_f {
        Value::Obj(o) => o
            .fn_id
            .and_then(|id| interp.fn_entry(id))
            .map(|e| e.qualname)
            .unwrap_or_else(|| fname.clone()),
        _ => fname.clone(),
    };
    let mut text =
        format!("Exception ignored while calling deallocator <function {qual} at 0x{addr:x}>:\n");
    text.push_str("Traceback (most recent call last):\n");
    // Frame line: the raise site. exc.raise_line is captured at the Raise
    // arm from the frame_stack top — which at raise time IS the finalizer's
    // own cell (call_function pushed it before the body ran).
    let line = if exc.raise_line > 0 {
        exc.raise_line
    } else {
        interp
            .frame_stack
            .borrow()
            .first()
            .map(|c| c.get())
            .filter(|n| *n > 0)
            .unwrap_or(1)
    };
    text.push_str(&format!(
        "  File \"{}\", line {}, in {}\n",
        interp.script_path, line, fname
    ));
    if let Some(src) = interp.source.lines().nth(line.saturating_sub(1)) {
        text.push_str("    ");
        // CPython strips the source line and re-indents 4 spaces
        // (sys.unraisablehook text shaping) — trim_end kept raw leading
        // indent (gc009, issue #10).
        text.push_str(src.trim());
        text.push('\n');
    }
    let msg = exc.py_str().unwrap_or_default();
    if msg.is_empty() {
        text.push_str(&exc.class);
    } else {
        text.push_str(&format!("{}: {}", exc.class, msg));
    }
    text.push('\n');
    write_stderr(interp, &text);
}

fn sys_attr(interp: &Rc<Interpreter>, name: &str) -> Option<Value> {
    let sys_v = interp.sys_module.borrow().clone()?;
    match sys_v {
        Value::Obj(o) if o.kind == ap_obj::ObjKind::Module => o
            .module_ns
            .borrow()
            .as_ref()
            .and_then(|ns| ns.get(&Value::Str(PyString::new(name)))),
        _ => None,
    }
}

fn write_stderr(interp: &Rc<Interpreter>, text: &str) {
    // Route through sys.stderr's current binding (gc009 swaps it). Resolve
    // `write` through the interpreter's attribute protocol, not a raw attrs
    // lookup: native streams (io.StringIO et al) keep `write` in the methods
    // table, so attrs.get misses and the report silently falls through to
    // eprint! — observed: the swapped StringIO captured nothing.
    if let Some(se) = sys_attr(interp, "stderr") {
        if let Ok(Some(w)) = crate::interp::attr_get_public(interp, &se, "write") {
            let _ = crate::interp::call_value_pub(
                interp,
                &w,
                &[Value::Str(PyString::new(text.to_string()))],
                &[],
            );
            return;
        }
    }
    eprint!("{text}");
}

// ---------------------------------------------------------------------------
// gc module surface
// ---------------------------------------------------------------------------

/// Build the `gc` module Value (native-registry builder wrapper).
pub fn build_gc_module_pub() -> Value {
    build_gc_module_inner()
}

fn build_gc_module_inner() -> Value {
    let m = PyObject::new(ap_obj::ObjKind::Module, "gc");
    let ns = PyDict::new();
    let _ = ns.set(s("__name__"), s("gc"));
    let _ = ns.set(s("__file__"), Value::None);
    let _ = ns.set(s("__doc__"), Value::None);
    for n in [
        "collect",
        "get_count",
        "get_threshold",
        "set_threshold",
        "disable",
        "enable",
        "isenabled",
        "get_stats",
        "get_objects",
        "is_tracked",
        "freeze",
        "unfreeze",
        "get_referrers",
        "get_referents",
        "get_debug",
        "set_debug",
    ] {
        let _ = ns.set(s(n), marker(&format!("gc.{n}")));
    }
    // callbacks: a real list object (gc006 appends/clears) registered as
    // the collector's callback source.
    let callbacks = PyList::from_vec(vec![]);
    ap_gc::callbacks_set_list(&callbacks);
    let _ = ns.set(s("callbacks"), Value::List(callbacks));
    // garbage: an (always empty in v0 — PEP 442) list object.
    let _ = ns.set(s("garbage"), Value::List(PyList::from_vec(vec![])));
    // Debug flags constants.
    let _ = ns.set(s("DEBUG_STATS"), i(1));
    let _ = ns.set(s("DEBUG_COLLECTABLE"), i(2));
    let _ = ns.set(s("DEBUG_UNCOLLECTABLE"), i(4));
    let _ = ns.set(s("DEBUG_SAVEALL"), i(32));
    let _ = ns.set(s("DEBUG_LEAK"), i(37));
    let _ = m.module_ns.borrow_mut().replace(Rc::new((*ns).clone()));
    Value::Obj(m)
}

fn marker(name: &str) -> Value {
    Value::Obj(PyObject::new(ap_obj::ObjKind::BuiltinFn, name))
}

/// Dispatch a `gc.<fn>` call (routed from builtins::call_builtin_dotted).
pub fn call_gc(name: &str, args: &[Value], kwargs: &[(String, Value)]) -> Option<BResult> {
    let r: BResult = (|| {
        match name {
            "collect" => {
                // Optional generation arg (0/1/2) — stage 1 is generationless;
                // the arg only pins the generation field gc006 observes.
                let gen = ap_gc_arg(args, 0, 2)?;
                let found = ap_gc::collect_gen(gen);
                Ok(i(found as i64))
            }
            "get_count" => {
                let (a, b, c) = ap_gc::get_count();
                Ok(Value::Tuple(Gc::new(Tuple(vec![
                    i(a as i64),
                    i(b as i64),
                    i(c as i64),
                ]))))
            }
            "get_threshold" => {
                let (a, b, c) = ap_gc::get_threshold();
                Ok(Value::Tuple(Gc::new(Tuple(vec![
                    i(a as i64),
                    i(b as i64),
                    i(c as i64),
                ]))))
            }
            "set_threshold" => {
                let t0 = ap_gc_arg(args, 0, 700)?;
                let t1 = ap_gc_arg(args, 1, 10)?;
                let t2 = ap_gc_arg(args, 2, 10)?;
                ap_gc::set_threshold(t0, t1, t2);
                Ok(Value::None)
            }
            "disable" => {
                ap_gc::disable();
                Ok(Value::None)
            }
            "enable" => {
                ap_gc::enable();
                Ok(Value::None)
            }
            "isenabled" => Ok(Value::Bool(ap_gc::is_enabled())),
            "freeze" => {
                ap_gc::freeze();
                Ok(Value::None)
            }
            "unfreeze" => {
                ap_gc::unfreeze();
                Ok(Value::None)
            }
            "is_tracked" => {
                let v = args
                    .first()
                    .ok_or_else(|| raise("TypeError", "is_tracked() missing required argument"))?;
                Ok(Value::Bool(ap_gc::is_tracked(v)))
            }
            "get_objects" => {
                let objs = ap_gc::get_objects();
                Ok(Value::List(PyList::from_vec(objs)))
            }
            "get_stats" => {
                // Three generation dicts (v0: gen0 holds everything).
                let st = ap_gc::stats();
                let gen = |collections: u64, collected: u64, uncollectable: u64| {
                    let d = PyDict::new();
                    let _ = d.set(s("collections"), i(collections as i64));
                    let _ = d.set(s("collected"), i(collected as i64));
                    let _ = d.set(s("uncollectable"), i(uncollectable as i64));
                    Value::Dict(d)
                };
                Ok(Value::List(PyList::from_vec(vec![
                    gen(st.collections, st.objects_reclaimed, 0),
                    gen(0, 0, 0),
                    gen(0, 0, 0),
                ])))
            }
            "get_debug" => Ok(i(0)),
            "set_debug" => Ok(Value::None),
            "get_referrers" => Ok(Value::List(PyList::from_vec(vec![]))),
            "get_referents" => {
                // v0: direct referents of the argument (documented gap:
                // full transitive closure needs the edge walker exposed).
                let mut out = Vec::new();
                if let Some(v) = args.first() {
                    referents_of(v, &mut out);
                }
                Ok(Value::List(PyList::from_vec(out)))
            }
            _ => Err(raise(
                "AttributeError",
                format!("module 'gc' has no attribute '{name}'"),
            )),
        }
    })();
    let _ = kwargs;
    Some(r)
}

fn ap_gc_arg(args: &[Value], idx: usize, default: usize) -> Result<usize, Flow> {
    match args.get(idx) {
        None => Ok(default),
        Some(v) => v
            .as_int()
            .and_then(|i| i.to_i64())
            .map(|n| n.max(0) as usize)
            .ok_or_else(|| raise("TypeError", "an integer is required")),
    }
}

fn referents_of(v: &Value, out: &mut Vec<Value>) {
    match v {
        Value::List(l) => out.extend(l.items().iter().cloned()),
        Value::Tuple(t) => out.extend(t.0.iter().cloned()),
        Value::Dict(d) => {
            for (k, val) in d.items() {
                out.push(k);
                out.push(val);
            }
        }
        Value::Set(s) | Value::Frozen(s) => out.extend(s.iter()),
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// weakref module surface
// ---------------------------------------------------------------------------

/// Build the `weakref` module (native-registry builder wrapper).
pub fn build_weakref_module_pub() -> Value {
    let m = PyObject::new(ap_obj::ObjKind::Module, "weakref");
    let ns = PyDict::new();
    let _ = ns.set(s("__name__"), s("weakref"));
    let _ = ns.set(s("__file__"), Value::None);
    // ref IS a type (class KeyedRef(_weakref.ref) in importlib._bootstrap;
    // WeakMethod subclasses it conceptually). ReferenceType is the same
    // object (CPython: ReferenceType is type(ref())).
    let ref_type = Value::Obj(PyObject::new(ap_obj::ObjKind::Type, "ref"));
    let _ = ns.set(s("ref"), ref_type.clone());
    let _ = ns.set(s("ReferenceType"), ref_type.clone());
    // KeyedRef (importlib._bootstrap's ref subclass with a .key): v0 alias
    // of the ref type — construction produces the same token.
    let _ = ns.set(s("KeyedRef"), ref_type);
    let _ = ns.set(s("proxy"), marker("weakref.proxy"));
    for n in ["getweakrefcount", "getweakrefs", "finalize"] {
        let _ = ns.set(s(n), marker(&format!("weakref.{n}")));
    }
    let proxy_type = Value::Obj(PyObject::new(ap_obj::ObjKind::Type, "ProxyType"));
    let cproxy_type = Value::Obj(PyObject::new(ap_obj::ObjKind::Type, "CallableProxyType"));
    let _ = ns.set(s("ProxyType"), proxy_type.clone());
    let _ = ns.set(s("CallableProxyType"), cproxy_type.clone());
    let _ = ns.set(
        s("ProxyTypes"),
        Value::Tuple(Gc::new(Tuple(vec![proxy_type, cproxy_type]))),
    );
    // The mapping/set surfaces (02.04 completion): Opaque-backed native
    // classes over the weakref token machinery (keys/values weakly held).
    for cls in [WKD, WVD, WS, "WeakMethod"] {
        let _ = ns.set(
            s(cls),
            Value::Obj(PyObject::new(ap_obj::ObjKind::Type, cls)),
        );
    }
    let _ = m.module_ns.borrow_mut().replace(Rc::new((*ns).clone()));
    Value::Obj(m)
}

/// Dispatch a `weakref.<fn>` call.
pub fn call_weakref(name: &str, args: &[Value], kwargs: &[(String, Value)]) -> Option<BResult> {
    let r: BResult = (|| {
        // Container/WeakMethod surfaces: the module-attr Types call as
        // constructors; bound methods arrive as "WeakKeyDictionary.get"-style
        // fnames with self at args[0].
        if let Some(cls) = [WKD, WVD, WS, "WeakMethod"]
            .iter()
            .find(|c2| name == **c2 || name.starts_with(&format!("{c2}.")))
        {
            let cls = *cls;
            if name == cls {
                if cls == "WeakMethod" {
                    return weakmethod_ctor(args);
                }
                return container_ctor(cls, args);
            }
            let op = &name[cls.len() + 1..];
            return weakref_surface(cls, op, args);
        }
        match name {
            "ref" => weakref_ref(args, kwargs),
            "proxy" => weakref_proxy(args, kwargs),
            "getweakrefcount" => {
                let t = args
                    .first()
                    .ok_or_else(|| raise("TypeError", "getweakrefcount() needs a target"))?;
                Ok(i(ap_gc::weakref_count_for(target_addr_of(t)) as i64))
            }
            "getweakrefs" => {
                let t = args
                    .first()
                    .ok_or_else(|| raise("TypeError", "getweakrefs() needs a target"))?;
                Ok(Value::List(PyList::from_vec(ap_gc::weakref_tokens_for(
                    target_addr_of(t),
                ))))
            }
            "finalize" => call_weakref_finalize(args, kwargs).unwrap_or_else(|| Ok(Value::None)),
            // finalize-token BoundMethod markers (native_attr "_weakref_token"):
            // the token rides args[0].
            "detach" => {
                let tok = args
                    .first()
                    .cloned()
                    .ok_or_else(|| raise("TypeError", "detach without token"))?;
                match &tok {
                    Value::Obj(o) => match crate::gcsupport::finalize_detach(o) {
                        Some(v) => Ok(v),
                        None => Ok(Value::None),
                    },
                    _ => Ok(Value::None),
                }
            }
            "__call__" => {
                let tok = args
                    .first()
                    .cloned()
                    .ok_or_else(|| raise("TypeError", "call without token"))?;
                match &tok {
                    Value::Obj(o) => Ok(crate::gcsupport::finalize_call(o)),
                    _ => Ok(Value::None),
                }
            }
            _ => Err(raise(
                "AttributeError",
                format!("module 'weakref' has no attribute '{name}'"),
            )),
        }
    })();
    Some(r)
}

/// weakref.finalize surface (gc015). Lives on the finalize class VALUE that
/// the vendored weakref.py builds; this dispatch handles the native fallback
/// path where weakref.finalize is called directly.
pub fn call_weakref_finalize(args: &[Value], kwargs: &[(String, Value)]) -> Option<BResult> {
    let obj = args.first()?.clone();
    let func = args.get(1)?.clone();
    if !weakrefable(&obj) {
        return Some(Err(raise(
            "TypeError",
            format!(
                "cannot create weak reference to '{}' object",
                obj.type_name()
            ),
        )));
    }
    let target_addr = target_addr_of(&obj);
    let token = PyObject::new(ap_obj::ObjKind::Opaque, "_finalize_token");
    {
        let mut tattrs = token.attrs.borrow_mut();
        let _ = tattrs.set(s("<target_addr>"), i(target_addr as i64));
        let _ = tattrs.set(s("alive"), Value::Bool(true));
    }
    let rest: Vec<Value> = args[2..].to_vec();
    FINALIZERS.with(|f| {
        f.borrow_mut().push(FinalizerEntry {
            token: token.clone(),
            target_addr,
            func,
            args: rest,
            kwargs: kwargs.to_vec(),
            atexit: true,
            index: next_finalizer_index(),
            detached: false,
        });
    });
    // The finalize token watches the target via a weakref entry whose
    // callback is a native marker that invokes the finalize.
    ap_gc::weakref_add(ap_gc::WeakEntry {
        target: target_addr,
        token: Value::Obj(token.clone()),
        callback: Some(marker("__ap_finalize_dispatch__")),
        dead: false,
        fired: false,
    });
    Some(Ok(Value::Obj(token)))
}

/// Call a finalize token: run func(*args, **kwargs) once; None afterwards.
pub fn finalize_call(token: &Gc<PyObject>) -> Value {
    let entry = FINALIZERS.with(|f| {
        let mut f = f.borrow_mut();
        let pos = f.iter().position(|e| Rc::ptr_eq(&e.token, token))?;
        let e = &mut f[pos];
        if e.detached || !alive_bookkeeping(e) {
            return None;
        }
        e.detached = true;
        Some((e.func.clone(), e.args.clone(), e.kwargs.clone()))
    });
    match entry {
        Some((func, args, kwargs)) => {
            if let Ok(r) = invoke_callback(&func, &args) {
                return r;
            }
            let _ = kwargs;
            Value::None
        }
        None => Value::None,
    }
}

fn alive_bookkeeping(_e: &FinalizerEntry) -> bool {
    true
}

/// detach(): returns (obj, func, args, kwargs) and cancels.
pub fn finalize_detach(token: &Gc<PyObject>) -> Option<Value> {
    FINALIZERS.with(|f| {
        let mut f = f.borrow_mut();
        let pos = f.iter().position(|e| Rc::ptr_eq(&e.token, token))?;
        let e = &mut f[pos];
        if e.detached {
            return None;
        }
        e.detached = true;
        // Reconstruct the object if still alive.
        let obj = if ap_gc::weakref_alive(e.target_addr) {
            ap_gc::get_objects()
                .into_iter()
                .find(|v| target_addr_of(v) == e.target_addr)
                .unwrap_or(Value::None)
        } else {
            Value::None
        };
        Some(Value::Tuple(Gc::new(Tuple(vec![
            obj,
            e.func.clone(),
            Value::Tuple(Gc::new(Tuple(e.args.clone()))),
            dict_of(e.kwargs.clone()),
        ]))))
    })
}

fn dict_of(kwargs: Vec<(String, Value)>) -> Value {
    let d = PyDict::new();
    for (k, v) in kwargs {
        let _ = d.set(s(k), v);
    }
    Value::Dict(d)
}

/// alive property read on a finalize token.
pub fn finalize_alive(token: &Gc<PyObject>) -> bool {
    FINALIZERS.with(|f| {
        f.borrow()
            .iter()
            .any(|e| Rc::ptr_eq(&e.token, token) && !e.detached)
    })
}

/// Atexit drain for pending finalizers (C-22's atexit path; ap-run calls
/// after the atexit drain, before process exit).
pub fn finalize_atexit_drain() {
    let pending = FINALIZERS.with(|f| {
        let mut f = f.borrow_mut();
        let mut out: Vec<(Value, Vec<Value>, Vec<(String, Value)>)> = Vec::new();
        f.sort_by_key(|e| e.index);
        for e in f.iter_mut() {
            if e.atexit && !e.detached {
                e.detached = true;
                out.push((e.func.clone(), e.args.clone(), e.kwargs.clone()));
            }
        }
        out
    });
    for (func, args, kwargs) in pending {
        let _ = invoke_callback(&func, &args);
        let _ = kwargs;
    }
}

// ---------------------------------------------------------------------------
// Weak containers (02.04 completion): WeakKeyDictionary / WeakValueDictionary
// / WeakSet / WeakMethod — Opaque-backed native classes over the token
// machinery.
//
// DESIGN: entries hold ref TOKENS (target address only, no strong ref), so a
// key/value dies exactly when the collector sweeps it. Dead entries are
// pruned AT ACCESS (len/iter/get/contains); CPython prunes at death time via
// the _remove callback. Observable difference is timing-only: after a
// gc.collect() both models agree. Documented divergence (C-22 note).
// ---------------------------------------------------------------------------

pub const WKD: &str = "WeakKeyDictionary";
pub const WVD: &str = "WeakValueDictionary";
pub const WS: &str = "WeakSet";

/// Is this Opaque name one of our weak containers?
pub fn is_weak_container(name: &str) -> bool {
    matches!(name, WKD | WVD | WS)
}

fn container_new(cls: &str) -> Value {
    let o = PyObject::new(ap_obj::ObjKind::Opaque, cls);
    {
        let mut attrs = o.attrs.borrow_mut();
        if cls == WS {
            let _ = attrs.set(s("<data>"), Value::Set(ap_obj::dict::PySet::new()));
        } else {
            let _ = attrs.set(s("<data>"), Value::Dict(PyDict::new()));
        }
    }
    // CPython repr form (WeakKeyDictionary.__repr__): "<%s at %#x>".
    let addr = Rc::as_ptr(&o) as *const u8 as usize;
    *o.repr_text.borrow_mut() = Some(format!("<{cls} at 0x{addr:x}>"));
    Value::Obj(o)
}

/// The backing dict of a weak container instance.
fn container_data(v: &Value) -> Option<Gc<PyDict>> {
    match v {
        Value::Obj(o) => o.attrs.borrow().get(&s("<data>")).and_then(|d| match d {
            Value::Dict(d) => Some(d.clone()),
            _ => None,
        }),
        _ => None,
    }
}

/// WeakSet storage: the `<data>` PySet of tokens.
fn weakset_data(v: &Value) -> Option<Gc<ap_obj::dict::PySet>> {
    match v {
        Value::Obj(o) => o.attrs.borrow().get(&s("<data>")).and_then(|d| match d {
            Value::Set(set) => Some(set.clone()),
            _ => None,
        }),
        _ => None,
    }
}

/// A ref token for `target` (no callback — pruning is at access).
fn make_token(target: &Value) -> BResult {
    // Interned Type objects (list, dict, ...) are NOT GC-tracked (they live
    // in interp.type_objects), so a weak token to one reports dead at the
    // first prune and ABC registries (WeakSet of classes) silently empty —
    // s016's MutableSequence.register(list) vanished. These objects are
    // immortal for the run: pin a strong clone and build the token WITHOUT
    // an ap-gc weakref entry (no callbacks to mis-fire, no dead-prune).
    if let Value::Obj(o) = target {
        if o.kind == ap_obj::ObjKind::Type {
            INTERNED_TYPE_PINS.with(|p| p.borrow_mut().push(target.clone()));
            let addr = target_addr_of(target);
            let token = PyObject::new(ap_obj::ObjKind::Opaque, "_weakref_token");
            let _ = token
                .attrs
                .borrow_mut()
                .set(s("<target_addr>"), i(addr as i64));
            return Ok(Value::Obj(token));
        }
    }
    weakref_ref(&[target.clone()], &[])
}

thread_local! {
    /// Strong pins for weakref targets that are not GC-tracked (interned
    /// Type objects). They live for the whole run; the pin keeps their
    /// address stable so token_alive / target_value_by_addr agree.
    static INTERNED_TYPE_PINS: std::cell::RefCell<Vec<Value>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

fn token_target(token: &Value) -> Option<usize> {
    match token {
        Value::Obj(o) => {
            let a = o.attrs.borrow().get(&s("<target_addr>")).and_then(|v| {
                v.as_int()
                    .and_then(|iv| iv.to_i64())
                    .map(|n| n.max(0) as usize)
            });
            a
        }
        _ => None,
    }
}

fn token_alive(token: &Value) -> bool {
    token_target(token)
        .map(|a| {
            ap_gc::weakref_alive(a)
                || INTERNED_TYPE_PINS.with(|p| p.borrow().iter().any(|v| target_addr_of(v) == a))
        })
        .unwrap_or(false)
}

/// The live target object behind a token Value (Some for Obj tokens).
fn deref_token(tok: &Value) -> Option<Value> {
    match tok {
        Value::Obj(t) => match crate::gcsupport::weakref_call(t) {
            Value::None => None,
            v => Some(v),
        },
        _ => None,
    }
}

fn key_error(key: &Value) -> Flow {
    Flow::Raise(crate::exc_hier::ExceptionValue::new(
        "KeyError",
        vec![key.clone()],
    ))
}

/// Drop entries whose key (WKD) or value-token (WVD) is dead.
fn prune_dict(d: &Gc<PyDict>, which: KeyPos) {
    let dead: Vec<Value> = d
        .items()
        .iter()
        .filter(|(k, v)| {
            let tok = match which {
                KeyPos::Key => k,
                KeyPos::Value => v,
            };
            token_target(tok).is_some() && !token_alive(tok)
        })
        .map(|(k, _)| k.clone())
        .collect();
    for k in dead {
        d.pop(&k);
    }
}

#[derive(Clone, Copy, PartialEq)]
enum KeyPos {
    Key,
    Value,
}

fn container_ctor(cls: &str, args: &[Value]) -> BResult {
    let c = container_new(cls);
    // WeakKeyDictionary(dict=None)/WeakValueDictionary(dict=None) seed form.
    if let Some(init) = args.first() {
        if !matches!(init, Value::None) {
            let it = crate::builtins::iterable_of(init)?;
            let items = crate::iter::collect(&it);
            let d = container_data(&c).expect("container data");
            for item in items {
                match &item {
                    Value::Tuple(t) if t.0.len() == 2 => {
                        let (k, v) = (t.0[0].clone(), t.0[1].clone());
                        if cls == WVD {
                            let tok = make_token(&v)?;
                            let _ = d.set(k, tok);
                        } else {
                            let tok = make_token(&k)?;
                            let _ = d.set(tok, v);
                        }
                    }
                    _ => {
                        return Err(raise(
                            "TypeError",
                            format!(
                            "cannot convert dictionary update sequence element to a {cls} entry"
                        ),
                        ))
                    }
                }
            }
        }
    }
    Ok(c)
}

/// The weak-container method surface: `self` rides args[0], `op` is the
/// method name (bound via native_attr BoundMethod markers, subscript and
/// contains paths call it directly).
pub fn weakref_surface(cls: &str, op: &str, args: &[Value]) -> BResult {
    let self_v = args
        .first()
        .cloned()
        .ok_or_else(|| raise("SystemError", "weakref surface without self"))?;
    let arg = |n: usize| args.get(n).cloned();

    // --- WeakSet: storage is a PySet of tokens in <data> -------------------
    if cls == WS {
        return weakset_surface(&self_v, op, args);
    }

    let d = container_data(&self_v)
        .ok_or_else(|| raise("SystemError", "weakref container without data"))?;

    match (cls, op) {
        // ---------------- WeakKeyDictionary ----------------
        (WKD, "__getitem__") => {
            prune_dict(&d, KeyPos::Key);
            let tok = make_token(&arg(1).expect("key"))?;
            match d.get(&tok) {
                Some(v) => Ok(v),
                None => Err(key_error(&arg(1).unwrap())),
            }
        }
        (WKD, "__setitem__") => {
            let k = arg(1).expect("key");
            let v = arg(2).unwrap_or(Value::None);
            let tok = make_token(&k)?;
            let _ = d.set(tok, v);
            Ok(Value::None)
        }
        (WKD, "__delitem__") => {
            prune_dict(&d, KeyPos::Key);
            let k = arg(1).expect("key");
            let tok = make_token(&k)?;
            match d.pop(&tok) {
                Some(_) => Ok(Value::None),
                None => Err(key_error(&k)),
            }
        }
        (WKD, "__contains__") => {
            prune_dict(&d, KeyPos::Key);
            match arg(1) {
                Some(k) => match make_token(&k) {
                    Ok(tok) => Ok(Value::Bool(d.contains(&tok))),
                    Err(_) => Ok(Value::Bool(false)), // unweakrefable -> not in
                },
                None => Ok(Value::Bool(false)),
            }
        }
        (WKD, "__len__") => {
            prune_dict(&d, KeyPos::Key);
            Ok(i(d.len() as i64))
        }
        (WKD, "__bool__") => {
            prune_dict(&d, KeyPos::Key);
            Ok(Value::Bool(d.len() != 0))
        }
        (WKD, "__iter__") | (WKD, "keys") => {
            prune_dict(&d, KeyPos::Key);
            let keys: Vec<Value> = d
                .items()
                .iter()
                .filter_map(|(k, _)| match k {
                    Value::Obj(tok) => deref_token(&Value::Obj(tok.clone())),
                    _ => None,
                })
                .collect();
            Ok(Value::List(PyList::from_vec(keys)))
        }
        (WKD, "keyrefs") => {
            prune_dict(&d, KeyPos::Key);
            Ok(Value::List(PyList::from_vec(d.keys())))
        }
        (WKD, "values") => {
            prune_dict(&d, KeyPos::Key);
            let vals: Vec<Value> = d
                .items()
                .iter()
                .filter(|(k, _)| token_alive(k))
                .map(|(_, v)| v.clone())
                .collect();
            Ok(Value::List(PyList::from_vec(vals)))
        }
        (WKD, "items") => {
            prune_dict(&d, KeyPos::Key);
            let items: Vec<Value> = d
                .items()
                .iter()
                .filter_map(|(k, v)| match k {
                    Value::Obj(tok) => deref_token(&Value::Obj(tok.clone()))
                        .map(|o| Value::Tuple(Gc::new(Tuple(vec![o, v.clone()])))),
                    _ => None,
                })
                .collect();
            Ok(Value::List(PyList::from_vec(items)))
        }
        (WKD, "get") => {
            prune_dict(&d, KeyPos::Key);
            let k = arg(1).expect("key");
            let default = arg(2).unwrap_or(Value::None);
            match make_token(&k) {
                Ok(tok) => Ok(d.get(&tok).unwrap_or(default)),
                Err(_) => Ok(default),
            }
        }
        (WKD, "setdefault") => {
            prune_dict(&d, KeyPos::Key);
            let k = arg(1).expect("key");
            let default = arg(2).unwrap_or(Value::None);
            let tok = make_token(&k)?;
            if let Some(v) = d.get(&tok) {
                return Ok(v);
            }
            let _ = d.set(tok, default.clone());
            Ok(default)
        }
        (WKD, "pop") => {
            prune_dict(&d, KeyPos::Key);
            let k = arg(1).expect("key");
            let tok = make_token(&k)?;
            match d.pop(&tok) {
                Some(v) => Ok(v),
                None => match arg(2) {
                    Some(default) => Ok(default),
                    None => Err(key_error(&k)),
                },
            }
        }
        (WKD, "update") => {
            let src = arg(1).ok_or_else(|| raise("TypeError", "update needs an argument"))?;
            let it = crate::builtins::iterable_of(&src)?;
            for item in crate::iter::collect(&it) {
                if let Value::Tuple(t) = item {
                    if t.0.len() == 2 {
                        let tok = make_token(&t.0[0])?;
                        let _ = d.set(tok, t.0[1].clone());
                    }
                }
            }
            Ok(Value::None)
        }
        (WKD, "clear") => {
            for (k, _) in d.items() {
                d.pop(&k);
            }
            Ok(Value::None)
        }
        (WKD, "copy") => {
            prune_dict(&d, KeyPos::Key);
            let c = container_new(WKD);
            let cd = container_data(&c).expect("data");
            for (k, v) in d.items() {
                let _ = cd.set(k.clone(), v.clone());
            }
            Ok(c)
        }
        // ---------------- WeakValueDictionary ----------------
        (WVD, "__setitem__") => {
            let k = arg(1).expect("key");
            let v = arg(2).unwrap_or(Value::None);
            let tok = make_token(&v)?;
            let _ = d.set(k, tok);
            Ok(Value::None)
        }
        (WVD, "__getitem__") => {
            let k = arg(1).expect("key");
            let Some(Value::Obj(tok)) = d.get(&k) else {
                return Err(key_error(&k));
            };
            match crate::gcsupport::weakref_call(&tok) {
                Value::None => Err(key_error(&k)),
                v => Ok(v),
            }
        }
        (WVD, "__delitem__") => {
            let k = arg(1).expect("key");
            let removed = match d.get(&k) {
                Some(tok) => token_alive(&tok) || true, // entry exists -> removable
                None => false,
            };
            if !removed {
                return Err(key_error(&k));
            }
            d.pop(&k);
            Ok(Value::None)
        }
        (WVD, "__contains__") => {
            let k = arg(1).unwrap_or(Value::None);
            Ok(Value::Bool(match d.get(&k) {
                Some(tok) => token_alive(&tok),
                None => false,
            }))
        }
        (WVD, "__len__") => {
            prune_dict(&d, KeyPos::Value);
            Ok(i(d.len() as i64))
        }
        (WVD, "__bool__") => {
            prune_dict(&d, KeyPos::Value);
            Ok(Value::Bool(d.len() != 0))
        }
        (WVD, "__iter__") | (WVD, "keys") => Ok(Value::List(PyList::from_vec(d.keys()))),
        (WVD, "values") => {
            prune_dict(&d, KeyPos::Value);
            let vals: Vec<Value> = d
                .items()
                .iter()
                .filter_map(|(_, v)| match v {
                    Value::Obj(tok) => deref_token(&Value::Obj(tok.clone())),
                    _ => None,
                })
                .collect();
            Ok(Value::List(PyList::from_vec(vals)))
        }
        (WVD, "items") => {
            prune_dict(&d, KeyPos::Value);
            let items: Vec<Value> = d
                .items()
                .iter()
                .filter_map(|(k, v)| match v {
                    Value::Obj(tok) => deref_token(&Value::Obj(tok.clone()))
                        .map(|o| Value::Tuple(Gc::new(Tuple(vec![k.clone(), o])))),
                    _ => None,
                })
                .collect();
            Ok(Value::List(PyList::from_vec(items)))
        }
        (WVD, "get") => {
            let k = arg(1).expect("key");
            let default = arg(2).unwrap_or(Value::None);
            match d.get(&k) {
                Some(Value::Obj(tok)) => match crate::gcsupport::weakref_call(&tok) {
                    Value::None => Ok(default),
                    v => Ok(v),
                },
                _ => Ok(default),
            }
        }
        (WVD, "setdefault") => {
            let k = arg(1).expect("key");
            let default = arg(2).unwrap_or(Value::None);
            if let Some(Value::Obj(tok)) = d.get(&k) {
                if let Value::None = crate::gcsupport::weakref_call(&tok) {
                    // dead entry: CPython keeps it; we treat as present
                    let _ = tok;
                }
                return Ok(crate::gcsupport::weakref_call(&tok));
            }
            let tok = make_token(&default)?;
            let _ = d.set(k, tok);
            Ok(default)
        }
        (WVD, "pop") => {
            let k = arg(1).expect("key");
            match d.pop(&k) {
                Some(Value::Obj(tok)) => Ok(crate::gcsupport::weakref_call(&tok)),
                Some(other) => Ok(other),
                None => match arg(2) {
                    Some(default) => Ok(default),
                    None => Err(key_error(&k)),
                },
            }
        }
        (WVD, "update") => {
            let src = arg(1).ok_or_else(|| raise("TypeError", "update needs an argument"))?;
            let it = crate::builtins::iterable_of(&src)?;
            for item in crate::iter::collect(&it) {
                if let Value::Tuple(t) = item {
                    if t.0.len() == 2 {
                        let tok = make_token(&t.0[1])?;
                        let _ = d.set(t.0[0].clone(), tok);
                    }
                }
            }
            Ok(Value::None)
        }
        (WVD, "clear") => {
            for (k, _) in d.items() {
                d.pop(&k);
            }
            Ok(Value::None)
        }
        (WVD, "copy") => {
            let c = container_new(WVD);
            let cd = container_data(&c).expect("data");
            for (k, v) in d.items() {
                let _ = cd.set(k.clone(), v.clone());
            }
            Ok(c)
        }
        (WVD, "prune") => {
            prune_dict(&d, KeyPos::Value);
            Ok(Value::None)
        }
        _ => Err(raise(
            "AttributeError",
            format!("'{cls}' object has no attribute '{op}'"),
        )),
    }
}

/// WeakSet surface: <data> is a real PySet of ref tokens.
fn weakset_surface(self_v: &Value, op: &str, args: &[Value]) -> BResult {
    let Some(set) = weakset_data(self_v) else {
        return Err(raise("SystemError", "WeakSet without data"));
    };
    let arg = |n: usize| args.get(n).cloned();
    // Prune dead tokens first (set ops hash+eq BY TARGET — see ap-obj).
    let dead: Vec<Value> = set
        .iter()
        .filter(|t| token_target(t).is_some() && !token_alive(t))
        .collect();
    for t in dead {
        set.pop_discard(&t);
    }
    match op {
        "add" => {
            let item = arg(1).ok_or_else(|| raise("TypeError", "add needs an item"))?;
            let tok = make_token(&item)?;
            set.add(tok).map_err(Flow::from_py)?;
            Ok(Value::None)
        }
        "__contains__" => {
            let item = arg(1).unwrap_or(Value::None);
            match make_token(&item) {
                Ok(tok) => Ok(Value::Bool(set.contains(&tok).unwrap_or(false))),
                Err(_) => Ok(Value::Bool(false)), // unweakrefable -> not in
            }
        }
        "__len__" => Ok(i(set.len() as i64)),
        "__bool__" => Ok(Value::Bool(set.len() != 0)),
        "__iter__" => {
            let items: Vec<Value> = set.iter().filter_map(|t| deref_token(&t)).collect();
            Ok(Value::List(PyList::from_vec(items)))
        }
        "discard" => {
            if let Some(item) = arg(1) {
                if let Ok(tok) = make_token(&item) {
                    set.pop_discard(&tok);
                }
            }
            Ok(Value::None)
        }
        "remove" => {
            let item = arg(1).ok_or_else(|| raise("TypeError", "remove needs an item"))?;
            let tok = make_token(&item)?;
            if !set.contains(&tok).unwrap_or(false) {
                return Err(key_error(&item));
            }
            set.pop_discard(&tok);
            Ok(Value::None)
        }
        "pop" => {
            let tok = set
                .iter()
                .next()
                .ok_or_else(|| raise("KeyError", "'pop from empty WeakSet'"))?;
            let out = deref_token(&tok).unwrap_or(tok.clone());
            set.pop_discard(&tok);
            Ok(out)
        }
        "clear" => {
            for e in set.iter() {
                set.pop_discard(&e);
            }
            Ok(Value::None)
        }
        "copy" => {
            let c = container_new(WS);
            if let Value::Obj(co) = &c {
                let _ = co
                    .attrs
                    .borrow_mut()
                    .set(s("<data>"), Value::Set(set.clone()));
            }
            Ok(c)
        }
        _ => Err(raise(
            "AttributeError",
            format!("'WeakSet' object has no attribute '{op}'"),
        )),
    }
}

/// WeakMethod(meth[, callback]): watches the BOUND METHOD's receiver. A call
/// rebuilds the bound method while the receiver lives, None after (C-22).
pub fn weakmethod_ctor(args: &[Value]) -> BResult {
    let meth = args
        .first()
        .cloned()
        .ok_or_else(|| raise("TypeError", "WeakMethod needs a method"))?;
    let (recv, func, name) = match &meth {
        Value::Obj(o) if o.kind == ap_obj::ObjKind::BoundMethod => (
            o.self_value.borrow().clone().unwrap_or(Value::None),
            o.func.borrow().clone().unwrap_or(Value::None),
            o.name.clone(),
        ),
        _ => {
            return Err(raise(
                "TypeError",
                format!(
                    "WeakMethod: expected a bound method, got {}",
                    meth.type_name()
                ),
            ))
        }
    };
    if matches!(recv, Value::None) {
        return Err(raise("TypeError", "WeakMethod: unbound method"));
    }
    let target_addr = target_addr_of(&recv);
    let token = PyObject::new(ap_obj::ObjKind::Opaque, "_weakmethod_token");
    {
        let mut tattrs = token.attrs.borrow_mut();
        let _ = tattrs.set(s("<target_addr>"), i(target_addr as i64));
        let _ = tattrs.set(s("<func>"), func);
        let _ = tattrs.set(s("<name>"), s(name));
    }
    let callback = args.get(1).cloned();
    ap_gc::weakref_add(ap_gc::WeakEntry {
        target: target_addr,
        token: Value::Obj(token.clone()),
        callback,
        dead: false,
        fired: false,
    });
    Ok(Value::Obj(token))
}

/// WeakMethod token call: rebuild the bound method (receiver alive) or None.
pub fn weakmethod_call(token: &Gc<PyObject>) -> Value {
    let addr = token
        .attrs
        .borrow()
        .get(&s("<target_addr>"))
        .and_then(|v| v.as_int())
        .and_then(|iv| iv.to_i64())
        .map(|n| n.max(0) as usize)
        .unwrap_or(0);
    if addr == 0 || !ap_gc::weakref_alive(addr) {
        return Value::None;
    }
    let Some(recv) = target_value_by_addr(addr) else {
        return Value::None;
    };
    let func = token
        .attrs
        .borrow()
        .get(&s("<func>"))
        .unwrap_or(Value::None);
    let name = match token.attrs.borrow().get(&s("<name>")) {
        Some(Value::Str(x)) => x.as_str().to_string(),
        _ => String::new(),
    };
    let m = PyObject::with_self(ap_obj::ObjKind::BoundMethod, name, recv);
    *m.func.borrow_mut() = Some(func);
    Value::Obj(m)
}

// ---------------------------------------------------------------------------
// Bridge install (run_module entry)
// ---------------------------------------------------------------------------

/// Wire the collector to this interpreter: roots + the weakbridge fns.
pub fn install(interp: &Rc<Interpreter>) {
    ap_obj::gc_gate::set_hook(ap_gc::gc_register);
    ap_gc::set_bridge(
        weakbridge_sys_modules,
        weakbridge_frame_values,
        bridge_del,
        weakbridge_callback,
        fire_dead_weakrefs_pub,
    );
    ap_gc::clear_roots();
    ap_gc::add_root_dict(&interp.globals);
    ap_gc::add_root_dict(&interp.builtins);
    // Threshold from env (task: GC_THRESHOLD tunable).
    if let Ok(v) = std::env::var("APX_GC_THRESHOLD") {
        if let Ok(n) = v.parse::<usize>() {
            ap_gc::set_threshold(n, 10, 10);
        }
    }
}

fn fire_dead_weakrefs_pub() {
    fire_dead_weakrefs();
}

fn weakbridge_sys_modules() -> Vec<Value> {
    crate::importer::sys_modules_values_pub()
}

fn weakbridge_frame_values() -> Vec<Value> {
    // Live frame locals: the interpreter's active frames (module globals
    // are roots already; function locals ride the native stack — the
    // subtract-refs invariant covers them: a local holding the only strong
    // ref keeps strong_count > internal_edges).
    Vec::new()
}

fn weakbridge_callback(cb: &Value, args: &[Value]) -> Result<Value, String> {
    match invoke_callback(cb, args) {
        Ok(v) => Ok(v),
        Err(_) => Ok(Value::None),
    }
}

// ---------------------------------------------------------------------------
// 02.07 threading: worker-thread GC view
// ---------------------------------------------------------------------------

// A worker allocates values that must join the MAIN thread's gc registry.
// Value (Rc-based) is !Send by construction — but under GIL v1 all threads
// touch the graph only while holding the lock, so the transfer is
// GIL-serialized by protocol. The compiler cannot see the lock protocol;
// this crate declares the GIL-protocol-validated Send wrapper (a documented,
// audited unsafety, same class as CPython's own GIL-protected globals).
pub struct GilSendPub<T>(pub T);
unsafe impl<T> Send for GilSendPub<T> {}
unsafe impl<T> Sync for GilSendPub<T> {}
impl<T> Clone for GilSendPub<T>
where
    T: Clone,
{
    fn clone(&self) -> Self {
        GilSendPub(self.0.clone())
    }
}

static WORKER_REG_IN: std::sync::LazyLock<
    GilSendPub<std::sync::Mutex<Option<std::sync::mpsc::Sender<GilSendPub<Value>>>>>,
> = std::sync::LazyLock::new(|| GilSendPub(std::sync::Mutex::new(None)));
static WORKER_REG_OUT: std::sync::LazyLock<
    GilSendPub<std::sync::Mutex<Option<std::sync::mpsc::Receiver<GilSendPub<Value>>>>>,
> = std::sync::LazyLock::new(|| GilSendPub(std::sync::Mutex::new(None)));

fn gc_register_forward_worker(v: &Value) {
    let guard = WORKER_REG_IN.0.lock().unwrap();
    if let Some(tx) = guard.as_ref() {
        let _ = tx.send(GilSendPub(v.clone()));
        return;
    }
    drop(guard);
    ap_gc::gc_register(v);
}

/// Drain worker registrations into the MAIN thread's registry (main-thread
/// turns: every threading dispatch + collect).
pub fn drain_worker_registrations() -> usize {
    let rx_slot = WORKER_REG_OUT.0.lock().unwrap();
    let Some(rx) = rx_slot.as_ref() else {
        return 0;
    };
    let mut n = 0;
    while let Ok(GilSendPub(v)) = rx.try_recv() {
        ap_gc::gc_register(&v);
        n += 1;
    }
    n
}

/// Worker-thread GC wiring (called from threading::worker_bootstrap before
/// the GIL is ever taken on that thread).
pub fn install_worker_thread() {
    let (tx, rx) = std::sync::mpsc::channel::<GilSendPub<Value>>();
    *WORKER_REG_IN.0.lock().unwrap() = Some(tx);
    *WORKER_REG_OUT.0.lock().unwrap() = Some(rx);
    // The hook fn pointer is process-global; from here on, allocations made
    // BY ANY thread forward through the channel — the main thread drains.
    ap_obj::gc_gate::set_hook(gc_register_forward_worker);
}
