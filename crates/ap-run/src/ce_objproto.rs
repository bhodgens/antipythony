// ---------------------------------------------------------------------------
// Leaf 1005b: the object-protocol vtable (S1 band 2). ADDITIVE per the
// NEXT-1004 pattern: new fn + ensure_* installer, dispatch bodies above
// untouched. ap-ce cannot reach back into ap-run (layering: ap-run depends
// on ap-ce), so the object-protocol band — like 10.04's PycallFn — rides a
// REGISTERED VTABLE: ap-ce exports PyObject_GetAttr/... and dispatches
// through the pointers installed here at first ce import.
//
// Every impl defers to the runtime's OWN protocol functions (attr_get_opt /
// attr_set_protocol / attr_del_protocol / builtin_len / eval_compare /
// user_truthiness / builtin_iter / pull_iter_pub / py_hash / builtin_dir)
// so a C extension sees byte-identical behavior with the equivalent
// Python-level expression. Error text dialects come from those functions
// themselves — nothing re-rendered here except the attr-miss text, which
// mirrors classes::attr_get_protocol's pinned format (probed CPython
// 3.14.7: "'A' object has no attribute 'x'", the module phrasing
// "module 'mm' has no attribute 'x'", and the no-__dict__ suffix for
// builtin receivers).
// ---------------------------------------------------------------------------

use crate::Flow;
use ap_obj::value::{PyString, Value};

static OBJPROTO_INSTALL: std::sync::OnceLock<()> = std::sync::OnceLock::new();

fn op_err(class: &str, msg: impl Into<String>) -> ap_ce::exports_objproto::OpRes {
    ap_ce::exports_objproto::OpRes::err(class, msg)
}

fn flow_to_opres(r: Result<Value, Flow>) -> ap_ce::exports_objproto::OpRes {
    match r {
        Ok(v) => ap_ce::exports_objproto::OpRes::val(v),
        Err(Flow::Raise(e)) => {
            let msg = e
                .args
                .first()
                .map(|a| match a {
                    Value::Str(s) => s.as_str().to_string(),
                    other => ap_obj::ops::repr(other).unwrap_or_default(),
                })
                .unwrap_or_default();
            op_err(&e.class, msg)
        }
        Err(_) => op_err("SystemError", "bridge flow"),
    }
}

/// The attr-miss message in the receiver-kind dialect CPython pins (probed
/// 3.14.7): reads report "'T' object has no attribute 'n'" for every kind;
/// the "and no __dict__ for setting new attributes" suffix is the
/// SET/DELETE-dialect only (int/None/str/... builtin receivers).
fn attr_miss_text(obj: &Value, name: &str) -> String {
    match obj {
        Value::Obj(o) if o.kind == ap_obj::ObjKind::Module => {
            format!("module '{}' has no attribute '{}'", o.name, name)
        }
        other => format!(
            "'{}' object has no attribute '{}'",
            crate::classes::display_type_name(other),
            name
        ),
    }
}

/// The setattr/delattr miss dialect (the __dict__ suffix on builtin
/// receivers without instance storage; module phrasing stays its own).
fn attr_setdel_miss_text(obj: &Value, name: &str) -> String {
    match obj {
        Value::Obj(o) if o.kind == ap_obj::ObjKind::Module => {
            format!("'module' object has no attribute '{}'", name)
        }
        Value::Obj(_) => attr_miss_text(obj, name),
        other => format!(
            "'{}' object has no attribute '{}' and no __dict__ for setting new attributes",
            crate::classes::display_type_name(other),
            name
        ),
    }
}

fn objproto_attr_get(obj: &Value, name: &str) -> ap_ce::exports_objproto::OpRes {
    let Some(interp) = crate::builtins_modules::current_interp() else {
        return op_err("RuntimeError", "no interpreter on this thread");
    };
    match crate::classes::attr_get_opt(&interp, obj, name) {
        Ok(Some(v)) => ap_ce::exports_objproto::OpRes::val(v),
        Ok(None) => ap_ce::exports_objproto::OpRes::missing_with_text(attr_miss_text(obj, name)),
        Err(flow) => match flow {
            Flow::Raise(e) if e.class == "AttributeError" => {
                let msg = e
                    .args
                    .first()
                    .map(|a| match a {
                        Value::Str(s) => s.as_str().to_string(),
                        other => ap_obj::ops::repr(other).unwrap_or_default(),
                    })
                    .unwrap_or_default();
                ap_ce::exports_objproto::OpRes::missing_with_text(msg)
            }
            other => flow_to_opres(Err(other)),
        },
    }
}

