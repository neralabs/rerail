//! Hand-written contracts for the callback-free SQL relational compiler.
//!
//! These cases define the required lowering boundary before the compiler exists. Each case names
//! the terminal programs that SQL can demand and the SQL constructs that must compose their rows.
//! The cases do not permit a whole matcher, relation, or clause to become one opaque program.
//!
//! The compact forms are stable contract inputs. Do not rewrite a form into an easier equivalent
//! to make an implementation pass. Add separate tests from real compiled policies as each public
//! compiler API becomes available.

use super::contract_frontend;
use super::relational_ir::{SqlOperation as SqlShape, TerminalKind as Terminal};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum Family {
    Atomic,
    Boolean,
    Navigation,
    Capture,
    Utility,
    Semantic,
    ClauseOutput,
}

#[derive(Debug)]
struct LoweringCase {
    name: &'static str,
    family: Family,
    /// A compact description of the already validated Rerail form.
    form: &'static str,
    /// Native terminal programs interned while the SQL form is lowered.
    terminals: &'static [Terminal],
    /// Required constructs in the normalized SQL shape.
    sql: Vec<SqlShape>,
}

macro_rules! case {
    ($name:literal, $family:expr, $form:literal, [$($terminal:expr),* $(,)?], [$($sql:expr),* $(,)?]) => {
        LoweringCase {
            name: $name,
            family: $family,
            form: $form,
            terminals: &[$($terminal),*],
            sql: vec![
                SqlShape::PackedMaterialized,
                SqlShape::StatusJoin,
                $($sql),*
            ],
        }
    };
}

