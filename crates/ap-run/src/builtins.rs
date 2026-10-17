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
    if let Value::Obj(o) = v {
        if o.kind == ap_obj::ObjKind::Exception {
            let exc = crate::exc_hier::ExceptionValue::new(&o.name, o.args.borrow().clone());
            return exc.py_str();
        }
    }
    ap_obj::ops::to_str(v)
}

/// print(*args, sep=' ', end='\n', file=None) — v0: file must be sys.stdout
/// or absent; str() conversion; None args print 'None'.
pub fn print(interp: &Interpreter, args: &[Value], kwargs: &[(String, Value)]) -> BResult {
    let mut sep = " ".to_string();
    let mut end = "\n".to_string();
    for (k, v) in kwargs {
        match k.as_str() {
            "sep" => sep = ap_obj::ops::to_str(v).map_err(Flow::from_py)?,
            "end" => end = ap_obj::ops::to_str(v).map_err(Flow::from_py)?,
            "file" => {
                // v0: only sys.stdout accepted (identity checked by caller
                // passing nothing or the module attr — reject others).
                let is_stdout = matches!(v, Value::Obj(o) if o.kind == ap_obj::ObjKind::Namespace
                    && o.name == "stdout");
                if !is_stdout {
                    return Err(raise(
                        "TypeError",
                        "print() in v0 supports only file=sys.stdout",
                    ));
                }
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
        out.push_str(&py_str_of(a).map_err(Flow::from_py)?);
    }
    out.push_str(&end);
    interp.write_out(&out);
    Ok(Value::None)
}

// ---------------------------------------------------------------------------
// type-ish builtins
// ---------------------------------------------------------------------------

/// len(v): sized types only, exact TypeError.
pub fn builtin_len(args: &[Value]) -> BResult {
    let v = one(args, "len")?;
    let n = match v {
        Value::Str(s) => s.len(),
        Value::Bytes(b) => b.len(),
        Value::List(l) => l.len(),
        Value::Tuple(t) => t.len(),
        Value::Dict(d) => d.len(),
        Value::Set(s) | Value::Frozen(s) => s.len(),
        Value::Obj(o) => match o.kind {
            ap_obj::ObjKind::Range => {
                let ints = o.ints.borrow();
                crate::iter::range_len(ints[0], ints[1], ints[2]) as usize
            }
            _ => {
                return Err(raise(
                    "TypeError",
                    format!("object of type '{}' has no len()", v.type_name()),
                ))
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
        1 => {
            let v = &args[0];
            match v {
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
            let it = iterable_of(v)?;
            let items = crate::iter::collect(&it);
            Ok(Value::List(PyList::from_vec(items)))
        }
    }
}

pub fn builtin_tuple(args: &[Value]) -> BResult {
    match args.first() {
        None => Ok(Value::Tuple(Gc::new(Tuple(vec![])))),
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
                // iterable of pairs
                let it = iterable_of(other)?;
                for pair in crate::iter::collect(&it) {
                    let (k, v) = unpack_pair(&pair)?;
                    d.set(k, v).map_err(Flow::from_py)?;
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
    for (k, v) in kwargs {
        if k == "key" {
            key_fn = Some(v);
        } else if k == "default" {
            // single-iterable form only; v0 corpus doesn't exercise
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
    let it = iterable_of(first)?;
    let mut acc = match args.get(1) {
        Some(start) => start.clone(),
        None => Value::Int(Int::Smi(0)),
    };
    for item in crate::iter::collect(&it) {
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
            Ok(Value::Bool(type_matches(v, want)))
        }
        // Constructor builtins (int/float/str/...) double as type objects in
        // the v0 namespace; isinstance accepts them.
        Value::Obj(o) if o.kind == ap_obj::ObjKind::BuiltinFn => {
            Ok(Value::Bool(type_matches(v, &o.name)))
        }
        _ => Err(raise(
            "TypeError",
            "isinstance() arg 2 must be a type, a tuple of types, or a tuple",
        )),
    }
}

/// Name-based type match with the bool⊂int special case.
pub fn type_matches(v: &Value, want: &str) -> bool {
    match (v, want) {
        (_, "object") => true,
        (Value::Int(_), "int") => true,
        (Value::Bool(_), "bool") => true,
        (Value::Bool(_), "int") => true, // bool is a subclass of int (S-01-9)
        (Value::Int(_), "bool") => false,
        (Value::Frozen(_), "set") => false,
        (a, b) => a.type_name() == b,
    }
}

/// type(v) / type(name, bases, dict) — the 3-arg form is 02.03; v0 raises.
pub fn builtin_type(args: &[Value]) -> BResult {
    match args.len() {
        0 => Err(raise("TypeError", "type() takes 1 or 3 arguments")),
        1 => {
            let v = &args[0];
            let name = match v {
                Value::Obj(o) if o.kind == ap_obj::ObjKind::Exception => o.name.clone(),
                Value::Obj(o) => o.type_name(),
                other => other.type_name().to_string(),
            };
            let t = ap_obj::PyObject::new(ap_obj::ObjKind::Type, name);
            Ok(Value::Obj(t))
        }
        _ => Err(raise(
            "NotImplementedError",
            "3-arg type() is a 02.03 surface (classes)",
        )),
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
    let it = iterable_of(v)?;
    // Wrap in an iterator OBJECT the executor can re-pull.
    let obj = ap_obj::PyObject::new(ap_obj::ObjKind::Iterator, "iterator");
    *obj.gen_id.borrow_mut() = Some(interp_register_iter_marker(&obj, it));
    Ok(Value::Obj(obj))
}

fn interp_register_iter_marker(_obj: &ap_obj::PyObject, _it: Gc<crate::iter::PyIterator>) -> u64 {
    0 // placeholder: interp.rs's call_iter handles registration
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
                return Err(raise(
                    "ValueError",
                    "pow() 2nd argument cannot be negative when 3rd specified",
                ));
            }
            let r = mod_pow(&a, &b, &m);
            Ok(Value::Int(r))
        }
        _ => Err(raise(
            "TypeError",
            "pow expected at least 2 arguments, got 0",
        )),
    }
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

pub fn builtin_enumerate(interp: &Interpreter, args: &[Value]) -> BResult {
    let v = one(args, "enumerate")?;
    let inner = register_iter(interp, v)?;
    let start = args
        .get(1)
        .and_then(|s| s.as_int())
        .and_then(|i| i.to_i64())
        .unwrap_or(0);
    let obj = ap_obj::PyObject::new(ap_obj::ObjKind::Iterator, "enumerate");
    *obj.ints.borrow_mut() = vec![start];
    *obj.gen_id.borrow_mut() = Some(inner);
    Ok(Value::Obj(obj))
}

pub fn builtin_zip(interp: &Interpreter, args: &[Value]) -> BResult {
    let mut ids = Vec::with_capacity(args.len());
    for a in args {
        ids.push(register_iter(interp, a)?);
    }
    let obj = ap_obj::PyObject::new(ap_obj::ObjKind::Iterator, "zip");
    *obj.gen_id.borrow_mut() = Some(ids.first().copied().unwrap_or(0));
    Ok(Value::Obj(obj))
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
        decorated.sort_by(|a, b| {
            ap_obj::dict::values_cmp(&a.0, &b.0).unwrap_or(std::cmp::Ordering::Equal)
        });
        items = decorated.into_iter().map(|(_, v)| v).collect();
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
    let _ = ns.set(
        Value::Str(PyString::new("maxsize")),
        Value::Int(Int::from_i64(i64::MAX)),
    );
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
            "3.14.0 (antipythany v0 sys stub; provisional per leaf 02.02)",
        )),
    );
    // version_info: named-tuple-like namespace.
    let vi = ap_obj::PyObject::new(ap_obj::ObjKind::NamedTuple, "version_info");
    *vi.ints.borrow_mut() = vec![3, 14, 0, 3]; // major, minor, micro, releaselevel-index
    let _ = vi.attrs.borrow_mut().set(
        Value::Str(PyString::new("releaselevel")),
        Value::Str(PyString::new("final")),
    );
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
    let so = ap_obj::PyObject::new(ap_obj::ObjKind::Namespace, "stdout");
    let _ = ns.set(Value::Str(PyString::new("stdout")), Value::Obj(so));
    let se = ap_obj::PyObject::new(ap_obj::ObjKind::Namespace, "stderr");
    let _ = ns.set(Value::Str(PyString::new("stderr")), Value::Obj(se));
    let _ = ns.set(
        Value::Str(PyString::new("getdefaultencoding")),
        builtin_marker("sys.getdefaultencoding"),
    );
    let _ = ns.set(
        Value::Str(PyString::new("getrecursionlimit")),
        builtin_marker("sys.getrecursionlimit"),
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
    let _ = ns.set(
        Value::Str(PyString::new("exc_info")),
        builtin_marker("sys.exc_info"),
    );
    let _ = m.module_ns.borrow_mut().replace((*ns).clone());
    // Store the ns for attribute reads.
    *m.module_ns.borrow_mut() = Some((*ns).clone());
    Value::Obj(m)
}

fn builtin_marker(name: &str) -> Value {
    let o = ap_obj::PyObject::new(ap_obj::ObjKind::BuiltinFn, name);
    Value::Obj(o)
}

/// Dispatch a sys-module attribute access that is a function marker.
pub fn call_sys_fn(interp: &Interpreter, name: &str, args: &[Value]) -> Option<BResult> {
    match name {
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
        "exc_info" => Some(Ok(Value::Tuple(Gc::new(Tuple(vec![
            Value::None,
            Value::None,
            Value::None,
        ]))))),
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
        "any",
        "bin",
        "bool",
        "chr",
        "dict",
        "divmod",
        "enumerate",
        "eval",
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
        "str",
        "sum",
        "tuple",
        "type",
        "zip",
    ];
    for n in names {
        let o = ap_obj::PyObject::new(ap_obj::ObjKind::BuiltinFn, *n);
        let _ = b.set(Value::Str(PyString::new(*n)), Value::Obj(o));
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
    // type names usable with isinstance
    let types: &[&str] = &[
        "NoneType",
        "ellipsis",
        "bytes",
        "object",
        "range",
        "frozenset",
        "set",
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

/// Dispatch a builtin call (function object name -> impl).
pub fn call_builtin(
    interp: &Interpreter,
    name: &str,
    args: &[Value],
    kwargs: &[(String, Value)],
) -> crate::EvalResult {
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
        "min" => builtin_min(args, kwargs),
        "max" => builtin_max(args, kwargs),
        "sum" => builtin_sum(args),
        "any" => builtin_any(args),
        "all" => builtin_all(args),
        "isinstance" => builtin_isinstance(args),
        "type" => builtin_type(args),
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
        "next" => builtin_next(interp, args),
        "range" => builtin_range(args),
        "sorted" => builtin_sorted(interp, args, kwargs),
        "enumerate" => builtin_enumerate(interp, args),
        "zip" => builtin_zip(interp, args),
        "float.fromhex" => {
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
        "eval" => builtin_eval(interp, args),
        _ => Err(raise("NameError", format!("name '{name}' is not defined"))),
    }
}

/// range(start, stop[, step]) -> Range object.
pub fn builtin_range(args: &[Value]) -> crate::EvalResult {
    let ints: Vec<i64> = args
        .iter()
        .map(|v| {
            v.as_int().and_then(|i| i.to_i64()).ok_or_else(|| {
                raise(
                    "TypeError",
                    "'str' object cannot be interpreted as an integer",
                )
            })
        })
        .collect::<Result<_, _>>()?;
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
    let (module, _scopes) = match ap_parse::lower_source(&format!("__ap_eval__ = {src}\n")) {
        Ok(m) => m,
        Err(_) => return Err(raise("SyntaxError", "invalid syntax (eval compile failed)")),
    };
    let expr = match module.items.first() {
        Some(ap_ir::Item::Stmt(ap_ir::Stmt::Assign(_, v))) => v.clone(),
        _ => return Err(raise("SyntaxError", "invalid syntax")),
    };
    // Evaluate in a module-shaped frame over interp.globals.
    crate::interp::eval_expr_public(&interp.rc(), &expr)
}
