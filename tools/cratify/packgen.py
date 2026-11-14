"""The language pack interface, generated from the adapter traits: `language/adapter/pack.rs`.

usage: packgen.py [--write]     (CRATIFY_BIN: the syn tool)

The core asks a family's questions of a `&'static dyn LanguagePack` its pack registered (`register`, `pack`), not of
a closed `with_adapter!` match. LanguagePack mirrors the static traits every adapter implements (`Adapter`,
`SyntaxRules` and its `Scope`, `Value`, `Arguments`, `Calls`, `FormatArgs` rules), object-safe:

  - a constant is a method (`A::LABEL` is `label()`, `<A::Scope as ScopeRules>::SUPPORTED` is `scope_supported()`);
  - a method generic over `D: Doc` is one method per document type the engine has (`_tape`, `_sub`, `_str`), and
    a free function here of the same name (`scope_visit(pack, ...)`) picks it by `D`'s TypeId;
  - an `impl Trait` argument is a `Box<dyn Trait + '_>`.

`Pack<A>` is the bridge: LanguagePack for any adapter, each method the static one. The packs keep implementing the
static traits as they do; each registers `&Pack::<Its>::NEW` as the engine loads.
"""
import json, os, re, subprocess, sys
HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, '..', '..'))
DSL = os.path.join(ROOT, 'native/src/code_rule_dsl')
TOOL = os.environ.get('CRATIFY_BIN', 'cratify')
OUT = os.path.join(DSL, 'language/adapter/pack.rs')
# trait, item, kind (const: a method; fn: a method; free: the module's free function), method: for `cratify packify`
TABLE = os.path.join(HERE, 'pack-methods.tsv')

# trait -> (file, module path, prefix of its methods, how the bridge names it for an adapter A)
TRAITS = {
    'Adapter': ('language/adapter.rs', 'crate::code_rule_dsl::language::adapter', '', 'A'),
    'SyntaxRules': ('language/adapter/syntax.rs', 'crate::code_rule_dsl::language::adapter::syntax', 'syntax_', 'A'),
    'ScopeRules': ('language/adapter/scope.rs', 'crate::code_rule_dsl::language::adapter::scope', 'scope_', '<A as Adapter>::Scope'),
    'ValueRules': ('language/adapter/value.rs', 'crate::code_rule_dsl::language::adapter::value', 'value_', '<A as Adapter>::Value'),
    'ArgumentRules': ('language/adapter/arguments.rs', 'crate::code_rule_dsl::language::adapter::arguments', 'arguments_', '<A as Adapter>::Arguments'),
    'CallRules': ('language/adapter/calls.rs', 'crate::code_rule_dsl::language::adapter::calls', 'calls_', '<A as Adapter>::Calls'),
    'FormatArgRules': ('language/adapter/format_args.rs', 'crate::code_rule_dsl::language::adapter::format_args', 'format_args_', '<A as Adapter>::FormatArgs'),
}
DOCS = [
    ('tape', 'crate::code_rule_dsl::tape::TapeDoc'),
    ('sub', 'crate::code_rule_dsl::literal::SubDoc'),
    ('str', 'ast_grep_core::tree_sitter::StrDoc<crate::code_rule_dsl::language::GrammarDialect>'),
]
PRELUDE = {'Option', 'Some', 'None', 'Result', 'Ok', 'Err', 'Vec', 'String', 'Box', 'bool', 'str', 'usize', 'u8', 'u16',
           'u32', 'u64', 'i64', 'i32', 'f64', 'Self', 'FnMut', 'FnOnce', 'Fn', 'dyn', 'impl', 'mut', 'as', 'D', 'Doc',
           'crate', 'super', 'std', 'static', 'char', 'isize', 'u128', 'i128', 'f32', 'i8', 'i16'}


def run(*args):
    return subprocess.run([TOOL, *args], capture_output=True, text=True, check=True).stdout


def absolute(path, mod):
    segs = mod.split('::')
    n = 0
    while path.startswith('super::'):
        path, n = path[7:], n + 1
    if n:
        return '::'.join(segs[:len(segs) - n]) + '::' + path
    if path.startswith('self::'):
        return mod + '::' + path[6:]
    return path


