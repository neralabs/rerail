//! Bandit 1.9.4's `# nosec` comments (`bandit/core/manager.py`'s `NOSEC_COMMENT`,
//! `NOSEC_COMMENT_TESTS` and `_parse_nosec_comment`; `bandit/core/tester.py`'s
//! `_get_nosecs_from_contexts`; `bandit/core/utils.py`'s `get_nosec`), at
//! `92ae8b82fb422a639f0ed8d99e96cea769594e08`.

use super::names;
use crate::code_rule_dsl::json::{object, reject_unknown};
use crate::code_rule_dsl::lines::LineTable;
use ast_grep_core::{Doc, Node};
use regex::Regex;
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::LazyLock;

/// `nosec.codes`: the Bandit test ids a `# nosec` list must name (or name none) to suppress the
/// violation.
pub(super) fn spec(nosec: &Value, at: &str) -> napi::Result<Vec<String>> {
    reject_unknown(object(nosec, at)?, &["codes"], at)?;
    names(
        nosec.get("codes"),
        &format!("{at}/codes"),
        bandit_test,
        "Bandit test ids",
        "a Bandit test id (B, then digits: B101)",
    )
}

/// A Bandit test id a `# nosec` list can name: `B`, then digits.
fn bandit_test(id: &str) -> bool {
    id.len() > 1 && id.starts_with('B') && id[1..].bytes().all(|byte| byte.is_ascii_digit())
}

/// Bandit 1.9.4's registered tests, by id and name (`extension_loader.MANAGER`'s
/// `plugins_by_id`/`blacklist_by_id` and `plugins_by_name`/`blacklist_by_name`). A `# nosec` list
/// names a test by either; B001, the built-in blacklist test, has an id only.
const TESTS: &[(&str, &str)] = &[
    ("B101", "assert_used"),
    ("B102", "exec_used"),
    ("B103", "set_bad_file_permissions"),
    ("B104", "hardcoded_bind_all_interfaces"),
    ("B105", "hardcoded_password_string"),
    ("B106", "hardcoded_password_funcarg"),
    ("B107", "hardcoded_password_default"),
    ("B108", "hardcoded_tmp_directory"),
    ("B110", "try_except_pass"),
    ("B112", "try_except_continue"),
    ("B113", "request_without_timeout"),
    ("B201", "flask_debug_true"),
    ("B202", "tarfile_unsafe_members"),
    ("B301", "pickle"),
    ("B302", "marshal"),
    ("B303", "md5"),
    ("B304", "ciphers"),
    ("B305", "cipher_modes"),
    ("B306", "mktemp_q"),
    ("B307", "eval"),
    ("B308", "mark_safe"),
    ("B310", "urllib_urlopen"),
    ("B311", "random"),
    ("B312", "telnetlib"),
    ("B313", "xml_bad_cElementTree"),
    ("B314", "xml_bad_ElementTree"),
    ("B315", "xml_bad_expatreader"),
    ("B316", "xml_bad_expatbuilder"),
    ("B317", "xml_bad_sax"),
    ("B318", "xml_bad_minidom"),
    ("B319", "xml_bad_pulldom"),
    ("B321", "ftplib"),
    ("B323", "unverified_context"),
    ("B324", "hashlib_insecure_functions"),
    ("B401", "import_telnetlib"),
    ("B402", "import_ftplib"),
    ("B403", "import_pickle"),
    ("B404", "import_subprocess"),
    ("B405", "import_xml_etree"),
    ("B406", "import_xml_sax"),
    ("B407", "import_xml_expat"),
    ("B408", "import_xml_minidom"),
    ("B409", "import_xml_pulldom"),
    ("B411", "import_xmlrpclib"),
    ("B412", "import_httpoxy"),
    ("B413", "import_pycrypto"),
    ("B415", "import_pyghmi"),
    ("B501", "request_with_no_cert_validation"),
    ("B502", "ssl_with_bad_version"),
    ("B503", "ssl_with_bad_defaults"),
    ("B504", "ssl_with_no_version"),
    ("B505", "weak_cryptographic_key"),
    ("B506", "yaml_load"),
    ("B507", "ssh_no_host_key_verification"),
    ("B508", "snmp_insecure_version"),
    ("B509", "snmp_weak_cryptography"),
    ("B601", "paramiko_calls"),
    ("B602", "subprocess_popen_with_shell_equals_true"),
    ("B603", "subprocess_without_shell_equals_true"),
    ("B604", "any_other_function_with_shell_equals_true"),
    ("B605", "start_process_with_a_shell"),
    ("B606", "start_process_with_no_shell"),
    ("B607", "start_process_with_partial_path"),
    ("B608", "hardcoded_sql_expressions"),
    ("B609", "linux_commands_wildcard_injection"),
    ("B610", "django_extra_used"),
    ("B611", "django_rawsql_used"),
    ("B612", "logging_config_insecure_listen"),
    ("B613", "trojansource"),
    ("B614", "pytorch_load"),
    ("B615", "huggingface_unsafe_download"),
    ("B701", "jinja2_autoescape_false"),
    ("B702", "use_of_mako_templates"),
    ("B703", "django_mark_safe"),
    ("B704", "markupsafe_markup_xss"),
];

