//! A clause's ast-grep pattern leaves as one generated Metal kernel (06-first-slice S2, F1): each
//! `PackedPattern` becomes functions that mirror `PackedPattern::one` and `children` line for
//! line. A pattern is a tree, so each function calls only its children's and none recurses; the
//! structural equality of a repeated metavariable (`exact`) walks an explicit stack, and one too
//! deep for it fails the clause (flags[0]) rather than guessing.

use super::super::super::PackedPattern;
use std::fmt::Write as _;

pub(super) const KERNEL: &str = "clause_patterns";

const COMMON: &str = r#"#include <metal_stdlib>
using namespace metal;
struct Node { uint parent,nextAny,nextNamed,childStart,childCount,descendantEnd,sourceStart,sourceEnd,kind,field,flags; };
// A binding: mode 1 one node `a`, mode 2 the `n` children from offset `a` of the children table.
struct Var { uint mode,a,n,multi; };
struct Ctx { device const Node* nodes; device const uint* children; device const uchar* source; constant uchar* text; device atomic_uint* flags; };
struct Item { uint pattern,target,root,constraintStart,constraintCount; };
struct Shape { uint nodeCount,words,resultArena,taskCount,itemCount,targetCount,round,finish; };
static bool kinds_match(uint goal,uint kind){return goal==kind||goal==65535u;}
static bool named_leaf(Node n){return (n.flags&1u)!=0u&&n.childCount==0u;}
static bool same_text(Ctx c,uint left,uint right){Node l=c.nodes[left],r=c.nodes[right];uint n=l.sourceEnd-l.sourceStart;if(n!=r.sourceEnd-r.sourceStart)return false;for(uint i=0u;i<n;i++)if(c.source[l.sourceStart+i]!=c.source[r.sourceStart+i])return false;return true;}
static bool text_is(Ctx c,uint node,uint start,uint length){Node n=c.nodes[node];if(n.sourceEnd-n.sourceStart!=length)return false;for(uint i=0u;i<length;i++)if(c.source[n.sourceStart+i]!=c.text[start+i])return false;return true;}
static bool exact(Ctx c,uint left,uint right){uint2 stack[64];uint top=0u;stack[top++]=uint2(left,right);while(top>0u){uint2 pair=stack[--top];if(pair.x==pair.y)continue;Node l=c.nodes[pair.x],r=c.nodes[pair.y];if(named_leaf(l)||named_leaf(r)){if(!same_text(c,pair.x,pair.y))return false;continue;}if(l.kind!=r.kind||l.childCount!=r.childCount)return false;if(top+l.childCount>64u){atomic_fetch_or_explicit(&c.flags[0],2u,memory_order_relaxed);return false;}for(uint i=0u;i<l.childCount;i++)stack[top++]=uint2(c.children[l.childStart+i],c.children[r.childStart+i]);}return true;}
static uint var_at(Ctx c,Var v,uint i){return v.mode==1u?v.a:c.children[v.a+i];}
static bool bind_single(Ctx c,thread Var& v,uint node){if(v.mode!=0u)return v.n==1u&&exact(c,var_at(c,v,0u),node);v.mode=1u;v.a=node;v.n=1u;v.multi=0u;return true;}
static bool bind_multi(Ctx c,thread Var& v,uint mode,uint a,uint n){
  if(v.mode==0u){v.mode=mode;v.a=a;v.n=n;v.multi=1u;return true;}
  Var w;w.mode=mode;w.a=a;w.n=n;w.multi=1u;uint i=0u,j=0u;
  while(true){while(i<v.n&&(c.nodes[var_at(c,v,i)].flags&1u)==0u)i++;while(j<w.n&&(c.nodes[var_at(c,w,j)].flags&1u)==0u)j++;if(i==v.n||j==w.n)return i==v.n&&j==w.n;if(!exact(c,var_at(c,v,i),var_at(c,w,j)))return false;i++;j++;}
}
"#;

