//! Parsed captures retain decoded text and host coordinates across SQL composition.
use super::publication_tests::check_work;
use serde_json::{Value, json};

fn check(language: &str, rule: Value, utils: Value, sources: &[&str]) {
    let policy = super::parse_tests::policy(language, rule.clone(), utils);
    for allowance in [0, 1, 2, 3, 4, 8, 16, 64, 1000, 10000] {
        check_work(
            &policy,
            std::slice::from_ref(&rule),
            sources,
            None,
            allowance,
        );
    }
}

#[test]
fn parsed_capture_maps_escape_bytes_and_commits_only_successful_branch() {
    for inner in [
        json!({"has":{"kind":"char","capture":"C","value":{"eq":"y"}}}),
        json!({"has":{"kind":"char","any":[{"all":[{"capture":"C"},{"regex":"^never$"}]},{"capture":"C","regex":"^y$"}]}}),
        json!({"has":{"kind":"char","capture":"C","regex":"^y$","not":{"has":{"kind":"char","capture":"Z"}}}}),
    ] {
        let mut parse = inner;
        parse["grammar"] = json!("testChars");
        check(
            "python",
            json!({"kind":"string","parse":parse}),
            json!({}),
            &["a='q\\x79z'\nb='xy'\nc='none'\n"],
        );
    }
}

#[test]
fn nested_annotation_exports_map_through_every_document() {
    check(
        "python",
        json!({"kind":"string","parse":{"grammar":"annotation","has":{"kind":"string","parse":{"grammar":"annotation","has":{"kind":"identifier","pattern":"$T","stopBy":"end"}},"stopBy":"end"}}}),
        json!({}),
        &["from typing import List\na: 'List[\"int\"]'\nb: 'List[\"str\"]'\n"],
    );
}

#[test]
fn exported_capture_compares_as_text_and_preserves_existing_node_precedence() {
    let parse =
        json!({"parse":{"grammar":"testChars","has":{"kind":"char","capture":"C","regex":"^y$"}}});
    check(
        "python",
        json!({"kind":"assignment","pattern":"$A = $S","has":{"kind":"string","all":[parse.clone(),{"compare":{"left":"C","right":"A","op":"eq","text":"source"}}],"stopBy":"end"}}),
        json!({}),
        &["y='x\\x79'\nz='y'\n"],
    );
    check(
        "python",
        json!({"kind":"string","pattern":"$C","all":[parse,{"compare":{"left":"C","right":"C","op":"eq","text":"node"}}]}),
        json!({}),
        &["a='xy'\nb='y'\n"],
    );
}

#[test]
fn exports_cover_percent_brace_php_and_rust_error_nodes() {
    for (language, grammar, kind, source) in [
        (
            "python",
            "percentFormat",
            "string",
            "a='%'\nb='%q'\nc='%s'\n",
        ),
        ("python", "braceFormat", "string", "a='{'\nb='{}'\n"),
        (
            "php",
            "printfFormat",
            "string",
            "<?php $a='%'; $b='%q'; $c='%s';",
        ),
        (
            "php",
            "scanfFormat",
            "string",
            "<?php $a='%'; $b='%q'; $c='%d';",
        ),
        (
            "rust",
            "rustFormat",
            "string_literal",
            "fn main(){let a=\"{\";let b=\"{}\";}",
        ),
    ] {
        check(
            language,
            json!({"kind":kind,"parse":{"grammar":grammar,"has":{"kind":"error","capture":"E","value":{"regex":"."}}}}),
            json!({}),
            &[source],
        );
    }
}

#[test]
fn host_rule_runs_for_each_candidate_and_reads_exported_text() {
    let declared = json!({"inside":{"kind":"program","has":{"kind":"variable_declarator","has":{"field":"name","pattern":"$V"},"compare":{"left":"V","right":"N","op":"eq","text":"name"},"stopBy":"end"}}});
    for host in [declared.clone(), json!({"not":declared})] {
        check(
            "typescript",
            json!({"kind":"comment","parse":{"grammar":"eslintDirective","value":{"regex":"^globals?$"},"has":{"kind":"item","capture":"N","host":host}}}),
            json!({}),
            &["/*global b, a, c*/ var a, c;\n", "/*global z*/ var a;\n"],
        );
    }
}

#[test]
fn host_bindings_remain_private_and_original_host_capture_is_visible() {
    check(
        "typescript",
        json!({"kind":"comment","pattern":"$C","parse":{"grammar":"eslintDirective","has":{"kind":"item","capture":"N","host":{"pattern":"$C"}}}}),
        json!({}),
        &["/*global a, b*/\n"],
    );
}

