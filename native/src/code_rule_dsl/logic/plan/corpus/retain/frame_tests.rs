use super::*;
use crate::code_rule_dsl::logic::{native::component, profile, rr};

const SPACE: [u8; 32] = [51; 32];
const KEY: [u8; 32] = [17; 32];

/// A program's strata, as a run computes them once and passes them on.
fn st(p: &front::Program) -> Vec<BTreeSet<String>> { front::strata(p).unwrap().0 }

fn fixture(store: &ContentStore, count: usize) -> (front::Program, HashMap<String, u64>, InFrames) {
    let mut text = String::new();
    for i in 0..count {
        text += &format!(".decl input{i}(x:number)\n.input input{i}\n.decl result{i}(x:number)\nresult{i}(x) :- input{i}(x).\n");
    }
    let prog = rr::parse(&text).unwrap();
    let empty = Retained::from_frames(store,SPACE,KEY,&prog,&st(&prog), &HashMap::new()).unwrap();
    let frames = empty.frames.unwrap();
    let mut puts = Vec::new();
    let mut now = HashMap::new();
    for i in 0..count {
        let input = format!("input{i}");
        let result = format!("result{i}");
        let rows: Vec<i64> = (1..=3).map(|d| (d << 32) | i as i64).collect();
        let source = component::columns_source(vec!["c0".into()], vec![Some(rows.clone())]);
        let docs = docs_of(source.as_ref(), 0);
        let d = digest(source.as_ref());
        assert_eq!(digest_of_docs(&docs), d);
        now.insert(input.clone(), d);
        puts.push(FramePut { rel: frames.names[&input].clone(), part: DOCS_PART, digest: d, reads: vec![],
            rows: FrameRows { types: vec![b'i';3], n: docs.len(), rows: docs.iter().flat_map(|(doc,(sum,n))| [*doc,*sum as i64,*n as i64]).collect() } });
        for &row in &rows {
            puts.push(FramePut { rel: frames.names[&input].clone(), part: granule(row >> 32), digest: 0, reads: vec![],
                rows: FrameRows { types: vec![b'i'], n: 1, rows: vec![row] } });
        }
        puts.push(FramePut { rel: frames.names[&result].clone(), part: DOCS_PART,
            digest: crate::code_rule_dsl::logic::plan::rows_digest(1, &rows, 3), reads: vec![(input,d)],
            rows: FrameRows { types: vec![b'i'], n: rows.len(), rows } });
    }
    store.keep_frames(&SPACE, &puts, &[], None).unwrap();
    (prog, now, frames)
}

#[test]
fn k4_admission_batches_input_digests_derived_rows_and_mixed_granules() {
    let _run = profile::one_run();
    profile::set(true);
    let dir = tempfile::tempdir().unwrap();
    let store = ContentStore::open(&dir.path().join("facts.sqlite")).unwrap();
    let (prog, now, frames) = fixture(&store, 131);
    store.keep_frames(&SPACE, &[FramePut {rel:frames.names["result130"].clone(),part:0,digest:99,
        reads:vec![("input130".into(),now["input130"].wrapping_add(1))],
        rows:FrameRows{types:vec![b'i'],n:1,rows:vec![999]}}], &[], None).unwrap();
    let changed: HashMap<_,_> = now.iter().map(|(r,d)|(r.clone(),d.wrapping_add(1))).collect();
    profile::reset(); profile::set(true);
    let wave = profile::waves();
    let span = profile::span(|| "bulk K4 admission".into());
    let mut kept = Retained::from_frames(&store,SPACE,KEY,&prog,&st(&prog), &HashMap::new()).unwrap();
    let cut = cut(Some(&kept),KEY,&prog,&st(&prog), &changed).unwrap();
    kept.load(&store,&cut,&prog,&Locations::new(),false).unwrap();
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
    eprintln!("K4: 131 input headers, 393 granules, 130 derived frames: {statements} storage executions");
    assert!(statements>0,"the gate must trace storage SQL");
    assert!(statements<=16,"K4 must admit mixed partition demands in bounded batches: {statements}");
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
    let mut kept=Retained::from_frames(&store,SPACE,KEY,&prog,&st(&prog),&HashMap::new()).unwrap();
    assert!(prog.outputs.iter().all(|r|kept.holds(r,&prog)),"raw output facts are sufficient to keep a derivation");
    let cut=Cut{clean:prog.outputs.iter().cloned().collect(),strata:(0,130),changed:vec![]};
    assert_eq!(kept.load(&store,&cut,&prog,&Locations::new(),false).unwrap(),Some((130,390)));
    for i in 0..130 {
        let rel=&kept.rels[&format!("result{i}")];
        let source=rel.source(&format!("result{i}"));
        assert_eq!((0..source.len()).map(|r|source.value(r,0)).collect::<Vec<_>>(),(1..=3).map(|d|(d<<32)|i as i64).collect::<Vec<_>>());
        assert!(rel.output.is_none(),"decoding belongs at the output boundary");
    }
    drop(span); drop(wave);
    let statements:u64=profile::sql_waits().iter().filter(|(p,s,_,_)|p.starts_with("bulk output admission")&&s.starts_with("store:")).map(|x|x.2).sum();
    profile::set(false);
    assert!(statements>0 && statements<=16,"130 outputs must be admitted in bounded batches: {statements}");
}

