use super::*;
use crate::code_rule_dsl::logic::{native::component, profile, rr};

const SPACE: [u8; 32] = [51; 32];
const KEY: [u8; 32] = [17; 32];

/// A program's strata, as a run computes them once and passes them on.
/// A program's version, as a run reads it (t36 item 7).
fn pl(p: &front::Program) -> crate::code_rule_dsl::logic::plan::body::Placed { crate::code_rule_dsl::logic::plan::body::Placed::of_program(p).unwrap() }
fn st(p: &front::Program) -> Vec<BTreeSet<String>> { front::strata(p).unwrap().0 }

/// An entry of `rows`, flat, of `types`' columns, at partition `part`: a relation's (part 0) in canonical order, a
/// granule's or statistics' words as given (`retain::kept_flat`).
fn put(rel: &KeptRel, part: i64, digest: u64, reads: Vec<(String, u64)>, types: Vec<u8>, rows: Vec<i64>) -> KeptPut {
    let n = if types.is_empty() { 0 } else { rows.len() / types.len() };
    KeptPut { rel: rel.name.clone(), ident: rel.ident, part, digest, reads, what: crate::content_store::kept_facts::KeptWhat::Flat { widths: vec![8; types.len()], rows, n, levels: None, given: part != DOCS_PART }, types }
}
fn gone(rel: &KeptRel, part: i64) -> KeptPut {
    KeptPut { rel: rel.name.clone(), ident: rel.ident, part, digest: 0, reads: vec![], types: vec![], what: crate::content_store::kept_facts::KeptWhat::Gone }
}

fn fixture(store: &ContentStore, count: usize) -> (front::Program, HashMap<String, u64>, InKept) {
    let mut text = String::new();
    for i in 0..count {
        text += &format!(".decl input{i}(x:number)\n.input input{i}\n.decl result{i}(x:number)\nresult{i}(x) :- input{i}(x).\n");
    }
    let prog = rr::parse(&text).unwrap();
    let empty = Retained::bind_kept(store,SPACE,KEY,KEY,&pl(&prog),&st(&prog), &HashMap::new()).unwrap();
    let frames = empty.frames.unwrap();
    let mut puts = Vec::new();
    let mut now = HashMap::new();
    for i in 0..count {
        let input = format!("input{i}");
        let result = format!("result{i}");
        let rows: Vec<i64> = (1..=3).map(|d| (d << 32) | i as i64).collect();
        let source = component::columns_source(vec!["c0".into()], vec![Some(rows.clone())]);
        let docs = docs_of(source.as_ref(), 0, &crate::code_rule_dsl::logic::plan::Cells::plain(1));
        let d = digest(source.as_ref(), &crate::code_rule_dsl::logic::plan::Cells::plain(1));
        assert_eq!(digest_of_docs(&docs), d);
        now.insert(input.clone(), d);
        puts.push(put(&frames.names[&input], DOCS_PART, d, vec![], vec![b'i';3], docs.iter().flat_map(|(doc,(sum,n))| [*doc,*sum as i64,*n as i64]).collect()));
        for &row in &rows {
            puts.push(put(&frames.names[&input], granule(row >> 32, None), 0, vec![], vec![b'i'], vec![row]));
        }
        let rd = crate::code_rule_dsl::logic::plan::rows_digest(&crate::code_rule_dsl::logic::plan::Cells::plain(1), &rows, 3);
        puts.push(put(&frames.names[&result], DOCS_PART, rd, vec![(input,d)], vec![b'i'], rows));
    }
    store.queue_kept(SPACE, puts);
    (prog, now, frames)
}

