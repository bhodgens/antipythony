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
    /// Dense instr_idx -> site-id map (leaf fp-inlinecache): one entry per
    /// instruction, u32::MAX = no site. The dispatch loop indexes THIS to
    /// find a site in O(1) — a per-hit scan of `sites` would cost more
    /// than the cache saves.
    by_instr: Vec<u32>,
}

const NO_SITE: u32 = u32::MAX;

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
        // Dense map grows lazily: instr indices can arrive out of order
        // (alloc-before-emit sites record 0 and are retargeted below).
        if self.by_instr.len() <= instr_idx as usize {
            self.by_instr.resize(instr_idx as usize + 1, NO_SITE);
        }
        self.by_instr[instr_idx as usize] = id;
        id
    }

    /// Fix a site's instruction index after emission (the id must be known
    /// before the instruction carrying it is built, but the instruction's
    /// index is only known after it is pushed).
    pub fn retarget(&mut self, instr_idx: u32, id: u32) {
        if let Some(info) = self.sites.get_mut(id as usize) {
            // Move the dense-map entry (the old index, if any, clears).
            let old = info.instr_idx;
            if old != instr_idx {
                if (old as usize) < self.by_instr.len() && self.by_instr[old as usize] == id {
                    self.by_instr[old as usize] = NO_SITE;
                }
                if self.by_instr.len() <= instr_idx as usize {
                    self.by_instr.resize(instr_idx as usize + 1, NO_SITE);
                }
                self.by_instr[instr_idx as usize] = id;
            }
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

    /// Dense lookup: which site id sits on the instruction at `instr_idx`
    /// (None = no site). The dispatch loop's O(1) site finder.
    pub fn site_at(&self, instr_idx: u32) -> Option<u32> {
        let id = *self.by_instr.get(instr_idx as usize)?;
        if id == NO_SITE {
            None
        } else {
            Some(id)
        }
    }

    pub fn slot(&self, id: u32) -> Option<CacheSlot> {
        self.slots.get(id as usize).copied()
    }

    pub fn iter(&self) -> impl Iterator<Item = &SiteInfo> {
        self.sites.iter()
    }

    /// fp-superinstructions: remap every site's recorded instruction index
    /// after the peephole shrinks the stream, and rebuild the dense
    /// instr_idx -> site-id map. `remap[old] = new`; site ids themselves are
    /// stable (they are the interpreter's cache-array indices).
    pub fn remap_instr(&mut self, remap: &[u32]) {
        let new_len = (0..self.by_instr.len())
            .filter_map(|old| remap.get(old).copied())
            .max()
            .map_or(0, |m| m as usize + 1);
        self.by_instr = vec![NO_SITE; new_len];
        for (id, info) in self.sites.iter_mut().enumerate() {
            let old = info.instr_idx as usize;
            let new = remap.get(old).copied().unwrap_or(info.instr_idx);
            info.instr_idx = new;
            if (new as usize) < self.by_instr.len() {
                self.by_instr[new as usize] = id as u32;
            }
        }
    }
}
