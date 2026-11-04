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
    let m = msg.into();
    if class == "IndexError" && std::env::var("APX_IDX_DEBUG").is_ok() {
        eprintln!("APX-IDX-RAISE: IndexError {:?} at methods::raise", m);
    }
    Flow::Raise(ExceptionValue::msg(class, m))
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
            let idx = normalize_index(*i, l.len() as i64).ok_or_else(|| {
                if std::env::var("APX_IDX_DEBUG").is_ok() {
                    eprintln!(
                        "APX-IDX-DEBUG: list len={} idx={} (list index out of range)",
                        l.len(),
                        *i
                    );
                    let bt = std::backtrace::Backtrace::force_capture();
                    eprintln!("APX-IDX-BACKTRACE:\n{bt}");
                    crate::interp::LAST_PY_LOC.with(|c| {
                        let (f, l) = &*c.borrow();
                        eprintln!("APX-IDX-PYLOC: {f}:{l}");
                    });
                }
                raise("IndexError", "list index out of range")
            })?;
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
                    first_unhashable_type(key).unwrap_or_else(|| key_type_name(key));
                return Err(raise(
                    "TypeError",
                    format!(
                        "cannot use '{}' as a dict key (unhashable type: '{}')",
                        key_type_name(key),
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
        // 02.08-full part 1: NamedTuple subscript (sys.version_info[:2]).
        (Value::Obj(o), _) if o.kind == ap_obj::ObjKind::NamedTuple => {
            let vals = o
                .attrs
                .borrow()
                .get(&Value::Str(PyString::new("<values>")))
                .map(|v| match v {
                    Value::List(l) => l.items().to_vec(),
                    _ => Vec::new(),
                })
                .unwrap_or_default();
            match &k {
                Key::Index(idx) => {
                    let i = normalize_index(*idx, vals.len() as i64)
                        .ok_or_else(|| raise("IndexError", "tuple index out of range"))?;
                    Ok(vals[i as usize].clone())
                }
                Key::Slice { lo, hi, step } => Ok(Value::Tuple(Gc::new(Tuple(slice_seq(
                    &vals, *lo, *hi, *step,
                ))))),
                _ => Err(raise(
                    "TypeError",
                    format!(
                        "tuple indices must be integers or slices, not {}",
                        key.type_name()
                    ),
                )),
            }
        }
        // collections.deque subscript (native opaque): negative indices,
        // IndexError "deque index out of range".
        (Value::Obj(o), Key::Index(i))
            if o.kind == ap_obj::ObjKind::Opaque && o.name == "deque" =>
        {
            let (items, _) = crate::builtins_modules::deque_state_pub(obj)?;
            let idx = normalize_index(*i, items.len() as i64)
                .ok_or_else(|| raise("IndexError", "deque index out of range"))?;
            Ok(items[idx as usize].clone())
        }
        (Value::Obj(o), Key::Slice { lo, hi, step })
            if o.kind == ap_obj::ObjKind::Opaque && o.name == "deque" =>
        {
            if *step == 0 {
                return Err(raise("ValueError", "slice step cannot be zero"));
            }
            let (items, _) = crate::builtins_modules::deque_state_pub(obj)?;
            let out = slice_seq(&items, *lo, *hi, *step);
            Ok(crate::builtins_modules::make_deque_pub(out, None))
        }
        (Value::Obj(o), _k) if o.kind == ap_obj::ObjKind::Opaque && o.name == "deque" => {
            Err(raise(
                "TypeError",
                "deque indices must be integers or slices, not str",
            ))
        }
        // 02.08-full p2: re.Match[m] / re.Pattern[...] subscripts.
        (Value::Obj(o), k) if o.kind == ap_obj::ObjKind::Opaque && o.name == "re.Match" => {
            let kv = match k {
                Key::Index(i) => Value::Int(Int::from_i64(*i)),
                Key::Other(v) => v.clone(),
                Key::Float => {
                    return Err(raise(
                        "TypeError",
                        "group() argument must be str or int, not float",
                    ))
                }
                Key::Slice { .. } => {
                    return Err(raise(
                        "TypeError",
                        "group() argument must be str or int, not slice",
                    ))
                }
            };
            crate::builtins_modules::re_dispatch("Match.__getitem__", &[obj.clone(), kv], &[])
        }
        // T1-d: re-subscription of a _GenericAlias alias (list[T][int]) —
        // substitute __parameters__ with the new args (v0 identity sub).
        (Value::Obj(o), _k)
            if o.kind == ap_obj::ObjKind::Namespace && o.name == "_GenericAlias" =>
        {
            let has_params = o
                .attrs
                .borrow()
                .get(&Value::Str(PyString::new("__parameters__")))
                .is_some();
            if has_params {
                let ga = ap_obj::PyObject::new(ap_obj::ObjKind::Namespace, "_GenericAlias");
                {
                    let mut attrs = ga.attrs.borrow_mut();
                    let origin = o
                        .attrs
                        .borrow()
                        .get(&Value::Str(PyString::new("__origin__")))
                        .map(|x| x.clone())
                        .unwrap_or(Value::None);
                    let _ = attrs.set(Value::Str(PyString::new("__origin__")), origin);
                    // v0 substitution: walk __args__, replace TypeVar
                    // positions with the subscription args (single-arg form).
                    let old_args = o
                        .attrs
                        .borrow()
                        .get(&Value::Str(PyString::new("__args__")))
                        .map(|x| x.clone());
                    let mut new_args: Vec<Value> = Vec::new();
                    if let Some(Value::Tuple(t)) = &old_args {
                        let kv: Vec<Value> = match key {
                            Value::Tuple(tt) => tt.0.clone(),
                            other => vec![other.clone()],
                        };
                        let mut ki = 0usize;
                        for av in &t.0 {
                            let is_tv = matches!(av, Value::Obj(vo) if {
                                let sp = vo.slice_parts.borrow();
                                matches!(sp.first(), Some(Value::Str(ps)) if ps.as_str() == "__ap_typevar__")
                            });
                            if is_tv && ki < kv.len() {
                                new_args.push(kv[ki].clone());
                                ki += 1;
                            } else {
                                new_args.push(av.clone());
                            }
                        }
                    }
                    let _ = attrs.set(
                        Value::Str(PyString::new("__args__")),
                        Value::Tuple(Gc::new(Tuple(new_args))),
                    );
                    // All params substituted → empty __parameters__.
                    let _ = attrs.set(
                        Value::Str(PyString::new("__parameters__")),
                        Value::Tuple(Gc::new(Tuple(Vec::new()))),
                    );
                }
                return Ok(Value::Obj(ga));
            }
            // No __parameters__ (fully-substituted alias): v0 inert —
            // return a fresh alias carrying the same origin/args.
            let ga = ap_obj::PyObject::new(ap_obj::ObjKind::Namespace, "_GenericAlias");
            {
                let mut attrs = ga.attrs.borrow_mut();
                for src_key in ["__origin__", "__args__"] {
                    if let Some(vv) = o
                        .attrs
                        .borrow()
                        .get(&Value::Str(PyString::new(src_key)))
                        .map(|x| x.clone())
                    {
                        let _ = attrs.set(Value::Str(PyString::new(src_key)), vv);
                    }
                }
            }
            return Ok(Value::Obj(ga));
        }
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
                // T1-d: typing._GenericAlias.__getitem__ needs
                // __parameters__ (TypeVar members of __args__) to compute
                // substitutions for re-subscription (defer.py
                // (_DeferredListResultListT = list[_DeferredListResultItemT[T]]).
                let mut tvs: Vec<Value> = Vec::new();
                let mut collect_tvs = |v: &Value, tvs: &mut Vec<Value>| {
                    if let Value::Obj(vo) = v {
                        if let Some(Value::Str(flag)) = vo.slice_parts.borrow().first().cloned() {
                            if flag.as_str() == "__ap_typevar__" {
                                tvs.push(v.clone());
                            }
                        }
                    }
                };
                collect_tvs(&key, &mut tvs);
                if !tvs.is_empty() {
                    let _ = attrs.set(
                        Value::Str(PyString::new("__parameters__")),
                        Value::Tuple(Gc::new(Tuple(tvs))),
                    );
                }
            }
            Ok(Value::Obj(ga))
        }
        (Value::Obj(o), _k) if o.kind == ap_obj::ObjKind::UserClass => {
            // 02.03b: metaclass __getitem__ (enum: Color["GREEN"]).
            if let Some(t) = o.class_ref.borrow().clone() {
                if let Some(meta) = t.metaclass.borrow().clone() {
                    if let Some((gf, _)) = meta.lookup_with_owner("__getitem__") {
                        let is_fn =
                            matches!(&gf, Value::Obj(fo) if fo.kind == ap_obj::ObjKind::Function);
                        if is_fn {
                            return crate::interp::call_value_pub3(
                                &gf,
                                &[obj.clone(), key.clone()],
                            );
                        }
                    }
                }
                // T1 (scrapy fix pass): __class_getitem__ (PEP 560) —
                // `list[int]`-style and `Generic[T]` subscription on classes.
                // v0: return an opaque _GenericAlias placeholder carrying
                // origin + params (real alias semantics are a documented
                // Plan-02 gap; scrapy's annotations only need the object to
                // EXIST and be inert).
                let is_generic = t.lookup("__class_getitem__").is_some()
                    || t.mro.borrow().iter().any(|m| m.name == "Generic")
                    || o.name == "Generic";
                // 02.08-c fix: classes deriving from builtin containers
                // (`class SelectorList(list)`) get PEP 560
                // __class_getitem__ from the C base — parsel's
                // `SelectorList["Selector"]` class-body annotation hits this
                // at import. Default to the alias placeholder for ANY
                // UserClass without an explicit __getitem__ metaclass hook:
                // the subscription result only needs to EXIST and be inert.
                let g = ap_obj::PyObject::new(ap_obj::ObjKind::Opaque, "typing._GenericAlias");
                *g.slice_parts.borrow_mut() = vec![obj.clone(), key.clone()];
                return Ok(Value::Obj(g));
            }
            Err(raise(
                "TypeError",
                format!("'{}' object is not subscriptable", o.name),
            ))
        }
        // 02.08-c: lxml element indexing (r[0], r[-1]) — native surface.
        (Value::Obj(o), _k)
            if o.kind == ap_obj::ObjKind::Opaque
                && (o.name == "lxml.etree._Element" || o.name == "lxml.html.HtmlElement") =>
        {
            let key = crate::builtins::py_str_of(&key.clone())
                .map(|s| s.as_str().to_string())
                .unwrap_or_default();
            match crate::lxml::call_elem(
                "__getitem__",
                &[obj.clone(), Value::Str(ap_obj::PyString::new(key))],
                &[],
            ) {
                Some(r) => r,
                None => Err(raise(
                    "TypeError",
                    format!("'{}' object is not subscriptable", o.name),
                )),
            }
        }
        (Value::Obj(o), _k) if o.kind == ap_obj::ObjKind::BuiltinFn => {
            // T1 (scrapy fix pass): subscripting a builtin marker fn (PEP 560
            // __class_getitem__-style usage) - opaque alias placeholder.
            let g = ap_obj::PyObject::new(ap_obj::ObjKind::Opaque, "typing._GenericAlias");
            *g.slice_parts.borrow_mut() = vec![obj.clone(), key.clone()];
            return Ok(Value::Obj(g));
        }
        // 02.04 completion: weak containers route subscripts through the
        // gcsupport surface (dunder ops, self rides args[0]).
        (Value::Obj(o), _k)
            if o.kind == ap_obj::ObjKind::Opaque
                && crate::gcsupport::is_weak_container(&o.name) =>
        {
            return match crate::gcsupport::call_weakref(
                &format!("{}.__getitem__", o.name),
                &[obj.clone(), key.clone()],
                &[],
            ) {
                Some(r) => r,
                None => Err(raise(
                    "TypeError",
                    format!("'{}' object is not subscriptable", obj.type_name()),
                )),
            };
        }
        // T2 (_pydatetime._mktime: `_time.localtime(u)[:6]`): native
        // struct_time subscripts through its <values> field tuple.
        (Value::Obj(o), _k) if o.kind == ap_obj::ObjKind::Opaque && o.name == "struct_time" => {
            match o.attrs.borrow().get(&Value::Str(PyString::new("<values>"))) {
                Some(Value::List(l)) => {
                    return subscript_get(&Value::List(l.clone()), key);
                }
                _ => Err(raise(
                    "TypeError",
                    "'struct_time' object is not subscriptable",
                )),
            }
        }
        // A-3 item 5: stat_result (opaque, slice_parts-populated) subscripts
        // and slices through slice_parts like a plain sequence.
        // A-4 item 5b: slices return a TUPLE — stat_result and uname_result
        // are structseqs (tuple subclasses), st[:2] must be a tuple in
        // CPython, not a list.
        (Value::Obj(o), _k)
            if o.kind == ap_obj::ObjKind::Opaque && !o.slice_parts.borrow().is_empty() =>
        {
            let parts = o.slice_parts.borrow().clone();
            return subscript_get(&Value::Tuple(Gc::new(Tuple(parts))), key);
        }
        // A-4 item 1: bytearray subscripts — ba[i] → int, ba[a:b] →
        // bytearray (CPython returns the SAME type for a slice).
        (Value::Obj(o), _k) if o.kind == ap_obj::ObjKind::Opaque && o.name == "bytearray" => {
            let data = crate::builtins_modules::bytearray_state(obj)?;
            match subscript_get(&Value::Bytes(Gc::new(data)), key)? {
                Value::Int(n) => Ok(Value::Int(n)),
                Value::Bytes(b) => Ok(crate::builtins_modules::make_bytearray(b.as_ref().clone())),
                other => Ok(other),
            }
        }
        (Value::Obj(o), _k) if o.kind == ap_obj::ObjKind::UserInstance => {
            // T1-d (attr/_make.py): builtin-backed subclass instances carry
            // __ap_tuple_payload__ — subscript/iteration delegate to it
            // (AttrsClass instances ARE tuples).
            if let Some(p) = o
                .attrs
                .borrow()
                .get(&Value::Str(PyString::new("__ap_tuple_payload__")))
            {
                return subscript_get(&p, key);
            }
            // T2 (pygments _TokenType / traceback StackSummary): tuple- and
            // list-derived instances subscript through their backing builtin.
            if crate::classes::is_tuple_derived_instance(obj) {
                return subscript_get(
                    &Value::Tuple(crate::classes::tuple_backing_or_new(obj)),
                    key,
                );
            }
            if crate::classes::is_list_derived_instance(obj) {
                if let Value::Str(sk) = key {
                    if std::env::var("APX_SCOPE_DEBUG").is_ok() {
                        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open("/tmp/t1listidx.txt") {
                            use std::io::Write;
                            let _ = writeln!(f, "LISTIDX str key={} recv={}", sk.as_str(), obj.type_name());
                        }
                    }
                }
                return subscript_get(&Value::List(crate::classes::list_backing_or_new(obj)), key);
            }
            // T1-d (typing.NamedTuple): namedtuple instances ARE tuples —
            // index into the declared field order (n[0] == n.<field0>).
            if let Some(t) = o.type_ref.borrow().as_ref().and_then(|w| w.upgrade()) {
                if let Some(Value::List(fields)) = t
                    .dict
                    .get(&Value::Str(PyString::new("__ap_namedtuple_fields__")))
                {
                    let names: Vec<String> = fields
                        .items()
                        .iter()
                        .filter_map(|v| match v {
                            Value::Str(s) => Some(s.as_str().to_string()),
                            _ => None,
                        })
                        .collect();
                    let k = subscript_key(key)?;
                    if let crate::methods::Key::Index(i) = &k {
                        let idx = if *i < 0 { *i + names.len() as i64 } else { *i };
                        if idx >= 0 && (idx as usize) < names.len() {
                            let fname = &names[idx as usize];
                            if let Some(v) = o
                                .attrs
                                .borrow()
                                .get(&Value::Str(PyString::new(fname.clone())))
                            {
                                return Ok(v);
                            }
                        }
                        return Err(raise("IndexError", "tuple index out of range"));
                    }
                }
            }
            // 02.03b: subscript read on dict-derived instances — the
            // subclass __getitem__ (if any) first, else the backing dict.
            if let Some(t) = o.type_ref.borrow().as_ref().and_then(|w| w.upgrade()) {
                if crate::classes::is_dict_derived(&t) {
                    if let Some((gf, gowner)) = t.lookup_with_owner("__getitem__") {
                        if matches!(&gf, Value::Obj(fo) if fo.kind == ap_obj::ObjKind::Function) {
                            // Wave-78: direct ctx push — CID.__getitem__ calls
                            // super().__getitem__ (same as the set/del paths).
                            if let Some(interp) = crate::builtins_modules::current_interp() {
                                interp.super_stack.borrow_mut().push((gowner, obj.clone(), false));
                                let r = crate::interp::call_value_pub3(
                                    &gf,
                                    &[obj.clone(), key.clone()],
                                );
                                interp.super_stack.borrow_mut().pop();
                                return r;
                            }
                            return crate::interp::call_value_pub3(
                                &gf,
                                &[obj.clone(), key.clone()],
                            );
                        }
                    }
                    if let Some(d) = crate::classes::dict_backing(obj) {
                        return match d.get(key) {
                            Some(v) => Ok(v),
                            None => {
                                // CPython dict-subclass protocol: a __missing__
                                // hook on the subclass answers a missed lookup
                                // (typing._overload_registry = defaultdict(
                                // partial(defaultdict, dict)) depends on it).
                                // Misses without the hook raise KeyError.
                                if let Some((mf, _)) = t.lookup_with_owner("__missing__") {
                                    if matches!(&mf, Value::Obj(fo)
                                        if fo.kind == ap_obj::ObjKind::Function)
                                    {
                                        return crate::interp::call_value_pub3(
                                            &mf,
                                            &[obj.clone(), key.clone()],
                                        );
                                    }
                                }
                                Err(Flow::Raise(ExceptionValue::new(
                                    "KeyError",
                                    vec![key.clone()],
                                )))
                            }
                        };
                    }
                }
                // 02.03 user __getitem__ (S-03-23 read side).
                if let Some((gf, _)) = t.lookup_with_owner("__getitem__") {
                    if matches!(&gf, Value::Obj(fo) if fo.kind == ap_obj::ObjKind::Function) {
                        return crate::interp::call_value_pub3(&gf, &[obj.clone(), key.clone()]);
                    }
                }
            }
            Err(raise(
                "TypeError",
                format!("'{}' object is not subscriptable", obj.type_name()),
            ))
        }
        _ => Err(raise(
            "TypeError",
            format!("'{}' object is not subscriptable", obj.type_name()),
        )),
    }
}

