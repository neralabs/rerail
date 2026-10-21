//! A plan's stages and clauses as sessions run them, stored by *shape*. Planning a stage or clause
//! (`exec::plan`) makes a `Draft`: its statements' texts and effects over the program's counted
//! tables. Many clauses of a policy set are alike: they differ only in their own numbers (their
//! namespace and run id, the terminals they read), in the counted tables of their own, and in
//! their recursion blocks' marks. So a draft is stored as a `Shape`, which clauses alike share,
//! and an `Instance`, its own part:
//!
//! - each text is a skeleton (`texts`), its holes holding the instance's params (`Instance::params`,
//!   one per distinct number, in the order of their first hole);
//! - a counted table of its own is a `LOCAL` index into the instance's own tables, which the
//!   catalog numbers one after another (`Instance::tables`), and a mark is an index into its marks;
//! - a step's effect is an interned `Action`.
//!
//! A shape is shared exactly where binding it to an instance gives that instance's own draft back,
//! byte for byte (`verify`, which every test build checks as it binds each draft).

use super::emit::{self, Effect, Rule, Writes};
use super::frames;
use super::skip::{self, Outcome};
use super::texts::{Param, Skeletons, Texts, number};
use foldhash::HashMap;
use foldhash::fast::FixedState;
use hashbrown::HashTable;
use std::hash::{BuildHasher, Hash, Hasher};

/// One statement of a draft: its text, and its effect over the program's counted tables.
pub(super) struct DraftStep {
    pub(super) sql: String,
    pub(super) effect: Effect<usize>,
}

/// A recursion component's loop as drafted (`Block`).
pub(super) struct DraftBlock {
    pub(super) steps: Vec<DraftStep>,
    pub(super) demand: Vec<usize>,
    pub(super) mark: usize,
    pub(super) size: usize,
}

/// A clause's `violation.each` enumeration as drafted (`Each`), with its tables' prefix.
pub(super) struct DraftEach {
    pub(super) prefix: String,
    pub(super) clear: Vec<DraftStep>,
    pub(super) active: usize,
    pub(super) round: usize,
}

/// A stage's or clause's statements as `exec::plan` makes them, the fields as `Shape`'s; `own` is
/// the first of its own counted tables (the catalog numbers them one after another, up to the
/// catalog's end when it is bound), `marks` the first of its blocks' marks.
pub(super) struct Draft {
    pub(super) steps: Vec<DraftStep>,
    pub(super) blocks: Vec<DraftBlock>,
    pub(super) last: DraftStep,
    pub(super) clear: Vec<DraftStep>,
    pub(super) stages: Vec<usize>,
    pub(super) each: Option<DraftEach>,
    pub(super) root: Option<Root>,
    pub(super) tables: Vec<usize>,
    pub(super) own: usize,
    pub(super) marks: usize,
}

/// A counted table as a shape names it: the catalog's id, or, with `LOCAL`, the index of one of
/// the instance's own tables.
type TableRef = u32;

const LOCAL: TableRef = 1 << 31;

/// What a step is to the session that runs it (`exec::Runner::steps`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Kind {
    /// A statement it executes.
    Statement,
    /// Empties a frame through its buffers (`frames::CLEAR`).
    Clear(TableRef),
    /// Runs the block of this index (`emit::RUN`).
    Run(u32),
    /// Opens a loop (`emit::LOOP`).
    Loop,
    /// Opens a candidate loop that runs at least one phase (`emit::FRONTIER`).
    Frontier,
    /// Ends a loop with its dirty-key query (`emit::UNTIL`).
    Until,
    /// Accepts a loop's last pass by its recursive reads (`emit::ACYCLIC`).
    Acyclic,
}

/// What a step does: its kind, and its effect (`emit::Effect`) over its tables.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Action {
    pub(super) kind: Kind,
    pub(super) rule: Rule,
    writes: Writes<TableRef>,
    /// Its inputs' range in `Actions::inputs`.
    inputs: (u32, u32),
}

/// The distinct actions of a plan.
#[derive(Default)]
pub(super) struct Actions {
    actions: Vec<Action>,
    inputs: Vec<TableRef>,
}

