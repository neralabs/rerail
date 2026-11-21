use super::*;

/// R1–R3 of decision 0436 on `src`: the tape is a v9np image whose root spans the file (R1); its
/// leaves are the lexer's tokens, each once, in order (R2); its directives, their bytes and
/// their groups are the lean scan's (R3).
fn check(src: &[u8], opts: Options) -> Result<(), String> {
    let image = image(src, opts);
    let v = View::new(&image)?;
    if (v.key(0) & 0x3ff) as u16 != PP || v.start(0) != 0 || v.end(0) as usize != src.len() || v.size(0) != v.len() {
        return Err("the root is not the file's".into());
    }
    let want: Vec<_> = tokens(src, opts).iter().map(|t| (t.kind, t.start, t.end)).collect();
    let got = leaves(&image)?;
    if got != want {
        let k = got.iter().zip(&want).position(|(a, b)| a != b).unwrap_or(got.len().min(want.len()));
        return Err(format!("leaf {k}: {:?} for the lexer's {:?}", got.get(k), want.get(k)));
    }
    let d = directives::scan(src, opts);
    let want: Vec<_> = d.all.iter().map(|x| {
        let (start, end) = d.file(x.span);
        Row { kind: x.kind.name(), start, end, parent: x.parent }
    }).collect();
    let got = rows(&image)?;
    if got != want {
        let k = got.iter().zip(&want).position(|(a, b)| a != b).unwrap_or(got.len().min(want.len()));
        return Err(format!("directive {k}: {:?} for the scan's {:?}", got.get(k), want.get(k)));
    }
    sixteen(src, opts, &v)
}

/// The format 16 tape (0468 R1) has format 15's nodes, each its kind, field, bytes and subtree,
/// and its newline index gives each node's line.
fn sixteen(src: &[u8], opts: Options, v: &View<'_>) -> Result<(), String> {
    use rerail_tape_format::tape::Tape;
    let t = image_16(src, opts);
    let r = t.reader(lang())?;
    if r.len() != v.len() {
        return Err(format!("{} nodes in format 16, {} in 15", r.len(), v.len()));
    }
    for i in 0..r.len() {
        let e = r.table().entry(r.code(i));
        let key = v.key(i);
        let want = ((key & 0x3ff) as u16, (key >> rerail_tape_format::KIND_BITS & 0x3f) as u16, u64::from(v.start(i)), u64::from(v.end(i)), i + v.size(i));
        let got = (e.kind, e.field, r.start(i), r.end(i), r.subtree_end(i));
        if got != want {
            return Err(format!("node {i}: {got:?} in format 16, {want:?} in 15"));
        }
        let line = src[..v.start(i) as usize].iter().filter(|&&b| b == b'\n').count() as u32;
        if r.line(v.start(i)) != line {
            return Err(format!("node {i}: line {} by the index, {line} counted", r.line(v.start(i))));
        }
    }
    Ok(())
}

fn c() -> Options {
    Options::c(23, true)
}

/// The kinds' ids are the constants'.
#[test]
fn the_pp_tape_kinds_are_numbered_as_named() {
    for (id, name) in [(PP, "pp"), (CODE, "code"), (GROUP, "group"), (DIRECTIVES, "null"), (IDENTIFIER, "identifier"), (OTHER, "other"), (PARAMS, "params"), (MARKER_REST, "marker_rest")] {
        assert_eq!(KINDS[id as usize], name);
    }
    assert_eq!(KINDS.len(), MARKER_REST as usize + 1);
    for k in ["define", "include", "if", "endif", "line", "unassert"] {
        assert_eq!(KINDS[kind_id(Kind::of(k.as_bytes())) as usize], k);
    }
    for k in [Kind::Null, Kind::Unknown, Kind::LineMarker] {
        assert_eq!(KINDS[kind_id(k) as usize], k.name());
    }
}

/// R1 (0468): the tape is a format 16 image with its newline index, the nodes format 15's
/// (`image`, the Bitlex grammar's comparison), its root the file.
#[test]
fn the_pp_tape_is_a_format_16_image() {
    let src = b"#include <a.h>\n#define F(a, ...) a + 1\nint x = F(3);\n// a\\\n b\n\n#if X\n  y\n#endif\n";
    assert_eq!(&image(src, c())[..8], rerail_tape_format::MAGIC);
    assert!(image_16(src, c()).placement().newlines > 0, "the newline index");
    check(src, c()).unwrap();
}

/// R2: every token of the file is a leaf once, in directives, in skipped groups and in text.
#[test]
fn the_pp_tape_has_the_lexers_tokens() {
    for src in [
        &b"a b\\\nc /* x\n y */ d\n# define X(a) #a ## b\n"[..],
        b"#if 0\n don't \n#endif\n<: :> %: x\n",
        b"#error don't do it\n'a' \"s\" 1.2e+3 @\n",
        b"",
        b"// only a comment\n",
    ] {
        check(src, c()).unwrap_or_else(|e| panic!("{:?}: {e}", String::from_utf8_lossy(src)));
    }
}

/// R3: the directives, their groups and their operands' fields.
#[test]
fn the_pp_tape_gives_mode_a_rows() {
    let src = b"#ifndef G\n#define G\n#if A\n#elif B\nx\n#else\n#define F(a, b...) a\n#endif\n#endif\n#else\n#pragma once\n#line 3 \"f\"\n# 4 \"g\" 1\n#undef G\n";
    if let Err(e) = check(src, c()) {
        eprintln!("CHECK: {e}");
        panic!("{e}");
    }
    let image = image(src, c());
    let v = View::new(&image).unwrap();
    let named = |kind: u16, f: &str| (0..v.len()).any(|i| v.key(i) & 0xffff == key(kind, field(f), 0) & 0xffff);
    assert!(named(IDENTIFIER, "name") && named(IDENTIFIER, "param") && named(PUNCT, "variadic") && named(PARAMS, "params"));
    assert!(named(OPERANDS, "expr") && named(OPERANDS, "body") && named(PRAGMA_TEXT, "text") && named(IDENTIFIER, "first"));
    assert!(named(LINE_REST, "rest") && named(MARKER_REST, "rest") && named(NUMBER, "n") && named(LITERAL, "file"));
    // Two groups, the inner inside the outer.
    assert_eq!((0..v.len()).filter(|&i| (v.key(i) & 0x3ff) as u16 == GROUP).count(), 2);
}

/// R1–R3 over every file of the C and C++ corpora (where they are here).
#[test]
#[ignore = "the corpora: about a minute"]
fn the_corpus_pp_tapes_lean() {
    let Some(home) = std::env::var_os("HOME") else { return };
    let root = std::path::Path::new(&home).join("orca/rerail-shared/lean-corpus");
    let (mut files, mut failed) = (0, Vec::new());
    for (dir, opts) in [("c", Options::c(23, true)), ("cpp", Options::cpp(26, true))] {
        let Ok(rd) = std::fs::read_dir(root.join(dir)) else { continue };
        for e in rd.flatten() {
            let Ok(src) = std::fs::read(e.path()) else { continue };
            files += 1;
            if let Err(err) = check(&src, opts) {
                failed.push(format!("{}: {err}", e.path().display()));
            }
        }
    }
    eprintln!("{files} files, {} failed", failed.len());
    for f in failed.iter().take(20) {
        eprintln!("  {f}");
    }
    assert!(failed.is_empty());
}