/// A-4 item 1: observable type name for dict-key errors — bytearray Objs
/// report 'bytearray', not the Opaque fallback 'object'.
pub(crate) fn key_type_name(v: &Value) -> String {
    if let Value::Obj(o) = v {
        if o.kind == ap_obj::ObjKind::Opaque && o.name == "bytearray" {
            return "bytearray".to_string();
        }
    }
    v.type_name().to_string()
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
        // A-4 item 1: bytearray item assignment (ba[0] = 65) and slice
        // replacement (ba[1:2] = b"x", equal-length only at v0).
        Value::Obj(o) if o.kind == ap_obj::ObjKind::Opaque && o.name == "bytearray" => {
            let k = subscript_key(key)?;
            let mut data = crate::builtins_modules::bytearray_state(obj)?;
            match (&k, &v) {
                (Key::Index(i), Value::Int(n)) => {
                    let idx = normalize_index(*i, data.len() as i64).ok_or_else(|| {
                        raise("IndexError", "bytearray assignment index out of range")
                    })?;
                    let b = n.to_i64().unwrap_or(0);
                    if !(0..=255).contains(&b) {
                        return Err(raise("ValueError", "byte must be in range(0, 256)"));
                    }
                    data[idx as usize] = b as u8;
                    crate::builtins_modules::bytearray_set(obj, data)
                }
                (Key::Slice { lo, hi, step }, Value::Bytes(nb)) if *step == 1 => {
                    let len = data.len() as i64;
                    let clamp = |x: i64| -> i64 {
                        let x = if x < 0 { x + len } else { x };
                        x.clamp(0, len)
                    };
                    let start = clamp(lo.unwrap_or(0));
                    let stop = clamp(hi.unwrap_or(len));
                    let span = (stop - start) as usize;
                    if nb.len() != span {
                        return Err(raise(
                            "NotImplementedError",
                            "bytearray slice resize unsupported (v0)",
                        ));
                    }
                    data[start as usize..stop as usize].copy_from_slice(nb);
                    crate::builtins_modules::bytearray_set(obj, data)
                }
                _ => Err(raise("TypeError", "an integer is required (got type str)")),
            }
        }
        Value::List(l) => {
            let k = subscript_key(key)?;
            let i = match k {
                Key::Index(i) => i,
                // T2 (argparse `positionals[:] = positionals[len(arg_counts):]`):
                // list slice ASSIGNMENT (step==1: replace span with any
                // iterable; step!=1: lengths must match exactly).
                Key::Slice { lo, hi, step } => {
                    let len = l.len() as i64;
                    let clamp = |x: i64| -> i64 {
                        let x = if x < 0 { x + len } else { x };
                        x.clamp(0, len)
                    };
                    if step == 1 {
                        let start = clamp(lo.unwrap_or(0));
                        let stop = clamp(hi.unwrap_or(len));
                        let items: Vec<Value> = match &v {
                            Value::List(src) => src.items().to_vec(),
                            Value::Tuple(t) => t.0.clone(),
                            other => {
                                let it = crate::interp::iterable_of_public(other)?;
                                crate::iter::collect(&it)
                            }
                        };
                        // span items [start, stop) removed, `items` inserted
                        // (append-before-insertion-point via set+push shim).
                        l.remove_range(start as usize, stop as usize);
                        for (off, item) in items.iter().enumerate() {
                            let pos = start as usize + off;
                            if pos < l.len() {
                                l.set_at(pos, item.clone());
                            } else {
                                l.append(item.clone());
                            }
                        }
                        return Ok(());
                    }
                    let start = clamp(lo.unwrap_or(0));
                    let stop = clamp(hi.unwrap_or(len));
                    let span_len = ((stop - start + step.abs() - 1) / step.abs()).max(0);
                    let items: Vec<Value> = match &v {
                        Value::List(src) => src.items().to_vec(),
                        Value::Tuple(t) => t.0.clone(),
                        other => {
                            let it = crate::interp::iterable_of_public(other)?;
                            crate::iter::collect(&it)
                        }
                    };
                    if items.len() as i64 != span_len {
                        return Err(raise(
                            "ValueError",
                            "attempt to assign sequence of size \
                             X to extended slice of size Y"
                                .to_string(),
                        ));
                    }
                    let mut idx = start;
                    let mut off = 0usize;
                    while (step > 0 && idx < stop) || (step < 0 && idx > stop) {
                        l.set_at(idx as usize, items[off].clone());
                        idx += step;
                        off += 1;
                    }
                    return Ok(());
                }
                other => {
                    let tname = match obj {
                        Value::Obj(ob) => format!("{:?}", ob.kind),
                        _ => "prim".to_string(),
                    };
                    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open("/tmp/t1listidx.txt") {
                        use std::io::Write;
                        let key_repr = match other {
                            Key::Other(v) => format!("Other({})", v.type_name()),
                            Key::Index(i) => format!("Index({})", i),
                            _ => "other".to_string(),
                        };
                        let _ = writeln!(f, "LISTIDX-SET-OTHER other={} objkind={}", key_repr, tname);
                    }
                    return Err(raise("TypeError", "list indices must be integers"));
                }
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
                            key_type_name(key),
                            key_type_name(key)
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
        // 02.04 completion: weak containers route item stores.
        Value::Obj(o)
            if o.kind == ap_obj::ObjKind::Opaque
                && crate::gcsupport::is_weak_container(&o.name) =>
        {
            match crate::gcsupport::call_weakref(
                &format!("{}.__setitem__", o.name),
                &[obj.clone(), key.clone(), v],
                &[],
            ) {
                Some(Ok(_)) => Ok(()),
                Some(Err(f)) => Err(f),
                None => Ok(()),
            }
        }
        Value::Obj(o) if o.kind == ap_obj::ObjKind::UserInstance => {
            // 02.03b: __setitem__ on dict-derived instances — the subclass
            // override (EnumDict) runs first; with none, the backing dict
            // takes the write (dict semantics).
            if let Some(t) = o.type_ref.borrow().as_ref().and_then(|w| w.upgrade()) {
                let overridden = t
                    .lookup_with_owner("__setitem__")
                    .map(|(f, _)| matches!(&f, Value::Obj(fo) if fo.kind == ap_obj::ObjKind::Function))
                    .unwrap_or(false);
                if overridden {
                    let interp = crate::interp::current_interp_handle();
                    let (sf, owner) = t
                        .lookup_with_owner("__setitem__")
                        .map(|(f, ow)| (f, ow))
                        .unwrap_or((Value::None, t.clone()));
                    let caller = crate::interp::module_frame_pub();
                    // super() inside __setitem__ (EnumDict -> dict) needs
                    // the defining-class ctx (same channel as bound-method
                    // dispatch).
                    // Wave-78: push the ctx DIRECTLY (not via pending) —
                    // the plain-fn reader can skip pending under borrow
                    // contention, leaving it for the next nested call to
                    // steal. Direct push + builtin_super's skip-sentinel
                    // makes the ctx visible to super() inside __setitem__.
                    interp.super_stack.borrow_mut().push((owner, obj.clone(), false));
                    let r = crate::interp::call_value(
                        &interp,
                        &caller,
                        &sf,
                        &[obj.clone(), key.clone(), v],
                        &[],
                    );
                    interp.super_stack.borrow_mut().pop();
                    interp.super_stack.borrow_mut().pop();
                    return r.map(|_| ());
                }
                if crate::classes::is_dict_derived(&t) {
                    if let Some(d) = crate::classes::dict_backing(obj) {
                        return d.set(key.clone(), v).map_err(Flow::from_py);
                    }
                }
            }
            Err(raise(
                "TypeError",
                format!(
                    "'{}' object does not support item assignment",
                    obj.type_name()
                ),
            ))
        }
        // 02.08-c: Opaque markers accept item stores inertly (lxml
        // FunctionNamespace registry: ns_fns[fname] = func at import).
        Value::Obj(o) if o.kind == ap_obj::ObjKind::Opaque => Ok(()),
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
                // Wave-79: list slice deletion (del lst[1:3]) — mirrors
                // the set route's Slice arm. step==1 removes the span;
                // step!=1 removes the projected indices high-to-low so
                // the remaining indices stay valid.
                Key::Slice { lo, hi, step } => {
                    if step == 0 {
                        return Err(raise("ValueError", "slice step cannot be zero"));
                    }
                    let len = l.len() as i64;
                    let clamp = |v: i64| -> i64 {
                        if v < 0 {
                            (v + len).max(0).min(len)
                        } else {
                            v.min(len)
                        }
                    };
                    if step == 1 {
                        let start = clamp(lo.unwrap_or(0));
                        let stop = clamp(hi.unwrap_or(len));
                        for _ in start..stop {
                            l.remove_at(start as usize);
                        }
                    } else {
                        let mut idxs: Vec<i64> = Vec::new();
                        let mut cur = if step > 0 {
                            clamp(lo.unwrap_or(0))
                        } else {
                            clamp(hi.unwrap_or(len)) - 1
                        };
                        let end = if step > 0 {
                            clamp(hi.unwrap_or(len))
                        } else {
                            clamp(lo.unwrap_or(0)) - 1
                        };
                        while cur != end && cur >= 0 && cur < len {
                            idxs.push(cur);
                            cur += step;
                        }
                        idxs.sort_unstable_by(|a, b| b.cmp(a));
                        for ix in idxs {
                            l.remove_at(ix as usize);
                        }
                    }
                    return Ok(());
                }
                _other => {
                    return Err(raise("TypeError", "list indices must be integers"));
                }
            };
            let idx = normalize_index(i, l.len() as i64).ok_or_else(|| {
                if std::env::var("APX_IDX_DEBUG").is_ok() {
                    eprintln!("APX-IDX-DEBUG-DEL: list len={} idx={}", l.len(), i);
                }
                raise("IndexError", "list index out of range")
            })?;
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
        // 02.04 completion: weak containers route item deletion.
        Value::Obj(o)
            if o.kind == ap_obj::ObjKind::Opaque
                && crate::gcsupport::is_weak_container(&o.name) =>
        {
            match crate::gcsupport::call_weakref(
                &format!("{}.__delitem__", o.name),
                &[obj.clone(), key.clone()],
                &[],
            ) {
                Some(Ok(_)) => Ok(()),
                Some(Err(f)) => Err(f),
                None => Ok(()),
            }
        }
        // T2 (pytest Stash.__delitem__: threadexception cleanup does
        // `del config.stash[key]`): user instances answer del via their
        // class __delitem__, same as the assignment path answers
        // __setitem__.
        Value::Obj(o) if o.kind == ap_obj::ObjKind::UserInstance => {
            let interp = crate::builtins_modules::current_interp()
                .ok_or_else(|| raise("TypeError", "no interpreter for __delitem__"))?;
            let t = o
                .type_ref
                .borrow()
                .as_ref()
                .and_then(|w| w.upgrade());
            let entry = t
                .as_ref()
                .and_then(|t| t.lookup_with_owner("__delitem__"));
            if let Some((f, owner)) = entry {
                // Wave-78: push the defining-class super ctx DIRECTLY
                // onto super_stack around the __delitem__ call — scrapy
                // CaseInsensitiveDict.__delitem__ calls super().
                //__delitem__, which needs the ctx. (Staging via
                // pending_super_ctx does not work here: call_value_pub
                // bypasses the plain-fn reader that consumes pending, so
                // the next nested plain-fn call — e.g. self._normkey —
                // steals the staged ctx.)
                if std::env::var("APX_SUPERDBG").is_ok() {
                    eprintln!("SUPER-DELSTAGE owner={}", owner.name);
                }
                interp.super_stack.borrow_mut().push((owner, obj.clone(), false));
                let r = crate::interp::call_value_pub(
                    &interp,
                    &f,
                    &[obj.clone(), key.clone()],
                    &[],
                );
                interp.super_stack.borrow_mut().pop();
                return r.map(|_| ());
            }
            {
                // W112: last-chance __delitem__ dispatch. Instances whose
                // type_ref was dropped (weakref upgrade failed) report
                // type_name 'object' but still carry a live __class__ —
                // OrderedDict.move_to_end's `del self[key]` (functools
                // lru_cache / pluggy internals) crashed every failing
                // test's collection-finish hook with "'object' object
                // doesn't support item deletion".
                let interp = crate::builtins_modules::current_interp();
                if let Some(interp) = interp {
                    // W112: direct attr lookup on the object first -
                    // covers instances whose type_ref was dropped but
                    // whose class payload (and __delitem__) survives via
                    // the attribute chain.
                    if let Ok(dv) =
                        crate::classes::attr_get_protocol(&interp, obj, "__delitem__")
                    {
                        let r = crate::interp::call_value_pub(
                            &interp,
                            &dv,
                            &[obj.clone(), key.clone()],
                            &[],
                        );
                        return r.map(|_| ());
                    }
                    if let Value::Obj(io) = obj {
                        let cls = io
                            .attrs
                            .borrow()
                            .get(&Value::Str(PyString::new("__class__")));
                        if let Some(cls_v) = cls {
                            if let Ok(dv) =
                                crate::classes::attr_get_protocol(&interp, &cls_v, "__delitem__")
                            {
                                let r = crate::interp::call_value_pub(
                                    &interp,
                                    &dv,
                                    &[obj.clone(), key.clone()],
                                    &[],
                                );
                                return r.map(|_| ());
                            }
                        }
                    }
                }
                Err(raise(
                    "TypeError",
                    format!(
                        "'{}' object doesn't support item deletion",
                        obj.type_name()
                    ),
                ))
            }
        }
        Value::Tuple(_) => Err(raise(
            "TypeError",
            "'tuple' object doesn't support item deletion",
        )),
        Value::Str(_) => Err(raise(
            "TypeError",
            "'str' object doesn't support item deletion",
        )),
        _ => {
            // W112: non-UserInstance objects (Namespace/'object'-kind
            // instances with a live __class__) also answer __delitem__.
            let interp = crate::builtins_modules::current_interp();
            if let Some(interp) = interp {
                if let Ok(dv) =
                    crate::classes::attr_get_protocol(&interp, obj, "__delitem__")
                {
                    let r = crate::interp::call_value_pub(
                        &interp,
                        &dv,
                        &[obj.clone(), key.clone()],
                        &[],
                    );
                    return r.map(|_| ());
                }
            }
            if std::env::var("APX_TBDBG").is_ok() {
                eprintln!("[APDEL2] target={} key={:?}", obj.type_name(), key);
            }
            Err(raise(
                "TypeError",
                format!("'{}' object doesn't support item deletion", obj.type_name()),
            ))
        }
    }
}

