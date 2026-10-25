"""Build a regular preorder tape (kind:u16, parent:u32, size:u32, depth:u16) from Python ASTs.

Usage: python3 -I gen.py <stdlib-dir> <out-dir>
Every file is one document; documents are concatenated with absolute preorder indices.
Expression contexts (Load/Store/Del) and operator nodes are kept as children so the shape
is closer to a concrete syntax tree than a bare AST.
"""
import ast, json, os, struct, sys, warnings

warnings.simplefilter("ignore")
root, out = sys.argv[1], sys.argv[2]
kinds, kind_of = [], {}
K, P, S, D, files = [], [], [], [], []


def kid(name):
    if name not in kind_of:
        kind_of[name] = len(kinds)
        kinds.append(name)
    return kind_of[name]


def children(node):
    for _, value in ast.iter_fields(node):
        if isinstance(value, ast.AST):
            yield value
        elif isinstance(value, list):
            for item in value:
                if isinstance(item, ast.AST):
                    yield item


for dirpath, _, names in sorted(os.walk(root)):
    for name in sorted(names):
        if not name.endswith(".py"):
            continue
        try:
            tree = ast.parse(open(os.path.join(dirpath, name), "rb").read())
        except Exception:
            continue
        start = len(K)
        stack = [(tree, 0xFFFFFFFF, 0)]
        while stack:
            node, parent, depth = stack.pop()
            index = len(K)
            K.append(kid(type(node).__name__))
            P.append(parent)
            S.append(0)
            D.append(depth)
            for child in reversed(list(children(node))):
                stack.append((child, index, depth + 1))
        # subtree sizes: reverse preorder accumulation
        for i in range(len(K) - 1, start - 1, -1):
            S[i] += 1
            if P[i] != 0xFFFFFFFF:
                S[P[i]] += S[i]
        files.append((start, len(K) - start))

n = len(K)
os.makedirs(out, exist_ok=True)
open(os.path.join(out, "kind.u16"), "wb").write(struct.pack(f"<{n}H", *K))
open(os.path.join(out, "parent.u32"), "wb").write(struct.pack(f"<{n}I", *P))
open(os.path.join(out, "size.u32"), "wb").write(struct.pack(f"<{n}I", *S))
open(os.path.join(out, "depth.u16"), "wb").write(struct.pack(f"<{n}H", *D))
json.dump({"nodes": n, "files": len(files), "kinds": kinds, "max_depth": max(D)}, open(os.path.join(out, "meta.json"), "w"))
print(n, "nodes", len(files), "files", len(kinds), "kinds", "max depth", max(D))
