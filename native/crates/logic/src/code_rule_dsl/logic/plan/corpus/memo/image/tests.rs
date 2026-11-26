    use super::*;
    use super::super::tests::rows_of;
    use crate::code_rule_dsl::logic::plan::output::Cell;

    /// An entry reads back as it was made: its rows in their order and number, its symbols the same atoms, its
    /// outputs' typed rows (a bare output's number too) and their node columns, its nodes; a relation not taken is not read; bytes cut short are refused.
    #[test]
    fn an_entry_reads_back_as_made_its_symbols_by_content() {
        use super::super::super::super::lower::Domain;
        let (db, db2) = (rusqlite::Connection::open_in_memory().unwrap(), rusqlite::Connection::open_in_memory().unwrap());
        let atoms = Atoms::for_connection(&db);
        let (x, y) = (atoms.intern("x").unwrap(), atoms.intern("ünï").unwrap());
        let domains: std::collections::BTreeMap<String, Vec<Domain>> = [("named".to_owned(), vec![Domain::Int, Domain::Name])].into();
        let syms = Syms::of(&domains);
        let mut entry = Entry { key: 5, nodes: 7, ..Entry::default() };
        entry.rows.insert("named".into(), (2, rerail_blocks::vector![3, i64::from(y), 1, i64::from(x), 3, i64::from(y)]));
        entry.rows.insert("wide".into(), (11, (0..22).collect()));
        entry.rows.insert("empty".into(), (2, rerail_blocks::vector::Vector::new()));
        entry.rows.insert("bare".into(), (0, rerail_blocks::vector::Vector::new()));
        entry.outputs.insert("violation".into(), (rerail_blocks::vector![1], rows_of(&[col::TEXT, col::NODE, col::TEXT], &[&[Cell::Text("r"), Cell::Int(21474836480), Cell::Text("x")], &[Cell::Text("a"), Cell::Int(3), Cell::Text("r")]])));
        entry.outputs.insert("a name longer than one row of text holds, which is fifty-six bytes".into(), (rerail_blocks::vector![], rows_of(&[col::TEXT, col::INT], &[&[Cell::Text(""), Cell::Int(-3)], &[Cell::Text("ünïcode"), Cell::Int(i64::MAX)]])));
        entry.outputs.insert("bare".into(), (rerail_blocks::vector![], rows_of(&[], &[&[], &[]])));
        let made = encode(&entry, &syms, &atoms).unwrap();
        let bytes = SegmentBytes::of(&made);
        assert_eq!(decode(&bytes, &|_| true, &atoms).map(|e| Entry { key: 5, ..e }).unwrap(), entry);
        let some = decode(&bytes, &|k| k == "named", &atoms).unwrap();
        assert_eq!(some.rows.keys().collect::<Vec<_>>(), ["named"]);
        assert_eq!(some.outputs, entry.outputs, "outputs are read whatever relations are taken");
        assert!(decode(&SegmentBytes::of(&made[..made.len() - 1]), &|_| true, &atoms).is_err(), "cut short");
        let other = Atoms::for_connection(&db2);
        other.intern("w").unwrap();
        let z = other.intern("z").unwrap();
        let back = decode(&bytes, &|_| true, &other).unwrap();
        let named = &back.rows["named"].1;
        assert_eq!(other.with_str(named[1] as Sym, str::to_owned).as_deref(), Some("ünï"), "another table reads the strings");
        assert_ne!(named[3], i64::from(z));
        assert_eq!(named[1], named[5], "one string, one atom");
    }

    /// Decision 0397: `hold` reads no entry whose symbols' atoms the process holds already (stored, below
    /// `Atoms::held_below`); an entry naming an atom not stored yet is read, and is not once the atom is stored. Either
    /// way the entry reads back by its strings.
    #[test]
    fn hold_reads_no_entry_whose_symbols_are_held() {
        use super::super::super::super::lower::Domain;
        let dir = tempfile::tempdir().unwrap();
        let store = crate::content_store::ContentStore::open(&dir.path().join("store.sqlite")).unwrap();
        let atoms = store.atoms_handle();
        let domains: std::collections::BTreeMap<String, Vec<Domain>> = [("named".to_owned(), vec![Domain::Int, Domain::Name])].into();
        let syms = Syms::of(&domains);
        let entry_of = |strings: &[&str]| {
            let ids = store.intern_atoms(strings).unwrap();
            let mut entry = Entry { key: 1, ..Entry::default() };
            entry.rows.insert("named".into(), (2, ids.iter().flat_map(|&i| [0, i64::from(i)]).collect()));
            let bytes = encode(&entry, &syms, &atoms).unwrap();
            (entry, bytes)
        };
        assert!(atoms.ready_for_run().unwrap());
        let (stored, stored_bytes) = entry_of(&["a", "b"]);
        atoms.run_over();
        store.flush_atoms().unwrap();
        assert!(atoms.held_below().is_some_and(|h| h > 2), "the stored atoms held");
        let (pending, pending_bytes) = entry_of(&["c"]);
        let (s, p) = (SegmentBytes::of(&stored_bytes), SegmentBytes::of(&pending_bytes));
        assert_eq!(hold(&[&s], &atoms).unwrap(), 0, "its atoms stored and held: nothing read");
        assert_eq!(hold(&[&s, &p], &atoms).unwrap(), 1, "an atom not stored yet: that entry read");
        store.flush_atoms().unwrap();
        assert_eq!(hold(&[&s, &p], &atoms).unwrap(), 0, "once stored, held");
        for (entry, bytes) in [(&stored, &s), (&pending, &p)] {
            assert_eq!(decode(bytes, &|_| true, &atoms).map(|e| Entry { key: 1, ..e }).unwrap(), *entry, "read by its strings");
        }
    }
