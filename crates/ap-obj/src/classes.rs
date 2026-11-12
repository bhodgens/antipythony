//! Real class objects (leaf 02.03): `TypeObject` + instances-with-class.
//!
//! A user class is a `PyObject(ObjKind::UserClass)` whose `type_obj` field
//! carries the [`TypeObject`]: name, bases, the C3-linearized MRO (computed
//! once at class creation, S-03-15), the class-attribute dict, and flags
//! (`__slots__` names, has-dict). Instances of user classes are
//! `PyObject(ObjKind::UserInstance)` with `type_obj` pointing at the SAME
//! class object (identity: `x.__class__ is C`, S-03-31) plus a per-instance
//! dict (the compact `PyDict`).
//!
//! MRO: `mro(C) = C + merge(mro(B1)..mro(Bn), (B1..Bn))` (S-03-15) — the
//! classic C3 merge; failure raises the byte-exact CPython TypeError text
//! (handled in ap-run's class-creation path, which owns Flow).
//!
//! Exception classes: the builtin hierarchy (BaseException <- Exception <- …)
//! is ALSO modeled as TypeObjects (flagged `EXC`), so `except E` matching and
//! user `class MyError(Exception)` share ONE isinstance rule (S-03-31/32).
//! The old name-table `exc_hier::is_subclass` remains for the raise/str
//! plumbing that predates classes.

use crate::dict::PyDict;
use crate::int::Gc;
use crate::object::PyObject;
use crate::value::Value;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::RwLock;

/// Per-class flags.
#[derive(Debug, Clone, Default)]
pub struct TypeFlags {
    /// `__slots__` declared (instances have no `__dict__` unless "__dict__"
    /// is itself a slot member or a base provides one — S-03-13/14).
    pub slots: Option<Vec<String>>,
    /// The class participates in the exception hierarchy (derives from
    /// BaseException) — raise/catch machinery keys on this.
    pub is_exception: bool,
    /// Built-in exception class (not user-defined): __module__ == "builtins".
    pub builtin_exc: bool,
}

/// The class payload behind a `ObjKind::UserClass` / exception type object.
pub struct TypeObject {
    /// Simple name (S-03-04 `__name__`).
    pub name: String,
    /// Dotted qualname (S-03-04 `__qualname__`; slot, not a dict key).
    pub qualname: String,
    /// Direct bases as type-object handles (empty => `[object]`).
    pub bases: Vec<Arc<TypeRef>>,
    /// C3 linearization, computed at creation: [self, ..., object].
    /// 02.09a P4 write-once freeze: the field is constructed FULL inside
    /// `create` (the self-reference is spliced into the local Vec before
    /// the Arc even exists), so post-construction it is immutable — the
    /// Mutex is read-optimized (contended reads take the lock briefly and
    /// clone nothing; no write site exists after `create` returns). The
    /// audit's "MRO patching" mutator never existed in this tree (bases/
    /// MRO immutable post-creation per surgery_version's doc row).
    pub mro: std::sync::Mutex<Vec<Arc<TypeRef>>>,
    /// Class-attribute namespace (`C.__dict__` reads go through a
    /// mappingproxy- façade in ap-run; the raw dict lives here).
    pub dict: Gc<PyDict>,
    pub flags: TypeFlags,
    /// The metaclass type-object handle (type(C)); None == plain `type`.
    /// 02.09b P7 (decision 2): Mutex — resolved/finalized after creation
    /// (02.03) and read on EVERY instantiation by every worker.
    pub metaclass: std::sync::RwLock<Option<Arc<TypeRef>>>,
    /// Back-pointer to the class's PyObject Value (set right after
    /// creation; identity for `x.__class__ is C`).
    /// 02.09b P7 (decision 2): Mutex (set-once after creation; read per
    /// instantiation).
    pub class_value: RwLock<Option<Value>>,
    /// 02.05b Risk 4 (docs/ic-version-audit.md "Type version" domain, audit
    /// rows 11+16): per-class version counter bumped when the class's
    /// attribute surface mutates (`type.__setattr__` — method add/remove,
    /// the hot AttrIC invalidator). Atomic per the 02.09 entry-23 epoch
    /// pattern (Relaxed — ANY observed change invalidates the guarded IC;
    /// no ordering consumer). Seeded 1.
    pub type_version: std::sync::atomic::AtomicU64,
    /// 02.05b Risk 4 ("Class-surgery version" domain, audit row 10): bumped
    /// alongside `type_version` when class STRUCTURE changes. In this
    /// runtime bases/MRO are immutable post-creation (C3 computed once at
    /// `create`), so today the only surgery surface IS the namespace —
    /// both counters bump together at the same sites; the separation lets a
    /// tier-1 guard distinguish them if base surgery ever lands. Seeded 1.
    pub surgery_version: std::sync::atomic::AtomicU64,
}

