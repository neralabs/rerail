// Named-only documents' tokens (RER-131; `tape::tokens`), recognized on the device: per record of
// a kind a pattern reads (`wanted`), the tokens its stored children leave out, as
// `TokenGrammar::sequence` recognizes them on the host, line for line. The grammar's tables are
// `tape::tokens::DeviceTokens` (layout there); hidden text is its `HiddenText` DFA. A record's
// tokens go to `toks`, each `slot << 10 | kind` (its place among the record's parts, its
// tree-sitter kind), at `tokIndex[node]` (first, count); a record with none keeps (0, 0).
// The search keeps its frames in device memory (`stacks`, TP_DEPTH a thread of a pool striding
// over the batch's nodes), bounded as the host's is (RECOGNIZE_DEPTH, RECOGNIZE_STEPS).
// Shape: nodeCount the batch's nodes, fileCount its documents, resultArena `toks`' capacity,
// itemCount the pool's threads, steps 1 where every kind is wanted.
constant uint TP_NONE=0xffffffffu,TP_DEPTH=64u,TP_STEPS=4096u,TP_STATE=5u,TP_MOVE=3u,TP_TOKEN=5u,TP_HEADER=8u;
constant uint TP_NO_SYM=65535u,TP_NO_KIND=65535u,TP_KEPT=16u,TP_ERROR=0xfffffffeu;
struct Tp { device const uint* g; device const uchar* text; device const uchar* src; device uint* stack; };