fn objproto_attr_set(obj: &Value, name: &str, v: Value) -> ap_ce::exports_objproto::OpRes {
    let Some(interp) = crate::builtins_modules::current_interp() else {
        return op_err("RuntimeError", "no interpreter on this thread");
    };
    // CPython setattr on a builtin no-__dict__ receiver RAISES (probed
    // 3.14.7: "'int' object has no attribute 'q' and no __dict__ for
    // setting new attributes"); the runtime's attr_set_protocol treats
    // those writes as a documented no-op (enum member creation depends on
    // it). The C-API surface keeps CPython's dialect: reject here while
    // the runtime-internal no-op stays untouched (ap-run body unchanged).
    if matches!(
        obj,
        Value::Str(_) | Value::Int(_) | Value::Bool(_) | Value::Bytes(_) | Value::Float(_)
    ) {
        return op_err("AttributeError", attr_setdel_miss_text(obj, name));
    }
    match crate::classes::attr_set_protocol(&interp, obj, name, v) {
        Ok(()) => ap_ce::exports_objproto::OpRes::unit(),
        Err(Flow::Raise(e)) if e.class == "AttributeError" => {
            // Normalize the miss text to CPython's setattr dialect (the
            // runtime's protocol raises the READ phrasing on builtin
            // receivers; the set/delete dialect carries the no-__dict__
            // suffix — probed 3.14.7).
            op_err("AttributeError", attr_setdel_miss_text(obj, name))
        }
        Err(flow) => flow_to_opres(Err(flow)),
    }
}

fn objproto_attr_del(obj: &Value, name: &str) -> ap_ce::exports_objproto::OpRes {
    let Some(interp) = crate::builtins_modules::current_interp() else {
        return op_err("RuntimeError", "no interpreter on this thread");
    };
    match crate::classes::attr_del_protocol(&interp, obj, name) {
        Ok(()) => ap_ce::exports_objproto::OpRes::unit(),
        Err(Flow::Raise(e)) if e.class == "AttributeError" => {
            op_err("AttributeError", attr_setdel_miss_text(obj, name))
        }
        Err(flow) => flow_to_opres(Err(flow)),
    }
}

fn objproto_len_of(obj: &Value) -> ap_ce::exports_objproto::OpRes {
    // The builtin len() surface INCLUDING the user __len__ protocol (the
    // class_protocol_builtin layer is what Python-level len(x) dispatches
    // through — S-03-22; the exact "object of type '<T>' has no len()"
    // TypeError comes from there too).
    let Some(interp) = crate::builtins_modules::current_interp() else {
        return op_err("RuntimeError", "no interpreter on this thread");
    };
    let r = match crate::interp::class_protocol_builtin_pub(
        &interp,
        "len",
        std::slice::from_ref(obj),
        &[],
    ) {
        Some(r) => r,
        None => crate::builtins::builtin_len(std::slice::from_ref(obj)),
    };
    match r {
        Ok(v) => ap_ce::exports_objproto::OpRes::val(v),
        Err(Flow::Raise(e)) => {
            let msg = e
                .args
                .first()
                .map(|a| match a {
                    Value::Str(s) => s.as_str().to_string(),
                    other => ap_obj::ops::repr(other).unwrap_or_default(),
                })
                .unwrap_or_default();
            op_err(&e.class, msg)
        }
        Err(_) => op_err("SystemError", "bridge flow"),
    }
}

