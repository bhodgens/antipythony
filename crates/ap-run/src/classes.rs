//! Class machinery (leaf 02.03): class statements, instantiation, the
//! attribute protocol (S-03-09..14), descriptors (S-03-20/21), super()
//! (S-03-17..19), isinstance/issubclass (S-03-31), and special-method
//! dispatch helpers used by interp.rs (S-03-22..26).
//!
//! Layout: a user class is `Value::Obj(PyObject{kind: UserClass})` whose
//! `class_ref` holds the strong `Rc<classes::TypeObject>` payload (name,
//! bases, C3 MRO, class dict, flags). Instances are `UserInstance` values
//! with a `Weak` `type_ref` back-pointer plus per-instance attributes in
//! `attrs` (the compact dict; absent when `__slots__` withholds `__dict__`).
//!
//! Exceptions: builtin exception CLASSES are UserClass values (flagged
//! `is_exception`), created at interpreter entry into `interp.exc_classes`.
//! `raise`/`except` keep the legacy `ExceptionValue` (class NAME + args)
//! plumbing and translate through `exc_class`/`class_name_of`.

use std::rc::Rc;

pub use ap_obj::classes::TypeRef;
use ap_obj::classes::{DescrKind, TypeFlags, TypeObject};
use ap_obj::dict::PyDict;
use ap_obj::int::Gc;
use ap_obj::value::{PyList, PyString, Tuple, Value};

use crate::exc_hier::ExceptionValue;
use crate::frame::{Frame, Interpreter};
use crate::interp::{call_value, eval_expr, exec_block, store_name, FnRegistry};
use crate::{builtins, Flow};

type EResult = Result<Value, Flow>;

/// 02.03b: dynamic class creation — the 3-arg `type(name, bases, dict, **kwds)`
/// call (enum._simple_enum drives it). Mirrors exec_class_def's creation tail:
/// metaclass resolution from the bases, a user metaclass __new__ (with the
/// kwargs), set_name hooks, and module-marker registration.
/// T1 (scrapy fix pass): build a REAL traceback chain from interp.tb_frames —
/// vendored traceback.print_exception walks tb.tb_frame.f_code
/// (co_filename/co_name), f_lineno, f_globals and follows tb_next; the old
/// single stub frame had no f_code and no chain. Shared by both exception
/// attr paths (classes.rs class-instance arm + interp.rs Exception arm).
pub fn traceback_value(interp: &crate::frame::Interpreter) -> Value {
    let entries = interp.tb_frames.borrow().clone();
    let script = interp.script_path.clone();
    let mut next: Value = Value::None;
    for (qualname, line, file) in entries.iter().rev() {
        // T2 gate finding 2: per-frame file (empty = script fallback).
        let file = if file.is_empty() { &script } else { file };
        next = tb_node(interp, file, qualname, *line, next);
    }
    if matches!(next, Value::None) {
        next = tb_node(interp, &script, "<module>", 1, Value::None);
    }
    next
}

fn tb_node(
    interp: &crate::frame::Interpreter,
    script: &str,
    qualname: &str,
    line: usize,
    next: Value,
) -> Value {
    let frame = ap_obj::PyObject::new(ap_obj::ObjKind::Namespace, "<frame>");
    let code = ap_obj::PyObject::new(ap_obj::ObjKind::Namespace, "<code>");
    {
        let mut cattrs = code.attrs.borrow_mut();
        let _ = cattrs.set(
            Value::Str(PyString::new("co_filename")),
            Value::Str(PyString::new(script.to_string())),
        );
        let _ = cattrs.set(
            Value::Str(PyString::new("co_name")),
            Value::Str(PyString::new(qualname.to_string())),
        );
        let _ = cattrs.set(
            Value::Str(PyString::new("co_qualname")),
            Value::Str(PyString::new(qualname.to_string())),
        );
        let _ = cattrs.set(
            Value::Str(PyString::new("co_firstlineno")),
            Value::Int(ap_obj::Int::from_i64(line.max(1) as i64)),
        );
        // co_positions(): a BuiltinFn marker call_builtin answers (see the
        // "<code>.co_positions" arm in builtins::call_builtin).
        let _ = cattrs.set(
            Value::Str(PyString::new("co_positions")),
            Value::Obj(ap_obj::PyObject::new(
                ap_obj::ObjKind::BuiltinFn,
                "<code>.co_positions",
            )),
        );
    }
    {
        let mut attrs = frame.attrs.borrow_mut();
        let _ = attrs.set(Value::Str(PyString::new("f_code")), Value::Obj(code));
        let _ = attrs.set(
            Value::Str(PyString::new("f_lineno")),
            Value::Int(ap_obj::Int::from_i64(line.max(1) as i64)),
        );
        let _ = attrs.set(
            Value::Str(PyString::new("f_globals")),
            Value::Dict(interp.globals.clone()),
        );
        let _ = attrs.set(
            Value::Str(PyString::new("f_locals")),
            Value::Dict(PyDict::new()),
        );
        let _ = attrs.set(Value::Str(PyString::new("f_back")), Value::None);
    }
    let tb = ap_obj::PyObject::new(ap_obj::ObjKind::Namespace, "<traceback>");
    {
        let mut attrs = tb.attrs.borrow_mut();
        let _ = attrs.set(Value::Str(PyString::new("tb_frame")), Value::Obj(frame));
        let _ = attrs.set(Value::Str(PyString::new("tb_next")), next);
        let _ = attrs.set(
            Value::Str(PyString::new("tb_lineno")),
            Value::Int(ap_obj::Int::from_i64(line.max(1) as i64)),
        );
        let _ = attrs.set(
            Value::Str(PyString::new("tb_lasti")),
            Value::Int(ap_obj::Int::from_i64(0)),
        );
    }
    Value::Obj(tb)
}

pub fn dynamic_type(
    interp: &Rc<Interpreter>,
    name: &str,
    bases_v: &[Value],
    ns: Gc<PyDict>,
    kwargs: &[(String, Value)],
) -> EResult {
    let mut base_types: Vec<Rc<TypeRef>> = Vec::new();
    for b in bases_v {
        match base_type_ref(interp, b)? {
            Some(t) => base_types.push(t),
            None => {
                return Err(raise(
                    "TypeError",
                    format!("bases must be types, not '{}'", b.type_name()),
                ))
            }
        }
    }
    // Metaclass resolution: most derived among the bases' metaclasses.
    let mut best: Option<Rc<TypeRef>> = None;
    for b in &base_types {
        let bm = b
            .metaclass
            .borrow()
            .clone()
            .or_else(|| Some(type_class(interp)));
        match (&best, bm) {
            (None, Some(m)) => best = Some(m),
            (Some(cur), Some(m)) => {
                if !Rc::ptr_eq(cur, &m) {
                    if m.is_subclass(cur) {
                        best = Some(m);
                    } else if !cur.is_subclass(&m) {
                        return Err(raise(
                            "TypeError",
                            "metaclass conflict: the metaclass of a derived class must be a (non-strict) subclass of the metaclasses of all its bases",
                        ));
                    }
                }
            }
            _ => {}
        }
    }
    let resolved = best.unwrap_or_else(|| type_class(interp));
    let flags = TypeFlags {
        slots: None,
        is_exception: base_types.iter().any(|t| t.flags.is_exception),
        builtin_exc: false,
    };
    let meta_new_f = resolved
        .lookup_with_owner("__new__")
        .map(|(f, _)| f)
        .filter(|f| is_user_function(f));
    let t = if let Some(f) = meta_new_f {
        // T1-d fix: the cached class VALUE (make_class_value OVERWRITES the
        // back-pointer, so every metaclass-__new__ creation broke the
        // `type(X) is M2` identity for the metaclass itself).
        let meta_v = class_value_for_public(&resolved);
        let start_owner = resolved.lookup_with_owner("__new__").map(|(_, o)| o);
        let bound = match start_owner.as_ref() {
            Some(ow) => bind_function_owned(&f, meta_v.clone(), Some(ow)),
            None => f.clone(),
        };
        let created = call_value(
            interp,
            &module_frame(interp),
            &bound,
            &[
                Value::Str(PyString::new(name)),
                Value::Tuple(Gc::new(Tuple(bases_v.to_vec()))),
                Value::Dict(ns.clone()),
            ],
            kwargs,
        )?;
        match created {
            Value::Obj(o) if o.kind == ap_obj::ObjKind::UserClass => {
                o.class_ref
                    .borrow()
                    .clone()
                    .ok_or_else(|| raise("SystemError", "created class without payload"))?
            }
            other => {
                return Err(raise(
                    "TypeError",
                    format!(
                        "{}.__new__ should create an instance, not '{}'",
                        resolved.name,
                        other.type_name()
                    ),
                ))
            }
        }
    } else {
        let t = TypeObject::create(
            name.to_string(),
            name.to_string(),
            base_types.clone(),
            ns,
            flags,
            None,
        )
        .map_err(|e| {
            if std::env::var("APX_MRODBG").is_ok() {
                let bn: Vec<String> = bases_v
                    .iter()
                    .map(|b| match b {
                        Value::Obj(bo) => format!("{}({:?})", bo.name, bo.kind),
                        other => other.type_name().to_string(),
                    })
                    .collect();
                eprintln!("[MRODBG] class={} bases={:?}", name, bn);
            }
            raise("TypeError", ap_obj::classes::mro_error_message(&e))
        })?;
        // T1-d: builtin-base marker (3-arg type("X", (tuple,), body) and
        // class-stmt subclasses of tuple/list/set/str) — instantiate
        // constructs the backing builtin from ctor args.
        for (bv, bt) in bases_v.iter().zip(base_types.iter()) {
            let bname = match bv {
                Value::Obj(bo) if bo.kind == ap_obj::ObjKind::Type => Some(bo.name.clone()),
                _ => None,
            };
            let marker = bname.or_else(|| {
                bt.dict
                    .get(&Value::Str(PyString::new("__ap_builtin_base__")))
                    .and_then(|v| match v {
                        Value::Str(s) => Some(s.as_str().to_string()),
                        _ => None,
                    })
            });
            if let Some(b) = marker {
                // W110 (T2): int subclasses (enum.IntEnum(int, ...)) need
                // the builtin-ctor fallback too — MyInt(1) must build from
                // the arg like tuple/list subclasses do.
                if matches!(
                    b.as_str(),
                    "tuple" | "list" | "dict" | "set" | "str" | "property" | "int"
                ) {
                    let _ = t.dict.set(
                        Value::Str(PyString::new("__ap_builtin_base__")),
                        Value::Str(PyString::new(b)),
                    );
                    break;
                }
            }
        }
        *t.metaclass.borrow_mut() = Some(resolved.clone());
        t
    };
    if t.metaclass.borrow().is_none() {
        *t.metaclass.borrow_mut() = Some(resolved.clone());
    }
    let class_v = ap_obj::classes::make_class_value(t.clone());
    // 02.08-c: exec_class_def seeds __module__ from the DEFINING frame's
    // globals __name__ BEFORE calling here; dynamic_type (enum._simple_enum,
    // namedtuple) has no frame context and keeps the builtins default.
    if t.dict
        .get(&Value::Str(PyString::new("__module__")))
        .is_none()
    {
        let _ = t.dict.set(
            Value::Str(PyString::new("__module__")),
            Value::Str(PyString::new(ap_obj::MODULE_NAME)),
        );
    }
    run_set_name_hooks(interp, &class_v, &t)?;
    Ok(class_v)
}

fn raise(class: &str, msg: impl Into<String>) -> Flow {
    Flow::Raise(ExceptionValue::msg(class, msg.into()))
}

// ===========================================================================
// Class payload plumbing
// ===========================================================================

/// The TypeObject payload behind a class Value (None for non-classes).
pub fn type_of(_interp: &Interpreter, v: &Value) -> Option<Rc<TypeRef>> {
    match v {
        Value::Obj(o) if o.kind == ap_obj::ObjKind::UserClass => o.class_ref.borrow().clone(),
        // T2 (wave-39): stream-derived instances are Opaque carrying the
        // user class ref; super()/isinstance must see the user class.
        Value::Obj(o)
            if o.kind == ap_obj::ObjKind::Opaque && o.class_ref.borrow().is_some() =>
        {
            o.class_ref.borrow().clone()
        }
        _ => None,
    }
}

/// The class of a VALUE: instances via their Weak back-pointer, exception
/// instances via the interned exception classes, builtin values via the
/// interned builtin pseudo-classes (only where a class exists).
pub fn class_of_value(interp: &Interpreter, v: &Value) -> Option<Rc<TypeRef>> {
    match v {
        Value::Obj(o) if o.kind == ap_obj::ObjKind::UserInstance => o
            .type_ref
            .borrow()
            .as_ref()
            .and_then(|w| w.upgrade())
            .map(|rc| rc as Rc<TypeRef>),
        Value::Obj(o) if o.kind == ap_obj::ObjKind::Exception => o
            .class_ref
            .borrow()
            .clone()
            .or_else(|| exc_class(interp, &o.name)),
        Value::Obj(o) if o.kind == ap_obj::ObjKind::UserClass => o.class_ref.borrow().clone(),
        // T2 (wave-39): stream-derived instances are Opaque carrying the
        // user class ref; super()/isinstance must see the user class.
        Value::Obj(o)
            if o.kind == ap_obj::ObjKind::Opaque && o.class_ref.borrow().is_some() =>
        {
            o.class_ref.borrow().clone()
        }
        _ => None,
    }
}

/// Interned builtin exception CLASS payload by name.
pub fn exc_class(interp: &Interpreter, name: &str) -> Option<Rc<TypeRef>> {
    interp
        .exc_classes
        .borrow()
        .get(name)
        .and_then(|v| type_of(interp, v))
}

/// The exception class VALUE for a name (for except-name reads, raise).
pub fn exc_class_value(interp: &Interpreter, name: &str) -> Option<Value> {
    interp.exc_classes.borrow().get(name).cloned()
}

/// Class Value for an exception INSTANCE's class (for type(e) etc.).
pub fn class_value_of_exc(interp: &Interpreter, name: &str) -> Option<Value> {
    exc_class_value(interp, name)
}

// ===========================================================================
// Builtin exception hierarchy as REAL classes (S-03-32)
// ===========================================================================

/// (name, direct parent) edges of the pinned BaseException tree (S-03-32's
/// observed sorted lists + the notable chains). Dependency order.
pub const EXC_TREE: &[(&str, Option<&str>)] = &[
    ("BaseException", None),
    ("Exception", Some("BaseException")),
    ("BaseExceptionGroup", Some("BaseException")),
    ("GeneratorExit", Some("BaseException")),
    ("KeyboardInterrupt", Some("BaseException")),
    ("SystemExit", Some("BaseException")),
    ("ArithmeticError", Some("Exception")),
    ("AssertionError", Some("Exception")),
    ("AttributeError", Some("Exception")),
    ("BufferError", Some("Exception")),
    ("ExceptionGroup", Some("BaseExceptionGroup")),
    ("ImportError", Some("Exception")),
    ("ModuleNotFoundError", Some("ImportError")),
    ("InterpreterError", Some("Exception")),
    ("LookupError", Some("Exception")),
    ("MemoryError", Some("Exception")),
    ("NameError", Some("Exception")),
    ("OSError", Some("Exception")),
    // 02.04 completion: the OSError family (unittest/gettext chain).
    ("FileNotFoundError", Some("OSError")),
    ("PermissionError", Some("OSError")),
    ("FileExistsError", Some("OSError")),
    ("NotADirectoryError", Some("OSError")),
    ("IsADirectoryError", Some("OSError")),
    ("InterruptedError", Some("OSError")),
    ("ProcessLookupError", Some("OSError")),
    ("TimeoutError", Some("OSError")),
    ("BlockingIOError", Some("OSError")),
    ("ChildProcessError", Some("OSError")),
    ("BrokenPipeError", Some("OSError")),
    ("ConnectionError", Some("OSError")),
    ("ConnectionAbortedError", Some("ConnectionError")),
    ("ConnectionRefusedError", Some("ConnectionError")),
    ("ConnectionResetError", Some("ConnectionError")),
    ("PatternError", Some("Exception")),
    ("EOFError", Some("Exception")),
    ("ReferenceError", Some("Exception")),
    ("RuntimeError", Some("Exception")),
    ("StopAsyncIteration", Some("Exception")),
    ("StopIteration", Some("Exception")),
    ("SyntaxError", Some("Exception")),
    // T1-d (inspect stdlib chain): the SyntaxError subtree.
    ("IndentationError", Some("SyntaxError")),
    ("TabError", Some("IndentationError")),
    ("SystemError", Some("Exception")),
    ("TypeError", Some("Exception")),
    ("ValueError", Some("Exception")),
    // 02.08-lite stdlib exception type (json).
    ("JSONDecodeError", Some("ValueError")),
    ("Warning", Some("Exception")),
    ("UnboundLocalError", Some("NameError")),
    ("IndexError", Some("LookupError")),
    ("KeyError", Some("LookupError")),
    ("NotImplementedError", Some("RuntimeError")),
    ("RecursionError", Some("RuntimeError")),
    ("UnicodeError", Some("ValueError")),
    ("UnicodeDecodeError", Some("UnicodeError")),
    ("UnicodeEncodeError", Some("UnicodeError")),
    ("ZeroDivisionError", Some("ArithmeticError")),
    ("OverflowError", Some("ArithmeticError")),
    ("FloatingPointError", Some("ArithmeticError")),
    // Warning subtree (S-03-32's observed sorted children).
    ("BytesWarning", Some("Warning")),
    ("DeprecationWarning", Some("Warning")),
    ("EncodingWarning", Some("Warning")),
    ("FutureWarning", Some("Warning")),
    ("ImportWarning", Some("Warning")),
    ("PendingDeprecationWarning", Some("Warning")),
    ("ResourceWarning", Some("Warning")),
    ("RuntimeWarning", Some("Warning")),
    ("SyntaxWarning", Some("Warning")),
    ("UnicodeWarning", Some("Warning")),
    ("UserWarning", Some("Warning")),
];

/// Create every builtin exception class into interp.exc_classes (called
/// from run_module entry, before any user code).
pub fn install_exception_classes(interp: &Rc<Interpreter>) {
    let mut table = interp.exc_classes.borrow_mut();
    for (name, parent) in EXC_TREE {
        let mut bases: Vec<Rc<TypeRef>> = match parent {
            Some(p) => vec![table
                .get(*p)
                .and_then(|v| type_of(interp, v))
                .expect("parent created first")],
            None => vec![object_class(interp)],
        };
        // CPython: class ExceptionGroup(BaseExceptionGroup, Exception).
        if *name == "ExceptionGroup" {
            if let Some(exc) = table.get("Exception").and_then(|v| type_of(interp, v)) {
                bases.push(exc);
            }
        }
        let flags = TypeFlags {
            slots: None,
            is_exception: true,
            builtin_exc: true,
        };
        let t = TypeObject::create(
            name.to_string(),
            name.to_string(),
            bases,
            PyDict::new(),
            flags,
            None,
        )
        .expect("builtin exc MRO is acyclic");
        // 02.08-full p2: stdlib-exception classes report their home module.
        let exc_module = match *name {
            "PatternError" => "re",
            "JSONDecodeError" => "json",
            _ => "builtins",
        };
        let _ = t.dict.set(
            Value::Str(PyString::new("__module__")),
            Value::Str(PyString::new(exc_module)),
        );
        let v = ap_obj::classes::make_class_value(t);
        table.insert(name.to_string(), v.clone());
        // Shadow the name-Type interns in builtins with the REAL classes
        // (name reads, `except E`, and base-class resolution all see them).
        let _ = interp
            .builtins
            .set(Value::Str(PyString::new(name.to_string())), v);
    }
    // `object`/`type` as REAL class values (identity: type(A) is type,
    // c001/c014). Overrides the name-Type interned entries.
    let obj_v = class_value_for(&object_class(interp));
    let _ = interp
        .builtins
        .set(Value::Str(PyString::new("object")), obj_v);
    let type_v = class_value_for(&type_class(interp));
    let _ = interp
        .builtins
        .set(Value::Str(PyString::new("type")), type_v);
    // 02.03 class-machinery builtins (dispatched through
    // interp::class_protocol_builtin). staticmethod/classmethod bind their
    // INTERNED TYPE values so `type(sm) is staticmethod` identity checks in
    // user code hold — vendored enum.py relies on that identity to avoid
    // double-wrapping staticmethods. super/property keep the BuiltinFn
    // marker (their call paths depend on the marker dispatch).
    for n in ["super", "property"] {
        let o = ap_obj::PyObject::new(ap_obj::ObjKind::BuiltinFn, n);
        let _ = interp
            .builtins
            .set(Value::Str(PyString::new(n)), Value::Obj(o));
    }
    for n in ["classmethod", "staticmethod"] {
        let tv = crate::builtins::interned_type(interp, n);
        let _ = interp.builtins.set(Value::Str(PyString::new(n)), tv);
    }
    // NotImplemented singleton (S-03-25 defer marker).
    let ni = ap_obj::PyObject::new(ap_obj::ObjKind::Opaque, "NotImplemented");
    let _ = interp
        .builtins
        .set(Value::Str(PyString::new("NotImplemented")), Value::Obj(ni));
    // 02.08-full part 1: IOError/EnvironmentError are ALIASES of OSError
    // (identity: IOError is OSError — s001 pin). Re-bind after the real
    // exception classes shadowed the interned name-Types above.
    drop(table);
    if let Some(os_v) = interp.builtins.get(&Value::Str(PyString::new("OSError"))) {
        for alias in ["IOError", "EnvironmentError"] {
            let _ = interp
                .builtins
                .set(Value::Str(PyString::new(alias)), os_v.clone());
        }
        // Registering IOError/EnvironmentError as REAL classes in the exc
        // registry broke unittest's ABC register path (issubclass false-
        // positives) — the builtins alias alone carries the identity pin.
    }
}