def names_of(file, mod):
    """Name -> full path, for each name the file's `use`s bring in and each item it defines."""
    m = {}
    for line in run('uses', os.path.join(DSL, file)).splitlines():
        n, p = line.split('\t')
        if n != '*':
            m[n] = absolute(p, mod)
    for line in run('items', os.path.join(DSL, file)).splitlines():
        f, s, e, kind, name, inline, cfg = line.split('\t')
        if not inline and kind in ('struct', 'enum', 'type', 'trait', 'const', 'fn', 'union', 'static'):
            m.setdefault(name, f'{mod}::{name}')
    return m


def qualify(ty, names):
    """`ty` (tokens as syn prints them) with each bare name the file imports or defines as its full path."""
    toks = re.findall(r"'?[A-Za-z_][A-Za-z_0-9]*|::|\S", ty)
    out = []
    for i, t in enumerate(toks):
        prev = toks[i - 1] if i else ''
        if t in names and t not in PRELUDE and not t.startswith("'") and prev != '::':
            out.append(names[t])
        else:
            out.append(t)
    s = ' '.join(out)
    for a, b in ((' :: ', '::'), (' < ', '<'), (' >', '>'), ('< ', '<'), (' ,', ','), ('& ', '&'), (" '", " '"),
                 ('( ', '('), (' )', ')'), ('! ', '!')):
        s = s.replace(a, b)
    return s.replace('&mut', '&mut ').replace('&mut  ', '&mut ').replace('dyn', 'dyn ').replace('dyn  ', 'dyn ')


def snake(name):
    return name.lower()


def generics_split(gens):
    """(lifetimes, doc param name or None, other type params)."""
    lts, doc, other = [], None, []
    for g in gens:
        if g.startswith("'"):
            lts.append(g)
        elif re.match(r'D\s*:\s*Doc$', g.replace(' ', '')) or g.replace(' ', '') == 'D:Doc':
            doc = 'D'
        else:
            other.append(g)
    return lts, doc, other


def mentions_d(ty):
    return re.search(r'\bD\b', ty) is not None


def boxed(ty):
    """An `impl Trait` argument's type in the object-safe method: `Box<dyn Trait + '_>`."""
    if ty.startswith('impl '):
        return f"Box<dyn {ty[5:]} + '_>"
    return ty


# Methods the rules below do not cover, written out: (declaration, bridge, free function).
OVERRIDES = {
    'CallRules::definition_keys': (
        """    fn calls_definition_keys<'s>(&self, name: &crate::code_rule_dsl::calls::names::Name, flags: u8, spellings: Box<dyn FnOnce(&crate::code_rule_dsl::calls::names::Name) -> Box<dyn Iterator<Item = crate::code_rule_dsl::calls::names::Name> + 's> + 's>, key: Box<dyn FnMut(crate::code_rule_dsl::calls::names::Name) + 's>);""",
        """    fn calls_definition_keys<'s>(&self, name: &crate::code_rule_dsl::calls::names::Name, flags: u8, spellings: Box<dyn FnOnce(&crate::code_rule_dsl::calls::names::Name) -> Box<dyn Iterator<Item = crate::code_rule_dsl::calls::names::Name> + 's> + 's>, key: Box<dyn FnMut(crate::code_rule_dsl::calls::names::Name) + 's>) {
        <<A as Adapter>::Calls as CallRules>::definition_keys(name, flags, spellings, key)
    }""",
        """/// `CallRules::definition_keys` of `pack`.
pub(crate) fn calls_definition_keys<'s, I: Iterator<Item = crate::code_rule_dsl::calls::names::Name> + 's>(pack: &dyn LanguagePack, name: &crate::code_rule_dsl::calls::names::Name, flags: u8, spellings: impl FnOnce(&crate::code_rule_dsl::calls::names::Name) -> I + 's, key: impl FnMut(crate::code_rule_dsl::calls::names::Name) + 's) {
    pack.calls_definition_keys(name, flags, Box::new(move |name| Box::new(spellings(name)) as Box<dyn Iterator<Item = crate::code_rule_dsl::calls::names::Name> + 's>), Box::new(key))
}
"""),
}


