// A lowered matcher's passes between its programs (06-first-slice S2, O23), one dependency level
// at a time. The labels live in the prefix of the batch's supertype buffer (`has_supertype` reads
// them as derived classes): element 0 is {label count, words per label}, then each label's bitset
// over the batch's global nodes. `scratch` has a row per pass of the widest level (`Op::row`, a
// pass's index in its level); `label_settle` moves a level's scratch into its labels.
#include <metal_stdlib>
using namespace metal;
struct Result { uint taskOrdinal,localOrdinal,start,label,captureStart,captureCount,propertyStart,propertyCount,dynamicStart,dynamicCount; };
struct FinalCapture { uint label,node,start,end,textStart,textLength,propertyStart,propertyCount,retained; };
struct Arena { uint start,capacity,cursor; };
struct Status { uint error,resultCount,captureCount,programCounter,finalCaptureCount,propertyCount,outputByteCount,reserved; };
struct Shape { uint nodeCount,words,resultArena,taskCount,itemCount,targetCount,round,finish,recursive,lastLevel,fileCount,steps,profile,queue,budget; };
// flags: [0] a task failed (bit 0 an interpreter task, bit 1 a micro walk), [2] the group settled (skip), [3] a group did not settle within its
// encoded rounds, [4 + round % 2] the round changed a label.
// How a program's results land: mode 0 roots, 1 captured nodes (into scratch row `slot`), 2 and
// 3 counts by root and by captured node (into count slot `slot`).
struct Route { uint mode,slot; };
// How a target settles: kind 0 from scratch, 1 the complement of `of`, 2 count slot `slot`'s
// tallies within minimum..=maximum.
struct Op { uint kind,target,of,slot,minimum,maximum,row; };

static void mark(device atomic_uint* bits,uint node){atomic_fetch_or_explicit(&bits[node/32u],1u<<(node%32u),memory_order_relaxed);}

kernel void label_sink(device const Arena* arenas [[buffer(0)]],device const Result* results [[buffer(1)]],device const FinalCapture* captures [[buffer(2)]],device const Status* statuses [[buffer(3)]],device const Route* routes [[buffer(4)]],device atomic_uint* scratch [[buffer(5)]],device atomic_uint* counts [[buffer(6)]],device atomic_uint* flags [[buffer(7)]],constant Shape& shape [[buffer(8)]],uint tid [[thread_position_in_grid]]){
  if(atomic_load_explicit(&flags[2],memory_order_relaxed)!=0u)return;
  // Bit 1: a task failed. Decision 0334: bit 4, a task ran over its budget below the ceiling (status 8); bit 8, a
  // task stopped at the ceiling (status 4). Its evaluation says which (`Runner::unfinished`).
  if(tid<shape.taskCount&&statuses[tid].error!=0u){uint error=statuses[tid].error;atomic_fetch_or_explicit(&flags[0],error==8u?4u:error==4u?8u:1u,memory_order_relaxed);}
  Arena arena=arenas[shape.resultArena];
  if(tid>=arena.cursor)return;
  Result result=results[arena.start+tid];
  Route route=routes[result.taskOrdinal];
  device atomic_uint* bits=scratch+route.slot*shape.words;
  device atomic_uint* tally=counts+route.slot*shape.nodeCount;
  if(route.mode==0u){mark(bits,result.start);return;}
  if(route.mode==2u){atomic_fetch_add_explicit(&tally[result.start],1u,memory_order_relaxed);return;}
  for(uint at=0u;at<result.captureCount;at++){uint node=captures[result.captureStart+at].node;if(route.mode==1u)mark(bits,node);else atomic_fetch_add_explicit(&tally[node],1u,memory_order_relaxed);}
}

// Each of the level's targets takes its new bits: a complement's or a count's computed here (per
// word), the others' from scratch. A change sets flags[1]; scratch is cleared.
kernel void label_settle(device uint* labels [[buffer(0)]],device uint* scratch [[buffer(1)]],device atomic_uint* flags [[buffer(2)]],device const Op* ops [[buffer(3)]],device uint* counts [[buffer(4)]],constant Shape& shape [[buffer(5)]],uint tid [[thread_position_in_grid]]){
  if(atomic_load_explicit(&flags[2],memory_order_relaxed)!=0u)return;
  // A round's last settle decides for the round before it, whose changes are all in: one that
  // changed nothing reached the fixpoint, so the rest of the group is skipped. Deciding a round
  // late costs only a round that changes nothing.
  if(tid==0u&&shape.recursive!=0u&&shape.lastLevel!=0u&&shape.round>0u){
    uint previous=4u+(shape.round-1u)%2u;
    if(atomic_load_explicit(&flags[previous],memory_order_relaxed)==0u){atomic_store_explicit(&flags[2],1u,memory_order_relaxed);atomic_fetch_or_explicit((device atomic_uint*)&labels[-2],0x80000000u,memory_order_relaxed);}
    atomic_store_explicit(&flags[previous],0u,memory_order_relaxed);
  }
  if(tid>=shape.targetCount*shape.words)return;
  Op op=ops[tid/shape.words];uint index=tid%shape.words;
  uint at=op.target*shape.words+index,row=op.row*shape.words+index;
  uint word=scratch[row];scratch[row]=0u;
  if(op.kind==1u){
    word=~labels[op.of*shape.words+index];
    uint last=shape.nodeCount%32u;
    if(index==shape.words-1u&&last!=0u)word&=(1u<<last)-1u;
  }else if(op.kind==2u){
    word=0u;
    for(uint bit=0u;bit<32u;bit++){uint node=index*32u+bit;if(node>=shape.nodeCount)break;uint count=counts[op.slot*shape.nodeCount+node];counts[op.slot*shape.nodeCount+node]=0u;if(count>=op.minimum&&count<=op.maximum)word|=1u<<bit;}
  }
  if(labels[at]!=word){labels[at]=word;if(shape.recursive!=0u)atomic_fetch_or_explicit(&flags[4u+shape.round%2u],1u,memory_order_relaxed);}
}

// A recursive group's end, one thread: still changing in its last round, without having been
// skipped, means it did not settle (flags[3]). Clears the skip and the changed flags.
kernel void label_round(device uint* flags [[buffer(0)]],device uint* head [[buffer(1)]],constant Shape& shape [[buffer(2)]],uint tid [[thread_position_in_grid]]){
  if(tid!=0u)return;
  if(flags[2]==0u&&flags[4u+(shape.round-1u)%2u]!=0u)flags[3]=1u;
  flags[2]=0u;flags[4]=0u;flags[5]=0u;head[0]&=0x7fffffffu;
}

// A micro walk (O28, `compile/micro.rs`, the same walk): one thread per (pass, start), the pass's
// program run from its start with a stack of frames in the thread, each a forked path or a move's
// next candidate (children by index, descendants by range, following siblings by link). A roots
// pass marks its start at the first emit; a targets pass marks every node an emitting path
// captured. A walk that outgrows its frames, captures or steps, or reads an unsaved cursor, sets
// bit 1 of flags[0], and the clause runs again with its passes on the interpreter.
// A walk's document (decision 0032): its tape read in place from the slabs, by `metal_tape.metal`'s
// accessors. Nodes are the batch's numbering throughout (labels, texts, records); a step reads
// its node at `node - doc.nodeBase`. `head` is the label prefix: {label count, words}, then the
// labels' bits.
struct Tree { Slabs s; DocView d; };
static DocView doc_of(device const DocView* docs,uint count,uint node){uint first=0u,last=count;while(last-first>1u){uint middle=(first+last)/2u;if(docs[middle].nodeBase<=node)first=middle;else last=middle;}return docs[first];}
inline uint tree_local(thread const Tree& t,uint node){return node-t.d.nodeBase;}
inline uint tree_global(thread const Tree& t,uint local){return local==TAPE_NONE?TAPE_NONE:local+t.d.nodeBase;}
inline uint tree_flags(thread const Tree& t,uint node){return tape_flags(t.s,t.d,tree_local(t,node));}
inline uint tree_next(thread const Tree& t,uint node,bool named){uint local=tree_local(t,node);return tree_global(t,named?tape_next_named(t.s,t.d,local):tape_next(t.s,t.d,local));}
inline uint tree_first(thread const Tree& t,uint node){return tree_global(t,tape_first_child(t.s,t.d,tree_local(t,node)));}
inline uint tree_end(thread const Tree& t,uint node){return node+tape_size(t.s,t.d,tree_local(t,node));}
inline uint tree_parent(thread const Tree& t,uint node){return tree_global(t,tape_parent(t.s,t.d,tree_local(t,node)));}
// The node after `node`'s subtree (named: the next named one) while it is before `end`, a parent's end.
inline uint tree_next_in(thread const Tree& t,uint node,uint end,bool named){uint local=tree_local(t,node),last=end-t.d.nodeBase;return tree_global(t,named?tape_next_named_in(t.s,t.d,local,last):tape_next_in(t.s,t.d,local,last));}
// The end of `node`'s parent's subtree (NONE at the root).
inline uint tree_parent_end(thread const Tree& t,uint node){uint parent=tree_parent(t,node);return parent==TAPE_NONE?TAPE_NONE:tree_end(t,parent);}
// A pass: its words, its scratch label, whether it marks captures, and its roots: its rule at
// `itemStart` in `items` (`root_mask`) and its threads, a group of 64 nodes each.
struct MicroPass { uint wordStart,wordCount,target,targets,itemStart,itemCount,textStart,threadStart; };
// A path's state, and for a move's frame (resume 1 children, 2 descendants, 3 following
// siblings; 0 a forked path) where its candidates continue: `next` the next candidate (a child or
// sibling by link, a descendant by number up to `end`).
// Eight cursors, as micro.rs's CURSORS.
struct MicroFrame { uint pc,node,resume,named,next,end,cursors[8],capture0,capture1,captured; };
// The loop tier's pool: 4096 threads of 64 frames, or (a run that outgrew them) 2048 of 128 in
// the same stacks (metal_label_passes.rs's MICRO_POOL, MICRO_DEEP_POOL).
constant uint MICRO_NONE=0xffffffffu,MICRO_FRAMES=64u,MICRO_POOL=4096u,MICRO_DEEP_FRAMES=128u,MICRO_DEEP_POOL=2048u;

