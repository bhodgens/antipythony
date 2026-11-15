//! struct (04.01): minimal surface for pure-python third-party imports.
//!
//! six.py needs `struct.Struct(">B").pack(n)` at module scope (its `int2byte`).
//! `struct` itself was OUT-of-v1 (S-05 family); this shim serves exactly the
//! third-party import gate and FAILS LOUD (NotImplementedError naming the gap)
//! beyond the fixed-width scalar formats. Wire formats: big-endian default
//! (`>`/`!`/`@`/`=`), `<` little-endian; scalars b/B/h/H/i/I/l/L/q/Q/?/x.

use ap_obj::int::Gc;
use ap_obj::value::{PyString, Tuple, Value};

use crate::builtins_modules::{finish, new_module, raise, s};

pub(crate) fn build_struct(_i: &crate::frame::Interpreter) -> Value {
    let (m, ns) = new_module("struct");
    for n in [
        "pack",
        "unpack",
        "calcsize",
        "pack_into",
        "unpack_from",
        "iter_unpack",
        "Struct",
    ] {
        let _ = ns.set(
            s(n),
            crate::builtins_modules::marker(&format!("struct.{n}")),
        );
    }
    let _ = ns.set(
        s("error"),
        crate::builtins_modules::interned_type_pub("struct.error"),
    );
    finish(m, ns)
}

