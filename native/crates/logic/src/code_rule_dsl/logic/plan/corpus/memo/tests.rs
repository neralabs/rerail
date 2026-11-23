    /// A relation of facts with no rows (a constant folded empty, `constants::fold`) survives its facts: a stored
    /// program read back is the program written, so its digests (the memo's key, the relations' identities) are the
    /// same. Without it, the first run after a rebuild and the second keyed the memo apart.
    #[test]
    fn an_empty_relation_of_facts_reads_back_as_present() {
        use crate::code_rule_dsl::logic::program_image::{Builder, Version, fields::ProgramFields};
        let read = crate::code_rule_dsl::logic::rr::parse_version(".decl a(s: symbol)\n.decl b(s: symbol)\nb(\"x\").\n.output a\n").unwrap();
        let mut b = Builder::on(&read);
        b.fields().set_facts("a", &[]);
        let (v, _) = b.finish().unwrap();
        // Stored as an image, and opened again.
        let back = Version::open(v.fold().unwrap().base_bytes().unwrap()).unwrap();
        assert!(back.states_facts("a") && back.fact_rows("a").is_empty(), "a's empty facts read back:\n{}", back.show());
        assert_eq!(super::program_digest(&back), super::program_digest(&v));
    }

    use super::*;
    use crate::code_rule_dsl::logic::plan::output::{Cell, col};

    /// Rows of columns typed `types`, of `rows`.
    pub fn rows_of(types: &[u8], rows: &[&[Cell<'_>]]) -> crate::code_rule_dsl::logic::plan::output::Rows {
        let mut out = crate::code_rule_dsl::logic::plan::output::Rows::new(types.to_vec());
        for r in rows {
            out.push(r);
        }
        out
    }

    #[test]
    fn a_tree_demand_keys_its_own_frames() {
        assert_ne!(with_tree_demand([0; 32], false), with_tree_demand([0; 32], true), "wider consumers cannot hit narrower frames");
    }

    /// Decision 0295 §1: the run writes its entries into their segment itself, and its readers of an entry still
    /// queued read it there (no decoded copy is held for them); once written, the store's row finds the same.
    #[test]
    fn a_queued_entry_is_read_from_the_segment_the_run_wrote() {
        use super::super::super::lower::Domain;
        let dir = tempfile::tempdir().unwrap();
        let store = ContentStore::open(&dir.path().join("store.sqlite")).unwrap();
        let domains: BTreeMap<String, Vec<Domain>> = [("r".to_owned(), vec![Domain::Int, Domain::Int])].into();
        let program = Program::new([3; 32], &domains);
        let mut entry = Entry { key: 4, nodes: 9, ..Entry::default() };
        entry.rows.insert("r".into(), (2, vec![1, 2, 3, 4]));
        entry.rows.insert("unused".into(), (2, vec![1; 2000]));
        entry.outputs.insert("diagnostic".into(), (vec![0], rows_of(&[col::NODE, col::TEXT], &[&[Cell::Int(5), Cell::Text("bad")]])));
        let id = store.put_source(b"x = 1\n", &[]).unwrap();
        let found = |store: &ContentStore| Memo::Stored(store).found_all(&program, &[(Content::Stored(id), "a.py")]).unwrap().pop().flatten();
        let waves = crate::content_store::write_back::run_waves();
        remember_all(&store, &program, vec![(Content::Stored(id), "a.py".into(), 4, entry.clone())]).unwrap();
        let queued = found(&store).expect("queued, found");
        assert!(matches!(queued, Found::Frame(_)), "read from its segment, not held decoded");
        assert_eq!(queued.entry(4).unwrap(), Some(entry.clone()));
        // Projected: its demanded relations decoded, its outputs and size whole; another document's key finds none.
        let projected = queued.projected(4, &|r: &str| r == "r").unwrap().unwrap();
        assert_eq!(projected.rows.keys().collect::<Vec<_>>(), ["r"]);
        assert_eq!((&projected.outputs, projected.nodes), (&entry.outputs, 9));
        assert!(queued.projected(5, &|r: &str| r == "r").unwrap().is_none());
        assert_eq!(store.memo_pack_bytes().unwrap().1, 0, "not written yet");
        drop(waves);
        store.written().unwrap();
        assert!(store.memo_pack_bytes().unwrap().1 > 0, "its row written");
        assert_eq!(found(&store).expect("written, found").entry(4).unwrap(), Some(entry));
    }

    /// An entry symbol past the process's atoms is refused where the entry is made (decision 0170): a read gave the
    /// store's ids unmapped. It was found only by the writer, mapping it later (`symbol 2767 is no atom of this
    /// process's (it holds 2760)`), far from the read.
    #[test]
    fn an_entry_naming_no_atom_of_the_process_is_refused_where_it_is_made() {
        use super::super::super::lower::Domain;
        let domains: BTreeMap<String, Vec<Domain>> = [("named".to_owned(), vec![Domain::Int, Domain::Name])].into();
        let syms = Syms::of(&domains);
        let mut entry = Entry { key: 1, ..Entry::default() };
        entry.rows.insert("named".into(), (2, vec![9_999, 3, 10_000, 4]));
        assert_eq!(foreign_symbol(&[&entry], &syms, 5), None, "numbers past the atoms are not symbols");
        entry.rows.insert("named".into(), (2, vec![1, 3, 2, 5]));
        assert_eq!(foreign_symbol(&[&entry], &syms, 5), Some(("named".to_owned(), 5)));
    }

    /// A posting counts a constant as its column's source encodes it (`own:postings:field:f`: a grammar's field code,
    /// 65555, not an atom): its columns are known (a forked run refused every entry holding one when they were not),
    /// and none is a symbol, so none is mapped, whatever the relation declares.
    #[test]
    fn a_posting_and_a_grammar_code_hold_no_symbol() {
        use super::super::super::lower::Domain;
        let domains: BTreeMap<String, Vec<Domain>> = [("field".to_owned(), vec![Domain::Node, Domain::Field, Domain::Node]), ("named".to_owned(), vec![Domain::Kind, Domain::Name])].into();
        let syms = Syms::of(&domains);
        assert_eq!(syms.columns("own:postings:field:f", 2), Some(vec![false, false]));
        assert_eq!(syms.columns("named", 2), Some(vec![false, true]), "a kind's neither; a name is");
        let mut entry = Entry { key: 1, ..Entry::default() };
        entry.rows.insert("own:postings:field:f".into(), (2, vec![65_555, 10]));
        assert_eq!(foreign_symbol(&[&entry], &syms, 5), None, "not taken for an atom");
    }

    #[test]
    fn an_entry_holds_the_identities_it_was_made_under_and_no_others() {
        let (a, b, c) = ([1u8; 32], [2u8; 32], [3u8; 32]);
        let mut entry = Entry { key: 7, ..Entry::default() };
        cover(&mut entry, [a, b].iter());
        cover(&mut entry, [b].iter());
        let held = covered(&entry);
        assert_eq!(held.len(), 2);
        assert!(held.contains(&covered_words(&a)) && held.contains(&covered_words(&b)) && !held.contains(&covered_words(&c)));
        // It is an `own:` key, and names no symbol: what is kept over an entry whose rows stand (decision 0280).
        assert_eq!(identities_of(&entry).rows.keys().collect::<Vec<_>>(), [COVERED]);
        assert_eq!(Syms::default().columns(COVERED, 2), Some(vec![false, false]));
    }

    #[test]
    fn keys_are_stable() {
        let paths = ["a.py".to_owned(), "b/c.py".to_owned()];
        let keys = doc_keys(&paths);
        assert_eq!(keys, doc_keys(&paths));
        assert_eq!(doc_keys(&paths[1..]), keys[1..], "a key does not depend on the other paths");
        assert!(keys.iter().all(|&k| (1..1 << 30).contains(&k)));
        assert_eq!(document_of(keys[0] << 32 | 9), keys[0]);
        assert_eq!(document_of(keys[1]), keys[1]);
    }