#[test]
fn k4_retains_consumers_of_literal_facts_and_invalidates_them_when_the_literal_changes() {
    let dir=tempfile::tempdir().unwrap();
    let store=ContentStore::open(&dir.path().join("facts.sqlite")).unwrap();
    let text=".decl literal(x:number)\nliteral(1).\n.decl result(x:number)\nresult(x) :- literal(x), x > 0.\n.output result\n";
    let prog=rr::parse(text).unwrap();
    let empty=Retained::from_frames(&store,SPACE,KEY,&prog,&st(&prog),&HashMap::new()).unwrap();
    let initial=cut(Some(&empty),KEY,&prog,&st(&prog),&HashMap::new()).unwrap();
    let computed=Retained{key:KEY,rels:HashMap::from([("result".into(),Kept{
        rel:InRel::flat(&[vec![1]],vec![b'i']),width:1,output:None,
    })]),..Default::default()};
    assert_eq!(computed.write_frames(&store,empty.frames().unwrap(),&prog,&Locations::new(),&st(&prog),&initial,&[],&HashMap::new(),&BTreeSet::new(),&HashMap::new()).unwrap(),1);
    let restored=Retained::from_frames(&store,SPACE,KEY,&prog,&st(&prog),&HashMap::new()).unwrap();
    assert_eq!(cut(Some(&restored),KEY,&prog,&st(&prog),&HashMap::new()).unwrap().strata,(0,1));
    let changed=rr::parse(&text.replace("literal(1).","literal(2).")).unwrap();
    let restored=Retained::from_frames(&store,SPACE,KEY,&changed,&st(&changed),&HashMap::new()).unwrap();
    assert_eq!(cut(Some(&restored),KEY,&changed,&st(&changed),&HashMap::new()).unwrap().strata,(1,1),"literal rows participate in the relation's identity");
    let external=rr::parse(&format!("{text}\n.input literal\n")).unwrap();
    let empty=Retained::from_frames(&store,[52;32],KEY,&external,&st(&external),&HashMap::new()).unwrap();
    let initial=cut(Some(&empty),KEY,&external,&st(&external),&HashMap::new()).unwrap();
    assert_eq!(computed.write_frames(&store,empty.frames().unwrap(),&external,&Locations::new(),&st(&external),&initial,&[],&HashMap::new(),&BTreeSet::new(),&HashMap::new()).unwrap(),0,
        "a relation with literal rows AND external inputs still needs a runtime dependency digest");
}

#[test]
fn k4_rejects_a_derived_frame_replaced_after_metadata_admission() {
    let dir=tempfile::tempdir().unwrap();
    let store=ContentStore::open(&dir.path().join("facts.sqlite")).unwrap();
    let (prog,now,frames)=fixture(&store,1);
    let mut kept=Retained::from_frames(&store,SPACE,KEY,&prog,&st(&prog), &HashMap::new()).unwrap();
    let changed=now.iter().map(|(r,d)|(r.clone(),d.wrapping_add(1))).collect();
    let cut=cut(Some(&kept),KEY,&prog,&st(&prog),&changed).unwrap();
    store.keep_frames(&SPACE,&[FramePut{rel:frames.names["result0"].clone(),part:0,digest:99,reads:vec![],
        rows:FrameRows{types:vec![b'i'],n:1,rows:vec![999]}}],&[],None).unwrap();
    assert!(kept.load(&store,&cut,&prog,&Locations::new(),false).unwrap().is_none());
    assert!(kept.rels.is_empty(),"old dependency metadata must not certify replacement payloads");
    assert!(kept.frames().unwrap().derived.is_empty(),"a failed admission must invalidate the cut");
}