// A walk's spill (decision 0125): past `shape.budget` steps a walk gives its lowest move frame
// with candidates left to the dispatch's work queue (`metal_work_queue.metal`), a descendant
// range of at least `2 * MICRO_CHUNK` nodes halved, so a long walk is many threads' work, not
// one's critical path. An item is `MICRO_ITEM` words of `items` (the frame, its walk's start,
// pass and document), written and read by atomics (coherent between threadgroups). A queue
// out of room leaves the frame with its walk. `capacity` 0: no spill (a fused group's walks).
struct MicroQ { device atomic_uint* slots; device atomic_uint* control; device atomic_uint* items; uint capacity; uint pass; uint doc; };
// Only the first `MICRO_STANDBY` threads of a dispatch wait for spilled frames (`MICRO_POLLS`
// polls between items); the rest exit once the queue is empty. Every thread polling while a long
// walk ran was 10^8 contended loads a dispatch. A thread that spilled still takes its own frames.
constant uint MICRO_ITEM=20u,MICRO_CHUNK=64u,MICRO_POLLS=4096u,MICRO_STANDBY=512u;

static bool micro_class(thread const Tree& t,device const uint* head,uint node,uint kind){
  if(kind>=16384u){uint words=head[1];device const uint* bits=head+2;return ((bits[(kind-16384u)*words+node/32u]>>(node%32u))&1u)!=0u;}
  return tape_has_class(t.s,t.d,tree_local(t,node),kind);
}

// A move frame's next candidate (`MicroFrame`, `RootFrame`).
template<typename Frame>
static uint walk_next(thread const Tree& t,thread Frame& frame){
  if(frame.resume==1u){uint end=tree_end(t,frame.end);while(frame.next!=MICRO_NONE){uint child=frame.next;frame.next=tree_next_in(t,child,end,false);if(frame.named==0u||(tree_flags(t,child)&1u))return child;}return MICRO_NONE;}
  if(frame.resume==2u){while(frame.next<frame.end){uint node=frame.next;frame.next++;if(frame.named==0u||(tree_flags(t,node)&1u))return node;}return MICRO_NONE;}
  if(frame.resume==3u){uint node=frame.next;if(node==MICRO_NONE)return MICRO_NONE;frame.next=tree_next_in(t,node,frame.end,frame.named!=0u);return node;}
  return MICRO_NONE;
}

// A walk's frames live in thread memory, which the driver reserves per thread in flight: a
// walk takes the smallest stack its static depth fits (`Micro::depth`; 06 S7: 128 frames for
// every walk held ~850 MB a run).
template<uint FRAMES,typename Stack>
static void micro_from(thread const Tree& t,device const uint* head,MicroPass pass,device const uint* words,uint start,device atomic_uint* scratch,device atomic_uint* flags,device const uint* texts,device atomic_uint* counts,constant Shape& shape,Stack stack,thread uint& steps,MicroQ q,uint depth0);

template<uint FRAMES,typename Stack>
static void micro_walk(device const DocView* docs,thread const Slabs& slabs,device const uint* head,device const MicroPass* passes,device const uint* words,device const uint* items,device atomic_uint* scratch,device atomic_uint* flags,device const uint* texts,device atomic_uint* counts,constant Shape& shape,Stack stack,uint tid,device atomic_uint* wstats,MicroQ q){
  if(atomic_load_explicit(&flags[2],memory_order_relaxed)!=0u)return;
  if(tid>=shape.itemCount)return;
  // A pass's threads are contiguous from its threadStart, one a group of 64 nodes of the batch;
  // a thread walks from each of its group's roots (`root_mask`, read from the code bytes).
  uint low=0u,high=shape.targetCount;while(high-low>1u){uint middle=(low+high)/2u;if(passes[middle].threadStart<=tid)low=middle;else high=middle;}
  uint group=tid-passes[low].threadStart,file=roots_file(items,group);
  Tree t={slabs,docs[file]};
  group-=roots_first_group(items,file);
  ulong mask=root_mask(slabs,t.d,items,passes[low].itemStart,file,group);
  q.pass=low;q.doc=file;
  uint walks=0u,total=0u,longest=0u;
  ROOTS_EACH(mask,group,local){
    uint steps=0u;
    micro_from<FRAMES>(t,head,passes[low],words,t.d.nodeBase+local,scratch,flags,texts,counts,shape,stack,steps,q,0u);
    walks++;total+=steps;longest=max(longest,steps);
  }
  // Profiling (`RERAIL_GPU_WALK_STEPS`): the pass's walks, their steps, its longest walk's.
  if(shape.profile!=0u&&walks!=0u){device atomic_uint* w=wstats+3u*(shape.profile-1u+low);atomic_fetch_add_explicit(&w[0],walks,memory_order_relaxed);atomic_fetch_add_explicit(&w[1],total,memory_order_relaxed);atomic_fetch_max_explicit(&w[2],longest,memory_order_relaxed);}
}

// The queue's spilled frames (`MicroQ`), each walked from its frame, until none is left or the
// worker has polled its limit for nothing (`wq_idle`: `MICRO_POLLS` a standby thread's, else 1).
template<uint FRAMES,typename Stack>
static void micro_drain(device const DocView* docs,thread const Slabs& slabs,device const uint* head,device const MicroPass* passes,device const uint* words,device atomic_uint* scratch,device atomic_uint* flags,device const uint* texts,device atomic_uint* counts,constant Shape& shape,Stack stack,device atomic_uint* wstats,MicroQ q,uint tid){
  if(q.capacity==0u)return;
  WorkQueue wq={q.slots,q.control,q.capacity};
  uint polls=0u,limit=tid<MICRO_STANDBY?MICRO_POLLS:1u;
  for(uint turn=0u;turn<WQ_TURNS;turn++){
    if(atomic_load_explicit(&flags[2],memory_order_relaxed)!=0u)return;
    uint at;
    if(!wq_take(wq,at)){if(wq_idle(wq,polls,limit))return;continue;}
    atomic_thread_fence(mem_flags::mem_device,memory_order_seq_cst,thread_scope_device);
    device atomic_uint* item=q.items+at*MICRO_ITEM;
    uint w[MICRO_ITEM];
    for(uint k=0u;k<MICRO_ITEM;k++)w[k]=atomic_load_explicit(&item[k],memory_order_relaxed);
    MicroFrame f={w[0],w[1],w[2],w[3],w[4],w[5],{w[6],w[7],w[8],w[9],w[10],w[11],w[12],w[13]},w[14],w[15],w[16]};
    stack[0]=f;
    Tree t={slabs,docs[w[19]]};
    q.pass=w[18];q.doc=w[19];
    uint steps=0u;
    micro_from<FRAMES>(t,head,passes[w[18]],words,w[17],scratch,flags,texts,counts,shape,stack,steps,q,1u);
    if(shape.profile!=0u){device atomic_uint* s=wstats+3u*(shape.profile-1u+w[18]);atomic_fetch_add_explicit(&s[0],1u,memory_order_relaxed);atomic_fetch_add_explicit(&s[1],steps,memory_order_relaxed);atomic_fetch_max_explicit(&s[2],steps,memory_order_relaxed);}
    wq_done(wq);
    polls=0u;
  }
}

