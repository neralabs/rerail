//! Parsed-document frontier with source-proved ownership and ordered work.
//!
//! Ordinary Boolean/neighbor composition remains in its existing SQL regions.
//! Only a single utility whose recursive edge crosses an annotation parse is
//! specialized: entry, local preorder cursor, and boundary return are its three
//! phases. No matcher body is evaluated by an atomic callback.
use super::super::{Axis, CaptureKind, Compiled, lower::Lower};
use super::{Demand, Generation, Op, Plan, Slots, Sql, fields, ordered};
use crate::code_rule_dsl::matcher::{AstMatcher, GrammarMatcher, Semantic, StopBy, UtilRef};
use crate::code_rule_dsl::sql::relational::ir::Set;
use serde_json::{Value, json};
use std::collections::{BTreeSet, HashSet};

fn transparent(p: &Plan) -> &Plan {
    if let Op::Shared(body) = &p.op {
        transparent(body)
    } else {
        p
    }
}
fn kind_set(p: &Plan) -> Option<BTreeSet<u16>> {
    fn set(s: &Set) -> Option<BTreeSet<u16>> {
        match s {
            Set::Kinds(k) => Some(k.iter().copied().collect()),
            Set::Union(v) => {
                let mut out = BTreeSet::new();
                for s in v {
                    out.extend(set(s)?);
                }
                Some(out)
            }
            _ => None,
        }
    }
    match &transparent(p).op {
        Op::Predicate(s) => set(s),
        Op::Any(items) => {
            let mut union = BTreeSet::new();
            for item in items {
                union.extend(kind_set(item)?);
            }
            Some(union)
        }
        _ => None,
    }
}
fn source_safe(m: &AstMatcher, forbidden: &UtilRef, active: &mut HashSet<(usize, usize)>) -> bool {
    match m {
        AstMatcher::Util(u) => {
            let key = (u.table.as_ptr() as usize, u.index);
            if key == (forbidden.table.as_ptr() as usize, forbidden.index) {
                return false;
            }
            if !active.insert(key) {
                return true;
            }
            let okay = u
                .with_target(|m| source_safe(m, forbidden, active))
                .unwrap_or(false);
            active.remove(&key);
            okay
        }
        AstMatcher::All(v) | AstMatcher::Any(v) => {
            v.iter().all(|m| source_safe(m, forbidden, active))
        }
        AstMatcher::Not(m) => source_safe(m, forbidden, active),
        AstMatcher::Inside(r, s)
        | AstMatcher::Has(r, s)
        | AstMatcher::Follows(r, s)
        | AstMatcher::Precedes(r, s) => {
            matches!(s, StopBy::Neighbor | StopBy::End) && source_safe(r.rule(), forbidden, active)
        }
        AstMatcher::Kind(_)
        | AstMatcher::Regex(_)
        | AstMatcher::Pattern(_)
        | AstMatcher::Semantic(Semantic::Capture(_) | Semantic::Value(_) | Semantic::Resolves(_)) => {
            true
        }
        _ => false,
    }
}