/// Entry from call_native("struct", ...) AND from the module markers.
pub(crate) fn call_struct(name: &str, args: &[Value]) -> Option<crate::builtins_modules::BResult> {
    // Module-level functions take fmt first; instance methods carry the
    // Opaque self (attrs `<fmt>`) first.
    let (fmt, rest): (String, &[Value]) = match args.first() {
        Some(Value::Obj(o)) if o.name == "struct.Struct" => {
            let f = o
                .attrs
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .get(&s("<fmt>"))
                .and_then(|v| match v {
                    Value::Str(x) => Some(x.as_str().to_string()),
                    _ => None,
                });
            match f {
                Some(f) => (f, &args[1..]),
                None => return None,
            }
        }
        Some(Value::Str(f)) => (f.as_str().to_string(), &args[1..]),
        // T1-d (zipfile): formats are bytes literals (b"<4s4H2LH").
        Some(Value::Bytes(b)) => match std::str::from_utf8(b) {
            Ok(fs) => (fs.to_string(), &args[1..]),
            Err(_) => return None,
        },
        _ => return None,
    };
    let name = name.rsplit('.').next().unwrap_or(name); // Struct.<op> markers
    if !matches!(
        name,
        "calcsize" | "pack" | "unpack" | "pack_into" | "unpack_from" | "iter_unpack"
    ) {
        return None;
    }
    let r: crate::builtins_modules::BResult = (|| -> crate::builtins_modules::BResult {
        match name {
            "calcsize" => calcsize(&fmt),
            "pack" => pack(&fmt, rest),
            "unpack" => unpack(&fmt, rest),
            // s036: pack_into(fmt, buffer, offset, *vals) — pack then write
            // at offset (buffer must be a bytearray-sized list-backed obj).
            "pack_into" => {
                let buf = rest
                    .first()
                    .ok_or_else(|| raise("TypeError", "pack_into needs a buffer"))?;
                // 02.08 (s036) fix: layout is pack_into(fmt, buf, offset,
                // *vals) — offset is rest[1] (the old rest[2] read the
                // FIRST VALUE as the offset and pushed 258 zero bytes).
                let off = match rest.get(1) {
                    Some(Value::Int(n)) => n.to_i64().unwrap_or(0).max(0) as usize,
                    _ => 0,
                };
                let vals: Vec<Value> = rest.iter().skip(2).cloned().collect();
                let packed = pack_bytes(&fmt, &vals)?;
                // 02.08 gap (s036): bytearray IS the CPython-required
                // pack_into target. Mutate in place via the D1
                // bytearray_mutate helper (clone-modify-set for aliased
                // buffers — byte-identical either way).
                if matches!(buf, Value::Obj(bo)
                    if bo.kind == ap_obj::ObjKind::Opaque && bo.name == "bytearray")
                {
                    let packed2 = packed.clone();
                    return crate::builtins_modules::bytearray_mutate_pub(buf, |data| {
                        while data.len() < off + packed2.len() {
                            data.push(0);
                        }
                        data[off..off + packed2.len()].copy_from_slice(&packed2);
                        Ok(())
                    })
                    .map(|_| Value::None);
                }
                if let Value::Obj(bo) = buf {
                    if let Some(Value::List(l)) = bo
                        .attrs
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .get(&s("<data>"))
                    {
                        while l.len() < off + packed.len() {
                            l.append(Value::Int(ap_obj::Int::from_i64(0)));
                        }
                        for (k, b) in packed.iter().enumerate() {
                            l.set_at(off + k, Value::Int(ap_obj::Int::from_i64(*b as i64)));
                        }
                        return Ok(Value::None);
                    }
                }
                Err(raise("TypeError", "pack_into buffer must be a bytearray"))
            }
            // s036: unpack_from(fmt, buffer, offset=0)
            "unpack_from" => {
                let buf = rest
                    .first()
                    .ok_or_else(|| raise("TypeError", "unpack_from needs a buffer"))?;
                // 02.08 (s036): rest excludes fmt — offset is rest[1]
                // (the old rest[2] read the first value past the offset).
                let off = match rest.get(1) {
                    Some(Value::Int(n)) => n.to_i64().unwrap_or(0).max(0) as usize,
                    _ => 0,
                };
                let data = buffer_bytes(buf)?;
                let sub = &data[off.min(data.len())..];
                unpack_bytes(&fmt, sub)
            }
            // s036: iter_unpack(fmt, buffer) — repeated unpack over the whole
            // buffer; size must divide evenly.
            "iter_unpack" => {
                // 02.08 (s036): rest excludes fmt — buffer is rest.first().
                let buf = rest
                    .first()
                    .ok_or_else(|| raise("TypeError", "iter_unpack needs a buffer"))?;
                let data = buffer_bytes(buf)?;
                let size = match calcsize(&fmt)? {
                    Value::Int(n) => n.to_i64().unwrap_or(0).max(0) as usize,
                    _ => 0,
                };
                if size == 0 {
                    return Err(raise("struct.error", "iter_unpack: size is 0"));
                }
                if data.len() % size != 0 {
                    return Err(raise(
                        "struct.error",
                        "iter_unpack: length must be a multiple of the struct size",
                    ));
                }
                let mut out: Vec<Value> = Vec::new();
                for chunk in data.chunks(size) {
                    out.push(unpack_bytes(&fmt, chunk)?);
                }
                Ok(Value::List(ap_obj::value::PyList::from_vec(out)))
            }
            _ => unreachable!(),
        }
    })();
    Some(r)
}

/// Pack to raw bytes (shared by pack + pack_into).
type BResultV<T> = Result<T, crate::Flow>;
fn pack_bytes(fmt: &str, vals: &[Value]) -> BResultV<Vec<u8>> {
    match pack(fmt, vals)? {
        Value::Bytes(b) => Ok(b.iter().cloned().collect()),
        _ => Err(raise("SystemError", "pack produced non-bytes")),
    }
}

/// Bytes of a bytes-like buffer argument (bytes/bytearray).
fn buffer_bytes(buf: &Value) -> BResultV<Vec<u8>> {
    match buf {
        Value::Bytes(b) => Ok(b.iter().cloned().collect()),
        Value::Obj(o) => {
            // 02.08 (s036): bytearray payloads live under <data> as
            // Value::Bytes (Bytes-backed ctor); legacy list-backed
            // shape still supported.
            let attrs = o.attrs.lock().unwrap_or_else(|p| p.into_inner());
            match attrs.get(&s("<data>")) {
                Some(Value::Bytes(b)) => Ok(b.as_ref().clone()),
                Some(Value::List(l)) => {
                    let mut out = Vec::new();
                    for it in l.items().iter() {
                        if let Value::Int(n) = it {
                            out.push(n.to_i64().unwrap_or(0) as u8);
                        }
                    }
                    Ok(out)
                }
                _ => Err(raise("TypeError", "expected a bytes-like object")),
            }
        }
        _ => Err(raise("TypeError", "expected a bytes-like object")),
    }
}