// A pass's walk from `start`.
// Gives the lowest move frame of `stack` with candidates left to the queue (its range's upper
// half where a descendant range is long; the whole frame where frames above it are the walk's).
template<typename Stack>
static void micro_spill(MicroQ q,Stack stack,uint depth,uint start){
  for(uint i=0u;i<depth;i++){
    MicroFrame f=stack[i];
    if(f.resume==0u)continue;
    bool has=f.resume==2u?f.next<f.end:f.next!=MICRO_NONE;
    if(!has)continue;
    MicroFrame give=f;
    if(f.resume==2u&&f.end-f.next>=2u*MICRO_CHUNK){uint mid=f.next+(f.end-f.next)/2u;give.next=mid;f.end=mid;}
    else if(i+1u<depth){if(f.resume==2u)f.next=f.end;else f.next=MICRO_NONE;}
    else continue;
    uint at=atomic_fetch_add_explicit(&q.control[4],1u,memory_order_relaxed);
    if(at>=q.capacity)return;
    device atomic_uint* item=q.items+at*MICRO_ITEM;
    uint w[MICRO_ITEM]={give.pc,give.node,give.resume,give.named,give.next,give.end,give.cursors[0],give.cursors[1],give.cursors[2],give.cursors[3],give.cursors[4],give.cursors[5],give.cursors[6],give.cursors[7],give.capture0,give.capture1,give.captured,start,q.pass,q.doc};
    for(uint k=0u;k<MICRO_ITEM;k++)atomic_store_explicit(&item[k],w[k],memory_order_relaxed);
    atomic_thread_fence(mem_flags::mem_device,memory_order_seq_cst,thread_scope_device);
    if(wq_push(WorkQueue{q.slots,q.control,q.capacity},at))stack[i]=f;
    return;
  }
}

template<uint FRAMES,typename Stack>
static void micro_from(thread const Tree& t,device const uint* head,MicroPass pass,device const uint* words,uint start,device atomic_uint* scratch,device atomic_uint* flags,device const uint* texts,device atomic_uint* counts,constant Shape& shape,Stack stack,thread uint& steps,MicroQ q,uint depth0){
  device atomic_uint* bits=scratch+pass.target*shape.words;
  uint depth=depth0,fired=0u,spillAt=shape.budget;
  MicroFrame path={0u,start,0u,0u,0u,0u,{MICRO_NONE,MICRO_NONE,MICRO_NONE,MICRO_NONE,MICRO_NONE,MICRO_NONE,MICRO_NONE,MICRO_NONE},MICRO_NONE,MICRO_NONE,0u};
  #define MICRO_FAIL {atomic_fetch_or_explicit(&flags[0],2u,memory_order_relaxed);return;}
  // A spilled frame's walk starts at its frame (`depth0` 1), as a path that ended.
  bool fresh=depth0==0u;
  while(true){
    bool alive=fresh;
    fresh=true;
    while(alive){
      if(++steps>shape.steps)MICRO_FAIL
      uint word=words[pass.wordStart+path.pc],op=word&255u,mode=(word>>8u)&255u,arg=word>>16u;
      bool named=mode==1u;
      switch(op){
        case 0u:alive=false;break;
        case 1u:alive=tape_kind(t.s,t.d,tree_local(t,path.node))==arg;break;
        case 2u:alive=tape_field(t.s,t.d,tree_local(t,path.node))==arg;break;
        case 3u:{uint nodeFlags=tree_flags(t,path.node);alive=(nodeFlags&mode)==mode&&(nodeFlags&arg)==0u;}break;
        case 10u:alive=micro_class(t,head,path.node,arg);break;
        case 4u:alive=((texts[(pass.textStart+arg)*shape.words+path.node/32u]>>(path.node%32u))&1u)!=0u;break;
        case 17u:{uint next=tree_next(t,path.node,named);if(next==MICRO_NONE)alive=false;else path.node=next;}break;
        case 18u:{uint parent=tree_parent(t,path.node);if(parent==MICRO_NONE)alive=false;else path.node=parent;}break;
        case 16u:case 19u:case 20u:{
          MicroFrame frame=path;frame.pc=path.pc+1u;frame.named=named?1u:0u;
          if(op==16u){frame.resume=1u;frame.next=tree_first(t,path.node);frame.end=path.node;}
          else if(op==19u){frame.resume=2u;frame.next=path.node+1u;frame.end=tree_end(t,path.node);}
          else{frame.resume=3u;frame.end=tree_parent_end(t,path.node);frame.next=frame.end==TAPE_NONE?MICRO_NONE:tree_next_in(t,path.node,frame.end,named);}
          uint candidate=walk_next(t,frame);
          if(candidate==MICRO_NONE){alive=false;break;}
          if(depth==FRAMES)MICRO_FAIL
          stack[depth++]=frame;path.node=candidate;
        }break;
        case 21u:path.cursors[arg&7u]=path.node;break;
        case 22u:{uint saved=path.cursors[arg&7u];if(saved==MICRO_NONE)MICRO_FAIL path.node=saved;}break;
        case 32u:if(path.captured==0u)path.capture0=path.node;else if(path.captured==1u)path.capture1=path.node;else MICRO_FAIL path.captured++;break;
        case 48u:{if(depth==FRAMES)MICRO_FAIL MicroFrame frame=path;frame.pc=arg;frame.resume=0u;stack[depth++]=frame;}break;
        case 49u:path.pc=arg;continue;
        case 64u:
          // As label_sink routes: 0 the start (once), 1 the captured nodes, 2 a tally by start,
          // 3 a tally by captured node, one each path.
          // A pack (bit 2, `fusion::Fused`): the emit's row, by its label, follows the emit count
          // after the words; the walk ends once every emit marked its start.
          if((pass.targets&4u)!=0u){
            uint members=words[pass.wordStart+pass.wordCount];
            device atomic_uint* row=scratch+words[pass.wordStart+pass.wordCount+1u+arg]*shape.words;
            if((pass.targets&3u)==0u){mark(row,start);fired|=1u<<arg;if(fired==(members>=32u?0xffffffffu:((1u<<members)-1u)))return;}
            else{if(path.captured>0u)mark(row,path.capture0);if(path.captured>1u)mark(row,path.capture1);}
            alive=false;break;
          }
          if(pass.targets==0u){mark(bits,start);return;}
          if(pass.targets==1u){if(path.captured>0u)mark(bits,path.capture0);if(path.captured>1u)mark(bits,path.capture1);}
          else{device atomic_uint* tally=counts+pass.target*shape.nodeCount;
            if(pass.targets==2u)atomic_fetch_add_explicit(&tally[start],1u,memory_order_relaxed);
            else{if(path.captured>0u)atomic_fetch_add_explicit(&tally[path.capture0],1u,memory_order_relaxed);if(path.captured>1u)atomic_fetch_add_explicit(&tally[path.capture1],1u,memory_order_relaxed);}}
          alive=false;break;
        default:MICRO_FAIL
      }
      if(alive){path.pc++;if(path.pc>=pass.wordCount)MICRO_FAIL}
    }
    if(q.capacity!=0u&&steps>=spillAt){spillAt=steps+shape.budget;micro_spill(q,stack,depth,start);}
    // The next frame with a path left.
    while(true){
      if(depth==0u)return;
      if(stack[depth-1u].resume==0u){path=stack[--depth];break;}
      // The top frame advanced in thread memory (the stack may be in device memory).
      MicroFrame top=stack[depth-1u];
      uint candidate=walk_next(t,top);
      if(candidate==MICRO_NONE){depth--;continue;}
      stack[depth-1u]=top;path=top;path.node=candidate;break;
    }
  }
  #undef MICRO_FAIL
}
kernel void label_micro_8(device const DocView* docs [[buffer(0)]],device const uint* head [[buffer(1)]],device const MicroPass* passes [[buffer(2)]],device const uint* words [[buffer(3)]],device const uint* items [[buffer(4)]],device atomic_uint* scratch [[buffer(5)]],device atomic_uint* flags [[buffer(6)]],device const uint* texts [[buffer(7)]],device atomic_uint* counts [[buffer(8)]],device MicroFrame* stacks [[buffer(9)]],constant Shape& shape [[buffer(10)]],device const uchar* s0 [[buffer(27)]],device const uchar* s1 [[buffer(28)]],device const uchar* s2 [[buffer(29)]],device const uchar* s3 [[buffer(30)]],device atomic_uint* qslots [[buffer(23)]],device atomic_uint* qcontrol [[buffer(24)]],device atomic_uint* qitems [[buffer(25)]],device atomic_uint* wstats [[buffer(26)]],uint tid [[thread_position_in_grid]],uint grid [[threads_per_grid]]){Slabs slabs={s0,s1,s2,s3};MicroQ q={qslots,qcontrol,qitems,shape.queue,0u,0u};for(uint item=tid;item<shape.itemCount;item+=grid)micro_walk<8u>(docs,slabs,head,passes,words,items,scratch,flags,texts,counts,shape,stacks+tid*8u,item,wstats,q);micro_drain<8u>(docs,slabs,head,passes,words,scratch,flags,texts,counts,shape,stacks+tid*8u,wstats,q,tid);}
kernel void label_micro_32(device const DocView* docs [[buffer(0)]],device const uint* head [[buffer(1)]],device const MicroPass* passes [[buffer(2)]],device const uint* words [[buffer(3)]],device const uint* items [[buffer(4)]],device atomic_uint* scratch [[buffer(5)]],device atomic_uint* flags [[buffer(6)]],device const uint* texts [[buffer(7)]],device atomic_uint* counts [[buffer(8)]],device MicroFrame* stacks [[buffer(9)]],constant Shape& shape [[buffer(10)]],device const uchar* s0 [[buffer(27)]],device const uchar* s1 [[buffer(28)]],device const uchar* s2 [[buffer(29)]],device const uchar* s3 [[buffer(30)]],device atomic_uint* qslots [[buffer(23)]],device atomic_uint* qcontrol [[buffer(24)]],device atomic_uint* qitems [[buffer(25)]],device atomic_uint* wstats [[buffer(26)]],uint tid [[thread_position_in_grid]],uint grid [[threads_per_grid]]){Slabs slabs={s0,s1,s2,s3};MicroQ q={qslots,qcontrol,qitems,shape.queue,0u,0u};for(uint item=tid;item<shape.itemCount;item+=grid)micro_walk<32u>(docs,slabs,head,passes,words,items,scratch,flags,texts,counts,shape,stacks+tid*32u,item,wstats,q);micro_drain<32u>(docs,slabs,head,passes,words,scratch,flags,texts,counts,shape,stacks+tid*32u,wstats,q,tid);}
// Loops whose depth follows the tree: MICRO_POOL threads stride over the items, each with
// its stack in device memory (`stacks`), so no thread memory is reserved for 128 frames per
// thread in flight.
kernel void label_micro(device const DocView* docs [[buffer(0)]],device const uint* head [[buffer(1)]],device const MicroPass* passes [[buffer(2)]],device const uint* words [[buffer(3)]],device const uint* items [[buffer(4)]],device atomic_uint* scratch [[buffer(5)]],device atomic_uint* flags [[buffer(6)]],device const uint* texts [[buffer(7)]],device atomic_uint* counts [[buffer(8)]],device MicroFrame* stacks [[buffer(9)]],constant Shape& shape [[buffer(10)]],device const uchar* s0 [[buffer(27)]],device const uchar* s1 [[buffer(28)]],device const uchar* s2 [[buffer(29)]],device const uchar* s3 [[buffer(30)]],device atomic_uint* qslots [[buffer(23)]],device atomic_uint* qcontrol [[buffer(24)]],device atomic_uint* qitems [[buffer(25)]],device atomic_uint* wstats [[buffer(26)]],uint tid [[thread_position_in_grid]]){Slabs slabs={s0,s1,s2,s3};MicroQ q={qslots,qcontrol,qitems,shape.queue,0u,0u};for(uint item=tid;item<shape.itemCount;item+=MICRO_POOL)micro_walk<MICRO_FRAMES>(docs,slabs,head,passes,words,items,scratch,flags,texts,counts,shape,stacks+tid*MICRO_FRAMES,item,wstats,q);micro_drain<MICRO_FRAMES>(docs,slabs,head,passes,words,scratch,flags,texts,counts,shape,stacks+tid*MICRO_FRAMES,wstats,q,tid);}
kernel void label_micro_deep(device const DocView* docs [[buffer(0)]],device const uint* head [[buffer(1)]],device const MicroPass* passes [[buffer(2)]],device const uint* words [[buffer(3)]],device const uint* items [[buffer(4)]],device atomic_uint* scratch [[buffer(5)]],device atomic_uint* flags [[buffer(6)]],device const uint* texts [[buffer(7)]],device atomic_uint* counts [[buffer(8)]],device MicroFrame* stacks [[buffer(9)]],constant Shape& shape [[buffer(10)]],device const uchar* s0 [[buffer(27)]],device const uchar* s1 [[buffer(28)]],device const uchar* s2 [[buffer(29)]],device const uchar* s3 [[buffer(30)]],device atomic_uint* qslots [[buffer(23)]],device atomic_uint* qcontrol [[buffer(24)]],device atomic_uint* qitems [[buffer(25)]],device atomic_uint* wstats [[buffer(26)]],uint tid [[thread_position_in_grid]]){Slabs slabs={s0,s1,s2,s3};MicroQ q={qslots,qcontrol,qitems,shape.queue,0u,0u};for(uint item=tid;item<shape.itemCount;item+=MICRO_DEEP_POOL)micro_walk<MICRO_DEEP_FRAMES>(docs,slabs,head,passes,words,items,scratch,flags,texts,counts,shape,stacks+tid*MICRO_DEEP_FRAMES,item,wstats,q);micro_drain<MICRO_DEEP_FRAMES>(docs,slabs,head,passes,words,scratch,flags,texts,counts,shape,stacks+tid*MICRO_DEEP_FRAMES,wstats,q,tid);}

