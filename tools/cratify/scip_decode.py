"""Decode a SCIP index (protobuf) into two TSVs: definitions and references.

defs.tsv: symbol, file, line, col, end_line, end_col, enc_start_line, enc_start_col, enc_end_line, enc_end_col, kind
refs.tsv: file, line, col, end_col, symbol
Lines and columns are 0-based, as SCIP gives them.
"""
import os, sys

def varint(b, i):
    r = 0; s = 0
    while True:
        x = b[i]; i += 1
        r |= (x & 0x7F) << s
        if x < 0x80: return r, i
        s += 7

def fields(b, i, end):
    while i < end:
        key, i = varint(b, i)
        f, w = key >> 3, key & 7
        if w == 0:
            v, i = varint(b, i); yield f, w, v, None
        elif w == 2:
            n, i = varint(b, i); yield f, w, i, i + n; i += n
        elif w == 1: i += 8
        elif w == 5: i += 4
        else: raise ValueError(f'wire type {w}')

def packed(b, s, e):
    out = []; i = s
    while i < e:
        v, i = varint(b, i); out.append(v)
    return out

def rng(r):
    if len(r) == 3: return r[0], r[1], r[0], r[2]
    return tuple(r[:4])

def main(path, outdir):
    b = open(path, 'rb').read()
    os.makedirs(outdir, exist_ok=True)
    defs = open(f'{outdir}/defs.tsv', 'w'); refs = open(f'{outdir}/refs.tsv', 'w')
    nd = nr = 0
    for f, w, s, e in fields(b, 0, len(b)):
        if f != 2: continue
        rel = None; occs = []; kinds = {}
        for g, w2, s2, e2 in fields(b, s, e):
            if g == 1: rel = b[s2:e2].decode()
            elif g == 2: occs.append((s2, e2))
            elif g == 3:
                sym = None; kind = 0
                for h, w3, s3, e3 in fields(b, s2, e2):
                    if h == 1: sym = b[s3:e3].decode()
                    elif h == 5: kind = s3
                if sym: kinds[sym] = kind
        for s2, e2 in occs:
            r = []; sym = ''; roles = 0; enc = []
            for h, w3, s3, e3 in fields(b, s2, e2):
                if h == 1: r = packed(b, s3, e3) if w3 == 2 else r + [s3]
                elif h == 2: sym = b[s3:e3].decode()
                elif h == 3: roles = s3
                elif h == 7: enc = packed(b, s3, e3) if w3 == 2 else enc + [s3]
            if not sym or sym.startswith('local '): continue
            l, c, el, ec = rng(r)
            if roles & 1:
                E = rng(enc) if enc else ('', '', '', '')
                defs.write(f'{sym}\t{rel}\t{l}\t{c}\t{el}\t{ec}\t{E[0]}\t{E[1]}\t{E[2]}\t{E[3]}\t{kinds.get(sym, 0)}\n'); nd += 1
            else:
                refs.write(f'{rel}\t{l}\t{c}\t{ec}\t{sym}\n'); nr += 1
    print('defs', nd, 'refs', nr)

if __name__ == '__main__':
    main(sys.argv[1], sys.argv[2])
