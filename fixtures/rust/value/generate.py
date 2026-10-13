#!/usr/bin/env python3
"""Record Rust expression values with rustc 1.99.0 (offline maintainer tool).

Each case records `const` from `const C: T = src` and `eval` from running the
same expression, with errors encoded as ["error", code-or-lint, message].
Successful values use tagged arrays: integers, floats (debug and IEEE bits),
booleans, chars, strings, and byte arrays. `warnings` contains non-error
rustc diagnostics. Cases marked `unknown` retain the author's reason.

Reads rust.cases.json and writes rust.expected.json. The pinned compiler is
rustc 1.99.0 (b940084d7 2026-09-28), edition 2024; run with Python 3.9+.
Build products are written under <checkout>/.rerail/tools/rust/value/. The
recorder compiles each case independently, so a diagnostic in one case cannot
hide another; this is the fallback equivalent of batch compilation.
"""
import json, pathlib, re, subprocess, sys
HERE = pathlib.Path(__file__).resolve().parent
REPO = HERE.parents[2]
TOOLCHAIN = '1.99.0'
RUSTC_VERSION = 'rustc 1.99.0 (b940084d7 2026-09-28)'
EDITION = '2024'

def checkout_root():
    for p in [REPO, *REPO.parents]:
        if (p/'.rerail'/'corpora'/'rust').is_dir(): return p
    raise SystemExit('no .rerail/corpora/rust above '+str(REPO))
BUILD = checkout_root()/'.rerail'/'tools'/'rust'/'value'

def check_pins():
    got=subprocess.run(['rustc','+'+TOOLCHAIN,'-V'],capture_output=True,text=True,check=True).stdout.strip()
    if got != RUSTC_VERSION: raise SystemExit(f'want {RUSTC_VERSION}, found {got}')

def diag(stderr):
    out=[]
    for line in stderr.splitlines():
        try: d=json.loads(line)
        except ValueError: continue
        if d.get('$message_type')!='diagnostic': continue
        code=(d.get('code') or {}).get('code') or ''
        out.append(['error' if d.get('level')=='error' else d.get('level',''), code, d.get('message','')])
    return out

def rust_src(typ, expr, const):
    # The emitter is deliberately dependency-free and keeps the oracle's tagged shape.
    binding = f'const C: {typ} = {expr};' if const and typ else (f'let v: {typ} = {expr};' if typ else f'let v = {expr};')
    use = 'C' if const and typ else 'v'
    return f'''#![allow(unused, unused_comparisons)]
use std::fmt::Debug;
fn hx(b:&[u8])->String{{b.iter().map(|x|format!("{{:02x}}",x)).collect()}}
trait E{{fn e(&self)->String;}}
macro_rules! ints{{ ($($t:ty),*) => {{ $(impl E for $t{{fn e(&self)->String{{format!("[\\\"int\\\",\\\"{{}}\\\",\\\"{{}}\\\"]",std::any::type_name::<$t>(),self)}}}})* }}; }}
ints!(u8,u16,u32,u64,u128,usize,i8,i16,i32,i64,i128,isize);
impl E for bool{{fn e(&self)->String{{format!("[\\\"bool\\\",{{}}]",self)}}}}
impl E for char{{fn e(&self)->String{{format!("[\\\"char\\\",\\\"{{}}\\\",{{}}]",self,*self as u32)}}}}
impl E for f32{{fn e(&self)->String{{format!("[\\\"float\\\",\\\"f32\\\",\\\"{{:?}}\\\",\\\"{{:08x}}\\\"]",self,self.to_bits())}}}}
impl E for f64{{fn e(&self)->String{{format!("[\\\"float\\\",\\\"f64\\\",\\\"{{:?}}\\\",\\\"{{:016x}}\\\"]",self,self.to_bits())}}}}
impl E for &str{{fn e(&self)->String{{format!("[\\\"str\\\",\\\"{{}}\\\",\\\"{{}}\\\"]",self,hx(self.as_bytes()))}}}}
impl<const N:usize> E for &[u8;N]{{fn e(&self)->String{{format!("[\\\"bytes\\\",\\\"[u8; {{}}]\\\",\\\"{{}}\\\"]",N,hx(&self[..]))}}}}
fn main(){{ {binding} println!("{{}}",E::e(&{use})); }}
'''

def compile_one(i, case, const):
    typ,expr=case.get('type'),case['src']
    if not typ: return None, [['unknown','', 'type inference required']]
    src=rust_src(typ,expr,const); base=BUILD/(f'{i}_'+('const' if const else 'eval'))
    rs=base.with_suffix('.rs'); bin=base
    rs.write_text(src)
    p=subprocess.run(['rustc','+'+TOOLCHAIN,'--edition',EDITION,'--error-format=json','-o',str(bin),str(rs)],capture_output=True,text=True)
    ds=diag(p.stderr)
    if p.returncode: return None, ds
    q=subprocess.run([str(bin)],capture_output=True,text=True)
    if q.returncode: return None, [['error','','program failed']]
    try: return json.loads(q.stdout.strip()), ds
    except ValueError: return None, [['error','','invalid recorder output']]

def main():
    check_pins(); BUILD.mkdir(parents=True,exist_ok=True)
    cases=json.loads((HERE/'rust.cases.json').read_text())['cases']; out=[]
    for i,c in enumerate(cases):
        ent={'src':c['src'],'type':c.get('type')}
        if 'unknown' in c: ent['unknown']=c['unknown']
        cv,cd=compile_one(i,c,True); ev,ed=compile_one(i,c,False)
        ent['const']=cv if cv is not None else (['error',cd[0][1],cd[0][2]] if cd and cd[0][0]=='error' else ['unknown',cd[0][2] if cd else ''])
        ent['eval']=ev if ev is not None else (['error',ed[0][1],ed[0][2]] if ed and ed[0][0]=='error' else ['unknown',ed[0][2] if ed else ''])
        ent['warnings']=[d for d in cd+ed if d[0] not in ('error','unknown')]
        out.append(ent)
    record={'tool':'rustc','version':RUSTC_VERSION,'edition':EDITION,'cases':out}
    (HERE/'rust.expected.json').write_text(json.dumps(record,indent=1,ensure_ascii=False)+'\n')
    okc=sum(not isinstance(x['const'],list) or x['const'][0]!='error' for x in out)
    errc=len(out)-okc; oke=sum(not isinstance(x['eval'],list) or x['eval'][0]!='error' for x in out)
    print(f'cases: {len(out)}; const ok: {okc}; const error: {errc}; eval ok: {oke}')
if __name__=='__main__': main()