/// A shared handle to a class. `Arc<TypeObject>` behind the same Gc pattern
/// the rest of ap-obj uses (the handle lives inside a PyObject's type_obj
/// slot; identity comparisons compare Arc pointers).
pub type TypeRef = TypeObject;

impl TypeObject {
    /// Create a class: computes the C3 MRO or reports the failure reason.
    /// `bases` must already be type handles; empty => object-rooted.
    pub fn create(
        name: String,
        qualname: String,
        bases: Vec<Arc<TypeRef>>,
        dict: Gc<PyDict>,
        flags: TypeFlags,
        metaclass: Option<Arc<TypeRef>>,
    ) -> Result<Arc<TypeRef>, MroError> {
        // Duplicate base check (S-03-15, raised during base processing).
        for i in 0..bases.len() {
            for j in (i + 1)..bases.len() {
                if Arc::ptr_eq(&bases[i], &bases[j]) {
                    return Err(MroError::DuplicateBase(bases[i].name.clone()));
                }
            }
        }
        let mut mro = linearize(&name, &bases)?;
        let rc = Arc::new(TypeObject {
            name,
            qualname,
            bases,
            // 02.09a P4: seeded empty, filled via the lock BELOW before any
            // other thread can observe the Arc (the self-reference splice).
            mro: std::sync::Mutex::new(Vec::new()),
            dict,
            flags,
            metaclass: std::sync::RwLock::new(metaclass),
            class_value: std::sync::RwLock::new(None),
            type_version: std::sync::atomic::AtomicU64::new(1),
            surgery_version: std::sync::atomic::AtomicU64::new(1),
        });
        // The class's OWN mro starts with itself (S-03-15): splice after
        // construction (self-reference through the Arc).
        mro.insert(0, rc.clone());
        *rc.mro.lock().unwrap() = mro;
        Ok(rc)
    }

    /// First class in the MRO whose dict contains `name` (S-03-09 scan).
    ///
    /// A4 (memperf audit 2026-11-06): `PyDict::get_str` probes with the
    /// bare `&str` — the old path minted a fresh `Gc<PyString>` (heap alloc)
    /// PER MRO ENTRY per lookup, and the `__del__` probe in ap-gc paid it
    /// for every garbage instance on every collect.
    pub fn lookup(&self, name: &str) -> Option<Value> {
        let mro = self.mro.lock().unwrap();
        for t in mro.iter() {
            if let Some(v) = t.dict.get_str(name) {
                return Some(v);
            }
        }
        None
    }

    /// The class-of record for a name PLUS the defining class (needed for
    /// descriptor `__get__(obj, owner)` binding and super() positioning).
    pub fn lookup_with_owner(&self, name: &str) -> Option<(Value, Arc<TypeRef>)> {
        let mro = self.mro.lock().unwrap();
        for t in mro.iter() {
            if let Some(v) = t.dict.get_str(name) {
                return Some((v, t.clone()));
            }
        }
        None
    }

    /// Is `self` a subclass of (or equal to) `other`? MRO membership (S-03-31).
    pub fn is_subclass(&self, other: &Arc<TypeObject>) -> bool {
        self.mro
            .lock()
            .unwrap()
            .iter()
            .any(|t| Arc::ptr_eq(t, other))
    }

