//! Leaf 10.04: the ce->runtime value map — where ap-run's ce_bridge
//! attaches the RUNTIME-side instance Value for a CePayload::CeInstance
//! (keyed by the payload address). C handles stay in ap-ce; the runtime
//! Value pointer crosses only here.

use std::collections::HashMap;
use std::sync::Mutex;

static MAP: Mutex<Option<HashMap<usize, ap_obj::value::Value>>> = Mutex::new(None);

/// Attach the runtime Value for a ce instance payload key (called by
/// ap-run's ce_bridge right after alloc).
pub fn set_runtime_value(key: usize, v: ap_obj::value::Value) {
    let mut g = MAP.lock().unwrap_or_else(|p| p.into_inner());
    g.get_or_insert_with(HashMap::new).insert(key, v);
}

/// Resolve the runtime Value for a ce instance payload key (None when the
/// bridge never attached one — e.g. a handle minted by C alone).
pub fn get_runtime_value(key: usize) -> Option<ap_obj::value::Value> {
    let g = MAP.lock().unwrap_or_else(|p| p.into_inner());
    g.as_ref().and_then(|m| m.get(&key)).cloned()
}

/// to_value arm: resolve + REMOVE the entry (the map is a handoff, not a
/// store — one Value::Obj per CeInstance handle consumption; entries
/// re-attach on every bridge call that re-mints the handle).
pub fn take_runtime_value(key: usize) -> Option<ap_obj::value::Value> {
    let mut g = MAP.lock().unwrap_or_else(|p| p.into_inner());
    g.as_mut().and_then(|m| m.remove(&key))
}
