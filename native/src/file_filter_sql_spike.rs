use super::super::*;
use ast_grep_language::LanguageExt;
use rusqlite::{Connection, params};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;
use tree_sitter::Parser;

#[derive(Clone)]
struct IndexedFile {
    id: i64,
    path: String,
    language: String,
    kinds: BTreeSet<u16>,
}

struct CheckPlan {
    id: i64,
    policy_index: usize,
    check_index: usize,
}

fn tracked(root: &Path) -> Vec<PathBuf> {
    let output = Command::new("git")
        .args(["-C", root.to_str().unwrap(), "ls-files", "-z"])
        .output()
        .expect("git ls-files");
    assert!(output.status.success());
    output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|bytes| !bytes.is_empty())
        .filter_map(|bytes| {
            let relative = String::from_utf8(bytes.to_vec()).ok()?;
            language_for_path(&relative)?;
            Some(root.join(relative))
        })
        .collect()
}

fn language_for_path(path: &str) -> Option<&'static str> {
    crate::policy_dsl::language::label_of_path(path)
}

fn relative_path(repository: &Path, path: &Path) -> String {
    path.strip_prefix(repository)
        .expect("tracked file below repository")
        .to_string_lossy()
        .replace('\\', "/")
}

fn parse_kinds(language: &str, source: &[u8]) -> Option<BTreeSet<u16>> {
    let support = crate::policy_dsl::language::dialect_by_label(language)?;
    let mut parser = Parser::new();
    parser.set_language(&support.get_ts_language()).ok()?;
    let tree = parser.parse(source, None)?;
    if tree.root_node().has_error() {
        return None;
    }
    let mut kinds = BTreeSet::new();
    let mut work = vec![tree.root_node()];
    while let Some(node) = work.pop() {
        kinds.insert(node.kind_id());
        let mut cursor = node.walk();
        if cursor.goto_first_child() {
            loop {
                work.push(cursor.node());
                if !cursor.goto_next_sibling() {
                    break;
                }
            }
        }
    }
    Some(kinds)
}

fn expected_candidates(
    file: &IndexedFile,
    policies: &[Arc<CompiledPolicy>],
    sets: &mut CandidateSets,
) -> BTreeSet<(usize, usize)> {
    let input = FileInput {
        path: file.path.clone(),
        language: file.language.clone(),
        source: Arc::new(String::new()),
        module: None,
    };
    let state = make_file(input, 1, policies, sets);
    let mut expected = state.candidates.regex.iter().copied().collect::<BTreeSet<_>>();
    for id in &state.candidates.ast {
        let roots = candidate_grammar(&policies[id.0], id.1, &state.input)
            .anchors
            .root_kinds();
        if roots.is_none_or(|required| required.iter().any(|kind| file.kinds.contains(kind))) {
            expected.insert(*id);
        }
    }
    expected
}

fn percentile(values: &mut [u128], numerator: usize, denominator: usize) -> u128 {
    values.sort_unstable();
    values[(values.len().saturating_sub(1) * numerator) / denominator]
}

const CANDIDATE_SQL: &str =
    "SELECT pf.file_id
       FROM check_languages cl
       JOIN checks c ON c.check_id=cl.check_id
       JOIN policy_file pf ON pf.policy_id=c.policy_id
       JOIN files f ON f.file_id=pf.file_id
      WHERE cl.check_id=?1 AND cl.unrestricted=1
        AND (cl.language='' OR cl.language=f.language)
      UNION
     SELECT fk.file_id
       FROM check_kind ck
       JOIN file_kind fk ON fk.language=ck.language AND fk.kind_id=ck.kind_id
       JOIN checks c ON c.check_id=ck.check_id
       JOIN policy_file pf ON pf.policy_id=c.policy_id AND pf.file_id=fk.file_id
      WHERE ck.check_id=?1";

const CANDIDATE_COUNT_SQL: &str =
    "SELECT count(*) FROM (
       SELECT pf.file_id
         FROM check_languages cl
         JOIN checks c ON c.check_id=cl.check_id
         JOIN policy_file pf ON pf.policy_id=c.policy_id
         JOIN files f ON f.file_id=pf.file_id
        WHERE cl.check_id=?1 AND cl.unrestricted=1
          AND (cl.language='' OR cl.language=f.language)
        UNION
       SELECT fk.file_id
         FROM check_kind ck
         JOIN file_kind fk ON fk.language=ck.language AND fk.kind_id=ck.kind_id
         JOIN checks c ON c.check_id=ck.check_id
         JOIN policy_file pf ON pf.policy_id=c.policy_id AND pf.file_id=fk.file_id
        WHERE ck.check_id=?1
     )";

