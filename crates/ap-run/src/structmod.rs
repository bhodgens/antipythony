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
        "Struct",
    ] {
        let _ = ns.set(
            s(n),
            crate::builtins_modules::marker(&format!("struct.{n}")),
        );
    }
    finish(m, ns)
}

/// Entry from call_native("struct", ...) AND from the module markers.
pub(crate) fn call_struct(name: &str, args: &[Value]) -> Option<crate::builtins_modules::BResult> {
    // Module-level functions take fmt first; instance methods carry the
    // Opaque self (attrs `<fmt>`) first.
    let (fmt, rest): (String, &[Value]) = match args.first() {
        Some(Value::Obj(o)) if o.name == "struct.Struct" => {
            let f = o.attrs.borrow().get(&s("<fmt>")).and_then(|v| match v {
                Value::Str(x) => Some(x.as_str().to_string()),
                _ => None,
            });
            match f {
                Some(f) => (f, &args[1..]),
                None => return None,
            }
        }
        Some(Value::Str(f)) => (f.as_str().to_string(), &args[1..]),
        _ => return None,
    };
    if !matches!(name, "calcsize" | "pack" | "unpack") {
        if matches!(name, "pack_into" | "unpack_from") {
            return Some(Err(raise(
                "NotImplementedError",
                format!("ap: struct.{name} unsupported (v0 shim)"),
            )));
        }
        return None;
    }
    let r: crate::builtins_modules::BResult = (|| -> crate::builtins_modules::BResult {
        match name {
            "calcsize" => calcsize(&fmt),
            "pack" => pack(&fmt, rest),
            "unpack" => unpack(&fmt, rest),
            _ => unreachable!(),
        }
    })();
    Some(r)
}

fn strip_order(fmt: &str) -> (&str, bool) {
    if let Some(rest) = fmt.strip_prefix('<') {
        (rest, true)
    } else {
        (
            fmt.trim_start_matches(|c| matches!(c, '@' | '=' | '>' | '!')),
            false,
        )
    }
}

fn calcsize(fmt: &str) -> crate::builtins_modules::BResult {
    let (f, _) = strip_order(fmt);
    let mut total = 0usize;
    let mut chars = f.chars().peekable();
    while let Some(c) = chars.next() {
        let mut count_str = String::new();
        while let Some(d) = chars.peek() {
            if d.is_ascii_digit() {
                count_str.push(chars.next().unwrap());
            } else {
                break;
            }
        }
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
    let mut chars = f.chars();
    while let Some(c) = chars.next() {
        let mut count_str = String::new();
        while let Some(d) = chars.clone().next() {
            if d.is_ascii_digit() {
                count_str.push(chars.next().unwrap());
            } else {
                break;
            }
        }
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
    let mut chars = f.chars();
    while let Some(c) = chars.next() {
        let mut count_str = String::new();
        while let Some(d) = chars.clone().next() {
            if d.is_ascii_digit() {
                count_str.push(chars.next().unwrap());
            } else {
                break;
            }
        }
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
        let mut attrs = o.attrs.borrow_mut();
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
                "size" | "format" => {
                    let fmt = o.attrs.borrow().get(&s("<fmt>")).map(|v| v.clone());
                    return Some(fmt.unwrap_or(Value::None));
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
