//! A clause's ast-grep pattern leaves as one generated kernel, in UKL (decision 0440) (06-first-slice S2, F1): each
//! `PackedPattern` becomes functions that mirror `PackedPattern::one` and `children` line for
//! line. A pattern is a tree, so each function calls only its children's and none recurses; the
//! structural equality of a repeated metavariable (`exact`) compares the two subtrees' preorder
//! runs in the tape, with no stack.

use super::super::super::PackedPattern;
use std::fmt::Write as _;

pub(super) const KERNEL: &str = "clause_patterns";
/// The item pattern of a fact's findings on one file (`Pass::Fact`), read in the fact block (`facts`, Q289): its first
/// constraint is (file, whether the findings are bits), then a (0, label) per slot; `root` is the findings' first u32.
pub(super) const FACT: u32 = u32::MAX;
/// The threads a pattern pass's item (a group of 64 nodes) takes, each finding the roots among 16 of them; a fact's
/// item keeps one thread for its one root, so no lane is a fact's idle copy. A thread a group gave the kernel a
/// quarter of these threads: at s1000, 1, 4, 8 and 32 lanes a group took 3.6–6.8, 2.1–2.5, 2.8–2.9 and 2.7–2.8 ms
/// (the hub engine's thread a listed root, 3.0–3.2; decision 0359).
pub(super) const LANES: usize = 4;