impl Actions {
    fn inputs(&self, action: &Action) -> &[TableRef] {
        &self.inputs[action.inputs.0 as usize..action.inputs.1 as usize]
    }
}

/// One step of a shape: its text's skeleton, its action, and where its holes' args start in
/// `Shape::args` (one per hole).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) struct Step {
    text: u32,
    action: u32,
    args: u32,
}

/// A recursion component's loop (`emit::BLOCK`): each `emit::RUN` of it runs its steps again.
/// `demand`: its units' demand tables; `mark`: where the session keeps their rows as the block's
/// last run left them (`skip`), among the instance's marks; `size`: its statements, each once.
#[derive(Debug, PartialEq, Eq, Hash)]
pub(super) struct Block {
    pub(super) steps: Box<[Step]>,
    demand: Box<[TableRef]>,
    mark: u32,
    pub(super) size: usize,
}

/// A clause's `violation.each` enumeration, whose tables live across the rounds of a batch: their
/// clears, and the counted tables of the roots still enumerated (`a`) and of the round (`r`).
#[derive(Debug, PartialEq, Eq, Hash)]
pub(super) struct Each {
    pub(super) clear: Box<[Step]>,
    active: TableRef,
    round: TableRef,
}

/// A stage's or clause's root frame: the step that fills it, its counted table, and how many
/// statements follow it, the last included (each loop's once).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) struct Root {
    pub(super) at: usize,
    pub(super) table: usize,
    pub(super) rest: usize,
}

/// A stage's or clause's statements as a session runs them, shared by every stage or clause alike.
#[derive(Debug, PartialEq, Eq, Hash)]
pub(super) struct Shape {
    /// The statements that fill its frames, in order.
    pub(super) steps: Box<[Step]>,
    /// The loops its steps run by index (`Kind::Run`), each emitted once (`emit::BLOCK`).
    pub(super) blocks: Box<[Block]>,
    /// A clause's: selects its roots and bindings. A stage's: fills the stage table.
    pub(super) last: Step,
    /// Empties each of its tables.
    pub(super) clear: Box<[Step]>,
    /// The stages its statements read.
    pub(super) stages: Box<[usize]>,
    /// A clause under `violation.each`: its enumeration.
    pub(super) each: Option<Each>,
    /// Its root frame, where it has one (a recursive stage's loop starts from seeds instead); its
    /// table a `TableRef`.
    root: Option<Root>,
    /// The counted tables its statements read and write, which a session makes on its first run
    /// (`Session::activate`); the stages' tables it reads are theirs.
    tables: Box<[TableRef]>,
    /// Each hole's arg, step after step: the index of the instance's param it holds.
    args: Box<[u32]>,
}

/// A stage or clause: its shape, and what is its own.
pub(super) struct Instance {
    shape: u32,
    /// The numbers of its texts' holes, each once (`texts::Param`).
    params: Box<[Param]>,
    /// The first of its own counted tables.
    tables: u32,
    /// The first of its blocks' marks.
    marks: u32,
    /// The distinct texts of the statements a session's cache holds for it: every step's but the
    /// markers' and frame clears', which prepare nothing (a loop's `emit::UNTIL` query does).
    pub(super) texts: Box<[u32]>,
    /// A clause under `violation.each`: the prefix of its enumeration's tables (`emit::EACH`).
    pub(super) prefix: Option<Box<str>>,
}

/// The shapes of a plan, and what they read: the texts' skeletons and the steps' actions.
#[derive(Default)]
pub(super) struct Shapes {
    skeletons: Skeletons,
    actions: Actions,
    shapes: Vec<Shape>,
}

impl Shapes {
    /// `instance` bound to its shape.
    pub(super) fn bound<'p>(&'p self, instance: &'p Instance) -> Bound<'p> {
        Bound {
            shape: &self.shapes[instance.shape as usize],
            instance,
            actions: &self.actions,
            skeletons: &self.skeletons,
        }
    }
}

/// A stage or clause bound to its shape: what its steps are as a session runs them.
#[derive(Clone, Copy)]
pub(super) struct Bound<'p> {
    pub(super) shape: &'p Shape,
    instance: &'p Instance,
    actions: &'p Actions,
    skeletons: &'p Skeletons,
}