// A recursive group's rounds to its fixpoint in one dispatch (decision 0024 §3): a threadgroup
// per file, whose labels are its own (a walk never leaves its tree). A round runs each level in
// order: its walks over the file's starts, then each target settled over the file's words, as
// `label_settle` does for every file, the words a file shares with its neighbours by masked
// atomics. The rounds repeat until one changes nothing; a file still changing after its node
// count and two more sets flags[3]. `levels`: per level its passes' range and its settles'.
struct GroupLevel { uint passStart,passCount,opStart,opCount; };

static uint group_mask(uint index,uint first,uint end){
  uint low=index*32u,high=low+32u;
  uint from=max(first,low)-low,to=min(end,high)-low;
  if(from>=to)return 0u;
  return (to-from==32u)?0xffffffffu:(((1u<<(to-from))-1u)<<from);
}

template<uint FRAMES,typename Stack>
static void label_group_run(Stack stack,device const DocView* docs,thread const Slabs& slabs,device const uint* head,device const MicroPass* passes,device const uint* words,device const uint* items,device atomic_uint* scratch,device atomic_uint* flags,device const uint* texts,device atomic_uint* counts,device const GroupLevel* levels,device const Op* ops,constant Shape& shape,uint file,uint lane,uint width,threadgroup atomic_uint* changed){
  device atomic_uint* labels=(device atomic_uint*)(head+2);
  DocView own=docs[file];
  Tree t={slabs,own};
  uint first=own.nodeBase,end=own.nodeBase+own.nodes;
  if(first>=end)return;
  uint firstWord=first/32u,endWord=(end+31u)/32u;
  uint limit=own.nodes+2u;
  bool settled=false;
  for(uint round=0u;round<limit;round++){
    if(lane==0u)atomic_store_explicit(changed,0u,memory_order_relaxed);
    threadgroup_barrier(mem_flags::mem_threadgroup);
    for(uint level=0u;level<shape.targetCount;level++){
      GroupLevel at=levels[level];
      for(uint p=0u;p<at.passCount;p++){
        MicroPass pass=passes[at.passStart+p];
        // The pass's roots in this file: its groups, a lane's every `width`th.
        for(uint group=lane,groups=roots_file_groups(items,file);group<groups;group+=width){
          ulong mask=root_mask(slabs,own,items,pass.itemStart,file,group);
          ROOTS_EACH(mask,group,local){uint steps=0u;micro_from<FRAMES>(t,head,pass,words,own.nodeBase+local,scratch,flags,texts,counts,shape,stack,steps,MicroQ{flags,flags,(device atomic_uint*)flags,0u,0u,0u},0u);}
        }
      }
      threadgroup_barrier(mem_flags::mem_device);
      for(uint task=lane;task<at.opCount*(endWord-firstWord);task+=width){
        Op op=ops[at.opStart+task/(endWord-firstWord)];
        uint index=firstWord+task%(endWord-firstWord),mask=group_mask(index,first,end);
        uint row=op.row*shape.words+index,word;
        word=atomic_fetch_and_explicit(&scratch[row],~mask,memory_order_relaxed)&mask;
        if(op.kind==1u){
          word=~atomic_load_explicit(&labels[op.of*shape.words+index],memory_order_relaxed)&mask;
        }else if(op.kind==2u){
          word=0u;
          for(uint bit=0u;bit<32u;bit++){
            if(((mask>>bit)&1u)==0u)continue;
            uint node=index*32u+bit;
            uint count=atomic_exchange_explicit(&counts[op.slot*shape.nodeCount+node],0u,memory_order_relaxed);
            if(count>=op.minimum&&count<=op.maximum)word|=1u<<bit;
          }
        }
        device atomic_uint* target=&labels[op.target*shape.words+index];
        uint old=atomic_load_explicit(target,memory_order_relaxed)&mask;
        if(old!=word){
          atomic_fetch_or_explicit(target,word,memory_order_relaxed);
          atomic_fetch_and_explicit(target,word|~mask,memory_order_relaxed);
          atomic_store_explicit(changed,1u,memory_order_relaxed);
        }
      }
      threadgroup_barrier(mem_flags::mem_device|mem_flags::mem_threadgroup);
    }
    if(atomic_load_explicit(changed,memory_order_relaxed)==0u){settled=true;break;}
    threadgroup_barrier(mem_flags::mem_threadgroup);
  }
  if(!settled&&lane==0u)atomic_fetch_or_explicit(&flags[3],1u,memory_order_relaxed);
}