/// A rejected prefix is total metadata followed by a fixed number of
/// nonrecursive utility entries. Its omitted original-document memo keys have
/// no other source reader: helper bodies and the remaining first-arm suffix
/// are checked against the same original UtilRef identity. Parsed uses cannot
/// observe them because native SubDocs bypass utility memoization.
struct RejectedPrefix {
    condition: String,
    cost: i64,
}
fn metadata(p: &Plan) -> Option<String> {
    fn allowed(s: &Set) -> bool {
        match s {
            Set::AllNodes | Set::Empty | Set::Kinds(_) | Set::Regex(_) => true,
            Set::Intersect(v) | Set::Union(v) => v.iter().all(allowed),
            Set::Complement(s) => allowed(s),
            _ => false,
        }
    }
    let p = transparent(p);
    if !p.writes.is_empty()
        || !p.mentions.is_empty()
        || p.effects.reads_value
        || p.effects.uses_memo
        || p.effects.cache_fills
        || p.effects.fallible
    {
        return None;
    }
    let Op::Predicate(s) = &p.op else { return None };
    allowed(s)
        .then(|| super::super::super::emit::condition(s, "n").ok())
        .flatten()
}
fn rejected_prefix(
    first: Option<&Plan>,
    source: Option<&AstMatcher>,
    lower: &mut Lower,
) -> Option<RejectedPrefix> {
    let Some(first) = first else {
        return Some(RejectedPrefix {
            condition: "0".into(),
            cost: 0,
        });
    };
    if let Some(condition) = metadata(first) {
        return Some(RejectedPrefix { condition, cost: 0 });
    }
    let (util, suffix) = match source? {
        AstMatcher::Util(u) => (u, &[][..]),
        AstMatcher::All(v) => match v.split_first()? {
            (AstMatcher::Util(u), rest) => (u, rest),
            _ => return None,
        },
        _ => return None,
    };
    // Every possible original-file reader is covered: only the initial call
    // may name this utility; all related-node/helper paths must exclude it.
    if !suffix
        .iter()
        .all(|m| source_safe(m, util, &mut HashSet::new()))
        || !util
            .with_target(|body| source_safe(body, util, &mut HashSet::new()))
            .unwrap_or(false)
    {
        return None;
    }
    // The physical compiler may fuse the utility and its helpers into a
    // PureRecursive region. Derive the entry prefix from the original AST,
    // rather than requiring its physical representation to remain Charge.
    let condition = util
        .with_target(|body| {
            let guard = match body {
                AstMatcher::All(v) => v.first()?,
                _ => body,
            };
            let plan = lower.plan(guard).ok()?;
            metadata(&plan)
        })
        .flatten()?;
    Some(RejectedPrefix { condition, cost: 1 })
}

struct Shape<'a> {
    first: Option<&'a Plan>,
    base: &'a Plan,
    initial_term: usize,
    nested_term: usize,
    exports: &'a [(usize, usize)],
    hosts: BTreeSet<u16>,
    filter: String,
}
fn shape<'a>(root: &'a Plan, utility: &UtilRef) -> Result<Shape<'a>, String> {
    let (first,call)=match &transparent(root).op {
        Op::Any(items) if items.len()==2=>(Some(&items[0]),&items[1]),
        Op::Charge(..)=>(None,root),
        Op::All(items) if items.len()==2 && kind_set(&items[0]).is_some() => (None,&items[1]),
        _=>return Err("document frontier: expected one-use parse utility, optionally last in a two-arm disjunction".into()),
    };
    let Op::Charge(_, body) = &transparent(call).op else {
        return Err("document frontier: original-file utility entry missing".into());
    };
    let Op::All(parts) = &transparent(body).op else {
        return Err("document frontier: expected parse followed by host metadata".into());
    };
    if parts.len() != 2 {
        return Err("document frontier: utility has additional reached effects".into());
    }
    let hosts =
        kind_set(&parts[1]).ok_or("document frontier: suffix is not a literal-kind disjunction")?;
    let Op::Parse {
        term,
        child,
        exports,
        hosted: false,
        ..
    } = &transparent(&parts[0]).op
    else {
        return Err("document frontier: first utility operation is not an unhosted parse".into());
    };
    let Op::Search(Axis::Child, true, body, None) = &transparent(child).op else {
        return Err("document frontier: expected strict descendant search".into());
    };
    let Op::Any(arms) = &transparent(body).op else {
        return Err("document frontier: expected base before recursive continuation".into());
    };
    if arms.len() != 2 {
        return Err("document frontier: expected exactly one base and continuation".into());
    }
    let Op::ParsedRecursive { program, entry } = &transparent(&arms[1]).op else {
        return Err("document frontier: parsed recursive continuation missing".into());
    };
    if *entry != 0 || program.functions.len() != 1 {
        return Err(
            "document frontier: mutual utility component is outside the local frontier contract"
                .into(),
        );
    }
    use super::super::lower::parse_recursive_lower::Instruction;
    let terms = program
        .code
        .iter()
        .filter_map(|i| match i {
            Instruction::Parse { term, .. } => Some(*term),
            _ => None,
        })
        .collect::<Vec<_>>();
    if terms.len() != 1 {
        return Err("document frontier: nested utility has multiple parse boundaries".into());
    }
    let valid=utility.with_target(|m| {
        let AstMatcher::All(v)=m else{return false};
        let Some(AstMatcher::Semantic(Semantic::Parse(p)))=v.first() else{return false};
        if p.host_sound.is_some() || *p.host_kinds!=hosts { return false; }
        let AstMatcher::Has(r,StopBy::End)=&p.rule else{return false};
        let AstMatcher::Any(v)=r.rule() else{return false};
        v.len()==2 && source_safe(&v[0],utility,&mut HashSet::new()) && matches!(&v[1],AstMatcher::Util(u) if u.table.as_ptr()==utility.table.as_ptr() && u.index==utility.index)
    }).unwrap_or(false);
    if !valid {
        return Err(
            "document frontier: source recursion is not one self-call under the same parse".into(),
        );
    }
    if arms[0].writes.is_empty() || !arms[0].mentions.is_subset(&arms[0].writes) {
        return Err(
            "document frontier: base must have strict outputs and no external capture inputs"
                .into(),
        );
    }
    let writes: Slots = exports.iter().map(|(inner, _)| *inner).collect();
    if writes != arms[0].writes || program.outputs[0] != writes {
        return Err(
            "document frontier: recursive export slots do not match finite base outputs".into(),
        );
    }
    Ok(Shape {
        first,
        base: &arms[0],
        initial_term: *term,
        nested_term: terms[0],
        exports,
        hosts,
        filter: ordered::candidate_filter(body, "candidate"),
    })
}