impl<'p> Bound<'p> {
    /// The stage or clause bound.
    pub(super) fn instance(&self) -> &'p Instance {
        self.instance
    }

    /// The catalog's id of `table`.
    pub(super) fn table(&self, table: TableRef) -> usize {
        if table & LOCAL == 0 {
            table as usize
        } else {
            (self.instance.tables + (table & !LOCAL)) as usize
        }
    }

    /// The session's mark of `block`.
    pub(super) fn mark(&self, block: &Block) -> usize {
        (self.instance.marks + block.mark) as usize
    }

    pub(super) fn action(&self, step: &Step) -> &'p Action {
        &self.actions.actions[step.action as usize]
    }

    /// The catalog's ids of `action`'s inputs.
    pub(super) fn inputs(&self, action: &Action) -> impl Iterator<Item = usize> {
        self.actions
            .inputs(action)
            .iter()
            .map(|&table| self.table(table))
    }

    /// What `action`'s rule says its statement will change (`skip::outcome`), over the rows each
    /// counted table holds (`rows`, by the catalog's id).
    pub(super) fn outcome(&self, action: &Action, rows: impl Fn(usize) -> i64) -> Outcome {
        skip::outcome(action.rule, self.actions.inputs(action), |table| {
            rows(self.table(table))
        })
    }

    /// The catalog's id of the table `action` writes.
    pub(super) fn writes(&self, action: &Action) -> Writes<usize> {
        match action.writes {
            Writes::None => Writes::None,
            Writes::Insert(table) => Writes::Insert(self.table(table)),
            Writes::Delete(table) => Writes::Delete(self.table(table)),
            Writes::Clear(table) => Writes::Clear(self.table(table)),
        }
    }

    /// The catalog's ids of `block`'s demand tables.
    pub(super) fn demand(&self, block: &'p Block) -> impl Iterator<Item = usize> {
        block.demand.iter().map(|&table| self.table(table))
    }

    /// The catalog's ids of the tables its statements read and write.
    pub(super) fn tables(&self) -> impl Iterator<Item = usize> {
        self.shape.tables.iter().map(|&table| self.table(table))
    }

    /// Its root frame, its table the catalog's id.
    pub(super) fn root(&self) -> Option<Root> {
        self.shape.root.map(|root| Root {
            table: self.table(root.table as TableRef),
            ..root
        })
    }

    /// Its enumeration's prefix, and the catalog's ids of its tables of the roots still
    /// enumerated and of the round.
    pub(super) fn each(&self) -> Option<(&'p str, usize, usize)> {
        let each = self.shape.each.as_ref()?;
        let prefix = self
            .instance
            .prefix
            .as_deref()
            .expect("an enumeration's prefix");
        Some((prefix, self.table(each.active), self.table(each.round)))
    }

    /// Writes `step`'s text into `out`, which it empties first.
    pub(super) fn text(&self, step: &Step, out: &mut String) {
        out.clear();
        let mut arg = step.args as usize;
        self.skeletons.write(
            step.text,
            || {
                let param = self.instance.params[self.shape.args[arg] as usize];
                arg += 1;
                number(param)
            },
            out,
        );
    }
}

fn hash_one(value: impl Hash) -> u64 {
    FixedState::default().hash_one(value)
}

/// The hash of an action's fields.
fn action_hash(kind: Kind, rule: Rule, writes: &Writes<TableRef>, inputs: &[TableRef]) -> u64 {
    let mut hasher = FixedState::default().build_hasher();
    kind.hash(&mut hasher);
    (rule as u8).hash(&mut hasher);
    match *writes {
        Writes::None => 0u8.hash(&mut hasher),
        Writes::Insert(table) => (1u8, table).hash(&mut hasher),
        Writes::Delete(table) => (2u8, table).hash(&mut hasher),
        Writes::Clear(table) => (3u8, table).hash(&mut hasher),
    }
    inputs.hash(&mut hasher);
    hasher.finish()
}

