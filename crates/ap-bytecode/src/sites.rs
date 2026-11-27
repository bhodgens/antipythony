//! Inline-cache site table.
//!
//! One stable `site: u32` per Load/LoadGlobal/LoadAttr/Call instruction,
//! assigned at lowering time in emission order. This leaf only assigns
//! ids + holds the (empty) per-site slot array; cache STORAGE keyed by
//! type_version lands in 02-perf/01-inline-caches and hangs its data off
//! [`SiteTable::slots`] by index.

/// What kind of site an instruction carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SiteKind {
    /// LoadGlobal (module-level name read / builtin read).
    GlobalLoad,
    /// LoadAttr (attribute read).
    AttrLoad,
    /// CallFunction (call target).
    Call,
}

/// Per-site record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SiteInfo {
    pub kind: SiteKind,
    /// Instruction index of the instruction carrying this site.
    pub instr_idx: u32,
}

/// Reserved per-site cache storage. Empty/untyped this leaf.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheSlot {
    /// Reserved: cache version stamp (type_version at record time).
    pub version: u64,
    /// Reserved: cached payload (opaque to this crate).
    pub payload: u64,
}

/// Site table: id == index into `sites` / `slots`.
#[derive(Debug, Clone, Default)]
pub struct SiteTable {
    sites: Vec<SiteInfo>,
    /// Per-site inline-cache slot array — EMPTY this leaf (02-perf/01
    /// fills it; the interpreter reads slots[site] once caches exist).
    slots: Vec<CacheSlot>,
}

impl SiteTable {
    pub fn new() -> Self {
        Self::default()
    }

    /// Allocate the next site id for `kind` at `instr_idx`.
    pub fn alloc(&mut self, kind: SiteKind, instr_idx: u32) -> u32 {
        let id = self.sites.len() as u32;
        self.sites.push(SiteInfo { kind, instr_idx });
        self.slots.push(CacheSlot {
            version: 0,
            payload: 0,
        });
        id
    }

    /// Fix a site's instruction index after emission (the id must be known
    /// before the instruction carrying it is built, but the instruction's
    /// index is only known after it is pushed).
    pub fn retarget(&mut self, instr_idx: u32, id: u32) {
        if let Some(info) = self.sites.get_mut(id as usize) {
            info.instr_idx = instr_idx;
        }
    }

    pub fn len(&self) -> usize {
        self.sites.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sites.is_empty()
    }

    pub fn get(&self, id: u32) -> Option<SiteInfo> {
        self.sites.get(id as usize).copied()
    }

    pub fn slot(&self, id: u32) -> Option<CacheSlot> {
        self.slots.get(id as usize).copied()
    }

    pub fn iter(&self) -> impl Iterator<Item = &SiteInfo> {
        self.sites.iter()
    }
}
