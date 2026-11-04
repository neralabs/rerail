//! A clause's ast-grep pattern leaves as one generated Metal kernel (06-first-slice S2, F1): each
//! `PackedPattern` becomes functions that mirror `PackedPattern::one` and `children` line for
//! line. A pattern is a tree, so each function calls only its children's and none recurses; the
//! structural equality of a repeated metavariable (`exact`) compares the two subtrees' preorder
//! runs in the tape, with no stack.

use super::super::super::PackedPattern;
use std::fmt::Write as _;

pub(super) const KERNEL: &str = "clause_patterns";
/// The item pattern of a fact's root (`Pass::Fact`): its constraints are (target node, label) pairs.
pub(super) const FACT: u32 = u32::MAX;

const COMMON: &str = r#"#include <metal_stdlib>
using namespace metal;
// A binding: mode 1 one node `a`, mode 2 the `n` children of node `a` from its child `from`.
struct Var { uint mode,a,from,n,multi; };
// The item's document (decision 0032): nodes are the batch's numbering, read at `node - d.nodeBase`.
struct Ctx { Slabs s; DocView d; constant uchar* text; device atomic_uint* flags; };
// A pattern pass (its roots: segments at `root` in `segs`, decision 0033) or a fact's root (`root`,
// one thread); its threads from `threadStart`.
struct Item { uint pattern,target,root,constraintStart,constraintCount,threadStart; };
struct Shape { uint nodeCount,words,resultArena,taskCount,itemCount,targetCount,round,finish,recursive,lastLevel,fileCount,steps; };
inline uint c_local(Ctx c,uint node){return node-c.d.nodeBase;}
inline uint c_global(Ctx c,uint local){return local==TAPE_NONE?TAPE_NONE:local+c.d.nodeBase;}
inline uint c_kind(Ctx c,uint node){return tape_kind(c.s,c.d,c_local(c,node));}
inline uint c_flags(Ctx c,uint node){return tape_flags(c.s,c.d,c_local(c,node));}
inline uint c_count(Ctx c,uint node){return tape_child_count(c.s,c.d,c_local(c,node));}
inline uint c_first(Ctx c,uint node){return c_global(c,tape_first_child(c.s,c.d,c_local(c,node)));}
inline uint c_next(Ctx c,uint node){return c_global(c,tape_next(c.s,c.d,c_local(c,node)));}
inline uint c_child(Ctx c,uint node,uint k){return c_global(c,tape_child(c.s,c.d,c_local(c,node),k));}
// A parent's children read in order: the last one read is kept, so a scan hops once a child.
struct Kids { uint parent,at,node; };
static uint kid(Ctx c,thread Kids& k,uint i){if(k.at==0xffffffffu||i<k.at){k.at=0u;k.node=c_first(c,k.parent);}while(k.at<i&&k.node!=TAPE_NONE){k.node=c_next(c,k.node);k.at++;}return k.node;}
static bool kinds_match(uint goal,uint kind){return goal==kind||goal==65535u;}
static bool named_leaf(Ctx c,uint node){return (c_flags(c,node)&1u)!=0u&&tape_size(c.s,c.d,c_local(c,node))==1u;}
static bool same_text(Ctx c,uint left,uint right){uint l=c_local(c,left),r=c_local(c,right);uint ls=tape_start(c.s,c.d,l),rs=tape_start(c.s,c.d,r);uint n=tape_end(c.s,c.d,l)-ls;if(n!=tape_end(c.s,c.d,r)-rs)return false;device const uchar* source=tape_source(c.s,c.d);for(uint i=0u;i<n;i++)if(source[ls+i]!=source[rs+i])return false;return true;}
static bool text_is(Ctx c,uint node,uint start,uint length){uint l=c_local(c,node);uint from=tape_start(c.s,c.d,l);if(tape_end(c.s,c.d,l)-from!=length)return false;device const uchar* source=tape_source(c.s,c.d);for(uint i=0u;i<length;i++)if(source[from+i]!=c.text[start+i])return false;return true;}
// Two subtrees are equal where their preorder runs are (a subtree is contiguous in the tape):
// pair by pair the same kind and child count, the same text at a named leaf, a node shared by
// both skipped whole. No stack: thread memory is the driver's scratch for every thread in flight.
static bool exact(Ctx c,uint left,uint right){uint a=c_local(c,left),b=c_local(c,right);uint aEnd=a+tape_size(c.s,c.d,a),bEnd=b+tape_size(c.s,c.d,b);while(a<aEnd&&b<bEnd){if(a==b){uint n=tape_size(c.s,c.d,a);a+=n;b+=n;continue;}uint x=c_global(c,a),y=c_global(c,b);if(named_leaf(c,x)||named_leaf(c,y)){if(!same_text(c,x,y))return false;a+=tape_size(c.s,c.d,a);b+=tape_size(c.s,c.d,b);continue;}if(c_kind(c,x)!=c_kind(c,y)||c_count(c,x)!=c_count(c,y))return false;a++;b++;}return a==aEnd&&b==bEnd;}
static uint var_at(Ctx c,Var v,uint i){return v.mode==1u?v.a:c_child(c,v.a,v.from+i);}
static bool bind_single(Ctx c,thread Var& v,uint node){if(v.mode!=0u)return v.n==1u&&exact(c,var_at(c,v,0u),node);v.mode=1u;v.a=node;v.from=0u;v.n=1u;v.multi=0u;return true;}
static bool bind_multi(Ctx c,thread Var& v,uint mode,uint a,uint from,uint n){
  if(v.mode==0u){v.mode=mode;v.a=a;v.from=from;v.n=n;v.multi=1u;return true;}
  Var w;w.mode=mode;w.a=a;w.from=from;w.n=n;w.multi=1u;uint i=0u,j=0u;
  while(true){while(i<v.n&&(c_flags(c,var_at(c,v,i))&1u)==0u)i++;while(j<w.n&&(c_flags(c,var_at(c,w,j))&1u)==0u)j++;if(i==v.n||j==w.n)return i==v.n&&j==w.n;if(!exact(c,var_at(c,v,i),var_at(c,w,j)))return false;i++;j++;}
}
static DocView doc_of(device const DocView* docs,uint count,uint node){uint first=0u,last=count;while(last-first>1u){uint middle=(first+last)/2u;if(docs[middle].nodeBase<=node)first=middle;else last=middle;}return docs[first];}
"#;

