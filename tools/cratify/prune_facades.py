"""Prunes a crate's facades (its generated lib.rs) of the repeated imports the compiler refuses.

A facade repeats its module's file `use`s so that a child's `use super::*` sees what it saw in one crate. Some of
those name items another crate keeps private, or modules it does not have: a child that reads one fails on its own,
and the rest are not needed. `cargo check`'s E0603 (private) and E0432 (unresolved) at a facade's line name each:
the name is dropped from that `use` (the `use` itself when it was the last).

    python3 prune_facades.py CHECK_OUTPUT...      (paths in the outputs are relative to the repository)
"""
import collections, re, sys


def main(*outputs):
    drops = collections.defaultdict(set)  # (lib.rs, line) -> names
    for out in outputs:
        text = open(out).read()
        for m in re.finditer(r'^(native/crates/[a-z_]+/src/lib\.rs):(\d+):(\d+): error\[(E0603|E0432|E0365)\]: (.*)$', text, re.M):
            f, line, msg = m.group(1), int(m.group(2)), m.group(5)
            names = re.findall(r'`(?:[A-Za-z_][A-Za-z_0-9]*::)*([A-Za-z_][A-Za-z_0-9]*)`', msg.split(': ')[0])
            if m.group(4) == 'E0432':
                # unresolved imports `a::b::X`, `a::Y`
                names = re.findall(r'`(?:[A-Za-z_][A-Za-z_0-9]*::)+([A-Za-z_][A-Za-z_0-9]*)`', msg.split(': ')[0])
            for n in names:
                drops[(f, line)].add(n)
    by_file = collections.defaultdict(dict)
    for (f, line), names in drops.items():
        by_file[f][line] = names
    total = 0
    for f, lines in by_file.items():
        src = open(f).read().split('\n')
        for line, names in lines.items():
            i = line - 1
            text = src[i]
            g = re.match(r'^(\s*(?:pub(?:\([^)]*\))?\s+)?use\s+)(.*?)\{(.*)\};\s*$', text)
            if g:
                keep = [b.strip() for b in g.group(3).split(',') if b.strip() and b.strip().split(' as ')[-1].strip() not in names and b.strip().split(' as ')[0].split('::')[-1].strip() not in names]
                src[i] = f'{g.group(1)}{g.group(2)}{{{", ".join(keep)}}};' if keep else ''
            else:
                one = re.match(r'^\s*(?:pub(?:\([^)]*\))?\s+)?use\s+(?:.*::)?([A-Za-z_][A-Za-z_0-9]*)(?:\s+as\s+([A-Za-z_][A-Za-z_0-9]*))?\s*;\s*$', text)
                if one and (one.group(1) in names or one.group(2) in names):
                    src[i] = ''
            total += 1
        # a blank line left where a use was: its `#[allow(unused_imports)]` goes too
        out = []
        for l in src:
            if l == '' and out and out[-1].strip() == '#[allow(unused_imports)]':
                out.pop()
                continue
            out.append(l)
        open(f, 'w').write('\n'.join(out))
    print(f'pruned {total} facade uses')


if __name__ == '__main__':
    main(*sys.argv[1:])
