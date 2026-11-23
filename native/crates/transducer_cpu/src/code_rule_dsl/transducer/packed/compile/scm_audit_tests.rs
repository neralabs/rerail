//! The pinned Python SCM patterns' audit (`scm`'s tests): every pattern through the query lowerer, then the compiled
//! CPU (`native_cpu`), which this crate (rerail-transducer-cpu) has and the query front end's (the base's) does not.
use super::scm::*;
use super::Program;
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;

#[test]
#[ignore = "all pinned Python SCM patterns through the Program lowerer"]
fn lowerer_audits_every_pinned_python_pattern_without_approximation() {
    use ast_grep_language::LanguageExt;
    let root = std::env::var("RERAIL_PACKED_PYTHON_SCM_QUERY_DIR")
        .expect("RERAIL_PACKED_PYTHON_SCM_QUERY_DIR must name the original Python SCM directory");
    let mut paths = std::fs::read_dir(&root)
        .unwrap_or_else(|error| {
            panic!("cannot read pinned Python SCM input at {root}: {error}")
        })
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "scm"))
        .collect::<Vec<_>>();
    paths.sort();
    let language = crate::code_rule_dsl::language::GrammarDialect::Python.get_ts_language();
    let LoweringAudit {
        compiled,
        incompatible,
        rejected,
    } = audit_pinned_python_lowering(&paths, &language);
    let incompatible_count = incompatible.values().sum::<usize>();
    let rejected_count = rejected.values().sum::<usize>();
    assert_eq!(compiled.len(), 335);
    assert_eq!(incompatible_count, 0);
    assert_eq!(rejected_count, 0, "compatible patterns must all lower");
    let ProgramCounts {
        opcode_counts,
        value_program_counts,
        capture_program_counts,
        output_instruction_counts,
    } = count_program_parts(&compiled);

    let native_cpu_millis = native_cpu_compile_millis(&compiled);
    let metal_compile_millis: Option<f64> = None;
    println!(
        "RERAIL_PACKED_PYTHON_SCM_COMPILATION {{\"programs\":{},\"nativeIrLowered\":{},\"callbackFreeCpuJitCompiled\":{},\"callbackFreeCpuJitCompileMillis\":{native_cpu_millis},\"metalJitCompiled\":{},\"metalJitCompileMillis\":{},\"structuralOpcodeCounts\":{},\"valueProgramCounts\":{},\"captureProgramCounts\":{},\"outputInstructionCounts\":{}}}",
        compiled.len(),
        compiled.len(),
        compiled.len(),
        if metal_compile_millis.is_some() {
            compiled.len()
        } else {
            0
        },
        serde_json::to_string(&metal_compile_millis).unwrap(),
        serde_json::to_string(&opcode_counts).unwrap(),
        serde_json::to_string(&value_program_counts).unwrap(),
        serde_json::to_string(&capture_program_counts).unwrap(),
        serde_json::to_string(&output_instruction_counts).unwrap(),
    );
    println!(
        "RERAIL_PACKED_PYTHON_SCM_LOWERING {{\"compiled\":{},\"grammarIncompatible\":{incompatible_count},\"loweringRejected\":{rejected_count},\"incompatibilities\":{},\"rejections\":{}}}",
        compiled.len(),
        serde_json::to_string(&incompatible).unwrap(),
        serde_json::to_string(&rejected).unwrap()
    );
}

/// The pinned patterns that lowered, and the rest counted by why they did not.
struct LoweringAudit {
    compiled: Vec<Program>,
    incompatible: BTreeMap<String, usize>,
    rejected: BTreeMap<String, usize>,
}

impl LoweringAudit {
    fn lower_patterns(&mut self, source: &str, language: &tree_sitter::Language) {
        for pattern in parse(source).unwrap() {
            if let Err(error) = tree_sitter::Query::new(language, pattern.source()) {
                *self.incompatible.entry(error.message.clone()).or_default() += 1;
                continue;
            }
            match compile(&pattern, language) {
                Ok(program) => self.compiled.push(program),
                Err(error) => {
                    *self.rejected.entry(format!("{error:?}")).or_default() += 1;
                }
            }
        }
    }
}

fn pinned_nvim_digests() -> BTreeMap<&'static str, &'static str> {
    BTreeMap::from([
        (
            "nvim-treesitter__python__folds.scm",
            "7ea0eb74ffee66415a3b6fd51b47873b411bc80d8eb4dfb7aa6ffa0cf7fad1f3",
        ),
        (
            "nvim-treesitter__python__highlights.scm",
            "ab8b66cbe6a7382e0ed7693c194101f1900c39f9ec0085a1749a8817e3e6895c",
        ),
        (
            "nvim-treesitter__python__indents.scm",
            "50781d783a18a566581d8280c53e150454d1e3523cfcd4f1401bb3a228a98e80",
        ),
        (
            "nvim-treesitter__python__injections.scm",
            "8558299709200b21d496890e4799153dc069bf302ce1545d23f70a0869c8b65d",
        ),
        (
            "nvim-treesitter__python__locals.scm",
            "370635358588e6d66e6065dd9fb1e21d864e8eb125abbf4728e573bc06ef4921",
        ),
    ])
}