const INDEXED_CANDIDATE_SQL: &str =
    "SELECT file_id FROM file_check WHERE check_id=?1";
const INDEXED_CANDIDATE_COUNT_SQL: &str =
    "SELECT count(*) FROM file_check WHERE check_id=?1";
const INDEXED_FILE_CHECK_SQL: &str =
    "SELECT c.policy_index,c.check_index
       FROM file_check fc JOIN checks c ON c.check_id=fc.check_id
      WHERE fc.file_id=?1";

const FILE_CHECK_SQL: &str =
    "SELECT c.policy_index,c.check_index
       FROM policy_file pf
       JOIN files f ON f.file_id=pf.file_id
       JOIN checks c ON c.policy_id=pf.policy_id
      WHERE pf.file_id=?1 AND (
        EXISTS(
          SELECT 1 FROM check_languages cl
           WHERE cl.check_id=c.check_id AND cl.unrestricted=1
             AND (cl.language='' OR cl.language=f.language)
        ) OR EXISTS(
          SELECT 1 FROM check_kind ck
          JOIN file_kind fk ON fk.file_id=pf.file_id
                           AND fk.language=ck.language
                           AND fk.kind_id=ck.kind_id
         WHERE ck.check_id=c.check_id
        )
      )";

/// Run with:
/// `cargo test --manifest-path Cargo.toml --release spike_sql_file_filter_indexes -- --ignored --nocapture`
#[test]
#[ignore = "measured spike over the tracked repository corpus"]
fn spike_sql_file_filter_indexes() {
    let native = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repository = native.parent().unwrap().to_path_buf();
    let rules = repository.join("policies/executable-policy-dsl/rules");
    let capabilities =
        fs::read_to_string(repository.join("policies/executable-policy-dsl/capabilities.json"))
            .unwrap();

    let mut workspace = PolicyDslWorkspace::new();
    let mut rule_paths = fs::read_dir(rules)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "json"))
        .collect::<Vec<_>>();
    rule_paths.sort();
    for path in &rule_paths {
        let compiled = PolicyDslRuntime::compile(
            fs::read_to_string(path).unwrap(),
            "json".to_owned(),
            capabilities.clone(),
        )
        .unwrap();
        workspace.add_policy(&compiled).unwrap();
    }
    let state = workspace.state.lock().unwrap();
    let policies = &state.policies;

    let mut paths = tracked(&repository);
    paths.sort();
    paths.dedup();

    let parse_started = Instant::now();
    let mut files = Vec::new();
    let mut parse_errors = 0usize;
    let mut source_bytes = 0usize;
    for path in paths {
        let relative = relative_path(&repository, &path);
        let Some(language) = language_for_path(&relative) else {
            continue;
        };
        let source = match fs::read(&path) {
            Ok(source) => source,
            Err(_) => continue,
        };
        source_bytes += source.len();
        let Some(kinds) = parse_kinds(language, &source) else {
            parse_errors += 1;
            continue;
        };
        files.push(IndexedFile {
            id: i64::try_from(files.len() + 1).unwrap(),
            path: relative,
            language: language.to_owned(),
            kinds,
        });
    }
    let parse_ms = parse_started.elapsed().as_secs_f64() * 1_000.0;

    let mut checks = Vec::new();
    let mut check_by_tuple = HashMap::new();
    for (policy_index, policy) in policies.iter().enumerate() {
        for (check_index, check) in policy.checks.iter().enumerate() {
            if !matches!(check.kind, CheckKind::Forbid(Query::Regex { .. } | Query::Ast { .. })) {
                continue;
            }
            let id = i64::try_from(checks.len() + 1).unwrap();
            checks.push(CheckPlan {
                id,
                policy_index,
                check_index,
            });
            check_by_tuple.insert((policy_index, check_index), id);
        }
    }

    let oracle_started = Instant::now();
    let mut oracle_by_check = checks
        .iter()
        .map(|check| (check.id, BTreeSet::new()))
        .collect::<BTreeMap<_, _>>();
    let mut oracle_by_file = HashMap::new();
    let mut candidate_sets = CandidateSets::new();
    for file in &files {
        let candidates = expected_candidates(file, policies, &mut candidate_sets);
        for candidate in &candidates {
            oracle_by_check
                .get_mut(&check_by_tuple[candidate])
                .unwrap()
                .insert(file.id);
        }
        oracle_by_file.insert(file.id, candidates);
    }
    let oracle_ms = oracle_started.elapsed().as_secs_f64() * 1_000.0;
    let candidate_pairs = oracle_by_check.values().map(BTreeSet::len).sum::<usize>();

    let database = PathBuf::from("/tmp/rerail-file-filter-sql-spike.sqlite");
    for suffix in ["", "-wal", "-shm"] {
        let _ = fs::remove_file(format!("{}{suffix}", database.display()));
    }
    let mut connection = Connection::open(&database).unwrap();
    connection
        .execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=NORMAL;
             PRAGMA foreign_keys=ON;
             PRAGMA busy_timeout=250;
             PRAGMA cache_size=-16384;
             PRAGMA journal_size_limit=16777216;
             CREATE TABLE files(file_id INTEGER PRIMARY KEY,path TEXT NOT NULL UNIQUE,language TEXT NOT NULL) STRICT;
             CREATE TABLE policy_file(policy_id INTEGER NOT NULL,file_id INTEGER NOT NULL,PRIMARY KEY(policy_id,file_id)) STRICT, WITHOUT ROWID;
             CREATE INDEX policy_file_by_file ON policy_file(file_id,policy_id);
             CREATE TABLE file_kind(language TEXT NOT NULL,kind_id INTEGER NOT NULL,file_id INTEGER NOT NULL,PRIMARY KEY(language,kind_id,file_id)) STRICT, WITHOUT ROWID;
             CREATE INDEX file_kind_by_file ON file_kind(file_id,language,kind_id);
             CREATE TABLE checks(check_id INTEGER PRIMARY KEY,policy_id INTEGER NOT NULL,policy_index INTEGER NOT NULL,check_index INTEGER NOT NULL) STRICT;
             CREATE INDEX checks_by_policy ON checks(policy_id,check_id);
             CREATE TABLE check_languages(check_id INTEGER NOT NULL,language TEXT NOT NULL,unrestricted INTEGER NOT NULL,PRIMARY KEY(check_id,language)) STRICT, WITHOUT ROWID;
             CREATE TABLE check_kind(check_id INTEGER NOT NULL,language TEXT NOT NULL,kind_id INTEGER NOT NULL,PRIMARY KEY(check_id,language,kind_id)) STRICT, WITHOUT ROWID;
             CREATE TABLE file_check(check_id INTEGER NOT NULL,file_id INTEGER NOT NULL,PRIMARY KEY(check_id,file_id)) STRICT, WITHOUT ROWID;
             CREATE INDEX file_check_by_file ON file_check(file_id,check_id);",
        )
        .unwrap();

    let build_started = Instant::now();
    {
        let transaction = connection.transaction().unwrap();
        for plan in &checks {
            let policy = &policies[plan.policy_index];
            let check = &policy.checks[plan.check_index];
            transaction
                .execute(
                    "INSERT INTO checks VALUES (?1,?2,?3,?4)",
                    params![
                        plan.id,
                        plan.policy_index as i64,
                        plan.policy_index as i64,
                        plan.check_index as i64
                    ],
                )
                .unwrap();
            match &check.kind {
                CheckKind::Forbid(Query::Regex { .. }) => {
                    transaction
                        .execute(
                            "INSERT INTO check_languages VALUES (?1,'',1)",
                            [plan.id],
                        )
                        .unwrap();
                }
                CheckKind::Forbid(Query::Ast { matchers, .. }) => {
                    for matcher in matchers {
                        let language = match matcher.language {
                            GrammarDialect::Python => "python",
                            GrammarDialect::TypeScript => "typescript",
                            GrammarDialect::Tsx => "tsx",
                            _ => continue,
                        };
                        let roots = matcher.anchors.root_kinds();
                        transaction
                            .execute(
                                "INSERT INTO check_languages VALUES (?1,?2,?3)",
                                params![plan.id, language, i64::from(roots.is_none())],
                            )
                            .unwrap();
                        if let Some(roots) = roots {
                            for kind in roots {
                                transaction
                                    .execute(
                                        "INSERT INTO check_kind VALUES (?1,?2,?3)",
                                        params![plan.id, language, i64::from(*kind)],
                                    )
                                    .unwrap();
                            }
                        }
                    }
                }
                _ => unreachable!(),
            }
        }
        for file in &files {
            transaction
                .execute(
                    "INSERT INTO files VALUES (?1,?2,?3)",
                    params![file.id, file.path, file.language],
                )
                .unwrap();
            for kind in &file.kinds {
                transaction
                    .execute(
                        "INSERT INTO file_kind VALUES (?1,?2,?3)",
                        params![file.language, i64::from(*kind), file.id],
                    )
                    .unwrap();
            }
            for (policy_index, policy) in policies.iter().enumerate() {
                if policy.scope.matches(&file.path) {
                    transaction
                        .execute(
                            "INSERT INTO policy_file VALUES (?1,?2)",
                            params![policy_index as i64, file.id],
                        )
                        .unwrap();
                }
            }
        }
        transaction.commit().unwrap();
    }
    let base_build_ms = build_started.elapsed().as_secs_f64() * 1_000.0;
    let candidate_index_started = Instant::now();
    connection
        .execute_batch(
            "INSERT OR IGNORE INTO file_check(check_id,file_id)
             SELECT cl.check_id,pf.file_id
               FROM check_languages cl
               JOIN checks c ON c.check_id=cl.check_id
               JOIN policy_file pf ON pf.policy_id=c.policy_id
               JOIN files f ON f.file_id=pf.file_id
              WHERE cl.unrestricted=1 AND (cl.language='' OR cl.language=f.language)
             UNION
             SELECT ck.check_id,fk.file_id
               FROM check_kind ck
               JOIN file_kind fk ON fk.language=ck.language AND fk.kind_id=ck.kind_id
               JOIN checks c ON c.check_id=ck.check_id
               JOIN policy_file pf ON pf.policy_id=c.policy_id AND pf.file_id=fk.file_id;",
        )
        .unwrap();
    let candidate_index_ms = candidate_index_started.elapsed().as_secs_f64() * 1_000.0;
    connection.execute_batch("ANALYZE;").unwrap();
    let build_ms = build_started.elapsed().as_secs_f64() * 1_000.0;
    connection
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
        .unwrap();

    let explain = |sql: &str, parameter: i64| -> Vec<String> {
        let mut statement = connection
            .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
            .unwrap();
        statement
            .query_map([parameter], |row| row.get::<_, String>(3))
            .unwrap()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap()
    };
    let anchored = checks
        .iter()
        .find(|plan| {
            matches!(
                policies[plan.policy_index].checks[plan.check_index].kind,
                CheckKind::Forbid(Query::Ast { .. })
            )
        })
        .map(|plan| plan.id)
        .unwrap_or(1);
    let candidate_plan = explain(CANDIDATE_SQL, anchored);
    let file_plan = explain(FILE_CHECK_SQL, files[0].id);
    let indexed_candidate_plan = explain(INDEXED_CANDIDATE_SQL, anchored);
    let indexed_file_plan = explain(INDEXED_FILE_CHECK_SQL, files[0].id);

    let mut count_statement = connection.prepare_cached(CANDIDATE_COUNT_SQL).unwrap();
    for check in &checks {
        let count: i64 = count_statement.query_row([check.id], |row| row.get(0)).unwrap();
        assert_eq!(usize::try_from(count).unwrap(), oracle_by_check[&check.id].len());
    }

    let mut count_pass_samples = Vec::new();
    let mut count_query_samples = Vec::new();
    for _ in 0..9 {
        let pass_started = Instant::now();
        let mut total = 0_i64;
        for check in &checks {
            let started = Instant::now();
            total += count_statement.query_row([check.id], |row| row.get::<_, i64>(0)).unwrap();
            count_query_samples.push(started.elapsed().as_nanos());
        }
        assert_eq!(usize::try_from(total).unwrap(), candidate_pairs);
        count_pass_samples.push(pass_started.elapsed().as_micros());
    }

    let mut indexed_count_statement = connection
        .prepare_cached(INDEXED_CANDIDATE_COUNT_SQL)
        .unwrap();
    let mut indexed_count_pass_samples = Vec::new();
    let mut indexed_count_query_samples = Vec::new();
    for _ in 0..9 {
        let pass_started = Instant::now();
        let mut total = 0_i64;
        for check in &checks {
            let started = Instant::now();
            total += indexed_count_statement
                .query_row([check.id], |row| row.get::<_, i64>(0))
                .unwrap();
            indexed_count_query_samples.push(started.elapsed().as_nanos());
        }
        assert_eq!(usize::try_from(total).unwrap(), candidate_pairs);
        indexed_count_pass_samples.push(pass_started.elapsed().as_micros());
    }

    let mut candidate_statement = connection.prepare_cached(CANDIDATE_SQL).unwrap();
    let mut retrieval_pass_samples = Vec::new();
    let mut retrieval_query_samples = Vec::new();
    let mut expected_digest = None;
    for _ in 0..7 {
        let pass_started = Instant::now();
        let mut hasher = DefaultHasher::new();
        let mut total = 0usize;
        for check in &checks {
            let started = Instant::now();
            let mut rows = candidate_statement.query([check.id]).unwrap();
            let mut actual = BTreeSet::new();
            while let Some(row) = rows.next().unwrap() {
                let file_id = row.get::<_, i64>(0).unwrap();
                file_id.hash(&mut hasher);
                actual.insert(file_id);
                total += 1;
            }
            retrieval_query_samples.push(started.elapsed().as_nanos());
            assert_eq!(actual, oracle_by_check[&check.id]);
        }
        assert_eq!(total, candidate_pairs);
        let digest = hasher.finish();
        assert!(expected_digest.is_none_or(|expected| expected == digest));
        expected_digest = Some(digest);
        retrieval_pass_samples.push(pass_started.elapsed().as_micros());
    }

    let mut indexed_candidate_statement = connection
        .prepare_cached(INDEXED_CANDIDATE_SQL)
        .unwrap();
    let mut indexed_retrieval_pass_samples = Vec::new();
    let mut indexed_retrieval_query_samples = Vec::new();
    for _ in 0..7 {
        let pass_started = Instant::now();
        let mut total = 0usize;
        for check in &checks {
            let started = Instant::now();
            let mut rows = indexed_candidate_statement.query([check.id]).unwrap();
            let mut actual = BTreeSet::new();
            while let Some(row) = rows.next().unwrap() {
                actual.insert(row.get::<_, i64>(0).unwrap());
                total += 1;
            }
            indexed_retrieval_query_samples.push(started.elapsed().as_nanos());
            assert_eq!(actual, oracle_by_check[&check.id]);
        }
        assert_eq!(total, candidate_pairs);
        indexed_retrieval_pass_samples.push(pass_started.elapsed().as_micros());
    }

    let mut file_statement = connection.prepare_cached(FILE_CHECK_SQL).unwrap();
    let mut point_samples = Vec::with_capacity(10_000);
    for index in 0..10_000 {
        let file = &files[index % files.len()];
        let started = Instant::now();
        let actual = file_statement
            .query_map([file.id], |row| {
                Ok((
                    usize::try_from(row.get::<_, i64>(0)?).unwrap(),
                    usize::try_from(row.get::<_, i64>(1)?).unwrap(),
                ))
            })
            .unwrap()
            .collect::<std::result::Result<BTreeSet<_>, _>>()
            .unwrap();
        point_samples.push(started.elapsed().as_nanos());
        assert_eq!(actual, oracle_by_file[&file.id]);
    }
    let mut indexed_file_statement = connection.prepare_cached(INDEXED_FILE_CHECK_SQL).unwrap();
    let mut indexed_point_samples = Vec::with_capacity(10_000);
    for index in 0..10_000 {
        let file = &files[index % files.len()];
        let started = Instant::now();
        let actual = indexed_file_statement
            .query_map([file.id], |row| {
                Ok((
                    usize::try_from(row.get::<_, i64>(0)?).unwrap(),
                    usize::try_from(row.get::<_, i64>(1)?).unwrap(),
                ))
            })
            .unwrap()
            .collect::<std::result::Result<BTreeSet<_>, _>>()
            .unwrap();
        indexed_point_samples.push(started.elapsed().as_nanos());
        assert_eq!(actual, oracle_by_file[&file.id]);
    }
    let mut rust_point_samples = Vec::with_capacity(10_000);
    let mut rust_point_sets = CandidateSets::new();
    for index in 0..10_000 {
        let file = &files[index % files.len()];
        let started = Instant::now();
        let actual = expected_candidates(file, policies, &mut rust_point_sets);
        rust_point_samples.push(started.elapsed().as_nanos());
        assert_eq!(actual, oracle_by_file[&file.id]);
    }

    drop(indexed_file_statement);
    drop(file_statement);
    drop(indexed_candidate_statement);
    drop(candidate_statement);
    drop(indexed_count_statement);
    drop(count_statement);

    connection
        .execute_batch(
            "CREATE TEMP TABLE changed_policy(policy_id INTEGER PRIMARY KEY) STRICT;
             CREATE TEMP TABLE changed_kind(language TEXT NOT NULL,kind_id INTEGER NOT NULL,PRIMARY KEY(language,kind_id)) STRICT, WITHOUT ROWID;",
        )
        .unwrap();
    let update_sql =
        "INSERT OR IGNORE INTO file_check(check_id,file_id)
         SELECT cl.check_id,?1
           FROM check_languages cl
           JOIN checks c ON c.check_id=cl.check_id
           JOIN changed_policy cp ON cp.policy_id=c.policy_id
          WHERE cl.unrestricted=1 AND (cl.language='' OR cl.language=?2)
         UNION
         SELECT ck.check_id,?1
           FROM check_kind ck
           JOIN checks c ON c.check_id=ck.check_id
           JOIN changed_policy cp ON cp.policy_id=c.policy_id
           JOIN changed_kind cg ON cg.language=ck.language AND cg.kind_id=ck.kind_id";
    let mut update_samples = Vec::with_capacity(1_000);
    for index in 0..1_000 {
        let file = &files[index % files.len()];
        let started = Instant::now();
        let transaction = connection.transaction().unwrap();
        transaction
            .execute("DELETE FROM file_check WHERE file_id=?1", [file.id])
            .unwrap();
        transaction.execute("DELETE FROM changed_policy", []).unwrap();
        transaction.execute("DELETE FROM changed_kind", []).unwrap();
        for (policy_index, policy) in policies.iter().enumerate() {
            if policy.scope.matches(&file.path) {
                transaction
                    .execute(
                        "INSERT INTO changed_policy VALUES (?1)",
                        [policy_index as i64],
                    )
                    .unwrap();
            }
        }
        for kind in &file.kinds {
            transaction
                .execute(
                    "INSERT INTO changed_kind VALUES (?1,?2)",
                    params![file.language, i64::from(*kind)],
                )
                .unwrap();
        }
        transaction
            .execute(update_sql, params![file.id, file.language])
            .unwrap();
        let count: i64 = transaction
            .query_row(
                "SELECT count(*) FROM file_check WHERE file_id=?1",
                [file.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(usize::try_from(count).unwrap(), oracle_by_file[&file.id].len());
        transaction.commit().unwrap();
        update_samples.push(started.elapsed().as_nanos());
    }

    connection
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
        .unwrap();
    let database_bytes = fs::metadata(&database).unwrap().len();
    let mut dbstat = connection
        .prepare("SELECT name,sum(pgsize) FROM dbstat GROUP BY name")
        .unwrap();
    let object_bytes = dbstat
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)))
        .unwrap()
        .collect::<std::result::Result<BTreeMap<_, _>, _>>()
        .unwrap();
    drop(dbstat);
    let file_check_bytes = object_bytes.get("file_check").copied().unwrap_or(0);
    let file_check_reverse_bytes = object_bytes
        .get("file_check_by_file")
        .copied()
        .unwrap_or(0);
    let wal_bytes = fs::metadata(format!("{}-wal", database.display()))
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    let rows = |table: &str| -> i64 {
        connection
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| row.get(0))
            .unwrap()
    };
    let file_kind_rows = rows("file_kind");
    let policy_file_rows = rows("policy_file");
    let check_kind_rows = rows("check_kind");
    let file_check_rows = rows("file_check");
    connection
        .execute_batch(
            "DROP TABLE policy_file;
             DROP TABLE file_kind;
             DROP TABLE files;
             VACUUM;
             PRAGMA wal_checkpoint(TRUNCATE);",
        )
        .unwrap();
    let retained_database_bytes = fs::metadata(&database).unwrap().len();
    let retained_pairs: i64 = connection
        .query_row("SELECT count(*) FROM file_check", [], |row| row.get(0))
        .unwrap();
    assert_eq!(usize::try_from(retained_pairs).unwrap(), candidate_pairs);

    let timings = json!({
        "parseMilliseconds": parse_ms,
        "rustOracleMilliseconds": oracle_ms,
        "sqlBaseIndexBuildMilliseconds": base_build_ms,
        "sqlCandidateIndexBuildMilliseconds": candidate_index_ms,
        "sqlTotalIndexBuildMilliseconds": build_ms,
        "dynamic": {
            "countAllChecksP50Microseconds": percentile(&mut count_pass_samples, 1, 2),
            "countQueryP50Nanoseconds": percentile(&mut count_query_samples, 1, 2),
            "countQueryP95Nanoseconds": percentile(&mut count_query_samples, 95, 100),
            "retrieveAllCandidatesP50Microseconds": percentile(&mut retrieval_pass_samples, 1, 2),
            "retrieveQueryP50Nanoseconds": percentile(&mut retrieval_query_samples, 1, 2),
            "retrieveQueryP95Nanoseconds": percentile(&mut retrieval_query_samples, 95, 100),
            "changedFileP50Nanoseconds": percentile(&mut point_samples, 1, 2),
            "changedFileP95Nanoseconds": percentile(&mut point_samples, 95, 100),
        },
        "indexed": {
            "countAllChecksP50Microseconds": percentile(&mut indexed_count_pass_samples, 1, 2),
            "countQueryP50Nanoseconds": percentile(&mut indexed_count_query_samples, 1, 2),
            "countQueryP95Nanoseconds": percentile(&mut indexed_count_query_samples, 95, 100),
            "retrieveAllCandidatesP50Microseconds": percentile(&mut indexed_retrieval_pass_samples, 1, 2),
            "retrieveQueryP50Nanoseconds": percentile(&mut indexed_retrieval_query_samples, 1, 2),
            "retrieveQueryP95Nanoseconds": percentile(&mut indexed_retrieval_query_samples, 95, 100),
            "changedFileP50Nanoseconds": percentile(&mut indexed_point_samples, 1, 2),
            "changedFileP95Nanoseconds": percentile(&mut indexed_point_samples, 95, 100),
            "updateP50Nanoseconds": percentile(&mut update_samples, 1, 2),
            "updateP95Nanoseconds": percentile(&mut update_samples, 95, 100),
        },
        "rustChangedFileP50Nanoseconds": percentile(&mut rust_point_samples, 1, 2),
        "rustChangedFileP95Nanoseconds": percentile(&mut rust_point_samples, 95, 100),
    });
    let storage = json!({
        "expandedBuildDatabaseBytes": database_bytes,
        "retainedDatabaseBytes": retained_database_bytes,
        "baseIndexBytes": i64::try_from(database_bytes).unwrap() - file_check_bytes - file_check_reverse_bytes,
        "fileCheckBytes": file_check_bytes,
        "fileCheckReverseBytes": file_check_reverse_bytes,
        "walBytesAfterCheckpoint": wal_bytes,
    });
    let plans = json!({
        "dynamicCandidate": candidate_plan,
        "dynamicChangedFile": file_plan,
        "indexedCandidate": indexed_candidate_plan,
        "indexedChangedFile": indexed_file_plan,
    });
    let report = json!({
        "files": files.len(),
        "parseErrorsExcluded": parse_errors,
        "sourceBytes": source_bytes,
        "policies": policies.len(),
        "checks": checks.len(),
        "candidatePairs": candidate_pairs,
        "fileKindRows": file_kind_rows,
        "policyFileRows": policy_file_rows,
        "checkKindRows": check_kind_rows,
        "fileCheckRows": file_check_rows,
        "timings": timings,
        "storage": storage,
        "plans": plans,
        "parity": true,
        "databasePath": database,
    });
    println!("FILE_FILTER_SQL_SPIKE={report}");
}
