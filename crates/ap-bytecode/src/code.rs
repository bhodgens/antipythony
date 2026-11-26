//! The assembled unit of bytecode: what leaf 02's frame consumes.

use crate::instr::Instr;
use crate::linetable::LineTable;
use crate::pool::ConstPool;
use crate::sites::SiteTable;

/// One lowered function.
#[derive(Debug, Clone)]
pub struct CodeObject {
    /// Frame name (`co_name`; tracebacks / `<lambda>` per S-02-38).
    pub name: String,
    /// 1-based line of the `def` (`co_firstlineno`).
    pub def_line: usize,
    /// Number of local slots (frame locals vector length).
    pub n_locals: usize,
    /// Slot -> name, ordered by slot index.
    pub local_names: Vec<String>,
    /// The flat instruction stream.
    pub instrs: Vec<Instr>,
    pub consts: ConstPool,
    /// Shared name table (LoadGlobal/StoreGlobal/LoadAttr indices).
    pub names: Vec<String>,
    pub lines: LineTable,
    pub sites: SiteTable,
}

impl CodeObject {
    /// Instruction count (also the jump-target address space bound).
    pub fn instr_count(&self) -> usize {
        self.instrs.len()
    }

    /// Validate structural invariants that lowering must preserve:
    /// - every jump target is in bounds (back-edges ARE emitted since
    ///   leaf 01-core/01: loop back-edges Jump to the loop head),
    /// - every site id carried by an instruction is a valid table index
    ///   whose recorded instr_idx matches the instruction's own index,
    /// - every const-pool index referenced is in bounds.
    ///
    /// Used by tests and debug tooling; the dispatch loop re-asserts the
    /// jump bound per opcode (buffer bounds proven at lowering time).
    pub fn validate(&self) -> Result<(), String> {
        let n = self.instrs.len();
        for (i, ins) in self.instrs.iter().enumerate() {
            if let Some(t) = ins.jump_target() {
                let t = t as usize;
                if t >= n {
                    return Err(format!("jump target {t} out of bounds (n={n})"));
                }
                if *ins == Instr::Jump(u32::MAX) || *ins == Instr::ForNext(u32::MAX) {
                    return Err(format!("unpatched jump placeholder at instr {i}"));
                }
            }
            if let Some(site) = ins.site_id() {
                match self.sites.get(site) {
                    Some(info) => {
                        if info.instr_idx as usize != i {
                            return Err(format!(
                                "site {site} points at instr {} but sits on instr {i}",
                                info.instr_idx
                            ));
                        }
                    }
                    None => return Err(format!("site id {site} not in site table")),
                }
            }
            if let Instr::LoadConst(idx) = ins {
                if (*idx as usize) >= self.consts.len() {
                    return Err(format!("const idx {idx} out of bounds"));
                }
            }
            if let Instr::LoadAttr { idx, .. } = ins {
                if (*idx as usize) >= self.names.len() {
                    return Err(format!("attr name idx {idx} out of bounds"));
                }
            }
        }
        Ok(())
    }
}
