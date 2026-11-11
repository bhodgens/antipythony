//! 02.09a P4/P5: the `CellSlot` handle (`Arc<RefCell<Option<Value>>>`) with
//! a staged Send+Sync marker. See ap_obj::markers + the int.rs contract
//! block: the interior RefCell is touched only under the GIL v1 protocol /
//! the owning table's Mutex; refcount traffic is Arc (Decision 1). Orphan
//! rules forbid marking `RefCell`, hence the Deref newtype.

use std::ops::Deref;
use std::sync::Arc;

use ap_obj::Value;

pub struct CellSlotHandle(pub Arc<std::cell::RefCell<Option<Value>>>);

impl Deref for CellSlotHandle {
    type Target = std::cell::RefCell<Option<Value>>;
    fn deref(&self) -> &std::cell::RefCell<Option<Value>> {
        &self.0
    }
}

impl Clone for CellSlotHandle {
    fn clone(&self) -> Self {
        CellSlotHandle(self.0.clone())
    }
}

unsafe impl Send for CellSlotHandle {}
unsafe impl Sync for CellSlotHandle {}

impl CellSlotHandle {
    pub fn new(v: Option<Value>) -> Self {
        CellSlotHandle(Arc::new(std::cell::RefCell::new(v)))
    }
}