kernel void label_group_8(device const DocView* docs [[buffer(0)]],device const uint* head [[buffer(1)]],device const MicroPass* passes [[buffer(2)]],device const uint* words [[buffer(3)]],device const uint* items [[buffer(4)]],device atomic_uint* scratch [[buffer(5)]],device atomic_uint* flags [[buffer(6)]],device const uint* texts [[buffer(7)]],device atomic_uint* counts [[buffer(8)]],device const GroupLevel* levels [[buffer(9)]],device const Op* ops [[buffer(10)]],constant Shape& shape [[buffer(11)]],device const uchar* s0 [[buffer(27)]],device const uchar* s1 [[buffer(28)]],device const uchar* s2 [[buffer(29)]],device const uchar* s3 [[buffer(30)]],device MicroFrame* stacks [[buffer(12)]],uint file [[threadgroup_position_in_grid]],uint lane [[thread_index_in_threadgroup]],uint width [[threads_per_threadgroup]]){Slabs slabs={s0,s1,s2,s3};threadgroup atomic_uint changed;label_group_run<8u>(stacks+(file*width+lane)*8u,docs,slabs,head,passes,words,items,scratch,flags,texts,counts,levels,ops,shape,file,lane,width,&changed);}
kernel void label_group_32(device const DocView* docs [[buffer(0)]],device const uint* head [[buffer(1)]],device const MicroPass* passes [[buffer(2)]],device const uint* words [[buffer(3)]],device const uint* items [[buffer(4)]],device atomic_uint* scratch [[buffer(5)]],device atomic_uint* flags [[buffer(6)]],device const uint* texts [[buffer(7)]],device atomic_uint* counts [[buffer(8)]],device const GroupLevel* levels [[buffer(9)]],device const Op* ops [[buffer(10)]],constant Shape& shape [[buffer(11)]],device const uchar* s0 [[buffer(27)]],device const uchar* s1 [[buffer(28)]],device const uchar* s2 [[buffer(29)]],device const uchar* s3 [[buffer(30)]],device MicroFrame* stacks [[buffer(12)]],uint file [[threadgroup_position_in_grid]],uint lane [[thread_index_in_threadgroup]],uint width [[threads_per_threadgroup]]){Slabs slabs={s0,s1,s2,s3};threadgroup atomic_uint changed;label_group_run<32u>(stacks+(file*width+lane)*32u,docs,slabs,head,passes,words,items,scratch,flags,texts,counts,levels,ops,shape,file,lane,width,&changed);}
kernel void label_group_64(device const DocView* docs [[buffer(0)]],device const uint* head [[buffer(1)]],device const MicroPass* passes [[buffer(2)]],device const uint* words [[buffer(3)]],device const uint* items [[buffer(4)]],device atomic_uint* scratch [[buffer(5)]],device atomic_uint* flags [[buffer(6)]],device const uint* texts [[buffer(7)]],device atomic_uint* counts [[buffer(8)]],device const GroupLevel* levels [[buffer(9)]],device const Op* ops [[buffer(10)]],constant Shape& shape [[buffer(11)]],device const uchar* s0 [[buffer(27)]],device const uchar* s1 [[buffer(28)]],device const uchar* s2 [[buffer(29)]],device const uchar* s3 [[buffer(30)]],device MicroFrame* stacks [[buffer(12)]],uint file [[threadgroup_position_in_grid]],uint lane [[thread_index_in_threadgroup]],uint width [[threads_per_threadgroup]]){Slabs slabs={s0,s1,s2,s3};threadgroup atomic_uint changed;label_group_run<64u>(stacks+(file*width+lane)*64u,docs,slabs,head,passes,words,items,scratch,flags,texts,counts,levels,ops,shape,file,lane,width,&changed);}
kernel void label_group_128(device const DocView* docs [[buffer(0)]],device const uint* head [[buffer(1)]],device const MicroPass* passes [[buffer(2)]],device const uint* words [[buffer(3)]],device const uint* items [[buffer(4)]],device atomic_uint* scratch [[buffer(5)]],device atomic_uint* flags [[buffer(6)]],device const uint* texts [[buffer(7)]],device atomic_uint* counts [[buffer(8)]],device const GroupLevel* levels [[buffer(9)]],device const Op* ops [[buffer(10)]],constant Shape& shape [[buffer(11)]],device const uchar* s0 [[buffer(27)]],device const uchar* s1 [[buffer(28)]],device const uchar* s2 [[buffer(29)]],device const uchar* s3 [[buffer(30)]],device MicroFrame* stacks [[buffer(12)]],uint file [[threadgroup_position_in_grid]],uint lane [[thread_index_in_threadgroup]],uint width [[threads_per_threadgroup]]){Slabs slabs={s0,s1,s2,s3};threadgroup atomic_uint changed;label_group_run<128u>(stacks+(file*width+lane)*128u,docs,slabs,head,passes,words,items,scratch,flags,texts,counts,levels,ops,shape,file,lane,width,&changed);}
kernel void label_group_256(device const DocView* docs [[buffer(0)]],device const uint* head [[buffer(1)]],device const MicroPass* passes [[buffer(2)]],device const uint* words [[buffer(3)]],device const uint* items [[buffer(4)]],device atomic_uint* scratch [[buffer(5)]],device atomic_uint* flags [[buffer(6)]],device const uint* texts [[buffer(7)]],device atomic_uint* counts [[buffer(8)]],device const GroupLevel* levels [[buffer(9)]],device const Op* ops [[buffer(10)]],constant Shape& shape [[buffer(11)]],device const uchar* s0 [[buffer(27)]],device const uchar* s1 [[buffer(28)]],device const uchar* s2 [[buffer(29)]],device const uchar* s3 [[buffer(30)]],device MicroFrame* stacks [[buffer(12)]],uint file [[threadgroup_position_in_grid]],uint lane [[thread_index_in_threadgroup]],uint width [[threads_per_threadgroup]]){Slabs slabs={s0,s1,s2,s3};threadgroup atomic_uint changed;label_group_run<256u>(stacks+(file*width+lane)*256u,docs,slabs,head,passes,words,items,scratch,flags,texts,counts,levels,ops,shape,file,lane,width,&changed);}

// A root program's walk as records (decision 0024 §3, `Micro::records`, the same walk): a
// thread per (program, file, task, start) item, after the labels, reading them in place. Each
// path that emits is a record of 20 words: item, its ordinal in the item, the output label, the
// captures held, then 8 captured nodes and their 8 labels. The walk runs twice: once to count its
// records, once to write them where one atomic per item reserved them (an item numbered from
// its dispatch's `shape.round`) (`cursor[0]`; `cursor[1]`
// the buffer was too small). A walk outgrowing its frames, captures or steps flags its program.
// pad: 1 where the walk ends at its first emit (set mode, one label); 1 + its emit count for a
// pack in set mode.
// `anchor`: its anchor's row in `texts` (decision 0365), a start outside it not walked; `0xffffffff` none.
struct RootPass { uint wordStart,wordCount,textStart,pad,anchor; };
struct RootFrame { uint pc,node,resume,named,next,end,cursors[8],captured,caps[8],labs[8]; };
// The 32-frame roots run as a pool of ROOT_POOL threads with device stacks (metal_label_passes.rs's ROOT_POOL).
constant uint ROOT_CAPTURES=8u,ROOT_RECORD=20u,ROOT_POOL=4096u;

template<uint FRAMES,typename Stack>
static bool root_walk(thread const Tree& t,device const uint* head,RootPass pass,device const uint* words,uint start,device const uint* texts,constant Shape& shape,Stack stack,device uint* out,uint capacity,uint item,uint at,thread uint& emitted){
  uint depth=0u,steps=0u,fired=0u;
  RootFrame path;
  path.pc=0u;path.node=start;path.resume=0u;path.named=0u;path.next=0u;path.end=0u;path.captured=0u;
  for(uint c=0u;c<8u;c++){path.cursors[c]=MICRO_NONE;path.caps[c]=MICRO_NONE;path.labs[c]=0u;}
  while(true){
    bool alive=true;
    while(alive){
      if(++steps>shape.steps)return false;
      uint word=words[pass.wordStart+path.pc],op=word&255u,mode=(word>>8u)&255u,arg=word>>16u;
      bool named=mode==1u;
      switch(op){
        case 0u:alive=false;break;
        case 1u:alive=tape_kind(t.s,t.d,tree_local(t,path.node))==arg;break;
        case 2u:alive=tape_field(t.s,t.d,tree_local(t,path.node))==arg;break;
        case 3u:{uint nodeFlags=tree_flags(t,path.node);alive=(nodeFlags&mode)==mode&&(nodeFlags&arg)==0u;}break;
        case 10u:alive=micro_class(t,head,path.node,arg);break;
        case 4u:alive=((texts[(pass.textStart+arg)*shape.words+path.node/32u]>>(path.node%32u))&1u)!=0u;break;
        case 17u:{uint next=tree_next(t,path.node,named);if(next==MICRO_NONE)alive=false;else path.node=next;}break;
        case 18u:{uint parent=tree_parent(t,path.node);if(parent==MICRO_NONE)alive=false;else path.node=parent;}break;
        case 16u:case 19u:case 20u:{
          RootFrame frame=path;frame.pc=path.pc+1u;frame.named=named?1u:0u;
          if(op==16u){frame.resume=1u;frame.next=tree_first(t,path.node);frame.end=path.node;}
          else if(op==19u){frame.resume=2u;frame.next=path.node+1u;frame.end=tree_end(t,path.node);}
          else{frame.resume=3u;frame.end=tree_parent_end(t,path.node);frame.next=frame.end==TAPE_NONE?MICRO_NONE:tree_next_in(t,path.node,frame.end,named);}
          uint candidate=walk_next(t,frame);
          if(candidate==MICRO_NONE){alive=false;break;}
          if(depth==FRAMES)return false;
          stack[depth++]=frame;path.node=candidate;
        }break;
        case 21u:path.cursors[arg&7u]=path.node;break;
        case 22u:{uint saved=path.cursors[arg&7u];if(saved==MICRO_NONE)return false;path.node=saved;}break;
        case 32u:if(path.captured==ROOT_CAPTURES)return false;path.caps[path.captured]=path.node;path.labs[path.captured]=arg;path.captured++;break;
        case 48u:{if(depth==FRAMES)return false;RootFrame frame=path;frame.pc=arg;frame.resume=0u;stack[depth++]=frame;}break;
        case 49u:path.pc=arg;continue;
        case 64u:
          // A pack in set mode (pad: 1 + its emit count, `fusion::Fused`): an emit whose label
          // fired is dropped, and the walk ends once every label fired.
          if(pass.pad>1u){uint bit=1u<<arg;if((fired&bit)!=0u){alive=false;break;}fired|=bit;}
          if(out!=nullptr&&at+emitted<capacity){
            device uint* record=out+(at+emitted)*ROOT_RECORD;
            record[0]=item;record[1]=emitted;record[2]=arg;record[3]=path.captured;
            for(uint c=0u;c<8u;c++){record[4u+c]=path.caps[c];record[12u+c]=path.labs[c];}
          }
          emitted++;alive=false;
          // Set mode with one label (`Micro::key_stop`): the task's key is found.
          if(pass.pad==1u)return true;
          if(pass.pad>1u&&fired==(pass.pad-1u>=32u?0xffffffffu:((1u<<(pass.pad-1u))-1u)))return true;
          break;
        default:return false;
      }
      if(alive){path.pc++;if(path.pc>=pass.wordCount)return false;}
    }
    while(true){
      if(depth==0u)return true;
      if(stack[depth-1u].resume==0u){path=stack[--depth];break;}
      RootFrame top=stack[depth-1u];
      uint candidate=walk_next(t,top);
      if(candidate==MICRO_NONE){depth--;continue;}
      stack[depth-1u]=top;path=top;path.node=candidate;break;
    }
  }
}

