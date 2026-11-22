//! PyType_FromSpec machinery: spec tables -> real ap-obj heap types with
//! slot dispatch back into C function pointers.
//!
//! This is THE kill-criterion surface (survey kill #2 / RustPython PR
//! #8435): a FromSpec-built type must
//!   1. produce a real class Value (UserClass + TypeObject payload) so
//!      isinstance/attr/MRO protocol work natively,
//!   2. carry slot entries (tp_new / tp_init / tp_methods / tp_getset) that
//!      dispatch to C function pointers with CPython argument semantics.
//!
//! Mechanism: from_spec records the spec in a process-global table and
//! builds the TypeObject whose class dict holds MARKER callables — dotted
//! BuiltinFn values named `__ap_ce__.<spec>#<slot>` for slots and
//! `__ap_ce__.<spec>#m<i>` for methods. ap-run's ce_bridge recognizes the
//! `__ap_ce__.` prefix, parses the spec index + entry, and calls the C
//! pointer with converted args. tp_new/tp_init flow through the normal
//! instantiate() path because that path calls any callable Value (not just
//! user functions).

use std::sync::{Arc, Mutex};

use ap_obj::classes::{make_class_value, TypeFlags, TypeObject, TypeRef};
use ap_obj::dict::PyDict;
use ap_obj::int::Gc;
use ap_obj::object::PyObject;
use ap_obj::value::{PyString, Value};

/// A PyMethodDef-style entry (C side passes the raw struct; we normalize).
#[derive(Clone, Copy)]
pub struct SlotFn(pub crate::ffi_types::PyCFunctionRaw);

/// One registered type spec.
pub struct SpecRecord {
    pub name: String,
    /// The C slot table snapshot, normalized.
    pub tp_new: Option<SlotFn>,
    pub tp_init: Option<SlotFn>,
    pub tp_dealloc: Option<SlotFn>,
    /// (name, flags, fn) in declaration order.
    pub methods: Vec<(String, i32, SlotFn)>,
    /// (name, getter, setter) in declaration order.
    pub getsets: Vec<(String, Option<SlotFn>, Option<SlotFn>)>,
    /// The built class Value (set once by from_spec).
    pub class_value: Option<Value>,
    /// The TypeObject payload handle.
    pub type_ref: Option<Arc<TypeRef>>,
}

impl SpecRecord {
    fn empty(name: &str) -> Self {
        SpecRecord {
            name: name.to_string(),
            tp_new: None,
            tp_init: None,
            tp_dealloc: None,
            methods: Vec::new(),
            getsets: Vec::new(),
            class_value: None,
            type_ref: None,
        }
    }
}

/// The spec registry (index-stable; PyType_FromSpec appends).
pub fn spec_table() -> &'static Mutex<Vec<SpecRecord>> {
    static T: Mutex<Vec<SpecRecord>> = Mutex::new(Vec::new());
    &T
}

