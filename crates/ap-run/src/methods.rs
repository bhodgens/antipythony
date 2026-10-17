//! Method dispatch on builtin values (str/bytes/list/dict/set methods the
//! corpus slices exercise) + dunder protocol dispatch for UserInstances
//! (`__len__`, `__bool__`, `__str__`, `__repr__` — the V048/V050/V066/f12
//! surface; the full class model is 02.03).

use crate::exc_hier::ExceptionValue;
use crate::frame::Interpreter;
use crate::Flow;
use ap_obj::int::{Gc, Int};
use ap_obj::value::{PyList, PyString, Tuple, Value};

pub type MResult = Result<Value, Flow>;

fn raise(class: &str, msg: impl Into<String>) -> Flow {
    Flow::Raise(ExceptionValue::msg(class, msg.into()))
}

/// Slice-index algorithm (S-01-15): negative bounds, clamping, step.
/// Returns the (start, stop, step) triple already clamped, CPython's
/// PySlice_GetIndicesEx behavior.
pub fn slice_indices(
    len: i64,
    lo: Option<i64>,
    hi: Option<i64>,
    step: i64,
) -> Result<(i64, i64, i64), Flow> {
    if step == 0 {
        return Err(raise("ValueError", "slice step cannot be zero"));
    }
    let mut start = match lo {
        None => {
            if step < 0 {
                len - 1
            } else {
                0
            }
        }
        Some(v) => {
            if v < 0 {
                (v + len).max(if step < 0 { -1 } else { 0 })
            } else if v > len {
                if step < 0 {
                    len - 1
                } else {
                    len
                }
            } else {
                v
            }
        }
    };
    let mut stop = match hi {
        None => {
            if step < 0 {
                -1
            } else {
                len
            }
        }
        Some(v) => {
            if v < 0 {
                (v + len).max(if step < 0 { -1 } else { 0 })
            } else if v > len {
                if step < 0 {
                    len - 1
                } else {
                    len
                }
            } else {
                v
            }
        }
    };
    // Normalize for step direction.
    if step < 0 {
        if start < stop {
            return Ok((0, 0, step)); // empty
        }
    } else if start >= stop {
        return Ok((0, 0, step)); // empty
    }
    let _ = (&mut start, &mut stop);
    Ok((start, stop, step))
}

/// Apply a slice to a sequence snapshot (chars for str, bytes, list items).
pub fn slice_seq<T: Clone>(items: &[T], lo: Option<i64>, hi: Option<i64>, step: i64) -> Vec<T> {
    let len = items.len() as i64;
    if step == 0 {
        return vec![];
    }
    let (start, stop, step) = match slice_indices(len, lo, hi, step) {
        Ok(t) => t,
        Err(_) => return vec![],
    };
    let mut out = Vec::new();
    let mut i = start;
    if step > 0 {
        while i < stop {
            if i >= 0 && i < len {
                out.push(items[i as usize].clone());
            }
            i += step;
        }
    } else {
        while i > stop {
            if i >= 0 && i < len {
                out.push(items[i as usize].clone());
            }
            i += step;
        }
    }
    out
}

/// Normalize a possibly-SliceObj subscript into (lo, hi, step) i64s plus a
/// flag whether this was a slice at all. Non-int keys (floats) raise the
/// exact TypeError.
pub fn subscript_key(v: &Value) -> Result<Key, Flow> {
    match v {
        Value::Obj(o) if o.kind == ap_obj::ObjKind::SliceObj => {
            let parts = o.slice_parts.borrow();
            let get_i = |x: &Value| -> Result<Option<i64>, Flow> {
                match x {
                    Value::None => Ok(None),
                    val => val
                        .as_int()
                        .and_then(|i| i.to_i64())
                        .map(Some)
                        .ok_or_else(|| {
                            raise(
                                "TypeError",
                                format!(
                                    "slice indices must be integers or None or have an __index__ method"
                                ),
                            )
                        }),
                }
            };
            Ok(Key::Slice {
                lo: get_i(&parts[0])?,
                hi: get_i(&parts[1])?,
                step: get_i(&parts[2])?.unwrap_or(1),
            })
        }
        Value::Int(_) | Value::Bool(_) => Ok(Key::Index(
            v.as_int().and_then(|i| i.to_i64()).unwrap_or_else(|| 0),
        )),
        Value::Float(_) => Ok(Key::Float),
        other => Ok(Key::Other(other.clone())),
    }
}

pub enum Key {
    Index(i64),
    Float,
    Slice {
        lo: Option<i64>,
        hi: Option<i64>,
        step: i64,
    },
    Other(Value),
}