template<uint FRAMES,typename Stack>
static void root_records_run(device const DocView* docs,thread const Slabs& slabs,device const uint* head,device const RootPass* passes,device const uint* words,device const uint4* items,device const uint* texts,device uint* records,device atomic_uint* cursor,device atomic_uint* failures,constant Shape& shape,uint tid,Stack stack){
  if(tid>=shape.itemCount)return;
  uint4 item=items[tid];RootPass pass=passes[item.x];
  if(pass.anchor!=0xffffffffu&&((texts[pass.anchor*shape.words+item.w/32u]>>(item.w%32u))&1u)==0u)return;
  Tree t={slabs,doc_of(docs,shape.fileCount,item.w)};
  uint emitted=0u;
  if(!root_walk<FRAMES>(t,head,pass,words,item.w,texts,shape,stack,nullptr,0u,tid,0u,emitted)){atomic_fetch_or_explicit(&failures[item.x],1u,memory_order_relaxed);return;}
  if(emitted==0u)return;
  uint at=atomic_fetch_add_explicit(&cursor[0],emitted,memory_order_relaxed);
  if(at+emitted>shape.targetCount){atomic_fetch_or_explicit(&cursor[1],1u,memory_order_relaxed);return;}
  uint again=0u;
  root_walk<FRAMES>(t,head,pass,words,item.w,texts,shape,stack,records,shape.targetCount,shape.round+tid,at,again);
}

kernel void root_records_8(device const DocView* docs [[buffer(0)]],device const uint* head [[buffer(1)]],device const RootPass* passes [[buffer(2)]],device const uint* words [[buffer(3)]],device const uint4* items [[buffer(4)]],device const uint* texts [[buffer(5)]],device uint* records [[buffer(6)]],device atomic_uint* cursor [[buffer(7)]],device atomic_uint* failures [[buffer(8)]],constant Shape& shape [[buffer(9)]],device const uchar* s0 [[buffer(27)]],device const uchar* s1 [[buffer(28)]],device const uchar* s2 [[buffer(29)]],device const uchar* s3 [[buffer(30)]],device RootFrame* stacks [[buffer(10)]],uint tid [[thread_position_in_grid]],uint grid [[threads_per_grid]]){Slabs slabs={s0,s1,s2,s3};for(uint item=tid;item<shape.itemCount;item+=grid)root_records_run<8u>(docs,slabs,head,passes,words,items,texts,records,cursor,failures,shape,item,stacks+tid*8u);}
kernel void root_records_32(device const DocView* docs [[buffer(0)]],device const uint* head [[buffer(1)]],device const RootPass* passes [[buffer(2)]],device const uint* words [[buffer(3)]],device const uint4* items [[buffer(4)]],device const uint* texts [[buffer(5)]],device uint* records [[buffer(6)]],device atomic_uint* cursor [[buffer(7)]],device atomic_uint* failures [[buffer(8)]],constant Shape& shape [[buffer(9)]],device const uchar* s0 [[buffer(27)]],device const uchar* s1 [[buffer(28)]],device const uchar* s2 [[buffer(29)]],device const uchar* s3 [[buffer(30)]],device RootFrame* stacks [[buffer(10)]],uint tid [[thread_position_in_grid]]){Slabs slabs={s0,s1,s2,s3};for(uint item=tid;item<shape.itemCount;item+=ROOT_POOL)root_records_run<32u>(docs,slabs,head,passes,words,items,texts,records,cursor,failures,shape,item,stacks+tid*32u);}

// A level's anchors (decision 0365, `Runner::formulas`). First `label_leaves`: each node's code read once, and every
// distinct leaf test of the level's formulas (kind, field, flags, a grammar supertype) into its row of `leaves`, a
// file's words at its region (`FORMULA_REGION`). Then `label_formula`: a threadgroup a (file, formula), the formula
// (`compile_sets::DeviceFormula`, postfix over a stack of node sets in `scratch`, the file's region) into its rule's row
// in `roots`: set where `mode` 0 (a rule over codes, kind 2), cleared outside the anchor where 1 (a listed rule). A leaf
// is a copy of its row, a label a read of `head` (the levels before have settled), and an axis step goes over the
// members of its set only (a climb stops at a node another thread set, whose climb goes on).
// A formula: its targets (`FormulaTarget`: {row, mode, rule (`0xffffffff` none: a root walk's anchor in `texts`)}) and
// words; identical formulas of a group are one, written to each target. Shape:
// `round` a formula's slots, `itemCount` the dispatch's first formula, `queue` the words of a formula's scratch,
// `recursive` the words of a leaf's row, `targetCount` (`label_leaves`) the leaves.
struct Formula { uint targetStart,targetCount,wordStart,wordCount,pad; };
// Where a formula's set goes: a rule's row, as `Formula` was (one formula, several rules).
struct FormulaTarget { uint row,mode,rule; };
#define FORMULA_REGION(d,file) ((d).nodeBase/32u+2u*(file))
static bool formula_get(device atomic_uint* slot,uint n){return ((atomic_load_explicit(&slot[n/32u],memory_order_relaxed)>>(n%32u))&1u)!=0u;}
static bool formula_set(device atomic_uint* slot,uint n){uint bit=1u<<(n%32u);return (atomic_fetch_or_explicit(&slot[n/32u],bit,memory_order_relaxed)&bit)!=0u;}
static bool formula_named(thread const Tree& t,uint n,bool named){return !named||(tape_flags(t.s,t.d,n)&1u)!=0u;}
// The members of a slot: `FORMULA_EACH(slot, fw, lane, width, n) { ... }`, a word a thread.
#define FORMULA_EACH(SLOT_,FW,LANE,WIDTH,N) for(uint formulaW=(LANE);formulaW<(FW);formulaW+=(WIDTH))for(uint formulaBits=atomic_load_explicit(&(SLOT_)[formulaW],memory_order_relaxed);formulaBits!=0u;formulaBits&=formulaBits-1u)for(uint N=formulaW*32u+ctz(formulaBits),formulaOnce=0u;formulaOnce==0u;formulaOnce=1u)