/// Register a spec and build the heap type. Returns the spec index.
/// `tp_new`/`tp_init`/`tp_dealloc` arrive as raw C fns; methods/getsets as
/// normalized triples. `bases` are class Values (None => object).
pub fn from_spec(
    name: &str,
    tp_new: Option<SlotFn>,
    tp_init: Option<SlotFn>,
    tp_dealloc: Option<SlotFn>,
    methods: Vec<(String, i32, SlotFn)>,
    getsets: Vec<(String, Option<SlotFn>, Option<SlotFn>)>,
    bases: Vec<Value>,
) -> Result<usize, String> {
    // 1. Resolve bases to TypeRef payloads (empty => object-rooted).
    let base_refs: Vec<Arc<TypeRef>> = bases
        .iter()
        .filter_map(|b| {
            if let Value::Obj(o) = b {
                if o.kind == ap_obj::ObjKind::UserClass {
                    return o
                        .class_ref
                        .read()
                        .unwrap_or_else(|p| p.into_inner())
                        .clone();
                }
            }
            None
        })
        .collect();

    // 2. Build the TypeObject payload. The bare name (post-dot) is what
    // type(x).__name__ reports (CPython: spec name "spike.Point" ->
    // __name__ "Point", __module__ "spike").
    let bare = name.rsplit_once('.').map(|(_, b)| b).unwrap_or(name);
    let t = TypeObject::create(
        bare.to_string(),
        bare.to_string(),
        base_refs,
        PyDict::new(),
        TypeFlags::default(),
        None,
    )
    .map_err(|e| ap_obj::classes::mro_error_message(&e))?;
    let _ = t.dict.set(
        ap_obj::value::Value::Str(ap_obj::value::PyString::new("__module__")),
        ap_obj::value::Value::Str(ap_obj::value::PyString::new(
            name.rsplit_once('.').map(|(m, _)| m).unwrap_or("builtins").to_string(),
        )),
    );

    // 3. Fill the class dict with dispatch markers.
    let spec_index = {
        let mut tab = spec_table().lock().unwrap_or_else(|p| p.into_inner());
        tab.push(SpecRecord::empty(name));
        tab.len() - 1
    };
    {
        let mut tab = spec_table().lock().unwrap_or_else(|p| p.into_inner());
        let rec = tab.last_mut().expect("just pushed");
        rec.tp_new = tp_new;
        rec.tp_init = tp_init;
        rec.tp_dealloc = tp_dealloc;
        rec.methods = methods;
        rec.getsets = getsets;
    }

    let dict = t.dict.clone();
    let mut ns = dict;
    // tp_new marker: instantiate() looks up __new__ and CALLS it with
    // (cls, *args) — any callable Value works (no is_user_function gate).
    if tp_new.is_some() {
        let _ = ns.set(
            Value::Str(PyString::new("__new__")),
            slot_marker(spec_index, "__new__"),
        );
    } else {
        // object.__new__-style allocation marker: ce_bridge's no-tp_new arm
        // allocates a plain instance; still needs a marker so the default
        // arity check behaves like CPython's object.__new__.
        let _ = ns.set(
            Value::Str(PyString::new("__new__")),
            slot_marker(spec_index, "__object_new__"),
        );
    }
    // tp_init marker: instantiate()'s init path calls __init__ when it is
    // present; the is_user_function gate falls to the marker branch in
    // ce_bridge via the class-attr binding (BoundMethod route below).
    if let Some(_) = tp_init {
        let _ = ns.set(
            Value::Str(PyString::new("__init__")),
            slot_marker(spec_index, "__init__"),
        );
    }
    // tp_dealloc: registered for the bridge's GC-visible awareness; nothing
    // Python-visible enters the dict.
    // Methods: plain marker entries; attr reads on instances bind them as
    // BoundMethods via the builtinfn-with-dot route (proven lxml pattern —
    // instance_attr's dotted-BuiltinFn arm wraps with the receiver).
    {
        let tab = spec_table().lock().unwrap_or_else(|p| p.into_inner());
        let rec = &tab[spec_index];
        for (i, (mname, _, _)) in rec.methods.iter().enumerate() {
            let _ = ns.set(
                Value::Str(PyString::new(mname.clone())),
                Value::Obj(PyObject::new(
                    ap_obj::ObjKind::BuiltinFn,
                    format!("__ap_ce__.{}#m{}", spec_index, i),
                )),
            );
        }
        // Getsets: property descriptors whose fget is the marker callable
        // (descr_get calls fget(self) — exact property semantics).
        for (i, (gname, getter, _setter)) in rec.getsets.iter().enumerate() {
            let _ = ns.set(
                Value::Str(PyString::new(gname.clone())),
                make_getset_property(spec_index, i, getter.is_some()),
            );
        }
    }

    // 4. Wire the class Value both ways and record it.
    let cv = make_class_value(t.clone());
    // 4b. Dotted spec names ("spike.Point") auto-install the class into
    // the module ns (the PyModule_AddObject(m, "Point", Type) contract).
    if let Some((mod_name, bare)) = name.rsplit_once('.') {
        crate::api::module_install_class(mod_name, bare, &cv);
    }
    {
        let mut tab = spec_table().lock().unwrap_or_else(|p| p.into_inner());
        let rec = tab.last_mut().expect("just pushed");
        rec.class_value = Some(cv);
        rec.type_ref = Some(t.clone());
    }
    Ok(spec_index)
}