/// Numeric frames contain caller identity, next local cursor, bound and depth.
/// Captures remain typed SQL cells and are mapped through each parse boundary.
fn frontier(s: &Shape<'_>, generation: Generation) -> Result<(String, Value), String> {
    let slots = &s.base.writes;
    if 9 + slots.len() > 128 {
        return Err("document frontier: numeric frontier exceeds 128 cells".into());
    }
    let empty = fields(slots, |_| "NULL".into());
    let columns = slots
        .iter()
        .map(|slot| format!(",c{slot}"))
        .collect::<String>();
    let mut entered = generation;
    entered.memo_bypass = true;
    entered.utility_depth = 0;
    entered.dynamic_depth = Some("s.level".into());
    let input = Demand {
        name: String::new(),
        slots: Slots::new(),
    };
    let (base, profile) = ordered::nested::program(
        s.base,
        &input,
        true,
        "s.cursor<=s.bound",
        slots,
        ("s.file", "s.cursor", "s.work"),
        entered,
    )?;
    let returned = fields(slots, |slot| {
        let index = slots.iter().position(|s| s == &slot).unwrap();
        format!("b.c{}", index + 2)
    });
    let mapped = fields(slots, |slot| {
        format!("CASE WHEN s.truth=1 THEN relation_parse_export(s.file,p.c0,s.c{slot}) END")
    });
    let seed_columns = [
        "file", "node", "cursor", "bound", "level", "handle", "phase", "truth", "work",
    ]
    .iter()
    .map(|name| format!("scan.{name}"))
    .chain(slots.iter().map(|slot| format!("scan.c{slot}")))
    .collect::<Vec<_>>()
    .join(",");
    let seed = |phase| format!("SELECT {seed_columns} WHERE scan.phase={phase}");
    let entry_seed = seed(0);
    let scan_seed = seed(1);
    let return_seed = seed(2);
    let filter = &s.filter;
    let first_candidate = format!(
        "COALESCE((SELECT candidate.id FROM node candidate WHERE candidate.file=p.parsed_file AND candidate.id>p.parsed_node AND candidate.id<=n.\"last\" AND ({filter}) ORDER BY candidate.id LIMIT 1),n.\"last\"+1)"
    );
    let next_candidate = format!(
        "COALESCE((SELECT candidate.id FROM node candidate WHERE candidate.file=s.file AND candidate.id>s.cursor AND candidate.id<=s.bound AND ({filter}) ORDER BY candidate.id LIMIT 1),s.bound+1)"
    );
    let entry = format!(
        "WITH s AS MATERIALIZED ({entry_seed}),e AS MATERIALIZED (SELECT s.*,CASE WHEN s.level<256 AND s.work>0 THEN s.work-1 END allowance,CASE WHEN s.level<256 AND s.work>0 THEN relation_peak_note(s.level+1) END noted FROM s),p AS MATERIALIZED (SELECT e.*,a.truth parsed_truth,a.work parsed_work,a.parsed_file,a.parsed_node FROM e LEFT JOIN relation_parse_budget(CASE WHEN e.level=0 THEN {} ELSE {} END,e.file,e.node,e.allowance) a ON 1) SELECT COALESCE(p.parsed_file,p.file),p.node,CASE WHEN p.parsed_truth=1 THEN {first_candidate} ELSE p.cursor END,COALESCE(n.\"last\",p.bound),p.level+CASE WHEN p.parsed_truth=1 THEN 1 ELSE 0 END,p.handle,CASE WHEN p.parsed_truth=1 THEN 1 ELSE 2 END,CASE WHEN p.level>=256 THEN -1 WHEN p.work=0 THEN -2 ELSE p.parsed_truth END,COALESCE(p.parsed_work,p.work){empty} FROM p LEFT JOIN node n ON n.file=p.parsed_file AND n.id=p.parsed_node WHERE p.noted IS NOT NULL OR p.allowance IS NULL",
        s.initial_term, s.nested_term
    );
    let scan = format!(
        "WITH s AS MATERIALIZED ({scan_seed}) SELECT s.file,CASE WHEN s.cursor<=s.bound THEN s.cursor ELSE s.node END,s.cursor,s.bound,s.level,COALESCE(frame.handle,s.handle),CASE WHEN s.cursor>s.bound OR b.c0 IS NOT 0 THEN 2 ELSE 0 END,CASE WHEN s.cursor>s.bound THEN 0 ELSE b.c0 END,COALESCE(b.c1,s.work){returned} FROM s {} LEFT JOIN relation_continuation frame ON frame.mode IS CASE WHEN scan.phase=1 AND s.cursor<=s.bound AND b.c0=0 THEN 1 END AND frame.parent_input IS s.handle AND frame.v0 IS s.file AND frame.v1 IS s.cursor AND frame.v2 IS {next_candidate} AND frame.v3 IS s.bound AND frame.v4 IS s.level",
        format_args!(
            "CROSS JOIN relation_row b ON ({}) IS ({base})",
            (0..2 + slots.len())
                .map(|i| format!("b.c{i}"))
                .collect::<Vec<_>>()
                .join(",")
        )
    );
    let unwind = format!(
        "WITH s AS MATERIALIZED ({return_seed}) SELECT p.c0,p.c1,p.c2,p.c3,p.c4,p.parent,CASE WHEN p.parent IS NULL THEN 3 WHEN s.truth=0 THEN 1 ELSE 2 END,s.truth,s.work{mapped} FROM s CROSS JOIN relation_continuation p ON p.mode IS CASE WHEN scan.phase=2 THEN 2 END AND p.h IS s.handle"
    );
    let phases = format!(
        "SELECT * FROM ({entry}) UNION ALL SELECT * FROM ({scan}) UNION ALL SELECT * FROM ({unwind})"
    );
    let next = (0..9 + slots.len())
        .map(|i| format!("r.c{i}"))
        .collect::<Vec<_>>()
        .join(",");
    let exports = s
        .exports
        .iter()
        .map(|(inner, _)| format!(",s.c{inner}"))
        .collect::<String>();
    let query = format!(
        "WITH RECURSIVE initial AS MATERIALIZED (SELECT f.handle FROM relation_continuation f WHERE f.mode IS CASE WHEN d.host THEN 1 END AND f.v0 IS d.file AND f.v1 IS d.node AND f.v2 IS 0 AND f.v3 IS 0 AND f.v4 IS 0),scan(file,node,cursor,bound,level,handle,phase,truth,work{columns}) AS (SELECT d.file,d.node,0,0,0,handle,0,0,d.work{empty} FROM initial UNION ALL SELECT {next} FROM scan {} WHERE scan.phase<>3) SELECT s.truth,s.work{exports} FROM scan s WHERE s.phase=3",
        ordered::nested::bridge(&phases, 9 + slots.len())
    );
    Ok((
        query,
        json!({"base":profile,"frontierPhases":3,"key":["document","node","parseTerm","nativeOwner","enclosingHost"],"memo":"parsed entries bypass; original one-use utility outputs are dead after publication","nativeComposition":false}),
    ))
}