fn cases() -> Vec<LoweringCase> {
    use Family::{Atomic, Boolean, Capture, ClauseOutput, Navigation, Semantic, Utility};
    use SqlShape::{
        AntiJoin, Bindings, CaseTruth, CrossFileJoin, Distinct, Exists, FileFilter, GroupBy,
        Having, InnerJoin, LeftJoin, MapDocument, MinPosition, OrderBy, OrderedUnion, Projection,
        RecursiveCte, ScanCapture, ScanFact, ScanNavigation, ScanTerminal, WhereFilter, WindowRank,
    };
    use Terminal::{
        CallSeed, Equality, Fact, Kind, Navigation as NavigationTerminal, Parse, Pattern, Regex,
        Text, Transform, Value,
    };

    vec![
        // Atomic terminal forms: 12.
        case!(
            "pattern-terminal",
            Atomic,
            "pattern(named terminal)",
            [Pattern],
            [ScanTerminal, ScanCapture]
        ),
        case!(
            "pattern-structural",
            Atomic,
            "pattern(internal tree)",
            [Pattern],
            [ScanTerminal, ScanCapture]
        ),
        case!(
            "pattern-single-metavariable",
            Atomic,
            "pattern($NAME)",
            [Pattern],
            [ScanTerminal, ScanCapture, Bindings]
        ),
        case!(
            "pattern-multi-metavariable",
            Atomic,
            "pattern($$$ARGS)",
            [Pattern],
            [ScanTerminal, ScanCapture, Bindings]
        ),
        case!(
            "pattern-repeated-capture",
            Atomic,
            "pattern($A + $A)",
            [Pattern],
            [ScanTerminal, ScanCapture, Bindings]
        ),
        case!(
            "pattern-context-selector",
            Atomic,
            "pattern(context, selector)",
            [Pattern],
            [ScanTerminal, ScanCapture]
        ),
        case!(
            "kind-single",
            Atomic,
            "kind(identifier)",
            [Kind],
            [ScanTerminal]
        ),
        case!(
            "kind-set",
            Atomic,
            "kind(identifier|string)",
            [Kind],
            [ScanTerminal]
        ),
        case!(
            "node-regex",
            Atomic,
            "regex(^unsafe$)",
            [Regex],
            [ScanTerminal]
        ),
        case!(
            "node-text-equality",
            Atomic,
            "text(eq, literal)",
            [Text],
            [ScanTerminal]
        ),
        case!(
            "node-value-positive",
            Atomic,
            "value(is, constant)",
            [Value],
            [ScanTerminal, CaseTruth]
        ),
        case!(
            "node-value-unknown",
            Atomic,
            "value(flow-dependent)",
            [Value],
            [ScanTerminal, CaseTruth]
        ),
        // Ordered Boolean forms: 14.
        case!(
            "all-two-positive",
            Boolean,
            "all(kind, regex)",
            [Kind, Regex],
            [ScanTerminal, InnerJoin, Bindings]
        ),
        case!(
            "all-capture-threading",
            Boolean,
            "all(pattern($A), compare($A))",
            [Pattern, Equality],
            [ScanTerminal, ScanCapture, InnerJoin, Bindings]
        ),
        case!(
            "all-three-valued-false-after-unknown",
            Boolean,
            "all(value(unknown), kind(false))",
            [Value, Kind],
            [ScanTerminal, InnerJoin, CaseTruth]
        ),
        case!(
            "all-three-valued-withheld-capture",
            Boolean,
            "all(value($A, unknown), compare($A))",
            [Value, Equality],
            [ScanTerminal, ScanCapture, LeftJoin, CaseTruth, Bindings]
        ),
        case!(
            "any-first-branch",
            Boolean,
            "any(pattern(a), pattern(b))",
            [Pattern],
            [ScanTerminal, OrderedUnion, WindowRank, Bindings]
        ),
        case!(
            "any-unknown-binding-blocks-later",
            Boolean,
            "any(value($A, unknown), pattern($A))",
            [Value, Pattern],
            [
                ScanTerminal,
                ScanCapture,
                OrderedUnion,
                WindowRank,
                CaseTruth,
                Bindings
            ]
        ),
        case!(
            "any-unknown-without-binding",
            Boolean,
            "any(value(unknown), kind(match))",
            [Value, Kind],
            [ScanTerminal, OrderedUnion, WindowRank, CaseTruth]
        ),
        case!(
            "not-complete-positive",
            Boolean,
            "not(kind(identifier))",
            [Kind],
            [ScanTerminal, AntiJoin]
        ),
        case!(
            "not-unknown",
            Boolean,
            "not(value(unknown))",
            [Value],
            [ScanTerminal, AntiJoin, CaseTruth]
        ),
        case!(
            "nested-all-any-not",
            Boolean,
            "all(any(kind, regex), not(pattern))",
            [Kind, Regex, Pattern],
            [ScanTerminal, OrderedUnion, InnerJoin, AntiJoin, WindowRank]
        ),
        case!(
            "constraint-bound-capture",
            Boolean,
            "constrained(pattern($A), $A:kind)",
            [Pattern, Kind],
            [ScanTerminal, ScanCapture, InnerJoin, Bindings]
        ),
        case!(
            "constraint-unbound-capture",
            Boolean,
            "constrained(pattern, $A:kind)",
            [Pattern, Kind],
            [ScanTerminal, ScanCapture, LeftJoin, Bindings]
        ),
        case!("never", Boolean, "never", [], [AntiJoin, CaseTruth]),
        case!(
            "disabled-when",
            Boolean,
            "when(false)",
            [],
            [FileFilter, AntiJoin]
        ),
        // Navigation, field, position, and count forms: 22.
        case!(
            "has-neighbor",
            Navigation,
            "has(kind, stopBy=neighbor)",
            [NavigationTerminal, Kind],
            [ScanNavigation, ScanTerminal, InnerJoin, MinPosition]
        ),
        case!(
            "has-end",
            Navigation,
            "has(kind, stopBy=end)",
            [NavigationTerminal, Kind],
            [ScanNavigation, ScanTerminal, InnerJoin, Exists]
        ),
        case!(
            "has-stop-rule",
            Navigation,
            "has(kind, stopBy=pattern)",
            [NavigationTerminal, Kind, Pattern],
            [
                ScanNavigation,
                ScanTerminal,
                InnerJoin,
                AntiJoin,
                MinPosition
            ]
        ),
        case!(
            "inside-neighbor",
            Navigation,
            "inside(kind, stopBy=neighbor)",
            [NavigationTerminal, Kind],
            [ScanNavigation, ScanTerminal, InnerJoin, MinPosition]
        ),
        case!(
            "inside-end",
            Navigation,
            "inside(kind, stopBy=end)",
            [NavigationTerminal, Kind],
            [ScanNavigation, ScanTerminal, InnerJoin, MinPosition]
        ),
        case!(
            "inside-stop-rule",
            Navigation,
            "inside(kind, stopBy=pattern)",
            [NavigationTerminal, Kind, Pattern],
            [
                ScanNavigation,
                ScanTerminal,
                InnerJoin,
                AntiJoin,
                MinPosition
            ]
        ),
        case!(
            "precedes-neighbor",
            Navigation,
            "precedes(kind, stopBy=neighbor)",
            [NavigationTerminal, Kind],
            [ScanNavigation, ScanTerminal, InnerJoin, MinPosition]
        ),
        case!(
            "precedes-end",
            Navigation,
            "precedes(kind, stopBy=end)",
            [NavigationTerminal, Kind],
            [ScanNavigation, ScanTerminal, InnerJoin, MinPosition]
        ),
        case!(
            "precedes-stop-rule",
            Navigation,
            "precedes(kind, stopBy=pattern)",
            [NavigationTerminal, Kind, Pattern],
            [
                ScanNavigation,
                ScanTerminal,
                InnerJoin,
                AntiJoin,
                MinPosition
            ]
        ),
        case!(
            "follows-neighbor",
            Navigation,
            "follows(kind, stopBy=neighbor)",
            [NavigationTerminal, Kind],
            [ScanNavigation, ScanTerminal, InnerJoin, MinPosition]
        ),
        case!(
            "follows-end",
            Navigation,
            "follows(kind, stopBy=end)",
            [NavigationTerminal, Kind],
            [ScanNavigation, ScanTerminal, InnerJoin, MinPosition]
        ),
        case!(
            "follows-stop-rule",
            Navigation,
            "follows(kind, stopBy=pattern)",
            [NavigationTerminal, Kind, Pattern],
            [
                ScanNavigation,
                ScanTerminal,
                InnerJoin,
                AntiJoin,
                MinPosition
            ]
        ),
        case!(
            "field-has",
            Navigation,
            "has(field=body, kind)",
            [NavigationTerminal, Kind],
            [ScanNavigation, ScanTerminal, InnerJoin, WhereFilter]
        ),
        case!(
            "field-inside",
            Navigation,
            "inside(field=value, kind)",
            [NavigationTerminal, Kind],
            [ScanNavigation, ScanTerminal, InnerJoin, WhereFilter]
        ),
        case!(
            "nth-child-fixed",
            Navigation,
            "nthChild(2)",
            [NavigationTerminal],
            [ScanNavigation, WindowRank, WhereFilter]
        ),
        case!(
            "nth-child-formula",
            Navigation,
            "nthChild(2n+1)",
            [NavigationTerminal],
            [ScanNavigation, WindowRank, WhereFilter]
        ),
        case!(
            "nth-child-of-rule",
            Navigation,
            "nthChild(2, ofRule=not(comment))",
            [NavigationTerminal, Kind],
            [ScanNavigation, ScanTerminal, AntiJoin, WindowRank]
        ),
        case!(
            "count-descendants-range",
            Navigation,
            "count(has(kind), 2..4)",
            [NavigationTerminal, Kind],
            [ScanNavigation, ScanTerminal, InnerJoin, GroupBy, Having]
        ),
        case!(
            "count-ancestors-min",
            Navigation,
            "count(inside(kind), min=1)",
            [NavigationTerminal, Kind],
            [ScanNavigation, ScanTerminal, InnerJoin, GroupBy, Having]
        ),
        case!(
            "count-following-max",
            Navigation,
            "count(precedes(kind), max=3)",
            [NavigationTerminal, Kind],
            [ScanNavigation, ScanTerminal, InnerJoin, GroupBy, Having]
        ),
        case!(
            "count-preceding-exact",
            Navigation,
            "count(follows(kind), exact=2)",
            [NavigationTerminal, Kind],
            [ScanNavigation, ScanTerminal, InnerJoin, GroupBy, Having]
        ),
        case!(
            "count-distinct-capture",
            Navigation,
            "count(has(pattern($A)), distinct=$A)",
            [NavigationTerminal, Pattern],
            [
                ScanNavigation,
                ScanTerminal,
                ScanCapture,
                InnerJoin,
                Distinct,
                GroupBy,
                Having
            ]
        ),
        // Capture and output-value forms: 14.
        case!(
            "capture-single-binding",
            Capture,
            "pattern($A) then bind($A)",
            [Pattern],
            [ScanTerminal, ScanCapture, Bindings]
        ),
        case!(
            "capture-multi-binding",
            Capture,
            "pattern($$$A) then bind-many($A)",
            [Pattern],
            [ScanTerminal, ScanCapture, Bindings, OrderBy]
        ),
        case!(
            "capture-repeated-structural-equality",
            Capture,
            "compare($A, $A, structural)",
            [Equality],
            [ScanTerminal, ScanCapture, InnerJoin, Bindings]
        ),
        case!(
            "capture-cross-term-equality",
            Capture,
            "all(pattern($A), pattern($B), eq($A,$B))",
            [Pattern, Equality],
            [ScanTerminal, ScanCapture, InnerJoin, Bindings]
        ),
        case!(
            "capture-cross-term-inequality",
            Capture,
            "all(pattern($A), pattern($B), ne($A,$B))",
            [Pattern, Equality],
            [ScanTerminal, ScanCapture, InnerJoin, AntiJoin, Bindings]
        ),
        case!(
            "capture-alternative-rollback",
            Capture,
            "any(all(pattern($A),false), pattern($A))",
            [Pattern],
            [
                ScanTerminal,
                ScanCapture,
                OrderedUnion,
                WindowRank,
                Bindings
            ]
        ),
        case!(
            "capture-negation-isolation",
            Capture,
            "all(pattern($A), not(pattern($B)))",
            [Pattern],
            [ScanTerminal, ScanCapture, InnerJoin, AntiJoin, Bindings]
        ),
        case!(
            "capture-each-distinct",
            Capture,
            "violation.each($A)",
            [Pattern],
            [ScanTerminal, ScanCapture, Distinct, Projection, OrderBy]
        ),
        case!(
            "capture-anchor",
            Capture,
            "anchor(capture=$A)",
            [Pattern],
            [ScanTerminal, ScanCapture, LeftJoin, Projection]
        ),
        case!(
            "capture-anchor-fallback",
            Capture,
            "anchor(capture=$MISSING, fallback=span)",
            [Pattern],
            [ScanTerminal, ScanCapture, LeftJoin, CaseTruth, Projection]
        ),
        case!(
            "capture-transform",
            Capture,
            "transform($A, downcase)",
            [Pattern, Transform],
            [ScanTerminal, ScanCapture, InnerJoin, Bindings]
        ),
        case!(
            "capture-message-template",
            Capture,
            "message(template($A))",
            [Pattern],
            [ScanTerminal, ScanCapture, LeftJoin, Projection]
        ),
        case!(
            "capture-fix-range",
            Capture,
            "fix(range=$A, template=$B)",
            [Pattern],
            [ScanTerminal, ScanCapture, InnerJoin, Projection]
        ),
        case!(
            "capture-projection",
            Capture,
            "project(name=$A, value=$B)",
            [Pattern],
            [ScanTerminal, ScanCapture, InnerJoin, Projection, OrderBy]
        ),
        // Utility and component forms: 10.
        case!(
            "utility-simple",
            Utility,
            "matches(simple-kind)",
            [Kind],
            [ScanTerminal, InnerJoin]
        ),
        case!(
            "utility-nested",
            Utility,
            "matches(outer -> inner)",
            [Kind, Regex],
            [ScanTerminal, InnerJoin]
        ),
        case!(
            "utility-shared-component",
            Utility,
            "all(matches(shared), matches(shared))",
            [Pattern],
            [ScanTerminal, InnerJoin]
        ),
        case!(
            "utility-self-recursive",
            Utility,
            "matches(recursive-self)",
            [NavigationTerminal, Kind],
            [ScanNavigation, ScanTerminal, RecursiveCte, CaseTruth]
        ),
        case!(
            "utility-mutual-recursion",
            Utility,
            "matches(a -> b -> a)",
            [NavigationTerminal, Kind],
            [
                ScanNavigation,
                ScanTerminal,
                RecursiveCte,
                OrderedUnion,
                CaseTruth
            ]
        ),
        case!(
            "utility-parameterized-call",
            Utility,
            "call(util, node=$A, text=$B)",
            [Pattern],
            [ScanTerminal, ScanCapture, InnerJoin, Bindings]
        ),
        case!(
            "utility-argument-frame-isolation",
            Utility,
            "any(call(util,$A), call(util,$B))",
            [Pattern],
            [
                ScanTerminal,
                ScanCapture,
                OrderedUnion,
                WindowRank,
                Bindings
            ]
        ),
        case!(
            "utility-capture-return",
            Utility,
            "matches(util-binds-$A)",
            [Pattern],
            [ScanTerminal, ScanCapture, InnerJoin, Bindings]
        ),
        case!(
            "utility-depth-limit",
            Utility,
            "matches(recursive, maxDepth)",
            [NavigationTerminal],
            [ScanNavigation, RecursiveCte, CaseTruth]
        ),
        case!(
            "utility-work-limit",
            Utility,
            "matches(recursive, maxVisitedNodes)",
            [NavigationTerminal],
            [ScanNavigation, RecursiveCte, CaseTruth]
        ),
        // Semantic fact and generated-document forms: 22.
        case!(
            "semantic-resolves",
            Semantic,
            "resolves(symbol)",
            [Fact],
            [ScanFact, InnerJoin, CaseTruth]
        ),
        case!(
            "semantic-binding-site",
            Semantic,
            "resolves.binding.site(rule)",
            [Fact, Kind],
            [ScanFact, ScanTerminal, InnerJoin, Bindings]
        ),
        case!(
            "semantic-binding-value",
            Semantic,
            "resolves.binding.value(rule)",
            [Fact, Pattern],
            [ScanFact, ScanTerminal, InnerJoin, Bindings]
        ),
        case!(
            "semantic-binding-annotation",
            Semantic,
            "resolves.binding.annotation(rule)",
            [Fact, Pattern],
            [ScanFact, ScanTerminal, LeftJoin, Bindings]
        ),
        case!(
            "semantic-binding-occurrence",
            Semantic,
            "resolves.binding.occurrence(rule)",
            [Fact, Pattern],
            [ScanFact, ScanTerminal, InnerJoin, WindowRank, Bindings]
        ),
        case!(
            "semantic-binds",
            Semantic,
            "binds(filter, subrules)",
            [Fact, Pattern],
            [ScanFact, ScanTerminal, InnerJoin, WindowRank, Bindings]
        ),
        case!(
            "semantic-redefines-any",
            Semantic,
            "binds.redefines(any)",
            [Fact, Pattern],
            [ScanFact, ScanTerminal, InnerJoin, WindowRank, Bindings]
        ),
        case!(
            "semantic-redefines-every",
            Semantic,
            "binds.redefines(every)",
            [Fact, Pattern],
            [ScanFact, ScanTerminal, GroupBy, Having, Bindings]
        ),
        case!(
            "semantic-value-literal",
            Semantic,
            "value(literal-kind)",
            [Value],
            [ScanTerminal, CaseTruth]
        ),
        case!(
            "semantic-value-flow-unknown",
            Semantic,
            "value(flow)",
            [Value, Fact],
            [ScanTerminal, ScanFact, LeftJoin, CaseTruth]
        ),
        case!(
            "semantic-arguments-bind",
            Semantic,
            "arguments(signature, bind)",
            [Fact],
            [ScanFact, InnerJoin, Bindings]
        ),
        case!(
            "semantic-arguments-filter",
            Semantic,
            "arguments(signature, where)",
            [Fact, Pattern],
            [ScanFact, ScanTerminal, InnerJoin, Bindings]
        ),
        case!(
            "semantic-parse-literal",
            Semantic,
            "parse(grammar, rule)",
            [Parse, Pattern],
            [MapDocument, ScanTerminal, InnerJoin, Bindings]
        ),
        case!(
            "semantic-parse-host",
            Semantic,
            "parse(grammar, host(rule))",
            [Parse, Pattern],
            [MapDocument, ScanTerminal, InnerJoin, Bindings]
        ),
        case!(
            "semantic-parse-capture-export",
            Semantic,
            "parse(grammar, capture($A))",
            [Parse, Pattern],
            [MapDocument, ScanTerminal, ScanCapture, InnerJoin, Bindings]
        ),
        case!(
            "semantic-format-unused-arguments",
            Semantic,
            "formatArgs(unused)",
            [Fact, Pattern],
            [ScanFact, ScanTerminal, AntiJoin, Bindings]
        ),
        case!(
            "semantic-format-missing-fields",
            Semantic,
            "formatArgs(missing)",
            [Fact, Pattern],
            [ScanFact, ScanTerminal, AntiJoin, Bindings]
        ),
        case!(
            "semantic-layout",
            Semantic,
            "layout(lines, neighbour)",
            [Fact],
            [ScanFact, InnerJoin, CaseTruth]
        ),
        case!(
            "semantic-calls-direct",
            Semantic,
            "calls(definition)",
            [CallSeed, Fact],
            [ScanFact, CrossFileJoin, Exists]
        ),
        case!(
            "semantic-calls-through",
            Semantic,
            "calls(definition, through)",
            [CallSeed, Fact],
            [ScanFact, CrossFileJoin, RecursiveCte, Exists]
        ),
        case!(
            "semantic-generated-document-map",
            Semantic,
            "generated(source-map, rule)",
            [Parse, Pattern],
            [MapDocument, ScanTerminal, InnerJoin, Projection]
        ),
        case!(
            "semantic-fact-producer-failure",
            Semantic,
            "binding-facts(capacity-failure)",
            [Fact],
            [ScanFact, LeftJoin, CaseTruth]
        ),
        // Clause, file, completion, and output forms: 14.
        case!(
            "clause-forbid-ast",
            ClauseOutput,
            "forbid(ast)",
            [Pattern],
            [ScanTerminal, Projection, OrderBy]
        ),
        case!(
            "clause-forbid-content-regex",
            ClauseOutput,
            "forbid(regex(content))",
            [Regex],
            [ScanTerminal, Projection, OrderBy]
        ),
        case!(
            "clause-forbid-path-regex",
            ClauseOutput,
            "forbid(regex(path))",
            [Regex],
            [FileFilter, ScanTerminal, Projection, OrderBy]
        ),
        case!(
            "clause-select-when-exists",
            ClauseOutput,
            "selectWhen(select, exists=true)",
            [Pattern, Kind],
            [ScanTerminal, Exists, InnerJoin, Projection]
        ),
        case!(
            "clause-select-when-absent",
            ClauseOutput,
            "selectWhen(select, exists=false)",
            [Pattern, Kind],
            [ScanTerminal, AntiJoin, Projection]
        ),
        case!(
            "clause-require-each",
            ClauseOutput,
            "requireEach(subject, evidence)",
            [Pattern, Fact],
            [ScanTerminal, ScanFact, CrossFileJoin, AntiJoin, Projection]
        ),
        case!(
            "clause-budget-maximum",
            ClauseOutput,
            "budget(query, maximum)",
            [Pattern],
            [ScanTerminal, GroupBy, Having, Projection]
        ),
        case!(
            "clause-budget-distinct",
            ClauseOutput,
            "budget(query, distinct=$A)",
            [Pattern],
            [
                ScanTerminal,
                ScanCapture,
                Distinct,
                GroupBy,
                Having,
                Projection
            ]
        ),
        case!(
            "clause-budget-group",
            ClauseOutput,
            "budget(query, groupBy=$A)",
            [Pattern],
            [
                ScanTerminal,
                ScanCapture,
                GroupBy,
                Having,
                Projection,
                OrderBy
            ]
        ),
        case!(
            "clause-file-condition",
            ClauseOutput,
            "file(path, language, content)",
            [Regex],
            [FileFilter, ScanTerminal, Projection]
        ),
        case!(
            "clause-syntax-error-unknown",
            ClauseOutput,
            "unsound-rule(error-overlap)",
            [Pattern, Fact],
            [ScanTerminal, ScanFact, LeftJoin, CaseTruth, Projection]
        ),
        case!(
            "output-fix-and-suppression",
            ClauseOutput,
            "violation(fix, suppress)",
            [Pattern, Fact, Transform],
            [ScanTerminal, ScanCapture, ScanFact, AntiJoin, Projection]
        ),
        case!(
            "output-complete-negative",
            ClauseOutput,
            "forbid(no terminal matches)",
            [Pattern],
            [ScanTerminal, AntiJoin, Projection]
        ),
        case!(
            "output-stable-multiplicity",
            ClauseOutput,
            "violation(each alternative and capture)",
            [Pattern],
            [
                ScanTerminal,
                ScanCapture,
                OrderedUnion,
                Distinct,
                Projection,
                OrderBy
            ]
        ),
    ]
}

