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

/// Bandit's `# nosec` comments: the test ids a list must name (or name none).
pub(crate) struct Suppressions;

impl super::SuppressionKind for Suppressions {
    const KEY: &'static str = "nosec";
    const DETAIL: &'static str = "nosec: nosec comments are Bandit's";
    type Spec = Vec<String>;
    fn spec(value: &serde_json::Value, at: &str) -> napi::Result<Vec<String>> {
        spec(value, at)
    }
}

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

    /// Each line holding a `# nosec` comment, with the tests it names (none: every test).
    pub(crate) fn directives(&self) -> impl Iterator<Item = (usize, &[&'static str])> {
        self.lines.iter().filter_map(|(&line, tests)| Some((line, tests.as_deref()?)))
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
mod tests;