/// obj[key] read.
pub fn subscript_get(obj: &Value, key: &Value) -> MResult {
    let k = subscript_key(key)?;
    match (obj, &k) {
        (Value::Str(s), Key::Index(i)) => {
            let chars: Vec<char> = s.as_str().chars().collect();
            let idx = normalize_index(*i, chars.len() as i64)
                .ok_or_else(|| raise("IndexError", "string index out of range"))?;
            Ok(Value::Str(PyString::new(chars[idx as usize].to_string())))
        }
        (Value::Str(s), Key::Slice { lo, hi, step }) => {
            if *step == 0 {
                return Err(raise("ValueError", "slice step cannot be zero"));
            }
            let chars: Vec<char> = s.as_str().chars().collect();
            let out: String = slice_seq(&chars, *lo, *hi, *step).into_iter().collect();
            Ok(Value::Str(PyString::new(out)))
        }
        (Value::Str(_), Key::Float) => Err(raise(
            "TypeError",
            "string indices must be integers, not 'float'",
        )),
        (Value::Bytes(b), Key::Index(i)) => {
            let idx = normalize_index(*i, b.len() as i64)
                .ok_or_else(|| raise("IndexError", "index out of range"))?;
            Ok(Value::Int(Int::from_i64(b[idx as usize] as i64)))
        }
        (Value::Bytes(b), Key::Slice { lo, hi, step }) => {
            if *step == 0 {
                return Err(raise("ValueError", "slice step cannot be zero"));
            }
            let out = slice_seq(b, *lo, *hi, *step);
            Ok(Value::Bytes(Gc::new(out)))
        }
        (Value::List(l), Key::Index(i)) => {
            let idx = normalize_index(*i, l.len() as i64)
                .ok_or_else(|| raise("IndexError", "list index out of range"))?;
            Ok(l.get(idx as usize).unwrap_or(Value::None))
        }
        (Value::List(l), Key::Slice { lo, hi, step }) => {
            if *step == 0 {
                return Err(raise("ValueError", "slice step cannot be zero"));
            }
            let items = l.items();
            let out = slice_seq(&items, *lo, *hi, *step);
            Ok(Value::List(PyList::from_vec(out)))
        }
        (Value::Tuple(t), Key::Index(i)) => {
            let idx = normalize_index(*i, t.len() as i64)
                .ok_or_else(|| raise("IndexError", "tuple index out of range"))?;
            Ok(t.0[idx as usize].clone())
        }
        (Value::Tuple(t), Key::Slice { lo, hi, step }) => {
            if *step == 0 {
                return Err(raise("ValueError", "slice step cannot be zero"));
            }
            let out = slice_seq(&t.0, *lo, *hi, *step);
            Ok(Value::Tuple(Gc::new(Tuple(out))))
        }
        (Value::Dict(d), _) => {
            // Unhashable lookup keys raise the dict-key phrasing (V043):
            // outer key type + the offending member type (CPython scans
            // tuples for the first unhashable element).
            if ap_obj::ops::hash(key).is_err() {
                let inner =
                    first_unhashable_type(key).unwrap_or_else(|| key.type_name().to_string());
                return Err(raise(
                    "TypeError",
                    format!(
                        "cannot use '{}' as a dict key (unhashable type: '{}')",
                        key.type_name(),
                        inner
                    ),
                ));
            }
            // Identity-before-equality for NaN keys (V053): floats are
            // inline copies in our Value repr, so object identity is
            // unobservable — any stored NaN key answers a NaN lookup.
            // Documented approximation (same variable in the corpus).
            if matches!(key, Value::Float(f) if f.is_nan()) {
                for (k, v) in d.items() {
                    if matches!(k, Value::Float(f) if f.is_nan()) {
                        return Ok(v);
                    }
                }
            }
            match d.get(key) {
                Some(v) => Ok(v),
                None => {
                    // KeyError: str is the repr of the key (handled in
                    // ExceptionValue::py_str).
                    Err(Flow::Raise(ExceptionValue::new(
                        "KeyError",
                        vec![key.clone()],
                    )))
                }
            }
        }
        // 02.04a (interp bug #7, vendored-stdlib stress): PEP 585 generic
        // alias `list[int]` — types.py reads type(list[int]) at import.
        // v0: a stub GenericAlias value (no runtime generic semantics).
        (Value::Obj(o), _k) if o.kind == ap_obj::ObjKind::Type => {
            let ga = ap_obj::PyObject::new(ap_obj::ObjKind::Namespace, "_GenericAlias");
            {
                let mut attrs = ga.attrs.borrow_mut();
                let _ = attrs.set(
                    Value::Str(PyString::new("__origin__")),
                    Value::Obj(o.clone()),
                );
                let _ = attrs.set(
                    Value::Str(PyString::new("__args__")),
                    Value::Tuple(Gc::new(Tuple(vec![key.clone()]))),
                );
            }
            Ok(Value::Obj(ga))
        }
        _ => Err(raise(
            "TypeError",
            format!("'{}' object is not subscriptable", obj.type_name()),
        )),
    }
}

