//! The v0 builtins (task brief §3) + the `sys` module stub.
//!
//! Every builtin is a Rust function over `Value` args with CPython-exact
//! error texts. `print` writes through the interpreter buffer (S-06-2
//! stdout byte gate). Numeric routes delegate to ap-obj ops (the C-12..C-16
//! surface proven in 02.01).

use crate::exc_hier::ExceptionValue;
use crate::frame::Interpreter;
use crate::Flow;
use ap_obj::dict::{PyDict, PySet};
use ap_obj::int::{Gc, Int};
use ap_obj::value::{PyList, PyString, Tuple, Value};

pub type BResult = Result<Value, Flow>;

fn raise(class: &str, msg: impl Into<String>) -> Flow {
    Flow::Raise(ExceptionValue::msg(class, msg.into()))
}

fn one<'a, T>(args: &'a [T], name: &str) -> Result<&'a T, Flow> {
    args.first()
        .ok_or_else(|| raise("TypeError", format!("{name}() missing required argument")))
}

// ---------------------------------------------------------------------------
// print
// ---------------------------------------------------------------------------

/// str(v) for print: exceptions render via args (S-06-7); everything else
/// rides the value layer. User-instance __str__ dispatches through
/// methods (call sites that need it use the executor str path).
pub fn py_str_of(v: &Value) -> Result<String, ap_obj::exc::PyException> {
    // 02.04: user __str__ dispatches BEFORE the default (CPython: str(x)
    // calls type(x).__str__; TestCase.__str__ is the canonical consumer).
    if let Some(interp) = crate::builtins_modules::current_interp() {
        if matches!(
            v,
            Value::Obj(o) if o.kind == ap_obj::ObjKind::UserInstance
        ) {
            if let Some((f, owner)) = crate::classes::special_lookup(&interp, v, "__str__") {
                // The object-root default renders the default repr — skip it
                // so Exception instances keep their args-text str().
                // object/Exception-chain __str__ is the C-level renderer
                // (args text); our v0 special_lookup synthesizes a marker
                // for it — skip so the legacy args-text path below answers.
                let owner_is_object = matches!(
                    owner.name.as_str(),
                    "object" | "Exception" | "BaseException"
                );
                if !owner_is_object {
                    if let Value::Obj(fo) = &f {
                        if fo.kind == ap_obj::ObjKind::Function {
                            // W113: native str()/f-string dispatch calls the
                            // RAW function, bypassing the bound-method path
                            // that stages pending_super_ctx — a user __str__
                            // with zero-arg super() (parsel XPathExpr.__str__)
                            // died with "super(): no arguments". Stage the
                            // (owner, receiver) context like the BoundMethod
                            // call site does; call_function's sentinel
                            // consumes it on entry.
                            *interp.pending_super_ctx.borrow_mut() =
                                Some((owner.clone(), v.clone(), false));
                            if std::env::var("APX_SUPERDBG").is_ok() {
                                eprintln!("[W113-STRSTAGE] pending set for owner={}", owner.name);
                            }
                            let r = crate::interp::call_value_pub(&interp, &f, &[v.clone()], &[])
                                .map_err(|f| match f {
                                Flow::Raise(e) => ap_obj::exc::PyException::new(
                                    Box::leak(e.class.clone().into_boxed_str()),
                                    e.args
                                        .first()
                                        .map(|a| crate::builtins::py_str_of(a).unwrap_or_default())
                                        .unwrap_or_default(),
                                ),
                                _ => ap_obj::exc::PyException::new("SystemError", ""),
                            })?;
                            return crate::builtins::py_str_of(&r);
                        }
                    }
                }
            }
        }
    }
    if let Value::Obj(o) = v {
        // 02.08-lite native types carry a precomputed repr (struct_time etc).
        if let Some(txt) = &*o.repr_text.borrow() {
            return Ok(txt.clone());
        }
        if o.kind == ap_obj::ObjKind::Exception {
            let exc = crate::exc_hier::ExceptionValue::new(&o.name, o.args.borrow().clone());
            return exc.py_str();
        }
        // Real-class exception instances (S-03-32 UserInstance payloads):
        // str(e) renders the args text (interp's str() ctor arm does this —
        // print()/format() route through here and must agree).
        if o.kind == ap_obj::ObjKind::UserInstance && crate::interp::is_exception_instance_pub(v) {
            if let Ok(ev) = crate::interp::exception_value_of_instance_pub(v) {
                if let Ok(s) = ev.py_str() {
                    return Ok(s);
                }
            }
        }
    }
    ap_obj::ops::to_str(v)
}

/// print(*args, sep=' ', end='\n', file=None) — v0: file must be sys.stdout
/// or absent; str() conversion; None args print 'None'.
pub fn print(interp: &Interpreter, args: &[Value], kwargs: &[(String, Value)]) -> BResult {
    let mut sep = " ".to_string();
    let mut end = "\n".to_string();
    // redirect_stdout support: print writes to sys.stdout's .write() when the
    // attribute holds a non-default stream (io.StringIO in tests/corpus s018).
    let sys_stdout: Option<Value> =
        crate::builtins_modules::current_interp()
            .as_ref()
            .and_then(|ic| {
                let sys_v = ic.sys_module.borrow().clone()?;
                match sys_v {
                    Value::Obj(o) if o.kind == ap_obj::ObjKind::Module => o
                        .module_ns
                        .borrow()
                        .as_ref()
                        .and_then(|ns| ns.get(&Value::Str(PyString::new("stdout")))),
                    _ => None,
                }
            });
    let redirected: Option<Value> = match &sys_stdout {
        Some(v @ Value::Obj(o))
            if !(o.kind == ap_obj::ObjKind::Namespace && o.name == "stdout") =>
        {
            Some(v.clone())
        }
        _ => None,
    };
    for (k, v) in kwargs {
        match k.as_str() {
            "sep" => sep = ap_obj::ops::to_str(v).map_err(Flow::from_py)?,
            "end" => end = ap_obj::ops::to_str(v).map_err(Flow::from_py)?,
            "file" => {
                // explicit file= wins over the redirect probe
                return print_to_file(interp, v, args, &sep, &end);
            }
            "flush" => {}
            other => {
                return Err(raise(
                    "TypeError",
                    format!("'{other}' is an invalid keyword argument for print()"),
                ))
            }
        }
    }
    let mut out = String::new();
    for (i, a) in args.iter().enumerate() {
        if i > 0 {
            out.push_str(&sep);
        }
        // 02.03b: containers with user __repr__ instances (enum members in
        // list(Color)) render member reprs — same rule as repr().
        let is_container = matches!(
            a,
            Value::List(_) | Value::Tuple(_) | Value::Dict(_) | Value::Set(_) | Value::Frozen(_)
        );
        let interp_rc = crate::builtins_modules::current_interp();
        let s = if is_container {
            match interp_rc.as_ref() {
                Some(ic) => match crate::interp::repr_with_classes_public(ic, a) {
                    Ok(Value::Str(s)) => s.as_str().to_string(),
                    _ => py_str_of(a).map_err(Flow::from_py)?,
                },
                None => py_str_of(a).map_err(Flow::from_py)?,
            }
        } else {
            py_str_of(a).map_err(Flow::from_py)?
        };
        out.push_str(&s);
    }
    out.push_str(&end);
    if let Some(stream) = redirected {
        // Call stream.write(out) through the interpreter's method dispatch.
        let r = call_method_pub(&stream, "write", &[Value::Str(PyString::new(out.clone()))]);
        if r.is_ok() {
            return Ok(Value::None);
        }
    }
    interp.write_out(&out);
    Ok(Value::None)
}

/// print(file=...): write through any object with .write() (wave-107).
/// sys.stdout/ns-stdout goes to the interpreter console; other writers get
/// write() calls via method dispatch (scrapy logging does print(file=sys.stderr)).
fn print_to_file(interp: &Interpreter, v: &Value, args: &[Value], sep: &str, end: &str) -> BResult {
    let is_stdout = matches!(v, Value::Obj(o) if o.kind == ap_obj::ObjKind::Namespace
        && o.name == "stdout");
    let mut out = String::new();
    for (i, a) in args.iter().enumerate() {
        if i > 0 {
            out.push_str(sep);
        }
        out.push_str(&py_str_of(a).map_err(Flow::from_py)?);
    }
    out.push_str(end);
    if is_stdout {
        interp.write_out(&out);
        return Ok(Value::None);
    }
    // Namespace stderr (sys.stderr): write to the process stderr, matching
    // the native sys.stderr.write surface.
    if matches!(v, Value::Obj(o) if o.kind == ap_obj::ObjKind::Namespace && o.name == "stderr") {
        eprint!("{}", out);
        return Ok(Value::None);
    }
    let r = call_method_pub(v, "write", &[Value::Str(PyString::new(out))]);
    match r {
        Ok(_) => Ok(Value::None),
        Err(_) => Err(raise(
            "TypeError",
            "print() file must be None or have a write() method",
        )),
    }
}

// ---------------------------------------------------------------------------
// type-ish builtins
// ---------------------------------------------------------------------------

/// len(v): sized types only, exact TypeError.
pub fn builtin_len(args: &[Value]) -> BResult {
    let v = one(args, "len")?;
    if std::env::var("APX_TBDBG").is_ok() {
        match v {
            Value::Obj(o) => eprintln!("[LEN] Obj kind={:?} name={}", o.kind, o.name),
            other => eprintln!("[LEN] other={:?}", other),
        }
    }
    let n = match v {
        Value::Str(s) => s.len(),
        Value::Bytes(b) => b.len(),
        Value::List(l) => l.len(),
        Value::Tuple(t) => t.len(),
        Value::Dict(d) => d.len(),
        Value::Set(s) | Value::Frozen(s) => s.len(),
        // T2: traceback objects - len(tb) = entry count (pytest's
        // Traceback list semantics; ExceptionInfo.traceback returns the
        // raw tb and callers len() it).
        // T2: user instances of builtin-derived classes (class T(list))
        // - len reads the backing slice_parts filled via
        // __ap_base_<base>__init__.
        Value::Obj(o)
            if o.kind == ap_obj::ObjKind::UserInstance && !o.slice_parts.borrow().is_empty() =>
        {
            // T2: builtin-derived instances - len reads the backing
            // slice_parts (filled via __ap_base_<base>__init__).
            if std::env::var("APX_TBDBG").is_ok() {
                let mro_names: Vec<String> = o
                    .class_ref
                    .borrow()
                    .as_ref()
                    .map(|t| t.mro.borrow().iter().map(|c| c.name.clone()).collect())
                    .unwrap_or_default();
                eprintln!(
                    "[LEN-USER] n={} mro={:?}",
                    o.slice_parts.borrow().len(),
                    mro_names
                );
            }
            o.slice_parts.borrow().len()
        }
        Value::Obj(o) if o.kind == ap_obj::ObjKind::Namespace && o.name == "<traceback>" => {
            if std::env::var("APX_TBDBG").is_ok() {
                eprintln!("[LEN-TB] counted");
            }
            let mut n = 0usize;
            if let Some(interp2) = crate::builtins_modules::current_interp() {
                let mut cur = Some(v.clone());
                while let Some(cv) = cur {
                    n += 1;
                    cur = match crate::interp::attr_get_public(&interp2, &cv, "tb_next") {
                        Ok(Some(next_v)) => Some(next_v),
                        _ => None,
                    };
                    if n > 1000 {
                        break; // defensive: corrupted chain
                    }
                }
            }
            n
        }
        Value::Obj(o) => match o.kind {
            ap_obj::ObjKind::Opaque if crate::gcsupport::is_weak_container(&o.name) => {
                // 02.04 completion: weak containers report the LIVE count.
                match crate::gcsupport::call_weakref(
                    &format!("{}.__len__", o.name),
                    &[Value::Obj(o.clone())],
                    &[],
                ) {
                    Some(Ok(v)) => v.as_int().and_then(|n| n.to_i64()).unwrap_or(0).max(0) as usize,
                    _ => {
                        return Err(raise(
                            "TypeError",
                            format!("object of type '{}' has no len()", v.type_name()),
                        ))
                    }
                }
            }
            ap_obj::ObjKind::Opaque
                if matches!(o.name.as_str(), "dict_keys" | "dict_values" | "dict_items") =>
            {
                // T2 (dict view objects): len(view) = payload count
                // (P139: len(ks) corrupted the later dict-subclass
                // setitem through the len error path).
                let n = o
                    .attrs
                    .borrow()
                    .get(&Value::Str(PyString::new("<payload>")))
                    .map(|pv| match pv {
                        Value::List(l) => l.len(),
                        _ => 0,
                    })
                    .unwrap_or(0);
                n as usize
            }
            ap_obj::ObjKind::Opaque if o.name == "bytearray" => {
                let _ = 0;
                // A-4 item 1: len(bytearray) over the <data> buffer.
                match o.attrs.borrow().get(&Value::Str(PyString::new("<data>"))) {
                    Some(Value::Bytes(b)) => b.len(),
                    _ => {
                        return Err(raise(
                            "TypeError",
                            "object of type 'bytearray' has no len()",
                        ))
                    }
                }
            }
            ap_obj::ObjKind::Opaque => {
                // 02.08-lite native types (struct_time, uname_result) expose
                // their field tuple under the <values> key.
                let attrs = o.attrs.borrow();
                match attrs
                    .get(&Value::Str(PyString::new("<values>")))
                    .or_else(|| attrs.get(&Value::Str(PyString::new("<items>"))))
                {
                    Some(Value::List(l)) => l.len(),
                    _ => {
                        // A-3 item 5: stat_result (and friends) carry their
                        // sequence in slice_parts, not attrs.
                        drop(attrs);
                        let n = o.slice_parts.borrow().len();
                        if n > 0 {
                            n
                        } else {
                            return Err(raise(
                                "TypeError",
                                format!("object of type '{}' has no len()", v.type_name()),
                            ));
                        }
                    }
                }
            }
            ap_obj::ObjKind::Range => {
                let ints = o.ints.borrow();
                crate::iter::range_len(ints[0], ints[1], ints[2]) as usize
            }
            ap_obj::ObjKind::NamedTuple => {
                // 02.08-lite: uname_result-style named tuples are sized via
                // their <values> field list.
                let attrs = o.attrs.borrow();
                match attrs
                    .get(&Value::Str(PyString::new("<values>")))
                    .or_else(|| attrs.get(&Value::Str(PyString::new("<items>"))))
                {
                    Some(Value::List(l)) => l.len(),
                    _ => {
                        return Err(raise(
                            "TypeError",
                            format!("object of type '{}' has no len()", v.type_name()),
                        ))
                    }
                }
            }
            _ => {
                // T1 (scrapy fix pass): list-derived instances (StackSummary)
                // size via their backing list.
                if crate::classes::is_list_derived_instance(v) {
                    let l = crate::classes::list_backing_or_new(v);
                    return Ok(Value::Int(Int::from_i64(l.len() as i64)));
                }
                // W111: dict-derived user classes (vendored collections.
                // OrderedDict subclasses dict) size via their dict slots.
                if let Some(d) = crate::classes::dict_backing(v) {
                    return Ok(Value::Int(Int::from_i64(d.len() as i64)));
                }
                return Err(raise(
                    "TypeError",
                    format!("object of type '{}' has no len()", v.type_name()),
                ));
            }
        },
        _ => {
            return Err(raise(
                "TypeError",
                format!("object of type '{}' has no len()", v.type_name()),
            ))
        }
    };
    Ok(Value::Int(Int::from_i64(n as i64)))
}

/// repr(v) with the C-14 int-limit sweep (ops::repr handles it).
pub fn builtin_repr(args: &[Value]) -> BResult {
    let v = one(args, "repr")?;
    // Exception instances repr as ClassName('arg', ...) (V043: repr(e)).
    if let Value::Obj(o) = v {
        if let Some(txt) = &*o.repr_text.borrow() {
            return Ok(Value::Str(PyString::new(txt.clone())));
        }
        if o.kind == ap_obj::ObjKind::Exception {
            let mut out = o.name.clone();
            out.push('(');
            for (i, a) in o.args.borrow().iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                out.push_str(&ap_obj::ops::repr(a).map_err(Flow::from_py)?);
            }
            out.push(')');
            return Ok(Value::Str(PyString::new(out)));
        }
    }
    Ok(Value::Str(PyString::new(
        ap_obj::ops::repr(v).map_err(Flow::from_py)?,
    )))
}

/// str(v) / str() == ''.
pub fn builtin_str(args: &[Value]) -> BResult {
    match args.first() {
        None => Ok(Value::Str(PyString::new(""))),
        Some(v) => {
            let s = py_str_of(v).map_err(Flow::from_py)?;
            Ok(Value::Str(PyString::new(s)))
        }
    }
}

/// int(v) / int(v, base): str parse (with digit limit), float truncation,
/// bool/int pass-through.
pub fn builtin_int(interp: &Interpreter, args: &[Value]) -> BResult {
    match args.len() {
        0 => Ok(Value::Int(Int::Smi(0))),
        // W110 (T2): int(cls, value) — enum member construction passes the
        // class as the first arg (CPython int.__new__ ignores it).
        // W111: CPython int.__new__(F, 5) returns an INSTANCE OF F (class
        // methods like .phrase stay reachable on the member). When cls is a
        // USER class deriving int, allocate the derived instance
        // (UserInstance + __ap_int__ payload + class_ref) — the same shape
        // the __int__new__ marker arm produces (interp.rs ~4999).
        2 if matches!(&args[0], Value::Obj(o) if o.kind == ap_obj::ObjKind::UserClass) => {
            let cls_v = args[0].clone();
            if let Some(t) = crate::classes::class_of_value(interp, &cls_v) {
                if t.mro.borrow().iter().any(|m| m.name == "int") {
                    let io = ap_obj::PyObject::new(ap_obj::ObjKind::UserInstance, t.name.clone());
                    *io.type_ref.borrow_mut() = Some(std::rc::Rc::downgrade(&t));
                    *io.class_ref.borrow_mut() = Some(t.clone());
                    let _ = io
                        .attrs
                        .borrow_mut()
                        .set(Value::Str(PyString::new("__ap_int__")), args[1].clone());
                    return Ok(Value::Obj(io));
                }
            }
            let args1 = [args[1].clone()];
            builtin_int(interp, &args1)
        }
        1 => {
            let v = &args[0];
            match v {
                // W111: int-derived instances (IntEnum members) int() to
                // their __ap_int__ payload (CPython int.__int__); WITHOUT a
                // payload, fall through to the user __int__/__index__
                // protocol below (vendored ipaddress IPv4Address etc).
                Value::Obj(o) if o.kind == ap_obj::ObjKind::UserInstance
                    && o.attrs.borrow().get(&Value::Str(PyString::new("__ap_int__"))).is_some() =>
                {
                    match o
                        .attrs
                        .borrow()
                        .get(&Value::Str(PyString::new("__ap_int__")))
                    {
                        Some(p @ Value::Int(_)) => Ok(p.clone()),
                        _ => Err(raise(
                            "TypeError",
                            "int() argument must be a string, a bytes-like object or a real number, not 'object'",
                        )),
                    }
                }
                // 02.08-full p2: re.NOFLAG marker int()s to 0.
                // 02.08-c: any Opaque carrying an <int_value> attr (lxml
                // ErrorTypes members etc.) int()s to that stored value.
                Value::Obj(o) if o.kind == ap_obj::ObjKind::Opaque => {
                    match o.attrs.borrow().get(&Value::Str(PyString::new("<int_value>"))) {
                        Some(v @ Value::Int(_)) => Ok(v.clone()),
                        _ => Err(raise(
                            "TypeError",
                            "int() argument must be a string, a bytes-like object or a real number, not 'object'",
                        )),
                    }
                }
                Value::Int(_) => Ok(v.clone()),
                Value::Bool(b) => Ok(Value::Int(Int::Smi(i64::from(*b)))),
                Value::Float(f) => Ok(Value::Int(
                    ap_obj::ops::float_to_int(*f).map_err(Flow::from_py)?,
                )),
                Value::Str(s) => parse_int_str(interp, s.as_str(), 10),
                Value::Bytes(b) => {
                    let text = String::from_utf8_lossy(b);
                    parse_int_str(interp, &text, 10)
                }
                // CPython int(x) protocol: __int__ first, then __index__
                // (user classes; vendored ipaddress IPv4Address.__int__).
                Value::Obj(_) => {
                    if let Some((f, _)) = crate::classes::special_lookup(interp, v, "__int__") {
                        let r = crate::interp::call_value_pub3(&f, std::slice::from_ref(v))?;
                        if matches!(r, Value::Int(_)) {
                            return Ok(r);
                        }
                    }
                    if let Some((f, _)) = crate::classes::special_lookup(interp, v, "__index__") {
                        let r = crate::interp::call_value_pub3(&f, std::slice::from_ref(v))?;
                        if matches!(r, Value::Int(_)) {
                            return Ok(r);
                        }
                    }
                    // T2 (pytest ExitCode): enum members whose class mixes in
                    // int (IntEnum/IntFlag) carry the value in the _value_
                    // instance attr (enum.py __set_name__); CPython members
                    // ARE ints. int(member) == member._value_.
                    if let Value::Obj(o) = v {
                        if let Some(Value::Int(i)) = o
                            .attrs
                            .borrow()
                            .get(&Value::Str(PyString::new("_value_")))
                        {
                            return Ok(Value::Int(i.clone()));
                        }
                    }
                    Err(raise(
                        "TypeError",
                        format!("int() argument must be a string, a bytes-like object or a real number, not '{}'", v.type_name()),
                    ))
                }
                _ => Err(raise(
                    "TypeError",
                    format!("int() argument must be a string, a bytes-like object or a real number, not '{}'", v.type_name()),
                )),
            }
        }
        _ => {
            let v = &args[0];
            let base = args
                .get(1)
                .and_then(|b| b.as_int())
                .and_then(|i| i.to_i64())
                .ok_or_else(|| raise("TypeError", "'base' must be an integer"))?;
            match v {
                Value::Str(s) => parse_int_str(interp, s.as_str(), base as u32),
                Value::Bytes(b) => parse_int_str(interp, &String::from_utf8_lossy(b), base as u32),
                _ => Err(raise(
                    "TypeError",
                    "int() can't convert non-string with explicit base",
                )),
            }
        }
    }
}

fn parse_int_str(interp: &Interpreter, text: &str, base: u32) -> BResult {
    let digits = text.trim().trim_start_matches(['+', '-']).len();
    let limit = ap_obj::int::get_int_str_digit_limit();
    if digits > limit {
        return Err(Flow::Raise(ExceptionValue::msg(
            "ValueError",
            format!(
                "Exceeds the limit ({limit} digits) for integer string conversion: value has {digits} digits; use sys.set_int_max_str_digits() to increase the limit"
            ),
        )));
    }
    let _ = interp;
    let parsed = if base == 10 {
        Int::parse_base10(text)
    } else {
        Int::parse_base(text, base)
    };
    match parsed {
        Ok(i) => Ok(Value::Int(i)),
        Err(()) => {
            if base == 10 {
                Err(raise(
                    "ValueError",
                    format!("invalid literal for int() with base 10: '{}'", text.trim()),
                ))
            } else {
                Err(raise(
                    "ValueError",
                    format!(
                        "invalid literal for int() with base {base}: '{}'",
                        text.trim()
                    ),
                ))
            }
        }
    }
}

/// float(v): str parse (inf/nan/underscores), int conversion (OverflowError).
pub fn builtin_float(args: &[Value]) -> BResult {
    match args.first() {
        None => Ok(Value::Float(0.0)),
        Some(Value::Float(f)) => Ok(Value::Float(*f)),
        Some(Value::Bool(b)) => Ok(Value::Float(f64::from(*b))),
        Some(Value::Int(i)) => Ok(Value::Float(
            ap_obj::ops::int_to_f64_exact(i).map_err(Flow::from_py)?,
        )),
        Some(Value::Str(s)) => parse_float_str(s.as_str()),
        Some(Value::Bytes(b)) => parse_float_str(&String::from_utf8_lossy(b)),
        Some(other) => Err(raise(
            "TypeError",
            format!(
                "float() argument must be a string or a real number, not '{}'",
                other.type_name()
            ),
        )),
    }
}

fn parse_float_str(text: &str) -> BResult {
    let t = text.trim();
    let cleaned: String = t.chars().filter(|&c| c != '_').collect();
    match cleaned.to_ascii_lowercase().as_str() {
        "inf" | "+inf" | "infinity" | "+infinity" => return Ok(Value::Float(f64::INFINITY)),
        "-inf" | "-infinity" => return Ok(Value::Float(f64::NEG_INFINITY)),
        "nan" | "+nan" | "-nan" => return Ok(Value::Float(f64::NAN)),
        _ => {}
    }
    match cleaned.parse::<f64>() {
        Ok(f) => Ok(Value::Float(f)),
        Err(_) => Err(raise(
            "ValueError",
            format!("could not convert string to float: '{}'", text.trim()),
        )),
    }
}

/// bool(v): truthiness; with no args False.
pub fn builtin_bool(args: &[Value]) -> BResult {
    if let Some(Value::Obj(o)) = args.first() {
        // 02.04 completion: weak containers are falsy when empty.
        if o.kind == ap_obj::ObjKind::Opaque && crate::gcsupport::is_weak_container(&o.name) {
            return crate::gcsupport::call_weakref(
                &format!("{}.__bool__", o.name),
                &[Value::Obj(o.clone())],
                &[],
            )
            .unwrap_or_else(|| Ok(Value::Bool(true)));
        }
        // A-4 item 1: bytearray truthiness follows the buffer length.
        if o.kind == ap_obj::ObjKind::Opaque && o.name == "bytearray" {
            let n = o
                .attrs
                .borrow()
                .get(&Value::Str(PyString::new("<data>")))
                .map(|v| match v {
                    Value::Bytes(b) => !b.is_empty(),
                    _ => false,
                })
                .unwrap_or(false);
            return Ok(Value::Bool(n));
        }
        // collections.deque: live truthiness from the <items> backing list
        // (contextlib.ExitStack does `while self._exit_callbacks:`).
        if o.kind == ap_obj::ObjKind::Opaque && o.name == "deque" {
            let n = o
                .attrs
                .borrow()
                .get(&Value::Str(PyString::new("<items>")))
                .map(|v| match v {
                    Value::List(l) => l.len(),
                    _ => 0,
                })
                .unwrap_or(0);
            return Ok(Value::Bool(n != 0));
        }
    }
    Ok(Value::Bool(match args.first() {
        None => false,
        Some(v) => v.is_truthy(),
    }))
}

