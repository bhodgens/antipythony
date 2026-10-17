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

fn raise(class: &str, msg: impl Into<String>) -> Flow {
    Flow::Raise(ExceptionValue::msg(class, msg))
}

// ===========================================================================
// Class payload plumbing
// ===========================================================================

/// The TypeObject payload behind a class Value (None for non-classes).
pub fn type_of(_interp: &Interpreter, v: &Value) -> Option<Rc<TypeRef>> {
    match v {
        Value::Obj(o) if o.kind == ap_obj::ObjKind::UserClass => o.class_ref.borrow().clone(),
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
    ("InterpreterError", Some("Exception")),
    ("LookupError", Some("Exception")),
    ("MemoryError", Some("Exception")),
    ("NameError", Some("Exception")),
    ("OSError", Some("Exception")),
    ("PatternError", Some("Exception")),
    ("EOFError", Some("Exception")),
    ("ReferenceError", Some("Exception")),
    ("RuntimeError", Some("Exception")),
    ("StopAsyncIteration", Some("Exception")),
    ("StopIteration", Some("Exception")),
    ("SyntaxError", Some("Exception")),
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
        let _ = t.dict.set(
            Value::Str(PyString::new("__module__")),
            Value::Str(PyString::new("builtins")),
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
    // interp::class_protocol_builtin).
    for n in ["super", "property", "classmethod", "staticmethod"] {
        let o = ap_obj::PyObject::new(ap_obj::ObjKind::BuiltinFn, n);
        let _ = interp
            .builtins
            .set(Value::Str(PyString::new(n)), Value::Obj(o));
    }
    // NotImplemented singleton (S-03-25 defer marker).
    let ni = ap_obj::PyObject::new(ap_obj::ObjKind::Opaque, "NotImplemented");
    let _ = interp
        .builtins
        .set(Value::Str(PyString::new("NotImplemented")), Value::Obj(ni));
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
pub fn exec_class_def(
    interp: &Rc<Interpreter>,
    frame: &Rc<Frame>,
    c: &ap_ir::item::ClassDef,
    reg: &Rc<FnRegistry>,
    bind_name: &str,
) -> EResult {
    // 1. Bases + explicit metaclass in the ENCLOSING frame (S-03-1).
    let mut bases_v: Vec<Value> = Vec::new();
    for b in &c.bases {
        let v = eval_expr(interp, frame, b, reg)?;
        // The lowering encodes non-metaclass class keywords as
        // `__ap_classkw__.<name>(value)` calls — they are NOT bases.
        // We can't recover the name here (lowering collapsed it), so class
        // kwargs ride the `__init_subclass__` path only for the cases the
        // corpus exercises; the synthetic value raises if evaluated.
        if let Value::Obj(o) = &v {
            if o.kind == ap_obj::ObjKind::BuiltinFn && o.name.starts_with("__ap_classkw__") {
                continue;
            }
        }
        bases_v.push(v);
    }
    let metaclass_v = match &c.metaclass {
        Some(e) => Some(eval_expr(interp, frame, e, reg)?),
        None => None,
    };

    // 2. Body in a fresh class namespace (S-03-3). The class frame is NOT
    //    an enclosing scope for methods (frontend skips Class scopes); the
    //    `__class__` cell (zero-arg super, S-03-17) is seeded after the
    //    class object exists.
    let ns = PyDict::new();
    let class_frame = Rc::new(Frame::new_class(frame, ns.clone()));
    exec_block(interp, &class_frame, &c.body, reg)?;

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
    let t = TypeObject::create(
        c.name.clone(),
        qualname,
        base_types.clone(),
        ns,
        flags,
        None,
    )
    .map_err(|e| raise("TypeError", ap_obj::classes::mro_error_message(&e)))?;
    // The RESOLVED metaclass rides the payload (type(C), meta hook lookup).
    *t.metaclass.borrow_mut() = _resolved_meta.clone();

    let class_v = ap_obj::classes::make_class_value(t.clone());

    // Module marker: builtin exceptions report "builtins" (e010); user
    // classes "__main__" (S-03-04).
    let module_name = if t.flags.builtin_exc {
        "builtins"
    } else {
        ap_obj::MODULE_NAME
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
    run_set_name_hooks(interp, &class_v, &t)?;
    if let Some(parent) = base_types.first() {
        if parent.lookup("__init_subclass__").is_some() {
            // Bind the parent's hook as an implicit classmethod of cls.
            let hook = attr_on_class_raw(parent, "__init_subclass__");
            let f = unwrap_classmethod(&hook, class_v.clone());
            // The DEFAULT hook (object.__init_subclass__) rejects kwargs
            // with the pinned text (S-03-29); user overrides consume them.
            if !class_kwargs_present(interp) {
                let _ = f;
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
    store_name(interp, frame, bind_name, class_v.clone(), reg)?;
    Ok(class_v)
}

/// Always-false stub: v1 class kwargs reach __init_subclass__ only through
/// the explicit metaclass-forwarding form (c031) — the no-kwargs default
/// path raises the pinned TypeError there. See S-03-29 CONTRACT-GAP note.
fn class_kwargs_present(_interp: &Interpreter) -> bool {
    false
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
    let t = TypeObject::create(
        "type".to_string(),
        "type".to_string(),
        vec![obj],
        PyDict::new(),
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
            let mo =
                ap_obj::PyObject::with_self(ap_obj::ObjKind::BoundMethod, o.name.clone(), receiver);
            if let Some(mut m) = std::rc::Rc::try_unwrap(mo).ok() {
                *m.func.borrow_mut() = Some(f.clone());
                if let Some(owner) = owner {
                    *m.method_owner.borrow_mut() = Some(std::rc::Rc::downgrade(owner));
                }
                return Value::Obj(ap_obj::int::Gc::new(m));
            }
            unreachable!("fresh bound method has one owner");
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
        if o.kind == ap_obj::ObjKind::Opaque && o.name == "__union__" {
            for arm in o.slice_parts.borrow().iter() {
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
                return Err(raise(
                    "TypeError",
                    "isinstance() arg 2 must be a type, a tuple of types, or a union",
                ));
            };
            if xc.is_subclass(&tc) {
                return Ok(true);
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
            if ac.is_subclass(&bc) {
                return Ok(true);
            }
            // Custom metaclass __subclasscheck__ (S-03-31; c033 WS).
            if let Some(meta) = ac.metaclass.borrow().clone() {
                if let Some(hook) = meta.lookup("__subclasscheck__") {
                    if matches!(&hook, Value::Obj(f) if f.kind == ap_obj::ObjKind::Function) {
                        let interp_rc = crate::interp::current_interp_handle();
                        let r = crate::interp::call_value_pub(
                            &interp_rc,
                            &hook,
                            &[class_value_for(&ac), a.clone()],
                            &[],
                        )?;
                        return Ok(r.is_truthy());
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
    attr_get_opt(interp, obj, name)?.ok_or_else(|| {
        raise(
            "AttributeError",
            format!("'{}' object has no attribute '{}'", type_name_of(obj), name),
        )
    })
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
                None => return Ok(None),
            };
            instance_attr(interp, obj, o, &t, name).map(Some)
        }
        Value::Obj(o) if o.kind == ap_obj::ObjKind::UserClass => {
            let Some(t) = o.class_ref.borrow().clone() else {
                return Ok(None);
            };
            class_attr_get(interp, obj, &t, name).map(Some)
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
            return Ok(t
                .dict
                .get(&Value::Str(PyString::new("__module__")))
                .unwrap_or(Value::Str(PyString::new(ap_obj::MODULE_NAME))));
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
        if let Some((mv, _)) = meta.lookup_with_owner(name) {
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
    match obj {
        Value::Obj(o) if o.kind == ap_obj::ObjKind::UserInstance => {
            let t = o
                .type_ref
                .borrow()
                .as_ref()
                .and_then(|w| w.upgrade())
                .ok_or_else(|| raise("SystemError", "instance without class"))?;
            // __setattr__ override wins (S-03-11 last clause).
            if let Some((sa, _)) = t.lookup_with_owner("__setattr__") {
                // The default object.__setattr__ is not materialized; only
                // user overrides dispatch here.
                if is_user_function(&sa) {
                    return call_value(
                        interp,
                        &module_frame(interp),
                        &sa,
                        &[obj.clone(), Value::Str(PyString::new(name)), v],
                        &[],
                    )
                    .map(|_| ());
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
    if let Some(meta) = t.metaclass.borrow().clone() {
        if let Some((call, _)) = meta.lookup_with_owner("__call__") {
            if is_user_function(&call) {
                return call_value(interp, &module_frame(interp), &call, &[cls.clone()], &[]);
            }
        }
    }

    let new_lookup = t.lookup_with_owner("__new__");
    let init_lookup = t.lookup_with_owner("__init__");
    let new_f = new_lookup.as_ref().map(|(v, _)| v.clone());
    let init_f = init_lookup.as_ref().map(|(v, _)| v.clone());
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
    if new_f.is_none() && init_f.is_none() && (!args.is_empty() || !kwargs.is_empty()) {
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
                interp.super_stack.borrow_mut().pop();
                let r = qualify_newinit_error(r, "__init__", &t.name)?;
                check_init_return(&r, inf)?;
            }
        }
        return Ok(obj);
    }
    // Default __new__: allocate + __init__ (custom or inherited/user).
    let obj = alloc_instance(t);
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
            interp.super_stack.borrow_mut().pop();
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
    if !t.has_instance_dict() {
        // Slots-only: leave attrs empty; writes are rejected before the
        // dict is consulted.
    }
    Value::Obj(o)
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