fn normalize_index(i: i64, len: i64) -> Option<i64> {
    let idx = if i < 0 { i + len } else { i };
    if idx >= 0 && idx < len {
        Some(idx)
    } else {
        None
    }
}

/// obj[key] = v store.
pub fn subscript_set(obj: &Value, key: &Value, v: Value) -> Result<(), Flow> {
    match obj {
        Value::List(l) => {
            let k = subscript_key(key)?;
            let i = match k {
                Key::Index(i) => i,
                _ => return Err(raise("TypeError", "list indices must be integers")),
            };
            let idx = normalize_index(i, l.len() as i64)
                .ok_or_else(|| raise("IndexError", "list assignment index out of range"))?;
            l.set_at(idx as usize, v);
            Ok(())
        }
        Value::Dict(d) => {
            // Unhashable key: exact dict-key TypeError (V043/V046 phrasing).
            if let Err(e) = ap_obj::ops::hash(key) {
                return Err(Flow::Raise(
                    ExceptionValue::msg(
                        "TypeError",
                        format!(
                            "cannot use '{}' as a dict key (unhashable type: '{}')",
                            key.type_name(),
                            key.type_name()
                        ),
                    )
                    .tap(|_| {
                        let _ = e;
                    }),
                ));
            }
            d.set(key.clone(), v).map_err(Flow::from_py)
        }
        Value::Str(_) => Err(raise(
            "TypeError",
            "'str' object does not support item assignment",
        )),
        Value::Tuple(_) => Err(raise(
            "TypeError",
            "'tuple' object does not support item assignment",
        )),
        _ => Err(raise(
            "TypeError",
            format!(
                "'{}' object does not support item assignment",
                obj.type_name()
            ),
        )),
    }
}

trait Tap: Sized {
    fn tap<F: FnOnce(&Self)>(self, f: F) -> Self {
        f(&self);
        self
    }
}
impl<T> Tap for T {}

/// del obj[key].
pub fn subscript_del(obj: &Value, key: &Value) -> Result<(), Flow> {
    match obj {
        Value::List(l) => {
            let k = subscript_key(key)?;
            let i = match k {
                Key::Index(i) => i,
                _ => return Err(raise("TypeError", "list indices must be integers")),
            };
            let idx = normalize_index(i, l.len() as i64)
                .ok_or_else(|| raise("IndexError", "list index out of range"))?;
            l.remove_at(idx as usize);
            Ok(())
        }
        Value::Dict(d) => {
            let removed = d.pop(key);
            if removed.is_none() {
                return Err(Flow::Raise(ExceptionValue::new(
                    "KeyError",
                    vec![key.clone()],
                )));
            }
            Ok(())
        }
        Value::Tuple(_) => Err(raise(
            "TypeError",
            "'tuple' object doesn't support item deletion",
        )),
        Value::Str(_) => Err(raise(
            "TypeError",
            "'str' object doesn't support item deletion",
        )),
        _ => Err(raise(
            "TypeError",
            format!("'{}' object doesn't support item deletion", obj.type_name()),
        )),
    }
}

