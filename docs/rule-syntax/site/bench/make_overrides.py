"""Scratch: build the python-main overrides from a fresh translation in <gen dir> (hand edits as patches)."""
import sys, re, glob, os
gen, o = sys.argv[1:]
os.makedirs(o, exist_ok=True)
STMTS = ["expression_statement","return_statement","delete_statement","raise_statement","pass_statement",
         "break_statement","continue_statement","global_statement","nonlocal_statement","exec_statement",
         "print_statement","assert_statement","import_statement","import_from_statement","future_import_statement",
         "type_alias_statement","if_statement","for_statement","while_statement","try_statement","with_statement",
         "function_definition","class_definition","decorated_definition","match_statement"]
kinds = lambda v: "(" + " ; ".join(f'kind({v}, "{k}")' for k in STMTS) + ")"
KIND_NOTE = ("// The reader matches a regex with a capture only over a node whose kind the clause names, so the\n"
             "// statement's possible kinds (tree-sitter-python's statements) are named as a disjunction.\n")
def rd(rid): return open(f"{gen}/{rid}.rr").read()
def wr(rid, why, t): open(f"{o}/{rid}.rr", "w").write(why + t)

# B110, B112: pattern {context, selector}
for rid, stmt in (("bandit-b110-try-except-pass", "pass"), ("bandit-b112-try-except-continue", "continue")):
    t = rd(rid)
    t2 = re.sub(r"n ~ `try:\\n    work\(\)\\nexcept:\\n    " + stmt + r"` at \"except_clause\"",
                'count : { child(n, c0, _), !kind(c0, "block"), !kind(c0, "comment") } = 0', t)
    assert t2 != t, rid
    wr(rid, "// OVERRIDE (hand-written, not generated): main's `pattern: {context, selector}` (a pattern parsed in a\n"
            "// context, matched at the selector's node) has no .rr form; the reader refuses `~ `...` at \"kind\"`.\n"
            f"// The context `try: work()\\nexcept: {stmt}` selects a bare `except:` clause, so that alternative is\n"
            "// written as an except_clause with no child but its block. The rest is the generated translation.\n", t2)

# RET505-RET508: the else/elif keyword token
for f in sorted(glob.glob(f"{gen}/ruff-ret50[5-8]-*.rr")):
    rid = os.path.basename(f)[:-3]
    t = open(f).read()
    code = re.search(r"^(RET50\d)\(n, BRANCH\) :- ", t, re.M).group(1)
    old = 'match("(?<BRANCH>elif|else)", n), child(p1, n, _),\n    (kind(p1, "elif_clause") ; kind(p1, "else_clause")),'
    assert old in t, f
    i = t.index(old); j = t.index(".\n", i)
    rest = re.sub(r"\bp1\b", "n", t[i + len(old):j])
    new = ('(kind(n, "elif_clause") ; kind(n, "else_clause")),\n    child(n, c0, 0), structural_before(c0, g0), '
           'match("(?s)\\\\s*(?<BRANCH>elif|else)(?:\\\\W.*)?", g0),' + rest)
    t = t[:i] + new + t[j:]
    t = t.replace(f".violation {code}(at: node, branch: loc)", f".violation {code}(at: node, branch: symbol)")
    wr(rid, "// OVERRIDE (hand-written, not generated): main's rule matches the `else` / `elif` keyword token itself\n"
            "// (`regex: ^(elif|else)$` inside an else_clause or elif_clause) and captures it as BRANCH. The tape leaves\n"
            "// that keyword out (the clause implies it), and the reader matches a capture regex only over a node of a\n"
            "// named kind. So the finding is at the clause, which starts with the keyword (same line), and BRANCH is\n"
            "// read from the source before the clause's first stored child. The rest is the generated translation.\n", t)

