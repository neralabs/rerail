//! A rules pass's program (decision 0662 §5.1): an analysis over a program image, written as rules over the image's
//! view (`rr/view.rerail`), lowered once and run by the native engine over the view read in place.

use crate::code_rule_dsl::facts::vtab::{Modules, Source};
use crate::code_rule_dsl::logic::plan::{self, lower::Lowered};
use crate::code_rule_dsl::logic::program_image::Version;
use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, OnceLock};

/// A rules pass's program: its text (a library file with its includes, read as one program) and, made at its first
/// run, the program lowered.
pub struct RulesProgram {
    /// Its text, or why it could not be read.
    text: Result<String, String>,
    lower: fn(&str) -> Result<Arc<Version>, String>,
    planned: OnceLock<Result<Planned, String>>,
}

/// The program lowered, with the atoms its own constants are.
struct Planned {
    lowered: Lowered,
    constants: HashMap<String, i64>,
}

/// The first atom of a pass's own constants: an image's atoms are below it.
const CONSTANTS: u32 = 1 << 31;

/// Each output's rows, in the order asked.
pub type OutputRows = Vec<Vec<Vec<i64>>>;

thread_local! {
    /// The thread's engine for rules passes: a connection with the native engine registered and the tables bound on
    /// it (thread scratch: a run binds its inputs, reads its outputs and forgets them).
    static ENGINE: RefCell<Option<(Modules, rusqlite::Connection)>> = const { RefCell::new(None) };
}

impl RulesProgram {
    /// The program `text`, lowered by `lower` (the reader that runs the lowering's tail) at its first run.
    pub fn new(text: Result<String, String>, lower: fn(&str) -> Result<Arc<Version>, String>) -> Self {
        RulesProgram { text, lower, planned: OnceLock::new() }
    }

    /// Its text, which a pass's settings hold.
    pub fn text(&self) -> &str {
        self.text.as_deref().unwrap_or_default()
    }

    fn planned(&self) -> Result<&Planned, String> {
        self.planned
            .get_or_init(|| {
                let _s = crate::code_rule_dsl::logic::profile::span(|| "a rules pass's program lowered".into());
                let v = (self.lower)(self.text.as_ref()?)?;
                let mut constants: HashMap<String, i64> = HashMap::new();
                let mut intern = |names: &[&str]| -> plan::Result<rerail_blocks::vector::Vector<u32>> {
                    Ok(names
                        .iter()
                        .map(|n| {
                            let next = CONSTANTS + constants.len() as u32;
                            *constants.entry((*n).to_owned()).or_insert(i64::from(next)) as u32
                        })
                        .collect())
                };
                let lowered = plan::lower::lowered(v, crate::code_rule_dsl::language::GrammarDialect::Python, &mut intern).map_err(|e| e.to_string())?;
                Ok(Planned { lowered, constants })
            })
            .as_ref()
            .map_err(Clone::clone)
    }

    /// The atom the program's constant `name` is.
    pub fn constant(&self, name: &str) -> Result<i64, String> {
        self.planned()?.constants.get(name).copied().ok_or_else(|| format!("the rules pass's program has no constant {name:?}"))
    }

    /// The program run over `inputs` (each bound by its relation's name): the live rows of each of `outputs`.
    pub fn run(&self, inputs: Vec<(&str, Arc<dyn Source>)>, outputs: &[&str]) -> Result<OutputRows, String> {
        use crate::code_rule_dsl::logic::profile::span;
        let p = self.planned()?;
        let _s = span(|| "a rules pass run".into());
        ENGINE.with(|engine| {
            let mut engine = engine.borrow_mut();
            if engine.is_none() {
                let _s = span(|| "the thread's engine opened".into());
                let db = rusqlite::Connection::open_in_memory().map_err(|e| e.to_string())?;
                crate::code_rule_dsl::logic::native::register(&db).map_err(|e| e.to_string())?;
                *engine = Some((Modules::default(), db));
            }
            let Some((modules, db)) = engine.as_ref() else { unreachable!() };
            // Each input's statistics, measured from its rows as a source bound at run time is (`syntax::statistics`).
            let mut stats = {
                let _s = span(|| "statistics: the program's empty".into());
                plan::Stats::empty_of_lowered(&p.lowered).map_err(|e| e.to_string())?
            };
            let mut bound: Vec<(&str, Vec<String>)> = Vec::with_capacity(inputs.len());
            for (name, source) in inputs {
                bound.push((name, source.columns()));
                {
                    let _s = span(|| "statistics: an input's measured".into());
                    let measured = crate::code_rule_dsl::facts::vtab::syntax::statistics(source.as_ref());
                    stats.measured(name, measured.into_iter().map(|(c, rows, ndv, sq, sqd)| (c, [rows, ndv, sq, sqd])), []);
                }
                let _s = span(|| "an input bound".into());
                modules.bind(db, name, source).map_err(|e| e.to_string())?;
            }
            let kept: BTreeSet<String> = outputs.iter().map(|o| (*o).to_owned()).collect();
            let ran = {
                let _s = span(|| "the engine's run".into());
                plan::run_atoms_native_over(&p.lowered, db, "main", stats, kept, "program").map_err(|e| e.to_string())
            };
            let _s = span(|| "outputs read, the run forgotten".into());
            let read = ran.map(|_| {
                outputs
                    .iter()
                    .map(|o| {
                        let Some(rel) = crate::code_rule_dsl::logic::native::setfix::held(db, o) else { return Vec::new() };
                        (0..rel.len()).filter(|&id| !crate::code_rule_dsl::logic::native::setfix::is_dead(&rel.dead, id as u32)).map(|id| rel.values(id).iter().copied().collect()).collect()
                    })
                    .collect()
            });
            // Neither the run's relations nor its inputs' image are held past it.
            crate::code_rule_dsl::logic::native::setfix::forget_derived(db);
            for (name, columns) in bound {
                let columns: Vec<&str> = columns.iter().map(String::as_str).collect();
                modules.bind(db, name, Arc::new(crate::code_rule_dsl::facts::vtab::Relation::new(&columns))).map_err(|e| e.to_string())?;
            }
            read
        })
    }
}
