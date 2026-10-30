"""ES2018-ES2023 RegExp syntax features es-syntax checks: lookbehinds, named groups and Unicode property escapes.

es-x reads them with regexpp (lib/util/define-regexp-handler.js) in each regex literal and in each call or `new` of the
global RegExp (ReferenceTracker) whose pattern and flags getStringIfConstant reads, in unicode mode where the flags hold
`u` (a `v` alone is read without it), and reports the literal or the call once, where the pattern is valid. Here the
pattern's text (a literal's regex_pattern, a call's decoded string) is tokenized with regular expressions: an escape is
two characters, a character class runs to its first unescaped `]`. A literal's pattern is valid (the parser checks it);
a call's string must also match SAFE, a regular approximation of regexpp's grammar that accepts only valid patterns.
"""
import json
import os

import node_policies

ESX_VERSION = "7.8.0"


def _load_properties():
    """es-x's unicode-properties.js tables: the key names, and per year the values and binary properties it adds."""
    tools = node_policies.tools_dir()
    plugin = os.path.join(tools, "node_modules", "eslint-plugin-es-x")
    found = json.load(open(os.path.join(plugin, "package.json")))["version"]
    if found != ESX_VERSION:
        raise SystemExit(f"eslint-plugin-es-x {found} under {tools}; node_es_syntax/regexp.py pins {ESX_VERSION}")
    return node_policies.node_json(tools, plugin, r'''
import { createRequire } from "node:module";
const p = createRequire(process.cwd() + "/package.json")("./lib/util/unicode-properties.js");
const years = [2018, 2019, 2020, 2021, 2022, 2023, 2024, 2025];
const table = sets => Object.fromEntries(years.map(y => [y, [...sets["es" + y]].filter(Boolean)]));
console.log(JSON.stringify({ gcNames: [...p.gcNameSet], scNames: [...p.scNameSet], gc: table(p.gcValueSets), sc: table(p.scValueSets), bin: table(p.binPropertySets) }));
''')


PROPS = _load_properties()


def _alt(names):
    return "(?:" + "|".join(node_policies.esc(n) for n in sorted(names, key=lambda n: (-len(n), n))) + ")"


def _every(table):
    return [n for year in table.values() for n in year]


# A property escape's body regexpp accepts: a binary property or General_Category value alone, or a key=value pair.
VALID_PROPERTY = "(?:" + "|".join([
    _alt(_every(PROPS["bin"]) + _every(PROPS["gc"])),
    _alt(PROPS["gcNames"]) + "=" + _alt(_every(PROPS["gc"])),
    _alt(PROPS["scNames"]) + "=" + _alt(_every(PROPS["sc"]))]) + ")"


def new_property(year):
    """The bodies es-x's no-regexp-unicode-property-escapes-<year> forbids: the year's binary properties and Script values."""
    return "(?:" + "|".join(filter(None, [
        _alt(PROPS["bin"][str(year)]) if PROPS["bin"][str(year)] else None,
        _alt(PROPS["scNames"]) + "=" + _alt(PROPS["sc"][str(year)]) if PROPS["sc"][str(year)] else None])) + ")"


# ---- tokens --------------------------------------------------------------------------------------------------------

# Up to the token that follows, outside escapes and character classes: what regexpp reads as a group's start there.
OUTSIDE_CLASSES = r"(?s)^(?:[^\\\[]|\\.|\[(?:[^\\\]]|\\.)*\])*"
# Up to the token that follows, outside escapes (a property escape means the same in a class).
OUTSIDE_ESCAPES = r"(?s)^(?:[^\\]|\\.)*"
LOOKBEHIND = OUTSIDE_CLASSES + r"\(\?<[=!]"
NAMED_GROUP = OUTSIDE_CLASSES + r"\(\?<[^=!]"
PROPERTY = OUTSIDE_ESCAPES + r"\\[pP]\{"
# Every \p{...} or \P{...} names a property regexpp knows.
PROPERTIES_VALID = r"(?s)^(?:[^\\]|\\[^pP]|\\[pP]\{" + VALID_PROPERTY + r"\})*$"


def property_of(year):
    return OUTSIDE_ESCAPES + r"\\[pP]\{" + new_property(year) + r"\}"


# ---- SAFE: valid patterns, depth-bounded ---------------------------------------------------------------------------
# A regular subset of the ECMAScript pattern grammar regexpp 4.12 validates (ecmaVersion 2025): groups nest at most
# DEPTH deep; names, ranges and braced quantifiers are restricted to forms that are always valid. A pattern outside it
# is not reported (a missed report, never a false one). Checked beside it: a quantified lookaround (UNQUANTIFIABLE),
# a named back reference no group defines and a name used twice (value.regex's backreferences).
DEPTH = 6
NAME = r"[\p{ID_Start}_$][\p{ID_Continue}$\x{200C}\x{200D}]*"
HEX = "[0-9A-Fa-f]"


