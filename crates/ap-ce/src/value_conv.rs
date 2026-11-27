//! Handle -> Value reconstruction. The forward direction (Value -> handle)
//! lives in state::from_value; this module owns the reverse, including the
//! Instance arm that rebuilds a real UserInstance against the registered
//! heap type (so isinstance/attr protocol see the real class).

use crate::state::{CeKind, CePayload};
use ap_obj::dict::PyDict;
use ap_obj::int::{Gc, Int};
use ap_obj::value::{PyList, PyString, Tuple, Value};

/// Handle -> Value (borrowing: payloads clone out, Arcs bump).
/// Returns None for a NULL handle (caller decides what that means; C
/// convention is "error already set" or an explicit API contract).
pub fn to_value(h: crate::state::CeHandle) -> Option<Value> {
    if h.is_null() {
        return None;
    }
    // SAFETY: h is non-null (checked above) and a live minted ce handle
    // (Box::into_raw'd, never freed), so the deref is valid; the payload RefCell
    // borrow is short (worker-serialization protocol, the markers.rs claim).
    unsafe {
        let o = &*h;
        let p = o.payload.borrow();
        match p.as_ref()? {
            CePayload::Int(i) => Some(Value::Int(i.clone())),
            CePayload::Float(x) => Some(Value::Float(*x)),
            CePayload::Bool(b) => {
                if o.ob_type == CeKind::None {
                    Some(Value::None)
                } else {
                    Some(Value::Bool(*b))
                }
            }
            CePayload::Str(s) => Some(Value::Str(PyString::new(s.clone()))),
            CePayload::BytesMarker(b) => Some(Value::Bytes(b.clone())),
            CePayload::SetMarker(st, frozen) => {
                if *frozen {
                    Some(Value::Frozen(st.clone()))
                } else {
                    Some(Value::Set(st.clone()))
                }
            }
            CePayload::Tuple(items) => {
                let vals: Vec<Value> = items.iter().filter_map(|h| to_value(*h)).collect();
                Some(Value::Tuple(Gc::new(Tuple(vals))))
            }
            CePayload::List(items) => {
                let vals: Vec<Value> = items.iter().filter_map(|h| to_value(*h)).collect();
                Some(Value::List(PyList::from_vec(vals)))
            }
            CePayload::Dict(d) => Some(Value::Dict(d.clone())),
            CePayload::Module { name, ns } => {
                let m = ap_obj::PyObject::new(ap_obj::ObjKind::Module, name.clone());
                *m.module_ns.lock().unwrap_or_else(|p| p.into_inner()) =
                    Some(std::sync::Arc::new((**ns).clone()));
                Some(Value::Obj(m))
            }
            CePayload::CFunction { module, name, .. } => {
                // Round-trips as the dotted marker the runtime dispatches
                // (spike.add_two etc.) — equality by marker name.
                Some(crate::state::marker_for(module, name))
            }
            CePayload::Type { spec_index } => {
                let v = crate::slots::class_value_for_spec(*spec_index)?;
                Some(v)
            }
            CePayload::Instance { class_spec, state } => {
                let t = crate::slots::type_ref_for_spec(*class_spec)?;
                let io = ap_obj::PyObject::new(ap_obj::ObjKind::UserInstance, t.name.clone());
                *(io.type_ref.write().unwrap_or_else(|p| p.into_inner())) =
                    Some(std::sync::Arc::downgrade(&t));
                *(io.class_ref.write().unwrap_or_else(|p| p.into_inner())) = Some(t.clone());
                for (k, v) in state.items() {
                    let _ = io
                        .attrs
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .set(k.clone(), v.clone());
                }
                Some(Value::Obj(io))
            }
            CePayload::CeInstance { payload_key, .. } => {
                // Leaf 10.04: OUR tp_new-route instance — the runtime-side
                // UserInstance lives in the ce->runtime value map under the
                // payload key (attached by ap-run's ce_bridge at alloc time).
                Some(crate::value_map::take_runtime_value(*payload_key)?)
            }
            CePayload::Opaque(v) => Some(Value::Obj(v.clone())),
        }
    }
}

/// The instance dict of a CeHandle::Instance (bridges setattr from the
/// runtime side into C-visible state when needed later).
pub fn instance_state(h: crate::state::CeHandle) -> Option<Gc<PyDict>> {
    // SAFETY: caller precondition — h is a live minted ce handle (the C callers
    // null-check per the apx header contract; handles are Box::into_raw'd and
    // never freed), so the deref is valid; the payload borrow is short.
    unsafe {
        let o = &*h;
        match o.payload.borrow().as_ref()? {
            CePayload::Instance { state, .. } => Some(state.clone()),
            _ => None,
        }
    }
}

/// int payload read (PyLong_AsSsize_t route in the dispatcher).
pub fn as_i64(h: crate::state::CeHandle) -> Option<i64> {
    // SAFETY: caller precondition — h is a live minted ce handle (the C callers
    // null-check per the apx header contract; handles are Box::into_raw'd and
    // never freed), so the deref is valid; the payload borrow is short.
    unsafe {
        let o = &*h;
        match o.payload.borrow().as_ref()? {
            CePayload::Int(i) => i.to_i64(),
            CePayload::Bool(b) => Some(if *b { 1 } else { 0 }),
            _ => None,
        }
    }
}

/// Int::from_i64 shorthand re-export for the FFI layer.
pub fn int_from_i64(n: i64) -> Int {
    Int::from_i64(n)
}