# COM818
rid = "ruff-com818-trailing-comma-on-bare-tuple"
t = rd(rid)
old = 'COM818(COMMA, n) :- match(".*(?P<COMMA>,)", n), !kind(n, "comment"), child(p1, n, _),'
assert old in t
t = t.replace(old, 'COM818(COMMA, n) :- ' + kinds("n") + ',\n    match(".*(?P<COMMA>,)", n), !kind(n, "comment"), child(p1, n, _),')
old3 = ('COM818_trailing_comma_before_assignment_line(n) :- glob("\\\\,", n),\n'
        '    com818_ends_line_trailing_comma_before_assignment_line(n), child(p1, n, _),\n'
        '    kind(p1, "pattern_list"), child(p2, p1, _),\n')
assert old3 in t, "c3"
t = t.replace(old3, ('COM818_trailing_comma_before_assignment_line(n) :- com818_trailing_comma(n),\n'
                     '    com818_ends_line_trailing_comma_before_assignment_line(n), child(p2, n, _),\n'))
t += '''
// trailing_comma(p): p's last child is followed by a comma: in the source gap after it, or as a stored token
// (a trailing comma is not the word the grammar implies there, so the tape may keep it).
.decl com818_trailing_comma(p: node)
.stored com818_trailing_comma "a comma after a pattern_list's last child is its trailing comma, which the grammar does not imply"
com818_trailing_comma(p) :- kind(p, "pattern_list"), child(p, c, _), count : { child(p, c9, _), precedes(c, c9) } = 0,
    structural_after(c, g), match("(?s)\\\\s*,\\\\s*", g).
com818_trailing_comma(p) :- kind(p, "pattern_list"), child(p, c, _), count : { child(p, c9, _), precedes(c, c9) } = 0,
    node(_, c, _, _, e), token(p, _, ",", st, _), st >= e.
'''
wr(rid, "// OVERRIDE (hand-written, not generated). Clause trailing-comma-on-bare-tuple: as generated, except that\n"
        + KIND_NOTE +
        "// Clause trailing-comma-before-assignment-line: main matches the trailing `,` token of a pattern_list; the\n"
        "// tape's child relation holds no token, so the clause asks for a pattern_list whose source after its last\n"
        "// child is the comma (or whose stored token is), and the finding is at the pattern_list.\n", t)

# PYI010
rid = "ruff-pyi010-non-empty-stub-body"
t = rd(rid)
old = "PYI010_non_empty_stub_body_before_comment_raw(BODY, n) :- pyi010_sole_statement(n),\n"
assert old in t
t = t.replace(old, old + "    " + kinds("n") + ",\n")
wr(rid, "// OVERRIDE (hand-written, not generated). Clause non-empty-stub-body-before-comment: as generated, except\n"
        "// for the kinds below.\n" + KIND_NOTE, t)
print("overrides written")

# N806: a passed-in metavariable read only in a disjunction's test
rid = "ruff-n806-non-lowercase-variable-in-function"
t = rd(rid)
old = 'n806_model_value(n, LAST, MODEL, MODELTEXT, NAME, PATH) :- node(_, MODEL, _, _, _),kind(n, "call"),'
assert old in t, rid
i = t.index(old); j = t.index(".\n", i)
rule = t[i:j].replace(old, 'n806_model_value(n, LAST, MODEL, MODELTEXT, NAME, PATH) :- node(_, MODEL, _, _, _),'
                           'kind(n, "call"), docof(n, d_n), node(d_n, NAME, _, _, _),')
assert ",unset(MODEL),text(MODEL, t1_td)," in rule, rid
rule = rule.replace(",unset(MODEL),text(MODEL, t1_td),", ",text(MODEL, t1_td),")
t = t[:i] + rule + t[j:]
wr(rid, "// OVERRIDE (hand-written, not generated): util model-value compares the caller's NAME with the model's\n"
        "// text (`(!plain_string(m) ; text(NAME, MODELTEXT))`). NAME is passed in by the caller, which Datalog cannot\n"
        "// do, so the reader refuses NAME as ungrounded; here NAME ranges over the nodes of the model's file (main\n"
        "// compares within one file), and the caller's join on NAME keeps its own. The rest is the generated translation.\n", t)
print("n806 override")