kernel void label_leaves(device const DocView* docs [[buffer(0)]],device const uint* tests [[buffer(1)]],device atomic_uint* leaves [[buffer(2)]],constant Shape& shape [[buffer(3)]],device const uchar* s0 [[buffer(27)]],device const uchar* s1 [[buffer(28)]],device const uchar* s2 [[buffer(29)]],device const uchar* s3 [[buffer(30)]],uint file [[threadgroup_position_in_grid]],uint lane [[thread_index_in_threadgroup]],uint width [[threads_per_threadgroup]]){
  Slabs slabs={s0,s1,s2,s3};
  DocView d=docs[file];
  uint nodes=d.nodes,fw=(nodes+31u)/32u,region=FORMULA_REGION(d,file);
  for(uint w=lane;w<fw;w+=width){
    for(uint l=0u;l<shape.targetCount;l++)atomic_store_explicit(&leaves[l*shape.recursive+region+w],0u,memory_order_relaxed);
    for(uint k=0u;k<32u&&w*32u+k<nodes;k++){
      uint n=w*32u+k;
      device const DeviceCode& code=tape_code(slabs,d,n);
      for(uint l=0u;l<shape.targetCount;l++){
        uint test=tests[l],op=test&255u,arg=test>>8u;bool hit;
        if(op==18u)hit=tape_next(slabs,d,n)!=TAPE_NONE;
        else if(op==22u){hit=false;for(uint c=tape_next(slabs,d,n);c!=TAPE_NONE;c=tape_next(slabs,d,c))if((tape_flags(slabs,d,c)&1u)!=0u){hit=true;break;}}
        else if(op==21u){hit=false;for(uint c=tape_first_child(slabs,d,n);c!=TAPE_NONE;c=tape_next(slabs,d,c))if((tape_flags(slabs,d,c)&1u)!=0u){hit=true;break;}}
        else if(op==19u)hit=tape_size(slabs,d,n)>1u;
        else if(op==1u)hit=code.kind==arg;
        else if(op==2u)hit=code.field==arg;
        else if(op==3u){uint m=arg&255u,a=arg>>8u;hit=(code.flags&m)==m&&(code.flags&a)==0u;}
        else{hit=false;for(uint c=0u;c<code.classCount;c++){if(code.classes[c]==arg){hit=true;break;}if(code.classes[c]>arg)break;}}
        if(hit)atomic_fetch_or_explicit(&leaves[l*shape.recursive+region+w],1u<<k,memory_order_relaxed);
      }
    }
  }
}