#[derive(Debug, Eq, PartialEq)]
struct ExplainedLowering {
    terminals: Vec<Terminal>,
    sql: Vec<SqlShape>,
}

/// Runs the stable hand-written suite against a compiler plan-explanation adapter.
///
/// The adapter can normalize aliases and compiler-assigned identifiers. It cannot remove or
/// replace a required terminal or SQL construct from the contract.
fn assert_compiler_contract(mut explain: impl FnMut(&str) -> Result<ExplainedLowering, String>) {
    for case in cases() {
        let actual = explain(case.form)
            .unwrap_or_else(|error| panic!("{} (`{}`): {error}", case.name, case.form));
        assert_eq!(actual.terminals, case.terminals, "{} terminals", case.name);
        assert_eq!(actual.sql, case.sql, "{} SQL shape", case.name);
    }
}

#[test]
fn relational_compiler_matches_all_hand_written_forms() {
    assert_compiler_contract(|form| {
        let ir = contract_frontend::lower(form)?;
        Ok(ExplainedLowering {
            terminals: ir.terminals().to_vec(),
            sql: ir.operations().to_vec(),
        })
    });
}

#[test]
fn lowering_contract_fixture_has_108_hand_written_forms() {
    let cases = cases();
    assert_eq!(cases.len(), 108);

    let mut names = BTreeSet::new();
    let mut forms = BTreeSet::new();
    for case in &cases {
        assert!(
            names.insert(case.name),
            "duplicate case name: {}",
            case.name
        );
        assert!(
            forms.insert(case.form),
            "duplicate lowered form: {}",
            case.form
        );
        assert_eq!(
            case.sql.first(),
            Some(&SqlShape::PackedMaterialized),
            "{}",
            case.name
        );
        assert!(
            case.sql.contains(&SqlShape::StatusJoin),
            "{} has no completion join",
            case.name
        );
    }
}