#[test]
fn k4_rejects_an_input_granule_replaced_after_its_digest_was_admitted() {
    let dir=tempfile::tempdir().unwrap();
    let store=ContentStore::open(&dir.path().join("facts.sqlite")).unwrap();
    let (prog,now,frames)=fixture(&store,1);
    let mut kept=Retained::from_frames(&store,SPACE,KEY,&prog,&st(&prog), &HashMap::new()).unwrap();
    let changed=now.iter().map(|(r,d)|(r.clone(),d.wrapping_add(1))).collect();
    let cut=cut(Some(&kept),KEY,&prog,&st(&prog),&changed).unwrap();
    // Granule headers have no content digest. Validate the rows against the admitted per-document digests.
    store.keep_frames(&SPACE,&[FramePut{rel:frames.names["input0"].clone(),part:granule(1),digest:0,reads:vec![],
        rows:FrameRows{types:vec![b'i'],n:1,rows:vec![(1<<32)|999]}}],&[],None).unwrap();
    assert!(kept.load(&store,&cut,&prog,&Locations::new(),false).unwrap().is_none());
    assert!(kept.input_srcs.is_empty(),"replacement rows must not become the old side of maintenance");
    assert!(kept.frames().unwrap().inputs.is_empty(),"a failed admission must invalidate the cut");
}

#[test]
fn k4_rejects_input_headers_replaced_after_the_metadata_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let store = ContentStore::open(&dir.path().join("facts.sqlite")).unwrap();
    let (prog, now, frames) = fixture(&store, 1);
    let names = frames.names;
    let keys: Vec<_> = names.values().cloned().collect();
    let metadata = store.frames(&SPACE, &keys, DOCS_PART).unwrap();
    store.keep_frames(&SPACE, &[], &[(names["input0"].clone(), DOCS_PART)], None).unwrap();
    let (sccs, _) = front::strata(&prog).unwrap();
    let kept = Retained::from_frame_snapshot(&store, SPACE, KEY, &prog, &sccs, names, metadata, &HashMap::new()).unwrap();
    assert!(kept.inputs.is_empty());
    assert!(kept.frames().unwrap().derived.is_empty());
    let cold = cut(Some(&kept),KEY,&prog,&st(&prog), &now).unwrap();
    assert_eq!(cold.strata, (1, 1));
    assert!(cold.clean.is_empty());
}

#[test]
fn failed_k4_admission_makes_a_clean_consumer_run_again() {
    let dir = tempfile::tempdir().unwrap();
    let store = ContentStore::open(&dir.path().join("facts.sqlite")).unwrap();
    let (mut prog, now, _) = fixture(&store, 1);
    // A filter prevents an input view: this result is materialized and a later consumer reads it.
    let extra = rr::parse(".decl consumer(x:number)\nconsumer(x) :- result0(x), x > 0.\n").unwrap();
    prog.decls.extend(extra.decls);
    prog.rules.extend(extra.rules);
    let mut kept = Retained::from_frames(&store,SPACE,KEY,&prog,&st(&prog), &HashMap::new()).unwrap();
    let initial = cut(Some(&kept),KEY,&prog,&st(&prog), &now).unwrap();
    // Input views are recomputed; force the result's existing payload to be required, as a dirty reader does.
    let demanded = Cut { clean: BTreeSet::from(["result0".into()]), strata: initial.strata, changed: vec![] };
    let result = kept.frames().unwrap().names["result0"].clone();
    store.keep_frames(&SPACE, &[], &[(result, DOCS_PART)], None).unwrap();
    assert!(kept.load(&store, &demanded, &prog, &Locations::new(), false).unwrap().is_none());
    let rerun = cut(None,KEY,&prog,&st(&prog), &now).unwrap();
    assert!(rerun.clean.is_empty());
    assert_eq!(rerun.strata.0, rerun.strata.1);
    assert!(kept.frames().unwrap().derived.is_empty(), "new results must be republished");
}

/// Smoking gun 3 (t34, 2026-11-13): an export's kept rows are grouped by the column the memo attributes them by (the
/// split's locations), not by the first node column. `p197_arg_at(d, i, a: node)` was kept by column 2 and the memo
/// attributed it by column 0, so every edit refused the kept inputs ("memo entry is gone").
#[test]
fn kept_rows_are_grouped_by_the_column_the_memo_attributes_them_by() {
    let prog = rr::parse(".decl arg_at(d:number, i:number, a:node)\n.input arg_at\n").unwrap();
    assert_eq!(group_col(&prog, &Locations::new(), "arg_at"), 2, "with no location: the node column");
    let located: Locations = [("arg_at".to_owned(), vec![0])].into_iter().collect();
    assert_eq!(group_col(&prog, &located, "arg_at"), 0, "the memo's location column");
}