def _ranges():
    """A class range between ASCII letters or digits in code point order: per low end, the ends from it up."""
    alts = []
    for low in "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz":
        runs = [(max(a, low), b) for a, b in (("0", "9"), ("A", "Z"), ("a", "z")) if b >= low]
        alts.append(f"{low}-[" + "".join(a if a == b else f"{a}-{b}" for a, b in runs) + "]")
    return "(?:" + "|".join(alts) + ")"


RANGE = _ranges()
QUANTIFIER = (r"(?:(?:[*+?]|\{[0-9]+,?\}|\{0*[0-9],0*[1-9][0-9]+\}|\{0*[1-9][0-9],0*[1-9][0-9]{2,}\}|\{0*(?:"
              + "|".join(f"{n},0*[{n}-9]" for n in range(10)) + r")\})\??)")
SYNTAX_ESCAPE = r"[\^$\\.*+?()\[\]{}|/]"
U_CHARACTER_ESCAPE = rf"[fnrtv]|c[A-Za-z]|x{HEX}{{2}}|u{HEX}{{4}}|u\{{0*(?:{HEX}{{1,5}}|10{HEX}{{4}})\}}"
MODES = {
    # Unicode mode: identity escapes of syntax characters only; no lone ] { }; no quantified lookahead.
    "u": {
        "char": r"[^\^$\\.*+?()\[\]{}|]",
        "escape": rf"\\(?:[dDsSwW]|{U_CHARACTER_ESCAPE}|{SYNTAX_ESCAPE}|[pP]\{{[A-Za-z0-9_=]+\}}|k<{NAME}>)",
        "class_atom": rf"[^\\\]\-]|\\(?:[dDsSwWb]|{U_CHARACTER_ESCAPE}|{SYNTAX_ESCAPE}|\-|[pP]\{{[A-Za-z0-9_=]+\}})",
        "unquantifiable": r"\(\?<?[=!]",
    },
    # Annex B: any identity escape but \c (a control letter only) and \k (a named reference only); lone ] } and a
    # { before a plain character or the end are characters; a lookahead takes a quantifier.
    "n": {
        "char": r"[^\^$\\.*+?()\[{|]|\{(?:[^0-9\^$\\.*+?()\[{|]|$)",
        "escape": rf"\\(?:[^ckbB]|c[A-Za-z]|k<{NAME}>)",
        "class_atom": r"[^\\\]\-]|\\(?:[^ck]|c[A-Za-z0-9_])",
        "unquantifiable": r"\(\?<[=!]",
    },
}


def safe(mode):
    m = MODES[mode]
    atom = rf"\[\^?-?(?:{RANGE}|{m['class_atom']})*-?\]|{m['escape']}|{m['char']}|\."
    disjunction = rf"(?:\^|\$|\\[bB]|(?:{atom}){QUANTIFIER}?|\|)*"
    for _ in range(DEPTH):
        disjunction = rf"(?:\^|\$|\\[bB]|(?:{atom}|\((?:\?:|\?<{NAME}>|\?<?[=!])?{disjunction}\)){QUANTIFIER}?|\|)*"
    return rf"(?s)^{disjunction}$"


def _balanced(depth):
    """Tokens up to a group's closing parenthesis, groups nested at most `depth` deep."""
    inner = r"(?:[^\\\[()]|\\.|\[(?:[^\\\]]|\\.)*\])*"
    for _ in range(depth):
        inner = rf"(?:[^\\\[()]|\\.|\[(?:[^\\\]]|\\.)*\]|\({inner}\))*"
    return inner


SAFE = {mode: safe(mode) for mode in MODES}
# A lookaround regexpp refuses a quantifier on, followed by one.
UNQUANTIFIABLE = {mode: OUTSIDE_CLASSES + m["unquantifiable"] + _balanced(DEPTH) + r"\)(?:[*+?]|\{[0-9])" for mode, m in MODES.items()}
# Named back references regexpp resolves: none, or a single one whose group is defined (a pattern with two is not
# reported); call() also accepts an Annex B pattern without named groups, where \k<x> is an identity escape. Tokens
# outside classes but \k.
_NOT_REFERENCE = r"(?:[^\\\[]|\\[^k]|\[(?:[^\\\]]|\\.)*\])*"
REFERENCES_RESOLVED = (rf"(?s)^{_NOT_REFERENCE}$|^{_NOT_REFERENCE}\(\?<({NAME})>{_NOT_REFERENCE}\\k<\1>{_NOT_REFERENCE}$"
                       rf"|^{_NOT_REFERENCE}\\k<({NAME})>{_NOT_REFERENCE}\(\?<\2>{_NOT_REFERENCE}$")