/// Unpack to a tuple Value from raw bytes (shared by unpack/unpack_from/
/// iter_unpack).
fn unpack_bytes(fmt: &str, data: &[u8]) -> BResultV<Value> {
    unpack(fmt, &[Value::Bytes(Gc::new(data.to_vec()))])
}

fn strip_order(fmt: &str) -> (&str, bool) {
    if let Some(rest) = fmt.strip_prefix('<') {
        (rest, true)
    } else {
        (fmt.trim_start_matches(['@', '=', '>', '!']), false)
    }
}

fn calcsize(fmt: &str) -> crate::builtins_modules::BResult {
    let (f, _) = strip_order(fmt);
    let mut total = 0usize;
    let mut chars = f.chars().peekable();
    loop {
        // Repeat count precedes the format char (s036: ">2h").
        let mut count_str = String::new();
        while let Some(d) = chars.peek() {
            if d.is_ascii_digit() {
                count_str.push(chars.next().unwrap());
            } else {
                break;
            }
        }
        let Some(c) = chars.next() else {
            break;
        };
        let count: usize = if count_str.is_empty() {
            1
        } else {
            match count_str.parse() {
                Ok(n) => n,
                Err(_) => {
                    return Err(raise(
                        "struct.error",
                        format!("bad repeat count in format {fmt:?}"),
                    ))
                }
            }
        };
        let unit = match c {
            'x' | 'b' | 'B' | 'c' | '?' => 1usize,
            'h' | 'H' => 2,
            'i' | 'I' | 'l' | 'L' | 'f' => 4,
            'q' | 'Q' | 'd' | 'n' | 'N' => 8,
            // T1-d (zipfile): 's' (bytes) and 'p' (pascal string) count
            // toward the size with their repeat count.
            's' | 'p' => 1, // repeat count in `count` handles the size
            'P' => 8,
            _ => {
                return Err(raise(
                    "NotImplementedError",
                    format!("ap: struct.calcsize format char {c:?} unsupported (v0 shim)"),
                ))
            }
        };
        total += unit * count;
    }
    Ok(Value::Int(ap_obj::Int::from_i64(total as i64)))
}

fn scalar_width(c: char) -> Option<usize> {
    match c {
        'b' | 'B' | '?' | 'c' => Some(1usize),
        'h' | 'H' => Some(2),
        'i' | 'I' | 'l' | 'L' | 'f' => Some(4),
        'q' | 'Q' | 'd' | 'n' | 'N' => Some(8),
        _ => None,
    }
}