#[test]
fn lowering_contract_fixture_sql_shapes_are_callback_free() {
    for case in cases() {
        let shape = case
            .sql
            .iter()
            .map(|part| part.fragment())
            .collect::<Vec<_>>()
            .join(" -> ");
        assert!(
            shape.starts_with("WITH packed AS MATERIALIZED"),
            "{}: {shape}",
            case.name
        );
        assert!(
            shape.contains("JOIN producer_status"),
            "{}: {shape}",
            case.name
        );
        for prohibited in ["tq_", "rerail_nodes", "rerail_facts", "GrammarMatcher"] {
            assert!(
                !shape.contains(prohibited),
                "{} contains {prohibited}: {shape}",
                case.name
            );
        }
    }
}

#[test]
fn lowering_contract_fixture_covers_each_main_family() {
    let counts = cases()
        .into_iter()
        .fold(BTreeMap::new(), |mut counts, case| {
            *counts.entry(case.family).or_insert(0) += 1;
            counts
        });
    assert_eq!(
        counts,
        BTreeMap::from([
            (Family::Atomic, 12),
            (Family::Boolean, 14),
            (Family::Navigation, 22),
            (Family::Capture, 14),
            (Family::Utility, 10),
            (Family::Semantic, 22),
            (Family::ClauseOutput, 14),
        ])
    );
}