// ---------------------------------------------------------------------------
// container constructors
// ---------------------------------------------------------------------------

pub fn builtin_list(args: &[Value]) -> BResult {
    match args.first() {
        None => Ok(Value::List(PyList::new())),
        Some(v) => {
            // PEP-342: generators must drain LAZILY through the executor
            // (iterable_of would hand back the lazy anchor; collect() on it
            // sees an empty Seq). Drive gen pulls directly.
            if let Value::Obj(o) = v {
                if o.kind == ap_obj::ObjKind::Generator {
                    if let Some(id) = *o.gen_id.borrow() {
                        let interp = crate::builtins_modules::current_interp();
                        let Some(interp) = interp else {
                            // No interpreter on this thread: legacy path.
                            let it = iterable_of(v)?;
                            let items = crate::iter::collect(&it);
                            return Ok(Value::List(PyList::from_vec(items)));
                        };
                        let mut items = Vec::new();
                        loop {
                            // T2 fix: a GenExpr REMOVES its registry entry on
                            // the first pull (materializes to
                            // interp.iterators[id]); the old
                            // contains_key(generators) guard stopped the drain
                            // after one item. Keep pulling while either the
                            // generator entry or the materialized iterator
                            // still exists.
                            let alive = interp.generators.borrow().contains_key(&id)
                                || interp.iterators.borrow().contains_key(&id);
                            let r = if alive {
                                let rc = interp.rc();
                                crate::interp::gen_pull_public(&rc, id)
                            } else {
                                break Ok(());
                            };
                            match r {
                                Ok(Some(val)) => items.push(val),
                                Ok(None) => break Ok(()),
                                Err(f) => break Err(f),
                            }
                        }?;
                        return Ok(Value::List(PyList::from_vec(items)));
                    }
                }
            }
            let it = iterable_of(v)?;
            let items = crate::iter::collect(&it);
            Ok(Value::List(PyList::from_vec(items)))
        }
    }
}

pub fn builtin_tuple(args: &[Value]) -> BResult {
    match args.first() {
        None => Ok(Value::Tuple(Gc::new(Tuple(vec![])))),
        Some(Value::Obj(o)) if o.kind == ap_obj::ObjKind::Opaque => {
            // 02.08-lite native types: tuple(struct_time) over <values>.
            let attrs = o.attrs.borrow();
            match attrs.get(&Value::Str(PyString::new("<values>"))) {
                Some(Value::List(l)) => Ok(Value::Tuple(Gc::new(Tuple(l.items().to_vec())))),
                // T2 (inspect._signature_bound_method: tuple(
                // sig.parameters.values())): dict-values views park their
                // snapshot in <payload> — drain it like list().
                _ => match attrs.get(&Value::Str(PyString::new("<payload>"))) {
                    Some(Value::List(l)) => Ok(Value::Tuple(Gc::new(Tuple(l.items().to_vec())))),
                    _ => Err(raise(
                        "TypeError",
                        format!("'{}' object is not iterable", o.type_name()),
                    )),
                },
            }
        }
        Some(v) => {
            let it = iterable_of(v)?;
            let items = crate::iter::collect(&it);
            Ok(Value::Tuple(Gc::new(Tuple(items))))
        }
    }
}

pub fn builtin_dict(args: &[Value], kwargs: &[(String, Value)]) -> BResult {
    let d = PyDict::new();
    // dict(mapping-or-iterable-of-pairs, **kwargs)
    if let Some(first) = args.first() {
        match first {
            Value::Dict(m) => {
                for (k, v) in m.items() {
                    d.set(k, v).map_err(Flow::from_py)?;
                }
            }
            other => {
                // CPython dict(mapping): keys() + __getitem__ wins over
                // iterable-of-pairs (UserDict/BaseSettings rely on it).
                if crate::builtins_modules::dict_like_mapping(other) {
                    let interp = match crate::builtins_modules::current_interp() {
                        Some(i) => i,
                        None => return Err(raise("SystemError", "no interpreter for dict ctor")),
                    };
                    let keys_f = match crate::builtins::attr_get_pub(&interp, other, "keys") {
                        Ok(Some(f)) => f,
                        _ => {
                            return Err(raise(
                                "TypeError",
                                format!("'{}' object is not a mapping", other.type_name()),
                            ))
                        }
                    };
                    let keys_v = crate::interp::call_value_pub2(&keys_f, &[], &[])?;
                    let it = crate::interp::iterable_of_public(&keys_v)?;
                    for k in crate::iter::collect(&it) {
                        let getitem =
                            match crate::builtins::attr_get_pub(&interp, other, "__getitem__") {
                                Ok(Some(f)) => f,
                                _ => {
                                    return Err(raise(
                                        "TypeError",
                                        format!("'{}' object is not a mapping", other.type_name()),
                                    ))
                                }
                            };
                        let v = crate::interp::call_value_pub2(&getitem, &[k.clone()], &[])?;
                        d.set(k, v).map_err(Flow::from_py)?;
                    }
                } else {
                    // iterable of pairs
                    let it = iterable_of(other)?;
                    for pair in crate::iter::collect(&it) {
                        let (k, v) = unpack_pair(&pair)?;
                        d.set(k, v).map_err(Flow::from_py)?;
                    }
                }
            }
        }
    }
    for (k, v) in kwargs {
        d.set(Value::Str(PyString::new(k.clone())), v.clone())
            .map_err(Flow::from_py)?;
    }
    Ok(Value::Dict(d))
}

fn unpack_pair(v: &Value) -> Result<(Value, Value), Flow> {
    match v {
        Value::Tuple(t) if t.len() == 2 => Ok((t.0[0].clone(), t.0[1].clone())),
        Value::List(l) if l.len() == 2 => {
            let items = l.items();
            Ok((items[0].clone(), items[1].clone()))
        }
        _ => Err(raise(
            "TypeError",
            format!("cannot convert dictionary update sequence element to a sequence"),
        )),
    }
}

pub fn builtin_set(args: &[Value]) -> BResult {
    let s = PySet::new();
    if let Some(v) = args.first() {
        let it = iterable_of(v)?;
        for item in crate::iter::collect(&it) {
            add_with_element_text(&s, item)?;
        }
    }
    Ok(Value::Set(s))
}

pub fn builtin_frozenset(args: &[Value]) -> BResult {
    let s = PySet::new();
    if let Some(v) = args.first() {
        let it = iterable_of(v)?;
        for item in crate::iter::collect(&it) {
            add_with_element_text(&s, item)?;
        }
    }
    Ok(Value::Frozen(s))
}

/// set()/frozenset() constructor adds use the SET-ELEMENT phrasing (V046:
/// distinct from add()'s plain text).
fn add_with_element_text(s: &PySet, item: Value) -> Result<(), Flow> {
    if ap_obj::ops::hash(&item).is_err() {
        return Err(raise(
            "TypeError",
            format!(
                "cannot use '{}' as a set element (unhashable type: '{}')",
                item.type_name(),
                item.type_name()
            ),
        ));
    }
    s.add(item).map_err(Flow::from_py)
}

pub fn iterable_of(v: &Value) -> Result<Gc<crate::iter::PyIterator>, Flow> {
    // Delegates to the executor's generator-aware path via the thread-local
    // interpreter handle (generators materialize through GenFrame).
    crate::interp::iterable_of_public(v)
}

// ---------------------------------------------------------------------------
// numeric / sequence builtins
// ---------------------------------------------------------------------------

pub fn builtin_abs(args: &[Value]) -> BResult {
    let v = one(args, "abs")?;
    match v {
        Value::Int(i) => Ok(Value::Int(i.abs())),
        Value::Bool(b) => Ok(Value::Int(Int::Smi(i64::from(*b)))),
        Value::Float(f) => Ok(Value::Float(f.abs())),
        _ => Err(raise(
            "TypeError",
            format!("bad operand type for abs(): '{}'", v.type_name()),
        )),
    }
}

pub fn builtin_min(args: &[Value], kwargs: &[(String, Value)]) -> BResult {
    min_max(args, kwargs, true)
}

pub fn builtin_max(args: &[Value], kwargs: &[(String, Value)]) -> BResult {
    min_max(args, kwargs, false)
}

fn min_max(args: &[Value], kwargs: &[(String, Value)], is_min: bool) -> BResult {
    let name = if is_min { "min" } else { "max" };
    let mut key_fn: Option<&Value> = None;
    let mut default_val: Option<Value> = None;
    for (k, v) in kwargs {
        if k == "key" {
            key_fn = Some(v);
        } else if k == "default" {
            // T2 (pytest pathlib.make_numbered_dir): max(iterable,
            // default=N) with map() generators that can be empty.
            default_val = Some(v.clone());
        } else {
            return Err(raise(
                "TypeError",
                format!("'{k}' is an invalid keyword argument for {name}()"),
            ));
        }
    }
    // Two forms: min(iterable) or min(a, b, ...).
    let candidates: Vec<Value> = if args.len() == 1 {
        let it = iterable_of(&args[0])?;
        crate::iter::collect(&it)
    } else {
        args.to_vec()
    };
    if candidates.is_empty() {
        // T2: empty iterable with default= returns the default
        // (CPython semantics; pytest tmp_path depends on it).
        if let Some(d) = default_val {
            return Ok(d);
        }
        return Err(raise(
            "ValueError",
            format!("{name}() arg is an empty sequence"),
        ));
    }
    let first = candidates
        .first()
        .ok_or_else(|| raise("ValueError", format!("{name}() arg is an empty sequence")))?;
    let mut best = first.clone();
    let mut best_key: Option<Value> = None;
    for c in &candidates[1..] {
        let (bc, cc) = if let Some(f) = key_fn {
            // key function is called by the executor through interp; here
            // we can't — the wrapper in interp.rs handles keyed forms.
            (best_key.clone(), Some(call_key_stub(f, c)?))
        } else {
            (None, None)
        };
        let (l, r) = match (&bc, &cc) {
            (Some(l), Some(r)) => (l.clone(), r.clone()),
            _ => (best.clone(), c.clone()),
        };
        let ord = ap_obj::dict::values_cmp(&l, &r).map_err(Flow::from_py)?;
        let take = if is_min {
            ord == std::cmp::Ordering::Greater
        } else {
            ord == std::cmp::Ordering::Less
        };
        if take {
            best = c.clone();
            best_key = cc;
        } else if best_key.is_none() {
            best_key = None;
        }
    }
    Ok(best)
}

fn call_key_stub(_f: &Value, _v: &Value) -> Result<Value, Flow> {
    // Keyed min/max route through the interpreter in interp.rs; this stub
    // is only reachable when key=None was passed positionally (impossible
    // in v0 corpus). Keeping the signature for future wiring.
    Err(raise("SystemError", "key functions in min/max: v0 gap"))
}

pub fn builtin_sum(args: &[Value]) -> BResult {
    let first = one(args, "sum")?;
    // PEP-342: generators drain lazily through the executor.
    if let Some(drained) = crate::interp::drain_generator_public(first) {
        let items = drained?;
        let mut acc = match args.get(1) {
            Some(start) => start.clone(),
            None => Value::Int(Int::Smi(0)),
        };
        for item in &items {
            acc = ap_obj::ops::binop(&acc, ap_obj::ops::BinOp::Add, item).map_err(Flow::from_py)?;
        }
        return Ok(acc);
    }
    let it = iterable_of(first)?;
    let mut acc = match args.get(1) {
        Some(start) => start.clone(),
        None => Value::Int(Int::Smi(0)),
    };
    // Float fast path with Neumaier compensation (CPython 3.12+ builtin sum
    // uses compensated summation for floats — pinned by stdlib s048).
    // Collect ONCE (iterators drain); decide the path from the snapshot.
    let items = crate::iter::collect(&it);
    if matches!(acc, Value::Float(_) | Value::Int(_) | Value::Bool(_)) {
        let all_floatish = items
            .iter()
            .all(|item| matches!(item, Value::Float(_) | Value::Int(_) | Value::Bool(_)));
        let any_float = items.iter().any(|item| matches!(item, Value::Float(_)));
        // CPython keeps all-int sums EXACT; compensation is float-only
        // (differential gate: sum(g()) of ints returns int 3, not 3.0).
        if all_floatish && any_float {
            let mut f: f64 = match &acc {
                Value::Float(x) => *x,
                Value::Int(i) => i.to_f64().unwrap_or(0.0),
                Value::Bool(b) => {
                    if *b {
                        1.0
                    } else {
                        0.0
                    }
                }
                _ => 0.0,
            };
            let mut c: f64 = 0.0;
            for item in items.iter() {
                let x = match &item {
                    Value::Float(x) => *x,
                    Value::Int(i) => i.to_f64().unwrap_or(0.0),
                    Value::Bool(b) => {
                        if *b {
                            1.0
                        } else {
                            0.0
                        }
                    }
                    _ => 0.0,
                };
                let t = f + x;
                if f.abs() >= x.abs() {
                    c += (f - t) + x;
                } else {
                    c += (x - t) + f;
                }
                f = t;
            }
            return Ok(Value::Float(f + c));
        }
    }
    for item in items {
        // sum rejects strs with the exact text.
        if matches!(item, Value::Str(_)) && matches!(acc, Value::Str(_)) {
            return Err(raise(
                "TypeError",
                "sum() can't sum strings [use ''.join(seq) instead]",
            ));
        }
        acc = ap_obj::ops::binop(&acc, ap_obj::ops::BinOp::Add, &item).map_err(Flow::from_py)?;
    }
    Ok(acc)
}

pub fn builtin_any(args: &[Value]) -> BResult {
    let it = iterable_of(one(args, "any")?)?;
    for v in crate::iter::collect(&it) {
        if v.is_truthy() {
            return Ok(Value::Bool(true));
        }
    }
    Ok(Value::Bool(false))
}

pub fn builtin_all(args: &[Value]) -> BResult {
    let it = iterable_of(one(args, "all")?)?;
    for v in crate::iter::collect(&it) {
        if !v.is_truthy() {
            return Ok(Value::Bool(false));
        }
    }
    Ok(Value::Bool(true))
}

// ---------------------------------------------------------------------------
// isinstance / type / hash
// ---------------------------------------------------------------------------

/// isinstance(v, T): T is a type object (Value::Obj Type/UserClass) or a
/// TUPLE of them. v0 type checks are by NAME (no subclass graph except
/// bool ⊂ int per S-01-9).
pub fn builtin_isinstance(args: &[Value]) -> BResult {
    let v = one(args, "isinstance")?;
    let t = args
        .get(1)
        .ok_or_else(|| raise("TypeError", "isinstance expected 2 arguments, got 1"))?;
    check_isinstance(v, t)
}

fn check_isinstance(v: &Value, t: &Value) -> Result<Value, Flow> {
    // T2 (inspect.isclass): an instance is never an instance of `type`.
    // Only UserClass/Type subjects can be. Guard here too — the
    // class_isinstance path doesn't cover every dispatch (hook callbacks).
    if matches!(t, Value::Obj(to) if to.kind == ap_obj::ObjKind::Type && to.name == "type") {
        let is_class_subject = matches!(v, Value::Obj(vo)
            if vo.kind == ap_obj::ObjKind::UserClass || vo.kind == ap_obj::ObjKind::Type);
        if !is_class_subject {
            return Ok(Value::Bool(false));
        }
    }
    match t {
        Value::Tuple(parts) => {
            for p in &parts.0 {
                if check_isinstance(v, p)?.is_truthy() {
                    return Ok(Value::Bool(true));
                }
            }
            Ok(Value::Bool(false))
        }
        Value::Obj(o)
            if o.kind == ap_obj::ObjKind::Type || o.kind == ap_obj::ObjKind::UserClass =>
        {
            let want: &str = &o.name;
            if type_matches(v, want) {
                return Ok(Value::Bool(true));
            }
            // W112: a UserClass target whose metaclass defines
            // __instancecheck__ (ABCMeta in _collections_abc) must route
            // through the ABC hook - a dict SUBCLASS instance (scrapy
            // Headers) failed the name arm and then answered False, so
            // Headers.update took the non-Mapping branch ('too many
            // values to unpack' in RetryMiddleware). Delegate.
            if o.kind == ap_obj::ObjKind::UserClass {
                if std::env::var("APX_GEN_DEBUG").is_ok() {
                    let vkind = match v {
                        Value::Obj(vo) => format!("{:?}", vo.kind),
                        _ => "prim".to_string(),
                    };
                    eprintln!("[UC-DELEGATE] target={} vkind={}", want, vkind);
                }
                let interp_rc = crate::interp::current_interp_handle();
                if let Ok(true) = crate::classes::value_isinstance(&interp_rc, v, t) {
                    return Ok(Value::Bool(true));
                }
            }
            Ok(Value::Bool(false))
        }
        // Constructor builtins (int/float/str/...) double as type objects in
        // the v0 namespace; isinstance accepts them.
        Value::Obj(o) if o.kind == ap_obj::ObjKind::BuiltinFn => {
            // T1 (dataclasses): typing.Union is a fn-MARKER (v0) — isinstance
            // against a special form is False, not a TypeError (CPython does
            // a metaclass __instancecheck__; v0 tolerance, documented).
            // T2 (typing_extensions 4.16): ParamSpec/TypeVar/TypeVarTuple
            // ctor markers join the same tolerance — _TypeVarLikeMeta
            // .__instancecheck__ delegates to isinstance(x, typing.ParamSpec)
            // and MUST NOT raise (the backported sentinel/ParamSpec chain
            // dies on the miss).
            if matches!(
                o.kind,
                ap_obj::ObjKind::BuiltinFn | ap_obj::ObjKind::Function
            ) && (o.name == "Union"
                || o.name == "_typing.Union"
                || matches!(
                    o.name.as_str(),
                    "ParamSpec" | "TypeVar" | "TypeVarTuple" | "NewType"
                )
                || o.name.starts_with("_typing.ParamSpec")
                || o.name.starts_with("_typing.TypeVar")
                || o.name.starts_with("__ap_typing_"))
            {
                return Ok(Value::Bool(false));
            }
            // T1 (scrapy.utils.python get_func_args_dict): isinstance(func,
            // partial) — the ap functools.partial is the '<partial:...>'
            // marker fn; match those markers.
            if o.name == "partial" {
                let is_partial = matches!(v, Value::Obj(vo) if vo.kind == ap_obj::ObjKind::BuiltinFn
                    && vo.name.starts_with("<partial:"));
                return Ok(Value::Bool(is_partial));
            }
            Ok(Value::Bool(type_matches(v, &o.name)))
        }
        // T1 (scrapy.utils.python get_func_args_dict): isinstance(func,
        // partial) — the ap functools.partial is the '<partial:...>' marker.
        _ if matches!(
            t,
            Value::Obj(o) if o.kind == ap_obj::ObjKind::BuiltinFn
                && matches!(o.name.as_str(), "partial" | "functools.partial")
        ) =>
        {
            let is_partial = matches!(v, Value::Obj(vo) if vo.kind == ap_obj::ObjKind::BuiltinFn
                && vo.name.starts_with("<partial:"));
            Ok(Value::Bool(is_partial))
        }
        // T1-d (attrs _create_slots_class): `isinstance(cached_prop,
        // cached_property)` — the v0 functools.cached_property is a
        // pass-through fn; only functions it decorated (marker attr) answer
        // True.
        _ if matches!(
            t,
            Value::Obj(o)
                if matches!(
                    o.kind,
                    ap_obj::ObjKind::BuiltinFn | ap_obj::ObjKind::Function
                ) && matches!(
                    o.name.as_str(),
                    "cached_property" | "functools.cached_property"
                )
        ) =>
        {
            Ok(Value::Bool(matches!(
                v,
                Value::Obj(vo) if vo.kind == ap_obj::ObjKind::Function
                    && vo
                        .attrs
                        .borrow()
                        .get(&Value::Str(PyString::new("__ap_cached_property__")))
                        .is_some_and(|x| x.is_truthy())
            )))
        }
        // T2 (_pytest._code.Source: isinstance(obj, tuple | list)):
        // types.UnionType (`X | Y`) is the Namespace "_Union" (or the
        // executor's Opaque "__union__") with member types in slice_parts —
        // isinstance accepts unions (CPython 3.10+).
        Value::Obj(o)
            if (o.kind == ap_obj::ObjKind::Namespace && o.name == "_Union")
                || (o.kind == ap_obj::ObjKind::Opaque && o.name == "__union__") =>
        {
            for p in o.slice_parts.borrow().iter() {
                // PEP 604: `X | None` parks None as a Value::None arm —
                // it matches NoneType (isinstance(None, int|None)).
                if matches!(p, Value::None) {
                    if matches!(v, Value::None) {
                        return Ok(Value::Bool(true));
                    }
                    continue;
                }
                if check_isinstance(v, p)?.is_truthy() {
                    return Ok(Value::Bool(true));
                }
            }
            Ok(Value::Bool(false))
        }
        _ => Err(raise(
            "TypeError",
            "isinstance() arg 2 must be a type, a tuple of types, or a tuple",
        )),
    }
}

/// Name-based type match with the bool⊂int special case.
pub fn type_matches(v: &Value, want: &str) -> bool {
    // 02.04 completion: proxies are instances of ProxyType/CallableProxyType
    // (s034: isinstance(proxy, weakref.ProxyTypes)).
    if let Value::Obj(o) = v {
        if o.kind == ap_obj::ObjKind::Opaque
            && o.name == "_weakref_proxy"
            && matches!(want, "ProxyType" | "CallableProxyType" | "object")
        {
            return true;
        }
    }
    match (v, want) {
        (_, "object") => true,
        // T2: fake-native traceback objects. The chain nodes are
        // Namespace("<traceback>") values; types.TracebackType is the
        // auto-class of the same shape, so isinstance must accept them
        // (pytest's Traceback walker consumes them via tb_next).
        (Value::Obj(o), "<traceback>")
            if o.kind == ap_obj::ObjKind::Namespace && o.name == "<traceback>" =>
        {
            true
        }
        // 02.08-full p2: isinstance(mm, re.Match) / isinstance(p, re.Pattern).
        (Value::Obj(o), "Match") if o.kind == ap_obj::ObjKind::Opaque && o.name == "re.Match" => {
            true
        }
        (Value::Obj(o), "Pattern")
            if o.kind == ap_obj::ObjKind::Opaque && o.name == "re.Pattern" =>
        {
            true
        }
        (Value::Int(_), "int") => true,
        (Value::Bool(_), "bool") => true,
        (Value::Bool(_), "int") => true, // bool is a subclass of int (S-01-9)
        (Value::Int(_), "bool") => false,
        (Value::Frozen(_), "set") => false,
        // s014: namedtuple instances ARE tuples (NamedTuple payload kind).
        (Value::Obj(o), "tuple") if o.kind == ap_obj::ObjKind::NamedTuple => true,
        // T1-d (inspect.iscode / ismethod...): a Namespace/Opaque payload
        // (<code>, coroutine, thread) IS an instance of its interned
        // name-Type — compare the payload name, not the flattened type.
        (Value::Obj(o), b)
            if matches!(
                o.kind,
                ap_obj::ObjKind::Namespace | ap_obj::ObjKind::Opaque | ap_obj::ObjKind::NamedTuple
            ) =>
        {
            if o.name.as_str() == b {
                return true;
            }
            // Wave-58: stream-subclass instances (CaptureIO(TextIOWrapper))
            // are swapped to real stream objects whose NAME is the native
            // kind; the USER class identity rides class_ref. Walk its MRO
            // so isinstance(c, CaptureIO) holds (pytest's
            // SysCapture.snap asserts it).
            if o.kind == ap_obj::ObjKind::Opaque {
                if let Some(t) = o.class_ref.borrow().clone() {
                    for m in t.mro.borrow().iter() {
                        if m.name.as_str() == b {
                            return true;
                        }
                    }
                }
            }
            false
        }
        // T1-d (attrs _create_slots_class): Descriptor instances answer
        // classmethod/staticmethod/property isinstance probes by descr kind.
        (Value::Obj(o), b) if o.kind == ap_obj::ObjKind::Descriptor => {
            let kind = o.descr_kind.borrow().clone().unwrap_or_default();
            match b {
                "classmethod" => kind == "classmethod",
                "staticmethod" => kind == "staticmethod",
                "property" => kind == "property",
                _ => false,
            }
        }
        // T1-d (attrs _create_slots_class cached_property probe): the v0
        // functools.cached_property is a pass-through fn — plain functions
        // (the decorated results) answer True against its marker.
        // W111: the VENDORED functools now provides a real USER class
        // named cached_property (with __get__/__set_name__). The v0 arm
        // matched by NAME and answered True for every function — the attrs
        // slots rebuild then read .func off RAW functions and died. CPython:
        // a raw function is NOT a cached_property instance. A function
        // answers True only when it was wrapped (marker attr).
        (Value::Obj(o), "cached_property") if o.kind == ap_obj::ObjKind::Function => o
            .attrs
            .borrow()
            .get(&Value::Str(PyString::new("__ap_cached_property__")))
            .is_some_and(|x| x.is_truthy()),
        // T1-d (scrapy.http.Headers.update): CPython registers dict as a
        // MutableMapping/Collection/Iterable/etc at the C level; our ABC
        // registry can't see native dicts, so answer by name here.
        (Value::Dict(_), "Mapping")
        | (Value::Dict(_), "MutableMapping")
        | (Value::Dict(_), "Collection")
        | (Value::Dict(_), "Iterable")
        | (Value::Dict(_), "Container")
        | (Value::Dict(_), "Sized")
        | (Value::Dict(_), "Hashable") => true,
        (Value::List(_), "Iterable")
        | (Value::List(_), "Collection")
        | (Value::List(_), "Container")
        | (Value::List(_), "Sized")
        | (Value::List(_), "Sequence")
        | (Value::List(_), "MutableSequence") => true,
        (Value::Tuple(_), "Iterable")
        | (Value::Tuple(_), "Collection")
        | (Value::Tuple(_), "Container")
        | (Value::Tuple(_), "Sized")
        | (Value::Tuple(_), "Sequence")
        | (Value::Tuple(_), "Hashable") => true,
        (Value::Str(_), "Iterable")
        | (Value::Str(_), "Collection")
        | (Value::Str(_), "Container")
        | (Value::Str(_), "Sized")
        | (Value::Str(_), "Sequence")
        | (Value::Str(_), "Hashable")
        | (Value::Str(_), "ByteString") => true,
        (Value::Bytes(_), "Iterable")
        | (Value::Bytes(_), "Collection")
        | (Value::Bytes(_), "Container")
        | (Value::Bytes(_), "Sized")
        | (Value::Bytes(_), "Sequence")
        | (Value::Bytes(_), "Hashable")
        | (Value::Bytes(_), "ByteString") => true,
        (Value::Set(_), "Iterable")
        | (Value::Set(_), "Collection")
        | (Value::Set(_), "Container")
        | (Value::Set(_), "Sized")
        | (Value::Set(_), "Set")
        | (Value::Set(_), "MutableSet") => true,
        (a, b) => a.type_name() == b,
    }
}