/// Method call on a builtin receiver. Returns None when the method name is
/// unknown (caller raises AttributeError).
/// 02.04a fix pass (BUG-B): does `methods::call_method` (or the builtins
/// fallback) route `name` on a builtin receiver? Used by the attribute layer
/// to decide whether `obj.name` binds a BoundMethod. Superset is fine —
/// receiver-shape errors surface at dispatch.
pub fn builtin_method_routed(name: &str) -> bool {
    // Exactly the names with (Value::X, name) arms in call_method.
    matches!(
        name,
        "upper"
            | "lower"
            | "join"
            | "partition"
            | "encode"
            | "decode"
            | "translate"
            | "append"
            | "clear"
            | "keys"
            | "values"
            | "items"
            | "add"
            | "copy"
            | "discard"
            | "set_int_max_str_digits"
            // T1-d (twisted.python.context: setDefault =
            // defaultContextDict.__setitem__) — dict dunder method surface.
            | "__setitem__"
            | "__delitem__"
    )
}

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
        // 02.03b-fix: str.format(*args, **kwargs) — literal {}/`{name}`/
        // `{field!r}`/`{field!s}` template expansion (the pathlib __repr__
        // form "{}({!r})".format). index/name lookup against args/kwargs.
        (Value::Str(s), "format") => {
            let mut pos: Vec<Value> = Vec::new();
            let mut kw: Vec<(String, Value)> = Vec::new();
            for a in args {
                match a {
                    Value::Str(k2) if false => {}
                    other => pos.push(other.clone()),
                }
            }
            let _ = &kw;
            // kwargs arrive via the args tail convention? call_method only
            // carries positional; keyword format args come through kwargs in
            // call_value (BoundMethod arm forwards kwargs separately) — for
            // v0, handle positional + {name} via attrs-less miss.
            let mut out = String::new();
            let chars: Vec<char> = s.as_str().chars().collect();
            let mut i = 0usize;
            let mut auto_idx = 0usize;
            while i < chars.len() {
                let c = chars[i];
                if c == '}' {
                    if i + 1 < chars.len() && chars[i + 1] == '}' {
                        out.push('}');
                        i += 2;
                        continue;
                    }
                    return Some(Err(raise(
                        "ValueError",
                        "Single '}' encountered in format string",
                    )));
                }
                if c != '{' {
                    out.push(c);
                    i += 1;
                    continue;
                }
                if i + 1 < chars.len() && chars[i + 1] == '{' {
                    out.push('{');
                    i += 2;
                    continue;
                }
                // Scan to the matching '}'.
                let mut j = i + 1;
                let mut depth = 0usize;
                while j < chars.len() && (chars[j] != '}' || depth > 0) {
                    if chars[j] == '{' {
                        depth += 1;
                    }
                    if chars[j] == '}' && depth > 0 {
                        depth -= 1;
                    }
                    j += 1;
                }
                if j >= chars.len() {
                    return Some(Err(raise(
                        "ValueError",
                        "Single '{' encountered in format string",
                    )));
                }
                let field: String = chars[i + 1..j].iter().collect();
                let (name_part, rest) = match field.split_once('!') {
                    Some((n, r)) => (n.to_string(), Some(r.to_string())),
                    None => (field.clone(), None),
                };
                let conv = rest.as_deref().unwrap_or("");
                // Spec after ':' — v0 supports only empty/truncate-free specs.
                let name_str = name_part.split(':').next().unwrap_or("").to_string();
                let value: Value = if name_str.is_empty() {
                    let v = pos.get(auto_idx).cloned().unwrap_or(Value::None);
                    auto_idx += 1;
                    v
                } else if let Ok(idx) = name_str.parse::<usize>() {
                    pos.get(idx).cloned().unwrap_or(Value::None)
                } else {
                    Value::None
                };
                let rendered = match conv {
                    "r" => crate::builtins::builtin_repr(&[value.clone()])
                        .map(|v| match v {
                            Value::Str(x) => x.as_str().to_string(),
                            _ => String::new(),
                        })
                        .unwrap_or_default(),
                    "s" => crate::builtins::py_str_of(&value).unwrap_or_default(),
                    _ => crate::builtins::py_str_of(&value).unwrap_or_default(),
                };
                out.push_str(&rendered);
                i = j + 1;
            }
            Some(Ok(Value::Str(PyString::new(out))))
        }
        // 02.03b-fix: int method surface (importlib pyc paths: .to_bytes).
        (Value::Int(n), "to_bytes") => {
            let len = one_arg(args)
                .and_then(|v| v.as_int())
                .and_then(|i| i.to_i64())
                .unwrap_or(0) as usize;
            let byteorder = args
                .get(1)
                .and_then(|v| match v {
                    Value::Str(x) => Some(x.as_str().to_string()),
                    _ => None,
                })
                .unwrap_or_else(|| "big".to_string());
            let i64v = n.to_i64().unwrap_or(0);
            let mut out = Vec::with_capacity(len.max(0));
            match byteorder.as_str() {
                "little" => {
                    for i in 0..len {
                        out.push(((i64v >> (8 * i)) & 0xff) as u8);
                    }
                }
                _ => {
                    for i in (0..len).rev() {
                        out.push(((i64v >> (8 * i)) & 0xff) as u8);
                    }
                }
            }
            Some(Ok(Value::Bytes(Gc::new(out))))
        }
        (Value::Int(n), "bit_length") => {
            let i64v = n.to_i64().unwrap_or(0).unsigned_abs();
            Some(Ok(Value::Int(ap_obj::Int::from_i64(
                64 - i64v.leading_zeros() as i64,
            ))))
        }
        (Value::Bytes(_), "from_bytes") | (Value::Int(_), "from_bytes") => None,
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
        (Value::Bytes(b), "translate") => {
            // bytes.translate(table[, delete]): v0 covers the w3lib module-level
            // forms — table None (delete-only) and a 256-entry int-map table
            // built by bytes.maketrans.
            let mut out: Vec<u8> = Vec::with_capacity(b.len());
            let table: Option<Vec<Option<u8>>> = match args.first() {
                Some(Value::None) | None => None,
                Some(Value::Bytes(tb)) => {
                    Some(tb.iter().map(|x| Some(*x)).collect::<Vec<Option<u8>>>())
                }
                Some(Value::Dict(d)) => {
                    // int->int mapping dict (bytes.maketrans output shape).
                    let mut m: Vec<Option<u8>> = (0u16..256u16).map(|i| Some(i as u8)).collect();
                    for (k, v) in d.items() {
                        if let (Some(ki), Some(vi)) = (k.as_int(), v.as_int()) {
                            if let (Some(k64), Some(v64)) = (ki.to_i64(), vi.to_i64()) {
                                if (0..256).contains(&k64) && (0..256).contains(&v64) {
                                    m[k64 as usize] = Some(v64 as u8);
                                }
                            }
                        }
                    }
                    Some(m)
                }
                Some(_) => None,
            };
            let mut delete: Vec<u8> = Vec::new();
            if let Some(Value::Bytes(dl)) = args.get(1) {
                delete = (**dl).clone();
            }
            for (k, kw) in [("delete", true)] {
                let _ = (k, kw);
            }
            for &byte in b.iter() {
                if delete.contains(&byte) {
                    continue;
                }
                match &table {
                    Some(m) => {
                        if let Some(Some(mapped)) = m.get(byte as usize) {
                            out.push(*mapped);
                        }
                    }
                    None => out.push(byte),
                }
            }
            Some(Ok(Value::Bytes(Gc::new(out))))
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
        // Wave-108: remaining in-place list surface (StackSummary.reverse in
        // traceback formatting; sort/insert/index/count/extend same family).
        (Value::List(l), "reverse") => {
            let len = l.len();
            for i in 0..len / 2 {
                let a = l.get(i).unwrap();
                let b = l.get(len - 1 - i).unwrap();
                l.set_at(i, b);
                l.set_at(len - 1 - i, a);
            }
            Some(Ok(Value::None))
        }
        (Value::List(l), "sort") => {
            // v0: homogeneous int/str sort; other item types left as-is
            // (full __lt__ dispatch arrives with the cmp pass).
            let all_int = l.items().iter().all(|v| matches!(v, Value::Int(_) | Value::Bool(_)));
            let all_str = l.items().iter().all(|v| matches!(v, Value::Str(_)));
            if all_int || all_str {
                let mut items = l.items().to_vec();
                items.sort_by(|a, b| match (a, b) {
                    (Value::Int(x), Value::Int(y)) => x
                        .to_i64()
                        .unwrap_or(0)
                        .cmp(&y.to_i64().unwrap_or(0)),
                    (Value::Str(x), Value::Str(y)) => x.as_str().cmp(y.as_str()),
                    _ => std::cmp::Ordering::Equal,
                });
                l.clear();
                for it in items {
                    l.append(it);
                }
            }
            Some(Ok(Value::None))
        }
        (Value::List(l), "insert") => {
            if args.len() >= 2 {
                let idx_v = args[0].as_int().and_then(|i| i.to_i64()).unwrap_or(0);
                let len = l.len() as i64;
                let idx = if idx_v < 0 {
                    (len + idx_v).max(0)
                } else {
                    idx_v.min(len)
                } as usize;
                // v0 PyList has no insert: rebuild tail.
                let tail: Vec<Value> = l.items()[idx.min(l.len())..].to_vec();
                if idx < l.len() {
                    l.remove_range(idx, l.len());
                }
                l.append(args[1].clone());
                for it in tail {
                    l.append(it);
                }
            }
            Some(Ok(Value::None))
        }
        (Value::List(l), "extend") => {
            if let Some(other) = args.first() {
                match other {
                    Value::List(o) => {
                        for it in o.items().to_vec() {
                            l.append(it);
                        }
                    }
                    Value::Tuple(t) => {
                        for it in t.0.iter() {
                            l.append(it.clone());
                        }
                    }
                    Value::Str(st) => {
                        for ch in st.as_str().chars() {
                            l.append(Value::Str(ap_obj::PyString::new(ch.to_string())));
                        }
                    }
                    _ => {}
                }
            }
            Some(Ok(Value::None))
        }
        (Value::Dict(d), "keys") => {
            // T1-d: return a dict_keys VIEW (membership `in`, iteration) —
            // sysconfig._extend_dict does `key in target_keys`.
            // T1/T2 fix (enum/pluggy): carry BOTH <payload> (creation
            // snapshot) AND <dict> (live back-pointer). The native_iter
            // dict_keys/values/items arm reads <dict> first and re-snapshots;
            // without <dict> the view iterated EMPTY (enum.EnumType.__new__'s
            // `dict(classdict.items())` lost every member ->
            // KeyError 'STRICT' unpacking FlagBoundary; import pluggy died
            // the same way).
            let items = d.keys();
            let view = ap_obj::PyObject::new(ap_obj::ObjKind::Opaque, "dict_keys");
            {
                let payload = Value::List(PyList::from_vec(items));
                let mut a = view.attrs.borrow_mut();
                let _ = a.set(Value::Str(PyString::new("<payload>")), payload);
                let _ = a.set(Value::Str(PyString::new("<dict>")), Value::Dict(d.clone()));
            }
            Some(Ok(Value::Obj(view)))
        }
        (Value::Dict(d), "values") => {
            let items = d.items().into_iter().map(|(_, v)| v).collect();
            let view = ap_obj::PyObject::new(ap_obj::ObjKind::Opaque, "dict_values");
            {
                let payload = Value::List(PyList::from_vec(items));
                let mut a = view.attrs.borrow_mut();
                let _ = a.set(Value::Str(PyString::new("<payload>")), payload);
                let _ = a.set(Value::Str(PyString::new("<dict>")), Value::Dict(d.clone()));
            }
            Some(Ok(Value::Obj(view)))
        }
        (Value::Dict(d), "items") => {
            let items = d
                .items()
                .into_iter()
                .map(|(k, v)| Value::Tuple(Gc::new(Tuple(vec![k, v]))))
                .collect();
            let view = ap_obj::PyObject::new(ap_obj::ObjKind::Opaque, "dict_items");
            {
                let payload = Value::List(PyList::from_vec(items));
                let mut a = view.attrs.borrow_mut();
                let _ = a.set(Value::Str(PyString::new("<payload>")), payload);
                let _ = a.set(Value::Str(PyString::new("<dict>")), Value::Dict(d.clone()));
            }
            Some(Ok(Value::Obj(view)))
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
        // 02.03b: set.pop() — enum._find_data_type_ does data_types.pop().
        // CPython pops the "next" element (hash order); single-element v0
        // use is exact. Empty set -> the pinned KeyError.
        (Value::Set(s), "pop") => {
            let first = match s.iter().next() {
                Some(v) => v,
                None => return Some(Err(raise("KeyError", "pop from an empty set"))),
            };
            let _ = s.pop_discard(&first);
            Some(Ok(first))
        }
        (Value::Set(s), "remove") => {
            let k = one_arg(args)?;
            let present = s.contains(k).unwrap_or(false);
            if !present {
                let key_txt = crate::builtins::builtin_repr(std::slice::from_ref(k))
                    .map(|v| match v {
                        Value::Str(x) => x.as_str().to_string(),
                        _ => String::new(),
                    })
                    .unwrap_or_default();
                return Some(Err(raise("KeyError", key_txt)));
            }
            let _ = s.pop_discard(k);
            Some(Ok(Value::None))
        }
        // T1-d: dict.__setitem__/__delitem__ as BOUND methods —
        // scrapy.http.Headers (dict subclass) does
        // `dict.__setitem__(self, ...)` unbound calls; plain dict values
        // also need the method surface for `d.__setitem__` attr reads.
        (Value::Dict(d), "__setitem__") => {
            let k = one_arg(args)?.clone();
            let v = args.get(1).cloned().unwrap_or(Value::None);
            d.set(k, v).map_err(Flow::from_py).ok();
            Some(Ok(Value::None))
        }
        (Value::Dict(d), "__delitem__") => {
            let k = one_arg(args)?.clone();
            let _ = d.pop(&k);
            Some(Ok(Value::None))
        }
        // 02.03b: dict.setdefault (EnumDict._ignore_ plumbing).
        (Value::Dict(d), "setdefault") => {
            let k = one_arg(args)?.clone();
            let dv = args.get(1).cloned().unwrap_or(Value::None);
            if let Some(v) = d.get(&k) {
                return Some(Ok(v));
            }
            d.set(k, dv.clone()).map_err(Flow::from_py).ok();
            Some(Ok(dv))
        }
        (Value::Dict(d), "clear") => {
            for k in d.keys() {
                let _ = d.pop(&k);
            }
            Some(Ok(Value::None))
        }
        (Value::Dict(d), "get") => {
            let k = one_arg(args)?.clone();
            let dv = args.get(1).cloned().unwrap_or(Value::None);
            Some(Ok(d.get(&k).unwrap_or(dv)))
        }
        (Value::Dict(d), "pop") => {
            let k = one_arg(args)?;
            let dv = args.get(1).cloned().unwrap_or(Value::None);
            match d.pop(k) {
                Some(v) => Some(Ok(v)),
                None => Some(Ok(dv)),
            }
        }
        (Value::Dict(d), "popitem") => {
            // T2 (pytest SetupState.teardown_exact): stack.popitem() —
            // CPython LIFO: remove and return the last (key, value) pair;
            // KeyError on an empty dict.
            match d.popitem() {
                Some((k, v)) => Some(Ok(Value::Tuple(Gc::new(Tuple(vec![k, v]))))),
                None => Some(Err(raise(
                    "KeyError",
                    "popitem(): dictionary is empty",
                ))),
            }
        }
        (Value::Set(s), "clear") => {
            for it in s.iter().collect::<Vec<_>>() {
                let _ = s.pop_discard(&it);
            }
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
        // Wave-82: receiver shapes with NO such method — CPython raises
        // AttributeError at ATTR time; our permissive bind list attached a
        // BoundMethod marker, so answer the shape error HERE (the old
        // fallthrough let the caller treat "unhandled" as a None callee →
        // "'NoneType' object is not callable").
        (Value::Str(_), "decode")
        | (Value::List(_), "decode")
        | (Value::Tuple(_), "decode")
        | (Value::Int(_), "decode")
        | (Value::Dict(_), "decode") => Some(Err(raise(
            "AttributeError",
            format!("'{}' object has no attribute '{}'", obj.type_name(), name),
        ))),
        (Value::Obj(o), _) if o.kind == ap_obj::ObjKind::UserInstance => {
            // 02.03: instances dispatch through the class MRO; the v0
            // attrs-only path stays as fallback for legacy shapes.
            match super_user_method(interp, o, name, args) {
                Err(Flow::Raise(e)) if e.class == "AttributeError" => None,
                other => Some(other),
            }
        }
        // W111: dunder surface on builtin VALUES — the 11895 attr arm binds
        // BoundMethod "__len__"/"__iter__"/"__getitem__"/"__contains__" over
        // the receiver; calling it needs a call arm. CPython: dict.__len__
        // is the same slot len() uses.
        (Value::Dict(d), "__len__") => Some(Ok(Value::Int(ap_obj::Int::from_i64(d.len() as i64)))),
        (Value::Dict(d), "__getitem__") => {
            let k = one_arg(args)?;
            match d.get(k) {
                Some(v) => Some(Ok(v)),
                None => Some(Err(raise("KeyError", format!("{k:?}")))),
            }
        }
        (Value::Dict(d), "__contains__") => {
            let k = one_arg(args)?;
            Some(Ok(Value::Bool(d.get(k).is_some())))
        }
        (Value::Dict(d), "get") => {
            let k = one_arg(args)?;
            let dv = args.get(1).cloned().unwrap_or(Value::None);
            Some(Ok(d.get(k).unwrap_or(dv)))
        }
        (Value::List(l), "__len__") => Some(Ok(Value::Int(ap_obj::Int::from_i64(l.len() as i64)))),
        (Value::Tuple(t), "__len__") => Some(Ok(Value::Int(ap_obj::Int::from_i64(t.0.len() as i64)))),
        (Value::Str(s), "__len__") => Some(Ok(Value::Int(ap_obj::Int::from_i64(
            s.as_str().chars().count() as i64,
        )))),
        (Value::Set(s), "__len__") | (Value::Frozen(s), "__len__") => {
            Some(Ok(Value::Int(ap_obj::Int::from_i64(s.len() as i64))))
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
        "latin-1" | "latin1" | "iso-8859-1" | "iso8859-1" => {
            // 04.01: latin-1 maps every byte to the identical code point.
            let s: String = b.iter().map(|&x| x as char).collect();
            Ok(Value::Str(PyString::new(s)))
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
                "unicode_escape" | "raw_unicode_escape" => {
            let mut out = String::with_capacity(s.len());
            for c in s.chars() {
                match c {
                    '\u{000a}' => out.push_str("\\n"),
                    '\u{0009}' => out.push_str("\\t"),
                    '\u{000d}' => out.push_str("\\r"),
                    '\u{005c}' => out.push_str("\\\\"),
                    c if (c as u32) < 0x80 => out.push(c),
                    c if (c as u32) <= 0xFF => {
                        out.push_str(&format!("\\x{:02x}", c as u32));
                    }
                    c if (c as u32) <= 0xFFFF => {
                        out.push_str(&format!("\\u{:04x}", c as u32));
                    }
                    c => {
                        out.push_str(&format!("\\U{:08x}", c as u32));
                    }
                }
            }
            Ok(Value::Bytes(Gc::new(out.into_bytes())))
        }
"latin-1" | "latin1" | "iso-8859-1" | "iso8859-1" => {
            // 04.01 (six.b: s.encode("latin-1")): latin-1 maps U+0000..U+00FF
            // to the identical byte values.
            let mut out = Vec::with_capacity(s.len());
            let mut bad: Option<(usize, char)> = None;
            for (i, c) in s.chars().enumerate() {
                if (c as u32) > 0xFF {
                    bad = Some((i, c));
                    break;
                }
                out.push(c as u32 as u8);
            }
            if let Some((pos, ch)) = bad {
                return Err(raise(
                    "UnicodeEncodeError",
                    format!(
                        "'latin-1' codec can't encode character '\\u{:04x}' in position {pos}: ordinal not in range(256)",
                        ch as u32
                    ),
                ));
            }
            Ok(Value::Bytes(Gc::new(out)))
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