fn objproto_length_hint(obj: &Value, default: i64) -> ap_ce::exports_objproto::OpRes {
    use ap_ce::exports_objproto::OpRes;
    // CPython Objects/abstract.c PyObject_LengthHint — the REAL order
    // (read off the 3.12/3.14 source):
    //  1. when the type HAS len (_PyObject_HasLen): try len(o); a
    //     non-TypeError error PROPAGATES, a TypeError is cleared and the
    //     hint path runs; success returns the len WITHOUT consulting
    //     __length_hint__ (probed: LenFirst hint=2 len=9 => 9).
    //  2. hint = lookup __length_hint__ (miss => default);
    //     NotImplemented => default; call TypeError => default;
    //     other call errors PROPAGATE (probed ValueError('boom'));
    //     non-int result => TypeError "__length_hint__ must be an
    //     integer, not '<T>'"; negative => ValueError
    //     "__length_hint__() should return >= 0".
    //  3. defaultvalue.
    let ic = crate::builtins_modules::current_interp();
    // Step 1: has-len => len first (class-protocol dispatch so user
    // __len__ instances answer — the Python-level len(x) path). Builtin
    // values (list/str/dict/...) have no class-level __len__ slot in the
    // value model, so ALSO try the direct builtin len: if it succeeds the
    // type "has len" (CPython's _PyObject_HasLen).
    let mut len_done = false;
    if let Some(ref icv) = ic {
        let has_len = crate::classes::special_lookup(icv, obj, "__len__").is_some();
        let r =
            if has_len {
                match crate::interp::class_protocol_builtin_pub(
                    icv,
                    "len",
                    std::slice::from_ref(obj),
                    &[],
                ) {
                    Some(r) => Some(r),
                    None => Some(crate::builtins::builtin_len(std::slice::from_ref(obj))),
                }
            } else {
                // Builtin-kind check: len succeeds => the type has len.
                match crate::builtins::builtin_len(std::slice::from_ref(obj)) {
                    Ok(_) => Some(Ok(crate::builtins::builtin_len(std::slice::from_ref(obj))
                        .unwrap_or(Value::None))),
                    Err(_) => None,
                }
            };
        if let Some(r) = r {
            len_done = true;
            match r {
                Ok(v) => {
                    if let Some(n) = v.as_int().and_then(|i| i.to_i64()) {
                        return OpRes::val(Value::Int(ap_obj::int::Int::from_i64(n)));
                    }
                    // non-int len: fall through to the hint path (CPython
                    // would have raised inside PyObject_Length; the builtin
                    // len's own TypeError text already surfaced for that
                    // shape via builtin_len — treat as cleared).
                }
                Err(Flow::Raise(e)) if e.class == "TypeError" => { /* cleared */ }
                Err(other) => return flow_to_opres(Err(other)),
            }
        }
    }
    // Step 2: __length_hint__ (CPython tries the hint even when len
    // FAILED with TypeError in step 1 — but when len SUCCEEDED it already
    // returned, so len_done==true can only reach here via non-int len).
    if let Some(ref icv) = ic {
        if let Some((f, _)) = crate::classes::special_lookup(icv, obj, "__length_hint__") {
            let call = crate::interp::call_value_pub(icv, &f, std::slice::from_ref(obj), &[]);
            match call {
                Err(Flow::Raise(e)) if e.class == "AttributeError" => { /* default */ }
                Err(Flow::Raise(e)) if e.class == "TypeError" => { /* default */ }
                Err(other) => return flow_to_opres(Err(other)),
                Ok(hv) => {
                    if matches!(&hv, Value::Obj(o) if o.name == "NotImplemented" && o.kind == ap_obj::ObjKind::Opaque)
                    {
                        // default
                    } else if let Some(n) = hv.as_int().and_then(|i| i.to_i64()) {
                        if n < 0 {
                            return op_err("ValueError", "__length_hint__() should return >= 0");
                        }
                        return OpRes::val(Value::Int(ap_obj::int::Int::from_i64(n)));
                    } else {
                        let tn = crate::classes::display_type_name(&hv);
                        // CPython abstract.c dialect: NO quotes around the
                        // type name (probed 3.14.7: "must be an integer,
                        // not str").
                        return op_err(
                            "TypeError",
                            format!("__length_hint__ must be an integer, not {tn}"),
                        );
                    }
                }
            }
        }
    }
    // Step 3
    OpRes::val(Value::Int(ap_obj::int::Int::from_i64(default)))
}

fn objproto_truth_of(obj: &Value) -> ap_ce::exports_objproto::OpRes {
    let Some(interp) = crate::builtins_modules::current_interp() else {
        return op_err("RuntimeError", "no interpreter on this thread");
    };
    // The bool(x) surface (user __bool__/__len__ protocol via the
    // class-protocol layer — the Python-level bool(x) path).
    let r = match crate::interp::class_protocol_builtin_pub(
        &interp,
        "bool",
        std::slice::from_ref(obj),
        &[],
    ) {
        Some(r) => r,
        None => crate::interp::truthy_public(&interp, obj).map(Value::Bool),
    };
    match r {
        Ok(Value::Bool(b)) => ap_ce::exports_objproto::OpRes::val(Value::Bool(b)),
        Ok(v) => ap_ce::exports_objproto::OpRes::val(Value::Bool(v.is_truthy())),
        Err(flow) => flow_to_opres(Err(flow)),
    }
}