/// Direct subclasses recorded for `__subclasses__` (exception tree only in
/// v1; user subclasses append at creation).
pub fn subclasses_of(interp: &Interpreter, name: &str) -> Vec<Value> {
    let table = interp.exc_classes.borrow();
    let is_child = |(_, p): &(&str, Option<&str>)| {
        *p == Some(name)
        // CPython: class ExceptionGroup(BaseExceptionGroup, Exception) —
        // Exception.__subclasses__() includes it (e002/e014).
    };
    let _ = is_child;
    let mut out: Vec<Value> = EXC_TREE
        .iter()
        .filter(|(_, p)| *p == Some(name))
        // PatternError (re.error) IS visible in Exception.__subclasses__():
        // CPython loads `re` (via enum) at startup, so the class exists by
        // the time any user code runs; the e002/e014 pins (regenerated from
        // live 3.14.7) include it. An earlier filter excluded it on the
        // wrong premise that module-defined classes stay invisible.
        .filter_map(|(n, _)| table.get(*n).cloned())
        .collect();
    // ExceptionGroup ALSO derives Exception (dual bases).
    if name == "Exception" {
        if let Some(eg) = table.get("ExceptionGroup") {
            out.push(eg.clone());
        }
    }
    drop(table);
    // User subclasses registered at class-creation time:
    for (child, parent) in interp.user_subclasses.borrow().iter() {
        if parent == name {
            if let Some(v) = interp.user_class_values.borrow().get(child) {
                out.push(v.clone());
            }
        }
    }
    out.sort_by(|a, b| match (a, b) {
        (Value::Obj(x), Value::Obj(y)) => x.name.cmp(&y.name),
        _ => std::cmp::Ordering::Equal,
    });
    out.dedup_by(|a, b| match (a, b) {
        (Value::Obj(x), Value::Obj(y)) => std::rc::Rc::ptr_eq(x, y),
        _ => false,
    });
    out
}

// ===========================================================================
// Class creation from a class statement (S-03-01..04, S-03-27..29)
// ===========================================================================

/// Execute a class statement: bases, body in a fresh namespace, class
/// creation, hooks, name binding in the enclosing scope.

thread_local! {
    /// Wave-104: the class namespace of the class body CURRENTLY executing
    /// (set by exec_class_def, cleared after). 1-arg exec() routes here so
    /// exec'd statements see class-body names (CPython exec-in-class rule).
    static CURRENT_CLASS_NS: std::cell::RefCell<Option<Gc<PyDict>>> =
        std::cell::RefCell::new(None);
}

/// Public accessor for builtin_exec (builtins.rs).
pub fn current_class_ns() -> Option<Gc<PyDict>> {
    CURRENT_CLASS_NS.with(|c| c.borrow().clone())
}