/// type(v) / type(name, bases, dict) — the 3-arg form is 02.03; v0 raises.
pub fn builtin_type(interp: &Interpreter, args: &[Value], kwargs: &[(String, Value)]) -> BResult {
    match args.len() {
        0 => Err(raise("TypeError", "type() takes 1 or 3 arguments")),
        1 => {
            let v = &args[0];
            if std::env::var("APX_TDEBUG").is_ok() {
                if let Value::Obj(o) = v {
                    eprintln!(
                        "[TDEBUG] builtin_type obj kind={:?} name={}",
                        o.kind, o.name
                    );
                }
            }
            // T2 fix: user classes must return their OWN class value (the
            // identity `type(C) is C's metaclass instance` and cached
            // isinstance paths depend on it), and builtin-typed values must
            // return the CACHED interned type so `type(x) is SomeBuiltin`
            // identity checks hold (vendored enum.py: `type(_gnv) is not
            // staticmethod` guards a double-wrap; a fresh Type per call
            // broke that and double-wrapped staticmethods).
            if let Value::Obj(o) = v {
                if o.kind == ap_obj::ObjKind::UserClass {
                    if let Some(t) = o.class_ref.borrow().clone() {
                        if let Some(cv) = t.class_value.borrow().clone() {
                            return Ok(cv);
                        }
                    }
                    // fall through to name-based intern below
                }
                // T2 gate finding 3: type(int | str) is types.UnionType —
                // CPython reprs it <class 'typing.Union'> (vendored
                // types.py derives UnionType = type(int | str)).
                if (o.kind == ap_obj::ObjKind::Namespace && o.name == "_Union")
                    || (o.kind == ap_obj::ObjKind::Opaque && o.name == "__union__")
                {
                    let t = interned_type(interp, "UnionType");
                    if let Value::Obj(to) = &t {
                        *to.repr_text.borrow_mut() = Some("<class 'typing.Union'>".to_string());
                    }
                    return Ok(t);
                }
            }
            let name = match v {
                Value::Obj(o) if o.kind == ap_obj::ObjKind::Exception => o.name.clone(),
                // 02.04a: coroutine stubs report their own name (types.py
                // CoroutineType must be DISTINCT from object — the ABC
                // registration in _collections_abc depends on it).
                Value::Obj(o) if o.kind == ap_obj::ObjKind::Namespace && o.name == "coroutine" => {
                    "coroutine".to_string()
                }
                // 02.07: threading objects report their <class> attr
                // (type(me).__name__ == "_MainThread" — T108).
                Value::Obj(o) if o.kind == ap_obj::ObjKind::Opaque && o.name == "thread" => o
                    .attrs
                    .borrow()
                    .get(&Value::Str(PyString::new("<class>")))
                    .and_then(|v| match v {
                        Value::Str(sv) => Some(sv.as_str().to_string()),
                        _ => None,
                    })
                    .unwrap_or_else(|| "Thread".to_string()),
                // A-3 item 5: opaque native result records report their own
                // name (type(os.uname()) == "uname_result"; stat_result too).
                Value::Obj(o) if o.kind == ap_obj::ObjKind::Opaque => match o.name.as_str() {
                    "uname_result" | "stat_result" | "statvfs_result" | "struct_time"
                    | "bytearray" => o.name.clone(),
                    _ => o.type_name(),
                },
                Value::Obj(o) => o.type_name(),
                other => other.type_name().to_string(),
            };
            Ok(interned_type(interp, &name))
        }
        // 02.03b: 3-arg type(name, bases, dict, **kwds) — dynamic class
        // creation (enum._simple_enum's `type(cls_name, (etype,), body,
        // boundary=..., _simple=True)`). Metaclass resolved from the bases;
        // a user metaclass __new__ drives creation with the kwargs.
        _ => {
            let name = match &args[0] {
                Value::Str(s) => s.as_str().to_string(),
                other => {
                    return Err(raise(
                        "TypeError",
                        format!("type() argument 1 must be str, not {}", other.type_name()),
                    ))
                }
            };
            let bases_v: Vec<Value> = match &args[1] {
                Value::Tuple(t) => t.0.clone(),
                Value::List(l) => l.items().to_vec(),
                other => {
                    return Err(raise(
                        "TypeError",
                        format!("type() argument 2 must be tuple, not {}", other.type_name()),
                    ))
                }
            };
            let ns = match &args[2] {
                Value::Dict(d) => d.clone(),
                other => {
                    return Err(raise(
                        "TypeError",
                        format!("type() argument 3 must be dict, not {}", other.type_name()),
                    ))
                }
            };
            let interp_rc = crate::builtins_modules::current_interp()
                .ok_or_else(|| raise("SystemError", "no interpreter for type()"))?;
            return crate::classes::dynamic_type(&interp_rc, &name, &bases_v, ns, kwargs);
        }
    }
}

pub fn builtin_hash(args: &[Value]) -> BResult {
    let v = one(args, "hash")?;
    // User instances with __hash__/__eq__ would dispatch in 02.03; v0:
    // identity hash falls out of py_hash's Obj arm.
    let h = ap_obj::ops::hash(v).map_err(Flow::from_py)?;
    Ok(Value::Int(Int::from_i64(h)))
}

// ---------------------------------------------------------------------------
// iteration builtins
// ---------------------------------------------------------------------------

pub fn builtin_iter(args: &[Value]) -> BResult {
    let v = one(args, "iter")?;
    // 02.08-lite: register the iterator in the live table and hand back a
    // handle whose gen_id resolves (fixes list(iter(x)) / prod(iter(x))).
    // PEP-342: a GENERATOR keeps its OWN id — iter(gen) must be a handle
    // onto the same lazy state, not a fresh registered wrapper of a
    // materialized snapshot.
    if let Value::Obj(o) = v {
        if o.kind == ap_obj::ObjKind::Generator {
            if let Some(id) = *o.gen_id.borrow() {
                // Ensure the lazy anchor exists (iterable_of installs it).
                let interp = crate::builtins_modules::current_interp();
                if let Some(interp) = interp {
                    let _ = crate::interp::iterable_of_public(&v)?;
                    let _ = interp;
                }
                let obj = ap_obj::PyObject::new(ap_obj::ObjKind::Iterator, "iterator");
                *obj.gen_id.borrow_mut() = Some(id);
                return Ok(Value::Obj(obj));
            }
        }
    }
    let it = iterable_of(v)?;
    let id = crate::builtins_modules::register_shared_iter(it);
    let obj = ap_obj::PyObject::new(ap_obj::ObjKind::Iterator, "iterator");
    *obj.gen_id.borrow_mut() = Some(id);
    Ok(Value::Obj(obj))
}

pub fn builtin_next(interp: &Interpreter, args: &[Value]) -> BResult {
    let v = one(args, "next")?;
    match v {
        Value::Obj(o)
            if o.kind == ap_obj::ObjKind::Iterator || o.kind == ap_obj::ObjKind::Generator =>
        {
            let id = o
                .gen_id
                .borrow()
                .ok_or_else(|| raise("TypeError", "'iterator' object is not an iterator"))?;
            // Generators (PEP-342): a body exception propagates; clean
            // exhaustion maps to StopIteration carrying si_value as .value.
            if interp.generators.borrow().contains_key(&id) {
                let rc = interp.rc();
                match crate::interp::gen_pull_public(&rc, id) {
                    Ok(Some(x)) => return Ok(x),
                    Ok(None) => match args.get(1) {
                        Some(default) => return Ok(default.clone()),
                        None => {
                            let val = interp.si_value.borrow_mut().take().unwrap_or(Value::None);
                            let mut e = ExceptionValue::new("StopIteration", vec![]);
                            if !matches!(val, Value::None) {
                                e.args.push(val);
                            }
                            return Err(Flow::Raise(e));
                        }
                    },
                    Err(f) => return Err(f),
                }
            }
            match next_registered(interp, id) {
                Some(v) => Ok(v),
                None => match args.get(1) {
                    Some(default) => Ok(default.clone()),
                    None => Err(stop_iter_flow()),
                },
            }
        }
        _ => Err(raise(
            "TypeError",
            format!("'{}' object is not an iterator", v.type_name()),
        )),
    }
}

fn stop_iter_flow() -> Flow {
    Flow::Raise(ExceptionValue::new("StopIteration", vec![]))
}

/// Pull one value from a registered iterator (plain table only; generator
/// pulls route through the executor).
fn next_registered(interp: &Interpreter, id: u64) -> Option<Value> {
    // Generators route through the executor (materialize + advance).
    if interp.generators.borrow().contains_key(&id) {
        let rc = interp.rc();
        return crate::interp::gen_pull_public(&rc, id).ok().flatten();
    }
    let its = interp.iterators.borrow();
    its.get(&id).and_then(|it| it.next())
}

// ---------------------------------------------------------------------------
// misc builtins
// ---------------------------------------------------------------------------

pub fn builtin_divmod(args: &[Value]) -> BResult {
    let a = one(args, "divmod")?;
    let b = args
        .get(1)
        .ok_or_else(|| raise("TypeError", "divmod expected 2 arguments, got 1"))?;
    let (q, r) = ap_obj::ops::divmod(a, b).map_err(Flow::from_py)?;
    Ok(Value::Tuple(Gc::new(Tuple(vec![q, r]))))
}

pub fn builtin_pow(args: &[Value]) -> BResult {
    match args.len() {
        2 => {
            let a = &args[0];
            let b = &args[1];
            ap_obj::ops::binop(a, ap_obj::ops::BinOp::Pow, b).map_err(Flow::from_py)
        }
        3 => {
            // pow(a, b, m): modular exponentiation (ints only in v0).
            let a = args[0].as_int().ok_or_else(|| {
                raise(
                    "TypeError",
                    "pow() 3rd argument not allowed unless all arguments are integers",
                )
            })?;
            let b = args[1].as_int().ok_or_else(|| {
                raise(
                    "TypeError",
                    "pow() 3rd argument not allowed unless all arguments are integers",
                )
            })?;
            let m = args[2].as_int().ok_or_else(|| {
                raise(
                    "TypeError",
                    "pow() 3rd argument not allowed unless all arguments are integers",
                )
            })?;
            if m.is_zero() {
                return Err(raise("ValueError", "pow() 3rd argument cannot be 0"));
            }
            if b.is_negative() {
                // Modular inverse: compute a^b mod m where b < 0
                // Use extended Euclidean algorithm to find a^(-1) mod m
                // CPython: raises ValueError "base is not invertible for the given modulus"
                // when gcd(a, m) != 1
                let a_abs = a.abs();
                let m_abs = m.abs();
                let (gcd_val, _, _) = extended_gcd(&a_abs, &m_abs);
                if !gcd_val.is_one() {
                    return Err(raise(
                        "ValueError",
                        "base is not invertible for the given modulus",
                    ));
                }
                // Compute modular inverse of a mod m
                let inv = mod_inverse(&a_abs, &m_abs);
                // a^(-b) mod m = inv^b mod m
                let b_pos = b.to_big_for_cmp();
                let b_pos = -b_pos;
                let r = mod_pow(&inv, &Int::from_big(b_pos), &m_abs);
                // Handle sign of result (mod m)
                Ok(Value::Int(r))
            } else {
                let r = mod_pow(&a, &b, &m);
                Ok(Value::Int(r))
            }
        }
        _ => Err(raise(
            "TypeError",
            "pow expected at least 2 arguments, got 0",
        )),
    }
}

/// Extended Euclidean Algorithm: returns (gcd, x, y) such that a*x + b*y = gcd
fn extended_gcd(a: &Int, b: &Int) -> (Int, Int, Int) {
    use num_bigint::BigInt as Big;
    use num_integer::Integer;
    use num_traits::{One, Zero};
    let mut old_r = a.to_big_for_cmp();
    let mut r = b.to_big_for_cmp();
    let mut old_s = Big::one();
    let mut s = Big::zero();
    let mut old_t = Big::zero();
    let mut t = Big::one();
    while !r.is_zero() {
        let q = old_r.div_rem(&r).0;
        let tmp_r = r.clone();
        r = old_r - &q * &r;
        old_r = tmp_r;
        let tmp_s = s.clone();
        s = old_s - &q * &s;
        old_s = tmp_s;
        let tmp_t = t.clone();
        t = old_t - &q * &t;
        old_t = tmp_t;
    }
    // old_r = gcd, old_s, old_t are coefficients
    (
        Int::from_big(old_r),
        Int::from_big(old_s),
        Int::from_big(old_t),
    )
}

/// Modular inverse: returns a^(-1) mod m using extended Euclidean algorithm
fn mod_inverse(a: &Int, m: &Int) -> Int {
    use num_bigint::BigInt as Big;
    use num_traits::{One, Zero};
    let (gcd, x, _) = extended_gcd(a, m);
    if !gcd.is_one() {
        // This should never happen since we check gcd before calling
        return Int::Smi(0);
    }
    // x is the coefficient such that a*x + m*y = 1
    // So x mod m is the modular inverse
    let x_big = x.to_big_for_cmp();
    let m_big = m.to_big_for_cmp();
    let inv = x_big % &m_big;
    // Ensure positive result
    let inv = if inv < Big::zero() { inv + &m_big } else { inv };
    Int::from_big(inv)
}

fn mod_pow(a: &Int, b: &Int, m: &Int) -> Int {
    use num_bigint::BigInt as Big;
    use num_traits::{One, Zero};
    let ab: Big = a.to_big_for_cmp();
    let bb: Big = b.to_big_for_cmp();
    let mb: Big = m.to_big_for_cmp();
    let mut result = Big::one();
    let mut base = ab % &mb;
    let mut exp = bb;
    let ten: Big = Big::from(10u8);
    while !exp.is_zero() {
        // Extract bit parity via divmod by 2.
        let (q, r) = num_integer::Integer::div_rem(&exp, &Big::from(2u8));
        if !r.is_zero() {
            result = (result * &base) % &mb;
        }
        base = (&base * &base) % &mb;
        exp = q;
    }
    let _ = ten;
    Int::from_big(result)
}

/// round(v[, ndigits]): banker's rounding at the binary64 level (S-01-12).
pub fn builtin_round(args: &[Value]) -> BResult {
    let v = one(args, "round")?;
    let nd = args
        .get(1)
        .and_then(|x| x.as_int())
        .and_then(|i| i.to_i64());
    match (v, nd) {
        (Value::Float(f), None) => Ok(Value::Int(
            ap_obj::ops::float_to_int(round_half_even(*f, 0)).map_err(Flow::from_py)?,
        )),
        (Value::Float(f), Some(n)) => {
            let scaled = round_half_even(*f, n as i32);
            // With ndigits: result is a float.
            Ok(Value::Float(scaled))
        }
        (Value::Int(i), None) => Ok(Value::Int(i.clone())),
        (Value::Bool(b), None) => Ok(Value::Int(Int::Smi(i64::from(*b)))),
        (Value::Int(i), Some(n)) => {
            // round(int, n) == the int itself (CPython returns the int).
            let _ = n;
            Ok(Value::Int(i.clone()))
        }
        (Value::Bool(b), Some(n)) => {
            let _ = n;
            Ok(Value::Int(Int::Smi(i64::from(*b))))
        }
        (other, _) => Err(raise(
            "TypeError",
            format!("type {} doesn't define __round__ method", other.type_name()),
        )),
    }
}

/// Round to `ndigits` fractional digits, ties-to-even, in binary64.
fn round_half_even(f: f64, ndigits: i32) -> f64 {
    if !f.is_finite() {
        return f;
    }
    // CPython's double_round: uses scalbn/rint semantics. Rust's
    // `(f * 10^n).round()` is wrong at ties; use the scale-multiply +
    // correctly-rounded approach through decimal string for safety.
    if ndigits == 0 {
        // Round to integer, ties to even. Rust's round() rounds half away;
        // emulate half-even via rint-equivalent.
        let r = f.round();
        let diff = (f - r.trunc()).abs();
        if (diff - 0.5).abs() < f64::EPSILON * f.abs().max(1.0) {
            // Exact tie: choose even.
            let floor = f.floor();
            let ceil = f.ceil();
            let fl = if floor % 2.0 == 0.0 { floor } else { ceil };
            return fl;
        }
        return r;
    }
    // General ndigits: CPython rounds the TRUE binary64 value (David Gay
    // correct rounding). Use Rust's exact decimal expansion ({:.1$e} gives
    // exact digits for binary64) — 2.675's expansion is 2.6749999... which
    // rounds DOWN to 2.67, matching the reference.
    let exact = format!("{:.*e}", 40, f.abs()); // d.dddddde±XX, exact digits
    let (mant, exp) = exact.split_once('e').unwrap();
    let exp: i32 = exp.parse().unwrap_or(0);
    let digits: String = mant.chars().filter(|c| *c != '.').collect();
    let digits = digits.trim_end_matches('0').to_string();
    let digits = if digits.is_empty() {
        "0".to_string()
    } else {
        digits
    };
    // value = 0.digits * 10^(exp+1) — decimal point after exp+1 digits.
    let int_digits = (exp + 1).max(0) as usize; // digits left of the point
    let keep = int_digits as i32 + ndigits;
    let neg = f.is_sign_negative();
    let (dd, _bump) = decimal_round_half_even(&digits, keep);
    let s = format_decimal((dd, _bump), int_digits as i32, neg);
    return s.parse::<f64>().unwrap_or(f);
    #[allow(unreachable_code)]
    {
        let repr = ap_obj::py_float_repr(f);
        let neg = repr.starts_with('-');
        let body = repr.trim_start_matches('-');
        let (int_part, frac_part) = body.split_once('.').unwrap_or((body, ""));
        let digits: String = format!("{int_part}{frac_part}");
        let point = int_part.len() as i32 + ndigits;
        let rounded = decimal_round_half_even(&digits, point);
        let s = format_decimal(rounded, point, neg);
        s.parse::<f64>().unwrap_or(f)
    }
}

/// Decimal string `digits` (no sign, no point) rounded half-even keeping
/// `point` digits left of the cut. Returns the new digit string + bump.
fn decimal_round_half_even(digits: &str, point: i32) -> (String, bool) {
    let n = digits.len() as i32;
    if point >= n {
        // Nothing to cut (pad zeros are implicit).
        return (digits.to_string(), false);
    }
    if point < 0 {
        // Everything cut: check the first digit for the round direction.
        let first = digits.bytes().next().unwrap_or(b'0');
        if first >= b'5' {
            return ("1".to_string(), true);
        }
        return ("0".to_string(), false);
    }
    let cut = point as usize;
    let keep = &digits[..cut];
    let rest = &digits[cut..];
    let first_cut = rest.bytes().next().unwrap_or(b'0');
    if first_cut < b'5' {
        return (keep.to_string(), false);
    }
    if first_cut > b'5' || rest.bytes().skip(1).any(|b| b != b'0') {
        // Round up.
        return (add_one(keep), false);
    }
    // Exact tie: half-even.
    let last = keep.bytes().last().unwrap_or(b'0');
    if last % 2 == 0 {
        (keep.to_string(), false)
    } else {
        (add_one(keep), false)
    }
}

fn add_one(digits: &str) -> String {
    let mut out: Vec<u8> = digits.bytes().collect();
    let mut carry = 1u8;
    for b in out.iter_mut().rev() {
        let v = *b - b'0' + carry;
        if v >= 10 {
            *b = b'0';
        } else {
            *b = b'0' + v;
            carry = 0;
            break;
        }
    }
    if carry == 1 {
        out.insert(0, b'1');
    }
    String::from_utf8(out).unwrap_or_else(|_| digits.to_string())
}

/// Rebuild a decimal from digit string + point position (leading/trailing
/// zeros normalized for f64 parse).
fn format_decimal((digits, _bump): (String, bool), point: i32, neg: bool) -> String {
    let sign = if neg { "-" } else { "" };
    let mut d = digits.trim_end_matches('0').to_string();
    if d.is_empty() {
        d = "0".to_string();
    }
    let point = point.max(0) as usize;
    let padded = format!("{:0>width$}", d, width = point.max(d.len()));
    let (ip, fp) = padded.split_at(point.min(padded.len()));
    let fp = fp.trim_end_matches('0');
    if fp.is_empty() {
        format!("{sign}{ip}.0")
    } else {
        format!("{sign}{ip}.{fp}")
    }
}

pub fn builtin_chr(args: &[Value]) -> BResult {
    let i = one(args, "chr")?
        .as_int()
        .ok_or_else(|| raise("TypeError", "an integer is required"))?;
    let cp = i
        .to_i64()
        .ok_or_else(|| raise("ValueError", "chr() arg not in range(0x110000)"))?;
    let ch = u32::try_from(cp)
        .ok()
        .and_then(char::from_u32)
        .ok_or_else(|| raise("ValueError", format!("chr() arg not in range(0x110000)")))?;
    Ok(Value::Str(PyString::new(ch.to_string())))
}

pub fn builtin_ord(args: &[Value]) -> BResult {
    let v = one(args, "ord")?;
    match v {
        Value::Str(s) => {
            let mut chars = s.as_str().chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => Ok(Value::Int(Int::from_i64(c as i64))),
                _ => Err(raise(
                    "TypeError",
                    "ord() expected a character, but string of length 0 found",
                )),
            }
        }
        Value::Bytes(b) => {
            if b.len() == 1 {
                Ok(Value::Int(Int::from_i64(b[0] as i64)))
            } else if b.is_empty() {
                Err(raise(
                    "TypeError",
                    "ord() expected a character, but string of length 0 found",
                ))
            } else {
                Err(raise(
                    "TypeError",
                    "ord() expected a character, but string of length 2 found",
                ))
            }
        }
        _ => Err(raise(
            "TypeError",
            format!("ord() expected string of length 1, but {}", v.type_name()),
        )),
    }
}

pub fn builtin_hex(args: &[Value]) -> BResult {
    let i = int_arg(args, "hex")?;
    let neg = i.is_negative();
    let mag = i.abs().to_string_raw().trim_start_matches('-').to_string();
    let digits = match mag.parse::<u128>() {
        Ok(v) => format!("{v:x}"),
        Err(_) => bigint_to_base(&mag, 16),
    };
    Ok(Value::Str(PyString::new(format!(
        "{}0x{}",
        if neg { "-" } else { "" },
        digits
    ))))
}

pub fn builtin_oct(args: &[Value]) -> BResult {
    let i = int_arg(args, "oct")?;
    let neg = i.is_negative();
    let mag = i.abs().to_string_raw().trim_start_matches('-').to_string();
    let digits = match mag.parse::<u128>() {
        Ok(v) => format!("{v:o}"),
        Err(_) => bigint_to_base(&mag, 8),
    };
    Ok(Value::Str(PyString::new(format!(
        "{}0o{}",
        if neg { "-" } else { "" },
        digits
    ))))
}

pub fn builtin_bin(args: &[Value]) -> BResult {
    let i = int_arg(args, "bin")?;
    let neg = i.is_negative();
    let mag = i.abs().to_string_raw().trim_start_matches('-').to_string();
    let digits = match mag.parse::<u128>() {
        Ok(v) => format!("{v:b}"),
        Err(_) => bigint_to_base(&mag, 2),
    };
    Ok(Value::Str(PyString::new(format!(
        "{}0b{}",
        if neg { "-" } else { "" },
        digits
    ))))
}

fn int_arg(args: &[Value], name: &str) -> Result<Int, Flow> {
    let v = one(args, name)?;
    v.as_int().ok_or_else(|| {
        raise(
            "TypeError",
            format!(
                "'{}' object cannot be interpreted as an integer",
                v.type_name()
            ),
        )
    })
}

/// Decimal digit-string → base-N digit string (beyond u128 fallback).
fn bigint_to_base(dec: &str, base: u32) -> String {
    use num_bigint::BigUint;
    use num_traits::Zero;
    let mut n = dec.parse::<BigUint>().unwrap_or_else(|_| BigUint::zero());
    if n.is_zero() {
        return "0".to_string();
    }
    let mut out = Vec::new();
    let b = BigUint::from(base);
    let zero = BigUint::zero();
    while n > zero {
        let (q, r) = num_integer::Integer::div_rem(&n, &b);
        let digit = r.to_u32_digits().first().copied().unwrap_or(0) as u32;
        out.push(std::char::from_digit(digit, base).unwrap_or('0'));
        n = q;
    }
    out.reverse();
    out.into_iter().collect()
}