    /// Instance dict enabled? (no __slots__, or __dict__ in slots, or a
    /// base has a dict — S-03-13/14).
    pub fn has_instance_dict(&self) -> bool {
        match &self.flags.slots {
            None => true,
            Some(slots) => {
                slots.iter().any(|s| s == "__dict__")
                    // A base without __slots__ gives every subclass a dict.
                    || self.mro.lock().unwrap().iter().skip(1).any(|t| {
                        // The object root never confers a dict by itself.
                        t.name != "object"
                            && (t.flags.slots.is_none()
                                || t.flags
                                    .slots
                                    .as_ref()
                                    .is_some_and(|s: &Vec<String>| s.iter().any(|x| x == "__dict__")))
                    })
            }
        }
    }

    /// Is `name` a declared slot on this class chain? (only meaningful when
    /// the instance has no dict).
    pub fn is_slot(&self, name: &str) -> bool {
        self.mro.lock().unwrap().iter().any(|t| {
            t.flags
                .slots
                .as_ref()
                .is_some_and(|s: &Vec<String>| s.iter().any(|x| x == name))
        })
    }

    /// 02.05b Risk 4: the max class-surgery version across the MRO (audit
    /// row 10 — the AttrIC chain guard). A method added on ANY MRO entry
    /// invalidates caches keyed on this; the chain carries the counter, not
    /// the cache site. Relaxed reads (02.09 epoch-pattern contract).
    pub fn chain_surgery_version(&self) -> u64 {
        self.mro
            .lock()
            .unwrap()
            .iter()
            .map(|t| t.class_version())
            .max()
            .unwrap_or(1)
    }

    /// 02.05b Risk 4: the surgery counter of THIS class payload (audit row
    /// 10 — AttrIC chain guard component).
    pub fn class_version(&self) -> u64 {
        self.surgery_version
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// 02.05b Risk 4: bump BOTH counters (rows 10/11/16 — the only live
    /// mutation path today is namespace surgery, which is both a type
    /// -surface and a class-structure change; bases/MRO are immutable
    /// post-creation so no other bump site exists). Relaxed fetch_add per
    /// the 02.09 entry-23 pattern. `chain_surgery_version` of every
    /// subclass sees the change through its MRO sweep.
    pub fn bump_class_versions(&self) {
        self.type_version
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.surgery_version
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
}

#[cfg(test)]
mod version_tests {
    use super::*;

    fn mkclass(name: &str) -> Arc<TypeRef> {
        TypeObject::create(
            name.to_string(),
            name.to_string(),
            vec![],
            PyDict::new(),
            TypeFlags::default(),
            None,
        )
        .expect("acyclic")
    }

    /// 02.05b: counters seeded at 1; a surgery bump moves BOTH counters and
    /// the subclass chain sees it through `chain_surgery_version`.
    #[test]
    fn counters_seed_and_bump_through_chain() {
        // 02.09a P4: the MRO field is write-once (filled inside `create`);
        // the test seeds a custom chain through a CHILD-of construct —
        // build it via create(parent) instead of mutating mro directly.
        let parent = mkclass("P");
        let grand = mkclass("O2");
        let child = TypeObject::create(
            "C".to_string(),
            "C".to_string(),
            vec![parent.clone(), grand.clone()],
            PyDict::new(),
            TypeFlags::default(),
            None,
        )
        .expect("acyclic");
        assert_eq!(child.chain_surgery_version(), 1);
        let before = parent
            .type_version
            .load(std::sync::atomic::Ordering::Relaxed);
        parent.bump_class_versions();
        assert_eq!(
            parent
                .type_version
                .load(std::sync::atomic::Ordering::Relaxed),
            before + 1
        );
        assert_eq!(child.chain_surgery_version(), 2);
        assert!(child.chain_surgery_version() > 1);
    }

    /// The dict-version domain: `PyDict::mutation_epoch` bumps on the
    /// store/pop surface (single source of truth — audit rule 4).
    #[test]
    fn dict_epoch_is_the_dict_version() {
        let d = PyDict::new();
        let e0 = d.mutation_epoch();
        d.set(Value::Str(crate::PyString::new("k")), Value::None)
            .unwrap();
        let e1 = d.mutation_epoch();
        assert!(e1 > e0);
        d.pop(&Value::Str(crate::PyString::new("k")));
        assert!(d.mutation_epoch() > e1);
    }

    /// Builtins-version domain seeds at 1 and is monotonic under bumps.
    #[test]
    fn builtins_version_monotonic() {
        let v0 = crate::versioning::builtins_version();
        crate::versioning::bump_builtins_version();
        assert!(crate::versioning::builtins_version() > v0);
    }
}

/// C3 failure modes with the CPython error text split out (ap-run raises).
#[derive(Debug)]
pub enum MroError {
    DuplicateBase(String),
    Inconsistent { bases: String },
}

/// C3 merge (S-03-15): `mro(C) = C + merge(mro(B1)..(Bn), (B1..Bn))`.
fn linearize(_name: &str, bases: &[Arc<TypeRef>]) -> Result<Vec<Arc<TypeRef>>, MroError> {
    let mut seqs: Vec<Vec<Arc<TypeRef>>> = bases
        .iter()
        .map(|b| b.mro.lock().unwrap().clone())
        .collect();
    seqs.push(bases.to_vec());
    let mut out: Vec<Arc<TypeRef>> = Vec::new();
    loop {
        seqs.retain(|s| !s.is_empty());
        if seqs.is_empty() {
            break;
        }
        // Find a head that appears in no tail.
        let mut found: Option<Arc<TypeRef>> = None;
        'outer: for s in &seqs {
            let head = &s[0];
            for t in &seqs {
                if t[1..].iter().any(|x| Arc::ptr_eq(x, head)) {
                    continue 'outer;
                }
            }
            found = Some(head.clone());
            break;
        }
        let Some(head) = found else {
            let names: Vec<String> = bases.iter().map(|b| b.name.clone()).collect();
            return Err(MroError::Inconsistent {
                bases: names.join(", "),
            });
        };
        out.push(head.clone());
        for s in &mut seqs {
            s.retain(|x| !Arc::ptr_eq(x, &head));
        }
    }
    Ok(out)
}

/// The MRO error text for `class X(A, B)` (S-03-15/16 byte-exact form).
pub fn mro_error_message(e: &MroError) -> String {
    match e {
        MroError::DuplicateBase(n) => format!("duplicate base class {n}"),
        MroError::Inconsistent { bases } => {
            format!("Cannot create a consistent method resolution order (MRO) for bases {bases}")
        }
    }
}

/// Descriptor kinds the runtime understands (S-03-20/21). All live as
/// `PyObject(ObjKind::Descriptor)` values with one of these payloads in
/// `descr_kind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DescrKind {
    Property,
    ClassMethod,
    StaticMethod,
    /// A user class instance whose TYPE defines `__get__` (data iff its type
    /// also defines `__set__`/`__delete__`).
    UserDescr,
}