/// The kernel source for `patterns`, with each pattern's metavariable names in index order.
pub(super) fn source(patterns: &[&PackedPattern]) -> (String, Vec<Vec<String>>) {
    let mut out = COMMON.to_owned();
    let mut text = Vec::<u8>::new();
    let mut names = Vec::new();
    let mut bodies = String::new();
    for (index, pattern) in patterns.iter().enumerate() {
        let mut vars = Vec::new();
        collect(pattern, &mut vars);
        let count = vars.len().max(1);
        let _ = writeln!(bodies, "struct B{index} {{ Var v[{count}]; }};");
        let mut next = 0;
        let root = emit(pattern, index, &vars, &mut next, &mut text, &mut bodies);
        let _ = writeln!(bodies, "static int p{index}_root(Ctx c,uint node,thread B{index}& b){{return p{index}_{root}(c,node,b,true);}}");
        names.push(vars);
    }
    let pool = if text.is_empty() { vec![0] } else { text };
    let _ = writeln!(
        out,
        "constant uchar TEXT[{}] = {{{}}};",
        pool.len(),
        pool.iter().map(|byte| byte.to_string()).collect::<Vec<_>>().join(",")
    );
    out.push_str(&bodies);
    let _ = write!(
        out,
        "kernel void {KERNEL}(device const Node* nodes [[buffer(0)]],device const uint* children [[buffer(1)]],device const uchar* source [[buffer(2)]],device const uint* labels [[buffer(3)]],device atomic_uint* scratch [[buffer(4)]],device atomic_uint* flags [[buffer(5)]],device const Item* items [[buffer(6)]],device const uint2* constraints [[buffer(7)]],constant Shape& shape [[buffer(8)]],uint tid [[thread_position_in_grid]]){{\n  if(tid>=shape.itemCount||atomic_load_explicit(&flags[2],memory_order_relaxed)!=0u)return;\n  Item item=items[tid];Ctx c;c.nodes=nodes;c.children=children;c.source=source;c.text=TEXT;c.flags=flags;\n  switch(item.pattern){{\n"
    );
    for (index, vars) in names.iter().enumerate() {
        let _ = write!(
            out,
            "  case {index}u:{{B{index} b;for(uint v=0u;v<{};v++){{b.v[v].mode=0u;b.v[v].a=0u;b.v[v].n=0u;b.v[v].multi=0u;}}if(p{index}_root(c,item.root,b)!=0)return;for(uint at=0u;at<item.constraintCount;at++){{uint2 constraint=constraints[item.constraintStart+at];Var v=b.v[constraint.x];if(v.mode!=0u&&v.multi==0u){{uint node=var_at(c,v,0u);if(((labels[constraint.y*shape.words+node/32u]>>(node%32u))&1u)==0u)return;}}}}break;}}\n",
            vars.len().max(1)
        );
    }
    out.push_str("  default:return;\n  }\n  atomic_fetch_or_explicit(&scratch[item.target*shape.words+item.root/32u],1u<<(item.root%32u),memory_order_relaxed);\n}\n");
    (out, names)
}

fn collect(pattern: &PackedPattern, vars: &mut Vec<String>) {
    match pattern {
        PackedPattern::Single { name: Some(name), .. } | PackedPattern::Ellipsis(Some(name)) => {
            if !vars.contains(name) {
                vars.push(name.clone());
            }
        }
        PackedPattern::Internal { children, .. } => children.iter().for_each(|child| collect(child, vars)),
        _ => {}
    }
}

fn var(vars: &[String], name: &Option<String>) -> String {
    name.as_ref()
        .and_then(|name| vars.iter().position(|known| known == name))
        .map_or_else(|| "0xffffffffu".to_owned(), |at| format!("{at}u"))
}