fn pack(fmt: &str, vals: &[Value]) -> crate::builtins_modules::BResult {
    let (f, little) = strip_order(fmt);
    let mut out: Vec<u8> = Vec::new();
    let mut vi = 0usize;
    let mut chars = f.chars().peekable();
    loop {
        // Repeat count precedes the format char (s036: ">2h").
        let mut count_str = String::new();
        while let Some(d) = chars.clone().next() {
            if d.is_ascii_digit() {
                count_str.push(chars.next().unwrap());
            } else {
                break;
            }
        }
        let Some(c) = chars.next() else {
            break;
        };
        let count: usize = if count_str.is_empty() {
            1
        } else {
            count_str.parse().unwrap_or(1)
        };
        for _ in 0..count {
            match c {
                'x' => out.push(0),
                _ => {
                    let width = match scalar_width(c) {
                        Some(w) => w,
                        None => {
                            return Err(raise(
                                "NotImplementedError",
                                format!("ap: struct.pack format char {c:?} unsupported (v0 shim)"),
                            ))
                        }
                    };
                    let n: i128 = match vals.get(vi) {
                        Some(Value::Int(x)) => x.to_i64().map(|v| v as i128).unwrap_or(0),
                        Some(Value::Bool(b)) => *b as i64 as i128,
                        other => {
                            return Err(raise(
                                "TypeError",
                                format!(
                                    "struct.pack expected an int for {c:?}, got {}",
                                    other.map(|v| v.type_name().to_string()).unwrap_or_default()
                                ),
                            ))
                        }
                    };
                    vi += 1;
                    // 02.08 (s036): CPython-exact range checks — struct.error
                    // "requires 0 <= number <= 2**k-1" for unsigned formats,
                    // "-2**(k-1) <= number <= 2**(k-1)-1" for signed.
                    // (lo, hi, hi_override): Q's 2**64-1 exceeds i128's
                    // displayable shift chain — hi_override carries the
                    // CPython-exact decimal for the message; the RANGE
                    // test uses hi (n <= 2**64-1 fits i128's 2**127).
                    let (lo, hi, hi_override): (i128, i128, Option<&str>) = match c {
                        'b' => (-(1i128 << 7), (1i128 << 7) - 1, None),
                        'B' => (0, (1i128 << 8) - 1, None),
                        'h' => (-(1i128 << 15), (1i128 << 15) - 1, None),
                        'H' => (0, (1i128 << 16) - 1, None),
                        'i' | 'l' => (-(1i128 << 31), (1i128 << 31) - 1, None),
                        'I' | 'L' => (0, (1i128 << 32) - 1, None),
                        'q' | 'n' | 'N' => (-(1i128 << 63), (1i128 << 63) - 1, None),
                        'Q' => (0, (1i128 << 64) - 1, Some("18446744073709551615")),
                        _ => (i128::MIN, i128::MAX, None),
                    };
                    if !(lo..=hi).contains(&n) {
                        let hi_s = hi_override.map(|s| s.to_string()).unwrap_or_else(|| hi.to_string());
                        let what = if lo == 0 {
                            format!("'{}' format requires 0 <= number <= {hi_s}", c)
                        } else {
                            format!("'{}' format requires {lo} <= number <= {hi_s}", c)
                        };
                        return Err(raise("struct.error", what));
                    }
                    let be = n.to_be_bytes();
                    let field = &be[16 - width.min(16)..];
                    if little {
                        let mut rev = field.to_vec();
                        rev.reverse();
                        out.extend_from_slice(&rev);
                    } else {
                        out.extend_from_slice(field);
                    }
                }
            }
        }
    }
    Ok(Value::Bytes(Gc::new(out)))
}

fn unpack(fmt: &str, vals: &[Value]) -> crate::builtins_modules::BResult {
    let (f, little) = strip_order(fmt);
    let expect = match calcsize(fmt)? {
        Value::Int(x) => x.to_i64().unwrap_or(0) as usize,
        _ => 0,
    };
    let data: Vec<u8> = match vals.first() {
        Some(Value::Bytes(b)) => b.iter().cloned().collect(),
        Some(Value::Str(st)) => st.as_str().bytes().collect(),
        _ => {
            return Err(raise(
                "TypeError",
                "struct.unpack expected a bytes-like object",
            ))
        }
    };
    if data.len() != expect {
        return Err(raise(
            "struct.error",
            format!("unpack requires a buffer of {expect} bytes"),
        ));
    }
    let mut out: Vec<Value> = Vec::new();
    let mut off = 0usize;
    let mut chars = f.chars().peekable();
    loop {
        // Repeat count precedes the format char (s036: ">2h").
        let mut count_str = String::new();
        while let Some(d) = chars.clone().next() {
            if d.is_ascii_digit() {
                count_str.push(chars.next().unwrap());
            } else {
                break;
            }
        }
        let Some(c) = chars.next() else {
            break;
        };
        let count: usize = if count_str.is_empty() {
            1
        } else {
            count_str.parse().unwrap_or(1)
        };
        for _ in 0..count {
            if c == 'x' {
                off += 1;
                continue;
            }
            let width = match scalar_width(c) {
                Some(w) => w,
                None => {
                    return Err(raise(
                        "NotImplementedError",
                        format!("ap: struct.unpack format char {c:?} unsupported (v0 shim)"),
                    ))
                }
            };
            if off + width > data.len() {
                return Err(raise("struct.error", "unpack buffer too short"));
            }
            let field = &data[off..off + width];
            off += width;
            let mut be = [0u8; 16];
            if little {
                let mut rev = field.to_vec();
                rev.reverse();
                be[16 - width..].copy_from_slice(&rev);
            } else {
                be[16 - width..].copy_from_slice(field);
            }
            let v = i128::from_be_bytes(be);
            let v = match width {
                1 if matches!(c, 'b') => v as i8 as i128,
                2 if matches!(c, 'h') => v as i16 as i128,
                4 if matches!(c, 'i' | 'l') => v as i32 as i128,
                8 if matches!(c, 'q' | 'n') => v as i64 as i128,
                _ => v,
            };
            out.push(int_from_i128(v));
        }
    }
    Ok(Value::Tuple(Gc::new(Tuple(out))))
}