/// A class statement executed at runtime creates:
/// 1. the `PyObject(UserClass)` value (identity of the class),
/// 2. a `TypeObject` payload, stored OUT of band (the executor maps
///    class-object address -> TypeRef in `Interpreter::class_table`) because
///    PyObject payloads must stay field-stable for 02.04's GC move design.
///
/// Convenience: build the class Value from a created TypeRef. Wires BOTH
/// directions (payload -> Value back-pointer, Value -> payload).
pub fn make_class_value(type_ref: Arc<TypeRef>) -> Value {
    // 02.09b P7: class_ref/class_value are RwLock (write-once at creation;
    // read-mostly everywhere else — read() reentrancy keeps recursive attr
    // probes on the same class deadlock-free).
    let o = PyObject::new(crate::ObjKind::UserClass, type_ref.name.clone());
    *o.class_ref.write().unwrap_or_else(|p| p.into_inner()) = Some(type_ref.clone());
    let v = Value::Obj(o);
    // 02.09b P7 (audit O1): install-if-None compare-exchange — two workers
    // lazily materializing the same class must land on ONE Value (identity:
    // `x.__class__ is C` and ptr-keyed caches). The winner installs; the
    // loser adopts the existing Value and drops its fresh copy (the
    // write-once pattern mro's freeze already uses).
    let mut existing = type_ref
        .class_value
        .write()
        .unwrap_or_else(|p| p.into_inner());
    if existing.is_none() {
        *existing = Some(v.clone());
    }
    let adopted = existing.clone().unwrap_or_else(|| v);
    adopted
}