/// Emits `pattern`'s function, its children's first, and returns its number.
fn emit(
    pattern: &PackedPattern,
    index: usize,
    vars: &[String],
    next: &mut usize,
    text: &mut Vec<u8>,
    out: &mut String,
) -> usize {
    let head = |id: usize| format!("static int p{index}_{id}(Ctx c,uint node,thread B{index}& b,bool root){{Node n=c.nodes[node];");
    match pattern {
        PackedPattern::Terminal { text: literal, named, kind } => {
            let id = *next;
            *next += 1;
            let start = text.len();
            text.extend_from_slice(literal.as_bytes());
            let _ = writeln!(
                out,
                "{}if(kinds_match({kind}u,n.kind)&&({}||text_is(c,node,{start}u,{}u)))return 0;if((n.flags&2u)!=0u||(n.flags&1u)==0u)return 3;return 4;}}",
                head(id),
                !named,
                literal.len()
            );
            id
        }
        PackedPattern::Single { name, named } => {
            let id = *next;
            *next += 1;
            let bind = match name {
                None => "return 0;".to_owned(),
                Some(_) => format!("return bind_single(c,b.v[{}],node)?0:4;", var(vars, name)),
            };
            let _ = writeln!(
                out,
                "{}if(!root&&(n.flags&2u)!=0u)return 3;if({named}&&(n.flags&1u)==0u)return 4;{bind}}}",
                head(id)
            );
            id
        }
        PackedPattern::Ellipsis(name) => {
            let id = *next;
            *next += 1;
            let bind = match name {
                None => "return 0;".to_owned(),
                Some(_) => format!("return bind_multi(c,b.v[{}],1u,node,1u)?0:4;", var(vars, name)),
            };
            let _ = writeln!(out, "{}if(!root)return 1;{bind}}}", head(id));
            id
        }
        PackedPattern::Internal { kind, children } => {
            let goals = children
                .iter()
                .map(|child| emit(child, index, vars, next, text, out))
                .collect::<Vec<_>>();
            let id = *next;
            *next += 1;
            let count = goals.len();
            if count == 0 {
                // No goals: `children` holds wherever there are candidates.
                let _ = writeln!(
                    out,
                    "{}if(!kinds_match({kind}u,n.kind))return 4;return n.childCount!=0u?0:4;}}",
                    head(id)
                );
                return id;
            }
            let ellipsis = children
                .iter()
                .map(|child| match child {
                    PackedPattern::Ellipsis(name) => var(vars, name),
                    _ => "0xfffffffeu".to_owned(),
                })
                .collect::<Vec<_>>();
            let trivial = children
                .iter()
                .map(|child| if child.is_trivial() { "true" } else { "false" })
                .collect::<Vec<_>>();
            // Goal tables: an ellipsis goal's variable (0xffffffff unnamed, 0xfffffffe not an
            // ellipsis) and whether a goal is trivial.
            let _ = writeln!(
                out,
                "constant uint E{index}_{id}[{count}]={{{}}};constant bool T{index}_{id}[{count}]={{{}}};",
                ellipsis.join(","),
                trivial.join(",")
            );
            let mut one = format!("static int o{index}_{id}(Ctx c,uint goal,uint node,thread B{index}& b){{switch(goal){{");
            for (goal, child) in goals.iter().enumerate() {
                let _ = write!(one, "case {goal}u:return p{index}_{child}(c,node,b,false);");
            }
            one.push_str("default:return 4;}}");
            let _ = writeln!(out, "{one}");
            let e = format!("E{index}_{id}");
            let t = format!("T{index}_{id}");
            let o = format!("o{index}_{id}");
            let _ = writeln!(
                out,
                r#"static bool c{index}_{id}(Ctx c,uint base,uint count,thread B{index}& b){{
  if(count==0u)return false;const uint G={count}u;uint goal=0u,candidate=0u;
  while(goal<G){{
    if({e}[goal]!=0xfffffffeu){{
      uint name={e}[goal];goal++;
      if(goal==G)return name==0xffffffffu||bind_multi(c,b.v[name&0xffu],2u,base+candidate,count-candidate);
      uint skipped=0u;while(goal<G&&{t}[goal]){{goal++;skipped++;}}
      if(goal==G){{uint end=max(count>skipped?count-skipped:0u,candidate);return name==0xffffffffu||bind_multi(c,b.v[name&0xffu],2u,base+candidate,end-candidate);}}
      if({e}[goal]!=0xfffffffeu){{if(candidate==count)return false;uint captured=c.children[base+candidate];candidate++;if(candidate==count)return false;if(name!=0xffffffffu&&!bind_multi(c,b.v[name&0xffu],1u,captured,1u))return false;continue;}}
      uint start=candidate;
      while(true){{if(candidate>=count)return false;uint node=c.children[base+candidate];B{index} probe=b;if({o}(c,goal,node,probe)==0){{uint end=max(candidate>=skipped?candidate-skipped:0u,start);if(name!=0xffffffffu&&!bind_multi(c,b.v[name&0xffu],2u,base+start,end-start))return false;break;}}candidate++;}}
    }}
    while(true){{
      if(candidate>=count){{for(uint g=goal;g<G;g++)if({e}[g]==0xfffffffeu)return false;return true;}}
      int verdict={o}(c,goal,c.children[base+candidate],b);
      if(verdict==0)break;
      if(verdict==1){{goal++;if(goal==G)return true;}}
      else if(verdict==2){{goal++;candidate++;if(goal==G)return true;}}
      else if(verdict==3)candidate++;
      else return false;
    }}
    goal++;candidate++;if(goal==G)return true;
  }}
  return true;
}}"#
            );
            let _ = writeln!(
                out,
                "{}if(!kinds_match({kind}u,n.kind))return 4;return c{index}_{id}(c,n.childStart,n.childCount,b)?0:4;}}",
                head(id)
            );
            id
        }
    }
}