/// Smoking gun 4 (t34, 2026-11-13): whether an edit takes the kept inputs is known before the first wave, with the
/// reason when it does not (decision 0104: the waves read no entry of the store). The fallback found out inside the
/// waves and read the store there.
#[test]
fn the_kept_inputs_are_known_before_the_waves_with_why_not() {
    let dir = tempfile::tempdir().unwrap();
    let store = ContentStore::open(&dir.path().join("facts.sqlite")).unwrap();
    let prog = rr::parse(".decl input0(x:number)\n.input input0\n.decl result0(x:number)\nresult0(x) :- input0(x).\n").unwrap();
    let why = kept_inputs_stored(&store, SPACE, KEY, &prog, &["input0".to_owned()]).unwrap().expect_err("an empty store holds no kept inputs");
    assert!(why.contains("input0"), "the reason names the input: {why}");
    let (prog, _, _) = fixture(&store, 1);
    kept_inputs_stored(&store, SPACE, KEY, &prog, &["input0".to_owned()]).unwrap().expect("the frames of its documents are stored");
}

/// Smoking gun 5 (t34, 2026-11-13): the retain budget is of memory, not of the store. A relation past it was neither
/// held nor written to the frames, so every next process derived it again (an edit ran 3,758 of 4,087 strata, 1,546
/// derived again). It is written, then let go.
#[test]
fn a_relation_past_the_retain_budget_is_written_to_the_frames_and_let_go() {
    let dir = tempfile::tempdir().unwrap();
    let store = ContentStore::open(&dir.path().join("facts.sqlite")).unwrap();
    let (prog, now, _) = fixture(&store, 2);
    let was = Retained::from_frames(&store, SPACE, KEY, &prog, &st(&prog), &HashMap::new()).unwrap().frames.unwrap();
    let cut = cut(None, KEY, &prog, &st(&prog), &now).unwrap();
    let rows = |base: i64| component::InRel::flat(&(0..4).map(|i| vec![base + i]).collect::<Vec<_>>(), vec![b'i']);
    let derived = vec![("result0".to_owned(), 1, rows(100)), ("result1".to_owned(), 1, rows(200))];
    let mut next = after(None, KEY, now.clone(), &cut, derived, &BTreeMap::new(), &[], HashMap::new(), HashMap::new(), HashMap::new(), Default::default(), HashMap::new(), 0);
    assert_eq!(next.bytes(), 0, "nothing past the budget is held for the next run");
    assert_eq!(next.parts().1.len(), 2, "both are this run's to write");
    let written = next.write_frames(&store, &was, &prog, &Locations::new(), &st(&prog), &cut, &[], &now, &BTreeSet::new(), &HashMap::new()).unwrap();
    assert_eq!(written, 8, "the rows past the budget are written to the frames");
    next.let_go_past();
    assert!(next.parts().1.is_empty(), "and then let go");
}

/// Smoking gun 9 (t34, 2026-11-13), red until fixed. A kept relation's statistics are a part of its frame under its
/// rows' digest (decision 0120 §3), admitted with its rows by `load`. Rows read where the walk reaches them (`admit`,
/// decision 0117 step 4) took the rows and their levels, not the statistics, so every `__old` the edit planned with
/// was counted from its rows ("count run_scope__old from its rows", dozens an edit).
#[test]
#[ignore = "smoking gun 9 open (t34): admit reads a reached relation's rows and levels, not its statistics part"]
fn rows_read_where_reached_come_with_their_kept_statistics() {
    let dir = tempfile::tempdir().unwrap();
    let store = ContentStore::open(&dir.path().join("facts.sqlite")).unwrap();
    let (prog, _, frames) = fixture(&store, 1);
    let stored = Retained::from_frames(&store, SPACE, KEY, &prog, &st(&prog), &HashMap::new()).unwrap();
    let digest = stored.frames.as_ref().unwrap().derived["result0"].digest;
    let mut all = kept_stats::ByDigest::new();
    all.insert(("result0".to_owned(), digest), kept_stats::KeptStat::Measured((vec![("result0_c0".to_owned(), 3, 3, 3, 3)], vec![], BTreeSet::new())));
    let words = kept_stats::to_words(&kept_stats::encode(&all));
    store.keep_frames(&SPACE, &[FramePut { rel: frames.names["result0"].clone(), part: kept_stats::PART, digest, reads: vec![], rows: FrameRows { types: vec![b'i'], n: words.len(), rows: words } }], &[], None).unwrap();
    let kept = Retained::from_frames(&store, SPACE, KEY, &prog, &st(&prog), &HashMap::new()).unwrap();
    assert!(kept.admit(&store, &prog, &["result0".to_owned()]).unwrap(), "its rows are read where reached");
    let at = Arc::as_ptr(&kept.late.borrow()["result0"].rel) as *const u8 as usize;
    assert!(kept.measured.keys().any(|(name, a)| name == "result0" && *a == at) || kept.patched.values().any(|p| p.at == at), "its statistics come with its rows: not counted from them");
}