/// The `__ap_ce__.<spec>#<slot>` marker for a tp_new/tp_init slot.
fn slot_marker(spec_index: usize, slot: &str) -> Value {
    Value::Obj(PyObject::new(
        ap_obj::ObjKind::BuiltinFn,
        format!("__ap_ce__.{}#{}", spec_index, slot),
    ))
}

/// A property descriptor whose fget is the getset marker (self rides as
/// the sole arg at descr_get call time).
fn make_getset_property(spec_index: usize, gi: usize, has_getter: bool) -> Value {
    let o = PyObject::new(ap_obj::ObjKind::Descriptor, "property");
    *o.descr_kind.lock().unwrap_or_else(|p| p.into_inner()) = Some("property".to_string());
    let fget = if has_getter {
        slot_marker(spec_index, &format!("g{}", gi))
    } else {
        Value::None
    };
    *o.self_value.lock().unwrap_or_else(|p| p.into_inner()) = Some(fget);
    Value::Obj(o)
}

/// Parse a `__ap_ce__.<spec>#<entry>` marker name into (spec, entry).
pub fn parse_marker(name: &str) -> Option<(usize, String)> {
    let rest = name.strip_prefix("__ap_ce__.")?;
    let (idx, entry) = rest.split_once('#')?;
    let idx: usize = idx.parse().ok()?;
    Some((idx, entry.to_string()))
}

/// The class Value for a registered spec (value_conv Type arm).
pub fn class_value_for_spec(spec_index: usize) -> Option<Value> {
    let tab = spec_table().lock().unwrap_or_else(|p| p.into_inner());
    tab.get(spec_index)?.class_value.clone()
}

/// The TypeRef for a registered spec (instance reconstruction).
pub fn type_ref_for_spec(spec_index: usize) -> Option<Arc<TypeRef>> {
    let tab = spec_table().lock().unwrap_or_else(|p| p.into_inner());
    tab.get(spec_index)?.type_ref.clone()
}

/// The spec record for the bridge (clone out the fn pointers; SlotFn is
/// Copy so this is cheap).
pub fn record_clone(spec_index: usize) -> Option<SpecSnapshot> {
    let tab = spec_table().lock().unwrap_or_else(|p| p.into_inner());
    let r = tab.get(spec_index)?;
    Some(SpecSnapshot {
        name: r.name.clone(),
        tp_new: r.tp_new,
        tp_init: r.tp_init,
        tp_dealloc: r.tp_dealloc,
        methods: r.methods.clone(),
        getsets: r.getsets.clone(),
    })
}

/// A cheap clone of the C-side dispatch surface (fn pointers are Copy).
pub struct SpecSnapshot {
    pub name: String,
    pub tp_new: Option<SlotFn>,
    pub tp_init: Option<SlotFn>,
    pub tp_dealloc: Option<SlotFn>,
    pub methods: Vec<(String, i32, SlotFn)>,
    pub getsets: Vec<(String, Option<SlotFn>, Option<SlotFn>)>,
}

impl SpecSnapshot {
    pub fn method_at(&self, i: usize) -> Option<SlotFn> {
        self.methods.get(i).map(|(_, _, f)| *f)
    }
    pub fn getter_at(&self, i: usize) -> Option<SlotFn> {
        self.getsets.get(i).and_then(|(_, g, _)| *g)
    }
    pub fn setter_at(&self, i: usize) -> Option<SlotFn> {
        self.getsets.get(i).and_then(|(_, _, s)| *s)
    }
}

/// Reverse lookup: registered spec index by bare class name (S0 contract:
/// one spec per name; collisions resolve to the LAST registration).
pub fn spec_index_for_class_name(name: &str) -> Option<usize> {
    let tab = spec_table().lock().unwrap_or_else(|p| p.into_inner());
    tab.iter()
        .rposition(|r| r.name == name || r.name.rsplit_once('.').map(|(_, b)| b) == Some(name))
}
