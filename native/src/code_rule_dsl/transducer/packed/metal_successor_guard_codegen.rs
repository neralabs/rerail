//! Read-only successor tails stop at the existing readiness and admission cuts.
use super::super::super::metal_grid_plan::{ExecutionPlan, ResidentInput};
use super::super::super::{
    ABSENT_FIELD, Error, FIELD, FIRST, FLAGS, FORK, GREEDY, KIND, LAST, SIZE, SUPERTYPE, split,
};
use std::fmt::Write as _;

pub(super) struct Code {
    pub source: String,
    pub ends: Vec<u32>,
}
pub(super) fn compile(resident: &ResidentInput, plan: &ExecutionPlan) -> Result<Code, Error> {
    let readiness = plan.readiness.entries(resident)?;
    let packed = &resident.input.packed;
    let mut ends = vec![0; packed.words.len()];
    let mut operations = std::collections::BTreeSet::new();
    for pg in &packed.programs {
        let first = pg.structural_start as usize;
        let words = &packed.words[first..first + pg.structural_count as usize];
        let incoming = super::super::super::metal_initial_prefix::incoming_counts(words);
        let query = pg.flags & 2 != 0;
        let greedy = words.iter().any(|&w| matches!(split(w), (FORK, GREEDY, _)));
        for entry in 0..words.len() {
            let mut end = entry;
            if !query || !greedy {
                for pc in entry..words.len().min(entry + 8) {
                    if query && (readiness[first + pc][0] != u32::MAX || incoming[pc] != 1) {
                        break;
                    }
                    let (op, mode, _) = split(words[pc]);
                    if !matches!(
                        op,
                        KIND | FIELD | FLAGS | SIZE | FIRST | LAST | ABSENT_FIELD | SUPERTYPE
                    ) {
                        break;
                    }
                    operations.insert((op, mode));
                    end = pc + 1;
                }
            }
            ends[first + entry] = u32::try_from(end).map_err(|_| Error::BackendCapacity)?;
        }
    }
    let mut source = String::from(
        "void expanded_guard(View v,Program pg,uint key,uint arg,thread Step& x){switch(key){\n",
    );
    for (op, mode) in operations {
        let key = u32::from(op) | u32::from(mode) << 8;
        let line =
            super::super::super::metal_successor_codegen::instruction_line(op, mode, false, true)?;
        writeln!(source, "case {key}u:{{{line}break;}} ").unwrap();
    }
    source.push_str("default:x.boundary=2u;break;}}\n");
    Ok(Code { source, ends })
}