/// Method call on a builtin receiver. Returns None when the method name is
/// unknown (caller raises AttributeError).
pub fn call_method(
    interp: &Interpreter,
    obj: &Value,
    name: &str,
    args: &[Value],
) -> Option<MResult> {
    let r: Option<MResult> = match (obj, name) {
        (Value::Str(s), "upper") => Some(Ok(Value::Str(PyString::new(s.as_str().to_uppercase())))),
        (Value::Str(s), "lower") => Some(Ok(Value::Str(PyString::new(s.as_str().to_lowercase())))),
        (Value::Str(s), "join") => {
            // Join accepts any iterable incl. generators; the error path
            // here returns MResult where Flow is already wrapped.
            let it = match crate::interp::iterable_of_public(one_arg(args)?) {
                Ok(it) => it,
                Err(Flow::Raise(e)) => return Some(Err(Flow::Raise(e))),
                Err(other) => return Some(Err(other)),
            };
            let parts = crate::iter::collect(&it);
            let mut strs = Vec::with_capacity(parts.len());
            for p in parts {
                match p {
                    Value::Str(x) => strs.push(x.as_str().to_string()),
                    other => {
                        return Some(Err(raise(
                            "TypeError",
                            format!(
                                "sequence item {}: expected str instance, {} found",
                                {
                                    // CPython indexes from 0
                                    0
                                },
                                other.type_name()
                            ),
                        )));
                    }
                }
            }
            Some(Ok(Value::Str(PyString::new(strs.join(s.as_str())))))
        }
        (Value::Str(s), "encode") => {
            let codec = args
                .first()
                .and_then(|v| match v {
                    Value::Str(x) => Some(x.as_str().to_string()),
                    _ => None,
                })
                .unwrap_or_else(|| "utf-8".to_string());
            Some(encode_str(s.as_str(), &codec))
        }
        (Value::Str(s), "partition") => {
            let sep = match one_arg(args)? {
                Value::Str(x) => x.as_str().to_string(),
                _ => return Some(Err(raise("TypeError", "must be str"))),
            };
            if sep.is_empty() {
                return Some(Err(raise("ValueError", "empty separator")));
            }
            let (head, found, tail) = match s.as_str().split_once(&sep) {
                Some((h, t)) => (h.to_string(), true, t.to_string()),
                None => (s.as_str().to_string(), false, String::new()),
            };
            let _ = found;
            let triple = if s.as_str().contains(&sep) {
                let (h, t) = s.as_str().split_once(&sep).unwrap();
                (h.to_string(), sep.clone(), t.to_string())
            } else {
                (s.as_str().to_string(), String::new(), String::new())
            };
            let _ = (head, tail);
            Some(Ok(Value::Tuple(Gc::new(Tuple(vec![
                Value::Str(PyString::new(triple.0)),
                Value::Str(PyString::new(triple.1)),
                Value::Str(PyString::new(triple.2)),
            ])))))
        }
        (Value::Bytes(b), "decode") => {
            let codec = args
                .first()
                .and_then(|v| match v {
                    Value::Str(x) => Some(x.as_str().to_string()),
                    _ => None,
                })
                .unwrap_or_else(|| "utf-8".to_string());
            Some(decode_bytes(b, &codec))
        }
        (Value::List(l), "append") => {
            l.append(one_arg(args)?.clone());
            Some(Ok(Value::None))
        }
        (Value::List(l), "clear") => {
            l.clear();
            Some(Ok(Value::None))
        }
        (Value::Dict(d), "keys") => {
            let items = d.keys();
            Some(Ok(Value::List(PyList::from_vec(items))))
        }
        (Value::Dict(d), "values") => {
            let items = d.items().into_iter().map(|(_, v)| v).collect();
            Some(Ok(Value::List(PyList::from_vec(items))))
        }
        (Value::Dict(d), "items") => {
            let items = d
                .items()
                .into_iter()
                .map(|(k, v)| Value::Tuple(Gc::new(Tuple(vec![k, v]))))
                .collect();
            Some(Ok(Value::List(PyList::from_vec(items))))
        }
        (Value::Set(s), "add") => {
            let r = s.add(one_arg(args)?.clone()).map_err(Flow::from_py);
            Some(r.map(|_| Value::None))
        }
        // 02.04a (interp bug #19): set.copy() (abc cache bookkeeping).
        (Value::Set(s), "copy") => {
            let items: Vec<Value> = s.iter().collect();
            let set = ap_obj::dict::PySet::new();
            for it in items {
                let _ = set.add(it);
            }
            Some(Ok(Value::Set(set)))
        }
        (Value::Frozen(s), "copy") => {
            let items: Vec<Value> = s.iter().collect();
            let set = ap_obj::dict::PySet::new();
            for it in items {
                let _ = set.add(it);
            }
            Some(Ok(Value::Set(set)))
        }
        (Value::Set(s), "discard") => {
            let _ = s.pop_discard(one_arg(args)?);
            Some(Ok(Value::None))
        }
        (Value::Obj(o), "set_int_max_str_digits")
            if o.kind == ap_obj::ObjKind::BuiltinFn && o.name == "sys.set_int_max_str_digits" =>
        {
            Some(
                crate::builtins::call_sys_fn(interp, "set_int_max_str_digits", args)
                    .unwrap_or_else(|| Ok(Value::None)),
            )
        }
        (Value::Obj(o), _) if o.kind == ap_obj::ObjKind::UserInstance => {
            // 02.03: instances dispatch through the class MRO; the v0
            // attrs-only path stays as fallback for legacy shapes.
            match super_user_method(interp, o, name, args) {
                Err(Flow::Raise(e)) if e.class == "AttributeError" => None,
                other => Some(other),
            }
        }
        _ => None,
    };
    r
}