static bool tp_white(uchar b){return b==32u||b==9u||b==10u||b==13u||b==11u||b==12u;}
static uint tp_skip(device const uchar* src,uint at,uint end){while(at<end&&tp_white(src[at]))at++;return at;}
static uchar tp_lower(uchar b){return (b>=65u&&b<=90u)?uchar(b+32u):b;}
// `spells`: the text exactly, or ignoring ASCII case.
static bool tp_spells(Tp t,uint at,uint text,uint len){for(uint i=0u;i<len;i++){uchar a=t.src[at+i],b=t.text[text+i];if(a!=b&&tp_lower(a)!=tp_lower(b))return false;}return true;}
// `HiddenText::find` over src[at..end]: where its match ends (from `at`), TP_NONE where none.
static uint tp_hidden(Tp t,uint h,uint at,uint end){
  device const uint* d=t.g+h;uint state=d[0],classes=d[1],states=d[2];
  device const uchar* cls=(device const uchar*)(d+3);device const uint* next=d+3u+64u;device const uint* eoi=next+states*classes;device const uint* fl=eoi+states;
  uint last=TP_NONE;
  for(uint i=at;i<end;i++){state=next[state*classes+cls[t.src[i]]];uint f=fl[state];if((f&1u)!=0u)last=i-at;else if((f&2u)!=0u)return last;}
  state=eoi[state];if((fl[state]&1u)!=0u)last=end-at;
  return last;
}
// The state a move by label (k0, k1) goes to from state row `row`, TP_NONE where it has none.
static uint tp_move(Tp t,uint row,uint k0,uint k1){
  uint at=t.g[row+1u],n=t.g[row+2u],lo=0u,hi=n;
  while(lo<hi){uint mid=(lo+hi)/2u;device const uint* m=t.g+at+mid*TP_MOVE;if(m[0]<k0||(m[0]==k0&&m[1]<k1))lo=mid+1u;else hi=mid;}
  if(lo<n){device const uint* m=t.g+at+lo*TP_MOVE;if(m[0]==k0&&m[1]==k1)return m[2];}
  return TP_NONE;
}
inline uint tp_row(uint automaton,uint state){return automaton+1u+state*TP_STATE;}
// `recognize`: a gap's tokens from `state` in src[at..limit], then the move by (k0, k1) (or, with
// none, acceptance). On success, the frames it kept (each frame: state, start, its move + 1) and
// `endState`; TP_NONE where none within the bounds. Depth-first in recognition order, as `walk`.
static uint tp_gap(Tp t,uint automaton,uint state,uint at,uint limit,bool hasNext,uint k0,uint k1,thread uint& endState){
  uint steps=TP_STEPS;int top=-1;uint s=state,a=at;
  while(true){
    // Enter a level at (s, a): `walk`'s checks.
    if(steps==0u)return TP_NONE;
    if(uint(top+1)<TP_DEPTH){
      steps--;a=tp_skip(t.src,a,limit);
      if(a==limit){
        uint row=tp_row(automaton,s);
        bool fits=hasNext?tp_move(t,row,k0,k1)!=TP_NONE:t.g[row]!=0u;
        if(fits){endState=s;return uint(top+1);}
      }else{top++;t.stack[top*3]=s;t.stack[top*3+1]=a;t.stack[top*3+2]=0u;}
    }
    // The innermost level's next move that reads the source; a level with none is left.
    bool called=false;
    while(!called){
      if(top<0)return TP_NONE;
      uint fs=t.stack[top*3],fa=t.stack[top*3+1],i=t.stack[top*3+2];
      uint row=tp_row(automaton,fs);uint first=t.g[row+3u],n=t.g[row+4u];
      while(i<n){
        device const uint* m=t.g+first+i*TP_TOKEN;i++;
        if(m[0]!=TP_NONE){uint found=tp_hidden(t,m[0],fa,limit);if(found!=TP_NONE&&found>0u){s=m[3];a=fa+found;called=true;break;}continue;}
        uint len=m[2];
        if(len==0u||fa+len>limit||!tp_spells(t,fa,m[1],len))continue;
        s=m[3];a=fa+len;called=true;break;
      }
      t.stack[top*3+2]=i;
      if(!called)top--;
    }
  }
}
// The tokens of a successful gap's `frames`, in order: count them, or write them from `slot`.
// TP_ERROR where one has no kind (`sequence`'s "a recognized token with no kind id").
static uint tp_emit(Tp t,uint automaton,uint frames,thread uint& slot,device uint* out,uint written){
  for(uint f=0u;f<frames;f++){
    uint row=tp_row(automaton,t.stack[f*3]);
    device const uint* m=t.g+t.g[row+3u]+(t.stack[f*3+2]-1u)*TP_TOKEN;
    if(m[0]!=TP_NONE)continue;
    uint kind=m[4]&0xffffu;
    if(kind==TP_NO_KIND||kind>=1024u||slot>=(1u<<22))return TP_ERROR;
    if(out)out[written]=(slot<<10)|kind;
    written++;slot++;
  }
  return written;
}
// `sequence`: record `local`'s recognized tokens, counted (`out` null) or written; 0 where it
// recognizes none, TP_ERROR where the host's would fail (then none, as `items` reads it).
static uint tp_node(Tp t,thread const Slabs& sl,DocView d,uint local,device uint* out){
  device const DeviceCode& code=tape_code(sl,d,local);
  if((code.flags&TP_KEPT)!=0u||(code.flags&1u)==0u||code.reserved==TP_NO_SYM)return 0u;
  uint automaton=t.g[t.g[1]+code.reserved];
  if(automaton==TP_NONE)return 0u;
  uint fieldCount=t.g[2],fieldAt=t.g[3];
  uint state=0u,at=tape_start(sl,d,local),slot=0u,written=0u;bool stored=false,refused=false;
  for(uint c=tape_first_child(sl,d,local);c!=TAPE_NONE;c=tape_next(sl,d,c)){
    device const DeviceCode& kid=tape_code(sl,d,c);
    uint start=tape_start(sl,d,c),end=tape_end(sl,d,c);
    uint field=kid.field<fieldCount?t.g[fieldAt+kid.field]:0u;
    if((kid.flags&2u)!=0u){slot++;at=end;continue;}
    if((kid.flags&1u)==0u){
      // A kept token: the automaton takes it before the next named child.
      if(kid.reserved==TP_NO_SYM)return TP_ERROR;
      if(!refused){uint next=tp_move(t,tp_row(automaton,state),0x80000000u|kid.reserved,field);if(next==TP_NONE)refused=true;else state=next;}
      stored=true;slot++;at=end;continue;
    }
    if(kid.reserved==TP_NO_SYM)return TP_ERROR;
    if(!stored){
      uint endState;uint frames=tp_gap(t,automaton,state,at,start,true,kid.reserved,field,endState);
      if(frames!=TP_NONE){written=tp_emit(t,automaton,frames,slot,out,written);if(written==TP_ERROR)return TP_ERROR;state=endState;}
    }else{
      if(refused)return TP_ERROR;
      stored=false;
    }
    uint next=tp_move(t,tp_row(automaton,state),kid.reserved,field);
    if(next==TP_NONE)return TP_ERROR;
    state=next;slot++;at=end;
  }
  if(!stored){
    uint endState;uint frames=tp_gap(t,automaton,state,at,tape_end(sl,d,local),false,0u,0u,endState);
    if(frames!=TP_NONE){written=tp_emit(t,automaton,frames,slot,out,written);if(written==TP_ERROR)return TP_ERROR;}
  }
  return written;
}
static uint tp_doc(device const DocView* docs,uint count,uint node){uint first=0u,last=count;while(last-first>1u){uint middle=(first+last)/2u;if(docs[middle].nodeBase<=node)first=middle;else last=middle;}return first;}
// flags[0] bit 2 (4): `toks` was too small, or a token past the encoding; the run fails.
kernel void token_parts(device const DocView* docs [[buffer(0)]],device const uint* tables [[buffer(1)]],device const uint* docTables [[buffer(2)]],device uint2* tokIndex [[buffer(3)]],device uint* toks [[buffer(4)]],device atomic_uint* bump [[buffer(5)]],device uint* stacks [[buffer(6)]],device const uint* wanted [[buffer(7)]],device atomic_uint* flags [[buffer(8)]],constant Shape& shape [[buffer(9)]],device const uchar* s0 [[buffer(27)]],device const uchar* s1 [[buffer(28)]],device const uchar* s2 [[buffer(29)]],device const uchar* s3 [[buffer(30)]],uint tid [[thread_position_in_grid]]){
  if(tid>=shape.itemCount)return;
  Slabs sl={s0,s1,s2,s3};
  for(uint node=tid;node<shape.nodeCount;node+=shape.itemCount){
    uint doc=tp_doc(docs,shape.fileCount,node);DocView d=docs[doc];
    if((d.widths&8192u)==0u||docTables[doc]==TP_NONE)continue;
    uint local=node-d.nodeBase;
    uint kind=tape_kind(sl,d,local);
    if(shape.steps==0u&&((wanted[kind/32u]>>(kind%32u))&1u)==0u)continue;
    device const uint* g=tables+docTables[doc];
    Tp t={g,(device const uchar*)(g+g[4]),tape_source(sl,d),stacks+tid*TP_DEPTH*3u};
    uint count=tp_node(t,sl,d,local,nullptr);
    if(count==0u||count==TP_ERROR)continue;
    uint first=atomic_fetch_add_explicit(bump,count,memory_order_relaxed);
    if(first+count>shape.resultArena){atomic_fetch_or_explicit(&flags[0],4u,memory_order_relaxed);return;}
    if(tp_node(t,sl,d,local,toks+first)!=count){atomic_fetch_or_explicit(&flags[0],4u,memory_order_relaxed);return;}
    tokIndex[node]=uint2(first,count);
  }
}