fn assert_pinned_nvim_digest(
    pinned_nvim: &BTreeMap<&str, &str>,
    path: &std::path::Path,
    source: &str,
) {
    if let Some(expected) = path
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| pinned_nvim.get(name))
    {
        assert_eq!(
            crate::code_rule_dsl::prof::lower_hex(&Sha256::digest(source.as_bytes())),
            *expected
        );
    }
}

fn audit_pinned_python_lowering(
    paths: &[std::path::PathBuf],
    language: &tree_sitter::Language,
) -> LoweringAudit {
    let pinned_nvim = pinned_nvim_digests();
    let mut audit = LoweringAudit {
        compiled: Vec::new(),
        incompatible: BTreeMap::new(),
        rejected: BTreeMap::new(),
    };
    for path in paths {
        let source = std::fs::read_to_string(path).unwrap();
        assert_pinned_nvim_digest(&pinned_nvim, path, &source);
        audit.lower_patterns(&source, language);
    }
    audit
}

#[derive(Default)]
struct ProgramCounts {
    opcode_counts: BTreeMap<&'static str, usize>,
    value_program_counts: BTreeMap<&'static str, usize>,
    capture_program_counts: BTreeMap<&'static str, usize>,
    output_instruction_counts: BTreeMap<&'static str, usize>,
}

fn count_program_parts(compiled: &[Program]) -> ProgramCounts {
    let mut counts = ProgramCounts::default();
    for program in compiled {
        for value in &program.value_programs {
            *counts
                .value_program_counts
                .entry(value_program_name(value))
                .or_default() += 1;
        }
        for capture in &program.capture_programs {
            *counts
                .capture_program_counts
                .entry(capture_program_name(capture))
                .or_default() += 1;
        }
        for output in &program.output_program.instructions {
            *counts
                .output_instruction_counts
                .entry(output_instruction_name(output))
                .or_default() += 1;
        }
        for word in &program.words {
            *counts.opcode_counts.entry(opcode_name(*word)).or_default() += 1;
        }
    }
    counts
}

fn value_program_name(value: &super::value::ValueProgram) -> &'static str {
    match value {
        super::value::ValueProgram::Equal { .. } => "equal",
        super::value::ValueProgram::Prefix { .. } => "prefix",
        super::value::ValueProgram::Regex(_) => "regex",
        super::value::ValueProgram::Lua(_) => "lua",
    }
}

fn capture_program_name(capture: &super::value::CaptureProgram) -> &'static str {
    match capture {
        super::value::CaptureProgram::Equal { .. } => "equal",
        super::value::CaptureProgram::Text { .. } => "text",
        super::value::CaptureProgram::AnyOf { .. } => "any_of",
        super::value::CaptureProgram::Kind { .. } => "kind",
        super::value::CaptureProgram::Contains { .. } => "contains",
        super::value::CaptureProgram::Relation { .. } => "relation",
        super::value::CaptureProgram::SameLine { .. } => "same_line",
    }
}

fn output_instruction_name(output: &super::output::OutputInstruction) -> &'static str {
    match output {
        super::output::OutputInstruction::MatchProperty { .. } => "match_property",
        super::output::OutputInstruction::CaptureProperty { .. } => "capture_property",
        super::output::OutputInstruction::Offset { .. } => "offset",
        super::output::OutputInstruction::Strip { .. } => "strip",
        super::output::OutputInstruction::SelectAdjacent { .. } => "select_adjacent",
        super::output::OutputInstruction::Downcase { .. } => "downcase",
        super::output::OutputInstruction::LanguageFromMime { .. } => {
            "language_from_mime"
        }
    }
}

fn opcode_name(word: u32) -> &'static str {
    match super::split(word).0 {
        super::STOP => "stop",
        super::KIND => "kind",
        super::FIELD => "field",
        super::FLAGS => "flags",
        super::TEXT => "text",
        super::SIZE => "size",
        super::ASSERT => "assert",
        super::FIRST => "first",
        super::LAST => "last",
        super::ABSENT_FIELD => "absent_field",
        super::SUPERTYPE => "supertype",
        super::CHILD => "child",
        super::DESCEND => "descend",
        super::NEXT => "next",
        super::FOLLOWING => "following",
        super::PARENT => "parent",
        super::SAVE_CURSOR => "save_cursor",
        super::RESTORE_CURSOR => "restore_cursor",
        super::CAPTURE => "capture",
        super::REPEAT_BEGIN => "repeat_begin",
        super::REPEAT_BUMP => "repeat_bump",
        super::REPEAT_END => "repeat_end",
        super::FORK => "fork",
        super::GOTO => "goto",
        super::EMIT => "emit",
        opcode => panic!("unrecognized compiled opcode {opcode}"),
    }
}

/// Lowers every Program to the native IR, then times the callback-free CPU JIT over them.
fn native_cpu_compile_millis(compiled: &[Program]) -> f64 {
    let native_modules = compiled
        .iter()
        .map(|program| {
            super::native_ir::lower(program)
                .expect("the native IR must lower every pinned Program")
        })
        .collect::<Vec<_>>();
    let native_cpu_started = std::time::Instant::now();
    for native in &native_modules {
        super::native_cpu::compile_straight_line(native)
            .expect("the callback-free CPU JIT must compile every pinned Program");
    }
    native_cpu_started.elapsed().as_secs_f64() * 1_000.0
}