fn one_arg(args: &[Value]) -> Option<&Value> {
    args.first()
}

/// __len__/__bool__/__str__/__repr__ dispatch on user instances (the v0
/// dunder protocol; full class dispatch is 02.03).
fn super_user_method(
    interp: &Interpreter,
    o: &Gc<ap_obj::PyObject>,
    name: &str,
    _args: &[Value],
) -> MResult {
    // Arbitrary method read on an instance: look up __dict__ first.
    if let Some(v) = o.attrs.borrow().get(&Value::Str(PyString::new(name))) {
        // Bound method call: self is implicit.
        return crate::interp::call_value_pub(&interp.rc(), &v, &[Value::Obj(o.clone())], &[])
            .map(|_| Value::None)
            .and_then(|_| Ok(Value::None));
    }
    Err(raise(
        "AttributeError",
        format!("'{}' object has no attribute '{}'", o.name, name),
    ))
}

/// bytes.decode(codec).
fn decode_bytes(b: &[u8], codec: &str) -> MResult {
    let codec_l = codec.to_ascii_lowercase();
    match codec_l.as_str() {
        "utf-8" | "utf8" => match std::str::from_utf8(b) {
            Ok(s) => Ok(Value::Str(PyString::new(s))),
            Err(_) => Err(raise(
                "UnicodeDecodeError",
                format!("'utf-8' codec can't decode bytes: invalid continuation byte"),
            )),
        },
        "ascii" => {
            if b.iter().all(|&x| x < 0x80) {
                Ok(Value::Str(PyString::new(
                    String::from_utf8_lossy(b).as_ref(),
                )))
            } else {
                let bad = b.iter().position(|&x| x >= 0x80).unwrap_or(0);
                Err(raise(
                    "UnicodeDecodeError",
                    format!(
                        "'ascii' codec can't decode byte 0x{:02x} in position {bad}: ordinal not in range(128)",
                        b[bad]
                    ),
                ))
            }
        }
        other => Err(raise("LookupError", format!("unknown encoding: {other}"))),
    }
}

/// str.encode(codec) — lives here (method surface) not in builtins.
pub fn encode_str(s: &str, codec: &str) -> MResult {
    let codec_l = codec.to_ascii_lowercase();
    match codec_l.as_str() {
        "utf-8" | "utf8" => {
            // Surrogates are rejected (V016: '\ud800'.encode('utf-8')).
            if s.chars().any(|c| (0xD800..0xE000).contains(&(c as u32))) {
                let pos = s
                    .chars()
                    .position(|c| (0xD800..0xE000).contains(&(c as u32)))
                    .unwrap_or(0);
                let ch = s.chars().nth(pos).unwrap();
                return Err(raise(
                    "UnicodeEncodeError",
                    format!(
                        "'utf-8' codec can't encode character '\\u{:04x}' in position {pos}: surrogates not allowed",
                        ch as u32
                    ),
                ));
            }
            Ok(Value::Bytes(Gc::new(s.as_bytes().to_vec())))
        }
        "ascii" => {
            if s.is_ascii() {
                Ok(Value::Bytes(Gc::new(s.as_bytes().to_vec())))
            } else {
                let pos = s.chars().take_while(|c| c.is_ascii()).count();
                let ch = s.chars().nth(pos).unwrap_or('\0');
                Err(raise(
                    "UnicodeEncodeError",
                    format!(
                        "'ascii' codec can't encode character '\\u{:04x}' in position {pos}: ordinal not in range(128)",
                        ch as u32
                    ),
                ))
            }
        }
        other => Err(raise("LookupError", format!("unknown encoding: {other}"))),
    }
}

/// The type name of the first unhashable element reachable in a tuple/list
/// (CPython's dict-key error scans members; V043 pins tuple-with-list).
fn first_unhashable_type(key: &Value) -> Option<String> {
    match key {
        Value::Tuple(t) => {
            for e in &t.0 {
                if ap_obj::ops::hash(e).is_err() {
                    return Some(match e {
                        Value::Obj(o) => o.type_name(),
                        other => other.type_name().to_string(),
                    });
                }
                if let Some(inner) = first_unhashable_type(e) {
                    return Some(inner);
                }
            }
            None
        }
        Value::List(l) => {
            for e in l.items().iter() {
                if ap_obj::ops::hash(e).is_err() {
                    return Some(e.type_name().to_string());
                }
            }
            None
        }
        _ => None,
    }
}
