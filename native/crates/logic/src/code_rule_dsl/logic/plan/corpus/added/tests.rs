    use super::*;
    use crate::code_rule_dsl::logic::program_facts::Field;
    use crate::code_rule_dsl::logic::program_image::Builder;
    use crate::code_rule_dsl::logic::program_image::fields::{FieldsMut, ProgramFields};

    /// A builder of a program with no rules: what `add_to` writes to.
    fn base() -> Builder<'static> {
        Builder::base()
    }

    #[test]
    fn added_rows_go_to_the_programs_that_declare_them_and_key_by_value() {
        let mut declares = base();
        declares.fields().declare("env", &[("k", "symbol"), ("v", "symbol")]);
        let mut not = base();
        let added = Added::settings(&[("a".into(), "1".into()), ("b".into(), "x\ty".into())]).with("declared_ret", Vec::new());
        added.add_to(&mut declares.fields());
        added.add_to(&mut not.fields());
        let env: Vec<Vec<Value>> = declares.fact_rows("env").iter().map(|r| r.to_vec()).collect();
        assert_eq!(env, [vec![Value::Str("a".into()), Value::Str("1".into())], vec![Value::Str("b".into()), Value::Str("x\ty".into())]]);
        assert!(not.fact_names().is_empty());
        // No rows: no relation.
        assert!(Added::default().with("env", Vec::new()).is_empty());
        // Keyed by every value, and by its type.
        assert_ne!(added.digest(), Added::settings(&[("a".into(), "1".into())]).digest());
        assert_ne!(Added::default().with("r", [vec![Value::Int(1)]]).digest(), Added::default().with("r", [vec![Value::Str("1".into())]]).digest());
        assert_eq!(added.digest(), Added::settings(&[("a".into(), "1".into()), ("b".into(), "x\ty".into())]).digest());
        // The dialect: a program's language, and part of the key.
        let tsx = Added::of(&[], &serde_json::json!({"dialect": "tsx"}));
        assert!(!tsx.is_empty());
        let mut named = base();
        named.fields().set_field(Field::Settings(&[("language".into(), "typescript".into()), ("policy".into(), "\"p\"".into())]));
        tsx.add_to(&mut named.fields());
        assert_eq!(named.settings(), [("language".to_owned(), "tsx".to_owned()), ("policy".to_owned(), "\"p\"".to_owned())]);
        let mut unnamed = base();
        tsx.add_to(&mut unnamed.fields());
        assert!(unnamed.settings().is_empty());
        assert_ne!(tsx.digest(), Added::default().digest());
        assert_eq!(Added::of(&[], &serde_json::json!({})), Added::default());
    }

    /// A run's tool calls are the rows of `tool_call` and `tool_input` (`docs/rule-syntax/tool-calls.md`): each string
    /// argument by its dotted path, as a JSON rule's `input.<path>` reads it; an option of any other shape is an error
    /// where the run is first given it (`check_tool_calls`), and no run.
    #[test]
    fn tool_calls_are_rows_of_their_relations() {
        let s = |t: &str| Value::Str(t.into());
        let calls = serde_json::json!([
            {"id": "e1", "tool": "Bash", "input": {"command": "npm run fixtures:push", "timeout": 10, "env": {"CI": "1"}, "argv": ["a"], "a.b": "x"}},
            {"id": "e2", "tool": "Read", "input": {}}
        ]);
        let options = serde_json::json!({ "toolCalls": calls });
        assert_eq!(check_tool_calls(&options), Ok(()));
        let added = Added::of(&[], &options);
        use crate::code_rule_dsl::logic::program_image::{Builder, fields::ProgramFields};
        let decls = crate::code_rule_dsl::logic::rr::parse_version(".decl tool_call(call: symbol, tool: symbol)\n.decl tool_input(call: symbol, key: symbol, value: symbol)\n").unwrap();
        let mut b = Builder::on(&decls);
        added.add_to(&mut b.fields());
        let (prog, _) = b.finish().unwrap();
        let rows = |rel: &str| ProgramFields::fact_rows(&*prog, rel).iter().map(|r| r.to_vec()).collect::<Vec<_>>();
        assert_eq!(rows("tool_call"), [vec![s("e1"), s("Bash")], vec![s("e2"), s("Read")]]);
        assert_eq!(rows("tool_input"), [vec![s("e1"), s("command"), s("npm run fixtures:push")], vec![s("e1"), s("env.CI"), s("1")]]);
        // The calls are part of the run's key.
        assert_ne!(added.digest(), Added::default().digest());
        for bad in [serde_json::json!({}), serde_json::json!([{"id": "e1", "tool": "Bash"}]), serde_json::json!([{"id": 1, "tool": "Bash", "input": {}}]), serde_json::json!([{"id": "e", "tool": "Bash", "input": {}}, {"id": "e", "tool": "Bash", "input": {}}])] {
            assert!(check_tool_calls(&serde_json::json!({ "toolCalls": bad })).is_err(), "{bad}");
        }
        assert_eq!(check_tool_calls(&serde_json::json!({})), Ok(()));
        // The run's entry refuses a malformed option, before any run reads it (nothing is made at `dir`).
        let dir = std::env::temp_dir().join(format!("rerail-tool-calls-refused-{}", std::process::id()));
        let refused = super::super::run_with_options(&[".decl r(x: symbol)\n"], &dir, None, &dir.join("out"), "program", super::super::Binding::Cpu, 1 << 18, &Default::default(), &serde_json::json!({ "toolCalls": {} }));
        assert!(matches!(&refused, Err(super::super::Error::Plan(e)) if e.contains("toolCalls: a JSON array")), "{:?}", refused.err());
        assert!(!dir.exists());
    }