/// Patterns a pattern kernel holds (`source`). A kernel's thread memory is the driver's scratch,
/// sized for every thread in flight and kept by the process, and one kernel of every clause's
/// patterns spills: on S7's 59, eight cost no scratch, thirty 16-48 MB, all 59 88 MB (O39).
pub(super) const GROUP: usize = 8;

/// The kernel source for `patterns`, numbered from `first` (the plan's numbering, which items
/// carry), with each pattern's metavariable names in index order. Every group's kernel answers
/// the fact case.
pub(super) fn source(patterns: &[&PackedPattern], first: usize) -> (String, Vec<Vec<String>>) {
    let mut out = format!("{}{COMMON}", super::TAPE_VIEW);
    let mut text = Vec::<u8>::new();
    let mut names = Vec::new();
    let mut bodies = String::new();
    for (index, pattern) in patterns.iter().enumerate() {
        let index = first + index;
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
        "kernel void {KERNEL}(device const DocView* docs [[buffer(0)]],device const uint* labels [[buffer(1)]],device atomic_uint* scratch [[buffer(2)]],device atomic_uint* flags [[buffer(3)]],device const Item* items [[buffer(4)]],device const uint2* constraints [[buffer(5)]],device const uint4* segs [[buffer(6)]],constant Shape& shape [[buffer(7)]],device const uchar* s0 [[buffer(27)]],device const uchar* s1 [[buffer(28)]],device const uchar* s2 [[buffer(29)]],device const uchar* s3 [[buffer(30)]],uint tid [[thread_position_in_grid]]){{\n  if(tid>=shape.itemCount||atomic_load_explicit(&flags[2],memory_order_relaxed)!=0u)return;\n  uint low=0u,high=shape.taskCount;while(high-low>1u){{uint middle=(low+high)/2u;if(items[middle].threadStart<=tid)low=middle;else high=middle;}}\n  Item item=items[low];Ctx c;c.s=Slabs{{s0,s1,s2,s3}};c.text=TEXT;c.flags=flags;\n  if(item.pattern=={FACT}u)c.d=doc_of(docs,shape.fileCount,item.root);\n  else{{uint i=tid-item.threadStart;uint4 seg=segs[root_segment(segs,item.root,i)];c.d=docs[seg.x];item.root=c.d.nodeBase+root_local(c.s,c.d,segs,seg,i-seg.w);}}\n  switch(item.pattern){{\n"
    );
    for (index, vars) in names.iter().enumerate() {
        let index = first + index;
        let _ = write!(
            out,
            "  case {index}u:{{B{index} b;for(uint v=0u;v<{};v++){{b.v[v].mode=0u;b.v[v].a=0u;b.v[v].from=0u;b.v[v].n=0u;b.v[v].multi=0u;}}if(p{index}_root(c,item.root,b)!=0)return;for(uint at=0u;at<item.constraintCount;at++){{uint2 constraint=constraints[item.constraintStart+at];Var v=b.v[constraint.x];if(v.mode!=0u&&v.multi==0u){{uint node=var_at(c,v,0u);if(((labels[constraint.y*shape.words+node/32u]>>(node%32u))&1u)==0u)return;}}}}break;}}\n",
            vars.len().max(1)
        );
    }
    let _ = writeln!(
        out,
        "  case {FACT}u:{{for(uint at=0u;at<item.constraintCount;at++){{uint2 constraint=constraints[item.constraintStart+at];uint node=constraint.x;if(((labels[constraint.y*shape.words+node/32u]>>(node%32u))&1u)==0u)return;}}break;}}"
    );
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
    let head = |id: usize| format!("static int p{index}_{id}(Ctx c,uint node,thread B{index}& b,bool root){{uint kind=c_kind(c,node),flags=c_flags(c,node);");
    match pattern {
        PackedPattern::Terminal { text: literal, named, kind } => {
            let id = *next;
            *next += 1;
            let start = text.len();
            text.extend_from_slice(literal.as_bytes());
            let _ = writeln!(
                out,
                "{}if(kinds_match({kind}u,kind)&&({}||text_is(c,node,{start}u,{}u)))return 0;if((flags&2u)!=0u||(flags&1u)==0u)return 3;return 4;}}",
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
                "{}if(!root&&(flags&2u)!=0u)return 3;if({named}&&(flags&1u)==0u)return 4;{bind}}}",
                head(id)
            );
            id
        }
        PackedPattern::Ellipsis(name) => {
            let id = *next;
            *next += 1;
            let bind = match name {
                None => "return 0;".to_owned(),
                Some(_) => format!("return bind_multi(c,b.v[{}],1u,node,0u,1u)?0:4;", var(vars, name)),
            };
            let _ = writeln!(out, "{}(void)kind;(void)flags;if(!root)return 1;{bind}}}", head(id));
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
                    "{}(void)flags;if(!kinds_match({kind}u,kind))return 4;return c_first(c,node)!=TAPE_NONE?0:4;}}",
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
  if(count==0u)return false;const uint G={count}u;uint goal=0u,candidate=0u;Kids kids={{base,0xffffffffu,TAPE_NONE}};
  while(goal<G){{
    if({e}[goal]!=0xfffffffeu){{
      uint name={e}[goal];goal++;
      if(goal==G)return name==0xffffffffu||bind_multi(c,b.v[name&0xffu],2u,base,candidate,count-candidate);
      uint skipped=0u;while(goal<G&&{t}[goal]){{goal++;skipped++;}}
      if(goal==G){{uint end=max(count>skipped?count-skipped:0u,candidate);return name==0xffffffffu||bind_multi(c,b.v[name&0xffu],2u,base,candidate,end-candidate);}}
      if({e}[goal]!=0xfffffffeu){{if(candidate==count)return false;uint captured=kid(c,kids,candidate);candidate++;if(candidate==count)return false;if(name!=0xffffffffu&&!bind_multi(c,b.v[name&0xffu],1u,captured,0u,1u))return false;continue;}}
      uint start=candidate;
      while(true){{if(candidate>=count)return false;uint node=kid(c,kids,candidate);B{index} probe=b;if({o}(c,goal,node,probe)==0){{uint end=max(candidate>=skipped?candidate-skipped:0u,start);if(name!=0xffffffffu&&!bind_multi(c,b.v[name&0xffu],2u,base,start,end-start))return false;break;}}candidate++;}}
    }}
    while(true){{
      if(candidate>=count){{for(uint g=goal;g<G;g++)if({e}[g]==0xfffffffeu)return false;return true;}}
      int verdict={o}(c,goal,kid(c,kids,candidate),b);
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
                "{}(void)flags;if(!kinds_match({kind}u,kind))return 4;return c{index}_{id}(c,node,c_count(c,node),b)?0:4;}}",
                head(id)
            );
            id
        }
    }
}