#[test]
fn k4_admission_batches_input_digests_derived_rows_and_mixed_granules() {
    let _run = profile::one_run();
    profile::set(true);
    let dir = tempfile::tempdir().unwrap();
    let store = ContentStore::open(&dir.path().join("facts.sqlite")).unwrap();
    let (prog, now, frames) = fixture(&store, 131);
    store.queue_kept(SPACE, vec![put(&frames.names["result130"], DOCS_PART, 99, vec![("input130".into(),now["input130"].wrapping_add(1))], vec![b'i'], vec![999])]);
    let changed: HashMap<_,_> = now.iter().map(|(r,d)|(r.clone(),d.wrapping_add(1))).collect();
    profile::reset(); profile::set(true);
    let wave = profile::waves();
    let span = profile::span(|| "bulk K4 admission".into());
    let mut kept = Retained::bind_kept(&store,SPACE,KEY,KEY,&pl(&prog),&st(&prog), &HashMap::new()).unwrap();
    let cut = cut(Some(&kept),KEY,&pl(&prog),&st(&prog), &changed).unwrap();
    kept.load(&store,&cut,&pl(&prog),false).unwrap();
    assert_eq!(kept.input_srcs.len(), 131);
    assert_eq!(kept.rels.len(), 130);
    for i in 0..131 {
        let source = &kept.input_srcs[&format!("input{i}")];
        assert_eq!(source.len(),3);
        assert_eq!((0..3).map(|r|source.value(r,0)).collect::<Vec<_>>(),(1..=3).map(|d|(d<<32)|i as i64).collect::<Vec<_>>());
    }
    drop(span); drop(wave);
    let statements:u64=profile::sql_waits().iter().filter(|(p,s,_,_)|p.starts_with("bulk K4 admission")&&s.starts_with("store:")).map(|x|x.2).sum();
    profile::set(false);
    eprintln!("K4: 131 input headers, 393 granules, 130 derived entries: {statements} storage executions");
    assert!(statements<=16,"the space's blocks are bound once, not read a query per entry: {statements}");
}

#[test]
fn k4_clean_outputs_are_admitted_together_without_rerunning_their_producers() {
    let _run=profile::one_run();
    profile::set(true);
    let dir=tempfile::tempdir().unwrap();
    let store=ContentStore::open(&dir.path().join("facts.sqlite")).unwrap();
    let (mut prog,_,_)=fixture(&store,130);
    prog.outputs=(0..130).map(|i|format!("result{i}")).collect();
    profile::reset(); profile::set(true);
    let wave=profile::waves();
    let span=profile::span(||"bulk output admission".into());
    let mut kept=Retained::bind_kept(&store,SPACE,KEY,KEY,&pl(&prog),&st(&prog),&HashMap::new()).unwrap();
    assert!(prog.outputs.iter().all(|r|kept.holds(r,&pl(&prog))),"raw output facts are sufficient to keep a derivation");
    let cut=Cut{clean:prog.outputs.iter().cloned().collect(),strata:(0,130),changed:vec![]};
    assert_eq!(kept.load(&store,&cut,&pl(&prog),false).unwrap(),Some((130,390)));
    for i in 0..130 {
        let rel=&kept.rels[&format!("result{i}")];
        let source=rel.source(&format!("result{i}"));
        assert_eq!((0..source.len()).map(|r|source.value(r,0)).collect::<Vec<_>>(),(1..=3).map(|d|(d<<32)|i as i64).collect::<Vec<_>>());
        assert!(rel.output.is_none(),"decoding belongs at the output boundary");
    }
    drop(span); drop(wave);
    let statements:u64=profile::sql_waits().iter().filter(|(p,s,_,_)|p.starts_with("bulk output admission")&&s.starts_with("store:")).map(|x|x.2).sum();
    profile::set(false);
    assert!(statements<=16,"130 outputs are bound from one mapping of the space, not a query each: {statements}");
}

