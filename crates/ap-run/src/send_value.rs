//! 02.09a P5 (SPLIT-02.09.md): the `Send`-only successor of the retired
//! `retired unsafe carrier <T>` blanket-unsafety wrapper.
//! `Value` and every `Gc<T>` payload are `!Sync` by construction (RefCell
//! interiors). Statics that share those payloads across threads therefore
//! cannot claim `Sync` merely because refcounts are atomic — `Send` says
//! "safe to move across threads" (what the gc-registration channel and the
//! worker reclaim queue need), while `Sync` would additionally claim
//! "safe to reference from many threads", which these payloads only satisfy
//! under a lock the compiler cannot see.
//!
//! Contract for `SendValue`/`SendBox`: every access goes through a
//! process-global `Mutex` (the same protocol the GIL provided — this is
//! the census P4 rule "no `retired unsafe carrier ` may outlive the GIL" honored with
//! real lock semantics instead of an unchecked blanket impl). 02.09-b
//! replaces these crossings with typed ownership transfers; until then the
//! mutex discipline is the audited seam.

use ap_obj::Value;

/// A `Value` legally transferred across threads. NOT `Sync` — deref only
/// behind the owning table's mutex.
pub struct SendValue(pub Value);

// SAFETY: `Value` is a plain data enum over `Arc` handles; moving it across
// threads copies the 16 bytes and bumps/drops atomic refcounts. No
// thread-local state is reachable through the move itself. Consumers are
// required (contract above) to touch the interior only under the owning
// table's mutex, which preserves the happens-before the old GIL provided.
unsafe impl Send for SendValue {}

/// A boxed payload legally transferred across threads (worker reclaim
/// queue). Same move-safety argument as `SendValue`; same mutex contract.
pub struct SendBox(pub Box<dyn std::any::Any>);

unsafe impl Send for SendBox {}