const COMMON: &str = r#"
// A binding: mode 1 one node `a`, mode 2 the `n` children of node `a` from its child `from`.
typedef struct Var { uint32_t mode,a,from,n,multi; } Var;
// The item's document (decision 0032): nodes are the batch's numbering, read at `node - d.node_base`.
typedef struct Ctx { UKL_GLOBAL const uint8_t* s; DocView d; UKL_TABLE_SPACE const uint8_t* text; } Ctx;
// A pattern pass (its roots: its rule at `root` in `roots`, `LANES` threads a group of 64 nodes) or a fact's
// findings on a file (at `root` in `facts`: a thread a row, or a word of its bits); its threads from `threadStart`.
typedef struct Item { uint32_t pattern,target,root,constraintStart,constraintCount,threadStart; } Item;
// A constraint: a variable and the label it must hold; a fact item's first is (its file, whether its findings are
// bits), then a (0, label) a slot.
typedef struct Pair { uint32_t x,y; } Pair;
// The label passes' shape (`label_passes.uglykl`'s), of which these kernels read the first fields.
typedef struct Shape { uint32_t nodeCount,words,resultArena,taskCount,itemCount,targetCount,round,finish,recursive,lastLevel,fileCount,steps,profile,queue,budget,lanes,frames; } Shape;
UKL_REFLECT_STRUCT(Shape, nodeCount,words,resultArena,taskCount,itemCount,targetCount,round,finish,recursive,lastLevel,fileCount,steps,profile,queue,budget,lanes,frames)
UKL_INLINE uint32_t c_local(Ctx c,uint32_t node){return node-c.d.node_base;}
UKL_INLINE uint32_t c_global(Ctx c,uint32_t at){return at==TAPE_NONE?TAPE_NONE:at+c.d.node_base;}
UKL_INLINE uint32_t c_kind(Ctx c,uint32_t node){return tape_kind(c.s,c.d,c_local(c,node));}
UKL_INLINE uint32_t c_flags(Ctx c,uint32_t node){return tape_flags(c.s,c.d,c_local(c,node));}
UKL_INLINE uint32_t c_count(Ctx c,uint32_t node){return tape_child_count(c.s,c.d,c_local(c,node));}
UKL_INLINE uint32_t c_first(Ctx c,uint32_t node){return c_global(c,tape_first_child(c.s,c.d,c_local(c,node)));}
UKL_INLINE uint32_t c_next(Ctx c,uint32_t node){return c_global(c,tape_next(c.s,c.d,c_local(c,node)));}
UKL_INLINE uint32_t c_child(Ctx c,uint32_t node,uint32_t k){return c_global(c,tape_child(c.s,c.d,c_local(c,node),k));}
// A parent's children read in order: the last one read is kept, so a scan hops once a child.
typedef struct Kids { uint32_t parent,at,node; } Kids;
UKL_INLINE uint32_t kid(Ctx c,UKL_PRIVATE Kids* k,uint32_t i){if(k->at==0xffffffffu||i<k->at){k->at=0u;k->node=c_first(c,k->parent);}while(k->at<i&&k->node!=TAPE_NONE){k->node=c_next(c,k->node);k->at++;}return k->node;}
UKL_INLINE bool kinds_match(uint32_t goal,uint32_t kind){return goal==kind||goal==65535u;}
UKL_INLINE bool named_leaf(Ctx c,uint32_t node){return (c_flags(c,node)&1u)!=0u&&tape_size(c.s,c.d,c_local(c,node))==1u;}
UKL_INLINE bool same_text(Ctx c,uint32_t left,uint32_t right){uint32_t l=c_local(c,left),r=c_local(c,right);uint32_t ls=tape_start(c.s,c.d,l),rs=tape_start(c.s,c.d,r);uint32_t n=tape_end(c.s,c.d,l)-ls;if(n!=tape_end(c.s,c.d,r)-rs)return false;UKL_GLOBAL const uint8_t* source=tape_source(c.s,c.d);for(uint32_t i=0u;i<n;i++)if(source[ls+i]!=source[rs+i])return false;return true;}
UKL_INLINE bool text_is(Ctx c,uint32_t node,uint32_t start,uint32_t length){uint32_t l=c_local(c,node);uint32_t from=tape_start(c.s,c.d,l);if(tape_end(c.s,c.d,l)-from!=length)return false;UKL_GLOBAL const uint8_t* source=tape_source(c.s,c.d);for(uint32_t i=0u;i<length;i++)if(source[from+i]!=c.text[start+i])return false;return true;}
// Two subtrees are equal where their preorder runs are (a subtree is contiguous in the tape):
// pair by pair the same kind and child count, the same text at a named leaf, a node shared by
// both skipped whole. No stack: thread memory is the driver's scratch for every thread in flight.
UKL_INLINE bool exact(Ctx c,uint32_t left,uint32_t right){uint32_t a=c_local(c,left),b=c_local(c,right);uint32_t aEnd=a+tape_size(c.s,c.d,a),bEnd=b+tape_size(c.s,c.d,b);while(a<aEnd&&b<bEnd){if(a==b){uint32_t n=tape_size(c.s,c.d,a);a+=n;b+=n;continue;}uint32_t x=c_global(c,a),y=c_global(c,b);if(named_leaf(c,x)||named_leaf(c,y)){if(!same_text(c,x,y))return false;a+=tape_size(c.s,c.d,a);b+=tape_size(c.s,c.d,b);continue;}if(c_kind(c,x)!=c_kind(c,y)||c_count(c,x)!=c_count(c,y))return false;a++;b++;}return a==aEnd&&b==bEnd;}
UKL_INLINE uint32_t var_at(Ctx c,Var v,uint32_t i){return v.mode==1u?v.a:c_child(c,v.a,v.from+i);}
UKL_INLINE bool bind_single(Ctx c,UKL_PRIVATE Var* v,uint32_t node){if(v->mode!=0u)return v->n==1u&&exact(c,var_at(c,*v,0u),node);v->mode=1u;v->a=node;v->from=0u;v->n=1u;v->multi=0u;return true;}
UKL_INLINE bool bind_multi(Ctx c,UKL_PRIVATE Var* v,uint32_t mode,uint32_t a,uint32_t from,uint32_t n){
  if(v->mode==0u){v->mode=mode;v->a=a;v->from=from;v->n=n;v->multi=1u;return true;}
  Var w;w.mode=mode;w.a=a;w.from=from;w.n=n;w.multi=1u;uint32_t i=0u,j=0u;
  while(true){while(i<v->n&&(c_flags(c,var_at(c,*v,i))&1u)==0u)i++;while(j<w.n&&(c_flags(c,var_at(c,w,j))&1u)==0u)j++;if(i==v->n||j==w.n)return i==v->n&&j==w.n;if(!exact(c,var_at(c,*v,i),var_at(c,w,j)))return false;i++;j++;}
}
"#;