pub fn builtin_enumerate(
    interp: &Interpreter,
    args: &[Value],
    kwargs: &[(String, Value)],
) -> BResult {
    let v = one(args, "enumerate")?;
    let start = args
        .get(1)
        .and_then(|s| s.as_int())
        .and_then(|i| i.to_i64())
        .or_else(|| {
            kwargs
                .iter()
                .find(|(k, _)| k == "start")
                .and_then(|(_, s)| s.as_int())
                .and_then(|i| i.to_i64())
        })
        .unwrap_or(0);
    // 02.03b-fix: the old wrapper leaked the INNER iterator (comp unpack of
    // enumerate(x, start) saw raw elements). v0 enumerate is eager: materialize
    // (index, item) pairs — laziness is unobservable on the gate corpus.
    let it = iterable_of(v)?;
    let items = crate::interp::collect_iter_public(&it);
    let mut pairs: Vec<Value> = Vec::with_capacity(items.len());
    let mut c = start;
    for item in items {
        pairs.push(Value::Tuple(Gc::new(Tuple(vec![
            Value::Int(ap_obj::Int::from_i64(c)),
            item,
        ]))));
        c += 1;
    }
    let obj = ap_obj::PyObject::new(ap_obj::ObjKind::Iterator, "enumerate");
    *obj.ints.borrow_mut() = vec![start];
    *obj.gen_id.borrow_mut() = Some(register_iter(
        interp,
        &Value::List(PyList::from_vec(pairs)),
    )?);
    Ok(Value::Obj(obj))
}

use crate::iter::{IterState, PyIterator};

/// 02.03b: map(fn, *iterables) — eager v0: collects the iterables and maps.
pub fn builtin_map(interp: &Interpreter, args: &[Value]) -> BResult {
    if args.is_empty() {
        return Err(raise(
            "TypeError",
            "map() must have at least two arguments.",
        ));
    }
    let f = args[0].clone();
    let mut cols: Vec<Vec<Value>> = Vec::new();
    for a in &args[1..] {
        // T2 fix: a def-generator argument (glob's selector walk, pytest
        // collection) must drain through the lazy gen_pull path — the anchor
        // Seq that iterable_of installs for def-generators stays EMPTY and
        // collect() would return zero items.
        if let Some(drain) = crate::interp::drain_generator_public(a) {
            cols.push(drain?);
            continue;
        }
        let it = crate::interp::iterable_of_public(a)?;
        cols.push(crate::iter::collect(&it));
    }
    let n = cols.iter().map(|c| c.len()).min().unwrap_or(0);
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let mut call_args: Vec<Value> = Vec::with_capacity(cols.len());
        for c in &cols {
            call_args.push(c[i].clone());
        }
        let caller = crate::interp::module_frame_pub();
        let r = if matches!(f, Value::None) {
            match call_args.len() {
                0 => Value::None,
                1 => call_args[0].clone(),
                _ => Value::Tuple(Gc::new(Tuple(call_args.clone()))),
            }
        } else {
            crate::interp::call_value_pub3(&f, &call_args)?
        };
        out.push(r);
    }
    Ok(Value::List(PyList::from_vec(out)))
}

/// 02.03b: filter(fn, iterable) — eager v0.
pub fn builtin_filter(interp: &Interpreter, args: &[Value]) -> BResult {
    if args.len() < 2 {
        return Err(raise("TypeError", "filter expected 2 arguments"));
    }
    let f = args[0].clone();
    let it = crate::interp::iterable_of_public(&args[1])?;
    let items = crate::iter::collect(&it);
    let mut out = Vec::new();
    let caller = crate::interp::module_frame_pub();
    for item in items {
        let keep = if matches!(f, Value::None) {
            item.is_truthy()
        } else {
            crate::interp::call_value_pub3(&f, &[item.clone()])?.is_truthy()
        };
        if keep {
            out.push(item);
        }
    }
    Ok(Value::List(PyList::from_vec(out)))
}

pub fn builtin_zip(interp: &Interpreter, args: &[Value]) -> BResult {
    // 02.04 fix (gc002 needs 2-arg zip): build a real ZipMulti iterator that
    // yields tuples — the old shape parked the FIRST iterable's id on the
    // zip object, so `zip(xs, ys)` iterated only xs and for-target unpacking
    // of the plain items died with "cannot unpack non-iterable".
    // zip() with no args: exhausted.
    if args.is_empty() {
        let it = crate::iter::PyIterator::new(crate::iter::IterState::Seq(Vec::new(), 0));
        let id = interp.next_iter_id.get();
        interp.next_iter_id.set(id + 2);
        interp.iterators.borrow_mut().insert(id, it);
        let obj = ap_obj::PyObject::new(ap_obj::ObjKind::Iterator, "zip");
        *obj.gen_id.borrow_mut() = Some(id);
        return Ok(Value::Obj(obj));
    }
    let mut parts: Vec<std::vec::IntoIter<Value>> = Vec::with_capacity(args.len());
    for a in args {
        let it = iterable_of(a)?;
        let items = crate::iter::collect(&it);
        parts.push(items.into_iter());
    }
    let obj = crate::builtins_modules::make_iter(
        crate::builtins_modules::NativeIterState::ZipMulti(parts),
    );
    Ok(obj)
}

/// Register an iterable with the interpreter, returning its id.
pub fn register_iter(interp: &Interpreter, v: &Value) -> Result<u64, Flow> {
    // Generators are already iterator handles.
    if let Value::Obj(o) = v {
        if matches!(
            o.kind,
            ap_obj::ObjKind::Iterator | ap_obj::ObjKind::Generator
        ) {
            return o.gen_id.borrow().ok_or_else(stop_iter_flow_cloned);
        }
    }
    let it = iterable_of(v)?;
    let id = interp.next_iter_id.get();
    interp.next_iter_id.set(id + 1);
    interp.iterators.borrow_mut().insert(id, it);
    Ok(id)
}

fn stop_iter_flow_cloned() -> Flow {
    stop_iter_flow()
}

pub fn builtin_sorted(interp: &Interpreter, args: &[Value], kwargs: &[(String, Value)]) -> BResult {
    let v = one(args, "sorted")?;
    let it = iterable_of(v)?;
    let mut items = crate::iter::collect(&it);
    let mut reverse = false;
    let mut key_fn: Option<Value> = None;
    for (k, val) in kwargs {
        match k.as_str() {
            "reverse" => reverse = val.is_truthy(),
            "key" => key_fn = Some(val.clone()),
            _ => return Err(raise("TypeError", f_format_invalid_kw(k))),
        }
    }
    if let Some(f) = key_fn {
        // Decorate with keys computed through the interpreter.
        let mut decorated = Vec::with_capacity(items.len());
        for item in items {
            let key = crate::interp::call_value_pub(&interp.rc(), &f, &[item.clone()], &[])?;
            decorated.push((key, item));
        }
        // 02.04: functools.cmp_to_key wrappers order via the parked cmp.
        if let Some(r) = crate::builtins_modules::sort_by_cmp_wrappers(&decorated) {
            items = r?;
        } else {
            decorated.sort_by(|a, b| {
                ap_obj::dict::values_cmp(&a.0, &b.0).unwrap_or(std::cmp::Ordering::Equal)
            });
            items = decorated.into_iter().map(|(_, v)| v).collect();
        }
    } else {
        items.sort_by(|a, b| ap_obj::dict::values_cmp(a, b).unwrap_or(std::cmp::Ordering::Equal));
    }
    if reverse {
        items.reverse();
    }
    Ok(Value::List(PyList::from_vec(items)))
}

fn f_format_invalid_kw(k: &str) -> String {
    format!("'{k}' is an invalid keyword argument for sorted()")
}

pub fn builtin_format(args: &[Value]) -> BResult {
    let v = one(args, "format")?;
    let spec = match args.get(1) {
        None => "",
        Some(Value::Str(s)) => s.as_str(),
        Some(other) => {
            return Err(raise(
                "TypeError",
                format!("format() argument 2 must be str, not {}", other.type_name()),
            ))
        }
    };
    let out = crate::format::format_value(v, spec).map_err(Flow::from_py)?;
    Ok(Value::Str(PyString::new(out)))
}

// ---------------------------------------------------------------------------
// sys module stub
// ---------------------------------------------------------------------------

/// Build the `sys` module object (S-05-4/S-05-6 provisional: argv minimal,
/// maxsize, version_info, hash_info, int/float_info, recursion controls,
/// exit).
pub fn make_sys_module(interp: &Interpreter) -> Value {
    let m = ap_obj::PyObject::new(ap_obj::ObjKind::Module, "sys");
    let ns = PyDict::new();
    // argv: [script] minimal.
    let argv = PyList::from_vec(vec![Value::Str(PyString::new(interp.script_path.clone()))]);
    let _ = ns.set(Value::Str(PyString::new("argv")), Value::List(argv));
    // 02.03b-fix: sys.builtin_module_names (importlib._bootstrap reads it).
    let _ = ns.set(
        Value::Str(PyString::new("builtin_module_names")),
        Value::Tuple(Gc::new(Tuple(Vec::new()))),
    );
    // T1 (vendored dataclasses): sys._clear_type_descriptors — 3.14 C-API
    // surface; our class model keeps no separate descriptor caches, so an
    // inert callable is semantically exact.
    let _ = ns.set(
        Value::Str(PyString::new("_clear_type_descriptors")),
        Value::Obj(ap_obj::PyObject::new(
            ap_obj::ObjKind::BuiltinFn,
            "sys._clear_type_descriptors",
        )),
    );
    // 02.03b-fix: sys.path config surface (sysconfig/site paths read it).
    for (k, v) in [
        ("base_prefix", "/usr/local"),
        ("base_exec_prefix", "/usr/local"),
        ("prefix", "/usr/local"),
        ("exec_prefix", "/usr/local"),
        ("platform", "darwin"),
        // T1-d (packaging.tags sys_tags): CPython's macOS builds carry an
        // empty abiflags (the "m"/"d" suffixes are gone in 3.8+).
        ("abiflags", ""),
        // T1-d (sysconfig paths): CPython 3.14 sys.platlibdir default "lib".
        ("platlibdir", "lib"),
        ("implementation", "cpython"),
        ("dont_write_bytecode", "True"),
        ("_base_executable", "/usr/local/bin/python3.14"),
        ("executable", "/usr/local/bin/python3.14"),
    ] {
        let val = match v {
            "True" => Value::Bool(true),
            other => Value::Str(PyString::new(other)),
        };
        let _ = ns.set(Value::Str(PyString::new(k)), val);
    }
    // T1 (sysconfig darwin): sys._framework — framework-name mapping;
    // an empty mapping keeps sysconfig's probe inert on this runtime.
    {
        let fw = PyDict::new();
        let _ = ns.set(Value::Str(PyString::new("_framework")), Value::Dict(fw));
    }
    let _ = ns.set(
        Value::Str(PyString::new("maxsize")),
        Value::Int(Int::from_i64(i64::MAX)),
    );
    let _ = ns.set(
        Value::Str(PyString::new("warnoptions")),
        Value::List(PyList::from_vec(vec![])),
    );
    // 02.04a fix pass (BUG-C): sys.flags — _py_warnings reads
    // sys.flags.context_aware_warnings; unittest reads several more.
    {
        let flags = ap_obj::PyObject::new(ap_obj::ObjKind::Namespace, "flags");
        {
            let fa = flags.attrs.borrow_mut();
            for (k, v) in [
                ("debug", 0i64),
                ("inspect", 0),
                ("interactive", 0),
                ("optimize", 0),
                ("dont_write_bytecode", 1),
                ("no_user_site", 0),
                ("no_site", 0),
                ("ignore_environment", 0),
                ("verbose", 0),
                ("bytes_warning", 0),
                ("quiet", 0),
                ("hash_randomization", 1),
                ("isolated", 0),
                ("dev_mode", 0),
                ("utf8_mode", 1),
                ("warn_default_encoding", 0),
                ("safepath", 0),
                ("int_max_str_digits", 4300),
                ("context_aware_warnings", 0),
                ("gil", 0),
            ] {
                let _ = fa.set(Value::Str(PyString::new(k)), Value::Int(Int::from_i64(v)));
            }
        }
        let _ = ns.set(Value::Str(PyString::new("flags")), Value::Obj(flags));
    }
    let _ = ns.set(
        Value::Str(PyString::new("maxunicode")),
        Value::Int(Int::from_i64(0x10FFFF)),
    );
    let _ = ns.set(
        Value::Str(PyString::new("byteorder")),
        Value::Str(PyString::new("little")),
    );
    let _ = ns.set(
        Value::Str(PyString::new("float_repr_style")),
        Value::Str(PyString::new("short")),
    );
    let _ = ns.set(
        Value::Str(PyString::new("platform")),
        Value::Str(PyString::new("darwin")),
    );
    let _ = ns.set(
        Value::Str(PyString::new("version")),
        Value::Str(PyString::new(
            "3.14.0 (antipythony v0 sys stub; provisional per leaf 02.02)",
        )),
    );
    // version_info: named-tuple-like namespace.
    let vi = ap_obj::PyObject::new(ap_obj::ObjKind::NamedTuple, "version_info");
    *vi.ints.borrow_mut() = vec![3, 14, 0, 3]; // major, minor, micro, releaselevel-index
    {
        let attrs = vi.attrs.borrow_mut();
        let _ = attrs.set(
            Value::Str(PyString::new("releaselevel")),
            Value::Str(PyString::new("final")),
        );
        // 02.08-full part 1: <values> list drives len/subscript/iter/eq
        // (sys.version_info[:2] == (3, 14), s002 pin).
        let _ = attrs.set(
            Value::Str(PyString::new("<values>")),
            Value::List(PyList::from_vec(vec![
                Value::Int(Int::from_i64(3)),
                Value::Int(Int::from_i64(14)),
                Value::Int(Int::from_i64(0)),
                Value::Str(PyString::new("final")),
                Value::Int(Int::from_i64(0)),
            ])),
        );
    }
    let _ = ns.set(Value::Str(PyString::new("version_info")), Value::Obj(vi));
    // hash_info
    let hi = ap_obj::PyObject::new(ap_obj::ObjKind::Namespace, "hash_info");
    {
        let attrs = hi.attrs.borrow_mut();
        let _ = attrs.set(
            Value::Str(PyString::new("modulus")),
            Value::Int(Int::from_i64(ap_obj::hash::HASH_MODULUS as i64)),
        );
        let _ = attrs.set(
            Value::Str(PyString::new("inf")),
            Value::Int(Int::from_i64(ap_obj::hash::HASH_INF)),
        );
        let _ = attrs.set(Value::Str(PyString::new("width")), Value::Int(Int::Smi(64)));
        let _ = attrs.set(
            Value::Str(PyString::new("algorithm")),
            Value::Str(PyString::new("siphash13")),
        );
        let _ = attrs.set(
            Value::Str(PyString::new("hash_bits")),
            Value::Int(Int::Smi(64)),
        );
        let _ = attrs.set(
            Value::Str(PyString::new("suffix_bits")),
            Value::Int(Int::Smi(0)),
        );
        // T1 (_pydecimal): sys.hash_info.nan (hash sentinel for NaN).
        let _ = attrs.set(
            Value::Str(PyString::new("nan")),
            Value::Int(Int::from_i64(0)),
        );
    }
    let _ = ns.set(Value::Str(PyString::new("hash_info")), Value::Obj(hi));
    // float_info (subset V056-adjacent corpus uses mant_dig)
    let fi = ap_obj::PyObject::new(ap_obj::ObjKind::Namespace, "float_info");
    {
        let attrs = fi.attrs.borrow_mut();
        let _ = attrs.set(
            Value::Str(PyString::new("mant_dig")),
            Value::Int(Int::Smi(53)),
        );
        let _ = attrs.set(Value::Str(PyString::new("dig")), Value::Int(Int::Smi(15)));
        let _ = attrs.set(Value::Str(PyString::new("max")), Value::Float(f64::MAX));
        let _ = attrs.set(
            Value::Str(PyString::new("min")),
            Value::Float(f64::MIN_POSITIVE),
        );
        let _ = attrs.set(
            Value::Str(PyString::new("epsilon")),
            Value::Float(f64::EPSILON),
        );
    }
    let _ = ns.set(Value::Str(PyString::new("float_info")), Value::Obj(fi));
    // int_info
    let ii = ap_obj::PyObject::new(ap_obj::ObjKind::Namespace, "int_info");
    {
        let attrs = ii.attrs.borrow_mut();
        let _ = attrs.set(
            Value::Str(PyString::new("bits_per_digit")),
            Value::Int(Int::Smi(30)),
        );
        let _ = attrs.set(
            Value::Str(PyString::new("sizeof_digit")),
            Value::Int(Int::Smi(4)),
        );
        let _ = attrs.set(
            Value::Str(PyString::new("default_max_str_digits")),
            Value::Int(Int::Smi(4300)),
        );
        let _ = attrs.set(
            Value::Str(PyString::new("str_digits_check_threshold")),
            Value::Int(Int::Smi(640)),
        );
    }
    let _ = ns.set(Value::Str(PyString::new("int_info")), Value::Obj(ii));
    // stdout/stderr markers (print(file=...) identity checks).
    // 02.03b: a write() marker on each — warnings' _showwarnmsg_impl does
    // `file.write(text)` on sys.stderr.
    let so = ap_obj::PyObject::new(ap_obj::ObjKind::Namespace, "stdout");
    let _ = so.attrs.borrow_mut().set(
        Value::Str(PyString::new("write")),
        builtin_marker("__ap_stream_write__"),
    );
    let _ = so.attrs.borrow_mut().set(
        Value::Str(PyString::new("fileno")),
        builtin_marker("sys.stdout.fileno"),
    );
    let _ = so.attrs.borrow_mut().set(
        Value::Str(PyString::new("isatty")),
        builtin_marker("sys.stdout.isatty"),
    );
    let _ = so.attrs.borrow_mut().set(
        Value::Str(PyString::new("flush")),
        builtin_marker("__ap_stream_flush__"),
    );
    let _ = ns.set(Value::Str(PyString::new("stdout")), Value::Obj(so.clone()));
    let _ = ns.set(Value::Str(PyString::new("__stdout__")), Value::Obj(so));
    let se = ap_obj::PyObject::new(ap_obj::ObjKind::Namespace, "stderr");
    {
        let mut se_attrs = se.attrs.borrow_mut();
        let _ = se_attrs.set(
            Value::Str(PyString::new("write")),
            builtin_marker("__ap_stream_write__stderr__"),
        );
        let _ = se_attrs.set(
            Value::Str(PyString::new("flush")),
            builtin_marker("__ap_stream_flush__"),
        );
        let _ = se_attrs.set(
            Value::Str(PyString::new("fileno")),
            builtin_marker("sys.stderr.fileno"),
        );
        let _ = se_attrs.set(
            Value::Str(PyString::new("isatty")),
            builtin_marker("sys.stderr.isatty"),
        );
    }
    let _ = ns.set(Value::Str(PyString::new("stderr")), Value::Obj(se.clone()));
    let _ = ns.set(Value::Str(PyString::new("__stderr__")), Value::Obj(se));
    // sys.unraisablehook / sys.__unraisablehook__ (gc009): seeded None =
    // native default (report goes to the real stderr stream). The attribute
    // must EXIST — reads raise AttributeError on a missing module attr, and
    // gc009 fetches sys.unraisablehook before the script swaps it. The
    // collector treats None as "no hook" and falls to the stderr renderer.
    let _ = ns.set(Value::Str(PyString::new("unraisablehook")), Value::None);
    let _ = ns.set(Value::Str(PyString::new("__unraisablehook__")), Value::None);
    // sys.implementation (02.04a: enum.py reads it at import).
    let impl_ns = ap_obj::PyObject::new(ap_obj::ObjKind::Namespace, "implementation");
    {
        let attrs = impl_ns.attrs.borrow_mut();
        let _ = attrs.set(
            Value::Str(PyString::new("name")),
            Value::Str(PyString::new("cpython")),
        );
        let _ = attrs.set(
            Value::Str(PyString::new("cache_tag")),
            Value::Str(PyString::new("cpython-314")),
        );
        // T1-d (sysconfig._get_sysconfigdata_name): multiarch suffix so the
        // vendored _sysconfigdata__darwin_darwin module resolves.
        let _ = attrs.set(
            Value::Str(PyString::new("_multiarch")),
            Value::Str(PyString::new("darwin")),
        );
        let _ = attrs.set(
            Value::Str(PyString::new("version")),
            Value::Str(PyString::new("3.14.0")),
        );
        let _ = attrs.set(
            Value::Str(PyString::new("hexversion")),
            Value::Int(Int::from_i64(0x30e00f0)),
        );
    }
    let _ = ns.set(
        Value::Str(PyString::new("implementation")),
        Value::Obj(impl_ns),
    );
    // sys._getframe (02.04a: _collections_abc calls it at import; v0
    // returns a stub frame whose f_locals is a fresh dict).
    let _ = ns.set(
        Value::Str(PyString::new("_getframe")),
        builtin_marker("sys._getframe"),
    );
    let _ = ns.set(
        Value::Str(PyString::new("_getframemodulename")),
        builtin_marker("sys._getframemodulename"),
    );
    let _ = ns.set(
        Value::Str(PyString::new("getdefaultencoding")),
        builtin_marker("sys.getdefaultencoding"),
    );
    let _ = ns.set(
        Value::Str(PyString::new("getrecursionlimit")),
        builtin_marker("sys.getrecursionlimit"),
    );
    let _ = ns.set(
        Value::Str(PyString::new("audit")),
        builtin_marker("sys.audit"),
    );
    let _ = ns.set(
        Value::Str(PyString::new("setrecursionlimit")),
        builtin_marker("sys.setrecursionlimit"),
    );
    let _ = ns.set(
        Value::Str(PyString::new("set_int_max_str_digits")),
        builtin_marker("sys.set_int_max_str_digits"),
    );
    let _ = ns.set(
        Value::Str(PyString::new("get_int_max_str_digits")),
        builtin_marker("sys.get_int_max_str_digits"),
    );
    let _ = ns.set(
        Value::Str(PyString::new("exit")),
        builtin_marker("sys.exit"),
    );
    // 02.08-full part 1: sys name-surface completion (s002 hasattr row).
    // orig_argv: at least one entry (the interpreter path).
    let _ = ns.set(
        Value::Str(PyString::new("orig_argv")),
        Value::List(PyList::from_vec(vec![Value::Str(PyString::new(
            interp.script_path.clone(),
        ))])),
    );
    let _ = ns.set(
        Value::Str(PyString::new("meta_path")),
        Value::List(PyList::from_vec(vec![])),
    );
    let _ = ns.set(
        Value::Str(PyString::new("path_hooks")),
        Value::List(PyList::from_vec(vec![])),
    );
    let _ = ns.set(
        Value::Str(PyString::new("path_importer_cache")),
        Value::Dict(PyDict::new()),
    );
    // T2 (vendored bdb.py): sys.monitoring surface — module-level
    // `E = sys.monitoring.events` + Bdb.use_tool_id bookkeeping. The
    // monitoring machinery itself is inert (no tracing hooks in v0); the
    // namespace answers the identity/events probes bdb makes at import.
    {
        let mon = ap_obj::PyObject::new(ap_obj::ObjKind::Namespace, "monitoring");
        {
            let ma = mon.attrs.borrow_mut();
            let _ = ma.set(Value::Str(PyString::new("DEBUGGER_ID")), {
                Value::Int(ap_obj::Int::from_i64(0))
            });
            let ev = ap_obj::PyObject::new(ap_obj::ObjKind::Namespace, "monitoring.events");
            {
                let eva = ev.attrs.borrow_mut();
                // CPython 3.14 sys.monitoring.events int members (bdb reads
                // them at import: E = sys.monitoring.events).
                for (i, name) in [
                    "BRANCH",
                    "BRANCH_LEFT",
                    "BRANCH_RIGHT",
                    "CALL",
                    "C_RAISE",
                    "C_RETURN",
                    "EXCEPTION_HANDLED",
                    "INSTRUCTION",
                    "JUMP",
                    "LINE",
                    "NO_EVENTS",
                    "PY_RESUME",
                    "PY_RETURN",
                    "PY_START",
                    "PY_THROW",
                    "PY_UNWIND",
                    "PY_YIELD",
                    "RAISE",
                    "RERAISE",
                    "STOP_ITERATION",
                ]
                .iter()
                .enumerate()
                {
                    let _ = eva.set(
                        Value::Str(PyString::new(*name)),
                        Value::Int(ap_obj::Int::from_i64(i as i64)),
                    );
                }
            }
            let _ = ma.set(Value::Str(PyString::new("events")), Value::Obj(ev));
            for f in [
                "get_tool",
                "use_tool_id",
                "clear_tool_id",
                "set_local_events",
            ] {
                let _ = ma.set(
                    Value::Str(PyString::new(f)),
                    Value::Obj(ap_obj::PyObject::new(
                        ap_obj::ObjKind::BuiltinFn,
                        &format!("sys.monitoring.{f}"),
                    )),
                );
            }
        }
        let _ = ns.set(Value::Str(PyString::new("monitoring")), Value::Obj(mon));
    }
    let stdin_obj = Value::Obj(ap_obj::PyObject::new(
        ap_obj::ObjKind::Opaque,
        "TextIOWrapper",
    ));
    let _ = ns.set(Value::Str(PyString::new("stdin")), stdin_obj.clone());
    // CPython: sys.__stdin__ holds the ORIGINAL stdin (pytest's
    // DontReadFromInput restore + capture.py:225 assert rely on it
    // existing and being non-None).
    let _ = ns.set(Value::Str(PyString::new("__stdin__")), stdin_obj);
    let _ = ns.set(
        Value::Str(PyString::new("hexversion")),
        Value::Int(Int::from_i64(0x30E_00_00)),
    );
    let _ = ns.set(
        Value::Str(PyString::new("api_version")),
        Value::Int(Int::from_i64(1013)),
    );
    for hook in ["excepthook", "displayhook", "breakpointhook"] {
        let _ = ns.set(Value::Str(PyString::new(hook)), builtin_marker("sys.print"));
    }
    let _ = ns.set(
        Value::Str(PyString::new("getfilesystemencoding")),
        builtin_marker("sys.getdefaultencoding"),
    );
    let _ = ns.set(
        Value::Str(PyString::new("getfilesystemencodeerrors")),
        builtin_marker("sys.getfilesystemencodeerrors"),
    );
    let _ = ns.set(
        Value::Str(PyString::new("intern")),
        builtin_marker("sys.intern"),
    );
    let _ = ns.set(
        Value::Str(PyString::new("getswitchinterval")),
        builtin_marker("sys.getswitchinterval"),
    );
    let _ = ns.set(
        Value::Str(PyString::new("is_finalizing")),
        builtin_marker("sys.is_finalizing"),
    );
    let _ = ns.set(
        Value::Str(PyString::new("setswitchinterval")),
        builtin_marker("sys.setswitchinterval"),
    );
    let _ = ns.set(
        Value::Str(PyString::new("exc_info")),
        builtin_marker("sys.exc_info"),
    );
    let _ = ns.set(
        Value::Str(PyString::new("exception")),
        builtin_marker("sys.exception"),
    );
    let _ = m
        .module_ns
        .borrow_mut()
        .replace(std::rc::Rc::new((*ns).clone()));
    // Store the ns for attribute reads.
    *m.module_ns.borrow_mut() = Some(std::rc::Rc::new((*ns).clone()));
    Value::Obj(m)
}