#[test]
fn k4_retains_consumers_of_literal_facts_and_invalidates_them_when_the_literal_changes() {
    let dir=tempfile::tempdir().unwrap();
    let store=ContentStore::open(&dir.path().join("facts.sqlite")).unwrap();
    let text=".decl literal(x:number)\nliteral(1).\n.decl result(x:number)\nresult(x) :- literal(x), x > 0.\n.output result\n";
    let prog=rr::parse(text).unwrap();
    let empty=Retained::bind_kept(&store,SPACE,KEY,KEY,&pl(&prog),&st(&prog),&HashMap::new()).unwrap();
    let initial=cut(Some(&empty),KEY,&pl(&prog),&st(&prog),&HashMap::new()).unwrap();
    let computed=Retained{key:KEY,rels:HashMap::from([("result".into(),Kept{
        rel:InRel::flat(&[vec![1]],vec![b'i']),width:1,output:None,
    })]),..Default::default()};
    assert_eq!(computed.write_kept(&store,empty.in_kept().unwrap(),&pl(&prog),&Default::default(),&Locations::new(),&st(&prog),&initial,&[],&HashMap::new(),&BTreeSet::new(),&HashMap::new(),HashMap::new(),None,&HashMap::new(), &BTreeSet::new()).unwrap(),1);
    let restored=Retained::bind_kept(&store,SPACE,KEY,KEY,&pl(&prog),&st(&prog),&HashMap::new()).unwrap();
    assert_eq!(cut(Some(&restored),KEY,&pl(&prog),&st(&prog),&HashMap::new()).unwrap().strata,(0,1));
    let changed=rr::parse(&text.replace("literal(1).","literal(2).")).unwrap();
    let restored=Retained::bind_kept(&store,SPACE,KEY,KEY,&pl(&changed),&st(&changed),&HashMap::new()).unwrap();
    assert_eq!(cut(Some(&restored),KEY,&pl(&changed),&st(&changed),&HashMap::new()).unwrap().strata,(1,1),"literal rows participate in the relation's identity");
    let external=rr::parse(&format!("{text}\n.input literal\n")).unwrap();
    let empty=Retained::bind_kept(&store,[52;32],KEY,KEY,&pl(&external),&st(&external),&HashMap::new()).unwrap();
    let initial=cut(Some(&empty),KEY,&pl(&external),&st(&external),&HashMap::new()).unwrap();
    assert_eq!(computed.write_kept(&store,empty.in_kept().unwrap(),&pl(&external),&Default::default(),&Locations::new(),&st(&external),&initial,&[],&HashMap::new(),&BTreeSet::new(),&HashMap::new(),HashMap::new(),None,&HashMap::new(), &BTreeSet::new()).unwrap(),0,
        "a relation with literal rows AND external inputs still needs a runtime dependency digest");
}

/// A run reads one commit of the space's blocks (decision 0401 F3): a derived entry replaced after the run bound the
/// space is not seen by it. Its old rows are the commit's, as its digests are: old metadata never certifies new rows.
#[test]
fn a_derived_entry_replaced_after_the_run_bound_its_space_is_not_seen() {
    let dir=tempfile::tempdir().unwrap();
    let store=ContentStore::open(&dir.path().join("facts.sqlite")).unwrap();
    let (prog,now,frames)=fixture(&store,1);
    let mut kept=Retained::bind_kept(&store,SPACE,KEY,KEY,&pl(&prog),&st(&prog), &HashMap::new()).unwrap();
    let changed=now.iter().map(|(r,d)|(r.clone(),d.wrapping_add(1))).collect();
    let cut=cut(Some(&kept),KEY,&pl(&prog),&st(&prog),&changed).unwrap();
    store.queue_kept(SPACE, vec![put(&frames.names["result0"], DOCS_PART, 99, vec![], vec![b'i'], vec![999])]);
    assert!(kept.load(&store,&cut,&pl(&prog),false).unwrap().is_some());
    assert_eq!(kept.rels["result0"].rel.live_rows().unwrap(), (1..=3).map(|d| d << 32).collect::<Vec<i64>>(), "the bound commit's rows");
    // The next run reads the new commit.
    let next=Retained::bind_kept(&store,SPACE,KEY,KEY,&pl(&prog),&st(&prog), &HashMap::new()).unwrap();
    assert_eq!(next.in_kept().unwrap().derived["result0"].digest, 99);
}

/// An input granule replaced after the run bound the space is not seen by it: the old side of maintenance is the
/// commit's rows.
#[test]
fn an_input_granule_replaced_after_the_run_bound_its_space_is_not_seen() {
    let dir=tempfile::tempdir().unwrap();
    let store=ContentStore::open(&dir.path().join("facts.sqlite")).unwrap();
    let (prog,now,frames)=fixture(&store,1);
    let mut kept=Retained::bind_kept(&store,SPACE,KEY,KEY,&pl(&prog),&st(&prog), &HashMap::new()).unwrap();
    let changed=now.iter().map(|(r,d)|(r.clone(),d.wrapping_add(1))).collect();
    let cut=cut(Some(&kept),KEY,&pl(&prog),&st(&prog),&changed).unwrap();
    store.queue_kept(SPACE, vec![put(&frames.names["input0"], granule(1, None), 0, vec![], vec![b'i'], vec![(1<<32)|999])]);
    assert!(kept.load(&store,&cut,&pl(&prog),false).unwrap().is_some());
    let src=&kept.input_srcs["input0"];
    assert_eq!((0..src.len()).map(|r| src.value(r,0)).collect::<Vec<_>>(), (1..=3).map(|d| d << 32).collect::<Vec<i64>>(), "the bound commit's rows");
}