/// `NOSEC_COMMENT`, searched anywhere in a comment.
static NOSEC_COMMENT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"#\s*nosec:?\s*(?P<tests>[^#]+)?#?").expect("NOSEC_COMMENT"));
/// `NOSEC_COMMENT_TESTS` (`re.IGNORECASE`): each match names one test, its group 1, which holds
/// the last repetition (`B101,B102` names only B102, as in Bandit).
static NOSEC_COMMENT_TESTS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(?:(B\d+|[a-z\d_]+),?)+").expect("NOSEC_COMMENT_TESTS"));

/// `_find_test_id_from_nosec_string`: a test id (`check_id`), else a test name's id
/// (`get_test_id`); anything else names no test.
fn test_id(word: &str) -> Option<&'static str> {
    if word == "B001" {
        return Some("B001");
    }
    TESTS
        .iter()
        .find(|(id, _)| *id == word)
        .or_else(|| TESTS.iter().find(|(_, name)| *name == word))
        .map(|(id, _)| *id)
}

/// `_parse_nosec_comment`: `None` for a comment without `# nosec`, else the tests it names. An
/// empty list is a blanket `# nosec`: no list, or a list naming no known test (`# nosec TODO`).
pub(crate) fn parse(comment: &str) -> Option<Vec<&'static str>> {
    let found = NOSEC_COMMENT.captures(comment)?;
    let mut ids = Vec::new();
    if let Some(tests) = found.name("tests") {
        for test in NOSEC_COMMENT_TESTS.captures_iter(tests.as_str()) {
            if let Some(id) = test.get(1).and_then(|word| test_id(word.as_str()))
                && !ids.contains(&id)
            {
                ids.push(id);
            }
        }
    }
    Some(ids)
}

/// A Python file's comments, by line: Bandit's `nosec_lines` (`None` for a comment that is no
/// `# nosec`).
#[derive(Debug, Default)]
pub(crate) struct NosecIndex {
    lines: BTreeMap<usize, Option<Vec<&'static str>>>,
}

impl NosecIndex {
    /// Reads every comment of the file `root` is the root of.
    pub(crate) fn read<D: Doc>(root: &Node<'_, D>, source: &str, lines: &LineTable) -> Self {
        let mut index = Self::default();
        for node in root.dfs() {
            if node.kind() == "comment" {
                let range = node.range();
                index
                    .lines
                    .insert(lines.line(range.start), parse(&source[range]));
            }
        }
        index
    }

