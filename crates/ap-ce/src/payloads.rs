//! Leaf 10.04 (S1 item 2): C-visible instance payloads (the pydantic-core
//! requirement).
//!
//! Payload model decision (tradeoffs recorded in
//! docs/plan10-1004-s1-bridge-report.md): a SIDE REGISTRY here, keyed by
//! the runtime object's stable address (Arc::as_ptr of the instance's
//! Gc<PyObject>), with tp_basicsize-sized malloc'd blocks allocated at
//! tp_new time by ap-run's ce_bridge.
//!
//! Rejected alternative: an opaque pointer slot directly ON the CeObject
//! handle. That would make payloads visible only while a handle exists —
//! but handles are MINTED PER CALL (from_value), so a handle-slot payload
//! would either be re-minted empty on every method/getset call (losing
//! the C struct) or force the bridge to thread a persistent per-instance
//! handle everywhere. The side registry decouples C payload lifetime from
//! handle minting: any handle for the same runtime object resolves to the
//! same block (via the payload_key carried in CePayload::Instance).
//!
//! Known limitation (honest gap): blocks are freed only at process exit —
//! the GC cycle collector does not know about them, and a freed runtime
//! object whose address gets recycled re-allocates (the stale block
//! leaks, bounded by construction sites). tp_dealloc wiring to
//! `payload_take` is the S1 follow-up.

use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::Mutex;

struct PayloadSlot {
    spec: usize,
    /// Raw block pointer held as usize so the slot is Send (the pointer is
    /// never dereferenced through this table without re-materialization).
    data: usize,
    basicsize: usize,
}

static SLOTS: Mutex<Option<HashMap<usize, PayloadSlot>>> = Mutex::new(None);

/// Allocate (zeroed) the C payload block for a runtime object. Called by
/// ap-run's ce_bridge right after instance allocation when the spec's
/// tp_basicsize > 0. Re-attachment for the same key re-initializes the
/// entry (the old block leaks — address-recycling case, see module doc).
pub fn payload_alloc(key: usize, spec: usize, basicsize: usize) -> *mut c_void {
    // SAFETY: the libc allocator has no preconditions beyond a non-zero
    // size (the .max(1) guard pins that); the returned block is owned by the
    // ce payload registry and released only through the matching free seam.
    let data = unsafe { libc::malloc(basicsize.max(1)) };
    if !data.is_null() {
        // SAFETY: data was just allocated with exactly basicsize.max(1) bytes
        unsafe {
            std::ptr::write_bytes(data as *mut u8, 0, basicsize.max(1));
        }
    }
    let mut g = SLOTS.lock().unwrap_or_else(|p| p.into_inner());
    g.get_or_insert_with(HashMap::new).insert(
        key,
        PayloadSlot {
            spec,
            data: data as usize,
            basicsize,
        },
    );
    data
}

/// The payload block for a runtime object (NULL when none was attached —
/// e.g. instances of non-ce classes, or specs with basicsize 0).
pub fn payload_of(key: usize) -> *mut c_void {
    let g = SLOTS.lock().unwrap_or_else(|p| p.into_inner());
    g.as_ref()
        .and_then(|m| m.get(&key))
        .map(|s| s.data as *mut c_void)
        .unwrap_or(std::ptr::null_mut())
}

/// Is `key` attached to a payload of spec `spec`?
pub fn payload_is_spec(key: usize, spec: usize) -> bool {
    let g = SLOTS.lock().unwrap_or_else(|p| p.into_inner());
    g.as_ref()
        .and_then(|m| m.get(&key))
        .map(|s| s.spec == spec)
        .unwrap_or(false)
}

/// Detach + free (the future tp_dealloc route; exposed for tests and the
/// S1 dealloc wiring).
pub fn payload_take(key: usize) {
    let mut g = SLOTS.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(m) = g.as_mut() {
        if let Some(s) = m.remove(&key) {
            // SAFETY: s.data came from payload_alloc's malloc route and the slot is
            unsafe { libc::free(s.data as *mut c_void) };
        }
    }
}
