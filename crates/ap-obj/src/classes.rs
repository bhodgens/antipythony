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
use std::cell::RefCell;
use std::rc::Rc;

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
    pub bases: Vec<Rc<TypeRef>>,
    /// C3 linearization, computed at creation: [self, ..., object].
    /// RefCell because the self-reference lands after construction.
    pub mro: std::cell::RefCell<Vec<Rc<TypeRef>>>,
    /// Class-attribute namespace (`C.__dict__` reads go through a
    /// mappingproxy- façade in ap-run; the raw dict lives here).
    pub dict: Gc<PyDict>,
    pub flags: TypeFlags,
    /// The metaclass type-object handle (type(C)); None == plain `type`.
    /// RefCell: resolved/finalized after creation (02.03).
    pub metaclass: std::cell::RefCell<Option<Rc<TypeRef>>>,
    /// Back-pointer to the class's PyObject Value (set right after
    /// creation; identity for `x.__class__ is C`).
    pub class_value: RefCell<Option<Value>>,
}

/// A shared handle to a class. `Rc<TypeObject>` behind the same Gc pattern
/// the rest of ap-obj uses (the handle lives inside a PyObject's type_obj
/// slot; identity comparisons compare Rc pointers).
pub type TypeRef = TypeObject;

impl TypeObject {
    /// Create a class: computes the C3 MRO or reports the failure reason.
    /// `bases` must already be type handles; empty => object-rooted.
    pub fn create(
        name: String,
        qualname: String,
        bases: Vec<Rc<TypeRef>>,
        dict: Gc<PyDict>,
        flags: TypeFlags,
        metaclass: Option<Rc<TypeRef>>,
    ) -> Result<Rc<TypeRef>, MroError> {
        // Duplicate base check (S-03-15, raised during base processing).
        for i in 0..bases.len() {
            for j in (i + 1)..bases.len() {
                if Rc::ptr_eq(&bases[i], &bases[j]) {
                    return Err(MroError::DuplicateBase(bases[i].name.clone()));
                }
            }
        }
        let mut mro = linearize(&name, &bases)?;
        let rc = Rc::new(TypeObject {
            name,
            qualname,
            bases,
            mro: std::cell::RefCell::new(Vec::new()),
            dict,
            flags,
            metaclass: std::cell::RefCell::new(metaclass),
            class_value: std::cell::RefCell::new(None),
        });
        // The class's OWN mro starts with itself (S-03-15): splice after
        // construction (self-reference through the Rc).
        mro.insert(0, rc.clone());
        *rc.mro.borrow_mut() = mro;
        Ok(rc)
    }

    /// First class in the MRO whose dict contains `name` (S-03-09 scan).
    pub fn lookup(&self, name: &str) -> Option<Value> {
        for t in self.mro.borrow().iter() {
            if let Some(v) = t.dict.get(&Value::Str(crate::PyString::new(name))) {
                return Some(v);
            }
        }
        None
    }

    /// The class-of record for a name PLUS the defining class (needed for
    /// descriptor `__get__(obj, owner)` binding and super() positioning).
    pub fn lookup_with_owner(&self, name: &str) -> Option<(Value, Rc<TypeRef>)> {
        for t in self.mro.borrow().iter() {
            if let Some(v) = t.dict.get(&Value::Str(crate::PyString::new(name))) {
                return Some((v, t.clone()));
            }
        }
        None
    }

    /// Is `self` a subclass of (or equal to) `other`? MRO membership (S-03-31).
    pub fn is_subclass(&self, other: &Rc<TypeObject>) -> bool {
        self.mro.borrow().iter().any(|t| Rc::ptr_eq(t, other))
    }

    /// Instance dict enabled? (no __slots__, or __dict__ in slots, or a
    /// base has a dict — S-03-13/14).
    pub fn has_instance_dict(&self) -> bool {
        match &self.flags.slots {
            None => true,
            Some(slots) => {
                slots.iter().any(|s| s == "__dict__")
                    // A base without __slots__ gives every subclass a dict.
                    || self.mro.borrow().iter().skip(1).any(|t| {
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
        self.mro.borrow().iter().any(|t| {
            t.flags
                .slots
                .as_ref()
                .is_some_and(|s: &Vec<String>| s.iter().any(|x| x == name))
        })
    }
}

/// C3 failure modes with the CPython error text split out (ap-run raises).
#[derive(Debug)]
pub enum MroError {
    DuplicateBase(String),
    Inconsistent { bases: String },
}

/// C3 merge (S-03-15): `mro(C) = C + merge(mro(B1)..(Bn), (B1..Bn))`.
fn linearize(_name: &str, bases: &[Rc<TypeRef>]) -> Result<Vec<Rc<TypeRef>>, MroError> {
    let mut seqs: Vec<Vec<Rc<TypeRef>>> = bases.iter().map(|b| b.mro.borrow().clone()).collect();
    seqs.push(bases.to_vec());
    let mut out: Vec<Rc<TypeRef>> = Vec::new();
    loop {
        seqs.retain(|s| !s.is_empty());
        if seqs.is_empty() {
            break;
        }
        // Find a head that appears in no tail.
        let mut found: Option<Rc<TypeRef>> = None;
        'outer: for s in &seqs {
            let head = &s[0];
            for t in &seqs {
                if t[1..].iter().any(|x| Rc::ptr_eq(x, head)) {
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
            s.retain(|x| !Rc::ptr_eq(x, &head));
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
pub fn make_class_value(type_ref: Rc<TypeRef>) -> Value {
    let o = PyObject::new(crate::ObjKind::UserClass, type_ref.name.clone());
    *o.class_ref.borrow_mut() = Some(type_ref.clone());
    let v = Value::Obj(o);
    *type_ref.class_value.borrow_mut() = Some(v.clone());
    v
}