fn cmpop_of(op: u8) -> Option<ap_ir::CmpOp> {
    use ap_ir::CmpOp;
    Some(match op {
        0 => CmpOp::Lt,
        1 => CmpOp::LtE,
        2 => CmpOp::Eq,
        3 => CmpOp::NotEq,
        4 => CmpOp::Gt,
        5 => CmpOp::GtE,
        _ => return None,
    })
}

fn objproto_rich_compare(
    a: &Value,
    b: &Value,
    op: u8,
    _bool_form: bool,
) -> ap_ce::exports_objproto::OpRes {
    use ap_ce::exports_objproto::OpRes;
    let Some(cop) = cmpop_of(op) else {
        return op_err("SystemError", "PyObject_RichCompare: bad op");
    };
    let Some(interp) = crate::builtins_modules::current_interp() else {
        return op_err("RuntimeError", "no interpreter on this thread");
    };
    // The FULL compare protocol via interp::compare_values_pub (the
    // additive 1005b shim over eval_compare): user slots + reflected ops +
    // proper-subclass priority + the builtin fallbacks with their exact
    // op-spelled TypeErrors.
    match crate::interp::compare_values_pub(&interp, a, cop, b) {
        Ok(b) => OpRes::val(Value::Bool(b)),
        Err(flow) => flow_to_opres(Err(flow)),
    }
}

fn objproto_get_iter(obj: &Value) -> ap_ce::exports_objproto::OpRes {
    // The builtin iter() surface: registers in the EXISTING iterator table
    // (register_shared_iter lifecycle), generators keep their lazy ids,
    // iter(it) re-fetches the same state. "'int' object is not iterable"
    // comes from iterable_of's pinned text.
    match crate::builtins::builtin_iter(std::slice::from_ref(obj)) {
        Ok(v) => ap_ce::exports_objproto::OpRes::val(v),
        Err(Flow::Raise(e)) => {
            let msg = e
                .args
                .first()
                .map(|a| match a {
                    Value::Str(s) => s.as_str().to_string(),
                    other => ap_obj::ops::repr(other).unwrap_or_default(),
                })
                .unwrap_or_default();
            op_err(&e.class, msg)
        }
        Err(_) => op_err("SystemError", "bridge flow"),
    }
}

fn objproto_iter_next(obj: &Value) -> ap_ce::exports_objproto::OpRes {
    use ap_ce::exports_objproto::OpRes;
    // The builtin next() surface minus the Python-visible StopIteration
    // raise: clean exhaustion reports OpRes::exhausted() so the C side
    // sees NULL + cleared error (PyO3 loop contract).
    let Some(interp) = crate::builtins_modules::current_interp() else {
        return op_err("RuntimeError", "no interpreter on this thread");
    };
    let Value::Obj(o) = obj else {
        return op_err(
            "TypeError",
            format!("'{}' object is not an iterator", obj.type_name()),
        );
    };
    if !matches!(
        o.kind,
        ap_obj::ObjKind::Iterator | ap_obj::ObjKind::Generator
    ) {
        return op_err(
            "TypeError",
            format!("'{}' object is not an iterator", obj.type_name()),
        );
    };
    let Some(id) = *o.gen_id.lock().unwrap_or_else(|p| p.into_inner()) else {
        return op_err("TypeError", "'iterator' object is not an iterator");
    };
    if crate::generators::gen_is_running(id) {
        return op_err("ValueError", "generator already executing");
    }
    if interp
        .generators
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .contains_key(&id)
    {
        let rc = interp.rc();
        match crate::interp::gen_pull_public(&rc, id) {
            Ok(Some(x)) => return OpRes::val(x),
            Ok(None) => return OpRes::exhausted(),
            Err(Flow::Raise(e)) if e.class == "StopIteration" => return OpRes::exhausted(),
            Err(other) => return flow_to_opres(Err(other)),
        }
    }
    match crate::interp::pull_iter_pub(&interp, id) {
        Ok(Some(x)) => OpRes::val(x),
        Ok(None) => OpRes::exhausted(),
        Err(Flow::Raise(e)) if e.class == "StopIteration" => OpRes::exhausted(),
        Err(other) => flow_to_opres(Err(other)),
    }
}