def main(*flags):
    traits = {}
    for line in run('traits', *[os.path.join(DSL, f) for f, *_ in TRAITS.values()]).splitlines():
        t = json.loads(line)
        if t['trait'] in TRAITS:
            traits[t['trait']] = t
    missing = set(TRAITS) - set(traits)
    if missing:
        sys.exit(f'no trait {missing}')
    decls, bridge, free = [], [], []
    table = []
    count = {'const': 0, 'fn': 0, 'doc fn': 0}
    for tname, (file, mod, prefix, who) in TRAITS.items():
        names = names_of(file, mod)
        decls.append(f'    // {tname}')
        bridge.append(f'    // {tname}')
        for it in traits[tname]['items']:
            if it['kind'] == 'type':
                continue
            if it['kind'] == 'const':
                ty = qualify(it['ty'], names)
                m = prefix + snake(it['name'])
                decls.append(f'    fn {m}(&self) -> {ty};')
                bridge.append(f'    fn {m}(&self) -> {ty} {{\n        <{who} as {tname}>::{it["name"]}\n    }}')
                count['const'] += 1
                table.append((tname, it['name'], 'const', m))
                continue
            key = f'{tname}::{it["name"]}'
            if key in OVERRIDES:
                d, b, f = OVERRIDES[key]
                decls.append(d); bridge.append(b); free.append(f)
                table.append((tname, it['name'], 'free', prefix + it['name']))
                count['fn'] += 1
                continue
            if it['receiver'] or it['where']:
                sys.exit(f'{tname}::{it["name"]}: a receiver or where clause is not handled')
            cfg = ''.join(c + '\n    ' for c in it.get('cfg', []))
            cfgf = ''.join(c + '\n' for c in it.get('cfg', []))
            lts, doc, other = generics_split(it['generics'])
            if other:
                sys.exit(f'{tname}::{it["name"]}: type parameters {other}')
            args = [(p, qualify(t, names)) for p, t in it['inputs']]
            out = qualify(it['output'], names) if it['output'] else None
            # an elided lifetime in the output came from the one reference argument; `&self` would take it
            if out and ("'_" in out or re.match(r'&(?!\')', out)) and not lts:
                lts = ["'e"]
                out = re.sub(r"&(?!')", "&'e ", out.replace("'_", "'e"))
                args = [(p, re.sub(r"^&(?!')", "&'e ", t.replace("'_", "'e"))) for p, t in args]
            m = prefix + it['name']
            pats = [re.sub(r'^mut ', '', p) for p, _ in args]
            pats = [p if re.match(r'^[a-z_][a-z_0-9]*$', p) else f'arg{i}' for i, p in enumerate(pats)]
            call_args = ', '.join(f'{p}' for p in pats)
            if not doc:
                lt = f'<{", ".join(lts)}>' if lts else ''
                sig = ', '.join(f'{p}: {boxed(t)}' for p, (_, t) in zip(pats, args))
                ret = f' -> {out}' if out else ''
                decls.append(f'    {cfg}fn {m}{lt}(&self{", " if sig else ""}{sig}){ret};')
                bridge.append(f'    {cfg}fn {m}{lt}(&self{", " if sig else ""}{sig}){ret} {{\n        <{who} as {tname}>::{it["name"]}({call_args})\n    }}')
                table.append((tname, it['name'], 'free' if any(t.startswith('impl ') for _, t in args) else 'fn', m))
                if any(t.startswith('impl ') for _, t in args):
                    fsig = ', '.join(f'{p}: {t}' for p, (_, t) in zip(pats, args))
                    fargs = ', '.join(f'Box::new({p})' if t.startswith('impl ') else p for p, (_, t) in zip(pats, args))
                    free.append(f'/// `{tname}::{it["name"]}` of `pack`.\n{cfgf}pub(crate) fn {m}{lt}(pack: &dyn LanguagePack, {fsig}){ret} {{\n    pack.{m}({fargs})\n}}\n')
                count['fn'] += 1
                continue
            count['doc fn'] += 1
            table.append((tname, it['name'], 'free', m))
            for suffix, dty in DOCS:
                sub = lambda t: re.sub(r'\bD\b', dty, t)
                lt = f'<{", ".join(lts)}>' if lts else ''
                sig = ', '.join(f'{p}: {boxed(sub(t))}' for p, (_, t) in zip(pats, args))
                ret = f' -> {sub(out)}' if out else ''
                decls.append(f'    {cfg}fn {m}_{suffix}{lt}(&self{", " if sig else ""}{sig}){ret};')
                bridge.append(f'    {cfg}fn {m}_{suffix}{lt}(&self{", " if sig else ""}{sig}){ret} {{\n        <{who} as {tname}>::{it["name"]}::<{dty}>({call_args})\n    }}')
            # the free function: D's document type picks the method
            gl = ', '.join(lts + ['D: Doc'])
            sig = ', '.join(f'{p}: {t}' for p, (_, t) in zip(pats, args))
            ret = f' -> {out}' if out else ''
            arms = []
            for suffix, dty in DOCS:
                cast_args = ', '.join(
                    (f'Box::new({p})' if t.startswith('impl ') else f'same({p})') if mentions_d(t) or t.startswith('impl ') else p
                    for p, (_, t) in zip(pats, args))
                callx = f'pack.{m}_{suffix}({cast_args})'
                if out and mentions_d(out):
                    callx = f'same({callx})'
                if 'same(' in callx:
                    callx = f'unsafe {{ {callx} }}'
                arms.append(f'    if TypeId::of::<D>() == TypeId::of::<{dty}>() {{\n        return {callx};\n    }}')
            free.append(
                f'/// `{tname}::{it["name"]}` of `pack`, for `D`.\n'
                f'{cfgf}pub(crate) fn {m}<{gl}>(pack: &dyn LanguagePack{", " if sig else ""}{sig}){ret} {{\n'
                + '\n'.join(arms) +
                f'\n    unreachable!("{m}: a document type no language pack takes: {{}}", std::any::type_name::<D>())\n}}\n')
    text = f'''//! The language pack interface (generated by tools/cratify/packgen.py from the adapter traits; do not edit).
//!
//! The core asks a family's questions of the pack its language crate registered: `pack(family)`, a
//! `&'static dyn LanguagePack`. LanguagePack mirrors the static adapter traits, object-safe: a constant is a
//! method, a method generic over the document is one method per document type the engine has, which the free
//! function of the same name picks. `Pack<A>` is the bridge every adapter registers as (`&Pack::<A>::NEW`).
#![allow(clippy::too_many_arguments)]
use std::any::TypeId;
use std::sync::OnceLock;

use ast_grep_core::Doc;

use crate::code_rule_dsl::language::Family;
use crate::code_rule_dsl::language::adapter::Adapter;
use crate::code_rule_dsl::language::adapter::arguments::ArgumentRules;
use crate::code_rule_dsl::language::adapter::calls::CallRules;
use crate::code_rule_dsl::language::adapter::format_args::FormatArgRules;
use crate::code_rule_dsl::language::adapter::scope::ScopeRules;
use crate::code_rule_dsl::language::adapter::syntax::SyntaxRules;
use crate::code_rule_dsl::language::adapter::value::ValueRules;

/// A language family's rules, as the core asks them.
pub(crate) trait LanguagePack: Send + Sync {{
{chr(10).join(decls)}
}}

/// An adapter's rules as a LanguagePack: each method the static one.
pub(crate) struct Pack<A>(std::marker::PhantomData<fn() -> A>);

impl<A> Pack<A> {{
    pub(crate) const NEW: Self = Pack(std::marker::PhantomData);
}}

impl<A: Adapter + SyntaxRules> LanguagePack for Pack<A> {{
{chr(10).join(bridge)}
}}

/// A value of type `A` as type `B`.
///
/// # Safety
/// `A` and `B` are the same type (the callers compare the document types' TypeIds first).
unsafe fn same<A, B>(a: A) -> B {{
    let a = std::mem::ManuallyDrop::new(a);
    unsafe {{ std::ptr::read(&*a as *const A as *const B) }}
}}

{chr(10).join(free)}
static PACKS: [OnceLock<&'static dyn LanguagePack>; Family::ALL.len()] = [const {{ OnceLock::new() }}; Family::ALL.len()];

/// Registers `family`'s pack; the first registration holds. The engine registers every pack its build has as it
/// loads (`crate::install_languages`).
pub(crate) fn register(family: Family, pack: &'static dyn LanguagePack) {{
    let _ = PACKS[family as usize].set(pack);
}}

/// `family`'s pack. Every family the build has is registered as the engine loads: none is a build without that
/// language's crate, which no file of the family reaches.
pub(crate) fn pack(family: Family) -> &'static dyn LanguagePack {{
    PACKS[family as usize].get().copied().unwrap_or_else(|| panic!("no language pack registered for {{family:?}}"))
}}
'''
    print(f'LanguagePack: {count["const"]} constants, {count["fn"]} methods, {count["doc fn"]} methods per document type')
    if '--write' in flags:
        open(OUT, 'w').write(text)
        open(TABLE, 'w').write(''.join('\t'.join(r) + '\n' for r in table))
        print('wrote', os.path.relpath(OUT, ROOT))
    else:
        sys.stdout.write(text[:3000])


if __name__ == '__main__':
    main(*sys.argv[1:])