/// An input whose documents' entry is gone is not kept: its readers derive again, cold. A run that bound the space
/// before still reads the entry, of its commit.
#[test]
fn an_input_whose_documents_entry_is_gone_runs_its_readers_again() {
    let dir = tempfile::tempdir().unwrap();
    let store = ContentStore::open(&dir.path().join("facts.sqlite")).unwrap();
    let (prog, now, frames) = fixture(&store, 1);
    let before = Retained::bind_kept(&store,SPACE,KEY,KEY,&pl(&prog),&st(&prog), &HashMap::new()).unwrap();
    store.queue_kept(SPACE, vec![gone(&frames.names["input0"], DOCS_PART)]);
    assert!(!before.in_kept().unwrap().inputs.is_empty(), "bound before: its commit's entry");
    let kept = Retained::bind_kept(&store,SPACE,KEY,KEY,&pl(&prog),&st(&prog), &HashMap::new()).unwrap();
    assert!(kept.inputs.is_empty());
    assert!(kept.in_kept().unwrap().derived.is_empty());
    let cold = cut(Some(&kept),KEY,&pl(&prog),&st(&prog), &now).unwrap();
    assert_eq!(cold.strata, (1, 1));
    assert!(cold.clean.is_empty());
}

/// A changed input with a granule its documents' entry names but the space does not keep fails the load: the cut
/// is recomputed and its readers derive again, and nothing kept of the old frames is republished as current.
#[test]
fn failed_k4_admission_makes_a_clean_consumer_run_again() {
    let dir = tempfile::tempdir().unwrap();
    let store = ContentStore::open(&dir.path().join("facts.sqlite")).unwrap();
    let (prog, now, frames) = fixture(&store, 1);
    store.queue_kept(SPACE, vec![gone(&frames.names["input0"], granule(2, None))]);
    let mut kept = Retained::bind_kept(&store,SPACE,KEY,KEY,&pl(&prog),&st(&prog), &HashMap::new()).unwrap();
    let changed = now.iter().map(|(r,d)|(r.clone(),d.wrapping_add(1))).collect();
    let cut = cut(Some(&kept),KEY,&pl(&prog),&st(&prog),&changed).unwrap();
    assert!(kept.load(&store, &cut, &pl(&prog), false).unwrap().is_none());
    assert!(kept.input_srcs.is_empty() && kept.rels.is_empty());
    let rerun = cut_again(&kept, &prog, &now);
    assert!(rerun.clean.is_empty());
    assert_eq!(rerun.strata.0, rerun.strata.1);
    assert!(kept.in_kept().unwrap().derived.is_empty(), "new results must be republished");
}

fn cut_again(kept: &Retained, prog: &front::Program, now: &HashMap<String, u64>) -> Cut {
    cut(Some(kept),KEY,&pl(prog),&st(prog), now).unwrap()
}

/// Smoking gun 3 (t34, 2026-11-13): an export's kept rows are grouped by the column the memo attributes them by (the
/// split's locations), not by the first node column. `p197_arg_at(d, i, a: node)` was kept by column 2 and the memo
/// attributed it by column 0, so every edit refused the kept inputs ("memo entry is gone").
#[test]
fn kept_rows_are_grouped_by_the_column_the_memo_attributes_them_by() {
    let prog = rr::parse(".decl arg_at(d:number, i:number, a:node)\n.input arg_at\n").unwrap();
    assert_eq!(group_col(&pl(&prog), &Locations::new(), "arg_at"), 2, "with no location: the node column");
    let located: Locations = [("arg_at".to_owned(), vec![0])].into_iter().collect();
    assert_eq!(group_col(&pl(&prog), &located, "arg_at"), 0, "the memo's location column");
}