pub fn exec_class_def(
    interp: &Rc<Interpreter>,
    frame: &Rc<Frame>,
    c: &ap_ir::item::ClassDef,
    reg: &Rc<FnRegistry>,
    bind_name: &str,
) -> EResult {
    // 1. Bases + explicit metaclass in the ENCLOSING frame (S-03-1).
    let mut bases_v: Vec<Value> = Vec::new();
    let mut class_kwargs: Vec<(String, Value)> = Vec::new();
    for b in &c.bases {
        let v = eval_expr(interp, frame, b, reg)?;
        // The lowering encodes non-metaclass class keywords as
        // `__ap_classkw__.<name>(value)` calls — they are NOT bases. The
        // call parked (name, value) on the marker; recover them and pass
        // them through to the metaclass __new__ (enum._simple_enum's
        // boundary=/_simple= ride this path).
        if let Value::Obj(o) = &v {
            if o.kind == ap_obj::ObjKind::BuiltinFn && o.name.starts_with("__ap_classkw__") {
                let parts = o.slice_parts.borrow();
                if parts.len() == 2 {
                    if let Value::Str(kn) = &parts[0] {
                        class_kwargs.push((kn.as_str().to_string(), parts[1].clone()));
                    }
                }
                continue;
            }
        }
        // T1-d (typing.NamedTuple / PEP 560): non-class bases with
        // __mro_entries__ replace themselves in the bases (NamedTuple ->
        // tuple). The replaced base rides __orig_bases__.
        let mut replaced = false;
        let is_user_cls = matches!(&v, Value::Obj(o) if o.kind == ap_obj::ObjKind::UserClass);
        // T2 (pytest `class CaptureResult(NamedTuple, Generic[AnyStr])`):
        // a _GenericAlias base (Generic[...]/ABC[...]) resolves to its ORIGIN
        // in the bases before the metaclass runs (CPython __mro_entries__ on
        // _GenericAlias) — NamedTupleMeta's `base is not Generic` identity
        // check depends on seeing the origin, not the alias.
        if let Value::Obj(ga) = &v {
            if ga.kind == ap_obj::ObjKind::Opaque && ga.name == "typing._GenericAlias" {
                if let Some(origin) = ga.slice_parts.borrow().first().cloned() {
                    bases_v.push(origin);
                    replaced = true;
                }
                continue;
            }
        }
        if !is_user_cls {
            if let Value::Obj(bo) = &v {
                // T1-d fix: instance-as-base PEP 560 (zope Interface) —
                // UserInstance bases can carry __mro_entries__ too.
                if bo.kind == ap_obj::ObjKind::Function
                    || bo.kind == ap_obj::ObjKind::BuiltinFn
                    || bo.kind == ap_obj::ObjKind::UserInstance
                {
                    let mut me_opt = bo
                        .attrs
                        .borrow()
                        .get(&Value::Str(PyString::new("__mro_entries__")))
                        .map(|x| x.clone());
                    if me_opt.is_none() {
                        if bo.kind == ap_obj::ObjKind::UserInstance {
                            // T1-d: class-level __mro_entries__ must bind to
                            // the INSTANCE (zope Interface shim).
                            if let Some(t) = bo.type_ref.borrow().as_ref().and_then(|w| w.upgrade())
                            {
                                if let Some(f) = t.lookup("__mro_entries__") {
                                    me_opt = Some(bind_function_owned(&f, v.clone(), Some(&t)));
                                }
                            }
                        }
                    }
                    if let Some(me) = me_opt {
                        // CPython passes the FULL original bases tuple
                        // (including this base) — _namedtuple_mro_entries
                        // asserts `NamedTuple in bases`.
                        let mut full = bases_v.clone();
                        full.push(v.clone());
                        let r = call_value(
                            interp,
                            &module_frame(interp),
                            &me,
                            &[Value::Tuple(Gc::new(Tuple(full)))],
                            &[],
                        )?;
                        let entries: Vec<Value> = match r {
                            Value::Tuple(t) => t.0.clone(),
                            Value::List(l) => l.items().to_vec(),
                            _ => vec![],
                        };
                        if !entries.is_empty() {
                            bases_v.extend(entries);
                            replaced = true;
                        }
                    }
                }
            }
        }
        if !replaced {
            bases_v.push(v);
        }
    }
    // Wave-109 (CPython 3.14 alias-base behavior): `class ZP(Generic[...],
    // Sub[...])` where Sub subclasses Generic — CPython's alias
    // __mro_entries__ drops the explicit Generic (the subclass already
    // pulls it into the MRO; keeping both contradicts base order in C3).
    // Mirror that: drop an explicit Generic base when a LATER base's class
    // chain already contains Generic.
    {
        let mut drop_idx: Vec<usize> = Vec::new();
        for (i, b) in bases_v.iter().enumerate() {
            let is_generic_base = matches!(b, Value::Obj(o)
                if o.name == "Generic"
                    && matches!(o.kind, ap_obj::ObjKind::Type | ap_obj::ObjKind::UserClass));
            if !is_generic_base {
                continue;
            }
            let later_has = bases_v[i + 1..].iter().any(|later| {
                matches!(base_type_ref(interp, later), Ok(Some(t))
                    if t.mro.borrow().iter().any(|m| m.name == "Generic" && !std::rc::Rc::ptr_eq(m, &t)))
            });
            if std::env::var("APX_MRODBG").is_ok() {
                let lnames: Vec<String> = bases_v[i + 1..]
                    .iter()
                    .map(|later| match base_type_ref(interp, later) {
                        Ok(Some(t)) => format!(
                            "{}[mro={:?}]",
                            t.name,
                            t.mro.borrow().iter().map(|m| m.name.clone()).collect::<Vec<_>>()
                        ),
                        _ => "<none>".to_string(),
                    })
                    .collect();
                eprintln!("[MRODBG-DROP] generic at {} later={:?}", i, lnames);
            }
            if later_has {
                drop_idx.push(i);
            }
        }
        for i in drop_idx.into_iter().rev() {
            bases_v.remove(i);
        }
    }
    let metaclass_v = match &c.metaclass {
        Some(e) => Some(eval_expr(interp, frame, e, reg)?),
        None => None,
    };

    // 2. Body in a fresh class namespace (S-03-3). The class frame is NOT
    //    an enclosing scope for methods (frontend skips Class scopes); the
    //    `__class__` cell (zero-arg super, S-03-17) is seeded after the
    //    class object exists.
    // 02.04a fix pass: consult the metaclass __prepare__ (PEP 3115) —
    // enum.EnumType returns an EnumDict subclass whose custom behavior
    // (_member_names) EnumType.__new__ requires. The effective metaclass is
    // resolved HERE from the base values (explicit metaclass wins; else the
    // first base whose type carries a metaclass pointer).
    let mut ns: Gc<PyDict> = PyDict::new();
    // 02.03b: the dict-subclass instance __prepare__ returned, if any
    // (EnumDict); rides the class frame for __setitem__ routing.
    let mut prepared_inst: Option<Value> = None;
    {
        let effective_meta: Option<Rc<TypeRef>> = match &metaclass_v {
            Some(m) => base_type_ref(interp, m)?,
            None => {
                let mut found: Option<Rc<TypeRef>> = None;
                for b in &bases_v {
                    if let Some(t) = base_type_ref(interp, b)? {
                        let bm = t.metaclass.borrow().clone();
                        if let Some(m) = bm {
                            match &found {
                                None => found = Some(m),
                                Some(cur) => {
                                    if !Rc::ptr_eq(cur, &m) && m.is_subclass(cur) {
                                        found = Some(m);
                                    }
                                }
                            }
                        }
                    }
                }
                found
            }
        };
        if let Some(meta) = &effective_meta {
            if let Some((prep, _)) = meta.lookup_with_owner("__prepare__") {
                // 02.03b: enum.EnumType.__prepare__ is a @classmethod — the
                // stored entry is a descriptor, not a bare function. Unwrap
                // classmethod to a cls-bound callable; plain functions pass.
                let prep_fn = match &prep {
                    Value::Obj(o)
                        if o.kind == ap_obj::ObjKind::Descriptor
                            && matches!(o.descr_kind.borrow().as_deref(), Some("classmethod")) =>
                    {
                        o.func.borrow().clone()
                    }
                    _ => {
                        if is_user_function(&prep) {
                            Some(prep.clone())
                        } else {
                            None
                        }
                    }
                };
                if let Some(prep_fn) = prep_fn {
                    let meta_cls_v = class_value_for(meta);
                    let prepared = call_value(
                        interp,
                        &module_frame(interp),
                        &prep_fn,
                        &[
                            meta_cls_v,
                            Value::Str(PyString::new(c.name.clone())),
                            Value::Tuple(Gc::new(Tuple(bases_v.iter().cloned().collect()))),
                        ],
                        &class_kwargs
                            .iter()
                            .map(|(k, v)| (k.clone(), v.clone()))
                            .collect::<Vec<_>>(),
                    )?;
                    match prepared {
                        Value::Dict(d) => ns = d.clone(),
                        // 02.03b: EnumDict (a dict subclass) instantiates as
                        // a dict-backed user instance. The BACKING dict is
                        // the class-body namespace (reads hit it directly);
                        // the instance rides the frame so STORES route
                        // through its __setitem__ (member tracking).
                        Value::Obj(io) if io.kind == ap_obj::ObjKind::UserInstance => {
                            if let Some(backing) =
                                crate::classes::dict_backing(&Value::Obj(io.clone()))
                            {
                                ns = backing;
                                prepared_inst = Some(Value::Obj(io.clone()));
                            } else {
                                for (k, v) in io.attrs.borrow().items() {
                                    let _ = ns.set(k, v);
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    let mut class_frame_val = Frame::new_class(frame, ns.clone());
    // 02.04 closure fix: the class-body frame must carry the compile-time
    // scope prefix ("C") so methods register as "C.<locals>.m" — exactly the
    // lowering's scope keys (open_scope pushes the class name, then
    // <locals> for functions). Without it, method frames get ".<locals>.m"
    // (leading dot, no class segment), scope_of misses, and nested defs in
    // methods lose their free-variable cells (argparse's
    // _metavar_formatter is the canonical victim).
    // NOTE(02.04 diagnosis, prior agent): a bare scope_path = c.name broke
    // enum.EnumType.__new__'s locals; that failure mode was the missing
    // parent-path JOIN (module-level classes have an empty parent, so the
    // bare name happened to match there) — the real delta is tested by the
    // metaclass/enum differential suites below.
    class_frame_val.scope_path = if frame.scope_path.is_empty() {
        c.name.clone()
    } else {
        format!("{}.{}", frame.scope_path, c.name)
    };
    class_frame_val.class_ns_inst = prepared_inst.clone();
    let class_frame = Rc::new(class_frame_val);
    // T1-d: CPython class namespaces carry __module__ (NamedTupleMeta reads
    // ns['__module__'] at class creation).
    if class_frame
        .class_ns
        .as_ref()
        .map(|ns| ns.get(&Value::Str(PyString::new("__module__"))).is_none())
        .unwrap_or(false)
    {
        let modname = frame
            .globals
            .get(&Value::Str(PyString::new("__name__")))
            .unwrap_or_else(|| Value::Str(PyString::new(ap_obj::MODULE_NAME)));
        if let Some(ns) = &class_frame.class_ns {
            let _ = ns.set(Value::Str(PyString::new("__module__")), modname);
        }
    }
    // Wave-104: publish the class ns for 1-arg exec() inside the class body
    // (twisted insults.py `exec(ch + " = _makeSimple(ch, fName)")`).
    CURRENT_CLASS_NS.with(|c| *c.borrow_mut() = Some(ns.clone()));
    let body_res = exec_block(interp, &class_frame, &c.body, reg);
    CURRENT_CLASS_NS.with(|c| *c.borrow_mut() = None);
    body_res?;

    // W110-5 fix (CPython private-name mangling): class-body names of the
    // form `__x` (2+ leading underscores, at most 1 trailing) are stored
    // MANGLED (`_<ClassName>__x`) by CPython. asyncio's Future relies on
    // it: `__asyncio_awaited_by = None` in the class body must be readable
    // as `fut._Future__asyncio_awaited_by` (future_add_to_awaited_by). We
    // keep the UNMANGLED key too (existing readers/behavior intact) and add
    // the mangled alias for each eligible name.
    {
        let cls_name = c.name.as_str();
        let mut aliases: Vec<(String, Value)> = Vec::new();
        for (k, v) in ns.items() {
            let name = match &k {
                Value::Str(s) => s.as_str().to_string(),
                _ => continue,
            };
            // Mangling rule: starts with "__", does NOT start with "___"?
            // CPython: strips at most ONE leading underscore for the count;
            // names with 2+ leading underscores and NOT ending with 2+
            // underscores are mangled. `__x__` (dunder) is exempt.
            if !name.starts_with("__") || name.starts_with("___") {
                continue;
            }
            if name.ends_with("__") {
                continue;
            }
            let mangled = format!("_{}{}", cls_name, name);
            aliases.push((mangled, v.clone()));
        }
        for (mangled, v) in aliases {
            if ns.get(&Value::Str(PyString::new(mangled.clone()))).is_none() {
                let _ = ns.set(Value::Str(PyString::new(mangled)), v);
            }
        }
    }

    // 3. Resolve base TypeRefs (S-03-1: `class A:` == `class A(object):`).
    let mut base_types: Vec<Rc<TypeRef>> = Vec::new();
    for b in &bases_v {
        match base_type_ref(interp, b)? {
            Some(t) => base_types.push(t),
            None => {
                return Err(raise(
                    "TypeError",
                    format!("bases must be types, not '{}'", b.type_name()),
                ))
            }
        }
    }
    // 02.04a fix pass: the ORIGINAL bases go to the metaclass __new__
    // (CPython semantics: class Enum(metaclass=EnumType) passes () — enum's
    // _get_mixins_ special-cases empty bases; a synthesized object base
    // breaks it).
    let original_bases_v: Vec<Value> = bases_v.clone();
    if base_types.is_empty() {
        base_types.push(object_class(interp));
    }

    // 4. Metaclass resolution (S-03-2).
    let _resolved_meta: Option<Rc<TypeRef>> = match &metaclass_v {
        Some(m) => {
            let t = base_type_ref(interp, m)?
                .ok_or_else(|| raise("TypeError", "metaclass must be a type"))?;
            Some(t)
        }
        None => {
            // Most derived among type(base) for every base (v0: bases'
            // metaclass pointers; plain classes carry None == type).
            let mut best: Option<Rc<TypeRef>> = None;
            for b in &base_types {
                let bm = b
                    .metaclass
                    .borrow()
                    .clone()
                    .or_else(|| Some(type_class(interp)));
                match (&best, bm) {
                    (None, Some(m)) => best = Some(m),
                    (Some(cur), Some(m)) => {
                        if !Rc::ptr_eq(cur, &m) {
                            if m.is_subclass(cur) {
                                best = Some(m);
                            } else if !cur.is_subclass(&m) {
                                return Err(raise(
                                    "TypeError",
                                    "metaclass conflict: the metaclass of a derived class must be a (non-strict) subclass of the metaclasses of all its bases",
                                ));
                            }
                        }
                    }
                    _ => {}
                }
            }
            best
        }
    };

    // 5. __slots__ validation (S-03-13/14). A bare string is ONE slot.
    let slots: Option<Vec<String>> = match ns.get(&Value::Str(PyString::new("__slots__"))) {
        Some(v) => {
            let names: Vec<String> = match &v {
                Value::Str(s) => vec![s.as_str().to_string()],
                Value::List(l) => string_items(l)?,
                Value::Tuple(t) => {
                    let mut out = Vec::new();
                    for e in &t.0 {
                        match e {
                            Value::Str(s) => out.push(s.as_str().to_string()),
                            other => {
                                return Err(raise(
                                    "TypeError",
                                    format!(
                                        "__slots__ items must be strings, not '{}'",
                                        other.type_name()
                                    ),
                                ))
                            }
                        }
                    }
                    out
                }
                Value::None => vec![],
                other => {
                    return Err(raise(
                        "TypeError",
                        format!(
                            "__slots__ must be a non-empty iterable, not '{}'",
                            other.type_name()
                        ),
                    ))
                }
            };
            for s in &names {
                if s == "__dict__" {
                    continue;
                }
                if ns.contains(&Value::Str(PyString::new(s.clone()))) {
                    return Err(raise(
                        "ValueError",
                        format!("'{s}' in __slots__ conflicts with class variable"),
                    ));
                }
            }
            Some(names)
        }
        None => None,
    };

    // 6. Create (C3 MRO inside; byte-exact errors, S-03-15/16).
    let qualname = qualname_for(frame, &c.name);
    let flags = TypeFlags {
        slots,
        is_exception: base_types.iter().any(|t| t.flags.is_exception),
        builtin_exc: false,
    };
    // 6b. 02.04a (interp bug #17): a USER metaclass __new__ drives creation
    // (ABCMeta.__new__ seeds _abc_cache/_abc_registry; EnumType.__new__
    // converts members). Call it as meta.__new__(meta, name, bases, ns);
    // its super().__new__ lands in builtin type.__new__ (bug #17 impl).
    let meta_new_f: Option<Value> = _resolved_meta
        .as_ref()
        .and_then(|meta| meta.lookup_with_owner("__new__"))
        .map(|(f, _)| f)
        .filter(|f| is_user_function(f));
    // 02.03b: the owner of the inherited __new__ (CPython zero-arg super()
    // starts at the DEFINING class — the __class__ cell — not at the class
    // the method was reached through). class M2(Meta) with class
    // C(metaclass=M2): resolved meta = M2 but __new__ is defined on Meta;
    // staging start=M2 made super().__new__ re-bind Meta.__new__ to M2
    // (5 positional args -> "takes 4 but 5 given"). start=Meta lands the
    // scan on type.__new__ (unbound builtin), per CPython.
    let meta_new_owner: Option<Rc<TypeRef>> = _resolved_meta
        .as_ref()
        .and_then(|meta| meta.lookup_with_owner("__new__"))
        .and_then(|(_, owner)| {
            if owner.name != _resolved_meta.as_ref().unwrap().name {
                Some(owner)
            } else {
                None
            }
        });
    CLASS_KWARGS.with(|c| *c.borrow_mut() = class_kwargs.clone());
    let t = if let Some(f) = meta_new_f {
        // 02.03b: when __prepare__ returned a dict-subclass instance
        // (EnumDict), the metaclass __new__ receives THE INSTANCE — its
        // attribute surface (_member_names etc.) is what EnumType.__new__
        // reads; the class payload comes from its backing dict.
        let ns_v = prepared_inst.clone().unwrap_or(Value::Dict(ns.clone()));
        let name_v = Value::Str(PyString::new(c.name.clone()));
        let bases_v = Value::Tuple(Gc::new(Tuple(original_bases_v.clone())));
        // T1-d fix: cached VALUE (make_class_value overwrote the
        // metaclass's cached class_value → identity broke).
        let meta_v = _resolved_meta
            .as_ref()
            .map(|m| class_value_for_public(m))
            .unwrap_or_else(|| Value::None);
        // Zero-arg super() inside the metaclass __new__ body needs the
        // method context. 02.03b: route through the BoundMethod channel
        // (bind_function_owned + the BoundMethod call arm stages the ctx
        // with the DEFINING class as start) — a manual pending_super_ctx
        // stage here was being consumed by an inner plain-fn call's
        // take(), leaving super() inside EnumType.__new__ contextless.
        let start_owner = meta_new_owner
            .clone()
            .unwrap_or_else(|| _resolved_meta.clone().unwrap());
        let bound_f = bind_function_owned(&f, meta_v.clone(), Some(&start_owner));
        let created = call_value(
            interp,
            &module_frame(interp),
            &bound_f,
            &[name_v, bases_v, ns_v],
            &class_kwargs
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect::<Vec<_>>(),
        );
        let created = created?;
        // 02.03b-fix: the metaclass __new__ CONSUMED the class kwargs
        // (boundary=... etc are its named params; CPython EnumType pops
        // them before super().__new__ — __init_subclass__ sees the EMPTY
        // remainder). type.__new__-level rejections (mk.py's Meta
        // forwarding **kw) surface through builtin_type_new's kwargs arm.
        CLASS_KWARGS.with(|c| c.borrow_mut().clear());
        match created {
            Value::Obj(ref o) if o.kind == ap_obj::ObjKind::UserClass => {
                let t = o
                    .class_ref
                    .borrow()
                    .clone()
                    .ok_or_else(|| raise("SystemError", "created class without payload"))?;
                t
            }
            other => {
                return Err(raise(
                    "TypeError",
                    format!(
                        "{}.__new__ should create an instance, not '{}'",
                        _resolved_meta
                            .as_ref()
                            .map(|m| m.name.clone())
                            .unwrap_or_default(),
                        other.type_name()
                    ),
                ))
            }
        }
    } else {
        // T2 (pluggy varnames heuristic): CPython's type.__new__ rewrites
        // the __qualname__ of every plain function in the class namespace to
        // "<Class>.<name>" — class-body ALIASES (`rep2 = rep`) share the
        // function object and must be requalified too. pluggy's bound-method
        // detection keys on the dotted qualname.
        {
            let mut keys: Vec<Value> = Vec::new();
            for (k, v) in ns.items() {
                if let Value::Obj(fo) = v {
                    if fo.kind == ap_obj::ObjKind::Function {
                        let dotted = fo
                            .attrs
                            .borrow()
                            .get(&Value::Str(PyString::new("__qualname__")))
                            .and_then(|q| match q {
                                Value::Str(s) => Some(s.as_str().contains('.')),
                                _ => Some(false),
                            })
                            .unwrap_or(false);
                        if !dotted {
                            keys.push(k.clone());
                        }
                    }
                }
            }
            for k in keys {
                if let Some(Value::Obj(fo)) = ns.get(&k) {
                    let fname = fo.name.clone();
                    let _ = fo.attrs.borrow_mut().set(
                        Value::Str(PyString::new("__qualname__")),
                        Value::Str(PyString::new(format!("{}.{}", c.name, fname))),
                    );
                }
            }
        }
        let t = TypeObject::create(
            c.name.clone(),
            qualname,
            base_types.clone(),
            ns,
            flags,
            None,
        )
        .map_err(|e| {
            if std::env::var("APX_MRODBG").is_ok() {
                let bn: Vec<String> = bases_v
                    .iter()
                    .map(|b| match b {
                        Value::Obj(bo) => format!("{}({:?})", bo.name, bo.kind),
                        other => other.type_name().to_string(),
                    })
                    .collect();
                eprintln!("[MRODBG-CLS] class={} bases={:?}", c.name, bn);
            }
            raise("TypeError", ap_obj::classes::mro_error_message(&e))
        })?;
        // T1-d: propagate the builtin-base marker — a class deriving from a
        // tuple/list/set/str subclass (directly via 3-arg type() or a class
        // stmt) keeps the backing-builtin construction semantics.
        for bt in &base_types {
            if let Some(Value::Str(b)) = bt
                .dict
                .get(&Value::Str(PyString::new("__ap_builtin_base__")))
            {
                let _ = t.dict.set(
                    Value::Str(PyString::new("__ap_builtin_base__")),
                    Value::Str(b.clone()),
                );
                break;
            }
        }
        // T2 (pytest capture EncodedFile(io.TextIOWrapper)): propagate the
        // stream-base marker so instantiate() routes construction through
        // the native stream ctor.
        for bt in &base_types {
            if let Some(Value::Str(b)) =
                bt.dict.get(&Value::Str(PyString::new("__ap_stream_base__")))
            {
                let _ = t.dict.set(
                    Value::Str(PyString::new("__ap_stream_base__")),
                    Value::Str(b.clone()),
                );
                break;
            }
            let bname = bt.name.clone();
            if matches!(
                bname.as_str(),
                "TextIOWrapper"
                    | "BytesIO"
                    | "StringIO"
                    | "BufferedReader"
                    | "BufferedWriter"
                    | "FileIO"
            ) {
                let _ = t.dict.set(
                    Value::Str(PyString::new("__ap_stream_base__")),
                    Value::Str(PyString::new(bname)),
                );
                break;
            }
        }
        // The RESOLVED metaclass rides the payload (type(C), meta hook lookup).
        *t.metaclass.borrow_mut() = _resolved_meta.clone();
        t
    };
    // T2 (pluggy varnames): the class-body __qualname__ rewrite must cover
    // BOTH creation paths (Session has a Python metaclass → meta-new path).
    // The class dict IS the live namespace; requalify undotted functions.
    {
        let mut keys: Vec<Value> = Vec::new();
        for (k, v) in t.dict.items() {
            if let Value::Obj(fo) = v {
                if fo.kind == ap_obj::ObjKind::Function {
                    let dotted = fo
                        .attrs
                        .borrow()
                        .get(&Value::Str(PyString::new("__qualname__")))
                        .and_then(|q| match q {
                            Value::Str(s) => Some(s.as_str().contains('.')),
                            _ => Some(false),
                        })
                        .unwrap_or(false);
                    if !dotted {
                        keys.push(k.clone());
                    }
                }
            }
        }
        for k in keys {
            if let Some(Value::Obj(fo)) = t.dict.get(&k) {
                let fname = fo.name.clone();
                let _ = fo.attrs.borrow_mut().set(
                    Value::Str(PyString::new("__qualname__")),
                    Value::Str(PyString::new(format!("{}.{}", t.name, fname))),
                );
            }
        }
    }
    // 02.04a fix pass: the creating metaclass rides the payload in BOTH
    // creation paths (the else branch set it inline; the meta __new__ path
    // left it None, breaking isinstance(C, EnumType)-style checks and any
    // metaclass hook dispatch on the new class).
    if t.metaclass.borrow().is_none() {
        if let Some(meta) = &_resolved_meta {
            *t.metaclass.borrow_mut() = Some(meta.clone());
        }
    }
    let class_v = ap_obj::classes::make_class_value(t.clone());

    // Module marker: builtin exceptions report "builtins" (e010); user
    // classes "__main__" (S-03-04). 02.08-full p2: stdlib-exception classes
    // report their home module (re.error.__module__ == 're').
    // 02.08-c: prefer the DEFINING frame's __name__ (CPython: a class
    // defined in module M reports __module__ == M; enum.global_enum's
    // sys.modules[cls.__module__].__dict__.update depends on it). Builtin
    // exceptions keep their pinned home modules.
    let module_name: String = if t.flags.builtin_exc {
        match t.name.as_str() {
            "PatternError" => "re".to_string(),
            "JSONDecodeError" => "json".to_string(),
            _ => "builtins".to_string(),
        }
    } else {
        match frame.globals.get(&Value::Str(PyString::new("__name__"))) {
            Some(Value::Str(modn)) => modn.as_str().to_string(),
            _ => ap_obj::MODULE_NAME.to_string(),
        }
    };
    let _ = t.dict.set(
        Value::Str(PyString::new("__module__")),
        Value::Str(PyString::new(module_name)),
    );

    // Slot members become member descriptors on the class (S-03-13:
    // data descriptors; type(A.__dict__["x"]).__name__ is member_descriptor).
    if let Some(slots) = &t.flags.slots {
        for sname in slots {
            if sname == "__dict__" {
                continue;
            }
            let d = ap_obj::PyObject::new(ap_obj::ObjKind::Descriptor, "member_descriptor");
            *d.descr_kind.borrow_mut() = Some("member".to_string());
            let _ = t
                .dict
                .set(Value::Str(PyString::new(sname.clone())), Value::Obj(d));
        }
    }

    // 7. Hooks: __set_name__ on ns descriptors, then the parent's
    //    __init_subclass__ with the class kwargs (S-03-27/29 order).
    //    04.01: search the hook across ALL bases' MROs — typing.py's
    //    _SpecialGenericAlias(_NotIterable, _BaseGenericAlias, _root=True)
    //    carries the hook on _Final, reached only through the SECOND base.
    run_set_name_hooks(interp, &class_v, &t)?;
    let hook_owner: Option<(Value, String)> = base_types.iter().find_map(|b| {
        b.lookup_with_owner("__init_subclass__")
            .filter(|(_f, owner)| owner.name != "object")
            .map(|(_f, owner)| {
                (
                    attr_on_class_raw(b, "__init_subclass__"),
                    owner.name.clone(),
                )
            })
    });
    if let Some(parent) = base_types.first() {
        let kwargs: Vec<(String, Value)> = CLASS_KWARGS.with(|c| c.borrow().clone());
        let has_user_hook = hook_owner.is_some();
        if !kwargs.is_empty() && !has_user_hook {
            // CPython (probed 3.14): object.__init_subclass__ rejects
            // class kwargs with the pinned TypeError.
            return Err(raise(
                "TypeError",
                format!("{}.__init_subclass__() takes no keyword arguments", c.name),
            ));
        }
        if hook_owner.is_some() {
            // Bind the parent's hook as an implicit classmethod of cls.
            // A USER def __init_subclass__(cls, ...) is stored as a raw
            // function (CPython wraps it implicitly) — bind cls here.
            let hook = hook_owner
                .as_ref()
                .map(|(h, _)| h.clone())
                .unwrap_or_else(|| attr_on_class_raw(parent, "__init_subclass__"));
            let hook_bound = if matches!(&hook, Value::Obj(o) if o.kind == ap_obj::ObjKind::Function)
            {
                bind_function(&hook, class_v.clone())
            } else {
                hook
            };
            let f = unwrap_classmethod(&hook_bound, class_v.clone());
            // T1 (scrapy fix pass): the user hook runs on EVERY subclass
            // creation (CPython calls __init_subclass__ unconditionally);
            // kwargs (if any) ride along. The old code only invoked it when
            // class kwargs were present, so plain `class B(A)` never ran
            // A.__init_subclass__ (typing's Protocol machinery needs it).
            // The DEFAULT hook (object.__init_subclass__) rejects kwargs
            // with the pinned text (S-03-29); user overrides consume them.
            let kwargs = CLASS_KWARGS.with(|c| c.borrow().clone());
            // T1 (scrapy fix pass): stage the classmethod super context so
            // zero-arg super() inside the hook resolves (typing:
            // super().__init_subclass__() in Protocol.__init_subclass__).
            if let Some(parent_t) = base_types.first() {
                *interp.pending_super_ctx.borrow_mut() =
                    Some((parent_t.clone(), class_v.clone(), true));
            }
            call_value(
                interp,
                &module_frame(interp),
                &f,
                &[],
                &kwargs
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect::<Vec<_>>(),
            )?;
        }
    }
    CLASS_KWARGS.with(|c| c.borrow_mut().clear());

    // T1 (scrapy fix pass): invoke the user metaclass __init__ (CPython calls
    // type.__new__ -> metaclass __init__(cls, name, bases, dict, **kwds)
    // during class creation; typing's _ProtocolMeta.__init__ sets
    // cls.__protocol_attrs__ here).
    {
        let meta = t.metaclass.borrow().clone();
        if let Some(mt) = meta {
            if mt.name != "type" {
                if let Some((minit, minit_owner)) = mt.lookup_with_owner("__init__") {
                    let kwargs: Vec<(String, Value)> = CLASS_KWARGS.with(|c| c.borrow().clone());
                    // Stage the super context: zero-arg super() inside the
                    // metaclass __init__ (typing: super().__init__()).
                    *interp.pending_super_ctx.borrow_mut() =
                        Some((minit_owner, class_v.clone(), true));
                    let _ = call_value(
                        interp,
                        &module_frame(interp),
                        &minit,
                        &[
                            class_v.clone(),
                            Value::Str(PyString::new(c.name.clone())),
                            Value::Tuple(Gc::new(Tuple(
                                base_types
                                    .iter()
                                    .map(|b| class_value_for_public(b))
                                    .collect(),
                            ))),
                            Value::Dict(t.dict.clone()),
                        ],
                        &kwargs,
                    );
                }
            }
        }
    }

    // 8. Register user-subclass edges (for __subclasses__).
    for b in &base_types {
        interp
            .user_subclasses
            .borrow_mut()
            .push((c.name.clone(), b.name.clone()));
    }
    interp
        .user_class_values
        .borrow_mut()
        .insert(c.name.clone(), class_v.clone());

    // 9. Seed the __class__ cell for zero-arg super() (S-03-17): every
    //    method defined in this body reads __class__ from its closure.
    crate::interp::seed_class_cell(interp, &class_frame, &class_v);

    // 10. Bind the class name in the enclosing scope (S-03-1).
    // 02.03b: class decorators apply bottom-up (S-02-7 applies to class
    // defs) — enum.py's @_simple_enum(StrEnum) converts a plain class by
    // re-creating it through 3-arg type(); dropping decorators silently
    // left EnumCheck with no members.
    let mut bound_v = class_v.clone();
    for d in c.decorators.iter().rev() {
        let df = eval_expr(interp, frame, d, reg)?;
        bound_v = call_value(interp, &module_frame(interp), &df, &[bound_v], &[])?;
    }
    store_name(interp, frame, bind_name, bound_v, reg)?;
    // 02.08-c: a class defined in module M reports __module__ == M (CPython
    // semantics; enum.global_enum's
    // `sys.modules[cls.__module__].__dict__.update(cls.__members__)` depends
    // on it). Take __name__ from the DEFINING frame's globals — the class
    // body's own ns may not carry it.
    if t.dict
        .get(&Value::Str(PyString::new("__module__")))
        .is_none()
    {
        if let Some(Value::Str(modn)) = frame.globals.get(&Value::Str(PyString::new("__name__"))) {
            let _ = t.dict.set(
                Value::Str(PyString::new("__module__")),
                Value::Str(PyString::new(modn.as_str().to_string())),
            );
        }
    }
    Ok(class_v)
}

/// 02.03b-fix: class kwargs are present when the class statement carried
/// keyword arguments (class A(metaclass=M, foo=1)).
fn class_kwargs_present(interp: &Interpreter) -> bool {
    let _ = interp;
    CLASS_KWARGS.with(|c| !c.borrow().is_empty())
}

thread_local! {
    static CLASS_KWARGS: std::cell::RefCell<Vec<(String, Value)>> =
        std::cell::RefCell::new(Vec::new());
}

fn qualname_for(frame: &Frame, name: &str) -> String {
    // Top-level: the bare name; nested: dotted (Outer.Inner per S-03-04).
    if frame.class_qual_prefix.is_empty() {
        name.to_string()
    } else {
        format!("{}.{}", frame.class_qual_prefix, name)
    }
}

fn string_items(l: &Gc<PyList>) -> Result<Vec<String>, Flow> {
    let mut out = Vec::new();
    for e in l.items().iter() {
        match e {
            Value::Str(s) => out.push(s.as_str().to_string()),
            other => {
                return Err(raise(
                    "TypeError",
                    format!(
                        "__slots__ items must be strings, not '{}'",
                        other.type_name()
                    ),
                ))
            }
        }
    }
    Ok(out)
}

/// The TypeRef behind a base-class expression value: user classes, builtin
/// exception classes, `object`, `type`.
pub fn base_type_ref(interp: &Interpreter, v: &Value) -> Result<Option<Rc<TypeRef>>, Flow> {
    match v {
        Value::Obj(o) if o.kind == ap_obj::ObjKind::UserClass => {
            let t = o.class_ref.borrow().clone();
            if t.is_none() {
                // Exception classes created by install_exception_classes.
                return Ok(exc_class(interp, &o.name));
            }
            Ok(t)
        }
        Value::Obj(o) if o.kind == ap_obj::ObjKind::Type && o.name == "object" => {
            Ok(Some(object_class(interp)))
        }
        Value::Obj(o) if o.kind == ap_obj::ObjKind::Type && o.name == "type" => {
            Ok(Some(type_class(interp)))
        }
        // 02.04a fix pass (BUG-A follow-on): `class M(type)` — metaclasses
        // written in Python (_ProtocolMeta(ABCMeta), NamedTupleMeta) must
        // derive the REAL type payload, otherwise super().__new__ inside
        // their bodies resolves against an empty synthesized "type" class
        // and dies with "no attribute '__new__'".
        Value::Obj(o) if o.kind == ap_obj::ObjKind::Type && o.name == "type" => {
            Ok(Some(type_class(interp)))
        }
        // 02.04a (interp bug #11, vendored-stdlib stress): builtin-type bases
        // (enum.py: `class IntEnum(int, Enum)`) — synthesize an object-derived
        // TypeObject carrying the builtin's name (isinstance/issubclass
        // against it stays a documented v0 gap).
        Value::Obj(o) if o.kind == ap_obj::ObjKind::Type => {
            let flags = TypeFlags {
                slots: None,
                is_exception: false,
                builtin_exc: false,
            };
            let t = TypeObject::create(
                o.name.clone(),
                o.name.clone(),
                vec![object_class(interp)],
                PyDict::new(),
                flags,
                None,
            )
            .map_err(|_| raise("TypeError", "cannot synthesize builtin base class"))?;
            // T1-d (attr/_make.py): dynamic tuple subclasses
            // (type("XAttributes", (tuple,), body)) must CONSTRUCT from an
            // iterable like tuple() does — mark the synthesized base so
            // instantiate's tuple-storage path engages.
            if matches!(
                o.name.as_str(),
                // W110 (T2): int subclasses (enum.IntEnum(int, Enum)) need
                // the builtin-ctor fallback — MyInt(1) builds from args.
                "tuple" | "list" | "dict" | "set" | "str" | "property" | "int"
            ) {
                let _ = t.dict.set(
                    Value::Str(PyString::new("__ap_builtin_base__")),
                    Value::Str(PyString::new(o.name.clone())),
                );
            }
            Ok(Some(t))
        }
        // T1 (scrapy fix pass): `class Foo(Generic[T])` - the Generic alias
        // placeholder resolves to its ORIGIN (Generic) as the effective base.
        // T1 (scrapy datatypes): the Namespace-shaped _GenericAlias produced
        // by Type.__getitem__ (`UserDict[str, object]`) carries __origin__ in
        // attrs — same treatment.
        Value::Obj(o)
            if (o.kind == ap_obj::ObjKind::Opaque && o.name == "typing._GenericAlias")
                || (o.kind == ap_obj::ObjKind::Namespace && o.name == "_GenericAlias") =>
        {
            let origin = if o.kind == ap_obj::ObjKind::Opaque {
                o.slice_parts
                    .borrow()
                    .first()
                    .cloned()
                    .unwrap_or(Value::None)
            } else {
                o.attrs
                    .borrow()
                    .get(&Value::Str(PyString::new("__origin__")))
                    .unwrap_or(Value::None)
            };

            match base_type_ref(interp, &origin)? {
                Some(t) => Ok(Some(t)),
                None => Ok(None),
            }
        }
        // 02.04a (interp bug #2, vendored-stdlib stress): abc.py defines
        // `class abstractclassmethod(classmethod)` — the decorator builtins
        // are BuiltinFn markers, not types. Synthesize an object-derived
        // class so the subclass CREATES; descriptor behavior still rides
        // the decorator path (documented v0 gap: inheriting descriptor
        // semantics from these bases is unsupported).
        // T1 (w3lib.url): `class ParseDataURIResult(NamedTuple)` — the vendored
        // typing.NamedTuple is a FUNCTION; a function base synthesizes an
        // object-derived class (annotation-driven fields are a documented
        // v0 gap; w3lib only instantiates it and reads .data/.headers).
        Value::Obj(o)
            if (o.kind == ap_obj::ObjKind::BuiltinFn || o.kind == ap_obj::ObjKind::Function)
                && matches!(o.name.as_str(), "NamedTuple" | "TypedDict") =>
        {
            let flags = TypeFlags {
                slots: None,
                is_exception: false,
                builtin_exc: false,
            };
            let t = TypeObject::create(
                "NamedTuple".to_string(),
                "NamedTuple".to_string(),
                vec![object_class(interp)],
                PyDict::new(),
                flags,
                None,
            )
            .map_err(|_| raise("TypeError", "cannot synthesize NamedTuple base"))?;
            Ok(Some(t))
        }
        Value::Obj(o)
            if o.kind == ap_obj::ObjKind::BuiltinFn
                && matches!(o.name.as_str(), "classmethod" | "staticmethod" | "property") =>
        {
            let flags = TypeFlags {
                slots: None,
                is_exception: false,
                builtin_exc: false,
            };
            let t = TypeObject::create(
                o.name.clone(),
                o.name.clone(),
                vec![object_class(interp)],
                PyDict::new(),
                flags,
                None,
            )
            .map_err(|_| raise("TypeError", "cannot synthesize decorator base class"))?;
            // T1-d (twisted _DeprecatedProperty(property)): subclasses of
            // property construct via the builtin-base path (fget parked).
            if o.name == "property" {
                let _ = t.dict.set(
                    Value::Str(PyString::new("__ap_builtin_base__")),
                    Value::Str(PyString::new("property")),
                );
            }
            Ok(Some(t))
        }
        // T1-d: catch-all — any remaining function-base (BuiltinFn or Function)
        // NOT covered by the descriptor allowlist above synthesizes an
        // object-derived TypeObject named after the function. CPython would
        // TypeError, but the T1 chain policy is create-then-degrade; descriptor
        // inheritance from these bases is a documented v0 gap (docs/lxml-shim.md
        // DEVIATIONS).
        Value::Obj(o)
            if o.kind == ap_obj::ObjKind::BuiltinFn || o.kind == ap_obj::ObjKind::Function =>
        {
            let flags = TypeFlags {
                slots: None,
                is_exception: false,
                builtin_exc: false,
            };
            let t = TypeObject::create(
                o.name.clone(),
                o.name.clone(),
                vec![object_class(interp)],
                PyDict::new(),
                flags,
                None,
            )
            .map_err(|_| raise("TypeError", "cannot synthesize function base class"))?;
            Ok(Some(t))
        }
        _ => Ok(None),
    }
}

/// The `object` root class (mro == [object]).
pub fn object_class(interp: &Interpreter) -> Rc<TypeRef> {
    if let Some(t) = interp.object_class.borrow().clone() {
        return t;
    }
    let t = TypeObject::create(
        "object".to_string(),
        "object".to_string(),
        vec![],
        PyDict::new(),
        TypeFlags::default(),
        None,
    )
    .expect("object mro");
    let _ = ap_obj::classes::make_class_value(t.clone()); // wire the Value back-pointer
    *interp.object_class.borrow_mut() = Some(t.clone());
    t
}

/// The `type` metaclass handle (identity target for metaclass checks).
pub fn type_class(interp: &Interpreter) -> Rc<TypeRef> {
    if let Some(t) = interp.type_class.borrow().clone() {
        return t;
    }
    let obj = object_class(interp);
    let dict = PyDict::new();
    // 02.04a (interp bug #17): type.__new__ reachable via super().__new__
    // inside user metaclass bodies (ABCMeta/EnumType).
    let _ = dict.set(
        Value::Str(PyString::new("__new__")),
        Value::Obj(ap_obj::PyObject::new(
            ap_obj::ObjKind::BuiltinFn,
            "type.__new__",
        )),
    );
    let t = TypeObject::create(
        "type".to_string(),
        "type".to_string(),
        vec![obj],
        dict,
        TypeFlags::default(),
        None,
    )
    .expect("type mro");
    let _ = ap_obj::classes::make_class_value(t.clone()); // wire the Value back-pointer
    *interp.type_class.borrow_mut() = Some(t.clone());
    t
}

// ===========================================================================
// Descriptor protocol (S-03-20/21)
// ===========================================================================

/// Classify a class-dict entry as a descriptor: Some(kind).
pub fn descriptor_kind(interp: &Interpreter, v: &Value) -> Option<DescrKind> {
    match v {
        Value::Obj(o) if o.kind == ap_obj::ObjKind::Descriptor => {
            match o.descr_kind.borrow().as_deref() {
                Some("property") => Some(DescrKind::Property),
                Some("classmethod") => Some(DescrKind::ClassMethod),
                Some("staticmethod") => Some(DescrKind::StaticMethod),
                Some("member") => Some(DescrKind::Property), // slot descriptor: data
                _ => None,
            }
        }
        Value::Obj(o) if o.kind == ap_obj::ObjKind::UserInstance => {
            let t = instance_class(interp, o)?;
            if t.lookup("__get__").is_some() {
                Some(DescrKind::UserDescr)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Is the descriptor DATA (type defines __set__ or __delete__, S-03-20)?
pub fn is_data_descriptor(interp: &Interpreter, v: &Value) -> bool {
    match v {
        Value::Obj(o) if o.kind == ap_obj::ObjKind::Descriptor => matches!(
            o.descr_kind.borrow().as_deref(),
            Some("property") | Some("member")
        ),
        Value::Obj(o) if o.kind == ap_obj::ObjKind::UserInstance => {
            matches!(instance_class(interp, o), Some(t) if t.lookup("__set__").is_some() || t.lookup("__delete__").is_some())
        }
        // T1-d fix (attr/_make.py cached_property slots path): a FUNCTION
        // marked as cached_property is a data descriptor — instance reads
        // must dispatch its __get__ instead of falling to instance dict /
        // __getattr__ (which returned the raw fn and broke `c.double`).
        Value::Obj(o)
            if o.kind == ap_obj::ObjKind::Function
                && o.attrs
                    .borrow()
                    .get(&Value::Str(PyString::new("__ap_cached_property__")))
                    .is_some_and(|x| x.is_truthy()) =>
        {
            true
        }
        _ => false,
    }
}

/// The class payload of a UserInstance PyObject.
pub fn instance_class(_interp: &Interpreter, o: &Gc<ap_obj::PyObject>) -> Option<Rc<TypeRef>> {
    o.type_ref.borrow().as_ref().and_then(|w| w.upgrade())
}

/// Raw class-dict lookup that does NOT invoke descriptors (for
/// unwrapping classmethod/staticmethod payloads).
fn attr_on_class_raw(t: &Rc<TypeRef>, name: &str) -> Value {
    t.lookup(name).unwrap_or(Value::None)
}

/// Invoke a descriptor's __get__ (or return the value unchanged).
/// `obj`: the instance (None-typed Value::None for class access),
/// `owner`: the class Value.
pub fn descr_get(interp: &Rc<Interpreter>, v: &Value, obj: &Value, owner: &Value) -> EResult {
    match v {
        Value::Obj(o) if o.kind == ap_obj::ObjKind::Descriptor => {
            let f: Value = o.func.borrow().clone().unwrap_or(Value::None);
            match o.descr_kind.borrow().as_deref() {
                Some("staticmethod") => Ok(f),
                Some("classmethod") => {
                    call_value(interp, &module_frame(interp), &f, &[owner.clone()], &[])
                }
                Some("property") | Some("member") => {
                    if matches!(obj, Value::None) {
                        // C.p -> the property object itself (S-03-21).
                        Ok(v.clone())
                    } else {
                        // W112: if obj is a UserInstance whose type_ref
                        // upgrade fails (dropped weakref), fget binds
                        // self = the class - the suspected source of
                        // '_rawentry'/'stash' class-receiver misses.
                        if let Value::Obj(io) = obj {
                            if io.kind == ap_obj::ObjKind::UserInstance {
                                let upgrade_ok = io
                                    .type_ref
                                    .borrow()
                                    .as_ref()
                                    .map(|w| w.upgrade().is_some())
                                    .unwrap_or(false);
                                if !upgrade_ok && std::env::var("APX_TBDBG").is_ok() {
                                    eprintln!(
                                        "[APROP] fget self type_ref DEAD: inst={} class={}",
                                        io.name,
                                        o.name
                                    );
                                }
                            }
                        }
                        let fget = o.self_value.borrow().clone().unwrap_or(Value::None);
                        call_value(interp, &module_frame(interp), &fget, &[obj.clone()], &[])
                    }
                }
                _ => Ok(v.clone()),
            }
        }
        Value::Obj(_) if descriptor_kind(interp, v) == Some(DescrKind::UserDescr) => {
            let t = instance_class(
                interp,
                match v {
                    Value::Obj(o) => o,
                    _ => unreachable!(),
                },
            )
            .expect("user descriptor class");
            let g = t.lookup("__get__").expect("checked");
            call_value(
                interp,
                &module_frame(interp),
                &g,
                &[v.clone(), obj.clone(), owner.clone()],
                &[],
            )
        }
        _ => Ok(v.clone()),
    }
}

/// Data-descriptor __set__ / property setter (S-03-10). Returns Err when
/// the descriptor is read-only with the byte-exact message.
pub fn descr_set(
    interp: &Rc<Interpreter>,
    v: &Value,
    obj: &Value,
    val: Value,
    cls_name: &str,
    attr: &str,
) -> Result<(), Flow> {
    match v {
        Value::Obj(o) if o.kind == ap_obj::ObjKind::Descriptor => {
            match o.descr_kind.borrow().as_deref() {
                Some("property") | Some("member") => {
                    let fset = o.func.borrow().clone().unwrap_or(Value::None);
                    if matches!(fset, Value::None) {
                        return Err(raise(
                            "AttributeError",
                            format!("property '{attr}' of '{cls_name}' object has no setter"),
                        ));
                    }
                    call_value(
                        interp,
                        &module_frame(interp),
                        &fset,
                        &[obj.clone(), val],
                        &[],
                    )
                    .map(|_| ())
                }
                _ => Ok(()),
            }
        }
        Value::Obj(_) if is_data_descriptor(interp, v) => {
            let o = match v {
                Value::Obj(o) => o,
                _ => unreachable!(),
            };
            let t = instance_class(interp, o).expect("user descriptor class");
            let s = t.lookup("__set__").expect("checked");
            call_value(
                interp,
                &module_frame(interp),
                &s,
                &[v.clone(), obj.clone(), val],
                &[],
            )
            .map(|_| ())
        }
        _ => Ok(()),
    }
}

/// Data-descriptor __delete__ / property deleter (S-03-11).
pub fn descr_delete(
    interp: &Rc<Interpreter>,
    v: &Value,
    obj: &Value,
    cls_name: &str,
    attr: &str,
) -> Result<(), Flow> {
    match v {
        Value::Obj(o) if o.kind == ap_obj::ObjKind::Descriptor => {
            match o.descr_kind.borrow().as_deref() {
                Some("property") | Some("member") => {
                    let fdel = o.dunders.borrow().get(&Value::Str(PyString::new("fdel")));
                    if let Some(f) = fdel {
                        return call_value(interp, &module_frame(interp), &f, &[obj.clone()], &[])
                            .map(|_| ());
                    }
                    Err(raise(
                        "AttributeError",
                        format!("property '{attr}' of '{cls_name}' object has no deleter"),
                    ))
                }
                _ => Ok(()),
            }
        }
        Value::Obj(_) if is_data_descriptor(interp, v) => {
            let o = match v {
                Value::Obj(o) => o,
                _ => unreachable!(),
            };
            let t = instance_class(interp, o).expect("user descriptor class");
            let d = t.lookup("__delete__").expect("checked");
            call_value(
                interp,
                &module_frame(interp),
                &d,
                &[v.clone(), obj.clone()],
                &[],
            )
            .map(|_| ())
        }
        _ => Ok(()),
    }
}

/// Unwrap a classmethod descriptor to a cls-bound callable (plain values
/// pass through).
pub fn unwrap_classmethod(v: &Value, cls: Value) -> Value {
    if let Value::Obj(o) = v {
        if o.kind == ap_obj::ObjKind::Descriptor
            && matches!(o.descr_kind.borrow().as_deref(), Some("classmethod"))
        {
            if let Some(f) = o.func.borrow().clone() {
                return bind_function(&f, cls);
            }
        }
    }
    v.clone()
}

/// Bind a plain function as a bound method with `receiver` (functions pass
/// through unchanged; staticmethods already unwrapped).
pub fn bind_function(f: &Value, receiver: Value) -> Value {
    bind_function_owned(f, receiver, None)
}

/// bind_function + the defining class (zero-arg super start point).
pub fn bind_function_owned(f: &Value, receiver: Value, owner: Option<&Rc<TypeRef>>) -> Value {
    if let Value::Obj(o) = f {
        if o.kind == ap_obj::ObjKind::Function {
            // 02.04: build the PyObject payload FIRST, mutate while owned,
            // then hand it to Gc::new (which registers it with the cycle
            // collector). The old try_unwrap-after-Gc::new shape broke once
            // the registry held a second strong count on fresh objects.
            let m = ap_obj::PyObject {
                kind: ap_obj::ObjKind::BoundMethod,
                name: o.name.clone(),
                fn_id: None,
                self_value: std::cell::RefCell::new(Some(receiver)),
                func: std::cell::RefCell::new(Some(f.clone())),
                ints: std::cell::RefCell::new(Vec::new()),
                slice_parts: std::cell::RefCell::new(Vec::new()),
                attrs: std::cell::RefCell::new(ap_obj::dict::PyDict::new()),
                args: std::cell::RefCell::new(Vec::new()),
                cause: std::cell::RefCell::new(None),
                context: std::cell::RefCell::new(None),
                dunders: std::cell::RefCell::new((*ap_obj::dict::PyDict::new()).clone()),
                gen_id: std::cell::RefCell::new(None),
                repr_text: std::cell::RefCell::new(None),
                module_ns: std::cell::RefCell::new(None),
                closure_slot: std::cell::RefCell::new(None),
                defaults: std::cell::RefCell::new(Vec::new()),
                class_ref: std::cell::RefCell::new(None),
                type_ref: std::cell::RefCell::new(None),
                descr_kind: std::cell::RefCell::new(None),
                method_owner: std::cell::RefCell::new(owner.map(std::rc::Rc::downgrade)),
            };
            let mo = ap_obj::int::Gc::new(m);
            return Value::Obj(mo);
        }
    }
    f.clone()
}

/// A bare module frame for hook/descriptor calls that need no caller scope.
pub fn module_frame(interp: &Rc<Interpreter>) -> Rc<Frame> {
    Rc::new(Frame::new_module(interp.globals.clone()))
}

fn run_set_name_hooks(interp: &Rc<Interpreter>, cls: &Value, t: &Rc<TypeRef>) -> Result<(), Flow> {
    for (k, v) in t.dict.items() {
        let name = match &k {
            Value::Str(s) => s.as_str().to_string(),
            _ => continue,
        };
        if let Value::Obj(o) = &v {
            if o.kind == ap_obj::ObjKind::UserInstance {
                if let Some(dt) = instance_class(interp, o) {
                    if let Some(hook) = dt.lookup("__set_name__") {
                        call_value(
                            interp,
                            &module_frame(interp),
                            &hook,
                            &[v.clone(), cls.clone(), Value::Str(PyString::new(name))],
                            &[],
                        )?;
                    }
                }
            }
        }
    }
    Ok(())
}

// ===========================================================================
// isinstance / issubclass (S-03-31)
// ===========================================================================

/// isinstance(x, T): T a class, tuple, or union-type shape (X|Y lowers to a
/// call in v1 — unions only surface via tuple form).
pub fn value_isinstance(interp: &Interpreter, x: &Value, t: &Value) -> Result<bool, Flow> {
    // Union marker (X | Y lowered by the executor): any arm matches.
    if let Value::Obj(o) = t {
        // Both union shapes: the executor's Opaque "__union__" marker AND
        // the binop arm's Namespace "_Union" (interp.rs). T2 gate finding
        // 1: only the Opaque shape matched here, so isinstance(user_inst,
        // tuple | list) fell through to base_type_ref and TypeErrored.
        if (o.kind == ap_obj::ObjKind::Opaque && o.name == "__union__")
            || (o.kind == ap_obj::ObjKind::Namespace && o.name == "_Union")
        {
            for arm in o.slice_parts.borrow().iter() {
                // PEP 604: `X | None` parks None as a Value::None arm —
                // the arm matches NoneType (isinstance(None, int|None)).
                if matches!(arm, Value::None) {
                    if matches!(x, Value::None) {
                        return Ok(true);
                    }
                    continue;
                }
                if value_isinstance(interp, x, arm)? {
                    return Ok(true);
                }
            }
            return Ok(false);
        }
    }
    match t {
        Value::Tuple(parts) => {
            for p in &parts.0 {
                if value_isinstance(interp, x, p)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        _ => {
            // s014: namedtuple instances ARE tuples (class carries the
            // __tupleclass__ marker set at class creation).
            if let Value::Obj(xo) = x {
                if xo.kind == ap_obj::ObjKind::UserInstance {
                    if let Some(xc) = crate::classes::type_of(interp, x) {
                        if xc
                            .dict
                            .get(&Value::Str(PyString::new("__tupleclass__")))
                            .is_some_and(|v| v.is_truthy())
                            && matches!(
                                t,
                                Value::Obj(to) if to.kind == ap_obj::ObjKind::Type && to.name == "tuple"
                            )
                        {
                            return Ok(true);
                        }
                    }
                }
            }
            // 02.04a fix pass: `isinstance(SomeType, type)` — a builtin/interned
            // type object against the real `type` class (warnings.py
            // isinstance(category, type)).
            if matches!(x, Value::Obj(o) if o.kind == ap_obj::ObjKind::Type || o.kind == ap_obj::ObjKind::UserClass)
            {
                if let Value::Obj(to) = t {
                    if to.kind == ap_obj::ObjKind::UserClass && to.name == "type" {
                        return Ok(true);
                    }
                }
            }
            // T1-d (inspect iscode etc): a Namespace/Opaque payload
            // (<code>, sys.implementation, thread) IS an instance of its
            // interned name-Type.
            if let Value::Obj(xo) = x {
                if matches!(
                    xo.kind,
                    ap_obj::ObjKind::Namespace
                        | ap_obj::ObjKind::Opaque
                        | ap_obj::ObjKind::NamedTuple
                ) {
                    if let Value::Obj(to) = t {
                        if to.kind == ap_obj::ObjKind::Type && to.name == xo.name {
                            return Ok(true);
                        }
                    }
                }
            }
            // 02.04a fix pass: CLASS subject vs a METACLASS target
            // (enum.py _get_mixins_: isinstance(first_enum, EnumType)) —
            // answer via the subject's metaclass chain.
            if let Value::Obj(xo) = x {
                if xo.kind == ap_obj::ObjKind::UserClass {
                    if let Some(xc) = crate::classes::type_of(interp, x) {
                        // T1-d fix (constantly): isinstance(C, C) for a
                        // plain class C (no user metaclass) is FALSE in
                        // CPython — a class is not an instance of itself.
                        // The v0 conflation (subject-vs-target subclass)
                        // made constantly's `_constantType = NamedConstant`
                        // class-body attr pass the NamedConstant isinstance
                        // filter and explode on `_container`.
                        if let Some(tc) = base_type_ref(interp, t)? {
                            if Rc::ptr_eq(&xc, &tc) {
                                let has_user_meta = xc
                                    .metaclass
                                    .borrow()
                                    .as_ref()
                                    .map(|m| !Rc::ptr_eq(m, &type_class(interp)))
                                    .unwrap_or(false);
                                if !has_user_meta {
                                    return Ok(false);
                                }
                            }
                            if let Some(meta) = xc.metaclass.borrow().clone() {
                                if meta.is_subclass(&tc) {
                                    return Ok(true);
                                }
                            }
                            // Wave-82: a CLASS subject is an instance of
                            // its METACLASS, never of the class's own MRO.
                            // With no user metaclass the effective
                            // metaclass is `type`; isinstance(T1Spider,
                            // Spider) must be False (issubclass(type,
                            // Spider) is False) — the old
                            // xc.is_subclass(&tc) wrongly walked the
                            // class's own MRO and answered True.
                            if !Rc::ptr_eq(&xc, &tc) {
                                return Ok(false);
                            }
                            return Ok(xc.is_subclass(&tc));
                        }
                        if matches!(t, Value::Obj(o) if o.kind == ap_obj::ObjKind::UserClass || o.kind == ap_obj::ObjKind::Type)
                        {
                            return Ok(false);
                        }
                    }
                }
            }
            let Some(xc) = type_for_check(interp, x) else {
                // Builtin values: fall back to the name table (S-01 types).
                return Ok(builtins::type_matches(x, type_name_of_check(t)));
            };
            let Some(tc) = base_type_ref(interp, t)? else {
                // Name-Type target (str/int/...): a user-instance subject is
                // simply not one (no error); only non-type targets error.
                if matches!(t, Value::Obj(o) if o.kind == ap_obj::ObjKind::Type) {
                    return Ok(false);
                }
                // T1 (dataclasses): typing.Union / special-form fn-MARKERS —
                // isinstance is False (v0 tolerance; CPython answers via the
                // metaclass __instancecheck__ the vendored typing lacks).
                let sf_marker = match t {
                    Value::Obj(o) => {
                        matches!(
                            o.kind,
                            ap_obj::ObjKind::BuiltinFn | ap_obj::ObjKind::Function
                        ) && (o.name == "Union"
                            || o.name == "_typing.Union"
                            // T2 (typing_extensions 4.16): _TypeVarLikeMeta
                            //.__instancecheck__ delegates to
                            // isinstance(x, typing.ParamSpec/TypeVar/...) —
                            // these ctor MARKERS must answer False (never
                            // raise), or the sentinel/ParamSpec import chain
                            // dies with "'str' object has no attribute
                            // 'setter'" from the except-miss.
                            || matches!(
                                o.name.as_str(),
                                "ParamSpec" | "TypeVar" | "TypeVarTuple" | "NewType"
                            )
                            || o.name.starts_with("_typing.ParamSpec")
                            || o.name.starts_with("_typing.TypeVar")
                            || o.name.starts_with("__ap_typing_"))
                    }
                    _ => false,
                };
                if sf_marker {
                    return Ok(false);
                }
                // T1 (scrapy.utils.python): isinstance(func, partial) — the
                // ap functools.partial is the '<partial:...>' marker fn.
                if matches!(t, Value::Obj(to) if to.kind == ap_obj::ObjKind::BuiltinFn
                    && matches!(to.name.as_str(), "partial" | "functools.partial"))
                {
                    let is_partial = matches!(x, Value::Obj(vo) if vo.kind == ap_obj::ObjKind::BuiltinFn
                        && vo.name.starts_with("<partial:"));
                    return Ok(is_partial);
                }
                return Err(raise(
                    "TypeError",
                    "isinstance() arg 2 must be a type, a tuple of types, or a union",
                ));
            };
            if xc.is_subclass(&tc) {
                return Ok(true);
            }
            // T2 (pluggy _static_hook_attr): CPython resolves
            // isinstance(x, C) as type(C).__instancecheck__(C, x) — the
            // hook lives on the TARGET's metaclass, never the subject's.
            // The old arm consulted the SUBJECT class's metaclass, whose
            // chain (NodeMeta/ABCMeta ⊂ type) subclass-matched every
            // user-class target: isinstance(session_instance,
            // AnyUserClass) answered True and pluggy treated Session
            // instances as classes (unbound hookimpls → "missing 1
            // required positional argument: 'self'"). For a plain
            // instance subject with no MRO match, CPython answers False;
            // a target with a custom metaclass hook is handled via
            // class_isinstance's target-metaclass arm.
            if matches!(x, Value::Obj(o) if o.kind == ap_obj::ObjKind::UserInstance) {
                // Target metaclass __instancecheck__ (proper spelling):
                // only when the TARGET carries a custom hook.
                if let Some(meta) = tc.metaclass.borrow().clone() {
                    {
                        if std::env::var("APX_GEN_DEBUG").is_ok() {
                            let hookshape = meta
                                .lookup("__instancecheck__")
                                .map(|h| match &h {
                                    Value::Obj(f) => format!("Obj kind={:?}", f.kind),
                                    other => format!("other {}", other.type_name()),
                                })
                                .unwrap_or_else(|| "MISSING".to_string());
                            eprintln!(
                                "[ICHECK] meta={} hook={}",
                                meta.name.as_str(),
                                hookshape
                            );
                        }
                        if let Some(hook) = meta.lookup("__instancecheck__") {
                            // W112: ABCMeta.__instancecheck__ is a
                            // classmethod (a descr marker, not a raw
                            // Function) - the kind check skipped it and
                            // every ABC isinstance on a dict SUBCLASS
                            // (scrapy Headers) answered False. Call
                            // through whatever shape the hook has.
                            let is_callable = matches!(&hook, Value::Obj(f) if matches!(
                                f.kind,
                                ap_obj::ObjKind::Function
                                    | ap_obj::ObjKind::BuiltinFn
                                    | ap_obj::ObjKind::Descriptor
                            )) || matches!(hook, Value::Obj(ref f) if f.kind == ap_obj::ObjKind::BoundMethod)
                                || !matches!(hook, Value::None);
                            if is_callable {
                                let interp_rc = crate::interp::current_interp_handle();
                                let r = crate::interp::call_value_pub(
                                    &interp_rc,
                                    &hook,
                                    &[t.clone(), x.clone()],
                                    &[],
                                )?;
                                return Ok(r.is_truthy());
                            }
                        }
                    }
                }
                return Ok(false);
            }
            // Custom metaclass __instancecheck__ (S-03-31; c033 Weird).
            if let Some(meta) = xc.metaclass.borrow().clone() {
                if let Some(hook) = meta.lookup("__instancecheck__") {
                    if matches!(&hook, Value::Obj(f) if f.kind == ap_obj::ObjKind::Function) {
                        let interp_rc = crate::interp::current_interp_handle();
                        let r = crate::interp::call_value_pub(
                            &interp_rc,
                            &hook,
                            &[class_value_for(&xc), x.clone()],
                            &[],
                        )?;
                        return Ok(r.is_truthy());
                    }
                }
            }
            // CLASS subject: isinstance(C, M) consults the metaclass chain
            // (type(C2) is M2 ⊂ M, c014/c032).
            if matches!(x, Value::Obj(o) if o.kind == ap_obj::ObjKind::UserClass) {
                if let Some(meta) = xc.metaclass.borrow().clone() {
                    return Ok(meta.is_subclass(&tc));
                }
            }
            Ok(false)
        }
    }
}

/// Type name a check-target reports (class objects report their name).
fn type_name_of_check(t: &Value) -> &str {
    match t {
        Value::Obj(o) => o.name.as_str(),
        _ => "",
    }
}

/// The class payload for isinstance's LEFT argument (None -> builtin path).
fn type_for_check(interp: &Interpreter, x: &Value) -> Option<Rc<TypeRef>> {
    match x {
        Value::Obj(o) if o.kind == ap_obj::ObjKind::UserInstance => o
            .type_ref
            .borrow()
            .as_ref()
            .and_then(|w| w.upgrade())
            .map(|rc| rc as Rc<TypeRef>),
        Value::Obj(o) if o.kind == ap_obj::ObjKind::Exception => exc_class(interp, &o.name),
        _ => None,
    }
}

/// issubclass(A, B) over class payloads.
pub fn value_issubclass(interp: &Interpreter, a: &Value, b: &Value) -> Result<bool, Flow> {
    match b {
        Value::Tuple(parts) => {
            for p in &parts.0 {
                if value_issubclass(interp, a, p)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        _ => {
            let Some(ac) = base_type_ref(interp, a)? else {
                return Err(raise("TypeError", "issubclass() arg 1 must be a class"));
            };
            let Some(bc) = base_type_ref(interp, b)? else {
                if matches!(b, Value::Obj(o) if o.kind == ap_obj::ObjKind::Type) {
                    return Ok(false);
                }
                return Err(raise(
                    "TypeError",
                    "issubclass() arg 2 must be a class, a tuple of classes, or a union",
                ));
            };
            // 02.08-full part 1: a USER class is never a subclass of an
            // unrelated LEGACY type via the metaclass hook walk (the ABC
            // register path, e.g. Mapping.register(mappingproxy), probes
            // issubclass(Mapping, dict) and must see FALSE, or the cycle
            // check mis-fires and aborts the _collections_abc import).
            if ac.is_subclass(&bc) {
                return Ok(true);
            }
            if matches!(b, Value::Obj(o) if o.kind == ap_obj::ObjKind::Type)
                && matches!(a, Value::Obj(o) if o.kind == ap_obj::ObjKind::UserClass)
            {
                return Ok(false);
            }
            // Custom metaclass __subclasscheck__ (S-03-31; c033 WS).
            // 02.04a fix pass (BUG-A): the hook lives on type(b) — CPython
            // resolves issubclass(a, b) as type(b).__subclasscheck__(b, a).
            // The old code consulted a's OWN metaclass and passed (a, a),
            // so _py_abc's register cycle check issubclass(Coroutine,
            // <coroutine type>) evaluated __subclasscheck__(Coroutine,
            // Coroutine) -> True -> "Refusing to create an inheritance
            // cycle" on `import pathlib`.
            if let Some(bct) = base_type_ref(interp, b)? {
                let meta = bct.metaclass.borrow().clone();
                if std::env::var("APX_DBG_HOOK").is_ok() {
                    let hn = match &meta {
                        Some(mt) => format!("meta={}", mt.name),
                        None => "no-meta".to_string(),
                    };
                    eprintln!("DBG hook b={} {hn}", bct.name);
                }
                if let Some(meta) = meta {
                    if let Some(hook) = meta.lookup("__subclasscheck__") {
                        if matches!(&hook, Value::Obj(f) if f.kind == ap_obj::ObjKind::Function) {
                            let interp_rc = crate::interp::current_interp_handle();
                            let r = crate::interp::call_value_pub(
                                &interp_rc,
                                &hook,
                                &[class_value_for(&bct), a.clone()],
                                &[],
                            )?;
                            if std::env::var("APX_DBG_HOOK").is_ok() {
                                eprintln!(
                                    "DBG hook res {} <= {}: {}",
                                    bct.name,
                                    match a {
                                        Value::Obj(o) => o.name.clone(),
                                        other => other.type_name().to_string(),
                                    },
                                    r.is_truthy()
                                );
                            }
                            return Ok(r.is_truthy());
                        }
                    }
                }
            }
            Ok(false)
        }
    }
}

// ===========================================================================
// Special-method type lookup (S-03-22/23): TYPE MRO only, never the
// instance dict.
// ===========================================================================

/// Look a special method up on the value's TYPE (MRO scan). Returns the
/// raw function value (unbound) + defining class.
pub fn special_lookup(interp: &Interpreter, v: &Value, name: &str) -> Option<(Value, Rc<TypeRef>)> {
    match v {
        Value::Obj(o) if o.kind == ap_obj::ObjKind::UserInstance => {
            let t = o.type_ref.borrow().as_ref().and_then(|w| w.upgrade())?;
            t.lookup_with_owner(name)
        }
        // T1-d fix (incremental/_Inf inside Version tuples): elementwise
        // tuple ordering consults the ORDERING dunders on OBJECT elements
        // (Version.__lt__ → _cmp → _Inf().__lt__/__gt__). UserClass elements
        // answer their class-level dunders bound to the class value.
        Value::Obj(o) if o.kind == ap_obj::ObjKind::UserClass => {
            // W111: class-as-instance dunder PROTOCOL lookups (__bool__/
            // __len__/__iter__...) on a USER class must NOT resolve from
            // the class's own MRO — CPython only consults the METACLASS
            // (no metaclass __len__ => assert SomeClass is always True).
            // The MRO scan here made `assert self.pqclass` (pqclass = the
            // DownloaderAwarePriorityQueue class) call its unbound __len__
            // with self=the class -> AttributeError 'pqueues'.
            if name.starts_with("__")
                && name.ends_with("__")
                && matches!(
                    name,
                    "__bool__" | "__len__" | "__iter__" | "__getitem__" | "__contains__"
                        | "__call__" | "__next__" | "__enter__" | "__exit__" | "__eq__"
                        | "__lt__" | "__le__" | "__gt__" | "__ge__" | "__hash__"
                        | "__str__" | "__repr__"
                )
            {
                return None;
            }
            let t = o.class_ref.borrow().clone()?;
            t.lookup_with_owner(name)
        }
        // 02.08-c: lxml elements answer len()/iter() dunders natively.
        Value::Obj(o)
            if o.kind == ap_obj::ObjKind::Opaque
                && (o.name == "lxml.etree._Element" || o.name == "lxml.html.HtmlElement")
                && matches!(name, "__len__" | "__iter__" | "__getitem__") =>
        {
            let marker =
                ap_obj::PyObject::new(ap_obj::ObjKind::BuiltinFn, &format!("lxml.elem.{}", name));
            *marker.slice_parts.borrow_mut() = vec![v.clone()];
            // owner type is unused by the marker dispatch path
            Some((Value::Obj(marker), object_class(interp)))
        }
        // T2 (wave-39): stream-derived user instances (CaptureIO etc) are
        // Opaque stream objects; super().__init__(buffer, ...) must bind
        // the base-stream init marker so the ctor args land in the attrs.
        Value::Obj(o)
            if o.kind == ap_obj::ObjKind::Opaque
                && matches!(
                    o.name.as_str(),
                    "TextIOWrapper"
                        | "BytesIO"
                        | "StringIO"
                        | "BufferedReader"
                        | "BufferedWriter"
                        | "FileIO"
                )
                && name == "__init__" =>
        {
            let marker = ap_obj::PyObject::new(
                ap_obj::ObjKind::BuiltinFn,
                &format!("__ap_base_{}__init__", o.name),
            );
            *marker.slice_parts.borrow_mut() = vec![v.clone()];
            Some((Value::Obj(marker), object_class(interp)))
        }
        // T2 (pytest collection): os.ScandirIterator iterates its cached
        // entries (CPython: scandir's iterator; our __iter__ marker yields
        // the stored list via the call_os surface).
        Value::Obj(o)
            if o.kind == ap_obj::ObjKind::Opaque
                && o.name == "os.ScandirIterator"
                && name == "__iter__" =>
        {
            let marker = ap_obj::PyObject::new(
                ap_obj::ObjKind::BuiltinFn,
                "os.ScandirIterator.__iter__",
            );
            *marker.slice_parts.borrow_mut() = vec![v.clone()];
            Some((Value::Obj(marker), object_class(interp)))
        }
        _ => None,
    }
}

// ===========================================================================
// Attribute protocol (S-03-09..12)
// ===========================================================================

/// `obj.name` full protocol. Returns Err(Flow) only for real errors;
/// misses raise AttributeError with the pinned text (caller distinguishes
/// via the `quiet` form below when it needs Option semantics).
pub fn attr_get_protocol(interp: &Rc<Interpreter>, obj: &Value, name: &str) -> EResult {
    match attr_get_opt(interp, obj, name)? {
        Some(v) => Ok(v),
        // T2 fix: frame-protocol attrs must never AttributeError on ANY
        // object - frame walkers read frame.f_back / frame.f_code during
        // exception unwinds; a miss REPLACES the in-flight exception
        // (pytest Skipped) and derails the runtest protocol.
        None if name == "f_back" && matches!(obj, Value::Obj(_)) => Ok(Value::None),
        None if name == "f_code" && matches!(obj, Value::Obj(_)) => {
            let code = ap_obj::PyObject::new(ap_obj::ObjKind::Namespace, "<code>");
            {
                let mut cattrs = code.attrs.borrow_mut();
                let _ = cattrs.set(
                    Value::Str(PyString::new("co_filename")),
                    Value::Str(PyString::new("<antipythony>")),
                );
                let oname = match obj {
                    Value::Obj(o) => o.name.clone(),
                    _ => "<anon>".to_string(),
                };
                let _ = cattrs.set(
                    Value::Str(PyString::new("co_name")),
                    Value::Str(PyString::new(oname)),
                );
                // Wave-57: match the primary code stub's surface —
                // co_positions must resolve to a callable here too,
                // else `tb_frame.f_code.co_positions()` call-sites see
                // None and our runtime raises "'NoneType' object is not
                // callable" mid-unwind (twisted/pytest traceback walks).
                let _ = cattrs.set(
                    Value::Str(PyString::new("co_positions")),
                    Value::Obj(ap_obj::PyObject::new(
                        ap_obj::ObjKind::BuiltinFn,
                        "<code>.co_positions",
                    )),
                );
                let _ = cattrs.set(
                    Value::Str(PyString::new("co_firstlineno")),
                    Value::Int(ap_obj::Int::from_i64(1)),
                );
            }
            Ok(Value::Obj(code))
        }
        None => Err(raise(
            "AttributeError",
            format!("'{}' object has no attribute '{}'", type_name_of(obj), name),
        )),
    }
}

/// The display type name used in attribute errors (class name for
/// instances; exceptions report their class name).
fn type_name_of(v: &Value) -> String {
    match v {
        Value::Obj(o) if o.kind == ap_obj::ObjKind::UserInstance => o.name.clone(),
        Value::Obj(o) if o.kind == ap_obj::ObjKind::Exception => o.name.clone(),
        other => other.type_name().to_string(),
    }
}

/// Option-form attribute read (for hasattr / import surfaces).
pub fn attr_get_opt(
    interp: &Rc<Interpreter>,
    obj: &Value,
    name: &str,
) -> Result<Option<Value>, Flow> {
    match obj {
        Value::Obj(o) if o.kind == ap_obj::ObjKind::UserInstance => {
            let t = match o.type_ref.borrow().as_ref().and_then(|w| w.upgrade()) {
                Some(t) => t,
                // W112: dead type_ref (dropped weakref) - the instance
                // keeps its attrs; consult them before giving up so
                // slot/attr reads survive class-weakref drops in
                // long pytest flows (unraisable collector family).
                None => {
                    if let Some(v) = o
                        .attrs
                        .borrow()
                        .get(&Value::Str(PyString::new(name)))
                    {
                        return Ok(Some(v));
                    }
                    return Ok(None);
                }
            };
            instance_attr(interp, obj, o, &t, name).map(Some)
        }
        Value::Obj(o) if o.kind == ap_obj::ObjKind::UserClass => {
            let Some(t) = o.class_ref.borrow().clone() else {
                return Ok(None);
            };
            class_attr_get(interp, obj, &t, name).map(Some)
        }
        // W110 (T2): interned builtin Type objects (dict.__bases__,
        // dict.__init__, ...) have no class_ref payload but still answer
        // through class_attr_get — synthesize a native TypeRef whose MRO
        // ends in object (dict/int/etc bases are object in CPython too).
        Value::Obj(o) if o.kind == ap_obj::ObjKind::Type => {
            let obj_t = object_class(interp);
            let t = match TypeObject::create(
                o.name.clone(),
                o.name.clone(),
                vec![obj_t.clone()],
                PyDict::new(),
                TypeFlags::default(),
                None,
            ) {
                Ok(t) => t,
                Err(_) => return Ok(None),
            };
            let cls_v = ap_obj::classes::make_class_value(t.clone());
            match class_attr_get(interp, &cls_v, &t, name) {
                Ok(v) => Ok(Some(v)),
                Err(_) => Ok(None),
            }
        }
        // Wave-58: stream-subclass instances (CaptureIO(TextIOWrapper) &
        // friends) are Opaque carrying a user class_ref. Resolve PYTHON
        // methods from that class FIRST (subclass overrides win —
        // CaptureIO.getvalue must beat the native TextIOWrapper one);
        // misses fall through to the native stream surface below.
        Value::Obj(o) if o.kind == ap_obj::ObjKind::Opaque && o.class_ref.borrow().is_some() => {
            let t = o.class_ref.borrow().clone().unwrap();
            match instance_attr(interp, obj, o, &t, name) {
                Ok(v) => Ok(Some(v)),
                Err(Flow::Raise(e)) if e.class == "AttributeError" => Ok(None),
                Err(other) => Err(other),
            }
        }
        // W112: dict-backed legacy instances (attrs["__ap_dict__"],
        // kind != UserInstance) answer attr reads from the backing dict
        // before falling through - the 'object'-receiver AttributeError
        // family in the pytest unraisable collector flow. Also consult
        // the receiver's own attrs first: slot writes land in o.attrs,
        // and a class-receiver read (self mis-bound to the class in
        // wrapper teardown) must find them.
        Value::Obj(o) => {
            if let Some(v) = o.attrs.borrow().get(&Value::Str(PyString::new(name))) {
                return Ok(Some(v));
            }
            if let Some(Value::Dict(d)) =
                o.attrs.borrow().get(&Value::Str(PyString::new("__ap_dict__")))
            {
                if let Some(v) = d.get(&Value::Str(PyString::new(name.to_string()))) {
                    return Ok(Some(v));
                }
            }
            Ok(None)
        }
        _ => Ok(None),
    }
}

/// S-03-09 on an instance: MRO data descriptor -> instance dict ->
/// non-data descriptor __get__ -> __getattr__ -> AttributeError.
fn instance_attr(
    interp: &Rc<Interpreter>,
    obj: &Value,
    o: &Gc<ap_obj::PyObject>,
    t: &Rc<TypeRef>,
    name: &str,
) -> EResult {
    // Dunder bookkeeping names answer from the class before everything.
    if name == "__class__" {
        return Ok(class_value_for(t));
    }
    if name == "__dict__" {
        if t.has_instance_dict() {
            // vars(obj) is the LIVE dict (mutations visible), not a copy.
            return Ok(Value::Dict(instance_dict_handle(o)));
        }
        return Err(raise(
            "AttributeError",
            format!("'{}' object has no attribute '__dict__'", t.name),
        ));
    }
    // Slot members: storage IS the instance dict slot (member descriptors
    // materialize only for introspection; reads/writes are plain).
    if let Some(v) = o.attrs.borrow().get(&Value::Str(PyString::new(name))) {
        if t.is_slot(name) {
            return Ok(v);
        }
    }
    // T2 (pytest exit codes): SystemExit instances answer .code from the
    // constructor arg (CPython: SystemExit() -> None, SystemExit(n) -> n).
    if t.flags.is_exception && name == "code" && t.name == "SystemExit" {
        let argvals: Vec<Value> = match o.attrs.borrow().get(&Value::Str(PyString::new("args"))) {
            Some(Value::Tuple(t)) => t.0.clone(),
            _ => o.args.borrow().clone(),
        };
        return Ok(argvals.into_iter().next().unwrap_or(Value::None));
    }
    // T1-d (twisted waker wakeUp): OSError instances answer .errno/
    // .strerror/.filename from the constructor args (errno first).
    if t.flags.is_exception
        && matches!(
            name,
            "errno" | "strerror" | "filename" | " characters_written"
        )
    {
        let name2 = name.trim().to_string();
        let idx = match name2.as_str() {
            "errno" => 0usize,
            "strerror" => 1,
            _ => 2,
        };
        let os_family = matches!(
            t.name.as_str(),
            "OSError"
                | "ConnectionError"
                | "TimeoutError"
                | "BlockingIOError"
                | "InterruptedError"
                | "PermissionError"
                | "FileNotFoundError"
                | "FileExistsError"
                | "NotADirectoryError"
                | "IsADirectoryError"
                | "ProcessLookupError"
                | "ConnectionAbortedError"
                | "ConnectionRefusedError"
                | "ConnectionResetError"
        );
        if os_family {
            // Prefer the instance-dict 'args' (set by BaseException init);
            // the payload o.args may be empty for UserInstance exceptions.
            let argvals: Vec<Value> = match o.attrs.borrow().get(&Value::Str(PyString::new("args")))
            {
                Some(Value::Tuple(t)) => t.0.clone(),
                _ => o.args.borrow().clone(),
            };
            if let Some(v) = argvals.get(idx) {
                return Ok(v.clone());
            }
            return Ok(Value::None);
        }
    }
    // 1. First MRO entry: data descriptor wins over the instance dict.
    if let Some((entry, _owner)) = t.lookup_with_owner(name) {
        if is_data_descriptor(interp, &entry) && !t.is_slot(name) {
            return descr_get(interp, &entry, obj, &class_value_for(t));
        }
    }
    // 2. Instance dict.
    if let Some(v) = o.attrs.borrow().get(&Value::Str(PyString::new(name))) {
        return Ok(v);
    }
    // 3. Non-data descriptor / plain class attribute. Plain functions bind
    // as methods with the runtime instance (S-03-21/30: type(self) binding);
    // the DEFINING class rides the binding (zero-arg super start, S-03-18).
    if let Some((entry, owner)) = t.lookup_with_owner(name) {
        match descriptor_kind(interp, &entry) {
            Some(DescrKind::ClassMethod) => {
                // c.cm -> the CLS-bound method (cls = type(self), S-03-21).
                if let Value::Obj(o) = &entry {
                    let f = o.func.borrow().clone().unwrap_or(Value::None);
                    let cls_v = class_value_for(t);
                    return Ok(bind_function_owned(&f, cls_v, Some(&owner)));
                }
                return Ok(entry);
            }
            Some(kind) => {
                let _ = kind;
                // 02.03b-fix: property.__doc__ (dis.py decorates the
                // namedtuple class fields' docstrings).
                if name == "__doc__"
                    && matches!(&entry, Value::Obj(d) if d.kind == ap_obj::ObjKind::Descriptor)
                {
                    return Ok(Value::None);
                }
                // 02.03b-fix: slot members on an UNSET slot must raise
                // AttributeError (try/except AttributeError lazy-init in
                // pathlib relies on it); the member descriptor has no real
                // fget and descr_get would call None.
                if t.is_slot(name) {
                    if std::env::var("APX_TBDBG").is_ok() {
                        let dict_keys: Vec<String> = t
                            .dict
                            .items()
                            .iter()
                            .map(|(k, _)| match k {
                                Value::Str(s2) => s2.as_str().to_string(),
                                _ => format!("{:?}", k).chars().take(20).collect(),
                            })
                            .collect();
                        eprintln!(
                            "[APSLOT-MISS] t.name={} t.qualname={} name={} dict-keys={:?}",
                            t.name,
                            t.qualname,
                            name,
                            dict_keys
                        );
                    }
                    return Err(raise(
                        "AttributeError",
                        format!("'{}' object has no attribute '{}'", t.name, name),
                    ));
                }
                return descr_get(interp, &entry, obj, &class_value_for(t));
            }
            None => {
                if is_user_function(&entry) {
                    return Ok(bind_function_owned(&entry, obj.clone(), Some(&owner)));
                }
                return Ok(entry);
            }
        }
    }
    // 02.04a (interp bug #6): exceptions carry __traceback__ (types.py
    // probes it at import: TracebackType = type(exc.__traceback__)).
    if t.flags.is_exception && name == "__traceback__" {
        return Ok(traceback_value(interp));
    }
    // T1-d (twisted _ModuleProxy): user-defined __getattribute__ on the
    // instance's class intercepts EVERY attr read (CPython data model).
    // object's own __getattribute__ (inherited) does NOT intercept.
    if name != "__getattribute__" && name != "__dict__" && name != "__class__" {
        if let Some((ga, owner)) = t.lookup_with_owner("__getattribute__") {
            let defined_on_self = owner.name != "object";
            if defined_on_self {
                return call_value(
                    interp,
                    &module_frame(interp),
                    &ga,
                    &[obj.clone(), Value::Str(PyString::new(name))],
                    &[],
                );
            }
        }
    }
    // 4. __getattr__ fallback (S-03-09).
    if let Some((ga, _)) = t.lookup_with_owner("__getattr__") {
        return call_value(
            interp,
            &module_frame(interp),
            &ga,
            &[obj.clone(), Value::Str(PyString::new(name))],
            &[],
        );
    }
    // 02.04a (interp bug #4b): object's virtual methods answer on plain
    // instances too (types.py: MethodWrapperType = type(object().__str__)).
    if t.name == "object"
        && matches!(
            name,
            "__init__" | "__new__" | "__str__" | "__repr__" | "__eq__" | "__hash__"
                // T1-d (twisted _deprecateAttribute): object.__setattr__ /
                // object.__getattribute__ read off the object class.
                | "__setattr__" | "__getattribute__"
        )
    {
        let f = ap_obj::PyObject::new(
            ap_obj::ObjKind::BuiltinFn,
            &format!("__object_virtual_{name}__"),
        );
        return Ok(Value::Obj(f));
    }
    // T2 (typing NamedTupleMeta: `nm_tpl.__init_subclass__()`): the implicit
    // subclass hook is readable (and callable) on every class — v0 default
    // is a no-op marker.
    if matches!(name, "__init_subclass__" | "__subclasshook__") {
        let f = ap_obj::PyObject::new(ap_obj::ObjKind::BuiltinFn, &format!("object.{name}"));
        return Ok(Value::Obj(f));
    }
    // T2 (contextlib _BaseExitStack.__exit__): exception-chain dunders
    // must answer on every exception-typed value, defaulting to None.
    if matches!(name, "__context__" | "__cause__" | "__suppress_context__") {
        return Ok(Value::None);
    }
    if name == "_rawentry" && std::env::var("APX_TBDBG").is_ok() {
        let bt = std::backtrace::Backtrace::force_capture().to_string();
        let interesting: Vec<&str> = bt
            .lines()
            .filter(|l| l.contains("ap_run") || l.contains("site-packages"))
            .take(10)
            .collect();
        eprintln!(
            "[APRAW] receiver={} name={}\n{}",
            t.name,
            name,
            interesting.join("\n")
        );
    }
    Err(raise(
        "AttributeError",
        format!("'{}' object has no attribute '{}'", t.name, name),
    ))
}

/// The class VALUE for a payload reached via an MRO walk (the back-pointer
/// is set by make_class_value at creation).
/// Public wrapper: the registered class Value for a payload.
pub fn class_value_for_public(t: &Rc<TypeRef>) -> Value {
    class_value_for(t)
}

fn class_value_for(t: &Rc<TypeRef>) -> Value {
    // Two-step: the read borrow must DROP before make_class_value takes
    // the write borrow (same RefCell).
    let existing = t.class_value.borrow().clone();
    existing.unwrap_or_else(|| ap_obj::classes::make_class_value(t.clone()))
}

/// `C.name` protocol (S-03-12): type slots -> MRO data descriptor
/// __get__(None, C) -> class dict -> metaclass fallback.
fn class_attr_get(interp: &Rc<Interpreter>, cls_v: &Value, t: &Rc<TypeRef>, name: &str) -> EResult {
    // type's own slots resolve FIRST (S-03-12: __name__, __bases__,
    // __mro__, __qualname__, __dict__ ...).
    match name {
        "__name__" => return Ok(Value::Str(PyString::new(t.name.clone()))),
        "__qualname__" => return Ok(Value::Str(PyString::new(t.qualname.clone()))),
        "__module__" => {
            // 02.08-full p2: stdlib-exception classes report their home
            // module (re.error.__module__ == 're', json.JSONDecodeError 'json').
            const STDLIB_EXC_MODULES: &[(&str, &str)] = &[("PatternError", "re")];
            return Ok(t
                .dict
                .get(&Value::Str(PyString::new("__module__")))
                .unwrap_or_else(|| {
                    Value::Str(PyString::new(
                        STDLIB_EXC_MODULES
                            .iter()
                            .find(|(n, _)| *n == t.name)
                            .map(|(_, m)| m.to_string())
                            .unwrap_or_else(|| ap_obj::MODULE_NAME.to_string()),
                    ))
                }));
        }
        "__bases__" => {
            let mut items: Vec<Value> = t.bases.iter().map(class_value_for).collect();
            if items.is_empty() && t.name != "object" {
                // CPython: BaseException.__bases__ == (<class 'object'>,).
                items.push(class_value_for(&object_class(interp)));
            }
            return Ok(Value::Tuple(Gc::new(Tuple(items))));
        }
        "__mro__" => {
            let items: Vec<Value> = t.mro.borrow().iter().map(class_value_for).collect();
            return Ok(Value::Tuple(Gc::new(Tuple(items))));
        }
        "__dict__" => {
            // mappingproxy façade: reads pass, writes raise (S-03-04).
            return Ok(Value::Dict(t.dict.clone()));
        }
        "__class__" => return Ok(type_class_value(interp)),
        "__subclasses__" => {
            // Bound builtin: list direct subclasses (S-03-29/e002 surface);
            // the call site (interp) reads the parked owner name.
            let f = ap_obj::PyObject::new(ap_obj::ObjKind::BuiltinFn, "__subclasses__");
            *f.slice_parts.borrow_mut() = vec![cls_v.clone()];
            return Ok(Value::Obj(f));
        }
        _ => {}
    }
    // 1. MRO data descriptor __get__(None, C).
    if let Some((entry, _)) = t.lookup_with_owner(name) {
        if is_data_descriptor(interp, &entry) {
            return descr_get(interp, &entry, &Value::None, cls_v);
        }
    }
    // 2. Plain class-dict entry: non-data descriptors invoke
    //    __get__(None, C); functions stay unbound (S-03-12(3)).
    if let Some((entry, _)) = t.lookup_with_owner(name) {
        return match descriptor_kind(interp, &entry) {
            Some(DescrKind::StaticMethod) => match &entry {
                Value::Obj(o) => Ok(o.func.borrow().clone().unwrap_or(Value::None)),
                _ => Ok(entry),
            },
            Some(DescrKind::ClassMethod) => {
                // C.cm -> cls-bound method (S-03-21: same cls on C and C());
                // the defining class rides for zero-arg super (c023).
                if let Value::Obj(o) = &entry {
                    let f = o.func.borrow().clone().unwrap_or(Value::None);
                    let owner = t.lookup_with_owner(name).map(|(_, o)| o);
                    return Ok(bind_function_owned(&f, cls_v.clone(), owner.as_ref()));
                }
                Ok(entry)
            }
            Some(_) => descr_get(interp, &entry, &Value::None, cls_v),
            None => Ok(entry),
        };
    }
    // 3. Metaclass attribute lookup (S-03-12(4)): first mro(type(C)) hit,
    //    then metaclass __getattr__.
    if let Some(meta) = t
        .metaclass
        .borrow()
        .clone()
        .or_else(|| Some(type_class(interp)))
    {
        if let Some((mv, owner)) = meta.lookup_with_owner(name) {
            // 02.04a (interp bug #16): metaclass-level plain methods read on
            // the class (ABCMeta.register via Hashable.register) bind the
            // CLASS as the receiver, like CPython method descriptors.
            // 02.08-c (zope.interface): `Base.__new__(cls)` where Base's
            // MRO has no own __new__ resolves to object.__new__ in CPython
            // (the metatype is consulted only for names absent from the
            // MRO) — the metaclass __new__ must NOT win the lookup.
            if name == "__new__" {
                let mro_len = t.mro.borrow().len();
                let mro_has_own_new = t
                    .mro
                    .borrow()
                    .iter()
                    .take(mro_len.saturating_sub(1))
                    .any(|c| c.dict.get(&Value::Str(PyString::new("__new__"))).is_some());
                if !mro_has_own_new {
                    let f = ap_obj::PyObject::new(ap_obj::ObjKind::BuiltinFn, "__object_new2__");
                    return Ok(Value::Obj(f));
                }
            }
            if is_user_function(&mv) {
                return Ok(bind_function_owned(&mv, cls_v.clone(), Some(&owner)));
            }
            // 02.03b-fix: metaclass-level PROPERTIES invoke on the class
            // (EnumType.__members__: cls.__members__ returns the member
            // mapping; returning the raw descriptor made len(cls.__members__)
            // die with "object of type 'object' has no len()").
            let is_prop = matches!(&mv, Value::Obj(d)
                if d.kind == ap_obj::ObjKind::Descriptor
                    && d.descr_kind.borrow().as_deref() == Some("property"));
            if is_prop {
                let meta_cls_v = class_value_for(&meta);
                return descr_get(interp, &mv, cls_v, &meta_cls_v);
            }
            // 02.08-c (zope.interface): `Base.__new__(cls)` where Base's
            // MRO has no own __new__ must resolve to object.__new__ (found
            // in the MRO in CPython), NOT the metaclass __new__ (CPython
            // consults the metatype only for names absent from the MRO).
            // Same for the type.__new__ marker case below.
            if name == "__new__" {
                let mro_has_own_new = t
                    .mro
                    .borrow()
                    .iter()
                    .take(t.mro.borrow().len().saturating_sub(1))
                    .any(|c| c.dict.get(&Value::Str(PyString::new("__new__"))).is_some());
                if !mro_has_own_new {
                    let f = ap_obj::PyObject::new(ap_obj::ObjKind::BuiltinFn, "__object_new2__");
                    return Ok(Value::Obj(f));
                }
            }
            // 02.03b: C.__new__ on a user class (enum member creation:
            // enum_class._new_member_(enum_class, ...)) must ALLOCATE an
            // instance of C — the metaclass type.__new__ marker would try
            // to build a CLASS. The __object_new__ marker allocates the
            // parked subject.
            if name == "__new__"
                && matches!(&mv, Value::Obj(mo) if mo.kind == ap_obj::ObjKind::BuiltinFn && mo.name == "type.__new__")
            {
                // 02.03b: object.__new__(enum_class) — park NOTHING; the
                // call site passes the receiving class as args[0] and the
                // handler allocates THAT class (getattr(obj, '__new__')
                // during enum._find_new_ must not freeze `object` as the
                // subject).
                let f = ap_obj::PyObject::new(ap_obj::ObjKind::BuiltinFn, "__object_new2__");
                return Ok(Value::Obj(f));
            }
            return Ok(mv);
        }
        if let Some(ga) = meta.lookup("__getattr__") {
            return call_value(
                interp,
                &module_frame(interp),
                &ga,
                &[cls_v.clone(), Value::Str(PyString::new(name))],
                &[],
            );
        }
    }
    // 02.04: inspect.isabstract probes type.__flags__ (int bit mask); 0 =
    // no TPFLAGS — every v1 type is concrete.
    if name == "__flags__" {
        return Ok(Value::Int(ap_obj::Int::from_i64(0)));
    }
    // 02.04a (interp bug #4, vendored-stdlib stress): object's virtual
    // methods answer at class level (types.py reads object.__init__ and
    // object().__str__ for WrapperDescriptorType etc).
    if matches!(
        name,
        "__init__"
            | "__new__"
            | "__str__"
            | "__repr__"
            | "__eq__"
            // T1 (http.cookies Morsel: `__ne__ = object.__ne__`): __ne__ joins
            // the object-virtual allowlist.
            | "__ne__"
            | "__hash__"
            | "__doc__"
            | "__format__"
            | "__reduce_ex__"
            | "__sizeof__"
            | "__dir__"
            | "__getattribute__"
            | "__setattr__"
            | "__delattr__"
            | "__reduce__"
            // 02.03b-fix: _py_abc.__subclasscheck__ probes
            // cls.__subclasshook__ (object's classmethod default: None).
            | "__subclasshook__"
    ) {
        if t.mro.borrow().iter().any(|m| m.name == "object") {
            if name == "__doc__" {
                return Ok(Value::None);
            }
            if name == "__subclasshook__" {
                // object.__subclasshook__ is a classmethod returning
                // NotImplemented (the _py_abc.__subclasscheck__ probe
                // compares `ok is not NotImplemented`). Marker: the call
                // site ignores its args and yields the singleton.
                let f = ap_obj::PyObject::new(
                    ap_obj::ObjKind::BuiltinFn,
                    "__object_virtual___subclasshook____",
                );
                return Ok(Value::Obj(f));
            }
            let f = ap_obj::PyObject::new(
                ap_obj::ObjKind::BuiltinFn,
                &format!("__object_virtual_{name}__"),
            );
            return Ok(Value::Obj(f));
        }
    }
    // 02.04a (interp bug #4c): types.py reads FunctionType.__code__ and
    // FunctionType.__globals__ at import (GetSet/MemberDescriptorType).
    if t.name == "function" && matches!(name, "__code__" | "__globals__") {
        let stub = ap_obj::PyObject::new(ap_obj::ObjKind::Namespace, "<stub>");
        return Ok(Value::Obj(stub));
    }
    // T2 (typing NamedTupleMeta `nm_tpl.__init_subclass__()`): the implicit
    // subclass hook is readable (and callable) on every class — v0 default
    // is a no-op marker. __subclasshook__ rides the same default.
    if matches!(name, "__init_subclass__" | "__subclasshook__") {
        let f = ap_obj::PyObject::new(ap_obj::ObjKind::BuiltinFn, &format!("object.{name}"));
        return Ok(Value::Obj(f));
    }
    Err(raise(
        "AttributeError",
        format!("type object '{}' has no attribute '{}'", t.name, name),
    ))
}

fn type_class_value(interp: &Interpreter) -> Value {
    let t = type_class(interp);
    class_value_for(&t)
}

// ===========================================================================
// Attribute SET / DELETE (S-03-10/11)
// ===========================================================================

pub fn attr_set_protocol(
    interp: &Rc<Interpreter>,
    obj: &Value,
    name: &str,
    v: Value,
) -> Result<(), Flow> {
    // 02.03b: attribute writes on builtin str/int/bytes values (enum member
    // creation: `member._value_ = value` where member is a str from
    // str.__new__) — v0 value model has no per-value storage; the write is
    // a documented no-op.
    if matches!(
        obj,
        Value::Str(_) | Value::Int(_) | Value::Bool(_) | Value::Bytes(_)
    ) {
        return Ok(());
    }
    // T1-d (attrs closure rewrite): cell objects with cell_contents writes
    // update the LIVE closure cell via the CELL_BACKING registry.
    if let Value::Obj(o) = obj {
        if o.kind == ap_obj::ObjKind::Namespace && o.name == "cell" && name == "cell_contents" {
            let key = std::rc::Rc::as_ptr(o) as usize;
            let live = crate::interp::CELL_BACKING.with(|reg| reg.borrow().get(&key).cloned());
            if let Some(cell) = live {
                if matches!(v, Value::None) {
                    *cell.borrow_mut() = None;
                } else {
                    *cell.borrow_mut() = Some(v);
                }
                return Ok(());
            }
        }
    }
    // T1 (scrapy fix pass): class-attribute writes on UserClasses
    // (typing: `cls.__protocol_attrs__ = ...` inside __init_subclass__).
    // Land in the class dict so MRO lookups see them.
    if let Value::Obj(o) = obj {
        if o.kind == ap_obj::ObjKind::UserClass {
            let t = o
                .class_ref
                .borrow()
                .clone()
                .or_else(|| o.type_ref.borrow().as_ref().and_then(|w| w.upgrade()));
            if let Some(t) = t {
                let _ = t.dict.set(Value::Str(PyString::new(name.to_string())), v);
                return Ok(());
            }
        }
    }
    // 02.03b-fix: interned-type attr writes (ast.py: Tuple.dims = property)
    // park on the PyObject; attr_get's Type arm reads them back.
    if let Value::Obj(o) = obj {
        if o.kind == ap_obj::ObjKind::Type {
            let _ = o
                .attrs
                .borrow_mut()
                .set(Value::Str(PyString::new(name.to_string())), v);
            return Ok(());
        }
    }
    match obj {
        Value::Obj(o) if o.kind == ap_obj::ObjKind::UserInstance => {
            let t = o
                .type_ref
                .borrow()
                .as_ref()
                .and_then(|w| w.upgrade())
                .ok_or_else(|| raise("SystemError", "instance without class"))?;
            // __setattr__ override wins (S-03-11 last clause).
            if let Some((sa, owner)) = t.lookup_with_owner("__setattr__") {
                // The default object.__setattr__ is not materialized; only
                // user overrides dispatch here.
                if is_user_function(&sa) {
                    // T1 (scrapy fix pass): stage the method super context so
                    // zero-arg super() inside __setattr__ works (typing.py's
                    // _BaseGenericAlias.__setattr__ calls super().__setattr__).
                    *interp.pending_super_ctx.borrow_mut() = Some((owner, obj.clone(), false));
                    let r = call_value(
                        interp,
                        &module_frame(interp),
                        &sa,
                        &[obj.clone(), Value::Str(PyString::new(name)), v],
                        &[],
                    )
                    .map(|_| ());
                    return r;
                }
            }
            // Slots with no dict: only declared names settable (S-03-13).
            if !t.has_instance_dict() && !t.is_slot(name) {
                return Err(raise(
                    "AttributeError",
                    format!(
                        "'{}' object has no attribute '{}' and no __dict__ for setting new attributes",
                        t.name, name
                    ),
                ));
            }
            // Slot member: direct instance-dict write (member descriptors
            // are introspection-only).
            if t.is_slot(name) {
                return o
                    .attrs
                    .borrow_mut()
                    .set(Value::Str(PyString::new(name)), v)
                    .map_err(Flow::from_py);
            }
            // Data descriptor __set__ (instance dict NOT consulted).
            if let Some((entry, _)) = t.lookup_with_owner(name) {
                if is_data_descriptor(interp, &entry) {
                    return descr_set(interp, &entry, obj, v, &t.name, name);
                }
            }
            o.attrs
                .borrow_mut()
                .set(Value::Str(PyString::new(name)), v)
                .map_err(Flow::from_py)
        }
        // 02.03b: module-attribute writes (warnings: `_wm._filters_version
        // += 1` where _wm is the warnings module object).
        Value::Obj(o) if o.kind == ap_obj::ObjKind::Module => {
            let ns = o.module_ns.borrow();
            if let Some(ns) = &*ns {
                ns.set(Value::Str(PyString::new(name)), v)
                    .map_err(Flow::from_py)
            } else {
                Err(raise(
                    "AttributeError",
                    format!("module '{}' has no namespace", o.name),
                ))
            }
        }
        Value::Obj(o) if o.kind == ap_obj::ObjKind::UserClass => {
            // Class-attribute write: plain dict set (C.x = 5 succeeds even
            // over a property — S-03-10 observed).
            let Some(t) = o.class_ref.borrow().clone() else {
                return Err(raise("SystemError", "class without payload"));
            };
            t.dict
                .set(Value::Str(PyString::new(name)), v)
                .map_err(Flow::from_py)
        }
        // 02.04a (interp bug #1b): functions accept arbitrary attribute
        // writes (CPython function.__dict__) — vendored stdlib does
        // `splitext.__doc__ = genericpath._splitext.__doc__` at import.
        // T1-d: BuiltinFn markers too (typing._make_nmtuple:
        // `nm_tpl.__new__.__annotate__ = fn` — object.__new__ is a marker).
        Value::Obj(o)
            if o.kind == ap_obj::ObjKind::Function || o.kind == ap_obj::ObjKind::BuiltinFn =>
        {
            o.attrs
                .borrow_mut()
                .set(Value::Str(PyString::new(name)), v)
                .map_err(Flow::from_py)
        }
        // 02.03b-fix: descriptor attr writes (dis.py decorates the namedtuple
        // fields: _Instruction.opname.__doc__ = "...") park on the object;
        // the descriptor __doc__ read arm already prefers o.attrs.
        Value::Obj(o) if o.kind == ap_obj::ObjKind::Descriptor => o
            .attrs
            .borrow_mut()
            .set(Value::Str(PyString::new(name)), v)
            .map_err(Flow::from_py),
        // 02.07: threading.local per-thread namespace writes; Thread
        // name/daemon writes route through the native surface.
        Value::Obj(o)
            if o.kind == ap_obj::ObjKind::Opaque
                && matches!(o.name.as_str(), "_local" | "thread") =>
        {
            crate::threading_call::native_thread_attr_set(obj, name, v)
        }
        // 02.08-c: lxml element instance writes (el.text = ..., el.tail = ...).
        Value::Obj(o)
            if o.kind == ap_obj::ObjKind::Opaque
                && (o.name == "lxml.etree._Element" || o.name == "lxml.html.HtmlElement") =>
        {
            crate::lxml::elem_attr_set(obj, name, v)
        }
        // T2 (contextlib/pluggy exception chaining): native exception
        // values accept chain-dunder writes (__context__/__cause__/
        // __suppress_context__/__traceback__) — stored in attrs so the
        // matching read arms (interp 9992 family) see them.
        Value::Obj(o) if o.kind == ap_obj::ObjKind::Exception => {
            if matches!(
                name,
                "__context__" | "__cause__" | "__suppress_context__" | "__traceback__"
            ) || name.starts_with("__ap_")
            {
                // W112: allow runtime bookkeeping dunders (__ap_tb__ and
                // friends) on legacy Exception-kind instances. CPython
                // exceptions carry a __dict__ and accept any attribute;
                // the strict allow-list here silently broke the tb
                // carry-back (exception_instance wrote __ap_tb__, the
                // Python-visible setattr/read path rejected it).
                let _ = o
                    .attrs
                    .borrow_mut()
                    .set(Value::Str(PyString::new(name.to_string())), v);
                return Ok(());
            }
            Err(raise(
                "AttributeError",
                format!("'{}' object has no attribute '{}'", o.name, name),
            ))
        }
        other => Err(raise(
            "AttributeError",
            format!("'{}' object has no attribute '{}'", other.type_name(), name),
        )),
    }
}

fn is_user_function(v: &Value) -> bool {
    matches!(v, Value::Obj(o) if o.kind == ap_obj::ObjKind::Function)
}

pub fn attr_del_protocol(interp: &Rc<Interpreter>, obj: &Value, name: &str) -> Result<(), Flow> {
    match obj {
        Value::Obj(o) if o.kind == ap_obj::ObjKind::UserInstance => {
            let t = o
                .type_ref
                .borrow()
                .as_ref()
                .and_then(|w| w.upgrade())
                .ok_or_else(|| raise("SystemError", "instance without class"))?;
            // __delattr__ override (S-03-11).
            if let Some((da, _)) = t.lookup_with_owner("__delattr__") {
                if is_user_function(&da) {
                    return call_value(
                        interp,
                        &module_frame(interp),
                        &da,
                        &[obj.clone(), Value::Str(PyString::new(name))],
                        &[],
                    )
                    .map(|_| ());
                }
            }
            // Data descriptor with __delete__ (S-03-11).
            if let Some((entry, _)) = t.lookup_with_owner(name) {
                if is_data_descriptor(interp, &entry) {
                    let has_del = match &entry {
                        Value::Obj(d) if d.kind == ap_obj::ObjKind::Descriptor => d
                            .dunders
                            .borrow()
                            .contains(&Value::Str(PyString::new("fdel"))),
                        Value::Obj(d) if d.kind == ap_obj::ObjKind::UserInstance => {
                            instance_class(interp, d)
                                .is_some_and(|dt| dt.lookup("__delete__").is_some())
                        }
                        _ => false,
                    };
                    if has_del {
                        return descr_delete(interp, &entry, obj, &t.name, name);
                    }
                    return Err(raise(
                        "AttributeError",
                        format!("property '{name}' of '{}' object has no deleter", t.name),
                    ));
                }
            }
            // Instance dict must hold the name (deleting an INHERITED attr
            // reports the instance class — S-03-11).
            if o.attrs
                .borrow()
                .pop(&Value::Str(PyString::new(name)))
                .is_none()
            {
                return Err(raise(
                    "AttributeError",
                    format!("'{}' object has no attribute '{}'", t.name, name),
                ));
            }
            Ok(())
        }
        Value::Obj(o) if o.kind == ap_obj::ObjKind::UserClass => {
            let Some(t) = o.class_ref.borrow().clone() else {
                return Err(raise("SystemError", "class without payload"));
            };
            if t.dict.pop(&Value::Str(PyString::new(name))).is_none() {
                return Err(raise(
                    "AttributeError",
                    format!("type object '{}' has no attribute '{}'", t.name, name),
                ));
            }
            Ok(())
        }
        // T2 (pytest -> pygments lexers/formatters __init__ swap):
        // `del newmod.newmod` on a ModuleType subclass instance — the module
        // NAMESPACE is the attribute store (setattr on modules routes there),
        // so delattr must unbind from module_ns too. CPython's module.__delattr__
        // raises AttributeError when absent — same message shape as the read
        // path ("module' object has no attribute").
        Value::Obj(o) if o.kind == ap_obj::ObjKind::Module => {
            let ns = o.module_ns.borrow().clone().ok_or_else(|| {
                raise(
                    "AttributeError",
                    format!("'module' object has no attribute '{}'", name),
                )
            })?;
            if ns.pop(&Value::Str(PyString::new(name))).is_none() {
                return Err(raise(
                    "AttributeError",
                    format!("'module' object has no attribute '{}'", name),
                ));
            }
            Ok(())
        }
        other => Err(raise(
            "AttributeError",
            format!("'{}' object has no attribute '{}'", other.type_name(), name),
        )),
    }
}

// ===========================================================================
// Instantiation (S-03-05..08)
// ===========================================================================

/// `C(*args, **kwargs)` for user classes: __new__ -> isinstance check ->
/// __init__ (S-03-06); arity fast-path errors (S-03-07); __init__ returning
/// non-None raises (S-03-08).
pub fn instantiate(
    interp: &Rc<Interpreter>,
    cls: &Value,
    t: &Rc<TypeRef>,
    args: &[Value],
    kwargs: &[(String, Value)],
) -> EResult {
    // Metaclass __call__ override wraps instantiation (S-03-28).
    // 02.03b: forward the constructor args/kwargs (EnumType.__call__(cls,
    // value, ...) — the old call passed only cls, dropping `value`).
    // T2 (pytest NodeMeta._create): a super().__call__ route must SKIP the
    // metaclass override (CPython type.__call__ allocates directly); the
    // marker `_ap_supercall` on the thread side marks that route.
    let skip_meta_call = crate::interp::supercall_flag_take();
    if !skip_meta_call {
        if let Some(meta) = t.metaclass.borrow().clone() {
            if let Some((call, _)) = meta.lookup_with_owner("__call__") {
                if is_user_function(&call) {
                    let mut full = vec![cls.clone()];
                    full.extend_from_slice(args);
                    return call_value(interp, &module_frame(interp), &call, &full, kwargs);
                }
            }
        }
    }
    // 02.03b: 3-arg type(name, bases, dict, **kwds) — dynamic class
    // creation (enum._simple_enum). The `type` payload has no real
    // __new__/__init__ plumbing for the arity-3 form.
    if t.name == "type" && args.len() >= 3 {
        let name = match args.first() {
            Some(Value::Str(s)) => s.as_str().to_string(),
            _ => String::new(),
        };
        let bases_v: Vec<Value> = match args.get(1) {
            Some(Value::Tuple(tup)) => tup.0.clone(),
            Some(Value::List(l)) => l.items().to_vec(),
            _ => vec![],
        };
        let ns = match args.get(2) {
            Some(Value::Dict(d)) => d.clone(),
            _ => PyDict::new(),
        };
        return dynamic_type(interp, &name, &bases_v, ns, kwargs);
    }

    // 02.03b: collections.namedtuple classes (marked __ap_namedtuple_fields__)
    // construct instances positionally into per-field attrs.
    // T1-d: walk the MRO — namedtuple SUBCLASSES (urllib SplitResult) carry
    // the marker on the parent class dict.
    let fields_v_opt = {
        let mut hit = t
            .dict
            .get(&Value::Str(PyString::new("__ap_namedtuple_fields__")));
        if hit.is_none() {
            for m in t.mro.borrow().iter() {
                if let Some(v) = m
                    .dict
                    .get(&Value::Str(PyString::new("__ap_namedtuple_fields__")))
                {
                    hit = Some(v);
                    break;
                }
            }
        }
        hit
    };
    if let Some(fields_v) = fields_v_opt {
        let fields: Vec<String> = match &fields_v {
            Value::List(l) => l
                .items()
                .iter()
                .filter_map(|v| match v {
                    Value::Str(x) => Some(x.as_str().to_string()),
                    _ => None,
                })
                .collect(),
            _ => vec![],
        };
        let obj = alloc_instance(t);
        if let Value::Obj(io) = &obj {
            let kwmap: Vec<(String, Value)> = kwargs.to_vec();
            for (i, f) in fields.iter().enumerate() {
                let v = args
                    .get(i)
                    .cloned()
                    .or_else(|| kwmap.iter().find(|(k, _)| k == f).map(|(_, v)| v.clone()))
                    .unwrap_or(Value::None);
                let _ = io
                    .attrs
                    .borrow_mut()
                    .set(Value::Str(PyString::new(f.clone())), v);
            }
        }
        return Ok(obj);
    }

    // T2 (pytest capture EncodedFile(io.TextIOWrapper)): a class deriving
    // from a native stream type constructs via the native base ctor
    // (CPython TextIOWrapper(buffer, ...)) — marker or MRO base-name check
    // (the base's generic __init__/__new__ markers make the user-override
    // detection unreliable here).
    let stream_base: Option<String> = t
        .dict
        .get(&Value::Str(PyString::new("__ap_stream_base__")))
        .and_then(|v| match v {
            Value::Str(s) => Some(s.as_str().to_string()),
            _ => None,
        })
        .or_else(|| {
            t.mro.borrow().iter().find_map(|m| {
                m.dict
                    .get(&Value::Str(PyString::new("__ap_stream_base__")))
                    .and_then(|v| match v {
                        Value::Str(s) => Some(s.as_str().to_string()),
                        _ => None,
                    })
                    .or_else(|| {
                        if matches!(
                            m.name.as_str(),
                            "TextIOWrapper"
                                | "BytesIO"
                                | "StringIO"
                                | "BufferedReader"
                                | "BufferedWriter"
                                | "FileIO"
                        ) {
                            Some(m.name.clone())
                        } else {
                            None
                        }
                    })
            })
        });

    let new_lookup = t
        .lookup_with_owner("__new__")
        .filter(|(v, _)| !matches!(v, Value::None));
    let init_lookup = t
        .lookup_with_owner("__init__")
        .filter(|(v, _)| !matches!(v, Value::None));
    let init_f = init_lookup.as_ref().map(|(v, _)| v.clone());
    // T2: a python __init__ on a native-stream subclass (CaptureIO
    // (io.TextIOWrapper)) supplies the buffer via super().__init__(buf)
    // AFTER allocation — do NOT run the native ctor with the outer
    // (possibly empty) args first.
    if let Some(sb) = stream_base.as_ref().filter(|_| init_f.is_none()).cloned() {
        let native = builtins::interned_type(interp, &sb);
        return call_value(interp, &module_frame(interp), &native, args, kwargs);
    }
    let new_f = new_lookup.as_ref().map(|(v, _)| v.clone());
    // CPython 3.14 observed (e013): when BOTH __new__ and __init__ are
    // inherited from the same ancestor, the ancestor's __new__ is NOT
    // invoked for the subclass (recorded expected bytes govern).

    // Exception classes: BaseException.__init__ stores args; instances
    // carry the class payload back-ref so except-as / isinstance see the
    // real class (e001/e007).
    if t.flags.is_exception && init_f.as_ref().map_or(true, |f| !is_user_function(f)) {
        let obj = alloc_instance(t);
        {
            let Value::Obj(io) = &obj else { unreachable!() };
            let argv = Value::Tuple(Gc::new(Tuple(args.to_vec())));
            let _ = io
                .attrs
                .borrow_mut()
                .set(Value::Str(PyString::new("args")), argv);
            *io.class_ref.borrow_mut() = Some(t.clone());
            // CPython: BaseExceptionGroup(...) with all-Exception excs
            // derives to ExceptionGroup (e008).
            if t.name == "BaseExceptionGroup" {
                let all_exc = args.get(1).is_some_and(|ex| match ex {
                    Value::List(l) => l.items().iter().all(|e| {
                        class_of_value_here(e)
                            .is_some_and(|c| c.is_subclass(&exception_payload(interp, "Exception")))
                    }),
                    _ => false,
                });
                if all_exc {
                    let eg = interp.exc_classes.borrow().get("ExceptionGroup").cloned();
                    if let Some(eg) = eg {
                        if let Some(egt) = type_of(interp, &eg) {
                            return instantiate(interp, &eg, &egt, args, kwargs);
                        }
                    }
                }
            }
            // ExceptionGroup surface: .message/.exceptions (e008).
            if t.name == "ExceptionGroup" || t.name == "BaseExceptionGroup" {
                if let Some(m) = args.first() {
                    let _ = io
                        .attrs
                        .borrow_mut()
                        .set(Value::Str(PyString::new("message")), m.clone());
                }
                if let Some(ex) = args.get(1) {
                    let _ = io
                        .attrs
                        .borrow_mut()
                        .set(Value::Str(PyString::new("exceptions")), ex.clone());
                }
            }
        }
        return Ok(obj);
    }

    // S-03-07 fast path: object.__new__ + not-overridden __init__ with
    // args -> "C() takes no arguments".
    // 04.01 (placed BEFORE the fast path): ModuleType-derived classes
    // (types.ModuleType = type(sys)) take (name, doc=None) — six.py's
    // Module_six_moves_urllib(types.ModuleType) is constructed at module
    // scope. CPython's ModuleType.__new__ consumes the name; mirror that:
    // allocate the instance, set __name__/__doc__.
    // 02.08-c fix: `type(sys)("name")` DIRECTLY (types.ModuleType("x")) —
    // the interned module-type payload. Same allocation as the subclass path.
    let is_module_type = t.name == "module" || t.mro.borrow().iter().any(|b| b.name == "module");
    if is_module_type && new_f.is_none() && init_f.is_none() {
        // Allocate a REAL module object (six.moves must pass ObjKind::Module
        // checks in the importer's submodule adoption + attr protocol);
        // __name__/__doc__ go into the module NAMESPACE (module attr reads
        // resolve through module_ns, not attrs).
        let io = ap_obj::PyObject::new(
            ap_obj::ObjKind::Module,
            match args.first() {
                Some(Value::Str(name)) => name.as_str().to_string(),
                _ => "module".to_string(),
            },
        );
        let ns = ap_obj::dict::PyDict::new();
        if let Some(Value::Str(name)) = args.first() {
            let _ = ns.set(
                Value::Str(PyString::new("__name__")),
                Value::Str(name.clone()),
            );
        }
        if let Some(doc) = args.get(1) {
            let _ = ns.set(Value::Str(PyString::new("__doc__")), doc.clone());
        }
        *io.type_ref.borrow_mut() = Some(std::rc::Rc::downgrade(t));
        *io.module_ns.borrow_mut() = Some(ns);
        return Ok(Value::Obj(io));
    }
    // T1-d (attr/_make.py _make_attr_tuple_class + twisted
    // _DeprecatedProperty): tuple/list/set/str/dict/property subclasses
    // with NO user __new__/__init__ construct their backing builtin from
    // the args instead of erroring "takes no arguments".
    if new_f.is_none() && init_f.is_none() && (!args.is_empty() || !kwargs.is_empty()) {
        let builtin: Option<String> = t
            .dict
            .get(&Value::Str(PyString::new("__ap_builtin_base__")))
            .and_then(|v| match v {
                Value::Str(s) => Some(s.as_str().to_string()),
                _ => None,
            })
            // W111: enum member classes build via a SYNTHESIZED int/str base
            // (a USER class named "int" without the dict marker). The mro
            // names the builtin chain — detect it here so the member
            // construction builds from the value arg instead of raising
            // "takes no arguments" (http.client.responses .phrase chain).
            .or_else(|| {
                const BUILTIN_NAMES: &[&str] = &[
                    "tuple", "list", "dict", "set", "str", "int", "float",
                    "bytes", "frozenset",
                ];
                t.mro.borrow().iter().find_map(|m| {
                    if BUILTIN_NAMES.contains(&m.name.as_str()) {
                        Some(m.name.clone())
                    } else {
                        None
                    }
                })
            });
        if let Some(builtin) = builtin {
            // T1-d (namedtuple SplitResult(5 args)): ALL-positional args
            // become the tuple payload fields.
            // W110 (T2): enum-style ctor calls int(cls, value) — CPython's
            // int.__new__(cls, value) ignores the cls arg. Drop a leading
            // class-valued arg so int(IE, 1) builds from 1.
            let args_eff: Vec<Value> = match args {
                [a, rest @ ..] if rest.len() == 1 && matches!(a, Value::Obj(ao) if ao.kind == ap_obj::ObjKind::UserClass || ao.kind == ap_obj::ObjKind::Type) => {
                    rest.to_vec()
                }
                other => other.to_vec(),
            };
            let payload = match args_eff.as_slice() {
                [one] => Some(one.clone()),
                [a, b, c, d, e] if !kwargs.iter().any(|_| true) => {
                    Some(Value::Tuple(Gc::new(Tuple(vec![
                        a.clone(),
                        b.clone(),
                        c.clone(),
                        d.clone(),
                        e.clone(),
                    ]))))
                }
                _ => None,
            };
            if let Some(p) = payload {
                let built = match builtin.as_str() {
                    "tuple" => match &p {
                        Value::Tuple(tup) => Some(Value::Tuple(tup.clone())),
                        Value::List(l) => Some(Value::Tuple(Gc::new(Tuple(l.items().to_vec())))),
                        _ => None,
                    },
                    "list" => crate::builtins::builtin_list(&[p.clone()]).ok(),
                    "set" => crate::builtins::call_builtin(interp, "set", &[p.clone()], &[]).ok(),
                    // T2 (pygments `class include(str)` + enum `_inherit`):
                    // str subclasses construct from the single arg like
                    // CPython str(obj).
                    "str" => crate::builtins::call_builtin(interp, "str", &[p.clone()], &[]).ok(),
                    // dict subclass: dict(mapping-or-iterable, **kw).
                    "dict" => crate::builtins::call_builtin(interp, "dict", &[p.clone()], &[]).ok(),
                    // W110 (T2): int subclasses (enum members) construct
                    // from the value arg like CPython int(x).
                    "int" => crate::builtins::call_builtin(interp, "int", &[p.clone()], &[]).ok(),
                    // property(fget): the fget rides the descriptor.
                    "property" => {
                        let o = ap_obj::PyObject::new(ap_obj::ObjKind::Descriptor, "property");
                        *o.descr_kind.borrow_mut() = Some("property".to_string());
                        *o.self_value.borrow_mut() = Some(p.clone());
                        Some(Value::Obj(o))
                    }
                    _ => None,
                };
                if let Some(v) = built {
                    let io = ap_obj::PyObject::new(ap_obj::ObjKind::UserInstance, t.name.clone());
                    *io.type_ref.borrow_mut() = Some(std::rc::Rc::downgrade(t));
                    *io.class_ref.borrow_mut() = Some(t.clone());
                    // T2 (pygments _TokenType): keep a TUPLE-typed backing so
                    // the derived-instance protocol (len/slice/contains/+)
                    // can see the ctor contents; the payload below answers
                    // subscript/iter for the attr-style consumers.
                    if matches!(builtin.as_str(), "tuple" | "list" | "str") {
                        if let Some(items) = match &v {
                            Value::Tuple(tup) => Some(tup.0.clone()),
                            Value::List(l) => Some(l.items().to_vec()),
                            Value::Str(sv) => {
                                // str subclasses park the string under
                                // __ap_str__ so eq/repr/concat normalize.
                                let _ = io.attrs.borrow_mut().set(
                                    Value::Str(PyString::new("__ap_str__")),
                                    Value::Str(sv.clone()),
                                );
                                None
                            }
                            _ => None,
                        } {
                            let backing = if builtin == "tuple" {
                                Value::Tuple(Gc::new(Tuple(items)))
                            } else {
                                Value::List(ap_obj::value::PyList::from_vec(items))
                            };
                            let key = if builtin == "tuple" {
                                "__ap_tuple__"
                            } else {
                                "__ap_list__"
                            };
                            let _ = io
                                .attrs
                                .borrow_mut()
                                .set(Value::Str(PyString::new(key)), backing);
                        }
                    }
                    // Wave-72: dict-derived instances carry their backing
                    // under __ap_dict__ so dict_backing()/pydict arms
                    // (pop/get/items) resolve (logging ConvertingDict).
                    if builtin == "dict" {
                        let _ = io
                            .attrs
                            .borrow_mut()
                            .set(Value::Str(PyString::new("__ap_dict__")), v.clone());
                    }
                    // W111: int-derived instances park the numeric payload
                    // under __ap_int__ — hash(member) == hash(value)
                    // (IntEnum members key dicts by numeric value).
                    if builtin == "int" {
                        let _ = io
                            .attrs
                            .borrow_mut()
                            .set(Value::Str(PyString::new("__ap_int__")), v.clone());
                    }
                    let _ = io
                        .attrs
                        .borrow_mut()
                        .set(Value::Str(PyString::new("__ap_tuple_payload__")), v);
                    return Ok(Value::Obj(io));
                }
            }
            return Err(raise(
                "TypeError",
                format!("{}() takes no arguments", t.name),
            ));
        }
        return Err(raise(
            "TypeError",
            format!("{}() takes no arguments", t.name),
        ));
    }
    // __init__ overridden but __new__ is object's: args reach __new__ and
    // the binding error surfaces from THERE (S-03-07 observed).
    if let Some(nf) = &new_f {
        // Custom __new__: call with (cls, *args).
        let mut new_args: Vec<Value> = vec![cls.clone()];
        new_args.extend_from_slice(args);
        // super() inside __new__ starts after the DEFINING class of
        // __new__ (c003: Base.__new__'s super lands on object). Staged for
        // call_function (its plain-fn sentinel lands first).
        let new_start = new_lookup
            .as_ref()
            .map(|(_, o)| o.clone())
            .unwrap_or_else(|| t.clone());
        *interp.pending_super_ctx.borrow_mut() = Some((new_start, cls.clone(), true));
        let obj = call_value(interp, &module_frame(interp), nf, &new_args, kwargs);
        let obj = qualify_newinit_error(obj, "__new__", &t.name)?;
        // isinstance(obj, cls) decides whether __init__ runs (S-03-06).
        if value_isinstance(interp, &obj, cls)? {
            if let Some(inf) = &init_f {
                let mut init_args: Vec<Value> = vec![obj.clone()];
                init_args.extend_from_slice(args);
                let init_start = init_lookup
                    .as_ref()
                    .map(|(_, o)| o.clone())
                    .unwrap_or_else(|| t.clone());
                *interp.pending_super_ctx.borrow_mut() = Some((init_start, obj.clone(), false));
                let r = call_value(interp, &module_frame(interp), inf, &init_args, kwargs);
                // 02.03b: call_function balances the staged ctx itself; the
                // extra pop here removed the ENCLOSING frame's context
                // (EnumType.__new__'s super() died after a nested
                // EnumDict(...) instantiation).
                let r = qualify_newinit_error(r, "__init__", &t.name)?;
                check_init_return(&r, inf)?;
            }
        }
        return Ok(obj);
    }
    let mut obj = alloc_instance(t);
    // T2 (wave-39, pytest CaptureIO(TextIOWrapper)): a stream-base
    // subclass instance must BE a stream so native stream method
    // dispatch works on it (native_attr keys on ObjKind::Opaque +
    // o.name). Replace the fresh UserInstance with a stream object
    // carrying the user class refs; the user __init__ then runs on it.
    if let Some(sb) = &stream_base {
        let kind = match sb.as_str() {
            "BytesIO" | "StringIO" => "BytesIO",
            _ => "TextIOWrapper",
        };
        if let Value::Obj(old_o) = &obj {
            let fresh = ap_obj::PyObject::new(ap_obj::ObjKind::Opaque, kind.clone());
            *fresh.type_ref.borrow_mut() = old_o.type_ref.borrow().clone();
            let cr = old_o.class_ref.borrow().clone().unwrap_or_else(|| t.clone());
            *fresh.class_ref.borrow_mut() = Some(cr);
            {
                let mut attrs = fresh.attrs.borrow_mut();
                let _ = attrs.set(
                    Value::Str(PyString::new("<buf>".to_string())),
                    Value::Bytes(Gc::new(vec![])),
                );
                let _ = attrs.set(
                    Value::Str(PyString::new("<pos>".to_string())),
                    Value::Int(ap_obj::Int::from_i64(0)),
                );
                let _ = attrs.set(
                    Value::Str(PyString::new("<encoding>".to_string())),
                    Value::Str(PyString::new("utf-8".to_string())),
                );
                let _ = attrs.set(
                    Value::Str(PyString::new("<closed>".to_string())),
                    Value::Bool(false),
                );
            }
            obj = Value::Obj(fresh);
        }
    }
if let Some(inf) = &init_f {
        if is_user_function(inf) {
            let mut a = vec![obj.clone()];
            a.extend_from_slice(args);
            let init_start2 = init_lookup
                .as_ref()
                .map(|(_, o)| o.clone())
                .unwrap_or_else(|| t.clone());
            *interp.pending_super_ctx.borrow_mut() = Some((init_start2, obj.clone(), false));
            let r = call_value(interp, &module_frame(interp), inf, &a, kwargs);
            // 02.03b: call_function balances the staged ctx itself (see the
            // custom-__new__ path note).
            let r = qualify_newinit_error(r, "__init__", &t.name)?;
            check_init_return(&r, inf)?;
        } else if !args.is_empty() || !kwargs.is_empty() {
            // Inherited __init__ that is object's + args: binding error
            // text surfaces from the (virtual) __init__.
            return Err(raise(
                "TypeError",
                format!("{}() takes no arguments", t.name),
            ));
        }
    } else if !args.is_empty() || !kwargs.is_empty() {
        return Err(raise(
            "TypeError",
            format!("{}() takes no arguments", t.name),
        ));
    }
    Ok(obj)
}

fn check_init_return(r: &Value, f: &Value) -> Result<(), Flow> {
    if matches!(r, Value::None) {
        return Ok(());
    }
    let fname = match f {
        Value::Obj(o) => o.name.clone(),
        _ => "__init__".to_string(),
    };
    let _ = fname;
    Err(raise(
        "TypeError",
        format!(
            "__init__() should return None, not '{}'",
            crate::classes::display_type_name(r)
        ),
    ))
}

/// Allocate a bare instance of `t` (dict-enabled classes get their attrs
/// dict via PyObject's default).
pub fn alloc_instance(t: &Rc<TypeRef>) -> Value {
    let o = ap_obj::PyObject::new(ap_obj::ObjKind::UserInstance, t.name.clone());
    *o.type_ref.borrow_mut() = Some(Rc::downgrade(t));
    // 02.03b: dict-subclass instances carry a REAL backing dict in
    // attrs["__ap_dict__"] (EnumDict and friends): item traffic routes
    // through it, attribute traffic through attrs as before.
    if is_dict_derived(t) {
        let _ = o.attrs.borrow_mut().set(
            Value::Str(PyString::new("__ap_dict__")),
            Value::Dict(PyDict::new()),
        );
    }
    if !t.has_instance_dict() {
        // Slots-only: leave attrs empty; writes are rejected before the
        // dict is consulted.
    }
    Value::Obj(o)
}

/// Does this class derive from the builtin `dict`? (02.03b: the synthesized
/// dict base from `class X(dict)` carries the name "dict" and descends from
/// object directly — the marker we key the backing-dict model on.)
pub fn is_dict_derived(t: &Rc<TypeRef>) -> bool {
    t.mro.borrow().iter().any(|m| m.name == "dict")
}

/// The backing dict of a dict-derived instance (None for other values).
pub fn dict_backing(v: &Value) -> Option<Gc<PyDict>> {
    match v {
        Value::Obj(o) if o.kind == ap_obj::ObjKind::UserInstance => o
            .attrs
            .borrow()
            .get(&Value::Str(PyString::new("__ap_dict__")))
            .and_then(|d| match d {
                Value::Dict(d) => Some(d.clone()),
                _ => None,
            }),
        _ => None,
    }
}

/// T1 (scrapy fix pass): the seeded __module__ of a UserClass value (the
/// class dict's __module__ if present).
pub fn userclass_module(v: &Value) -> Option<Value> {
    match v {
        Value::Obj(o) => {
            let t = o.type_ref.borrow().as_ref().and_then(|w| w.upgrade())?;
            t.dict.get(&Value::Str(PyString::new("__module__")))
        }
        _ => None,
    }
}

/// T1 (scrapy fix pass): is this a UserInstance whose class MRO carries a
/// synthesized `list` base (e.g. traceback.StackSummary(list))? The dict
/// twin is is_dict_derived keyed on the class TypeRef.
pub fn is_list_derived_instance(v: &Value) -> bool {
    match v {
        Value::Obj(o) if o.kind == ap_obj::ObjKind::UserInstance => {
            match o.type_ref.borrow().as_ref().and_then(|w| w.upgrade()) {
                Some(t) => t.mro.borrow().iter().any(|m| m.name == "list"),
                None => false,
            }
        }
        _ => false,
    }
}

/// T2 (pytest -> pygments token.py `_TokenType(tuple)`): is this a
/// UserInstance whose class MRO carries a synthesized `tuple` base? CPython
/// tuple subclasses ARE tuples: len/index/slice/iter/contains/eq/`+` all
/// behave tuple-wise. The dict/list twins above key the same model.
pub fn is_tuple_derived_instance(v: &Value) -> bool {
    match v {
        Value::Obj(o) if o.kind == ap_obj::ObjKind::UserInstance => {
            match o.type_ref.borrow().as_ref().and_then(|w| w.upgrade()) {
                Some(t) => t.mro.borrow().iter().any(|m| m.name == "tuple"),
                None => false,
            }
        }
        _ => false,
    }
}

/// The backing tuple of a tuple-derived instance, creating it (empty) on
/// first use. Stored under attrs["__ap_tuple__"] (mirrors list_backing_or_new).
pub fn tuple_backing_or_new(v: &Value) -> Gc<Tuple> {
    if let Value::Obj(o) = v {
        if let Some(t) = o
            .attrs
            .borrow()
            .get(&Value::Str(PyString::new("__ap_tuple__")))
        {
            if let Value::Tuple(t) = t {
                return t.clone();
            }
        }
        let t = Gc::new(Tuple(Vec::new()));
        let _ = o.attrs.borrow_mut().set(
            Value::Str(PyString::new("__ap_tuple__")),
            Value::Tuple(t.clone()),
        );
        return t;
    }
    Gc::new(Tuple(Vec::new()))
}

/// The backing list of a list-derived instance, creating it (empty) on first
/// use. Stored under attrs["__ap_list__"].
pub fn list_backing_or_new(v: &Value) -> Gc<PyList> {
    if let Value::Obj(o) = v {
        if let Some(l) = o
            .attrs
            .borrow()
            .get(&Value::Str(PyString::new("__ap_list__")))
        {
            if let Value::List(l) = l {
                return l.clone();
            }
        }
        let l = PyList::new();
        let _ = o.attrs.borrow_mut().set(
            Value::Str(PyString::new("__ap_list__")),
            Value::List(l.clone()),
        );
        return l;
    }
    PyList::new()
}

/// The display type name for error messages: instances/exceptions report
/// their CLASS name (dynamic), everything else the static type name.
pub fn display_type_name(v: &Value) -> String {
    match v {
        Value::Obj(o)
            if o.kind == ap_obj::ObjKind::UserInstance || o.kind == ap_obj::ObjKind::Exception =>
        {
            o.name.clone()
        }
        other => other.type_name().to_string(),
    }
}

/// A handle to the instance's LIVE attribute dict (vars(x) mutations must
/// be visible through x — the c008 `c.__dict__["x"] = 99` probe relies on
/// this). v0: PyDict clones share the RefCell state, so a clone IS live.
fn instance_dict_handle(o: &Gc<ap_obj::PyObject>) -> Gc<PyDict> {
    // attrs is the live Gc<PyDict> handle: clones share the state.
    o.attrs.borrow().clone()
}

/// The binding TypeErrors from __new__/__init__ carry the QUALIFIED name
/// (S-03-07 observed: `A7.__new__() takes ...`); the executor's generic
/// binder only knows the bare fn name — rewrite on the way out.
fn qualify_newinit_error(r: Result<Value, Flow>, which: &str, cls: &str) -> Result<Value, Flow> {
    match r {
        Err(Flow::Raise(mut e)) if e.class == "TypeError" => {
            if let Some(Value::Str(s)) = e.args.first() {
                let text = s.as_str();
                if text.starts_with(which) {
                    let qualified = format!("{cls}.{text}");
                    e.args = vec![Value::Str(PyString::new(qualified))];
                }
            }
            Err(Flow::Raise(e))
        }
        other => other,
    }
}

/// Is this instance of an exception class?
pub fn is_exception_value(v: &Value) -> bool {
    match v {
        Value::Obj(o) if o.kind == ap_obj::ObjKind::UserInstance => o
            .type_ref
            .borrow()
            .as_ref()
            .and_then(|w| w.upgrade())
            .is_some_and(|t| t.flags.is_exception),
        _ => false,
    }
}

/// (helper) class payload of a value via the back-pointers.
pub fn class_of_value_here(v: &Value) -> Option<Rc<TypeRef>> {
    match v {
        Value::Obj(o) if o.kind == ap_obj::ObjKind::UserInstance => {
            o.type_ref.borrow().as_ref().and_then(|w| w.upgrade())
        }
        Value::Obj(o) if o.kind == ap_obj::ObjKind::Exception => o.class_ref.borrow().clone(),
        _ => None,
    }
}

/// The interned exception class payload by name (must exist).
pub fn exception_payload(interp: &Interpreter, name: &str) -> std::rc::Rc<TypeRef> {
    exc_class(interp, name).expect("builtin exception class")
}