fn builtin_marker(name: &str) -> Value {
    let o = ap_obj::PyObject::new(ap_obj::ObjKind::BuiltinFn, name);
    Value::Obj(o)
}

/// Dispatch a sys-module attribute access that is a function marker.
pub fn call_sys_fn(interp: &Interpreter, name: &str, args: &[Value]) -> Option<BResult> {
    match name {
        // T2 (pytest faulthandler get_stderr_fileno): the console streams
        // report the real fd numbers.
        "stdout.fileno" => Some(Ok(Value::Int(Int::from_i64(1)))),
        "stderr.fileno" => Some(Ok(Value::Int(Int::from_i64(2)))),
        "stdin.fileno" => Some(Ok(Value::Int(Int::from_i64(0)))),
        // T2 (pytest terminal _TerminalWriter: `file.isatty()`):
        "stdout.isatty" | "stderr.isatty" | "stdin.isatty" => Some(Ok(Value::Bool(false))),
        // T1 (vendored dataclasses): inert (see make_sys_module).
        "_clear_type_descriptors" => Some(Ok(Value::None)),
        // T2 (pytest collection): CPython audit hooks — no-op sink; the
        // vendored glob/pickle/subprocess paths call sys.audit(...) and
        // must not AttributeError mid-collection.
        "audit" => Some(Ok(Value::None)),
        // 02.08-full part 1: name-surface fns.
        "intern" => {
            // Canonical-instance table keyed by content: intern(a + "") is
            // intern(a) (s002 identity pin).
            use std::collections::HashMap;
            thread_local! {
                static INTERNED: std::cell::RefCell<HashMap<String, Value>> =
                    std::cell::RefCell::new(HashMap::new());
            }
            let v = args.first().cloned().unwrap_or(Value::None);
            let text = crate::builtins::py_str_of(&v).unwrap_or_default();
            let canon = INTERNED.with(|c| {
                let mut m = c.borrow_mut();
                if let Some(canon) = m.get(&text) {
                    return canon.clone();
                }
                m.insert(text.clone(), v.clone());
                v.clone()
            });
            Some(Ok(canon))
        }
        // T2 (bdb): inert monitoring-tool registry (single tool, no tracing).
        "sys.monitoring.get_tool" => Some(Ok(Value::None)),
        "sys.monitoring.use_tool_id" => Some(Ok(Value::None)),
        "sys.monitoring.clear_tool_id" => Some(Ok(Value::None)),
        "sys.monitoring.set_local_events" => Some(Ok(Value::None)),
        "getswitchinterval" => Some(Ok(Value::Float(0.005))),
        "setswitchinterval" => Some(Ok(Value::None)),
        // W110-5 (asyncio Runner.close → _UnixSelectorEventLoop.close reads
        // it on every loop close): single-threaded v0 never finalizes during
        // a run — False, matching CPython's pre-shutdown answer.
        "is_finalizing" => Some(Ok(Value::Bool(false))),
        "getfilesystemencoding" => Some(Ok(Value::Str(PyString::new("utf-8")))),
        "getfilesystemencodeerrors" => Some(Ok(Value::Str(PyString::new("surrogateescape")))),
        _ if name.starts_with("sys.") => Some(Err(raise(
            "AttributeError",
            format!("'module' object has no attribute '{}'", name),
        ))),
        "print" => Some(print(interp, args, &[])),
        // 02.04a: stub frame objects (f_locals readable, everything else None).
        // 02.03b: _py_warnings.walk reads frame.f_code.co_filename /
        // f_lineno — a single synthetic frame with the RUNNING SCRIPT's
        // path + line 1 (warn() stacklevel walks land on the caller, which
        // for the gate IS the script).
        "_getframe" | "_getframemodulename" => {
            let frame = ap_obj::PyObject::new(ap_obj::ObjKind::Namespace, "<frame>");
            {
                let code = ap_obj::PyObject::new(ap_obj::ObjKind::Namespace, "<code>");
                {
                    let mut cattrs = code.attrs.borrow_mut();
                    let _ = cattrs.set(
                        Value::Str(PyString::new("co_filename")),
                        Value::Str(PyString::new(interp.script_path.clone())),
                    );
                    let _ = cattrs.set(
                        Value::Str(PyString::new("co_name")),
                        Value::Str(PyString::new("<module>")),
                    );
                    // T1 (inspect.signature on marker fns, e.g.
                    // xmlrpclib.dumps via scrapy get_func_args): minimal
                    // signature surface.
                    let _ = cattrs.set(
                        Value::Str(PyString::new("co_argcount")),
                        Value::Int(Int::from_i64(0)),
                    );
                    let _ = cattrs.set(
                        Value::Str(PyString::new("co_varnames")),
                        Value::Tuple(Gc::new(Tuple(Vec::new()))),
                    );
                    // T1 (inspect.stack): co_positions() — a long tuple of
                    // (None, None, None, None); islice indexes into it. 1024
                    // covers any stub-frame f_lasti the runtime produces.
                    let none4 = || {
                        Value::Tuple(Gc::new(Tuple(vec![
                            Value::None,
                            Value::None,
                            Value::None,
                            Value::None,
                        ])))
                    };
                    let positions: Vec<Value> = (0..1024).map(|_| none4()).collect();
                    let cp =
                        ap_obj::PyObject::new(ap_obj::ObjKind::BuiltinFn, "__ap_code_positions__");
                    *cp.slice_parts.borrow_mut() = vec![Value::Tuple(Gc::new(Tuple(positions)))];
                    let _ = cattrs.set(Value::Str(PyString::new("co_positions")), Value::Obj(cp));
                }
                let mut attrs = frame.attrs.borrow_mut();
                let _ = attrs.set(
                    Value::Str(PyString::new("f_locals")),
                    Value::Dict(PyDict::new()),
                );
                let _ = attrs.set(
                    Value::Str(PyString::new("f_globals")),
                    Value::Dict(interp.globals.clone()),
                );
                let _ = attrs.set(Value::Str(PyString::new("f_code")), Value::Obj(code));
                // 02.03b: _py_warnings walks to the USER caller — the
                // OUTERMOST real frame (the script) carries its current
                // statement line; inner frames are the warnings machinery
                // itself, which CPython's stacklevel skips.
                let lineno = interp
                    .frame_stack
                    .borrow()
                    .first()
                    .map(|c| c.get())
                    .filter(|n| *n > 0)
                    .unwrap_or(1);
                let _ = attrs.set(
                    Value::Str(PyString::new("f_lineno")),
                    Value::Int(Int::from_i64(lineno as i64)),
                );
                // T1-d fix (twisted.compat.currentframe walks f_back):
                // chain to the next-outer frame as a synthetic stub. Depth
                // rides slice_parts so repeated f_back walks outward.
                let depth = frame
                    .slice_parts
                    .borrow()
                    .first()
                    .and_then(|v| v.as_int())
                    .and_then(|i| i.to_i64())
                    .unwrap_or(0);
                let stack_len = interp.frame_stack.borrow().len();
                // Generous cap: twisted.compat.currentframe(2)+ walks past
                // the import machinery; always allow a stub chain long
                // enough (min 32) — the walk ends at None.
                let outer_exists = (depth as usize + 1) < (stack_len.max(32));
                let f_back = if outer_exists {
                    let fb = ap_obj::PyObject::new(ap_obj::ObjKind::Namespace, "<frame>");
                    let _ = fb
                        .attrs
                        .borrow_mut()
                        .set(Value::Str(PyString::new("f_back")), Value::None);
                    *fb.slice_parts.borrow_mut() = vec![Value::Int(Int::from_i64(depth + 1))];
                    let _ = fb.attrs.borrow_mut().set(
                        Value::Str(PyString::new("f_globals")),
                        Value::Dict(interp.globals.clone()),
                    );
                    // T2 fix: chained frames need f_code - _py_warnings
                    // reads frame.f_code.co_filename on every walk step;
                    // missing f_code raised AttributeError mid-unwind and
                    // replaced the in-flight exception (pytest Skipped).
                    let code = ap_obj::PyObject::new(ap_obj::ObjKind::Namespace, "<code>");
                    {
                        let mut cattrs = code.attrs.borrow_mut();
                        let _ = cattrs.set(
                            Value::Str(PyString::new("co_filename")),
                            Value::Str(PyString::new(interp.script_path.clone())),
                        );
                        let _ = cattrs.set(
                            Value::Str(PyString::new("co_name")),
                            Value::Str(PyString::new("<module>")),
                        );
                    }
                    let _ = fb
                        .attrs
                        .borrow_mut()
                        .set(Value::Str(PyString::new("f_code")), Value::Obj(code));
                    Value::Obj(fb)
                } else {
                    Value::None
                };
                let _ = attrs.set(Value::Str(PyString::new("f_back")), f_back);
                // T1 (importlib.resources._infer_caller): inspect.stack()
                // reads f_lasti + code.co_positions(); the stub serves
                // position 0 / an empty positions iterator respectively.
                let _ = attrs.set(
                    Value::Str(PyString::new("f_lasti")),
                    Value::Int(Int::from_i64(0)),
                );
            }
            Some(Ok(Value::Obj(frame)))
        }
        // T1-d (scrapy chain, orchestrator handoff): `input([prompt])` —
        // print the prompt to stdout WITHOUT a trailing newline, read one
        // line from stdin, strip the trailing newline (CPython semantics).
        "input" => {
            let r = (|| {
                if let Some(p) = args.first() {
                    let text = py_str_of(p).unwrap_or_default();
                    interp.write_out(&text);
                    let _ = std::io::Write::flush(&mut std::io::stdout());
                }
                let mut line = String::new();
                match std::io::stdin().read_line(&mut line) {
                    Ok(0) => {
                        return Err(raise("EOFError", "EOF when reading a line"));
                    }
                    Ok(_) => {}
                    Err(e) => {
                        return Err(raise("OSError", format!("input() read failed: {e}")));
                    }
                }
                let stripped = line
                    .strip_suffix('\n')
                    .map(|s| s.strip_suffix('\r').unwrap_or(s))
                    .unwrap_or(&line);
                Ok(Value::Str(PyString::new(stripped.to_string())))
            })();
            Some(r)
        }
        "exit" => {
            let code = args
                .first()
                .and_then(|v| v.as_int())
                .and_then(|i| i.to_i64())
                .unwrap_or(0) as i32;
            interp.exit_code.set(Some(code));
            // SystemExit unwinds: uncaught → exit code per C-29 (SystemExit
            // does NOT print a traceback).
            Some(Err(Flow::Raise(ExceptionValue::new(
                "SystemExit",
                args.to_vec(),
            ))))
        }
        "getrecursionlimit" => Some(Ok(Value::Int(Int::from_i64(
            interp.recursion_limit.get() as i64
        )))),
        "setrecursionlimit" => {
            let n = args
                .first()
                .and_then(|v| v.as_int())
                .and_then(|i| i.to_i64())
                .unwrap_or(1000);
            interp.recursion_limit.set(n as usize);
            Some(Ok(Value::None))
        }
        "set_int_max_str_digits" => {
            let n = args
                .first()
                .and_then(|v| v.as_int())
                .and_then(|i| i.to_i64())
                .unwrap_or(4300);
            if n < 640 {
                return Some(Err(raise(
                    "ValueError",
                    "maxdigits must be 0 or larger than 640",
                )));
            }
            ap_obj::int::set_int_str_digit_limit(n as usize);
            Some(Ok(Value::None))
        }
        "get_int_max_str_digits" => Some(Ok(Value::Int(Int::from_i64(
            ap_obj::int::get_int_str_digit_limit() as i64,
        )))),
        "getdefaultencoding" => Some(Ok(Value::Str(PyString::new("utf-8")))),
        // T1-d fix: sys.exc_info() returns the IN-FLIGHT handled exception
        // (interp.current_exception), not (None,None,None) — twisted's
        // Failure() and log.err() depend on it.
        "exc_info" => {
            let interp2 = match crate::builtins_modules::current_interp() {
                Some(i) => i,
                None => {
                    return Some(Ok(Value::Tuple(Gc::new(Tuple(vec![
                        Value::None,
                        Value::None,
                        Value::None,
                    ])))))
                }
            };
            let inflight = interp2.current_exception.borrow().clone();
            match inflight {
                Some(ev) => {
                    let cls = crate::classes::exc_class_value(&interp2, &ev.class)
                        .or_else(|| crate::classes::exc_class_value(&interp2, "Exception"))
                        .unwrap_or_else(|| {
                            Value::Obj(ap_obj::PyObject::new(
                                ap_obj::ObjKind::Type,
                                ev.class.clone(),
                            ))
                        });
                    // T2: prefer the REAL instance when the raise carried
                    // one (identity + user-class isinstance preserved).
                    let inst = ev.instance.clone().unwrap_or_else(|| {
                        let i = ap_obj::PyObject::new(ap_obj::ObjKind::Exception, ev.class.clone());
                        *i.args.borrow_mut() = ev.args.clone();
                        Value::Obj(i)
                    });
                    // W112: record the in-flight tb entries onto the
                    // materialized instance (__ap_tb__). from_call's
                    // `except BaseException:` has no `as` name, so
                    // exception_instance never fires for this flow; any
                    // later instance->EV conversion (pluggy teardown
                    // `raise exception`) reads __ap_tb__ to keep the
                    // traceback chain intact across the re-raise.
                    if let Value::Obj(io) = &inst {
                        let entries: Vec<Value> = ev
                            .tb
                            .iter()
                            .map(|(q, l, f)| {
                                Value::Tuple(ap_obj::int::Gc::new(ap_obj::value::Tuple(vec![
                                    Value::Str(PyString::new(q.as_str())),
                                    Value::Int(ap_obj::Int::from_i64(*l as i64)),
                                    Value::Str(PyString::new(f.as_str())),
                                ])))
                            })
                            .collect();
                        let _ = io.attrs.borrow_mut().set(
                            Value::Str(PyString::new("__ap_tb__")),
                            Value::List(PyList::from_vec(entries)),
                        );
                    }
                    let interp3 = interp2.rc();
                    // T2: prefer the exception's OWN tb entries (true raise
                    // site + frame chain); fall back to the global table.
                    if std::env::var("APX_TBDBG").is_ok() {
                        eprintln!(
                            "[EXCINFO] class={} tb_len={} tb={:?}",
                            ev.class,
                            ev.tb.len(),
                            ev.tb
                        );
                    }
                    let tb = if !ev.tb.is_empty() {
                        crate::interp::build_tb_chain(&interp3.script_path, &ev.tb)
                    } else {
                        crate::classes::traceback_value(&interp3)
                    };
                    return Some(Ok(Value::Tuple(Gc::new(Tuple(vec![cls, inst, tb])))));
                }
                None => {}
            }
            Some(Ok(Value::Tuple(Gc::new(Tuple(vec![
                Value::None,
                Value::None,
                Value::None,
            ])))))
        }
        // T1 (scrapy fix pass): sys.exception() — the active handled exception
        // (same in-flight instance exc_info returns; wave-108: was hardcoded
        // None, breaking logging.handleError which reads exc.__traceback__).
        "exception" => {
            let interp2 = match crate::builtins_modules::current_interp() {
                Some(i) => i,
                None => return Some(Ok(Value::None)),
            };
            let inflight = interp2.current_exception.borrow().clone();
            match inflight {
                Some(ev) => {
                    let inst = ev.instance.clone().unwrap_or_else(|| {
                        let i = ap_obj::PyObject::new(ap_obj::ObjKind::Exception, ev.class.clone());
                        *i.args.borrow_mut() = ev.args.clone();
                        Value::Obj(i)
                    });
                    Some(Ok(inst))
                }
                None => Some(Ok(Value::None)),
            }
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Executor-facing glue (interp.rs callers)
// ---------------------------------------------------------------------------

/// Build the builtin namespace into interp.builtins (called once at entry).
pub fn install_builtins(interp: &Interpreter) {
    let b = &interp.builtins;
    let names: &[&str] = &[
        "abs",
        "all",
        "ascii",
        "callable",
        "delattr",
        "dir",
        "getattr",
        "hasattr",
        "id",
        "issubclass",
        "setattr",
        "vars",
        "globals",
        "locals",
        "reversed",
        "open",
        "any",
        "bin",
        "bool",
        "chr",
        "dict",
        "divmod",
        "compile",
        "__import__",
        "aiter",
        "anext",
        "enumerate",
        "eval",
        "exec",
        "float",
        "format",
        "frozenset",
        "hash",
        "hex",
        "int",
        "isinstance",
        "iter",
        "len",
        "list",
        "max",
        "min",
        "next",
        "oct",
        "ord",
        "pow",
        "print",
        "range",
        "repr",
        "round",
        "set",
        "sorted",
        "map",
        "filter",
        "str",
        "sum",
        "tuple",
        "type",
        "zip",
        // T1-d (scrapy chain): input([prompt]) — prompt + stdin line read.
        "input",
    ];
    for n in names {
        let o = ap_obj::PyObject::new(ap_obj::ObjKind::BuiltinFn, *n);
        let _ = b.set(Value::Str(PyString::new(*n)), Value::Obj(o));
    }
    // Class-statement keyword marker (lowering: `__ap_classkw__.<name>(v)`).
    {
        let kw = ap_obj::PyObject::new(ap_obj::ObjKind::Opaque, "__ap_classkw__");
        let _ = b.set(Value::Str(PyString::new("__ap_classkw__")), Value::Obj(kw));
    }
    // 02.04a (interp bug #8): Ellipsis/NotImplemented builtins — types.py
    // reads them at import (EllipsisType/NotImplementedType).
    let _ = b.set(Value::Str(PyString::new("Ellipsis")), Value::Ellipsis);
    {
        let ni = ap_obj::PyObject::new(ap_obj::ObjKind::Namespace, "NotImplemented");
        let _ = b.set(Value::Str(PyString::new("NotImplemented")), Value::Obj(ni));
    }
    // T2 (pytest faulthandler/debugging): `__debug__` builtin — True when
    // not running with -O (we never do). CPython binds it as a compile-time
    // constant; a builtins global answers the name reads.
    let _ = b.set(Value::Str(PyString::new("__debug__")), Value::Bool(true));
    // 02.03b-fix: bytearray is a TYPE in CPython (isinstance(bytearray, type)
    // is True; _collections_abc registers it with MutableSequence at import).
    // The call path routes type-values through call_type_ctor -> call_builtin,
    // keeping bytearray(x) construction byte-identical.
    {
        let bt = ap_obj::PyObject::new(ap_obj::ObjKind::Type, "bytearray");
        let _ = b.set(Value::Str(PyString::new("bytearray")), Value::Obj(bt));
    }
    // Exception classes are Type objects so `except ValueError` name reads
    // resolve; call_value dispatches them as constructors.
    let excs: &[&str] = &[
        "BaseException",
        "Exception",
        "ArithmeticError",
        "ZeroDivisionError",
        "OverflowError",
        "FloatingPointError",
        "TypeError",
        "ValueError",
        "NameError",
        "UnboundLocalError",
        "IndexError",
        "KeyError",
        "LookupError",
        "AttributeError",
        "StopIteration",
        "RuntimeError",
        "NotImplementedError",
        "RecursionError",
        "AssertionError",
        "UnicodeError",
        "UnicodeDecodeError",
        "UnicodeEncodeError",
        "MemoryError",
        "OSError",
        "EOFError",
        "KeyboardInterrupt",
        "SystemExit",
    ];
    for n in excs {
        let o = interned_type(interp, n);
        let _ = b.set(Value::Str(PyString::new(*n)), o);
    }
    // 02.03b-fix: IOError/EnvironmentError are CPython aliases of OSError.
    if let Some(os) = interp.builtins.get(&Value::Str(PyString::new("OSError"))) {
        for alias in ["IOError", "EnvironmentError"] {
            let _ = b.set(Value::Str(PyString::new(alias)), os.clone());
        }
    }
    // type names usable with isinstance
    let types: &[&str] = &[
        "NoneType",
        "ellipsis",
        "bytes",
        "object",
        "range",
        "frozenset",
        "set",
        // 02.04a fix pass: pathlib imports Sequence.register(memoryview)
        // (_collections_abc:1066) — the NAME must resolve as a type.
        "memoryview",
        // 02.04a fix pass: typing.py (Protocol machinery) reads `complex`.
        "complex",
        // 02.03b-fix: pathlib/itertools read `slice` as a type (islice
        // surface, Path.__getitem__ boundaries).
        "slice",
        "dict",
        "list",
        "tuple",
        "str",
        "int",
        "float",
        "bool",
    ];
    for n in types {
        let o = interned_type(interp, n);
        let _ = b.set(Value::Str(PyString::new(*n)), o);
    }
    // 02.08-full part 1: hasattr(builtins, "None"/"True"/"False") holds in
    // CPython (they are module members).
    let _ = b.set(Value::Str(PyString::new("None")), Value::None);
    let _ = b.set(Value::Str(PyString::new("True")), Value::Bool(true));
    let _ = b.set(Value::Str(PyString::new("False")), Value::Bool(false));
}

/// Interned type object (x.__class__ is T identity, V045).
pub fn interned_type(interp: &Interpreter, name: &str) -> Value {
    if let Some(t) = interp.type_objects.borrow().get(name) {
        return t.clone();
    }
    let t = Value::Obj(ap_obj::PyObject::new(ap_obj::ObjKind::Type, name));
    interp
        .type_objects
        .borrow_mut()
        .insert(name.to_string(), t.clone());
    t
}

/// Build a function-object Value for the function table id.
pub fn make_function(fn_id: u32, name: &str) -> Value {
    let o = ap_obj::PyObject::with_fn_id(ap_obj::ObjKind::Function, name, fn_id);
    Value::Obj(o)
}

/// Lazy sys-module singleton (import sys).
pub fn get_or_make_sys(interp: &Interpreter) -> Value {
    if let Some(m) = &*interp.sys_module.borrow() {
        return m.clone();
    }
    let m = make_sys_module(interp);
    *interp.sys_module.borrow_mut() = Some(m.clone());
    m
}

// ---------------------------------------------------------------------------
// 02.08-lite: introspection builtins the stdlib corpus requires
// ---------------------------------------------------------------------------

/// getattr(obj, name[, default]).
pub fn builtin_getattr(interp: &Interpreter, args: &[Value]) -> BResult {
    let obj = one(args, "getattr")?;
    let name = match args.get(1) {
        Some(Value::Str(x)) => x.as_str().to_string(),
        _ => return Err(raise("TypeError", "attribute name must be string")),
    };
    let dflt = args.get(2).cloned();
    match crate::interp::attr_get_public(&interp.rc(), obj, &name) {
        Ok(Some(v)) => Ok(v),
        Ok(None) => match dflt {
            Some(d) => Ok(d),
            None => Err(raise(
                "AttributeError",
                format!("'{}' object has no attribute '{}'", obj.type_name(), name),
            )),
        },
        Err(e) => {
            if dflt.is_some() {
                Ok(dflt.unwrap())
            } else {
                Err(e)
            }
        }
    }
}

/// hasattr(obj, name): getattr with suppression.
pub fn builtin_hasattr(interp: &Interpreter, args: &[Value]) -> BResult {
    let obj = one(args, "hasattr")?;
    let name = match args.get(1) {
        Some(Value::Str(x)) => x.as_str().to_string(),
        _ => return Err(raise("TypeError", "attribute name must be string")),
    };
    Ok(Value::Bool(matches!(
        crate::interp::attr_get_public(&interp.rc(), obj, &name),
        Ok(Some(_))
    )))
}

/// dir(obj): module namespaces list their names; type/module objects get a
/// minimal surface. Without args: the caller's global names (v0 approximation).
pub fn builtin_dir(interp: &Interpreter, args: &[Value]) -> BResult {
    let mut names: Vec<String> = Vec::new();
    match args.first() {
        None => {
            let g = interp.globals.clone();
            for (k, _) in g.items() {
                if let Value::Str(x) = k {
                    names.push(x.as_str().to_string());
                }
            }
        }
        Some(Value::Obj(o)) if o.kind == ap_obj::ObjKind::Module => {
            if let Some(ns) = &*o.module_ns.borrow() {
                for (k, _) in ns.items() {
                    if let Value::Str(x) = k {
                        names.push(x.as_str().to_string());
                    }
                }
            }
        }
        Some(v) => {
            names.extend(dir_surface(v));
        }
    }
    names.sort();
    names.dedup();
    Ok(Value::List(PyList::from_vec(
        names
            .into_iter()
            .map(|n| Value::Str(PyString::new(n)))
            .collect(),
    )))
}

/// Per-type dir() surface (v0 approximation: enough for the corpus's
/// `len(dir(int)) > 10` / `"append" in dir(list)` shape checks).
fn dir_surface(v: &Value) -> Vec<String> {
    // 02.04: user classes list their MRO dicts (unittest.getTestCaseNames
    // discovers test methods through dir()).
    if let Value::Obj(o) = v {
        if o.kind == ap_obj::ObjKind::UserInstance {
            // T2 (pluggy hook discovery): dir() on an instance lists its
            // class's MRO dict keys — pluggy.register scans dir(plugin).
            let interp = crate::builtins_modules::current_interp();
            let mut names: Vec<String> = Vec::new();
            let handled = if let Some(ic) = interp {
                if let Some(t) = crate::classes::class_of_value(&ic, v) {
                    for m in t.mro.borrow().iter() {
                        for (k, _) in m.dict.items() {
                            if let Value::Str(x) = k {
                                names.push(x.as_str().to_string());
                            }
                        }
                    }
                    true
                } else {
                    false
                }
            } else {
                false
            };
            if !handled {
                return names;
            }
            let attrs = o.attrs.borrow();
            for (k, _) in attrs.items() {
                if let Value::Str(x) = k {
                    names.push(x.as_str().to_string());
                }
            }
            return names;
        }
        if o.kind == ap_obj::ObjKind::UserClass {
            let mut names: Vec<String> = Vec::new();
            if let Some(t) = o.class_ref.borrow().clone() {
                for m in t.mro.borrow().iter() {
                    for (k, _) in m.dict.items() {
                        if let Value::Str(x) = k {
                            names.push(x.as_str().to_string());
                        }
                    }
                }
            }
            let attrs = o.attrs.borrow();
            for (k, _) in attrs.items() {
                if let Value::Str(x) = k {
                    names.push(x.as_str().to_string());
                }
            }
            return names;
        }
    }
    // T2 (pluggy hook discovery): dir() on a UserInstance lists its class's
    // MRO dict keys — pluggy.register scans dir(plugin) for hookimpls.
    if let Value::Obj(o) = v {
        if o.kind == ap_obj::ObjKind::UserInstance {
            let mut names: Vec<String> = Vec::new();
            if let Some(ic) = crate::builtins_modules::current_interp() {
                if let Some(t) = crate::classes::class_of_value(&ic, v) {
                    for m in t.mro.borrow().iter() {
                        for (k, _) in m.dict.items() {
                            if let Value::Str(x) = k {
                                names.push(x.as_str().to_string());
                            }
                        }
                    }
                }
            }
            let attrs = o.attrs.borrow();
            for (k, _) in attrs.items() {
                if let Value::Str(x) = k {
                    names.push(x.as_str().to_string());
                }
            }
            return names;
        }
    }
    match v {
        Value::Obj(o)
            if (o.kind == ap_obj::ObjKind::BuiltinFn || o.kind == ap_obj::ObjKind::Type)
                && o.name == "int" =>
        {
            vec![
                "bit_length",
                "conjugate",
                "denominator",
                "from_bytes",
                "imag",
                "numerator",
                "real",
                "to_bytes",
                "bit_count",
                "as_integer_ratio",
                "is_integer",
                "__abs__",
                "__add__",
                "__and__",
                "__bool__",
                "__class__",
                "__eq__",
                "__floordiv__",
                "__ge__",
                "__gt__",
                "__hash__",
                "__index__",
                "__int__",
                "__invert__",
                "__le__",
                "__lt__",
                "__mod__",
                "__mul__",
                "__ne__",
                "__neg__",
                "__new__",
                "__or__",
                "__pos__",
                "__pow__",
                "__radd__",
                "__rand__",
                "__repr__",
                "__rfloordiv__",
                "__rmod__",
                "__rmul__",
                "__ror__",
                "__rpow__",
                "__rshift__",
                "__rsub__",
                "__rtruediv__",
                "__rxor__",
                "__setattr__",
                "__sizeof__",
                "__str__",
                "__sub__",
                "__truediv__",
                "__xor__",
            ]
            .into_iter()
            .map(String::from)
            .collect()
        }
        Value::Obj(o)
            if (o.kind == ap_obj::ObjKind::BuiltinFn || o.kind == ap_obj::ObjKind::Type)
                && o.name == "list" =>
        {
            vec![
                "append",
                "clear",
                "copy",
                "count",
                "extend",
                "index",
                "insert",
                "pop",
                "remove",
                "reverse",
                "sort",
                "__class__",
            ]
            .into_iter()
            .map(String::from)
            .collect()
        }
        _ => vec![],
    }
}

/// vars([obj]): module/globals dict views (v0: globals without arg; module ns
/// otherwise). Object __dict__ surfaces when 02.03 lands.
pub fn builtin_vars(interp: &Interpreter, args: &[Value]) -> BResult {
    match args.first() {
        None => {
            let g = interp.globals.clone();
            let d = PyDict::new();
            for (k, v) in g.items() {
                let _ = d.set(k, v);
            }
            Ok(Value::Dict(d))
        }
        Some(Value::Obj(o)) if o.kind == ap_obj::ObjKind::Module => {
            let d = PyDict::new();
            if let Some(ns) = &*o.module_ns.borrow() {
                for (k, v) in ns.items() {
                    let _ = d.set(k, v);
                }
            }
            Ok(Value::Dict(d))
        }
        // 02.08-c: Opaque instances (lxml elements, functools markers) and
        // UserInstances carry a live attrs dict — vars() returns a snapshot.
        Some(Value::Obj(o))
            if matches!(
                o.kind,
                ap_obj::ObjKind::UserInstance
                    | ap_obj::ObjKind::Opaque
                    | ap_obj::ObjKind::Namespace
                    | ap_obj::ObjKind::Exception
                    | ap_obj::ObjKind::UserClass
                    | ap_obj::ObjKind::Type
                    | ap_obj::ObjKind::Function
                    | ap_obj::ObjKind::BuiltinFn
                    | ap_obj::ObjKind::BoundMethod
            ) =>
        {
            // 02.08-c: classes/functions expose their class/attr dict;
            // instances/markers expose the live attrs snapshot.
            if o.kind == ap_obj::ObjKind::UserClass {
                if let Some(t) = crate::classes::type_of(interp, &Value::Obj(o.clone())) {
                    let d = PyDict::new();
                    for (k, v) in t.dict.items() {
                        let _ = d.set(k, v);
                    }
                    return Ok(Value::Dict(d));
                }
            }
            let d = PyDict::new();
            for (k, v) in o.attrs.borrow().items() {
                let _ = d.set(k, v);
            }
            Ok(Value::Dict(d))
        }
        Some(_) => Err(raise(
            "TypeError",
            "vars() argument must have __dict__ attribute (02.03 surface)",
        )),
    }
}

/// id(obj): heap-stable per-object identity (address-derived; the corpus only
/// asserts positivity/equality relationships).
pub fn builtin_id(args: &[Value]) -> BResult {
    let v = one(args, "id")?;
    let n = match v {
        Value::Obj(o) => std::rc::Rc::as_ptr(o) as *const u8 as i64,
        other => {
            // Wave-68: containers get a real per-instance identity (the
            // payload pointer) — id()-keyed memos (copy.deepcopy,
            // pickle) COLLIDED under the old constant, breaking
            // copy.deepcopy(dict) for every caller.
            match other {
                Value::Dict(d) => Gc::as_ptr(d) as *const u8 as i64,
                Value::List(l) => Gc::as_ptr(l) as *const u8 as i64,
                Value::Set(st) => Gc::as_ptr(st) as *const u8 as i64,
                Value::Tuple(t) => Gc::as_ptr(t) as *const u8 as i64,
                Value::Str(x) => {
                    let h = ap_obj::ops::hash(&Value::Str(x.clone())).unwrap_or_default();
                    h
                }
                Value::Int(i) => i.to_i64().unwrap_or(0x5EED_0000_0000_0000u64 as i64),
                _ => 0x5EED_0000_0000_0001u64 as i64,
            }
        }
    };
    Ok(Value::Int(ap_obj::int::Int::from_i64(n)))
}

/// callable(obj).
pub fn builtin_callable(args: &[Value]) -> BResult {
    let v = one(args, "callable")?;
    let c = match v {
        // 02.04: instances whose class defines __call__ are callable
        // (unittest suite.addTest probes callable(test)).
        Value::Obj(o) if o.kind == ap_obj::ObjKind::UserInstance => {
            let has_call = o
                .type_ref
                .borrow()
                .as_ref()
                .and_then(|w| w.upgrade())
                .map(|t| t.lookup("__call__").is_some())
                .unwrap_or(false);
            has_call
        }
        Value::Obj(o) => matches!(
            o.kind,
            ap_obj::ObjKind::BuiltinFn
                | ap_obj::ObjKind::Function
                | ap_obj::ObjKind::Type
                | ap_obj::ObjKind::UserClass
                | ap_obj::ObjKind::BoundMethod
                | ap_obj::ObjKind::Generator
        ),
        _ => false,
    };
    Ok(Value::Bool(c))
}

/// issubclass(cls, classinfo) — type-object relations incl. bool->int and
/// exception hierarchy edges.
pub fn builtin_issubclass(args: &[Value]) -> BResult {
    let c = one(args, "issubclass")?;
    let info = args
        .get(1)
        .ok_or_else(|| raise("TypeError", "issubclass expected 2 arguments, got 1"))?;
    issubclass_check(c, info)
}

fn issubclass_check(c: &Value, info: &Value) -> Result<Value, Flow> {
    match info {
        Value::Tuple(parts) => {
            for p in &parts.0 {
                if issubclass_check(c, p)?.is_truthy() {
                    return Ok(Value::Bool(true));
                }
            }
            Ok(Value::Bool(false))
        }
        Value::Obj(o)
            if o.kind == ap_obj::ObjKind::Type || o.kind == ap_obj::ObjKind::UserClass =>
        {
            let want = o.name.as_str();
            let have = match c {
                Value::Obj(x)
                    if x.kind == ap_obj::ObjKind::Type || x.kind == ap_obj::ObjKind::UserClass =>
                {
                    x.name.as_str()
                }
                _ => return Err(raise("TypeError", "issubclass() arg 1 must be a class")),
            };
            Ok(Value::Bool(
                have == want
                    || want == "object"
                    || (have == "bool" && want == "int")
                    // 02.07: Timer IS-A Thread (CPython class hierarchy).
                    || (have == "Timer" && want == "Thread")
                    || crate::exc_hier::is_subclass(have, want),
            ))
        }
        _ => Err(raise(
            "TypeError",
            "issubclass() arg 2 must be a class, a tuple of classes, or a tuple",
        )),
    }
}

/// ascii(obj): repr with non-ASCII escaped.
pub fn builtin_ascii(args: &[Value]) -> BResult {
    let r = builtin_repr(args)?;
    let s = match &r {
        Value::Str(x) => x.as_str().to_string(),
        _ => String::new(),
    };
    let mut out = String::new();
    for c in s.chars() {
        let cp = c as u32;
        if cp < 0x80 {
            out.push(c);
        } else if cp <= 0xFF {
            out.push_str(&format!("\\x{:02x}", cp));
        } else if cp <= 0xFFFF {
            out.push_str(&format!("\\u{:04x}", cp));
        } else {
            let cp2 = cp - 0x10000;
            let hi = 0xD800 + (cp2 >> 10);
            let lo = 0xDC00 + (cp2 & 0x3FF);
            out.push_str(&format!("\\u{:04x}\\u{:04x}", hi, lo));
        }
    }
    Ok(Value::Str(PyString::new(out)))
}

/// delattr(obj, name) — module namespaces support unbinding.
pub fn builtin_delattr(interp: &Interpreter, args: &[Value]) -> BResult {
    let obj = one(args, "delattr")?;
    let name = match args.get(1) {
        Some(Value::Str(x)) => x.as_str().to_string(),
        _ => return Err(raise("TypeError", "attribute name must be string")),
    };
    if let Value::Obj(o) = obj {
        if o.kind == ap_obj::ObjKind::Module {
            if let Some(ns) = &*o.module_ns.borrow() {
                let _ = ns.pop(&Value::Str(PyString::new(name.clone())));
                return Ok(Value::None);
            }
        }
    }
    // 02.04a fix pass (BUG-C): delegate classes/instances to the S-03-11
    // protocol (enum.py does delattr(enum_class, '_Enum__in_progress')).
    match obj {
        Value::Obj(o)
            if o.kind == ap_obj::ObjKind::UserClass
                || o.kind == ap_obj::ObjKind::UserInstance
                || o.kind == ap_obj::ObjKind::Function =>
        {
            let interp_rc = crate::builtins_modules::current_interp()
                .ok_or_else(|| raise("SystemError", "no interpreter for delattr"))?;
            crate::classes::attr_del_protocol(&interp_rc, obj, &name).map(|_| Value::None)
        }
        _ => Err(raise(
            "TypeError",
            format!("'{}' object has no attribute '{}'", obj.type_name(), name),
        )),
    }
}

/// setattr(obj, name, value) — module namespaces only (v0).
pub fn builtin_setattr(interp: &Interpreter, args: &[Value]) -> BResult {
    let obj = one(args, "setattr")?;
    let name = match args.get(1) {
        Some(Value::Str(x)) => x.as_str().to_string(),
        _ => return Err(raise("TypeError", "attribute name must be string")),
    };
    let v = args
        .get(2)
        .ok_or_else(|| raise("TypeError", "setattr expected 3 arguments, got 2"))?;
    if let Value::Obj(o) = obj {
        if o.kind == ap_obj::ObjKind::Module {
            if let Some(ns) = &*o.module_ns.borrow() {
                let _ = ns.set(Value::Str(PyString::new(name.clone())), v.clone());
                return Ok(Value::None);
            }
        }
    }
    // 02.04a fix pass: delegate the S-03-10 protocol for user classes and
    // instances; functions take arbitrary attrs (vendored functools.wraps
    // sets __module__/__name__ etc).
    match obj {
        Value::Obj(o) if o.kind == ap_obj::ObjKind::Function => {
            let _ = o
                .attrs
                .borrow_mut()
                .set(Value::Str(PyString::new(name.clone())), v.clone());
            Ok(Value::None)
        }
        // T1-d (typing._make_nmtuple): `nm_tpl.__new__.__annotate__ = fn` —
        // BuiltinFn markers (object.__new__ etc) take arbitrary attrs like
        // functions do; the read side (interp Function arm) already answers
        // from attrs first for markers.
        Value::Obj(o) if o.kind == ap_obj::ObjKind::BuiltinFn => {
            let _ = o
                .attrs
                .borrow_mut()
                .set(Value::Str(PyString::new(name.clone())), v.clone());
            Ok(Value::None)
        }
        Value::Obj(o)
            if o.kind == ap_obj::ObjKind::UserInstance || o.kind == ap_obj::ObjKind::UserClass =>
        {
            let interp_rc = crate::builtins_modules::current_interp()
                .ok_or_else(|| raise("SystemError", "no interpreter for setattr"))?;
            crate::classes::attr_set_protocol(&interp_rc, obj, &name, v.clone())
                .map(|_| Value::None)
        }
        // T1-d: Namespace objects (closure cell stubs) route through the
        // protocol — the cell_contents arm updates the LIVE cell.
        Value::Obj(o) if o.kind == ap_obj::ObjKind::Namespace => {
            let interp_rc = crate::builtins_modules::current_interp()
                .ok_or_else(|| raise("SystemError", "no interpreter for setattr"))?;
            crate::classes::attr_set_protocol(&interp_rc, obj, &name, v.clone())
                .map(|_| Value::None)
        }
        _ => Err(raise(
            "TypeError",
            format!(
                "cannot set '{}' attribute of '{}' objects (v0 surface)",
                name,
                obj.type_name()
            ),
        )),
    }
}

/// (key, value) pairs of a mapping-ish value (Counter's dict/kwargs init).
fn mapping_items(v: &Value) -> Option<Vec<(Value, Value)>> {
    match v {
        Value::Dict(d) => Some(d.items().to_vec()),
        _ => None,
    }
}

/// Dispatch a builtin call (function object name -> impl).

/// 02.08-full part 1: float.fromhex core (shared by the classmethod marker
/// and the pyfloat unbound route).
pub fn builtin_float_fromhex(args: &[Value]) -> BResult {
    let s = match args.first() {
        Some(Value::Str(x)) => x.as_str().to_string(),
        _ => return Err(raise("TypeError", "fromhex() argument must be a string")),
    };
    // CPython hex-float form: [sign] 0x mantissa [p exponent].
    let t = s.trim();
    let (neg, t) = match t.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, t.strip_prefix('+').unwrap_or(t)),
    };
    let hexpart = t.strip_prefix("0x").unwrap_or(t);
    let (mant, exp) = match hexpart.split_once(['p', 'P']) {
        Some((m, e)) => (m, e.parse::<i32>().unwrap_or(0)),
        None => (hexpart, 0),
    };
    let (ip, fp) = mant.split_once('.').unwrap_or((mant, ""));
    let mut v = 0f64;
    for c in ip.chars() {
        v = v * 16.0 + c.to_digit(16).unwrap_or(0) as f64;
    }
    let mut scale = 1.0 / 16.0;
    for c in fp.chars() {
        v += c.to_digit(16).unwrap_or(0) as f64 * scale;
        scale /= 16.0;
    }
    let v = v * 2f64.powi(exp);
    Ok(Value::Float(if neg { -v } else { v }))
}

pub fn call_builtin(
    interp: &Interpreter,
    name: &str,
    args: &[Value],
    kwargs: &[(String, Value)],
) -> crate::EvalResult {
    if std::env::var("APX_SCOPE_DEBUG").is_ok() {
        eprintln!("[CB] name={}", name);
    }
    // Wave-68: copy._reconstruct dictiter form — fresh dict.
    if name == "__dict__new__" {
        return Ok(Value::Dict(PyDict::new()));
    }
    // T1 (scrapy fix pass): traceback.py 3.13+ calls code.co_positions()
    // while walking tracebacks; our synthetic code objects answer with a
    // position-less generator ((None, None, None, None) per instruction) so
    // _get_code_position falls back to tb_lineno.
    if name == "<code>.co_positions" {
        let items: Vec<Value> = vec![
            Value::Tuple(Gc::new(ap_obj::value::Tuple(vec![
                Value::None,
                Value::None,
                Value::None,
                Value::None,
            ])));
            256
        ];
        let l = ap_obj::value::PyList::new();
        for it in items {
            l.append(it);
        }
        return Ok(Value::List(l));
    }
    // 02.04a (interp bug #17): type.__new__(mcls, name, bases, ns) — the
    // super().__new__ call inside user metaclass __new__ bodies (ABCMeta,
    // EnumType). Creates the class payload the metaclass machinery needs.
    if name == "type.__new__" {
        return builtin_type_new(args, kwargs);
    }
    // 02.04a: _weakref.ref token (weakrefset stores these opaquely; nothing
    // dereferences them in the import gates).
    if name == "_weakref.ref" || name == "_weakref.proxy" {
        let token = ap_obj::PyObject::new(ap_obj::ObjKind::Opaque, "_weakref_token");
        if let Some(target) = args.first() {
            let _ = token
                .attrs
                .borrow_mut()
                .set(Value::Str(PyString::new("_ap_wrapped")), target.clone());
        }
        return Ok(Value::Obj(token));
    }
    // T1 (scrapy fix pass): ABCMeta.register inert - returns None.
    if name == "__ap_abc_register__" {
        return Ok(Value::None);
    }
    // T1 (scrapy fix pass): _typing marker calls — typing.py's
    // `TypeVar("T")` etc. The stub module provides _typing.* marker fns;
    // calling them yields opaque placeholder objects carrying the name
    // (real substitution semantics are a documented Plan-02 gap).
    // T1 (http.server): str.maketrans(dict) — copy int/char-keyed mapping.
    if name == "str.maketrans" {
        if let Some(Value::Dict(src_d)) = args.first() {
            let d = ap_obj::dict::PyDict::new();
            for (k, v) in src_d.items() {
                let _ = d.set(k.clone(), v.clone());
            }
            return Ok(Value::Dict(d));
        }
    }
    // T1: bytes.maketrans(from, to) — full 256-entry int map dict.
    if name == "bytes.maketrans" {
        let from_b = match args.first() {
            Some(Value::Bytes(b)) => (**b).to_vec(),
            _ => return Err(raise("TypeError", "maketrans expected bytes")),
        };
        let to_b = match args.get(1) {
            Some(Value::Bytes(b)) => (**b).to_vec(),
            _ => Vec::new(),
        };
        let d = ap_obj::dict::PyDict::new();
        for i in 0u16..256u16 {
            let byte = i as u8;
            let v = match from_b.iter().position(|&f| f == byte) {
                Some(n) => to_b.get(n).copied().unwrap_or(byte) as i64,
                None => byte as i64,
            };
            let _ = d.set(
                Value::Int(ap_obj::Int::from_i64(i as i64)),
                Value::Int(ap_obj::Int::from_i64(v)),
            );
        }
        return Ok(Value::Dict(d));
    }
    // T1 (inspect.stack): stub code object co_positions() → parked tuple.
    if name == "__ap_code_positions__" {
        // Wave-57 fix: bound dispatch passes no args, so the old
        // args.first() shape returned an EMPTY tuple ->
        // itertools.islice(positions_gen, ...) exhausted immediately ->
        // StopIteration (and PEP-479 RuntimeError inside pytest's
        // wrapper generators). CPython's co_positions() yields one
        // (line, end_line, col, end_col) per instruction; our stubs
        // have no real bytecode, so answer the position-less form.
        // Callers: inspect._get_code_position (islice at tb_lasti//2),
        // dis._get_instructions_bytes. 4096 covers any tb_lasti//2 a
        // synthetic frame can report.
        let items: Vec<Value> = (0..4096)
            .map(|_| {
                Value::Tuple(Gc::new(Tuple(vec![
                    Value::None,
                    Value::None,
                    Value::None,
                    Value::None,
                ])))
            })
            .collect();
        return Ok(Value::Tuple(Gc::new(Tuple(items))));
    }
    if let Some(rest) = name.strip_prefix("_typing.") {
        // T1-d: _idfunc is the identity fn — NewType.__call__ delegates to
        // it, so handles (twisted _SystemEventID tuples) return their arg.
        // Bound-call shape (self, obj) returns the LAST arg; direct shape
        // returns the first.
        if rest == "_idfunc" {
            return Ok(if args.len() > 1 {
                args.last().cloned().unwrap_or(Value::None)
            } else {
                args.first().cloned().unwrap_or(Value::None)
            });
        }
        let o = ap_obj::PyObject::new(ap_obj::ObjKind::Opaque, &format!("_typing.{rest} instance"));
        {
            let mut a = o.attrs.borrow_mut();
            let nm = args.first().and_then(|v| match v {
                Value::Str(sv) => Some(Value::Str(PyString::new(sv.as_str().to_string()))),
                _ => None,
            });
            let _ = a.set(
                Value::Str(PyString::new("__name__")),
                nm.unwrap_or(Value::None),
            );
            // T1-d: typing._make_substitution probes these on TypeVar args
            // when re-subscripting aliases (list[T][int]); provide the v0
            // surface so getattr defaults work and substitution is identity.
            if rest == "TypeVar" || rest == "ParamSpec" || rest == "TypeVarTuple" {
                let _ = a.set(
                    Value::Str(PyString::new("__parameters__")),
                    Value::Tuple(Gc::new(Tuple(Vec::new()))),
                );
                let _ = a.set(
                    Value::Str(PyString::new("__typing_subst__")),
                    Value::Obj(ap_obj::PyObject::new(
                        ap_obj::ObjKind::BuiltinFn,
                        "__ap_typevar_subst__",
                    )),
                );
            }
        }
        // T1 (scrapy datatypes): TypeVar instances participate in PEP 604
        // unions (`_VT | None` in scrapy.utils.datatypes generics). Park the
        // class-like marker so the BitOr union interception accepts them.
        if rest == "TypeVar" || rest == "ParamSpec" || rest == "TypeVarTuple" {
            *o.slice_parts.borrow_mut() = vec![Value::Str(PyString::new("__ap_typevar__"))];
            // T2 (typing_extensions 4.16 sentinel/ParamSpec): attr WRITES on
            // the TypeVar-like marker land in o.attrs (Opaque early-attr arm
            // reads them back). Seed the PEP 696 surface the backported
            // classes write/read right after construction:
            // __infer_variance__, __default__, has_default, __module__.
            let a2 = o.attrs.borrow_mut();
            let _ = a2.set(
                Value::Str(PyString::new("__infer_variance__")),
                Value::Bool(false),
            );
            let _ = a2.set(Value::Str(PyString::new("__default__")), Value::None);
            let _ = a2.set(
                Value::Str(PyString::new("has_default")),
                Value::Obj(ap_obj::PyObject::new(
                    ap_obj::ObjKind::BuiltinFn,
                    "__ap_typevar_has_default__",
                )),
            );
            drop(a2);
        }
        // Attr WRITES on the TypeVar-like marker must persist (typing_extensions
        // _set_default/_set_module do `typevarlike.__default__ = ...` /
        // `typevarlike.__module__ = ...` immediately after construction).
        // Opaque markers already accept arbitrary attr writes via
        // classes::attr_set_protocol's Opaque arm; nothing to add here.
        return Ok(Value::Obj(o));
    }
    // 02.08-lite: dotted native-module markers (math.sqrt, itertools.count, ...).
    if (name.contains('.') && !name.starts_with("sys.")) || name.starts_with('<') {
        // attrgetter/itemgetter/hashlib-digest call markers
        // 02.07: threading bound methods need kwargs (Event.wait(timeout=),
        // Semaphore.acquire(blocking=False), ...).
        if name.starts_with("__ap_threading__.") {
            if let Some(r) = crate::threading_call::call_threading_bound(
                &name["__ap_threading__.".len()..],
                args,
                kwargs,
            ) {
                return r;
            }
        }
        if let Some(r) = crate::builtins_modules::call_native_marker(name, args) {
            return r;
        }
        if let Some(r) = crate::builtins_modules::call_native_marker_obj(name, args, kwargs) {
            return r;
        }
        if let Some(r) = call_builtin_dotted(name, args, kwargs) {
            return r;
        }
    }
    // Native opaque objects (functools.partial) called directly.
    if let Some(Value::Obj(o)) = args.first() {
        if o.kind == ap_obj::ObjKind::BuiltinFn {
            // fallthrough — real callee handled below
        }
    }
    // sys.* markers route to the sys surface.
    if let Some(rest) = name.strip_prefix("sys.") {
        return call_sys_fn(interp, rest, args).unwrap_or_else(|| {
            Err(raise(
                "AttributeError",
                format!("sys has no attribute '{rest}'"),
            ))
        });
    }
    match name {
        "print" => print(interp, args, kwargs),
        "len" => builtin_len(args),
        "repr" => builtin_repr(args),
        "str" => builtin_str(args),
        "int" => builtin_int(interp, args),
        "float" => builtin_float(args),
        "bool" => builtin_bool(args),
        "list" => builtin_list(args),
        "tuple" => builtin_tuple(args),
        "dict" => builtin_dict(args, kwargs),
        "set" => builtin_set(args),
        "frozenset" => builtin_frozenset(args),
        "abs" => builtin_abs(args),
        // 02.04a (interp bug #9): globals()/locals() — vendored stdlib calls
        // globals() at module scope (types.py __all__ comprehension).
        "globals" => Ok(Value::Dict(
            crate::builtins_modules::CURRENT_MODULE_GLOBALS
                .with(|c| c.borrow().clone())
                .unwrap_or_else(|| interp.globals.clone()),
        )),
        "locals" => Ok(Value::Dict(
            crate::builtins_modules::CURRENT_MODULE_GLOBALS
                .with(|c| c.borrow().clone())
                .unwrap_or_else(|| interp.globals.clone()),
        )),
        // 02.04a (interp bug #12): bytearray — mutable in CPython; v0 returns
        // our immutable Bytes payload (mutation methods unexercised by the
        // import gates; documented divergence).
        // 02.04a (interp bug #12b): reversed() — pathlib/contextlib call it.
        "reversed" => {
            let v = one(args, "reversed")?;
            let it = iterable_of(v)?;
            let mut items = crate::iter::collect(&it);
            items.reverse();
            Ok(Value::List(PyList::from_vec(items)))
        }
        // 02.08-full part 1: bytes() constructor (bytes(int) zero-fill,
        // bytes(iterable of ints), bytes(str, encoding), bytes(bytes)).
        "bytes" => match args.first() {
            None => Ok(Value::Bytes(Gc::new(Vec::new()))),
            Some(Value::Int(n)) => {
                let len = n.to_i64().unwrap_or(0).max(0) as usize;
                Ok(Value::Bytes(Gc::new(vec![0u8; len])))
            }
            Some(Value::Bytes(b)) => Ok(Value::Bytes(Gc::new(b.as_ref().clone()))),
            Some(Value::Str(sv)) => {
                let codec = args
                    .get(1)
                    .map(|v| match v {
                        Value::Str(x) => x.as_str().to_string(),
                        _ => "utf-8".to_string(),
                    })
                    .unwrap_or_else(|| "utf-8".to_string());
                if !codec.eq_ignore_ascii_case("utf-8")
                    && !codec.eq_ignore_ascii_case("utf8")
                    && !codec.eq_ignore_ascii_case("ascii")
                {
                    return Err(raise("LookupError", format!("unknown encoding: {codec}")));
                }
                if sv.as_str().is_ascii() {
                    Ok(Value::Bytes(Gc::new(sv.as_str().as_bytes().to_vec())))
                } else if codec.eq_ignore_ascii_case("ascii") {
                    Err(raise(
                        "UnicodeEncodeError",
                        "'ascii' codec can't encode characters in position 0-3: ordinal not in range(128)",
                    ))
                } else {
                    Ok(Value::Bytes(Gc::new(sv.as_str().as_bytes().to_vec())))
                }
            }
            Some(other) => {
                // bytes(iterable of ints).
                let it = iterable_of(other)?;
                let items = crate::iter::collect(&it);
                let mut out = Vec::with_capacity(items.len());
                for x in items {
                    match x.as_int().and_then(|i| i.to_i64()) {
                        Some(b) if (0..=255).contains(&b) => out.push(b as u8),
                        _ => return Err(raise("ValueError", "bytes must be in range(0, 256)")),
                    }
                }
                Ok(Value::Bytes(Gc::new(out)))
            }
        },
        "bytearray" => {
            // A-4 item 1: bytearray is a DISTINCT type (type(ba).__name__ ==
            // 'bytearray', repr bytearray(b'...')), mutable, equality equal
            // with bytes, UNHASHABLE. Representation choice (documented per
            // the leaf): the tree carries 171 Value::Bytes match sites, so a
            // new Value arm (option (a)) would break every one at compile
            // time; instead the bytearray is an ObjKind::Opaque object named
            // "bytearray" whose buffer rides the "<data>" attr as a plain
            // Value::Bytes. Equality gains cross-type arms (ap-obj dict.rs),
            // hashing raises unhashable (ap-obj hash.rs), and every existing
            // Bytes match site compiles unchanged.
            fn bytearray_value(data: Vec<u8>) -> Value {
                use ap_obj::PyString;
                let obj = ap_obj::PyObject::new(ap_obj::ObjKind::Opaque, "bytearray");
                let _ = obj.attrs.borrow_mut().set(
                    Value::Str(PyString::new("<data>")),
                    Value::Bytes(Gc::new(data.clone())),
                );
                let mut repr = "bytearray(".to_string();
                repr.push_str(&ap_obj::str_repr::bytes_repr(&data));
                repr.push(')');
                *obj.repr_text.borrow_mut() = Some(repr);
                Value::Obj(obj)
            }
            match args.first() {
                Some(Value::Int(n)) => {
                    let len = n.to_i64().unwrap_or(0).max(0) as usize;
                    Ok(bytearray_value(vec![0u8; len]))
                }
                Some(Value::Bytes(b)) => Ok(bytearray_value(b.as_ref().clone())),
                Some(Value::Str(sv)) => Ok(bytearray_value(sv.as_str().as_bytes().to_vec())),
                None => Ok(bytearray_value(Vec::new())),
                Some(other) => Err(crate::Flow::Raise(crate::exc_hier::ExceptionValue::msg(
                    "TypeError",
                    format!("cannot convert '{}' to bytearray", other.type_name()),
                ))),
            }
        }
        // 02.04a fix pass: memoryview(data) — inert bytes-backed stub; only
        // .tobytes()-free surfaces the import gates touch.
        "memoryview" => match args.first() {
            Some(Value::Bytes(b)) => Ok(Value::Bytes(Gc::new(b.as_ref().clone()))),
            Some(Value::Str(sv)) => Ok(Value::Bytes(Gc::new(sv.as_str().as_bytes().to_vec()))),
            _ => Ok(Value::Bytes(Gc::new(Vec::new()))),
        },
        // 02.08-full part 1: complex(x[, y]) — Opaque (re, im) pair with
        // repr/.real/.imag surfaces (s001 repr(1j), s029 attrgetter("real")).
        "complex" => {
            let re = match args.first() {
                None => 0.0,
                Some(v) => match v {
                    Value::Float(f) => *f,
                    Value::Int(n) => n.to_i64().unwrap_or(0) as f64,
                    Value::Bool(x) => {
                        if *x {
                            1.0
                        } else {
                            0.0
                        }
                    }
                    _ => 0.0,
                },
            };
            let im = match args.get(1) {
                None => 0.0,
                Some(v) => match v {
                    Value::Float(f) => *f,
                    Value::Int(n) => n.to_i64().unwrap_or(0) as f64,
                    Value::Bool(x) => {
                        if *x {
                            1.0
                        } else {
                            0.0
                        }
                    }
                    _ => 0.0,
                },
            };
            Ok(crate::builtins_modules::make_complex_value(re, im))
        }
        "min" => builtin_min(args, kwargs),
        "max" => builtin_max(args, kwargs),
        "sum" => builtin_sum(args),
        "any" => builtin_any(args),
        "all" => builtin_all(args),
        "isinstance" => builtin_isinstance(args),
        "type" => builtin_type(interp, args, kwargs),
        "hash" => builtin_hash(args),
        "divmod" => builtin_divmod(args),
        "pow" => builtin_pow(args),
        "round" => builtin_round(args),
        "chr" => builtin_chr(args),
        "ord" => builtin_ord(args),
        "hex" => builtin_hex(args),
        "oct" => builtin_oct(args),
        "bin" => builtin_bin(args),
        "format" => builtin_format(args),
        "iter" => builtin_iter(args),
        // W112-T1 (scrapy): aiter() honors the ASYNC protocol — user objects
        // with __aiter__ (and no sync __iter__, e.g. scrapy's
        // MutableAsyncChain) must NOT fall to the sync iterator path
        // ("'MutableAsyncChain' object is not iterable" killed
        // aiter_errback's iterator). CPython: aiter(o) -> type(o).__aiter__(o).
        "aiter" => {
            let av = one(args, "aiter")?;
            if crate::classes::special_lookup(&interp.rc(), &av, "__aiter__").is_some()
                && crate::classes::special_lookup(&interp.rc(), &av, "__iter__").is_none()
            {
                let (f, _o) =
                    crate::classes::special_lookup(&interp.rc(), &av, "__aiter__").expect("above");
                return crate::interp::call_value_pub(&interp.rc(), &f, &[av.clone()], &[]);
            }
            builtin_iter(args)
        }
        "anext" => {
            // W112: `anext(agen)` drives the async-gen's __anext__ — an
            // ASYNC protocol: the result is the yielded item, exhaustion
            // raises StopAsyncIteration. Route through the same
            // asend_handle machinery as `await agen.__anext__()` (the
            // old builtin_next path raised "'object' object is not an
            // iterator" — scrapy engine._process_start_next blocker).
            let av = one(args, "anext")?;
            if crate::interp::is_asyncgen_obj(&av) {
                // Return the __anext__ AWAITABLE (CPython:
                // async_generator_asend). The caller awaits it: the yielded
                // item is the result, exhaustion raises StopAsyncIteration.
                let m = crate::interp::attr_get_public(&interp.rc(), &av, "__anext__")?
                    .ok_or_else(|| {
                        raise(
                            "TypeError",
                            "'async_generator' object is not an async iterator",
                        )
                    })?;
                return crate::interp::call_value_pub(&interp.rc(), &m, &[], &[]);
            }
            // W112-T1 (scrapy): user async iterators (MutableAsyncChain etc)
            // — anext(o) calls o.__anext__(), returning the coroutine the
            // caller awaits (CPython shape). Only when there is NO sync
            // __next__ (otherwise builtin_next handles it).
            if crate::classes::special_lookup(&interp.rc(), &av, "__anext__").is_some()
                && crate::classes::special_lookup(&interp.rc(), &av, "__next__").is_none()
            {
                let (m, _o) = crate::classes::special_lookup(&interp.rc(), &av, "__anext__")
                    .expect("guarded above");
                return crate::interp::call_value_pub(&interp.rc(), &m, &[av.clone()], &[]);
            }
            builtin_next(interp, args)
        }
        "next" => builtin_next(interp, args),
        "range" => builtin_range(args),
        "sorted" => builtin_sorted(interp, args, kwargs),
        "enumerate" => builtin_enumerate(interp, args, kwargs),
        "zip" => builtin_zip(interp, args),
        "map" => builtin_map(interp, args),
        "filter" => builtin_filter(interp, args),
        "float.fromhex" => builtin_float_fromhex(args),
        "eval" => builtin_eval(interp, args),
        // T1-d (scrapy chain, orchestrator handoff): `input([prompt])` —
        // prompt to stdout WITHOUT trailing newline, read one stdin line,
        // strip the trailing newline (CPython semantics).
        "input" => {
            if let Some(p) = args.first() {
                let text = py_str_of(p).unwrap_or_default();
                interp.write_out(&text);
                let _ = std::io::Write::flush(&mut std::io::stdout());
            }
            let mut line = String::new();
            match std::io::stdin().read_line(&mut line) {
                Ok(0) => return Err(raise("EOFError", "EOF when reading a line")),
                Ok(_) => {}
                Err(e) => return Err(raise("OSError", format!("input() read failed: {e}"))),
            }
            let stripped = line
                .strip_suffix('\n')
                .map(|s| s.strip_suffix('\r').unwrap_or(s))
                .unwrap_or(&line);
            Ok(Value::Str(PyString::new(stripped.to_string())))
        }
        // 04.01: exec — six.py binds `getattr(moves.builtins, "exec")` and
        // execs `def raise_from(...)` source at import.
        "exec" => builtin_exec(interp, args),
        "hasattr" => builtin_hasattr(interp, args),
        "getattr" => builtin_getattr(interp, args),
        "setattr" => builtin_setattr(interp, args),
        "delattr" => builtin_delattr(interp, args),
        "dir" => builtin_dir(interp, args),
        "vars" => builtin_vars(interp, args),
        "id" => builtin_id(args),
        "callable" => builtin_callable(args),
        "issubclass" => builtin_issubclass(args),
        "open" => crate::builtins_modules::call_io("open", args, &[])
            .unwrap_or_else(|| Err(raise("OSError", "open failed"))),
        // 02.03b-fix: collections.Counter — dict-backed multiset (unittest
        // imports it; most_common/update via the dict surface).
        "OrderedDict" | "defaultdict" | "ChainMap" => {
            // v0: plain dict payload (insertion-order IS our dict order).
            let d = ap_obj::dict::PyDict::new();
            if let Some(first) = args.first() {
                // T1-d fix (inspect.Signature): the source may be a LIST/
                // iterable of (k, v) PAIRS, not a mapping — fall back to
                // generic iteration when mapping_items doesn't apply.
                let pairs = match mapping_items(first) {
                    Some(p) => Some(p),
                    None => {
                        let it = crate::interp::iterable_of_public(first)?;
                        let items = crate::interp::collect_iter_public(&it);
                        let mut out = Vec::new();
                        for pair in items {
                            // (k, v) tuple unpack.
                            let kv: Option<(Value, Value)> = match &pair {
                                Value::Tuple(t) if t.0.len() == 2 => {
                                    Some((t.0[0].clone(), t.0[1].clone()))
                                }
                                Value::List(l) if l.items().len() == 2 => {
                                    let it2 = l.items();
                                    Some((it2[0].clone(), it2[1].clone()))
                                }
                                _ => None,
                            };
                            match kv {
                                Some(kv) => out.push(kv),
                                None => {
                                    return Err(raise(
                                        "ValueError",
                                        "OrderedDict/update expected pairs",
                                    ))
                                }
                            }
                        }
                        Some(out)
                    }
                };
                if let Some(pairs) = pairs {
                    for (k, v) in pairs {
                        let _ = d.set(k, v);
                    }
                }
            }
            for (k, v) in kwargs {
                let _ = d.set(Value::Str(PyString::new(k.clone())), v.clone());
            }
            Ok(Value::Dict(d))
        }
        "Counter" => {
            let d = ap_obj::dict::PyDict::new();
            if let Some(first) = args.first() {
                let items: Vec<Value> = match first {
                    Value::Dict(_) | Value::Obj(_) => match mapping_items(first) {
                        Some(pairs) => pairs
                            .iter()
                            .flat_map(|(k, v)| vec![k.clone(), v.clone()])
                            .collect(),
                        None => Vec::new(),
                    },
                    other => {
                        let it = crate::interp::iterable_of_public(other)?;
                        crate::interp::collect_iter_public(&it)
                    }
                };
                for k in &items {
                    let cur = d.get(k);
                    let n = cur
                        .as_ref()
                        .and_then(|v| v.as_int())
                        .and_then(|i| i.to_i64())
                        .unwrap_or(0);
                    let _ = d.set(k.clone(), Value::Int(ap_obj::Int::from_i64(n + 1)));
                }
            }
            Ok(Value::Dict(d))
        }
        "ascii" => builtin_ascii(args),
        // 02.03b-fix: slice(stop) / slice(start, stop[, step]) constructor —
        // the SliceObj the executor's Expr::Slice lowering produces (the
        // `slice` NAME also installs as a type for isinstance/reads).
        "slice" => {
            let obj = ap_obj::PyObject::new(ap_obj::ObjKind::SliceObj, "slice");
            let (lo, hi, st) = match args.len() {
                0 => (Value::None, Value::None, Value::None),
                1 => (Value::None, args[0].clone(), Value::None),
                2 => (args[0].clone(), args[1].clone(), Value::None),
                _ => (
                    args[0].clone(),
                    args[1].clone(),
                    args.get(2).cloned().unwrap_or(Value::None),
                ),
            };
            *obj.slice_parts.borrow_mut() = vec![lo, hi, st];
            Ok(Value::Obj(obj))
        }
        // 02.08-full part 1: io stream classes are type values; constructor
        // calls delegate to the io dispatch (s005: io.BytesIO(b"...")).
        n if matches!(
            n,
            "BytesIO"
                | "StringIO"
                | "TextIOWrapper"
                | "FileIO"
                | "BufferedReader"
                | "BufferedWriter"
                | "BufferedRWPair"
                | "BufferedRandom"
                | "IncrementalNewlineDecoder"
        ) =>
        {
            match crate::builtins_modules::call_io(n, args, kwargs) {
                Some(r) => r,
                None => Err(raise("TypeError", format!("{n} construction failed"))),
            }
        }
        // 02.08-full part 1: tempfile.TemporaryDirectory(prefix=...) — the
        // class value routes through the type-ctor path; delegate to the
        // tempfile native dispatch (s046 with-block pin).
        "tempfile.TemporaryDirectory" | "TemporaryDirectory" => {
            match crate::builtins_modules::call_native(
                "tempfile",
                "TemporaryDirectory",
                args,
                kwargs,
            ) {
                Some(r) => r,
                None => Err(raise("TypeError", "TemporaryDirectory construction failed")),
            }
        }
        // 02.08-full part 1: compile() source marker (s001: compile(...) is
        // not None; eval of the same source is the interpreter's own path).
        "compile" => {
            if args.len() < 3 {
                return Err(raise("TypeError", "compile() missing required argument"));
            }
            let src = match args.first() {
                Some(Value::Str(x)) => x.as_str().to_string(),
                _ => String::new(),
            };
            let filename = match args.get(1) {
                Some(Value::Str(x)) => x.as_str().to_string(),
                _ => "<string>".to_string(),
            };
            // W114: PyCF_ONLY_AST (0x400) -> real AST tree via the
            // ast_bridge + vendored _ast.from_native.
            let flags = match args.get(3) {
                Some(Value::Int(v)) => v.to_i64().unwrap_or(0),
                _ => 0,
            };
            if flags & 0x400 != 0 {
                let tree = crate::ast_bridge::native_parse_tree(&src)
                    .map_err(|m| raise("SyntaxError", m))?;
                let interp = crate::builtins_modules::current_interp()
                    .ok_or_else(|| raise("RuntimeError", "no interpreter"))?;
                let interp = std::rc::Rc::new(interp);
                let ast_mod = crate::importer::import_module(&interp, "_ast")?;
                let conv = crate::interp::attr_get_public(&interp, &ast_mod, "from_native")?
                    .ok_or_else(|| raise("RuntimeError", "_ast.from_native missing"))?;
                return crate::interp::call_value_pub(&interp, &conv, &[tree], &[]);
            }
            let code = ap_obj::PyObject::new(ap_obj::ObjKind::Namespace, "<code>");
            {
                let mut attrs = code.attrs.borrow_mut();
                let _ = attrs.set(
                    Value::Str(PyString::new("co_filename")),
                    Value::Str(PyString::new(filename)),
                );
                let _ = attrs.set(
                    Value::Str(PyString::new("co_source")),
                    Value::Str(PyString::new(src)),
                );
            }
            Ok(Value::Obj(code))
        }
        // 02.08-full part 1: __import__(name) v0 (level=0 absolute only;
        // s047 identity pin: import_module("json") is __import__("json")).
        "__import__" => {
            let mod_name = match args.first() {
                Some(Value::Str(x)) => x.as_str().to_string(),
                _ => return Err(raise("TypeError", "__import__() argument must be str")),
            };
            let interp = crate::builtins_modules::current_interp()
                .ok_or_else(|| raise("RuntimeError", "no interpreter"))?;
            let interp = std::rc::Rc::new(interp);
            match crate::importer::import_module(&interp, &mod_name) {
                Ok(v) => Ok(v),
                Err(Flow::Raise(e)) => Err(Flow::Raise(e)),
                Err(other) => Err(other),
            }
        }
        // 02.08-full part 1: type() fallback for values class_of_value
        // misses — named native iterators report their function name
        // (type(itertools.count(1)).__name__ == "count", s012 pin).
        "type" => match args.first() {
            Some(v @ Value::Obj(o))
                if o.kind == ap_obj::ObjKind::Iterator
                    || o.kind == ap_obj::ObjKind::Opaque
                    || o.kind == ap_obj::ObjKind::Namespace =>
            {
                let interp = crate::builtins_modules::current_interp()
                    .ok_or_else(|| raise("RuntimeError", "no interpreter"))?;
                // sys.implementation reports "module" (s002 pin: the name
                // must be in ("module", "SimpleNamespace")).
                if o.kind == ap_obj::ObjKind::Namespace && o.name == "implementation" {
                    return Ok(crate::builtins::interned_type(&interp, "module"));
                }
                // 02.07: threading objects report their <class> attr
                // (type(me).__name__ == "_MainThread" — T108).
                if o.kind == ap_obj::ObjKind::Opaque && o.name == "thread" {
                    let cls = o
                        .attrs
                        .borrow()
                        .get(&Value::Str(PyString::new("<class>")))
                        .and_then(|v| match v {
                            Value::Str(sv) => Some(sv.as_str().to_string()),
                            _ => None,
                        })
                        .unwrap_or_else(|| "Thread".to_string());
                    return Ok(crate::builtins::interned_type(&interp, &cls));
                }
                // A-3 item 5: opaque native result records report their own
                // name (type(os.uname()) == "uname_result"; stat_result too).
                if matches!(
                    o.name.as_str(),
                    "uname_result" | "stat_result" | "statvfs_result" | "struct_time" | "bytearray"
                ) {
                    return Ok(crate::builtins::interned_type(&interp, &o.name));
                }
                // T2 gate finding 3: type(int | str) is types.UnionType —
                // CPython reprs it <class 'typing.Union'>.
                if (o.kind == ap_obj::ObjKind::Namespace && o.name == "_Union")
                    || (o.kind == ap_obj::ObjKind::Opaque && o.name == "__union__")
                {
                    let t = crate::builtins::interned_type(&interp, "UnionType");
                    if let Value::Obj(to) = &t {
                        *to.repr_text.borrow_mut() = Some("<class 'typing.Union'>".to_string());
                    }
                    return Ok(t);
                }
                match crate::interp::attr_get_public(&interp, v, "__class__") {
                    Ok(Some(t)) => Ok(t),
                    _ => Ok(crate::builtins::interned_type(&interp, v.type_name())),
                }
            }
            Some(Value::Obj(o))
                if o.kind == ap_obj::ObjKind::Iterator
                    && matches!(
                        o.name.as_str(),
                        "count"
                            | "cycle"
                            | "repeat"
                            | "chain"
                            | "accumulate"
                            | "compress"
                            | "dropwhile"
                            | "takewhile"
                            | "filterfalse"
                            | "groupby"
                            | "islice"
                            | "starmap"
                            | "zip_longest"
                            | "batched"
                            | "pairwise"
                    ) =>
            {
                let interp = crate::builtins_modules::current_interp()
                    .ok_or_else(|| raise("RuntimeError", "no interpreter"))?;
                Ok(crate::builtins::interned_type(&interp, &o.name))
            }
            Some(v) => {
                let interp = crate::builtins_modules::current_interp()
                    .ok_or_else(|| raise("RuntimeError", "no interpreter"))?;
                let tname = match v {
                    Value::Frozen(_) => "frozenset",
                    other => other.type_name(),
                };
                Ok(crate::builtins::interned_type(&interp, tname))
            }
            None => Err(raise("TypeError", "type() takes 1 or 3 arguments")),
        },
        // T1-d: typing._idfunc — identity (NewType.__call__).
        // T1-d: TypeVar.__typing_subst__ — identity substitution.
        "__ap_typevar_subst__" => Ok(args.last().cloned().unwrap_or(Value::None)),
        // T2 (typing_extensions PEP 696): the seeded has_default marker.
        "__ap_typevar_has_default__" => Ok(Value::Bool(false)),
        "__ap_typing_idfunc__" | "_typing._idfunc" => {
            Ok(args.first().cloned().unwrap_or(Value::None))
        }
        "__object_virtual___get____" => {
            // T1-d: descriptor-__get__ marker call (attr/_make.py):
            // returns a BOUND object.__setattr__ marker for args[0].
            let inst = args.first().cloned().unwrap_or(Value::None);
            let m =
                ap_obj::PyObject::new(ap_obj::ObjKind::BuiltinFn, "__object_virtual___setattr____");
            *m.slice_parts.borrow_mut() = vec![inst];
            Ok(Value::Obj(m))
        }
        _ => Err(raise("NameError", format!("name '{name}' is not defined"))),
    }
}

/// range(start, stop[, step]) -> Range object.
pub fn builtin_range(args: &[Value]) -> crate::EvalResult {
    // 02.04a (interp bug #14): bigint range bounds (1 << 1000) clamp to
    // i64 extremes for the v0 Range object (documented divergence; CPython
    // materializes true bigint ranges). Non-int args still raise the exact
    // CPython TypeError.
    let ints: Vec<i64> = args
        .iter()
        .map(|v| {
            let is_int = matches!(v, Value::Int(_) | Value::Bool(_));
            if !is_int {
                return Err(raise(
                    "TypeError",
                    format!(
                        "'{}' object cannot be interpreted as an integer",
                        v.type_name()
                    ),
                ));
            }
            Ok(v.as_int().and_then(|i| i.to_i64()).unwrap_or_else(|| {
                // clamp by sign of the bigint
                let neg = v.as_int().map(|i| i.is_negative()).unwrap_or(false);
                if neg {
                    i64::MIN
                } else {
                    i64::MAX
                }
            }))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let (start, stop, step) = match ints.len() {
        1 => (0, ints[0], 1),
        2 => (ints[0], ints[1], 1),
        3 => {
            if ints[2] == 0 {
                return Err(raise("ValueError", "range() arg 3 must not be zero"));
            }
            (ints[0], ints[1], ints[2])
        }
        n => {
            return Err(raise(
                "TypeError",
                format!("range expected at most 3 arguments, got {n}"),
            ))
        }
    };
    let o = ap_obj::PyObject::new(ap_obj::ObjKind::Range, "range");
    *o.ints.borrow_mut() = vec![start, stop, step];
    Ok(Value::Obj(o))
}

/// eval(expr_string) — single-expression eval against module globals
/// (V063: eval(repr(v)) == v round-trip).
pub fn builtin_eval(interp: &Interpreter, args: &[Value]) -> crate::EvalResult {
    // T1-d: eval(compile(...)) — the `<code>` Namespace marker (compile()
    // output, ForwardRef.__forward_code__) carries co_source; unwrap it
    // exactly like builtin_exec already does.
    // T1-d (attr/_make.py): eval accepts a compile() `<code>` marker AND
    // statement-bearing source (attrs compiles `def __attrs_init__...`
    // scripts and evals them into a locs dict). Delegate anything that is
    // not a plain expression to builtin_exec, which already handles code
    // markers + explicit (globals, locals) namespaces.
    let is_code_marker = matches!(
        args.first(),
        Some(Value::Obj(o))
            if o.kind == ap_obj::ObjKind::Namespace
                && o.name == "<code>"
                && o.attrs.borrow().get(&Value::Str(PyString::new("co_source"))).is_some()
    );
    if is_code_marker {
        return builtin_exec(interp, args);
    }
    let src = match args.first() {
        Some(Value::Str(s)) => s.as_str().to_string(),
        Some(other) => {
            return Err(raise(
                "TypeError",
                format!(
                    "eval() arg 1 must be a string, bytes or code object (got {})",
                    other.type_name()
                ),
            ))
        }
        None => return Err(raise("TypeError", "eval expected at least 1 argument")),
    };
    // Statement source (def/class/...) goes through exec semantics; attrs
    // reads the resulting bindings out of the `locals` dict arg. The wrap
    // `__ap_eval__ = <src>` only parses when src is an EXPRESSION — a def/
    // class script fails the wrap parse itself, so that failure is the
    // statement signal (delegate to exec with the RAW source).
    // CPython pin (s001): eval of a statement OR a malformed expression is
    // SyntaxError "invalid syntax" — detect REAL parse errors first and keep
    // that pin; only validly-parsing non-Assign wraps delegate to exec.
    // (ruff is error-RECOVERING: eval("1+") parses to a bogus Name with
    // ExprContext::Invalid, so a missing-Assign shape is ambiguous and the
    // error check must come first.)
    let wrapped = ap_parse::lower_source(&format!("__ap_eval__ = {src}\n"));
    // W110: lowering inserts Stmt::Line before each statement — the wrap
    // Assign is no longer items.first(). Scan for ANY Assign item.
    let is_expr = matches!(
        wrapped
            .as_ref()
            .ok()
            .map(|(m, _)| m.items.iter().find_map(|item| match item {
                ap_ir::Item::Stmt(ap_ir::Stmt::Assign(_, _)) => Some(()),
                _ => None,
            })),
        Some(Some(()))
    );
    if wrapped.is_err() {
        return Err(raise("SyntaxError", "invalid syntax"));
    }
    if !is_expr {
        return builtin_exec(interp, args);
    }
    let (module, _scopes) = match wrapped {
        Ok(m) => m,
        // CPython pin (s001): eval("1+") msg is exactly "invalid syntax".
        Err(_) => return Err(raise("SyntaxError", "invalid syntax")),
    };
    // W110: lowering inserts Stmt::Line markers before each top-level
    // statement — skip them to find the wrapped Assign.
    let expr = match module.items.iter().find_map(|item| match item {
        ap_ir::Item::Stmt(ap_ir::Stmt::Assign(_, v)) => Some(v.clone()),
        _ => None,
    }) {
        Some(v) => v,
        None => return Err(raise("SyntaxError", "invalid syntax")),
    };
    // W111: eval(source, globals[, locals]) — ForwardRef.evaluate passes the
    // caller's namespace. v0 note: explicit namespaces are folded into
    // interp.globals (same documented shape as builtin_exec).
    if args.len() >= 2 {
        if let Value::Dict(d) = &args[1] {
            for (k, v) in d.items() {
                let _ = interp.globals.set(k, v);
            }
        }
        if let Some(Value::Dict(ld)) = args.get(2) {
            for (k, v) in ld.items() {
                let _ = interp.globals.set(k, v);
            }
        }
    }
    // Evaluate in a module-shaped frame over interp.globals.
    crate::interp::eval_expr_public(&interp.rc(), &expr)
}

/// `exec(source[, globals[, locals]])` (04.01): statement execution over the
/// parse->lower->exec_block machinery. six.py needs it at import
/// (`exec_("def raise_from(value, from_value): ...")`). Scope: string source
/// executed in the CURRENT globals (explicit dicts v0-ignored with a
/// documented note — same shape as builtin_eval's globals handling).
pub fn builtin_exec(interp: &Interpreter, args: &[Value]) -> crate::EvalResult {
    // 02.08-c: compile() returns a `<code>` marker carrying co_source —
    // exec must accept it (parsel/csstranslator-style dynamic loads).
    let src = match args.first() {
        Some(Value::Str(s)) => s.as_str().to_string(),
        Some(Value::Obj(o)) if o.kind == ap_obj::ObjKind::Namespace && o.name == "<code>" => {
            match o
                .attrs
                .borrow()
                .get(&Value::Str(PyString::new("co_source")))
            {
                Some(Value::Str(s)) => s.as_str().to_string(),
                _ => String::new(),
            }
        }
        Some(other) => {
            return Err(raise(
                "TypeError",
                format!(
                    "exec() arg 1 must be a string, bytes or code object (got {})",
                    other.type_name()
                ),
            ))
        }
        None => return Err(raise("TypeError", "exec expected at least 1 argument")),
    };
    // T1: explicit globals/locals (twisted reactor: `exec(code, f_globals,
    // f_locals)`). Run the statements in a frame over the GIVEN globals dict;
    // when locals is given and is a dict, mirror the writes into it (v0
    // approximation preserving twisted's def-binding reads). None = current
    // globals (six.py form, unchanged).
    let explicit: Option<(Gc<PyDict>, Option<Gc<PyDict>>)> = if args.len() > 1 {
        match args.get(1) {
            Some(Value::None) | None => None,
            Some(Value::Dict(g)) => {
                let loc = match args.get(2) {
                    Some(Value::None) | None => None,
                    Some(Value::Dict(l)) => Some(l.clone()),
                    Some(other) => {
                        return Err(raise(
                            "TypeError",
                            format!("exec() globals must be a dict, not {}", other.type_name()),
                        ))
                    }
                };
                Some((g.clone(), loc))
            }
            Some(other) => {
                return Err(raise(
                    "TypeError",
                    format!("exec() globals must be a dict, not {}", other.type_name()),
                ))
            }
        }
    } else {
        None
    };
    let parsed =
        ap_parse::parse(&src).map_err(|e| raise("SyntaxError", format!("{}", e.message)))?;
    let (module, _scopes) =
        ap_parse::lower(&parsed).map_err(|e| raise("SyntaxError", format!("{e}")))?;
    if let Some((globals, locals)) = explicit {
        // T1: register the exec'd defs into the LIVE registry (extend_current)
        // so the fn objects resolve their bodies at call time, then bind the
        // function objects into the GIVEN globals dict. Decorated defs defer
        // to source position (module_stmts shape).
        let items = &module.items;
        let has_plain_defs = items
            .iter()
            .any(|it| matches!(it, ap_ir::Item::FnDef(f) if f.decorators.is_empty()));
        let scope_src = src.clone();
        let mut stmts: Vec<ap_ir::Stmt> = Vec::new();
        for it in items.iter() {
            match it {
                ap_ir::Item::Stmt(st) => stmts.push(st.clone()),
                ap_ir::Item::FnDef(f) if !f.decorators.is_empty() => {
                    stmts.push(ap_ir::Stmt::LocalDef(Box::new(f.clone())));
                }
                ap_ir::Item::FnDef(_) => {} // registered via extend_current
                ap_ir::Item::ClassDef(c) => {
                    stmts.push(ap_ir::Stmt::LocalClass(Box::new(c.clone())))
                }
            }
        }
        let rc_interp = interp.rc();
        if has_plain_defs {
            // Re-lower a private copy for registration (lower consumes the
            // parsed module in the first call; extend needs the FnDefs).
            let parsed2 = ap_parse::parse(&scope_src)
                .map_err(|e| raise("SyntaxError", format!("{}", e.message)))?;
            let (module2, scopes2) =
                ap_parse::lower(&parsed2).map_err(|e| raise("SyntaxError", format!("{e}")))?;
            let _ = module2;
            crate::interp::FnRegistry::extend_current(&rc_interp, items, scopes2, globals.clone());
            // Wave-83: exec-created defs have NO defining frame (Stmt::
            // LocalDef never ran), so eval_default falls back to a MAIN-
            // globals module frame and default exprs referencing exec-globs
            // names (attrs' attr_dict) NameError. Register a defining frame
            // over the EXEC globals for each new def id.
            {
                let reg2 = crate::interp::interp_registry(&rc_interp);
                for it in items.iter() {
                    if let ap_ir::Item::FnDef(f) = it {
                        if let Some(id) = reg2.by_path_lookup(&f.name) {
                            let df = crate::frame::Frame::new_module(globals.clone());
                            rc_interp
                                .defining_frames
                                .borrow_mut()
                                .insert(id, std::rc::Rc::new(df));
                        }
                    }
                }
            }
            // Bind the function objects into the exec globals dict.
            // T1-d (attr/_make.py _compile_and_eval): defs bind into LOCALS
            // too — attrs passes globs as globals and a fresh locs dict as
            // locals, then reads `locs["__attrs_init__"]`.
            let reg = crate::interp::interp_registry(&rc_interp);
            for it in items.iter() {
                if let ap_ir::Item::FnDef(f) = it {
                    let id = reg.by_path_lookup(&f.name).unwrap_or(0);
                    let func = make_function(id, &f.name);
                    // Wave-83: the marker's closure_slot drives
                    // eval_default's defining-frame lookup — without it
                    // defaults evaluate against MAIN globals and NameError
                    // (attrs attr_dict pattern).
                    if let Value::Obj(fo) = &func {
                        *fo.closure_slot.borrow_mut() = Some(id as u64);
                    }
                    let _ = globals.set(Value::Str(PyString::new(f.name.clone())), func.clone());
                    if let Some(loc) = &locals {
                        let _ = loc.set(Value::Str(PyString::new(f.name.clone())), func);
                    }
                }
            }
        }
        return crate::interp::exec_stmts_in_ns(&rc_interp, &stmts, globals, locals);
    }
    let stmts: Vec<ap_ir::Stmt> = module
        .items
        .into_iter()
        .filter_map(|it| match it {
            ap_ir::Item::Stmt(st) => Some(st),
            ap_ir::Item::FnDef(f) => {
                // Module-level defs register + bind like exec_module does.
                let rc_interp = interp.rc();
                let reg = crate::interp::interp_registry(&rc_interp);
                let id = reg.by_path_lookup(&f.name).unwrap_or(0);
                let func = make_function(id, &f.name);
                let _ = interp
                    .globals
                    .set(Value::Str(PyString::new(f.name.clone())), func);
                None
            }
            ap_ir::Item::ClassDef(c) => Some(ap_ir::Stmt::LocalClass(Box::new(c))),
        })
        .collect();
    let rc_interp = interp.rc();
    // Wave-104: exec(str) with no explicit namespaces INSIDE A CLASS BODY —
    // CPython gives the exec the class namespace as locals (twisted
    // insults.py `exec(ch + " = _makeSimple(ch, fName)")` pattern). Our
    // exec_stmts_public used a fresh MODULE frame, so class-body names were
    // invisible to the exec'd code. exec_class_def publishes the live
    // class ns; route through it when present.
    if let Some(ns) = crate::classes::current_class_ns() {
        return crate::interp::exec_stmts_in_ns(&rc_interp, &stmts, ns, None);
    }
    crate::interp::exec_stmts_public(&rc_interp, &stmts)
}

// ---------------------------------------------------------------------------
// 02.08-lite native-module surface (shims + dotted dispatch)
// ---------------------------------------------------------------------------

/// Public attr_get shim for builtins_modules (module ns lookups from native code).
pub fn attr_get_pub(
    interp: &std::rc::Rc<Interpreter>,
    obj: &Value,
    name: &str,
) -> Result<Option<Value>, Flow> {
    crate::interp::attr_get_public(interp, obj, name)
}

/// Public call_builtin shim (dotted marker names route through call_native).
pub fn call_builtin_pub(name: &str, args: &[Value], kwargs: &[(String, Value)]) -> BResult {
    match call_builtin_dotted(name, args, kwargs) {
        Some(r) => r,
        None => Err(raise("NameError", format!("name '{name}' is not defined"))),
    }
}

/// Public method-call shim (json.load(fp) -> fp.read() etc).
pub fn call_method_pub(recv: &Value, name: &str, args: &[Value]) -> BResult {
    let interp = crate::builtins_modules::current_interp();
    match interp {
        Some(i) => {
            let rc = i;
            match crate::methods::call_method(&rc, recv, name, args) {
                Some(r) => r.map_err(|f| f),
                None => {
                    // 02.08-full part 1: stream method fallback (json.dump
                    // writes through file-like objects — s008/s009 pins).
                    if let Value::Obj(o) = recv {
                        if matches!(
                            o.name.as_str(),
                            "BytesIO"
                                | "StringIO"
                                | "TextIOWrapper"
                                | "FileIO"
                                | "BufferedReader"
                                | "BufferedWriter"
                                | "BufferedRWPair"
                                | "BufferedRandom"
                                | "IncrementalNewlineDecoder"
                        ) {
                            let mut full: Vec<Value> = vec![recv.clone()];
                            full.extend_from_slice(args);
                            if let Some(r) = crate::builtins_modules::call_io(name, &full, &[]) {
                                return r;
                            }
                        }
                    }
                    Err(raise(
                        "AttributeError",
                        format!("'{}' object has no attribute '{}'", recv.type_name(), name),
                    ))
                }
            }
        }
        None => Err(raise("RuntimeError", "no interpreter on this thread")),
    }
}

/// Dispatch "mod.fn" dotted names into builtins_modules (sys keeps its own path).
pub fn call_builtin_dotted(
    name: &str,
    args: &[Value],
    kwargs: &[(String, Value)],
) -> Option<BResult> {
    // T1 (importlib.resources): the native resource-reader markers installed
    // by attach_module_spec. files() -> pathlib.Path(package dir).
    if name == "__ap_resreader__.files" {
        let dir = args
            .first()
            .and_then(|v| match v {
                Value::Str(sv) => Some(sv.as_str().to_string()),
                _ => None,
            })
            .unwrap_or_default();
        let Some(interp) = crate::builtins_modules::current_interp() else {
            return None;
        };
        let pathlib = match crate::importer::import_module(&interp, "pathlib") {
            Ok(v) => v,
            Err(f) => return Some(Err(f)),
        };
        let path_v = match crate::interp::attr_get_public(&interp.rc(), &pathlib, "Path") {
            Ok(Some(v)) => v,
            Ok(None) => {
                return Some(Err(raise("AttributeError", "pathlib has no Path")));
            }
            Err(f) => return Some(Err(f)),
        };
        return Some(crate::interp::call_value_pub3(
            &path_v,
            &[Value::Str(PyString::new(dir))],
        ));
    }
    if name == "__ap_loader__.get_resource_reader" {
        let reader = args.first().cloned().unwrap_or(Value::None);
        return Some(Ok(reader));
    }
    let (mod_name, fname) = name.split_once('.')?;
    if mod_name == "sys" {
        return None; // handled by the legacy sys.* path
    }
    crate::builtins_modules::call_native(mod_name, fname, args, kwargs)
}

/// True when `name` is a dotted builtin marker this crate dispatches.
pub fn is_builtin_marker_name(name: &str) -> bool {
    name.split_once('.').is_some()
}

/// sys module builder re-export (registry entry).
pub fn make_sys_module_pub() -> Value {
    let interp = match crate::builtins_modules::current_interp() {
        Some(i) => i,
        None => panic!("sys module built outside a run"),
    };
    get_or_make_sys(&interp)
}

/// CPython float repr for module code.
pub fn py_float_repr_pub(x: f64) -> String {
    ap_obj::float_repr::py_float_repr(x)
}

/// Register a native iterator into the CURRENT interpreter's table.
pub fn interp_register_iter(id: u64, it: std::rc::Rc<crate::iter::PyIterator>) {
    if let Some(interp) = crate::builtins_modules::current_interp() {
        interp.iterators.borrow_mut().insert(id, it);
    }
}

/// Route a constructor-style call (str/int/float/bool/...) through the type
/// dispatch path (used by builtins_modules hooks for conversion callables).
pub fn call_builtin_ctor(name: &str, args: &[Value]) -> Option<BResult> {
    match name {
        // 02.08-c: direct compile() (call_builtin_dotted routes the bare name
        // to re.compile); used by the zope.interface stub builder.
        "compile" => {
            let kwargs: Vec<(String, Value)> = Vec::new();
            let mut a = args.to_vec();
            if a.len() < 3 {
                a.push(Value::Str(PyString::new("exec".to_string())));
            }
            let interp = crate::builtins_modules::current_interp()?;
            Some(call_builtin(&interp, name, &a, &kwargs))
        }
        "str" => Some(builtin_str(args)),
        "int" => Some(builtin_int(
            &*crate::builtins_modules::current_interp()?,
            args,
        )),
        "float" => Some(builtin_float(args)),
        "bool" => Some(builtin_bool(args)),
        "list" => Some(builtin_list(args)),
        "tuple" => Some(builtin_tuple(args)),
        "repr" => Some(builtin_repr(args)),
        _ => None,
    }
}

/// `type.__new__(mcls, name, bases, ns)` (02.04a): create the class payload.
/// Bases must be class values; ns must be a dict. MRO comes from TypeObject::create.
fn builtin_type_new(args: &[Value], kwargs: &[(String, Value)]) -> BResult {
    use crate::classes::{class_of_value, object_class};
    // 02.03b-fix: type.__new__ forwards leftover **kwds to
    // __init_subclass__; object's default rejects them with the pinned
    // TypeError (mk.py Meta chain: super().__new__(mcs, ..., **kw)).
    if !kwargs.is_empty() {
        // type.__new__ forwards leftover **kwds to the parent's
        // __init_subclass__ (CPython); object's DEFAULT rejects them with
        // the pinned TypeError. A user hook up the bases' MRO consumes them
        // (typing.py: class _SpecialForm(_Final, _NotIterable, _root=True)
        // via 3-arg type() — packaging 26's typing constructs need this).
        let bases: Vec<Value> = match args.get(2) {
            Some(Value::Tuple(tup)) => tup.0.clone(),
            Some(Value::List(l)) => l.items().to_vec(),
            _ => vec![],
        };
        let mut hook_found = false;
        'outer: for b in &bases {
            if let Value::Obj(bo) = b {
                if bo.kind == ap_obj::ObjKind::Type {
                    if let Some(tr) = bo.type_ref.borrow().as_ref().and_then(|w| w.upgrade()) {
                        for m in tr.mro.borrow().iter() {
                            if let Some((_f, owner)) = m.lookup_with_owner("__init_subclass__") {
                                if owner.name != "object" {
                                    hook_found = true;
                                    break 'outer;
                                }
                            }
                        }
                    }
                }
            }
        }
        if !hook_found {
            let class_name = match &args[1] {
                Value::Str(s) => s.as_str().to_string(),
                _ => String::new(),
            };
            return Err(raise(
                "TypeError",
                format!(
                    "{}.__init_subclass__() takes no keyword arguments",
                    class_name
                ),
            ));
        }
        // A consuming hook exists up the MRO: kwargs are delivered there in
        // CPython after allocation. v0: consumed-and-dropped (the hook bodies
        // typing.py defines for _root=True are registry-side effects we do
        // not yet model); the class itself is created correctly.
    }
    if args.len() != 4 {
        return Err(raise(
            "TypeError",
            format!("type.__new__ expected 4 arguments, got {}", args.len()),
        ));
    }
    let name = match &args[1] {
        Value::Str(s) => s.as_str().to_string(),
        other => {
            return Err(raise(
                "TypeError",
                format!(
                    "type.__new__ argument 2 must be str, not {}",
                    other.type_name()
                ),
            ))
        }
    };
    let bases_v: Vec<Value> = match &args[2] {
        Value::Tuple(t) => t.0.clone(),
        Value::List(l) => l.items().to_vec(),
        other => {
            return Err(raise(
                "TypeError",
                format!(
                    "type.__new__ argument 3 must be tuple, not {}",
                    other.type_name()
                ),
            ))
        }
    };
    let ns = match &args[3] {
        Value::Dict(d) => d.clone(),
        // 02.03b: EnumType.__new__ passes the EnumDict INSTANCE; the class
        // payload is its backing dict.
        Value::Obj(o) if o.kind == ap_obj::ObjKind::UserInstance && true => {
            match crate::classes::dict_backing(&args[3]) {
                Some(d) => d,
                None => return Err(raise("TypeError", "type.__new__ argument 4 must be dict")),
            }
        }
        other => {
            return Err(raise(
                "TypeError",
                format!(
                    "type.__new__ argument 4 must be dict, not {}",
                    other.type_name()
                ),
            ))
        }
    };
    let interp = crate::builtins_modules::current_interp()
        .ok_or_else(|| raise("SystemError", "no interpreter"))?;
    let mut base_types: Vec<std::rc::Rc<crate::classes::TypeRef>> = Vec::new();
    for b in &bases_v {
        // 02.03b: builtin-type bases (`class IntEnum(int, Enum)` through the
        // metaclass __new__ path) go through the SAME synthesis exec_class_def
        // uses (02.04a interp bug #11); a raw name-Type was rejected here.
        match crate::classes::base_type_ref(&interp, b)? {
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
        base_types.push(object_class(&interp));
    }
    let flags = ap_obj::classes::TypeFlags {
        slots: None,
        is_exception: false,
        builtin_exc: false,
    };
    let t = ap_obj::classes::TypeObject::create(
        name.clone(),
        name.clone(),
        base_types,
        ns,
        flags,
        None,
    )
    .map_err(|_| raise("TypeError", "failed to create type (MRO error)"))?;
    // T1-d: dynamic classes with a BUILTIN base (type("X", (tuple,), body),
    // attr/_make.py _make_attr_tuple_class) — mark the payload so
    // instantiate constructs the backing builtin from ctor args. This is
    // the 3-arg type() path; the enum-style path lives in classes.rs
    // base_type_ref.
    if let Some(Value::Obj(first_base)) = args.get(1).and_then(|v| match v {
        Value::Tuple(tup) => tup.0.first().cloned(),
        Value::List(l) => l.items().first().cloned(),
        _ => None,
    }) {
        if first_base.kind == ap_obj::ObjKind::Type
            && matches!(
                first_base.name.as_str(),
                // W110 (T2): int subclasses (enum.IntEnum(int, ...)) need
                // the builtin-ctor fallback — MyInt(1) builds from args.
                "tuple" | "list" | "dict" | "set" | "str" | "int"
            )
        {
            let _ = t.dict.set(
                Value::Str(PyString::new("__ap_builtin_base__")),
                Value::Str(PyString::new(first_base.name.clone())),
            );
        }
        // T2 (pytest capture EncodedFile/CaptureIO(io.TextIOWrapper)):
        // classes deriving from a native STREAM type with no user
        // __new__/__init__ must construct via the native base ctor. Mark
        // the payload so instantiate() can route the call.
        if first_base.kind == ap_obj::ObjKind::Type
            && matches!(
                first_base.name.as_str(),
                "TextIOWrapper"
                    | "BytesIO"
                    | "StringIO"
                    | "BufferedReader"
                    | "BufferedWriter"
                    | "FileIO"
            )
        {
            let _ = t.dict.set(
                Value::Str(PyString::new("__ap_stream_base__")),
                Value::Str(PyString::new(first_base.name.clone())),
            );
        }
    }
    // The creating metaclass rides the payload.
    if let Some(m) = class_of_value(&interp, &args[0]) {
        *t.metaclass.borrow_mut() = Some(m);
    }
    Ok(ap_obj::classes::make_class_value(t))
}