/// Smoking gun 4 (t34, 2026-11-13): whether an edit takes the kept inputs is known before the first wave, with the
/// reason when it does not (decision 0104: the waves read no entry of the store). The fallback found out inside the
/// waves and read the store there.
#[test]
fn the_kept_inputs_are_known_before_the_waves_with_why_not() {
    let dir = tempfile::tempdir().unwrap();
    let store = ContentStore::open(&dir.path().join("facts.sqlite")).unwrap();
    let prog = rr::parse(".decl input0(x:number)\n.input input0\n.decl result0(x:number)\nresult0(x) :- input0(x).\n").unwrap();
    let why = kept_inputs_stored(&store, SPACE, KEY, &pl(&prog), &["input0".to_owned()]).unwrap().expect_err("an empty store holds no kept inputs");
    assert!(why.contains("input0"), "the reason names the input: {why}");
    let (prog, _, _) = fixture(&store, 1);
    kept_inputs_stored(&store, SPACE, KEY, &pl(&prog), &["input0".to_owned()]).unwrap().expect("the frames of its documents are stored");
}

/// Q151 (2026-11-16): the check runs before the run's own space is bound, in a process an earlier run of which bound
/// the space before it kept anything. Through that earlier binding no input had an entry, and every edit of a phased
/// run decoded its memo hits whole. The check reads the space's last commit (`Bound::of`).
#[test]
fn the_kept_inputs_are_found_past_an_earlier_runs_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let store = ContentStore::open(&dir.path().join("facts.sqlite")).unwrap();
    let early = rr::parse(".decl input0(x:number)\n.input input0\n").unwrap();
    store.queue_kept(SPACE, vec![put(&KeptRel { name: "other".into(), ident: [9; 32] }, 0, 1, vec![], vec![b'i'], vec![1])]);
    let earlier = kept_read::Bound::of(&store, &SPACE).unwrap().unwrap();
    assert!(earlier.entry("input0", &input_ident("input0", &pl(&early), &KEY), DOCS_PART).is_none(), "the earlier binding holds no entry of the input");
    let (prog, _, _) = fixture(&store, 1);
    kept_inputs_stored(&store, SPACE, KEY, &pl(&prog), &["input0".to_owned()]).unwrap().expect("found in the space's last commit");
}

/// Q20: a kept input is bound where its granules lie, not decoded before the cross part runs: its rows' count and
/// each owner's range come from its documents' entry, and no row is read to bind it. A granule whose rows are not as
/// many as its owners' is refused.
#[test]
fn a_kept_input_is_bound_where_its_granules_lie() {
    let dir = tempfile::tempdir().unwrap();
    let store = ContentStore::open(&dir.path().join("facts.sqlite")).unwrap();
    let (prog, _, frames) = fixture(&store, 2);
    let rels = ["input0".to_owned(), "input1".to_owned()];
    let kept = kept_inputs(&store, SPACE, KEY, &pl(&prog), &rels, &BTreeSet::new()).unwrap().expect("the inputs' entries are as kept");
    let (good, other) = (&kept["input0"].source, &kept["input1"].source);
    assert_eq!((good.len(), other.len()), (3, 3), "each input's rows counted from its granules' entries");
    let one = kept["input1"].ranges().unwrap()[&3];
    assert_eq!(other.value(one.0 as usize, 0), (3 << 32) | 1, "input1's row of document 3, from its range");
    let mut rows: Vec<i64> = (0..3).map(|r| good.value(r, 0)).collect();
    rows.sort_unstable();
    assert_eq!(rows, (1..=3).map(|d| d << 32).collect::<Vec<i64>>(), "input0 read as kept");
    // input1's granule of document 2 holds two rows: more than its owner's.
    store.queue_kept(SPACE, vec![put(&frames.names["input1"], granule(2, None), 0, vec![], vec![b'i'], vec![(2 << 32) | 1, (7 << 32) | 1])]);
    let Err(why) = kept_inputs(&store, SPACE, KEY, &pl(&prog), &rels, &BTreeSet::new()).unwrap() else { panic!("a granule not of its owners' rows is refused") };
    assert!(why.contains("input1") && why.contains("owners"), "{why}");
}