/// Select only after the source identity, effect and private-capture proofs
/// succeed. Emission errors after admission are propagated to the common caller.
/// Prefix terminal IDs and clause owners are allocated before lowering.
pub(in crate::code_rule_dsl::sql::relational::outcomes) fn try_compile(
    grammars: &[&GrammarMatcher],
    work: i64,
    terminal_base: usize,
    clause_base: usize,
) -> Result<Option<Compiled>, String> {
    if work < 0 {
        return Err("parsed frontier: negative work allowance".into());
    }
    if grammars.len() != 1 {
        return Ok(None);
    }
    let g = grammars[0];
    if g.file.is_some() || g.transform.is_some() {
        return Ok(None);
    }
    let (first_source, utility) = match &g.matcher {
        AstMatcher::Any(v) if v.len() == 2 => match &v[1] {
            AstMatcher::Util(u) => (Some(&v[0]), u),
            _ => return Ok(None),
        },
        AstMatcher::Util(u) => (None, u),
        AstMatcher::All(v) if v.len() == 2 && matches!(&v[0], AstMatcher::Kind(_)) => match &v[1] {
            AstMatcher::Util(u) => (None, u),
            _ => return Ok(None),
        },
        _ => return Ok(None),
    };
    if first_source.is_some_and(|m| !source_safe(m, utility, &mut HashSet::new())) {
        return Ok(None);
    }
    let mut lower = Lower::new(g.language, g.three_valued);
    lower.pure.program.terminals = super::super::super::seed_terminals(terminal_base);
    lower.root_kinds = g.anchors.root_kinds().cloned();
    lower.scope_owner = clause_base
        .checked_add(1)
        .ok_or("parsed frontier: clause namespace overflow")?;
    // Successful ordinary lowering is part of proving that each finite region
    // is available to this representation; another common representation may
    // still handle a source whose inline regions cannot be proved here.
    let root = match lower.plan(&g.matcher) {
        Ok(plan) => plan,
        Err(_) => return Ok(None),
    };
    let shape = match shape(&root, utility) {
        Ok(shape) => shape,
        Err(_) => return Ok(None),
    };
    if shape.first.is_some_and(|p| p.effects.reads_value)
        || lower.slots.iter().any(|s| s.kind != CaptureKind::Node)
    {
        return Ok(None);
    }
    let mut sql = Sql {
        definitions: vec![],
        profile: vec![],
        correlated: false,
        generation: Generation::default(),
    };
    sql.generation.peak_tracking = true;
    sql.generation.memo_clause = Some(clause_base);
    sql.generation.components.borrow_mut().clause = clause_base;
    sql.generation.memo_layout = lower.memo_layout(true);
    let rejection = rejected_prefix(shape.first, first_source, &mut lower);
    let class = rejection.as_ref().map_or("0", |r| r.condition.as_str());
    let reset = sql.relation(
        "parsed frontier utility storage reset",
        format!("SELECT relation_memo_reset({clause_base}) ready"),
        true,
        "ready",
    );
    let filter = g.anchors.root_kinds().map_or("1".into(), |ks| {
        format!(
            "n.kind IN ({})",
            ks.iter().map(u16::to_string).collect::<Vec<_>>().join(",")
        )
    });
    let roots=Demand{name:sql.relation("parsed frontier classified native roots",format!("SELECT ROW_NUMBER() OVER () k,n.file,n.id node,{work} work,ROW_NUMBER() OVER (PARTITION BY n.file ORDER BY n.id) ordinal,n.kind IN ({}) host,({class}) prefix_may_hold FROM node n CROSS JOIN {reset} i ON i.ready=1 WHERE {} AND ({filter})",shape.hosts.iter().map(u16::to_string).collect::<Vec<_>>().join(","),usize::from(g.applies)),true,"native candidate population, host class"),slots:Slots::new()};
    // A single-use original-file utility has no future lookups for rejected
    // (file,node) keys. Recursive calls enter SubDocs and bypass native memo.
    // Only this domain can combine rejected roots into a fixed-entry gap.
    let bulk = rejection.is_some();
    let rejection_cost = 1 + rejection.as_ref().map_or(0, |r| r.cost);
    let roots = if bulk {
        let name=sql.relation("parsed frontier host events and final rejected suffix",format!("WITH events AS (SELECT k,file,node,work,ordinal native_ordinal,1 real,host FROM {} WHERE host OR prefix_may_hold UNION ALL SELECT NULL,file,NULL,{work},MAX(ordinal)+1,0,0 FROM {} GROUP BY file) SELECT k,file,node,work,ROW_NUMBER() OVER (PARTITION BY file ORDER BY native_ordinal) ordinal,host,(native_ordinal-COALESCE(LAG(native_ordinal) OVER (PARTITION BY file ORDER BY native_ordinal),0)-1)*{rejection_cost} gap,real FROM events",roots.name,roots.name),true,"host demand plus exact fixed-entry gaps; final suffix retained");
        Demand {
            name,
            slots: Slots::new(),
        }
    } else {
        let name = sql.relation(
            "parsed frontier roots with nonelidable first-arm trace",
            format!("SELECT *,0 gap,1 real FROM {}", roots.name),
            true,
            "first-arm utility work prevents free structural filtering",
        );
        Demand {
            name,
            slots: Slots::new(),
        }
    };
    sql.generation.components.borrow_mut().roots = roots.name.clone();
    let active =
        2 + root.writes.len() + 2 * (2 + shape.exports.len()) + 9 + shape.base.writes.len();
    if active > 128 {
        return Err(
            "document frontier: active typed returns exceed unchanged 128 cell budget".into(),
        );
    }
    sql.generation.return_cells = active;
    sql.generation.return_peak.set(active);
    for _ in 0..3 {
        sql.generation
            .operator_as("local parsed-document frontier")?;
    }
    let (frontier, frontier_profile) = frontier(&shape, sql.generation.clone())?;
    let outputs = &root.writes;
    let second_width = 2 + shape.exports.len();
    let no_outputs = shape.exports.iter().map(|_| ",NULL").collect::<String>();
    let returned = (0..second_width)
        .map(|i| format!("r.c{i}"))
        .collect::<Vec<_>>()
        .join(",");
    let eligible = ordered::nested::bridge(&frontier, second_width);
    // Wrong literal kind is an atomic metadata fact, but still a reached
    // utility entry. No parse cache fill or memo replay can occur on this path.
    let second_query = format!(
        "SELECT CASE WHEN d.work=0 THEN -2 ELSE 0 END,MAX(0,d.work-1){no_outputs} FROM (SELECT 1) unused WHERE NOT d.host UNION ALL SELECT {returned} FROM (SELECT 1 reached WHERE d.host) allowed {eligible}"
    );
    let second = ordered::nested::bridge(&second_query, second_width);
    let first = if let Some(first) = shape.first {
        ordered::nested::program(
            first,
            &roots,
            true,
            "w.work>=d.gap AND d.real",
            &first.writes,
            ("d.file", "d.node", "w.work-d.gap"),
            sql.generation.clone(),
        )?
        .0
    } else {
        "SELECT 0,w.work-d.gap".into()
    };
    let mut choice = vec![];
    for slot in outputs {
        let first_index = shape
            .first
            .and_then(|p| p.writes.iter().position(|s| s == slot));
        let second_index = shape.exports.iter().position(|(_, outer)| outer == slot);
        let before = first_index.map_or("NULL".into(), |i| format!("f.c{}", i + 2));
        let after = second_index.map_or("NULL".into(), |i| format!("a.c{}", i + 2));
        choice.push(format!(
            "CASE WHEN f.c0 IS NOT 0 THEN {before} ELSE {after} END"
        ));
    }
    let captures = if choice.is_empty() {
        String::new()
    } else {
        format!(
            ",{}",
            choice
                .iter()
                .map(|c| format!("CASE WHEN w.work>=d.gap AND d.real THEN {c} END"))
                .collect::<Vec<_>>()
                .join(",")
        )
    };
    // The second query is opened only after the first arm returned false. Its
    // d binding carries that arm's remaining allowance, not the root allowance.
    let first_bridge = format!(
        "CROSS JOIN relation_row f ON ({}) IS ({first})",
        (0..2 + shape.first.map_or(0, |p| p.writes.len()))
            .map(|i| format!("f.c{i}"))
            .collect::<Vec<_>>()
            .join(",")
    );
    let query = format!(
        "SELECT CASE WHEN w.work<d.gap THEN -2 WHEN NOT d.real THEN 0 WHEN f.c0 IS NOT 0 THEN f.c0 ELSE a.c0 END,CASE WHEN w.work<d.gap THEN 0 WHEN NOT d.real THEN w.work-d.gap WHEN f.c0 IS NOT 0 THEN f.c1 ELSE a.c1 END{captures} FROM (SELECT 1) seed {} LEFT JOIN relation_row a ON ({}) IS (SELECT {} FROM (SELECT d.file,d.node,f.c1 work,d.host WHERE f.c0=0 AND d.real AND w.work>=d.gap) d {second})",
        first_bridge,
        (0..second_width)
            .map(|i| format!("a.c{i}"))
            .collect::<Vec<_>>()
            .join(","),
        (0..second_width)
            .map(|i| format!("r.c{i}"))
            .collect::<Vec<_>>()
            .join(",")
    );
    let empty = fields(outputs, |_| "NULL".into());
    let vals = outputs
        .iter()
        .enumerate()
        .map(|(i, _)| format!(",r.c{}", i + 2))
        .collect::<String>();
    let walk = format!("outcome{}", sql.definitions.len());
    sql.relation("parsed frontier reached roots",format!("SELECT NULL k,file,NULL node,0 ordinal,0 truth,{work} work{empty} FROM {} GROUP BY file UNION ALL SELECT d.k,d.file,d.node,d.ordinal,r.c0,r.c1{vals} FROM {walk} w CROSS JOIN {} d ON d.file=w.file AND d.ordinal=w.ordinal+1 {} WHERE w.truth>=0",roots.name,roots.name,ordered::nested::bridge(&query,2+outputs.len())),false,"ordered root work and local document frontier");
    sql.publish_components();
    let raw = sql.relation(
        "parsed frontier provisional output",
        format!(
            "SELECT w.file,w.node,w.truth,w.work{} FROM {walk} w WHERE w.truth=1 OR w.truth<0",
            fields(outputs, |slot| format!("w.c{slot}"))
        ),
        true,
        "truth,work,typed captures",
    );
    let failures = sql.relation(
        "parsed frontier file failures",
        format!("SELECT * FROM {raw} WHERE truth<0"),
        true,
        "file failure",
    );
    let mut observed = outputs.iter().copied().collect::<Vec<_>>();
    observed.sort_by_key(|s| &lower.slots[*s].name);
    let payload = observed
        .iter()
        .map(|slot| {
            lower.slots[*slot].kind.payload(
                &format!("m.c{slot}"),
                "m.file",
                sql.generation
                    .memo_layout
                    .as_ref()
                    .is_some_and(|l| l.span_capable(*slot)),
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    let query = format!(
        "WITH RECURSIVE {} SELECT {clause_base},m.file,m.node,json_array({payload}),0,m.work FROM {raw} m WHERE m.truth=1 AND NOT EXISTS (SELECT 1 FROM {failures} f WHERE f.file=m.file) UNION ALL SELECT {clause_base},file,NULL,'[]',truth,work FROM {failures} ORDER BY 1,2,3",
        sql.definitions.join(",\n")
    );
    if query.len() > 256000 {
        return Err("document frontier: SQL exceeds unchanged 256000 byte cap".into());
    }
    let diagnostic_sql = format!(
        "WITH RECURSIVE {} SELECT f.file,COALESCE((SELECT last.work FROM {walk} last WHERE last.file=f.file ORDER BY last.ordinal DESC LIMIT 1),{work}) FROM (SELECT DISTINCT file FROM node) f ORDER BY f.file",
        sql.definitions.join(",\n")
    );
    let mut lowering = lower.profile(terminal_base);
    lowering["finiteControlOperators"] = json!(0);
    let mut relations = json!({"nativeComposition":false,"failureColumn":4,"remainingWorkColumn":5,"terminalOwners":vec![clause_base;lower.pure.program.terminals.len()],"captures":observed.iter().map(|s|json!({"name":lower.slots[*s].name,"slot":s,"kind":"node"})).collect::<Vec<_>>(),"relations":sql.profile,"compositionOperators":[sql.generation.operators.get()],"frontier":frontier_profile,"workDiagnosticsSql":diagnostic_sql,"frontierAdmission":{"fullRuleTransfer":shape.first.is_some(),"bulkOriginalRoots":bulk,"rejectedRootCost":rejection_cost,"rejectedPrefixCost":rejection.as_ref().map(|r|r.cost),"ordinaryComposition":"existing finite SQL regions; no parsed control bytecode"}});
    relations["scope"] = json!("Workspace clause/file failure publication kernel");
    relations["physicalShape"] = json!("parsed document frontier");
    relations["lowering"] = json!([lowering]);
    relations["nestedReturnCells"] = json!([sql.generation.return_peak.get()]);
    relations["nestedReturnCellBudget"] = json!(128);
    Ok(Some(Compiled {
        sql: query,
        terminals: lower.pure.program.terminals,
        relations,
    }))
}

#[cfg(test)]
#[path = "parsed_frontier_tests.rs"]
mod tests;