    /// Whether a finding of one of `codes`, reported at `at` on a node spanning `node`, is
    /// suppressed (`_get_nosecs_from_contexts`): the tests named on the reported line, together
    /// with those of the first `# nosec` among the node's lines (`get_nosec` over
    /// `context["linerange"]`). Neither present: not suppressed. Present but naming no test: a
    /// blanket `# nosec`. Otherwise only the named tests.
    pub(crate) fn suppresses(
        &self,
        codes: &[String],
        at: usize,
        node: (usize, usize),
        lines: &LineTable,
    ) -> bool {
        let named = |line: usize| self.lines.get(&line).and_then(Option::as_ref);
        let reported = named(lines.line(at));
        let first = lines.line(node.0);
        let last = lines.line(node.1.max(node.0 + 1) - 1).max(first);
        let context = (first..=last).find_map(named);
        if reported.is_none() && context.is_none() {
            return false;
        }
        let tests = reported
            .into_iter()
            .chain(context)
            .flatten()
            .collect::<Vec<_>>();
        tests.is_empty()
            || tests
                .iter()
                .any(|test| codes.iter().any(|code| code == *test))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code_rule_dsl::lines::Terminators;

    #[test]
    fn nosec_comments_parse_as_bandit_parses_them() {
        assert_eq!(parse("# a comment"), None);
        assert_eq!(parse("# NOSEC"), None, "case-sensitive");
        assert_eq!(parse("# nosec"), Some(vec![]));
        assert_eq!(parse("#nosec"), Some(vec![]));
        assert_eq!(parse("# nosec:"), Some(vec![]));
        assert_eq!(parse("# nosec B101"), Some(vec!["B101"]));
        assert_eq!(parse("# nosec: B101"), Some(vec!["B101"]));
        assert_eq!(parse("# nosec B101, B102"), Some(vec!["B101", "B102"]));
        assert_eq!(parse("# nosec B101 B102"), Some(vec!["B101", "B102"]));
        // One repetition of `(?:(...),?)+` without a space keeps only its last test.
        assert_eq!(parse("# nosec B101,B102"), Some(vec!["B102"]));
        assert_eq!(parse("# nosec assert_used"), Some(vec!["B101"]));
        assert_eq!(parse("# nosec random, B602"), Some(vec!["B311", "B602"]));
        // Unknown words name no test; a list of them only is a blanket nosec.
        assert_eq!(parse("# nosec TODO"), Some(vec![]));
        assert_eq!(
            parse("# nosec b101"),
            Some(vec![]),
            "ids are case-sensitive"
        );
        assert_eq!(parse("# nosec B101 TODO"), Some(vec!["B101"]));
        assert_eq!(parse("# nosec B999"), Some(vec![]));
        assert_eq!(parse("# nosec B001"), Some(vec!["B001"]));
        // The list ends at the next `#`; `# nosec` may follow another comment.
        assert_eq!(parse("# nosec B101 # B102"), Some(vec!["B101"]));
        assert_eq!(parse("# reason # nosec B602"), Some(vec!["B602"]));
        assert_eq!(parse("# nosecB101"), Some(vec!["B101"]));
    }

    fn read(source: &str) -> (NosecIndex, LineTable) {
        let lines = LineTable::new(
            source,
            Terminators::of(crate::code_rule_dsl::language::GrammarDialect::Python),
        );
        let mut index = NosecIndex::default();
        for (at, _) in source.match_indices('#') {
            let line = lines.line(at);
            if index.lines.contains_key(&line) {
                continue;
            }
            let end = lines.end(line);
            index.lines.insert(line, parse(&source[at..end]));
        }
        (index, lines)
    }

    fn codes(codes: &[&str]) -> Vec<String> {
        codes.iter().map(|code| (*code).to_owned()).collect()
    }

    #[test]
    fn nosec_suppresses_on_the_reported_line_and_the_nodes_lines() {
        let source = "a = 1  # nosec\nb = 2  # nosec B101\nc = 3  # nosec B102\nd = 4  # comment\ncall(\n    x,  # nosec B602\n)\ne = 5\n";
        let (index, lines) = read(source);
        let line = |n: usize| (lines.start(n), lines.end(n));
        let at = |n: usize| lines.start(n);
        // Bare: every test.
        assert!(index.suppresses(&codes(&["B101"]), at(0), line(0), &lines));
        // Coded: only the named tests.
        assert!(index.suppresses(&codes(&["B101"]), at(1), line(1), &lines));
        assert!(!index.suppresses(&codes(&["B101"]), at(2), line(2), &lines));
        // Another comment, or none, suppresses nothing.
        assert!(!index.suppresses(&codes(&["B101"]), at(3), line(3), &lines));
        assert!(!index.suppresses(&codes(&["B101"]), at(7), line(7), &lines));
        // A node spanning lines 4 to 6: the first `# nosec` among its lines counts.
        let call = (lines.start(4), lines.end(6));
        assert!(index.suppresses(&codes(&["B602"]), at(4), call, &lines));
        assert!(!index.suppresses(&codes(&["B603"]), at(4), call, &lines));
        // The reported line's tests join the node's.
        let both = "x = f(  # nosec B101\n    y)  # nosec B102\n";
        let (index, lines) = read(both);
        let node = (lines.start(0), lines.end(1));
        assert!(index.suppresses(&codes(&["B101"]), lines.start(1), node, &lines));
        assert!(index.suppresses(&codes(&["B102"]), lines.start(1), node, &lines));
        assert!(!index.suppresses(&codes(&["B103"]), lines.start(1), node, &lines));
        // A blanket nosec on the reported line joined with a coded one on the node's first line
        // names tests, so it is no longer blanket.
        let mixed = "x = f(  # nosec B101\n    y)  # nosec\n";
        let (index, lines) = read(mixed);
        let node = (lines.start(0), lines.end(1));
        assert!(!index.suppresses(&codes(&["B102"]), lines.start(1), node, &lines));
    }

    /// Through the engine: a rule reporting every `assert`, with `suppress.nosec` naming B101,
    /// keeps exactly the asserts Bandit 1.9.4 keeps (checked with `bandit -t B101`).
    #[test]
    fn nosec_suppresses_violations_through_the_engine() {
        use crate::code_rule_dsl::test_support::*;
        use serde_json::{Value, json};
        let violation = json!({ "anchor": "witness.span", "code": "B101", "message": "assert",
            "suppress": { "nosec": { "codes": ["B101"] } } });
        let document = clause_document(json!({ "id": "assert", "violation": violation,
            "forbid": { "ast": { "language": "python", "rule": { "kind": "assert_statement" } } } }));
        let compiled = compile_document_with(
            document,
            &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
        )
        .unwrap_or_else(|error| panic!("{}", error.reason));
        let source = "\
assert a  # nosec
assert b  # nosec B101
assert c  # nosec assert_used
assert d  # nosec B102
assert e  # nosec TODO
assert f  # noqa
assert g
assert (h,
        i)  # nosec
x = '# nosec'; assert j
";
        let result = evaluate_file(&compiled, "python", "a.py", source);
        assert_eq!(result["failures"], json!([]), "{result}");
        let lines = result["violations"]
            .as_array()
            .expect("violations")
            .iter()
            .map(|violation| violation["span"]["startLine"].as_u64().expect("line"))
            .collect::<Vec<_>>();
        // 0-based: lines 4 (another test), 6 (`# noqa`), 7 (no comment) and 10 (`# nosec` in a
        // string), as Bandit reports them.
        assert_eq!(lines, [3, 5, 6, 9]);
    }
}