/// The shapes of a plan being made: each draft is bound as it is planned (`bind`), its texts and
/// actions stored once, its shape shared with every draft alike.
#[derive(Default)]
pub(super) struct ShapeMaker {
    texts: Texts,
    actions: Actions,
    action_index: HashTable<u32>,
    shapes: Vec<Shape>,
    shape_index: HashTable<u32>,
    /// The steps of every instance bound, each instance's own.
    steps: usize,
    /// A step's inputs, while it is bound.
    scratch: Vec<TableRef>,
    /// How many drafts `verify` found their shape gives back, byte for byte.
    #[cfg(test)]
    verified: usize,
}

/// Whether a test build checks each draft it binds (`verify`): on, but where a test measures the
/// compile's own cost.
#[cfg(test)]
pub(in crate::code_rule_dsl) static VERIFY: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(true);

/// What a plan's texts and shapes hold.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct ShapeCensus {
    /// The steps of every stage and clause, each its own.
    pub(super) steps: usize,
    /// The distinct texts, and their bytes.
    pub(super) texts: usize,
    pub(super) bytes: usize,
    /// The distinct shapes and skeletons, and the bytes the skeletons are stored in.
    pub(super) shapes: usize,
    pub(super) skeletons: usize,
    pub(super) stored: usize,
}

impl ShapeMaker {
    /// Binds `draft`, whose own tables end at the catalog's `end`, and whose frame clears name
    /// their frame (`table`: the catalog's id of a table by its name).
    pub(super) fn bind(
        &mut self,
        draft: Draft,
        end: usize,
        table: &dyn Fn(&str) -> usize,
    ) -> Result<Instance, String> {
        let own = draft.own;
        let local = |id: usize| -> TableRef {
            if (own..end).contains(&id) {
                LOCAL | (id - own) as TableRef
            } else {
                id as TableRef
            }
        };
        let mut binder = Binder {
            maker: self,
            local: &local,
            table,
            params: Vec::new(),
            numbering: HashMap::default(),
            args: Vec::new(),
            texts: Vec::new(),
        };
        let steps = binder.steps(&draft.steps)?;
        let blocks = draft
            .blocks
            .iter()
            .map(|block| {
                Ok(Block {
                    steps: binder.steps(&block.steps)?,
                    demand: block.demand.iter().map(|&table| local(table)).collect(),
                    mark: (block.mark - draft.marks) as u32,
                    size: block.size,
                })
            })
            .collect::<Result<_, String>>()?;
        let last = binder.step(&draft.last)?;
        let clear = binder.steps(&draft.clear)?;
        let each = match &draft.each {
            Some(each) => Some(Each {
                clear: binder.steps(&each.clear)?,
                active: local(each.active),
                round: local(each.round),
            }),
            None => None,
        };
        let Binder {
            params,
            args,
            mut texts,
            ..
        } = binder;
        let shape = Shape {
            steps,
            blocks,
            last,
            clear,
            stages: draft.stages.as_slice().into(),
            each,
            root: draft.root.map(|root| Root {
                table: local(root.table) as usize,
                ..root
            }),
            tables: draft.tables.iter().map(|&table| local(table)).collect(),
            args: args.into(),
        };
        texts.sort_unstable();
        texts.dedup();
        let instance = Instance {
            shape: self.shape(shape),
            params: params.into(),
            tables: own as u32,
            marks: draft.marks as u32,
            texts: texts.into(),
            prefix: draft.each.as_ref().map(|each| each.prefix.as_str().into()),
        };
        let shape = &self.shapes[instance.shape as usize];
        self.steps += shape.steps.len()
            + shape
                .blocks
                .iter()
                .map(|block| block.steps.len())
                .sum::<usize>()
            + 1
            + shape.clear.len()
            + shape.each.as_ref().map_or(0, |each| each.clear.len());
        #[cfg(test)]
        if VERIFY.load(std::sync::atomic::Ordering::Relaxed) {
            let bound = Bound {
                shape,
                instance: &instance,
                actions: &self.actions,
                skeletons: self.texts.skeletons(),
            };
            if let Err(error) = verify(bound, &draft) {
                panic!("a shape that does not give its draft back: {error}");
            }
            self.verified += 1;
        }
        Ok(instance)
    }

