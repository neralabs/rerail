    use crate::code_rule_dsl::logic::program_image::{build::Builder, manager::Pipeline};

    /// `Declared` on a version refuses a read of a relation not declared, and an atom with other columns than its
    /// relation declares; it passes a program whose reads are declared.
    #[test]
    fn declared_on_a_version_refuses_undeclared_reads() {
        use crate::code_rule_dsl::logic::program_image::T;
        let text = ".decl e(x: number)\n.input e\n.decl r(x: number)\nr(x) :- e(x).\n.output r\n";
        let check = |rel: &str, args: usize| {
            let v = crate::code_rule_dsl::logic::rr::parse_version(text).unwrap();
            let mut b = Builder::on(&v);
            b.rule().head("r", &[T::Var("x")]).atom(rel, &vec![T::Var("x"); args]).finish().unwrap();
            let (v, _) = b.finish().unwrap();
            let mut p = Pipeline::new("test", v);
            p.program(&mut super::Declared);
            p.finish().map(|_| ()).map_err(|e| e.to_string())
        };
        let got = check("nowhere", 1);
        assert!(got.as_ref().err().is_some_and(|e| e.contains("nowhere")), "{got:?}");
        let got = check("e", 2);
        assert!(got.as_ref().err().is_some_and(|e| e.contains("relation e is declared with 1 columns and read with 2")), "{got:?}");
        let got = check("e", 1);
        assert!(got.is_ok(), "{got:?}");
    }

    /// `BuiltinsOut` on a version: the built-ins' and the rewritten relations' declarations and inputs go, and `within`, read by a rule, is declared and an input. (On a modular text's path
    /// the unseeding hides this, since the library declares every built-in: so it is checked here, on its own.)
    #[test]
    fn builtins_out_on_a_version_takes_the_builtins_out() {
        let text = ".decl e(x: number)\n.input e\n.decl r(x: number)\nr(x) :- e(x).\n.output r\n";
        let v = crate::code_rule_dsl::logic::rr::parse_version(text).unwrap();
        let mut b = Builder::on(&v);
        for name in ["nearest", "earlier", "precedes"] {
            crate::code_rule_dsl::logic::program_image::fields::FieldsMut::declare(&mut b.fields(), name, &[("x", "number")]);
            crate::code_rule_dsl::logic::program_image::fields::FieldsMut::add_input(&mut b.fields(), name);
        }
        // A rule reading `within`, undeclared.
        {
            use crate::code_rule_dsl::logic::program_image::T;
            b.rule().head("r", &[T::Var("x")]).atom("within", &[T::Var("x"), T::Wild]).finish().unwrap();
        }
        let (v, _) = b.finish().unwrap();
        let rewritten = vec!["precedes".to_owned()];
        let mut p = Pipeline::new("test", v);
        p.program(&mut super::BuiltinsOut { rewritten: rewritten.clone() });
        let (out, _) = p.finish().unwrap();
        let inputs = out.input_names();
        for gone in ["nearest", "earlier", "precedes"] {
            assert!(!out.declares(gone) && !inputs.iter().any(|i| i == gone), "{gone}: {inputs:?}\n{}", out.show());
        }
        assert!(out.declares("within") && inputs.iter().any(|i| i == "within"), "{inputs:?}\n{}", out.show());
    }