kernel void label_formula(device const DocView* docs [[buffer(0)]],device const uint* head [[buffer(1)]],device const Formula* formulas [[buffer(2)]],device const uint* words [[buffer(3)]],device atomic_uint* roots [[buffer(4)]],device atomic_uint* scratch [[buffer(5)]],constant Shape& shape [[buffer(6)]],device const uint* leaves [[buffer(7)]],device atomic_uint* stats [[buffer(8)]],device const FormulaTarget* targets [[buffer(9)]],device atomic_uint* labelScratch [[buffer(10)]],device const uint* rules [[buffer(11)]],device const uint* texts [[buffer(12)]],device const uchar* s0 [[buffer(27)]],device const uchar* s1 [[buffer(28)]],device const uchar* s2 [[buffer(29)]],device const uchar* s3 [[buffer(30)]],uint2 group [[threadgroup_position_in_grid]],uint2 lanes [[thread_position_in_threadgroup]],uint2 widths [[threads_per_threadgroup]]){
  Slabs slabs={s0,s1,s2,s3};
  uint file=group.x,lane=lanes.x,width=widths.x;
  // A descendant step's prefix of its members' counts (a block of words a thread, `FORMULA_WIDTH` threads), and the
  // member counts a step within a conjunction is priced by.
  threadgroup uint partials[128];
  threadgroup atomic_uint tally[2];
  DocView d=docs[file];
  Tree t={slabs,d};
  uint nodes=d.nodes,fw=(nodes+31u)/32u,region=FORMULA_REGION(d,file);
  if(nodes==0u)return;
  Formula formula=formulas[shape.itemCount+group.y];
  // A rule over codes that does not run in this file: its row is never read here; none runs here, nothing to do.
  bool wanted=false;
  // A rule over codes runs in a file where its bit is set (an anchored rule's after its row, `Roots::table`).
  for(uint i=0u;i<formula.targetCount&&!wanted;i++){
    FormulaTarget target=targets[formula.targetStart+i];
    if(target.mode==1u||target.rule==0xffffffffu){wanted=true;continue;}
    uint codes=rules[target.rule]==2u?target.rule+rules[target.rule+1u]:target.rule;
    wanted=rules[codes]!=0u||((rules[codes+1u+file/32u]>>(file%32u))&1u)!=0u;
  }
  if(!wanted)return;
  device atomic_uint* base=scratch+group.y*shape.queue+shape.round*region;
  #define SLOT(i) (base+(i)*fw)
  // The bits of local word `w` that are nodes of the file.
  #define VALID(w) ((w)*32u+32u<=nodes?0xffffffffu:((1u<<(nodes-(w)*32u))-1u))
  // Profiling (`shape.profile`): per step kind its members, then parent and child reads, then the (file, formula)s.
  bool profile=shape.profile!=0u;
  uint parents=0u,childs=0u;
  #define FORMULA_COUNT(I,N) if(profile)atomic_fetch_add_explicit(&stats[I],N,memory_order_relaxed)
  if(lane==0u)FORMULA_COUNT(18u,1u);
  uint top=0u;
  for(uint pc=0u;pc<formula.wordCount;pc++){
    uint word=words[formula.wordStart+pc],op=word&255u,arg=word>>8u;
    if(op==0u||op==4u||op==12u||op==13u||op==20u){
      // A set: every node, a label (its row in `head`, in the batch's numbering), none, a leaf's row, or a text test's
      // row in `texts` (the batch's numbering).
      device atomic_uint* out=SLOT(top);
      for(uint w=lane;w<fw;w+=width){
        uint bits=0u;
        if(op==0u)bits=VALID(w);
        else if(op==13u)bits=leaves[arg*shape.recursive+region+w];
        else if(op==4u||op==20u){device const uint* row=op==4u?head+2u+(arg-16384u)*head[1]:texts+arg*shape.words;uint b=d.nodeBase+w*32u,at=b/32u,sh=b%32u;bits=sh==0u?row[at]:(row[at]>>sh)|(row[at+1u]<<(32u-sh));bits&=VALID(w);}
        atomic_store_explicit(&out[w],bits,memory_order_relaxed);
      }
      top++;
    }else if(op==5u||op==6u){
      uint first=top-arg;
      device atomic_uint* out=SLOT(first);
      for(uint w=lane;w<fw;w+=width){
        uint bits=atomic_load_explicit(&out[w],memory_order_relaxed);
        for(uint k=1u;k<arg;k++){uint other=atomic_load_explicit(&SLOT(first+k)[w],memory_order_relaxed);bits=op==5u?bits&other:bits|other;}
        atomic_store_explicit(&out[w],bits,memory_order_relaxed);
      }
      top=first+1u;
    }else if(op>=14u&&op<=16u){
      // A step within a conjunction: slot `c` its set, `src` the step's inner set, the result `c` ∩ the step's. Read
      // down from `c`'s members (children, the subtree by rank, the next siblings), or climb from the inner set's
      // (a parent is a search on the tape), whichever has fewer members (S277: priced by work).
      bool named=(arg&1u)!=0u;
      uint c=top-2u,src=top-1u,at=top;
      device atomic_uint* cand=SLOT(c);device atomic_uint* from=SLOT(src);device atomic_uint* out=SLOT(at);
      uint mine=0u,theirs=0u;
      for(uint w=lane;w<fw;w+=width){mine+=popcount(atomic_load_explicit(&cand[w],memory_order_relaxed));theirs+=popcount(atomic_load_explicit(&from[w],memory_order_relaxed));atomic_store_explicit(&out[w],0u,memory_order_relaxed);}
      if(lane==0u){atomic_store_explicit(&tally[0],0u,memory_order_relaxed);atomic_store_explicit(&tally[1],0u,memory_order_relaxed);}
      threadgroup_barrier(mem_flags::mem_threadgroup|mem_flags::mem_device);
      atomic_fetch_add_explicit(&tally[0],mine,memory_order_relaxed);atomic_fetch_add_explicit(&tally[1],theirs,memory_order_relaxed);
      threadgroup_barrier(mem_flags::mem_threadgroup);
      uint candidates=atomic_load_explicit(&tally[0],memory_order_relaxed),members=atomic_load_explicit(&tally[1],memory_order_relaxed);
      bool down=candidates<=4u*members;
      if(lane==0u){FORMULA_COUNT(op,down?candidates:members);FORMULA_COUNT(down?19u:20u,1u);}
      if(down&&op==15u){
        // The inner set's members (named where it says) ranked by a prefix of their words' counts.
        device atomic_uint* ranked=SLOT(at+1u);device atomic_uint* prefix=SLOT(at+2u);
        for(uint w=lane;w<fw;w+=width){
          uint bits=atomic_load_explicit(&from[w],memory_order_relaxed);
          if(named)for(uint rest=bits;rest!=0u;rest&=rest-1u){uint k=ctz(rest);if((tape_flags(slabs,d,w*32u+k)&1u)==0u)bits&=~(1u<<k);}
          atomic_store_explicit(&ranked[w],bits,memory_order_relaxed);
        }
        threadgroup_barrier(mem_flags::mem_device);
        uint block=(fw+width-1u)/width,low=min(lane*block,fw),high=min(low+block,fw),sum=0u;
        for(uint w=low;w<high;w++)sum+=popcount(atomic_load_explicit(&ranked[w],memory_order_relaxed));
        partials[lane]=sum;
        threadgroup_barrier(mem_flags::mem_threadgroup);
        if(lane==0u){uint run=0u;for(uint k=0u;k<width;k++){uint here=partials[k];partials[k]=run;run+=here;}}
        threadgroup_barrier(mem_flags::mem_threadgroup);
        uint run=partials[lane];
        for(uint w=low;w<high;w++){atomic_store_explicit(&prefix[w],run,memory_order_relaxed);run+=popcount(atomic_load_explicit(&ranked[w],memory_order_relaxed));}
        threadgroup_barrier(mem_flags::mem_device);
        FORMULA_EACH(cand,fw,lane,width,n){
          uint end=min(n+tape_size(slabs,d,n),nodes),first=n+1u;
          if(end<=first)continue;
          // The ranked members before `end`, less those before `first`.
          uint we=(end-1u)/32u,wf=first/32u,re=(end-1u)%32u;
          uint be=atomic_load_explicit(&prefix[we],memory_order_relaxed)+popcount(atomic_load_explicit(&ranked[we],memory_order_relaxed)&(re==31u?0xffffffffu:((1u<<(re+1u))-1u)));
          uint bf=atomic_load_explicit(&prefix[wf],memory_order_relaxed)+popcount(atomic_load_explicit(&ranked[wf],memory_order_relaxed)&((1u<<(first%32u))-1u));
          if(be>bf)formula_set(out,n);
        }
      }else if(down){
        // Each candidate's children (a child step) or next siblings (following): one in the inner set keeps it.
        FORMULA_EACH(cand,fw,lane,width,n){
          for(uint k=op==14u?tape_first_child(slabs,d,n):tape_next(slabs,d,n);k!=TAPE_NONE;k=tape_next(slabs,d,k)){
            childs++;
            if(formula_get(from,k)&&formula_named(t,k,named)){formula_set(out,n);break;}
          }
        }
      }else{
        // The climb, as the step alone (`CHILD`, `DESCENDANT`, `FOLLOWING`), then within the candidates.
        if(op==14u){
          FORMULA_EACH(from,fw,lane,width,n)if(formula_named(t,n,named)){parents++;uint p=tape_parent(slabs,d,n);if(p!=TAPE_NONE&&formula_get(cand,p))formula_set(out,p);}
        }else if(op==15u){
          FORMULA_EACH(from,fw,lane,width,n)if(formula_named(t,n,named)){for(uint p=tape_parent(slabs,d,n);p!=TAPE_NONE;p=tape_parent(slabs,d,p)){parents++;if(formula_set(out,p))break;}}
        }else{
          // The members' parents, then each such parent's children before its last member.
          device atomic_uint* parentSet=SLOT(at+1u);
          for(uint w=lane;w<fw;w+=width)atomic_store_explicit(&parentSet[w],0u,memory_order_relaxed);
          threadgroup_barrier(mem_flags::mem_device);
          FORMULA_EACH(from,fw,lane,width,n)if(formula_named(t,n,named)){parents++;uint p=tape_parent(slabs,d,n);if(p!=TAPE_NONE)formula_set(parentSet,p);}
          threadgroup_barrier(mem_flags::mem_device);
          FORMULA_EACH(parentSet,fw,lane,width,p){
            uint last=TAPE_NONE;
            for(uint k=tape_first_child(slabs,d,p);k!=TAPE_NONE;k=tape_next(slabs,d,k)){childs++;if(formula_get(from,k)&&formula_named(t,k,named))last=k;}
            if(last!=TAPE_NONE)for(uint k=tape_first_child(slabs,d,p);k!=last;k=tape_next(slabs,d,k)){childs++;formula_set(out,k);}
          }
        }
        threadgroup_barrier(mem_flags::mem_device);
        // Within the candidates (a descendant's climb marked every ancestor).
        for(uint w=lane;w<fw;w+=width)atomic_store_explicit(&out[w],atomic_load_explicit(&out[w],memory_order_relaxed)&atomic_load_explicit(&cand[w],memory_order_relaxed),memory_order_relaxed);
      }
      threadgroup_barrier(mem_flags::mem_device);
      for(uint w=lane;w<fw;w+=width)atomic_store_explicit(&cand[w],atomic_load_explicit(&out[w],memory_order_relaxed),memory_order_relaxed);
      top=c+1u;
    }else{
      // An axis step: from slot `src` (a chain's through, then to) into the slot past them, then back to `src`.
      bool named=(arg&1u)!=0u;
      uint src=op==11u?top-2u:top-1u,at=top;
      device atomic_uint* from=SLOT(src);device atomic_uint* out=SLOT(at);
      for(uint w=lane;w<fw;w+=width){atomic_store_explicit(&out[w],0u,memory_order_relaxed);if(op==10u)atomic_store_explicit(&SLOT(at+1u)[w],0u,memory_order_relaxed);}
      threadgroup_barrier(mem_flags::mem_device);
      uint members=0u;
      if(profile)FORMULA_EACH(op==11u?SLOT(src+1u):from,fw,lane,width,n)members++;
      FORMULA_COUNT(op,members);
      if(op==7u){
        // The parents of the members.
        FORMULA_EACH(from,fw,lane,width,n)if(formula_named(t,n,named)){parents++;uint p=tape_parent(slabs,d,n);if(p!=TAPE_NONE)formula_set(out,p);}
      }else if(op==8u){
        // The ancestors of the members.
        FORMULA_EACH(from,fw,lane,width,n)if(formula_named(t,n,named)){for(uint p=tape_parent(slabs,d,n);p!=TAPE_NONE;p=tape_parent(slabs,d,p)){parents++;if(formula_set(out,p))break;}}
      }else if(op==9u){
        // The children of the members.
        FORMULA_EACH(from,fw,lane,width,n)for(uint c=tape_first_child(slabs,d,n);c!=TAPE_NONE;c=tape_next(slabs,d,c)){childs++;formula_set(out,c);}
      }else if(op==10u){
        // The earlier siblings of the members: the members' parents, then each such parent's children before its last member.
        device atomic_uint* parentSet=SLOT(at+1u);
        FORMULA_EACH(from,fw,lane,width,n)if(formula_named(t,n,named)){parents++;uint p=tape_parent(slabs,d,n);if(p!=TAPE_NONE)formula_set(parentSet,p);}
        threadgroup_barrier(mem_flags::mem_device);
        FORMULA_EACH(parentSet,fw,lane,width,p){
          uint last=TAPE_NONE;
          for(uint c=tape_first_child(slabs,d,p);c!=TAPE_NONE;c=tape_next(slabs,d,c)){childs++;if(formula_get(from,c)&&formula_named(t,c,named))last=c;}
          if(last!=TAPE_NONE)for(uint c=tape_first_child(slabs,d,p);c!=last;c=tape_next(slabs,d,c)){childs++;formula_set(out,c);}
        }
      }else{
        // A chain: the parents of `to`'s members, and on up through `through`'s.
        device atomic_uint* through=SLOT(src);device atomic_uint* to=SLOT(src+1u);
        FORMULA_EACH(to,fw,lane,width,n){
          if(!formula_named(t,n,named))continue;
          for(uint p=tape_parent(slabs,d,n);p!=TAPE_NONE;p=tape_parent(slabs,d,p)){
            parents++;
            if(formula_set(out,p))break;
            if(!formula_get(through,p)||!formula_named(t,p,named))break;
          }
        }
      }
      threadgroup_barrier(mem_flags::mem_device);
      for(uint w=lane;w<fw;w+=width)atomic_store_explicit(&from[w],atomic_load_explicit(&out[w],memory_order_relaxed),memory_order_relaxed);
      top=src+1u;
    }
    threadgroup_barrier(mem_flags::mem_device);
  }
  FORMULA_COUNT(16u,parents);
  FORMULA_COUNT(17u,childs);
  #undef FORMULA_COUNT
  // The anchor into its rule's row, in the batch's numbering: a file's word spans two of the row's.
  device atomic_uint* anchor=SLOT(0u);
  uint sh=d.nodeBase%32u;
  for(uint i=0u;i<formula.targetCount;i++){
    FormulaTarget target=targets[formula.targetStart+i];
    // Modes 0 and 1 a rule's row in `roots`; 2 a formula pass's scratch row, within its rule.
    device atomic_uint* row=(target.mode==2u?labelScratch:roots)+target.row;
    for(uint w=lane;w<fw;w+=width){
      uint bits=atomic_load_explicit(&anchor[w],memory_order_relaxed),valid=VALID(w),at=(d.nodeBase+w*32u)/32u;
      if(target.mode==2u)bits&=uint(root_mask(slabs,d,rules,target.rule,file,w/2u)>>(32u*(w%2u)));
      if(target.mode!=1u){
        if(bits==0u)continue;
        atomic_fetch_or_explicit(&row[at],bits<<sh,memory_order_relaxed);
        if(sh!=0u)atomic_fetch_or_explicit(&row[at+1u],bits>>(32u-sh),memory_order_relaxed);
      }else{
        atomic_fetch_and_explicit(&row[at],~(valid<<sh)|(bits<<sh),memory_order_relaxed);
        if(sh!=0u)atomic_fetch_and_explicit(&row[at+1u],~(valid>>(32u-sh))|(bits>>(32u-sh)),memory_order_relaxed);
      }
    }
  }
  #undef SLOT
  #undef VALID
}