fn int_from_i128(v: i128) -> Value {
    match i64::try_from(v) {
        Ok(x) => Value::Int(ap_obj::Int::from_i64(x)),
        Err(_) => Value::Int(ap_obj::Int::from_big(big_from_i128(v))),
    }
}

fn big_from_i128(v: i128) -> num_bigint::BigInt {
    num_bigint::BigInt::from(v)
}

// Marker factory: struct.Struct(fmt) returns the Opaque carrying <fmt>;
// built in builtins_modules (needs its `marker` helper) — see call arm
// "struct" => structmod::struct_ctor.
pub(crate) fn struct_ctor(args: &[Value]) -> Option<crate::builtins_modules::BResult> {
    let fmt = match args.first() {
        Some(Value::Str(f)) => f.as_str().to_string(),
        Some(other) => {
            return Some(Err(raise(
                "TypeError",
                format!(
                    "Struct() argument must be a bytes-like object, not {}",
                    other.type_name()
                ),
            )))
        }
        None => {
            return Some(Err(raise(
                "TypeError",
                "Struct() missing required argument 'format'",
            )))
        }
    };
    let o = ap_obj::PyObject::new(ap_obj::ObjKind::Opaque, "struct.Struct");
    {
        let attrs = o.attrs.lock().unwrap_or_else(|p| p.into_inner());
        let _ = attrs.set(s("<fmt>"), Value::Str(PyString::new(fmt)));
    }
    Some(Ok(Value::Obj(o)))
}

pub(crate) fn struct_attr(obj: &Value, name: &str) -> Option<Value> {
    if let Value::Obj(o) = obj {
        if o.kind == ap_obj::ObjKind::Opaque && o.name == "struct.Struct" {
            let mname = match name {
                "pack" | "unpack" | "pack_into" | "unpack_from" => {
                    format!("struct.{name}")
                }
                "format" => {
                    let fmt = o
                        .attrs
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .get(&s("<fmt>"));
                    return Some(fmt.unwrap_or(Value::None));
                }
                "size" => {
                    // s036: Struct(fmt).size = calcsize(fmt).
                    let sz = o
                        .attrs
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .get(&s("<fmt>"))
                        .and_then(|v| match v {
                            Value::Str(x) => Some(x.as_str().to_string()),
                            _ => None,
                        })
                        .map(|f| calcsize(&f))
                        .and_then(|r| r.ok())
                        .unwrap_or(Value::Int(ap_obj::Int::from_i64(0)));
                    return Some(sz);
                }
                "iter_unpack" => {
                    let bm = ap_obj::PyObject::with_self(
                        ap_obj::ObjKind::BoundMethod,
                        "struct.Struct.iter_unpack",
                        obj.clone(),
                    );
                    return Some(Value::Obj(bm));
                }
                "calcsize" => {
                    return Some(crate::builtins_modules::marker("struct.Struct.calcsize"))
                }
                _ => return None,
            };
            let bm = ap_obj::PyObject::with_self(ap_obj::ObjKind::BoundMethod, mname, obj.clone());
            return Some(Value::Obj(bm));
        }
    }
    None
}