fn objproto_hash_of(obj: &Value) -> ap_ce::exports_objproto::OpRes {
    // The hash(x) surface INCLUDING the user __hash__ protocol (the
    // class-protocol layer — H8's `__hash__ = None` class must raise the
    // CPython "unhashable type: '<T>'" TypeError through it).
    let Some(interp) = crate::builtins_modules::current_interp() else {
        return op_err("RuntimeError", "no interpreter on this thread");
    };
    // CPython dialect: a __hash__ slot of None means UNHASHABLE (probed
    // 3.14.7: "unhashable type: 'Unhashable'"). The runtime would call the
    // None value; check the slot before dispatching.
    if let Some((f, _)) = crate::classes::special_lookup(&interp, obj, "__hash__") {
        if matches!(f, Value::None) {
            return op_err(
                "TypeError",
                format!(
                    "unhashable type: '{}'",
                    crate::classes::display_type_name(obj)
                ),
            );
        }
    }
    let r = match crate::interp::class_protocol_builtin_pub(
        &interp,
        "hash",
        std::slice::from_ref(obj),
        &[],
    ) {
        Some(r) => r,
        None => match ap_obj::ops::hash(obj) {
            Ok(h) => Ok(Value::Int(ap_obj::int::Int::from_i64(h))),
            Err(exc) => Err(Flow::Raise(crate::exc_hier::ExceptionValue::msg(
                exc.class,
                exc.message,
            ))),
        },
    };
    match r {
        Ok(v) => ap_ce::exports_objproto::OpRes::val(v),
        Err(Flow::Raise(e)) => {
            let msg = e
                .args
                .first()
                .map(|a| match a {
                    Value::Str(s) => s.as_str().to_string(),
                    other => ap_obj::ops::repr(other).unwrap_or_default(),
                })
                .unwrap_or_default();
            op_err(&e.class, msg)
        }
        Err(_) => op_err("SystemError", "bridge flow"),
    }
}

fn objproto_dir_of(obj: Option<&Value>) -> ap_ce::exports_objproto::OpRes {
    let Some(interp) = crate::builtins_modules::current_interp() else {
        return op_err("RuntimeError", "no interpreter on this thread");
    };
    let args: Vec<Value> = match obj {
        Some(v) => vec![v.clone()],
        None => vec![],
    };
    match crate::builtins::call_builtin(&interp, "dir", &args, &[]) {
        Ok(v) => ap_ce::exports_objproto::OpRes::val(v),
        Err(flow) => flow_to_opres(Err(flow)),
    }
}

fn objproto_display_name(v: &Value) -> String {
    crate::classes::display_type_name(v)
}

/// Install the object-protocol vtable into ap-ce (idempotent; called
/// alongside ensure_pycall_installed at first ce dispatch — an extension
/// may hit GetAttr before any Call).
pub fn ensure_objproto_installed() {
    ensure_mapping_installed();
    OBJPROTO_INSTALL.get_or_init(|| {
        let _ = ap_ce::exports_objproto::install_objproto_vtable(
            ap_ce::exports_objproto::ObjProtoVTable {
                attr_get: objproto_attr_get,
                attr_set: objproto_attr_set,
                attr_del: objproto_attr_del,
                len_of: objproto_len_of,
                length_hint: objproto_length_hint,
                truth_of: objproto_truth_of,
                rich_compare: objproto_rich_compare,
                get_iter: objproto_get_iter,
                iter_next: objproto_iter_next,
                hash_of: objproto_hash_of,
                dir_of: objproto_dir_of,
                display_name: objproto_display_name,
            },
        );
    });
}

// ---------------------------------------------------------------------------
// The mapping/sequence vtable (leaf pydantic-probe): Get/Set/DelItem,
// contains, IsInstance/IsSubclass, Number binops, Index, import, sets.
// Installed next to the objproto table (same OnceLock point).
// ---------------------------------------------------------------------------

fn mapping_get_item(obj: &Value, key: &Value) -> ap_ce::exports_objproto::OpRes {
    // crate::methods::subscript_get — the obj[key] surface (user
    // __getitem__ rides the special-lookup path inside it).
    match crate::methods::subscript_get(obj, key) {
        Ok(v) => ap_ce::exports_objproto::OpRes::val(v),
        Err(flow) => flow_to_opres(Err(flow)),
    }
}

fn mapping_set_item(obj: &Value, key: &Value, v: Value) -> ap_ce::exports_objproto::OpRes {
    let Some(interp) = crate::builtins_modules::current_interp() else {
        return op_err("RuntimeError", "no interpreter on this thread");
    };
    match crate::methods::subscript_set(obj, key, v) {
        Ok(()) => ap_ce::exports_objproto::OpRes::unit(),
        Err(flow) => flow_to_opres(Err(flow)),
    }
}