# A group name used twice (invalid unless in different alternatives: not reported either way).
DUPLICATE_NAME = OUTSIDE_CLASSES + rf"\(\?<({NAME})>" + OUTSIDE_CLASSES.removeprefix("(?s)^") + r"\(\?<\1>"

# ---- the nodes es-x reports ----------------------------------------------------------------------------------------

NOT_COMMENT = node_policies.NOT_COMMENT
FIRST = node_policies.FIRST
SECOND = {"position": 2, "ofRule": NOT_COMMENT}
REGEXP_UTILS, _REPORT = node_policies.tracker({}, {"RegExp": {"$CALL": 1, "$CONSTRUCT": 1}})
REGEXP_CALL = {"any": _REPORT["global RegExp"]}


def literal(pattern_rx, mode):
    """A regex literal whose pattern matches, read in `mode`: "u" (its flags hold u), "n" (they do not)."""
    pattern = {"has": {"field": "pattern", "regex": pattern_rx}}
    if mode == "u":
        return {"kind": "regex", "all": [{"has": {"field": "flags", "regex": "u"}}, pattern]}
    # A u or v literal (regexpp reads a v one, or a call String(/p/u), in Annex B mode) where SAFE holds there.
    return {"kind": "regex", "all": [pattern, {"any": [{"not": {"has": {"field": "flags", "regex": "[uv]"}}}, {"has": {"field": "pattern", "regex": SAFE["n"]}}]}]}


def _arg(nth, rule):
    return {"has": {"field": "arguments", "kind": "arguments", "has": {"nthChild": nth, **rule}}}


def _value(**predicate):
    return {"value": {**predicate, "follow": True}}


# The flags argument, as String(getStringIfConstant(flags) ?? ""): unicode mode where it holds u.
FLAGS = {
    "u": _arg(SECOND, {"any": [_value(regex="u"), _value(type=["undefined", "null"]), _value(eq=True)]}),
    "n": {"any": [{"has": {"field": "arguments", "kind": "arguments", "not": {"has": {"nthChild": SECOND}}}},
                  _arg(SECOND, {"any": [{"kind": "spread_element"}, _value(regex="^[^u]*$"), _value(type="number"), _value(eq=False), _value(constant=False)]})]},
}


def call(detect, mode):
    """A RegExp call whose pattern argument, a valid pattern in `mode`, matches `detect` (a regex literal argument
    too: String(/p/f) is `/p/f`, valid where the literal's flags read it in the same mode)."""
    string = {"all": [_value(regex=detect), _value(regex=SAFE[mode]), {"not": _value(regex=UNQUANTIFIABLE[mode])},
                      {"any": [_value(regex=REFERENCES_RESOLVED)] + ([{"not": _value(regex=NAMED_GROUP)}] if mode == "n" else [])},
                      {"not": _value(regex=DUPLICATE_NAME)}]
              + ([_value(regex=PROPERTIES_VALID)] if mode == "u" else [])}
    return {"all": [REGEXP_CALL, FLAGS[mode], _arg(FIRST, {"any": [string, literal(detect, mode)]})]}


def feature(detect, modes=("u", "n")):
    alts = [literal(detect, m) for m in modes] + [call(detect, m) for m in modes]
    return {"rule": {"any": alts}, "utils": REGEXP_UTILS}


FEATURES = {
    # A regex literal or RegExp call whose pattern has a (?<= or (?<! lookbehind.
    "no-regexp-lookbehind-assertions": feature(LOOKBEHIND),
    # ... a (?<name> group (a \k<name> reference needs one).
    "no-regexp-named-capture-groups": feature(NAMED_GROUP),
    # ... in unicode mode, a \p{...} or \P{...} escape.
    "no-regexp-unicode-property-escapes": feature(PROPERTY, modes=("u",)),
    # ... in unicode mode, an escape of a binary property or Script value the year adds.
    **{f"no-regexp-unicode-property-escapes-{year}": feature(property_of(year), modes=("u",)) for year in (2019, 2020, 2021, 2022, 2023)},
}

# Literals are exact but for a v literal (read through SAFE's Annex B mode). A RegExp call's string has no regex parser
# here: it is reported only where SAFE and the checks beside it prove regexpp accepts it.
_LIMIT = ("A RegExp call's constant pattern (and a v-flag literal, which es-x reads in Annex B mode) is reported only "
          "where a regular approximation of regexpp's grammar accepts it: groups nested at most "
          f"{DEPTH} deep, class ranges between ASCII letters/digits, {{n,m}} with n <= m provable by digit count, no \\0 "
          "or decimal back reference in unicode mode, no Annex B \\c or lone \\k, at most one named back reference, "
          "no group name used twice (ES2025 allows (?<a>x)|(?<a>y)). Valid patterns outside it are missed, never "
          "reported falsely; so is a non-string constant pattern (an array's String()).")
LIMITS = {feature_id: _LIMIT for feature_id in FEATURES}