/// Smoking gun 5 (t34, 2026-11-13): the retain budget is of memory, not of the store. A relation past it was neither
/// held nor written to the frames, so every next process derived it again (an edit ran 3,758 of 4,087 strata, 1,546
/// derived again). It is written, then let go.
#[test]
fn a_relation_past_the_retain_budget_is_written_to_the_frames_and_let_go() {
    let dir = tempfile::tempdir().unwrap();
    let store = ContentStore::open(&dir.path().join("facts.sqlite")).unwrap();
    let (prog, now, _) = fixture(&store, 2);
    let was = Retained::bind_kept(&store, SPACE, KEY,KEY, &pl(&prog), &st(&prog), &HashMap::new()).unwrap().frames.unwrap();
    let cut = cut(None, KEY, &pl(&prog), &st(&prog), &now).unwrap();
    let rows = |base: i64| component::InRel::flat(&(0..4).map(|i| vec![base + i]).collect::<Vec<_>>(), vec![b'i']);
    let derived = vec![("result0".to_owned(), 1, rows(100)), ("result1".to_owned(), 1, rows(200))];
    let mut next = after(None, KEY, now.clone(), &cut, derived, &BTreeMap::new(), &[], HashMap::new(), HashMap::new(), HashMap::new(), Default::default(), HashMap::new(), 0, false);
    assert_eq!(next.bytes(), 0, "nothing past the budget is held for the next run");
    assert_eq!(next.parts().1.len(), 2, "both are this run's to write");
    let written = next.write_kept(&store, &was, &pl(&prog),&Default::default(), &Locations::new(), &st(&prog), &cut, &[], &now, &BTreeSet::new(), &HashMap::new(),HashMap::new(),None,&HashMap::new(), &BTreeSet::new()).unwrap();
    assert_eq!(written, 8, "the rows past the budget are written to the frames");
    next.let_go_past();
    assert!(next.parts().1.is_empty(), "and then let go");
}

/// Smoking gun 9 (t34, 2026-11-13), red until fixed. A kept relation's statistics are a part of its frame under its
/// rows' digest (decision 0120 §3), admitted with its rows by `load`. Rows read where the walk reaches them (`admit`,
/// decision 0117 step 4) took the rows and their levels, not the statistics, so every `__old` the edit planned with
/// was counted from its rows ("count run_scope__old from its rows", dozens an edit).
#[test]
fn rows_read_where_reached_come_with_their_kept_statistics() {
    let dir = tempfile::tempdir().unwrap();
    let store = ContentStore::open(&dir.path().join("facts.sqlite")).unwrap();
    let (prog, _, frames) = fixture(&store, 1);
    let stored = Retained::bind_kept(&store, SPACE, KEY,KEY, &pl(&prog), &st(&prog), &HashMap::new()).unwrap();
    let digest = stored.frames.as_ref().unwrap().derived["result0"].digest;
    let mut all = kept_stats::ByDigest::new();
    all.insert(("result0".to_owned(), digest), kept_stats::KeptStat::Measured((vec![("result0_c0".to_owned(), 3, 3, 3, 3)], vec![], BTreeSet::new())));
    let words = kept_stats::to_words(&kept_stats::encode(&all));
    store.queue_kept(SPACE, vec![put(&frames.names["result0"], kept_stats::PART, digest, vec![], vec![b'i'], words)]);
    let kept = Retained::bind_kept(&store, SPACE, KEY,KEY, &pl(&prog), &st(&prog), &HashMap::new()).unwrap();
    assert!(kept.admit(&store, &pl(&prog), &["result0".to_owned()]).unwrap(), "its rows are read where reached");
    let at = Arc::as_ptr(&kept.late.borrow()["result0"].rel) as *const u8 as usize;
    let held = Arc::clone(&kept.late.borrow()["result0"].rel);
    assert_eq!(Arc::as_ptr(&held) as *const u8 as usize, at);
    assert!(kept.measured("result0", &held).is_some(), "its statistics come with its rows: not counted from them");
    assert!(kept.measured_all().contains_key(&("result0".to_owned(), at)), "and are carried to the next run");
}

