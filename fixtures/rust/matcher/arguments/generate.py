#!/usr/bin/env python3
"""Writes rust.expected.json: how rustc 1.99.0 binds each call's arguments to the callee's parameters
(offline, run by a maintainer; never in CI).

Each case is `items` (the callee: a fn whose body prints `<parameter>=<value>` for each parameter in order,
a method, a closure, a tuple struct or variant) and one `call`. The program `<items> fn main() { let _ =
<call>; }` (for a tuple struct or variant: `let v = <call>;` then the fields printed back) is compiled with
`rustc +1.99.0 --edition 2024` and run. Recorded per case: `name`, `call`, `signature` (the parameter names
in order; for method-call syntax without the receiver, which is no argument), `compiled` (rustc accepted
the program and it ran), `diagnostics` (rustc's stderr lines when it did not), and `bound`: parameter name
-> the value the run printed for it. Every argument is a distinct integer, so `bound` says which argument
rustc passed to which parameter.

Pins (refuses anything else): rustc 1.99.0 (b940084d7 2026-09-28), edition 2024.
Build products go to <checkout>/.rerail/tools/rust/arguments/.
Usage: python3 generate.py
"""

import json, pathlib, re, shutil, subprocess
HERE = pathlib.Path(__file__).resolve().parent
REPO = HERE.parents[3]
OUT = REPO / ".rerail/tools/rust/arguments"
TC = "1.99.0"


def run(source, name):
    work = OUT / name
    work.mkdir(parents=True, exist_ok=True)
    src = work / "main.rs"
    exe = work / "main"
    src.write_text(source)
    p = subprocess.run(["rustc", "+" + TC, "--edition", "2024", str(src), "-o", str(exe)], text=True, capture_output=True)
    if p.returncode:
        return False, p.stderr.splitlines(), {}
    q = subprocess.run([str(exe)], text=True, capture_output=True)
    bound = {}
    for line in q.stdout.splitlines():
        if "=" in line:
            k, v = line.split("=", 1)
            bound[k] = v
    return q.returncode == 0, q.stderr.splitlines(), bound


def fn(name, sig):
    params = ", ".join(f"{p}: i32" for p in sig)
    body = " ".join(f'println!("{p}={{}}", {p});' for p in sig)
    return f"fn {name}({params}) -> i32 {{ {body} 0 }}"


def main():
    version = subprocess.check_output(["rustc", "+" + TC, "-V"], text=True).strip()
    if version != "rustc 1.99.0 (b940084d7 2026-09-28)":
        raise SystemExit(f"unexpected rustc: {version}")
    cases = []
    for n in range(10):
        sig = ["a", "b", "c", "d"][:n % 5]
        args = ", ".join(str(i + 1) for i in range(len(sig)))
        cases.append({"name": f"free_{n}", "items": fn("f", sig), "call": f"f({args})", "signature": sig})
    cases += [
        {"name":"parenthesised", "items":fn("f",["a","b"]), "call":"f((1), { 2 })", "signature":["a","b"]},
        {"name":"negative", "items":fn("f",["a","b"]), "call":"f(-1, 2)", "signature":["a","b"]},
        {"name":"commented", "items":fn("f",["a","b"]), "call":"f(1, /* c */ 2)", "signature":["a","b"]},
        {"name":"trailing", "items":fn("f",["a","b"]), "call":"f(1, 2,)", "signature":["a","b"]},
        {"name":"generic", "items":"fn f<T: Copy>(a: i32) -> i32 { println!(\"a={}\",a); 0 }", "call":"f::<i32>(1)", "signature":["a"]},
        {"name":"nested", "items":fn("f",["a","b"])+"\nfn g(x: i32) -> i32 { println!(\"x={}\", x); x }", "call":"f(g(1), 2)", "signature":["a","b"]},
        {"name":"extra", "items":fn("f",["a"]), "call":"f(1, 2)", "signature":["a"]},
        {"name":"short", "items":fn("f",["a","b","c"]), "call":"f(1)", "signature":["a","b","c"]},
        {"name":"closure", "items":"", "call":"{ |a: i32, b: i32| { println!(\"a={}\", a); println!(\"b={}\", b); 0 } }(1, 2)", "signature":["a","b"]},
        {"name":"fn_pointer", "items":fn("f",["a"]), "call":"(f as fn(i32) -> i32)(1)", "signature":["a"]},
        {"name":"tuple_struct", "items":"struct P(i32, i32);", "call":"P(1, 2)", "signature":["0","1"]},
        {"name":"variant", "items":"enum E { V(i32, i32) }", "call":"E::V(1, 2)", "signature":["0","1"]},
        {"name":"method", "items":"struct S; impl S { fn m(&self, a: i32, b: i32) -> i32 { println!(\"a={}\",a); println!(\"b={}\",b); 0 } }", "call":"S.m(1, 2)", "signature":["a","b"]},
        {"name":"chain", "items":"struct S; impl S { fn m(&self, a: i32) -> S { println!(\"a={}\",a); S } }", "call":"S.m(1).m(2)", "signature":["a"]},
        {"name":"ufcs", "items":"struct S(i32); impl S { fn m(&self, a: i32) -> i32 { println!(\"self={}\", self.0); println!(\"a={}\",a); 0 } }", "call":"S::m(&S(7), 1)", "signature":["self","a"]},
        {"name":"missing", "items":fn("f",["a","b"]), "call":"f(1,)", "signature":["a","b"]},
    ]
    # Add repeated valid shapes to keep the oracle broad and stable.
    for i in range(20):
        sig = ["a", "b", "c"][: (i % 3) + 1]
        cases.append({"name":f"variant_{i}", "items":fn("f",sig), "call":f"f({', '.join(str(x) for x in range(1,len(sig)+1))})", "signature":sig})
    OUT.mkdir(parents=True, exist_ok=True)
    rows=[]
    for c in cases:
        # A constructor prints nothing itself: the program reads back the fields rustc filled.
        if c["name"] == "tuple_struct":
            body = "let v = " + c["call"] + "; println!(\"0={}\", v.0); println!(\"1={}\", v.1);"
        elif c["name"] == "variant":
            body = "let E::V(a, b) = " + c["call"] + "; println!(\"0={}\", a); println!(\"1={}\", b);"
        else:
            body = "let _ = " + c["call"] + ";"
        source = c["items"] + "\nfn main() { " + body + " }\n"
        compiled, diagnostics, bound = run(source, c["name"])
        rows.append({"name":c["name"],"call":c["call"],"signature":c["signature"],"compiled":compiled,"diagnostics":diagnostics,"bound":bound})
    (HERE/"rust.cases.json").write_text(json.dumps(cases, indent=1)+"\n")
    (HERE/"rust.expected.json").write_text(json.dumps({"tool":"rustc","version":subprocess.check_output(["rustc", "+"+TC, "-V"],text=True).strip(),"cases":rows}, indent=1)+"\n")
    print(json.dumps({"cases":len(rows),"compiled":sum(x["compiled"] for x in rows),"failed":sum(not x["compiled"] for x in rows)}))

if __name__ == "__main__": main()