#[test]
fn parameter_outputs_unify_span_channels_and_node_shadows_transactionally() {
    for second in ["y", "z"] {
        let utils = json!({"pair":{"params":["P","Q"],"rule":{"all":[
            {"parse":{"grammar":"testChars","has":{"kind":"char","capture":"P","regex":"^y$"}}},
            {"parse":{"grammar":"testChars","has":{"kind":"char","capture":"Q","regex":format!("^{second}$")}}}
        ]}}});
        check(
            "python",
            json!({"kind":"string","any":[{"matches":{"pair":{"P":"$C","Q":"$C"}}},{"pattern":"$Y"}]}),
            utils,
            &["a='yz'\nb='yy'\n"],
        );
    }
    let utils = json!({"pair":{"params":["P","Q"],"rule":{"all":[{"pattern":"$P"},{"parse":{"grammar":"testChars","has":{"kind":"char","capture":"Q","regex":"^y$"}}}]}}});
    check(
        "python",
        json!({"kind":"string","matches":{"pair":{"P":"$C","Q":"$C"}}}),
        utils,
        &["a='xy'\nb='y'\n"],
    );
}

#[test]
fn parameter_existing_transformed_capture_does_not_trigger_output_unification() {
    let utils = json!({"replace":{"params":["P"],"rule":{"parse":{"grammar":"testChars","has":{"kind":"char","capture":"P","regex":"^z$"}}}}});
    check(
        "python",
        json!({"kind":"string","all":[
            {"parse":{"grammar":"testChars","has":{"kind":"char","capture":"C","regex":"^y$"}}},
            {"matches":{"replace":{"P":"$C"}}}
        ]}),
        utils,
        &["a='yz'\nb='y'\n"],
    );
}

#[test]
fn parsed_parameter_frames_return_local_nodes_as_host_spans() {
    let utils = json!({"pick":{"params":["P"],"rule":{"kind":"identifier","pattern":"$P","regex":"^int$"}}});
    check(
        "python",
        json!({"kind":"string","parse":{"grammar":"annotation","has":{"matches":{"pick":{"P":"$T"}},"stopBy":"end"}}}),
        utils,
        &["a: 'list[int]'\nb: 'tuple[str,int]'\nc: 'str'\n"],
    );
}

#[test]
fn parsed_parameter_value_calls_are_transient_and_keep_probe_order() {
    let utils = json!({"probe":{"params":["P"],"rule":{"all":[{"compare":{"left":"P","right":"P","op":"eq","text":"source"}},{"value":{"regex":"."}}]}}});
    check(
        "python",
        json!({"kind":"string","parse":{"grammar":"annotation","has":{"kind":"identifier","pattern":"$T","all":[{"matches":{"probe":{"P":"$T"}}},{"matches":{"probe":{"P":"$T"}}}],"stopBy":"end"}}}),
        utils,
        &["a: 'list[int]'\nb: 'str'\n"],
    );
}

#[test]
fn parsed_pure_recursive_walk_uses_each_current_document_identity() {
    let utils = json!({"walk":{"any":[{"kind":"identifier","regex":"^int$"},{"has":{"matches":"walk","stopBy":"neighbor"}}]}});
    check(
        "python",
        json!({"kind":"string","parse":{"grammar":"annotation","matches":"walk"}}),
        utils,
        &["a: 'list[int]'\nb: 'tuple[list[int],str]'\nc: 'list[str]'\n"],
    );
}

#[test]
fn parsed_readonly_recursive_regions_keep_annotation_scopes_and_memo_bypass() {
    let utils = json!({"walk":{"any":[{"kind":"identifier","resolves":{"global":["int"]}},{"has":{"matches":"walk","stopBy":"end"}}]}});
    check(
        "python",
        json!({"kind":"string","parse":{"grammar":"annotation","matches":"walk"}}),
        utils,
        &[
            "a: 'list[int]'\nb: 'tuple[str,int]'\nc: 'list[str]'\n",
            "int = replacement\na: 'list[int]'\n",
        ],
    );
}

#[test]
fn nested_parameter_frames_preserve_late_discovered_span_shadows() {
    let utils = json!({
        "a":{"params":["P"],"rule":{"matches":{"b":{"Q":"$P"}}}},
        "b":{"params":["Q"],"rule":{"all":[{"pattern":"$Q"},{"matches":"probe"}]}},
        "probe":{"compare":{"left":"Q","right":"Q","op":"eq","text":"source"},"value":{"regex":"."}}
    });
    check(
        "python",
        json!({"kind":"string","all":[
            {"parse":{"grammar":"testChars","has":{"kind":"char","capture":"C","regex":"^y$"}}},
            {"matches":{"a":{"P":"$C"}}},
            {"parse":{"grammar":"testChars","has":{"kind":"char","capture":"C","regex":"^z$"}}},
            {"matches":{"a":{"P":"$C"}}}
        ]}),
        utils,
        &["a='yz'\nb='y\\x7az'\nc='y'\n"],
    );
}