/// A granule the space no longer keeps (forgotten, or never written) is a miss its reader is told of, so the part is
/// derived again: it was a failed run (`granule 9 of its stored rows is missing`, phase 0's rounds).
#[test]
fn an_inputs_missing_granule_is_a_miss_not_a_failure() {
    let dir = tempfile::tempdir().unwrap();
    let store = ContentStore::open(&dir.path().join("facts.sqlite")).unwrap();
    let (prog, _, frames) = fixture(&store, 1);
    let kept = Retained::bind_kept(&store, SPACE, KEY,KEY, &pl(&prog), &st(&prog), &HashMap::new()).unwrap();
    let all = kept.read_input(&store, &pl(&prog), &Locations::new(), "input0", None).unwrap();
    assert_eq!(all.unwrap().unwrap().len(), 3);
    store.queue_kept(SPACE, vec![gone(&frames.names["input0"], granule(2, None))]);
    let kept = Retained::bind_kept(&store, SPACE, KEY,KEY, &pl(&prog), &st(&prog), &HashMap::new()).unwrap();
    let why = kept.read_input(&store, &pl(&prog), &Locations::new(), "input0", None).unwrap().unwrap_err();
    assert!(why.contains(&format!("granule {} of its rows is not kept", granule(2, None))), "{why}");
}

/// Decision 0212: a kept frame's digests and granules are of its rows' content, not of a process's numbering of their
/// atoms. Two stores that numbered the same strings otherwise (as a forked process and the store do) digest the same
/// rows alike, part a text-owned input's owners alike, and still tell other rows apart; a kind column (no atom) is
/// read as it is.
#[test]
fn a_frames_digests_are_of_content_whichever_ids_its_atoms_have() {
    let dirs = [tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap()];
    let stores: Vec<ContentStore> = dirs.iter().map(|d| ContentStore::open(&d.path().join("facts.sqlite")).unwrap()).collect();
    let a = stores[0].intern_atoms(&["os.path", "join", "len"]).unwrap();
    let b = stores[1].intern_atoms(&["pad", "len", "join", "os.path"]).unwrap()[1..].to_vec();
    assert_ne!(a[0], b[2], "the two number `os.path` otherwise");
    // Rows (module, name, kind): two atom columns and a number.
    let types = [b's', b's', b'i'];
    let rows = |ids: &[crate::content_store::Sym]| -> Vec<i64> { vec![i64::from(ids[0]), i64::from(ids[1]), 7, i64::from(ids[2]), i64::from(ids[1]), 7] };
    let (atoms_a, atoms_b) = (stores[0].atoms_handle(), stores[1].atoms_handle());
    let (ca, cb) = (crate::code_rule_dsl::logic::plan::Cells::new(&atoms_a, &types), crate::code_rule_dsl::logic::plan::Cells::new(&atoms_b, &types));
    let (ra, rb) = (rows(&a), rows(&[b[2], b[1], b[0]]));
    assert_ne!(ra, rb, "the same rows, held under other ids");
    let digest = crate::code_rule_dsl::logic::plan::rows_digest;
    assert_eq!(digest(&ca, &ra, 2), digest(&cb, &rb, 2), "the same rows' digest in either");
    let source = |r: &[i64]| component::columns_source(vec!["m".into(), "x".into(), "k".into()], (0..3).map(|c| Some(r.chunks(3).map(|row| row[c]).collect())).collect());
    assert_eq!(super::digest(source(&ra).as_ref(), &ca), super::digest(source(&rb).as_ref(), &cb), "a source's too");
    assert_eq!(super::digest(source(&ra).as_ref(), &ca), digest(&ca, &ra, 2), "a source's digest is its rows'");
    // Other rows, other digests: `len` and `os.path` swapped in one row.
    let other = vec![ra[3], ra[1], 7, ra[0], ra[1], 7];
    assert_eq!(digest(&ca, &other, 2), digest(&ca, &ra, 2), "a set: its rows' order does not count");
    let swapped = vec![ra[3], ra[1], 7, ra[3], ra[1], 8];
    assert_ne!(digest(&ca, &swapped, 2), digest(&ca, &ra, 2));
    // A text-owned input's owners part alike; a document (a node's owner) by its number.
    assert_eq!(granule(i64::from(a[0]), Some(&atoms_a)), granule(i64::from(b[2]), Some(&atoms_b)));
    assert_eq!(granule(5 << 32, None), granule(5 << 32, None));
    // By id, as before, the two differed.
    let plain = crate::code_rule_dsl::logic::plan::Cells::plain(3);
    assert_ne!(digest(&plain, &ra, 2), digest(&plain, &rb, 2), "ids are no content");
}