#[test]
fn lowering_contract_fixture_uses_all_terminal_and_sql_shapes() {
    let cases = cases();
    let terminals = cases
        .iter()
        .flat_map(|case| case.terminals.iter().copied())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        terminals,
        BTreeSet::from([
            Terminal::Pattern,
            Terminal::Kind,
            Terminal::Regex,
            Terminal::Text,
            Terminal::Value,
            Terminal::Navigation,
            Terminal::Equality,
            Terminal::Fact,
            Terminal::Parse,
            Terminal::CallSeed,
            Terminal::Transform,
        ])
    );

    let sql = cases
        .iter()
        .flat_map(|case| case.sql.iter().copied())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        sql,
        BTreeSet::from([
            SqlShape::PackedMaterialized,
            SqlShape::StatusJoin,
            SqlShape::ScanTerminal,
            SqlShape::ScanCapture,
            SqlShape::ScanNavigation,
            SqlShape::ScanFact,
            SqlShape::InnerJoin,
            SqlShape::LeftJoin,
            SqlShape::OrderedUnion,
            SqlShape::AntiJoin,
            SqlShape::Exists,
            SqlShape::RecursiveCte,
            SqlShape::GroupBy,
            SqlShape::Having,
            SqlShape::WindowRank,
            SqlShape::MinPosition,
            SqlShape::Distinct,
            SqlShape::CaseTruth,
            SqlShape::Bindings,
            SqlShape::MapDocument,
            SqlShape::Projection,
            SqlShape::OrderBy,
            SqlShape::WhereFilter,
            SqlShape::FileFilter,
            SqlShape::CrossFileJoin,
        ])
    );
}
