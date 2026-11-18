//! Host-side `$str` reader (leaf 03.05): render an in-module `$str` struct
//! for the host (aprt.abort_msg payload). Layout mirrors the emitter
//! (crates/ap-codegen-wasm types.rs):
//!   $str:   hash(0, i64) kind(1, i32) len(2, i32) data(3, $byte_arr)
//!   $str16: + data16(4, $i16_arr)   $str32: + data32(4, $i32_arr)
//! kind 0 = latin1/bytes (data), 1 = UTF-16 (data16), 2 = UTF-32 (data32).
//!
//! SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

use wasmtime::{AnyRef, ArrayRef, AsContext, AsContextMut, Val};

/// Field indices in the `$str` struct family (emitter types.rs F_STR_*).
#[allow(dead_code)]
const F_STR_HASH: u32 = 0;
const F_STR_KIND: u32 = 1;
#[allow(dead_code)]
const F_STR_LEN: u32 = 2;
const F_STR_DATA: u32 = 3;
const F_STR16_DATA: u32 = 4;
const F_STR32_DATA: u32 = 4;

/// Render a `$str`-family struct (passed as `AnyRef` — the locked 03.02
/// deviation: abort_msg's declared param is anyref because wasmtime
/// host-func matching cannot express the self-referential `(ref $str)`)
/// into a Rust String. Non-struct refs render as a diagnostic placeholder
/// rather than panicking — abort_msg is the unrecoverable-error path and
/// must never trap recursively.
pub fn render_str<T: 'static>(mut store: impl AsContextMut<Data = T>, r: &AnyRef) -> String {
    let rooted = match r.as_struct(store.as_context_mut()) {
        Ok(Some(s)) => s,
        _ => return "<aprt: abort payload not a structref>".to_string(),
    };
    let fields: Vec<Val> = match rooted.fields(&mut store) {
        Ok(it) => it.collect(),
        Err(_) => return "<aprt: abort payload unreadable>".to_string(),
    };
    if fields.len() <= F_STR_DATA as usize {
        return "<aprt: abort payload short>".to_string();
    }
    let kind = match &fields[F_STR_KIND as usize] {
        Val::I32(k) => *k,
        _ => return "<aprt: abort payload bad kind>".to_string(),
    };
    match kind {
        0 => bytes_to_string(&fields[F_STR_DATA as usize], &mut store),
        1 => u16_slice_to_string(&fields[F_STR16_DATA as usize], &mut store),
        2 => u32_slice_to_string(&fields[F_STR32_DATA as usize], &mut store),
        _ => "<aprt: abort payload unknown kind>".to_string(),
    }
}

fn array_ref<T: 'static>(
    v: &Val,
    store: &mut impl AsContextMut<Data = T>,
) -> Option<wasmtime::Rooted<ArrayRef>> {
    let Val::AnyRef(Some(any)) = v else {
        return None;
    };
    any.as_array(store.as_context_mut()).ok().flatten()
}

fn bytes_to_string<T: 'static>(v: &Val, store: &mut impl AsContextMut<Data = T>) -> String {
    let Some(arr) = array_ref(v, store) else {
        return "<aprt: abort payload data not an array>".to_string();
    };
    let len = arr.len(store.as_context()).unwrap_or(0);
    let mut bytes = Vec::with_capacity(len as usize);
    for i in 0..len {
        match arr.get(&mut *store, i) {
            Ok(Val::I32(b)) => bytes.push(b as u8),
            _ => return "<aprt: abort payload byte read failed>".to_string(),
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

fn u16_slice_to_string<T: 'static>(v: &Val, store: &mut impl AsContextMut<Data = T>) -> String {
    let Some(arr) = array_ref(v, store) else {
        return "<aprt: abort payload data not an array>".to_string();
    };
    let len = arr.len(store.as_context()).unwrap_or(0);
    let mut units = Vec::with_capacity(len as usize);
    for i in 0..len {
        match arr.get(&mut *store, i) {
            Ok(Val::I32(u)) => units.push(u as u16),
            _ => return "<aprt: abort payload unit read failed>".to_string(),
        }
    }
    String::from_utf16_lossy(&units)
}

fn u32_slice_to_string<T: 'static>(v: &Val, store: &mut impl AsContextMut<Data = T>) -> String {
    let Some(arr) = array_ref(v, store) else {
        return "<aprt: abort payload data not an array>".to_string();
    };
    let len = arr.len(store.as_context()).unwrap_or(0);
    let mut chars = Vec::with_capacity(len as usize);
    for i in 0..len {
        match arr.get(&mut *store, i) {
            Ok(Val::I32(c)) => match char::from_u32(c as u32) {
                Some(c) => chars.push(c),
                None => return "<aprt: abort payload bad scalar>".to_string(),
            },
            _ => return "<aprt: abort payload scalar read failed>".to_string(),
        }
    }
    chars.into_iter().collect()
}