/// Patterns a pattern kernel holds (`source`). A kernel's thread memory is the driver's scratch,
/// sized for every thread in flight and kept by the process, and one kernel of every clause's
/// patterns spills: on S7's 59, eight cost no scratch, thirty 16-48 MB, all 59 88 MB (O39).
pub(super) const GROUP: usize = 8;

/// The kernel source for `patterns`, numbered from `first` (the plan's numbering, which items
/// carry), with each pattern's metavariable names in index order. Every group's kernel answers
/// the fact case.
pub(super) fn source(patterns: &[&PackedPattern], first: usize, format: u32) -> (String, Vec<Vec<String>>) {
    let mut out = format!("{}{COMMON}", super::tape_view(format));
    let mut text = Vec::<u8>::new();
    let mut names = Vec::new();
    let mut bodies = String::new();
    for (index, pattern) in patterns.iter().enumerate() {
        let index = first + index;
        let mut vars = Vec::new();
        collect(pattern, &mut vars);
        let count = vars.len().max(1);
        let _ = writeln!(bodies, "typedef struct B{index} {{ Var v[{count}]; }} B{index};");
        let mut next = 0;
        let root = emit(pattern, index, &vars, &mut next, &mut text, &mut bodies);
        let _ = writeln!(bodies, "UKL_INLINE int p{index}_root(Ctx c,uint32_t node,UKL_PRIVATE B{index}* b){{return p{index}_{root}(c,node,b,true);}}");
        names.push(vars);
    }
    let pool = if text.is_empty() { vec![0] } else { text };
    let _ = writeln!(
        out,
        "UKL_TABLE const uint8_t TEXT[{}] = {{{}}};",
        pool.len(),
        pool.iter().map(|byte| byte.to_string()).collect::<Vec<_>>().join(",")
    );
    out.push_str(&bodies);
    // Whether the item's pattern matches at `node` (`c.d` its document) with its constraints held.
    let _ = write!(
        out,
        "UKL_INLINE bool item_matches(Ctx c,Item item,uint32_t node,UKL_GLOBAL const uint32_t* labels,UKL_GLOBAL const Pair* constraints,UKL_GLOBAL const uint32_t* facts,uint32_t row,Shape shape){{\n  switch(item.pattern){{\n"
    );
    for (index, vars) in names.iter().enumerate() {
        let index = first + index;
        let _ = write!(
            out,
            "  case {index}u:{{B{index} b;for(uint32_t v=0u;v<{};v++){{b.v[v].mode=0u;b.v[v].a=0u;b.v[v].from=0u;b.v[v].n=0u;b.v[v].multi=0u;}}if(p{index}_root(c,node,&b)!=0)return false;for(uint32_t at=0u;at<item.constraintCount;at++){{Pair constraint=constraints[item.constraintStart+at];Var v=b.v[constraint.x];if(v.mode!=0u&&v.multi==0u){{uint32_t n=var_at(c,v,0u);if(((labels[constraint.y*shape.words+n/32u]>>(n%32u))&1u)==0u)return false;}}}}return true;}}\n",
            vars.len().max(1)
        );
    }
    let _ = writeln!(
        out,
        "  case {FACT}u:{{(void)node;UKL_GLOBAL const uint32_t* targets=facts+item.root+row*(item.constraintCount+1u)+1u;for(uint32_t at=0u;at<item.constraintCount;at++){{uint32_t t=c.d.node_base+targets[at],label=constraints[item.constraintStart+1u+at].y;if(((labels[label*shape.words+t/32u]>>(t%32u))&1u)==0u)return false;}}return true;}}"
    );
    out.push_str("  default:return false;\n  }\n}\n");
    // A pattern pass's item is a group of 64 of the batch's nodes, `LANES` threads, each finding the roots among its
    // share of them (`root_mask_in`); a fact's item is a file's findings, a thread holding a row's node or a word of
    // its bits, read where the host wrote them. A SIMD group tests all its
    // lanes' roots, one a lane a round (`simd_roots`): lanes need not line up with the items.
    let (lanes, per) = (LANES, 64 / LANES);
    let _ = write!(
        out,
        r#"DEFINE_KERNEL2(label_patterns, {KERNEL},
               ROBUFFER, DocView,  docs,        "[shape.fileCount]",
               ROBUFFER, uint32_t, head,        "[2 + labels * shape.words]",
               RWBUFFER, uint32_t, scratch,     "[rows * shape.words]",
               RWBUFFER, uint32_t, flags,       "[6]",
               ROBUFFER, Item,     items,       "[shape.taskCount]",
               ROBUFFER, Pair,     constraints, "[constraints]",
               ROBUFFER, uint32_t, roots,       "[roots]",
               ROBUFFER, uint32_t, facts,       "[facts]",
               ARGS,     Shape,    shape,       ,
               ROBUFFER, uint8_t,  chunk,       "[block]",
               GID0,     uint32_t, tid,         ,
               LANE,     uint32_t, lane,        )
{{
  uint64_t mask=0ul;uint32_t at=0u,file=0u,base=0u,group=0u,row=0u;
  if(tid<shape.itemCount&&ukl_atomic_load(&flags[2])==0u){{
  uint32_t low=0u,high=shape.taskCount;while(high-low>1u){{uint32_t middle=(low+high)/2u;if(items[middle].threadStart<=tid)low=middle;else high=middle;}}
  Item item=items[low];at=low;
  if(item.pattern=={FACT}u){{uint32_t rel=tid-item.threadStart;Pair first=constraints[item.constraintStart];file=first.x;base=docs[file].node_base;if(first.y!=0u){{mask=(uint64_t)facts[item.root+2u*rel]|((uint64_t)facts[item.root+2u*rel+1u]<<32u);base+=rel*64u;}}else{{row=rel;base+=facts[item.root+rel*(item.constraintCount+1u)];mask=1ul;}}}}
  else{{uint32_t rel=tid-item.threadStart,part=rel%{lanes}u;group=rel/{lanes}u;file=roots_file(roots,group);group-=roots_first_group(roots,file);mask=root_mask_in(chunk,docs[file],roots,item.root,file,group,part*{per}u,(part+1u)*{per}u);base=docs[file].node_base;}}
  }}
  SimdRoots r=simd_roots(mask,lane __UKL_ARGS);
  for(uint32_t round=0u;round<r.total;round+=UKL_SIMD_WIDTH){{
    uint32_t i=round+lane;uint16_t owner=(uint16_t)(simd_root_lane(r,i __UKL_ARGS));
    uint32_t lo=ukl_sg_shuffle((uint32_t)(mask),owner __UKL_ARGS),hi=ukl_sg_shuffle((uint32_t)(mask>>32u),owner __UKL_ARGS),from=ukl_sg_shuffle(r.prefix,owner __UKL_ARGS);
    uint32_t a=ukl_sg_shuffle(at,owner __UKL_ARGS),f=ukl_sg_shuffle(file,owner __UKL_ARGS),b=ukl_sg_shuffle(base,owner __UKL_ARGS),g=ukl_sg_shuffle(group,owner __UKL_ARGS),w=ukl_sg_shuffle(row,owner __UKL_ARGS);
    if(i>=r.total)continue;
    Item item=items[a];Ctx c;c.s=chunk;c.d=docs[f];c.text=TEXT;
    uint32_t node=b+g*ROOT_GROUP+mask_nth(lo,hi,i-from);
    if(item_matches(c,item,node,head+2,constraints,facts,w,shape))ukl_atomic_or(&scratch[item.target*shape.words+node/32u],1u<<(node%32u));
  }}
}}
"#
    );
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
    let head = |id: usize| format!("UKL_INLINE int p{index}_{id}(Ctx c,uint32_t node,UKL_PRIVATE B{index}* b,bool root){{uint32_t kind=c_kind(c,node),flags=c_flags(c,node);");
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
                Some(_) => format!("return bind_single(c,&b->v[{}],node)?0:4;", var(vars, name)),
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
                Some(_) => format!("return bind_multi(c,&b->v[{}],1u,node,0u,1u)?0:4;", var(vars, name)),
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
            let ends = children
                .iter()
                .map(|child| if child.is_children_end() { "true" } else { "false" })
                .collect::<Vec<_>>();
            // Goal tables: an ellipsis goal's variable (0xffffffff unnamed, 0xfffffffe not an
            // ellipsis), whether a goal is trivial, and whether it is the end of children (RER-131).
            let _ = writeln!(
                out,
                "UKL_TABLE const uint32_t E{index}_{id}[{count}]={{{}}};UKL_TABLE const uint8_t T{index}_{id}[{count}]={{{}}};UKL_TABLE const uint8_t D{index}_{id}[{count}]={{{}}};",
                ellipsis.join(","),
                trivial.join(","),
                ends.join(",")
            );
            let mut one = format!("UKL_INLINE int o{index}_{id}(Ctx c,uint32_t goal,uint32_t node,UKL_PRIVATE B{index}* b){{switch(goal){{");
            for (goal, child) in goals.iter().enumerate() {
                let _ = write!(one, "case {goal}u:return p{index}_{child}(c,node,b,false);");
            }
            one.push_str("default:return 4;}}");
            let _ = writeln!(out, "{one}");
            let e = format!("E{index}_{id}");
            let t = format!("T{index}_{id}");
            let o = format!("o{index}_{id}");
            let d = format!("D{index}_{id}");
            let _ = writeln!(
                out,
                r#"UKL_INLINE bool c{index}_{id}(Ctx c,uint32_t base,uint32_t count,UKL_PRIVATE B{index}* b){{
  const uint32_t G={count}u;
  if(count==0u){{for(uint32_t g=0u;g<G;g++){{if({d}[g])continue;if({e}[g]==0xfffffffeu)return false;if({e}[g]!=0xffffffffu&&!bind_multi(c,&b->v[{e}[g]&0xffu],2u,base,0u,0u))return false;}}return true;}}
  uint32_t goal=0u,candidate=0u;Kids kids={{base,0xffffffffu,TAPE_NONE}};
  while(goal<G){{
    if({e}[goal]!=0xfffffffeu){{
      uint32_t name={e}[goal];goal++;
      if(goal==G)return name==0xffffffffu||bind_multi(c,&b->v[name&0xffu],2u,base,candidate,count-candidate);
      uint32_t skipped=0u;while(goal<G&&{t}[goal]){{goal++;skipped++;}}
      if(goal==G){{uint32_t end=ukl_max(count>skipped?count-skipped:0u,candidate);return name==0xffffffffu||bind_multi(c,&b->v[name&0xffu],2u,base,candidate,end-candidate);}}
      if({e}[goal]!=0xfffffffeu){{if(candidate==count)return false;uint32_t captured=kid(c,&kids,candidate);candidate++;if(candidate==count)return false;if(name!=0xffffffffu&&!bind_multi(c,&b->v[name&0xffu],1u,captured,0u,1u))return false;continue;}}
      uint32_t start=candidate;
      while(true){{if(candidate>=count)return false;uint32_t node=kid(c,&kids,candidate);B{index} probe=*b;if({o}(c,goal,node,&probe)==0){{uint32_t end=ukl_max(candidate>=skipped?candidate-skipped:0u,start);if(name!=0xffffffffu&&!bind_multi(c,&b->v[name&0xffu],2u,base,start,end-start))return false;break;}}candidate++;}}
    }}
    while(true){{
      if(candidate>=count){{for(uint32_t g=goal;g<G;g++)if({e}[g]==0xfffffffeu&&!{d}[g])return false;return true;}}
      int verdict={o}(c,goal,kid(c,&kids,candidate),b);
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
