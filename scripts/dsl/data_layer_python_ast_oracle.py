#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Generate CPython 3.13 AST serialization and signed-fingerprint expectations."""
import argparse,ast,hashlib,json,sys
from pathlib import Path


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source',type=Path,required=True);parser.add_argument('--recording',type=Path,required=True);parser.add_argument('--output',type=Path,required=True)
    args=parser.parse_args()
    if sys.version_info[:2]!=(3,13):raise RuntimeError('This contract requires CPython 3.13')
    samples=[
        'def f(): pass',
        'def f(a, /, b:int=2, *args, k=None, **kw) -> str:\n return "x"',
        'def f[T, *Ts, **P](x:T)->T:\n return x',
        'async def f(x):\n async with x as y:\n  async for z in y:\n   await z()\n return None',
        'x = -1 + 2.0 * 0xff / 2j; x += 3; del x',
        'x = a and b and c or d; y = 1 < a <= 3; z = a is not b; w = x not in y',
        'x = {**a, "a": [1, *b], "b": {2, *c}}; y = a[1:2:3, ..., :]; z = (a,)',
        'x = [v for v in values if v]; y = {k:v for k,v in pairs}; z = (f(v) for v in xs)',
        'def f():\n yield 1\n yield from xs\n return',
        'if x:\n pass\nelif y:\n pass\nelse:\n pass',
        'while x:\n break\nelse:\n pass\nfor a,b in xs:\n continue\nelse:\n pass',
        'try:\n a()\nexcept (A, B) as e:\n raise C() from e\nelse:\n b()\nfinally:\n c()',
        'with a() as b, c as d:\n assert b, "message"',
        'from ..a import b as c, d\nimport os.path as path, sys\nglobal x, y\n',
        'class C[T](Base, flag=True):\n "doc"\n x: list[str] = []\n @staticmethod\n def f(): return 2',
        'x = lambda a=2, *, b: a+b\ny = (a:=3)\nx: A\nx: A = y',
        'x = f"value={a!r}; {b:03d}; {c=}"\ny = "a" "b"\nz = b"a\\x00"',
        'def 𝒞(ﬁle: "T") -> list[int]:\n return ﬁle',
    ]
    for event in json.loads(args.recording.read_text())['events']:
        if event['check']=='cached_pure_lazy_allowlist':samples.extend(file['source'] for file in event.get('files',[]))
    for path in ('l2055_lazy_result/purity.py','l2055_lazy_result/cached_pure_lazy_policy.py','l2070_batching/optimizer_run_cache.py'):
        samples.append((args.source/path).read_text())
    cases=[]
    for index,source in enumerate(dict.fromkeys(samples)):
        tree=ast.parse(source);functions=[]
        for node in ast.walk(tree):
            if not isinstance(node,(ast.FunctionDef,ast.AsyncFunctionDef)):continue
            body=ast.dump(ast.Module(body=node.body,type_ignores=[]),include_attributes=False)
            signature=ast.dump(node.args,include_attributes=False)+'|'+(ast.dump(node.returns,include_attributes=False) if node.returns else 'None')+'|'+ast.dump(ast.Tuple(elts=node.type_params),include_attributes=False)
            functions.append({'name':node.name,'line':node.lineno,'body':body,'signature':signature,'body_sha256':hashlib.sha256(body.encode()).hexdigest(),'signature_sha256':hashlib.sha256(signature.encode()).hexdigest()})
        cases.append({'id':index,'source':source,'dump':ast.dump(tree,include_attributes=False),'functions':functions})
    args.output.parent.mkdir(parents=True,exist_ok=True);args.output.write_text(json.dumps({'python':sys.version,'contract':'CPython 3.13 ast.dump, include_attributes=False, show_empty=False','cases':cases},indent=2)+'\n')
    print(json.dumps({'cases':len(cases),'functions':sum(len(c['functions']) for c in cases)}))
if __name__=='__main__':main()