    /// The id of `shape`, stored once.
    fn shape(&mut self, shape: Shape) -> u32 {
        let hashed = hash_one(&shape);
        let shapes = &self.shapes;
        if let Some(&id) = self
            .shape_index
            .find(hashed, |&id| shapes[id as usize] == shape)
        {
            return id;
        }
        let id = self.shapes.len() as u32;
        self.shapes.push(shape);
        let shapes = &self.shapes;
        self.shape_index
            .insert_unique(hashed, id, |&id| hash_one(&shapes[id as usize]));
        id
    }

    /// The id of the action `kind` with `effect`, its tables by `local`, stored once.
    fn action(
        &mut self,
        kind: Kind,
        effect: &Effect<usize>,
        local: &dyn Fn(usize) -> TableRef,
    ) -> u32 {
        let writes = match effect.writes {
            Writes::None => Writes::None,
            Writes::Insert(table) => Writes::Insert(local(table)),
            Writes::Delete(table) => Writes::Delete(local(table)),
            Writes::Clear(table) => Writes::Clear(local(table)),
        };
        self.scratch.clear();
        self.scratch
            .extend(effect.inputs.iter().map(|&table| local(table)));
        let hashed = action_hash(kind, effect.rule, &writes, &self.scratch);
        let (actions, scratch) = (&self.actions, &self.scratch);
        let found = self.action_index.find(hashed, |&id| {
            let action = &actions.actions[id as usize];
            action.kind == kind
                && action.rule == effect.rule
                && action.writes == writes
                && actions.inputs(action) == scratch.as_slice()
        });
        if let Some(&id) = found {
            return id;
        }
        let id = self.actions.actions.len() as u32;
        let start = self.actions.inputs.len() as u32;
        self.actions.inputs.extend_from_slice(&self.scratch);
        self.actions.actions.push(Action {
            kind,
            rule: effect.rule,
            writes,
            inputs: (start, self.actions.inputs.len() as u32),
        });
        let actions = &self.actions;
        self.action_index.insert_unique(hashed, id, |&id| {
            let action = &actions.actions[id as usize];
            action_hash(
                action.kind,
                action.rule,
                &action.writes,
                actions.inputs(action),
            )
        });
        id
    }

    /// The shapes, once every draft is bound, and what they hold.
    pub(super) fn finish(self) -> (Shapes, ShapeCensus) {
        let (texts, bytes) = self.texts.census();
        let skeletons = self.texts.finish();
        let census = ShapeCensus {
            steps: self.steps,
            texts,
            bytes,
            shapes: self.shapes.len(),
            skeletons: skeletons.len(),
            stored: skeletons.bytes(),
        };
        let Actions {
            mut actions,
            mut inputs,
        } = self.actions;
        actions.shrink_to_fit();
        inputs.shrink_to_fit();
        let mut shapes = self.shapes;
        shapes.shrink_to_fit();
        (
            Shapes {
                skeletons,
                actions: Actions { actions, inputs },
                shapes,
            },
            census,
        )
    }

    /// How many drafts `verify` checked.
    #[cfg(test)]
    pub(super) fn verified(&self) -> usize {
        self.verified
    }
}

/// One draft being bound: the params its texts' holes hold, each once (`numbering`: a param's
/// index), each hole's arg, and the texts a session's cache holds for it.
struct Binder<'m> {
    maker: &'m mut ShapeMaker,
    local: &'m dyn Fn(usize) -> TableRef,
    table: &'m dyn Fn(&str) -> usize,
    params: Vec<Param>,
    numbering: HashMap<Param, u32>,
    args: Vec<u32>,
    texts: Vec<u32>,
}