fn mapping_del_item(obj: &Value, key: &Value) -> ap_ce::exports_objproto::OpRes {
    let Some(interp) = crate::builtins_modules::current_interp() else {
        return op_err("RuntimeError", "no interpreter on this thread");
    };
    match crate::methods::subscript_del(obj, key) {
        Ok(()) => ap_ce::exports_objproto::OpRes::unit(),
        Err(flow) => flow_to_opres(Err(flow)),
    }
}

fn mapping_contains(obj: &Value, key: &Value) -> ap_ce::exports_objproto::OpRes {
    // `key in obj` — interp::contains (the __contains__/iteration protocol).
    match crate::interp::contains(key, obj) {
        Ok(b) => ap_ce::exports_objproto::OpRes::val(Value::Bool(b)),
        Err(flow) => flow_to_opres(Err(flow)),
    }
}

fn mapping_is_instance(inst: &Value, cls: &Value) -> ap_ce::exports_objproto::OpRes {
    let Some(interp) = crate::builtins_modules::current_interp() else {
        return op_err("RuntimeError", "no interpreter on this thread");
    };
    let args = vec![inst.clone(), cls.clone()];
    match crate::builtins::call_builtin(&interp, "isinstance", &args, &[]) {
        Ok(v) => ap_ce::exports_objproto::OpRes::val(v),
        Err(flow) => flow_to_opres(Err(flow)),
    }
}

fn mapping_is_subclass(derived: &Value, cls: &Value) -> ap_ce::exports_objproto::OpRes {
    let Some(interp) = crate::builtins_modules::current_interp() else {
        return op_err("RuntimeError", "no interpreter on this thread");
    };
    let args = vec![derived.clone(), cls.clone()];
    match crate::builtins::call_builtin(&interp, "issubclass", &args, &[]) {
        Ok(v) => ap_ce::exports_objproto::OpRes::val(v),
        Err(flow) => flow_to_opres(Err(flow)),
    }
}

fn mapping_number_bin(a: &Value, b: &Value, op: char) -> ap_ce::exports_objproto::OpRes {
    use ap_ir::expr::BinOp;
    let binop = match op {
        '%' => BinOp::Mod,
        '/' => BinOp::Div,
        _ => return op_err("SystemError", "mapping_number_bin: bad op"),
    };
    match crate::interp::eval_binop(a, binop, b) {
        Ok(v) => ap_ce::exports_objproto::OpRes::val(v),
        Err(flow) => flow_to_opres(Err(flow)),
    }
}

fn mapping_index_of(obj: &Value) -> ap_ce::exports_objproto::OpRes {
    // int passthrough (CPython answers the same object); other types go
    // through the __index__ protocol via int(x) semantics.
    match obj {
        Value::Int(_) => ap_ce::exports_objproto::OpRes::val(obj.clone()),
        other => {
            let Some(interp) = crate::builtins_modules::current_interp() else {
                return op_err("RuntimeError", "no interpreter on this thread");
            };
            let args = vec![other.clone()];
            match crate::builtins::call_builtin(&interp, "int", &args, &[]) {
                Ok(v) => ap_ce::exports_objproto::OpRes::val(v),
                Err(flow) => flow_to_opres(Err(flow)),
            }
        }
    }
}

fn mapping_import_module(name: &str) -> ap_ce::exports_objproto::OpRes {
    let Some(interp) = crate::builtins_modules::current_interp() else {
        return op_err("RuntimeError", "no interpreter on this thread");
    };
    match crate::importer::import_module(&interp, name) {
        Ok(v) => ap_ce::exports_objproto::OpRes::val(v),
        Err(flow) => flow_to_opres(Err(flow)),
    }
}

/// Install the mapping/number vtable (idempotent; same call point as the
/// objproto table).
pub fn ensure_mapping_installed() {
    use ap_ce::exports_pydantic::MappingVTable;
    let _ = ap_ce::exports_pydantic::install_mapping_vtable(MappingVTable {
        get_item: mapping_get_item,
        set_item: mapping_set_item,
        del_item: mapping_del_item,
        contains: mapping_contains,
        is_instance: mapping_is_instance,
        is_subclass: mapping_is_subclass,
        number_bin: mapping_number_bin,
        index_of: mapping_index_of,
        import_module: mapping_import_module,
    });
}