impl Binder<'_> {
    fn steps(&mut self, steps: &[DraftStep]) -> Result<Box<[Step]>, String> {
        steps.iter().map(|step| self.step(step)).collect()
    }

    fn step(&mut self, step: &DraftStep) -> Result<Step, String> {
        let sql = step.sql.as_str();
        let id = self.maker.texts.intern(sql);
        let (text, held) = self.maker.texts.text(id);
        let args = self.args.len() as u32;
        for &param in held {
            let next = self.params.len() as u32;
            let index = *self.numbering.entry(param).or_insert(next);
            if index == next {
                self.params.push(param);
            }
            self.args.push(index);
        }
        let kind = if let Some(name) = sql.strip_prefix(frames::CLEAR) {
            Kind::Clear((self.local)((self.table)(name)))
        } else if let Some(index) = sql.strip_prefix(emit::RUN) {
            Kind::Run(
                index
                    .parse()
                    .map_err(|_| format!("a block run without an index: {sql}"))?,
            )
        } else if sql == emit::LOOP {
            Kind::Loop
        } else if sql == emit::FRONTIER {
            Kind::Frontier
        } else if sql.starts_with(emit::UNTIL) {
            Kind::Until
        } else if sql.starts_with(emit::ACYCLIC) {
            Kind::Acyclic
        } else if sql.starts_with("/*") {
            return Err(format!("a marker no session runs: {sql}"));
        } else {
            Kind::Statement
        };
        if matches!(kind, Kind::Statement | Kind::Until) {
            self.texts.push(id);
        }
        let action = self.maker.action(kind, &step.effect, self.local);
        Ok(Step { text, action, args })
    }
}

/// Whether `bound` gives `draft` back: every step's text byte for byte and its effect over the
/// same tables, the blocks' demand, marks and sizes, and the stages, enumeration, root and tables.
#[cfg(test)]
pub(super) fn verify(bound: Bound<'_>, draft: &Draft) -> Result<(), String> {
    fn steps(bound: Bound<'_>, steps: &[Step], drafts: &[DraftStep]) -> Result<(), String> {
        if steps.len() != drafts.len() {
            return Err(format!("{} steps, drafted {}", steps.len(), drafts.len()));
        }
        let mut text = String::new();
        for (step, draft) in steps.iter().zip(drafts) {
            bound.text(step, &mut text);
            if text != draft.sql {
                return Err(format!("the text {text:?}, drafted {:?}", draft.sql));
            }
            let action = bound.action(step);
            let effect = Effect {
                rule: action.rule,
                inputs: bound.inputs(action).collect(),
                writes: bound.writes(action),
            };
            if effect != draft.effect {
                return Err(format!(
                    "the effect {effect:?} of {text:?}, drafted {:?}",
                    draft.effect
                ));
            }
            if let Kind::Clear(table) = action.kind
                && !matches!(draft.effect.writes, Writes::Clear(cleared) if cleared == bound.table(table))
            {
                return Err(format!("a frame clear of another table: {text:?}"));
            }
        }
        Ok(())
    }
    let shape = bound.shape;
    steps(bound, &shape.steps, &draft.steps)?;
    if shape.blocks.len() != draft.blocks.len() {
        return Err("another number of blocks".to_owned());
    }
    for (block, drafted) in shape.blocks.iter().zip(&draft.blocks) {
        steps(bound, &block.steps, &drafted.steps)?;
        if bound.demand(block).collect::<Vec<_>>() != drafted.demand
            || bound.mark(block) != drafted.mark
            || block.size != drafted.size
        {
            return Err(format!(
                "a block's demand, mark or size, drafted mark {}",
                drafted.mark
            ));
        }
    }
    steps(
        bound,
        std::slice::from_ref(&shape.last),
        std::slice::from_ref(&draft.last),
    )?;
    steps(bound, &shape.clear, &draft.clear)?;
    if *shape.stages != *draft.stages {
        return Err("other stages".to_owned());
    }
    match (&shape.each, &draft.each) {
        (None, None) => {}
        (Some(each), Some(drafted)) => {
            steps(bound, &each.clear, &drafted.clear)?;
            if bound.each() != Some((drafted.prefix.as_str(), drafted.active, drafted.round)) {
                return Err("another enumeration".to_owned());
            }
        }
        _ => return Err("an enumeration where none was drafted, or none".to_owned()),
    }
    if bound.root() != draft.root {
        return Err("another root frame".to_owned());
    }
    if bound.tables().collect::<Vec<_>>() != draft.tables {
        return Err("other tables".to_owned());
    }
    Ok(())
}
