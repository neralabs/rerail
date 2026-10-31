// A lowered matcher's passes between its programs (06-first-slice S2, O23), one dependency level
// at a time. The labels live in the prefix of the batch's supertype buffer (`has_supertype` reads
// them as derived classes): element 0 is {label count, words per label}, then each label's bitset
// over the batch's global nodes. `scratch` has the same shape; `label_settle` moves a level's
// scratch into its labels.
#include <metal_stdlib>
using namespace metal;
struct Result { uint taskOrdinal,localOrdinal,start,label,captureStart,captureCount,propertyStart,propertyCount,dynamicStart,dynamicCount; };
struct FinalCapture { uint label,node,start,end,textStart,textLength,propertyStart,propertyCount,retained; };
struct Arena { uint start,capacity,cursor; };
struct Status { uint error,resultCount,captureCount,programCounter,finalCaptureCount,propertyCount,outputByteCount,reserved; };
struct Shape { uint nodeCount,words,resultArena,taskCount,itemCount,targetCount,round,finish,recursive,lastLevel,fileCount,steps; };
// flags: [0] a task failed (bit 0 an interpreter task, bit 1 a micro walk), [2] the group settled (skip), [3] a group did not settle within its
// encoded rounds, [4 + round % 2] the round changed a label.
// How a program's results land: mode 0 roots, 1 captured nodes (into scratch label `slot`), 2 and
// 3 counts by root and by captured node (into count slot `slot`).
struct Route { uint mode,slot; };
// How a target settles: kind 0 from scratch, 1 the complement of `of`, 2 count slot `slot`'s
// tallies within minimum..=maximum.
struct Op { uint kind,target,of,slot,minimum,maximum; };

static void mark(device atomic_uint* bits,uint node){atomic_fetch_or_explicit(&bits[node/32u],1u<<(node%32u),memory_order_relaxed);}

kernel void label_sink(device const Arena* arenas [[buffer(0)]],device const Result* results [[buffer(1)]],device const FinalCapture* captures [[buffer(2)]],device const Status* statuses [[buffer(3)]],device const Route* routes [[buffer(4)]],device atomic_uint* scratch [[buffer(5)]],device atomic_uint* counts [[buffer(6)]],device atomic_uint* flags [[buffer(7)]],constant Shape& shape [[buffer(8)]],uint tid [[thread_position_in_grid]]){
  if(atomic_load_explicit(&flags[2],memory_order_relaxed)!=0u)return;
  if(tid<shape.taskCount&&statuses[tid].error!=0u)atomic_fetch_or_explicit(&flags[0],1u,memory_order_relaxed);
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
  uint at=op.target*shape.words+index;
  uint word=scratch[at];scratch[at]=0u;
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
struct Node { uint parent,nextAny,nextNamed,childStart,childCount,descendantEnd,sourceStart,sourceEnd,kind,field,flags; };
struct File { uint nodeStart,nodeCount,sourceStart,sourceLength,lineStart,lineCount,superStart,superCount; };
struct Supertype { uint node,kind; };
// A pass: its words, its scratch label, whether it marks captures, and its starts in `items`.
struct MicroPass { uint wordStart,wordCount,target,targets,itemStart,itemCount,textStart,pad; };
// A path's state, and for a move's frame (resume 1 children, 2 descendants, 3 following
// siblings; 0 a forked path) where its candidates continue.
struct MicroFrame { uint pc,node,resume,named,next,end,cursor0,cursor1,capture0,capture1,captured; };
constant uint MICRO_NONE=0xffffffffu,MICRO_FRAMES=128u;

static bool micro_class(device const Supertype* supertypes,device const File* files,uint fileCount,uint node,uint kind){
  if(kind>=61440u){uint words=supertypes[0].kind;device const uint* bits=(device const uint*)(supertypes+1);return ((bits[(kind-61440u)*words+node/32u]>>(node%32u))&1u)!=0u;}
  uint first=0u,last=fileCount;while(last-first>1u){uint middle=(first+last)/2u;if(files[middle].nodeStart<=node)first=middle;else last=middle;}
  File file=files[first];uint low=file.superStart,high=low+file.superCount;
  while(low<high){uint middle=low+(high-low)/2u;Supertype value=supertypes[middle];if(value.node<node||(value.node==node&&value.kind<kind))low=middle+1u;else high=middle;}
  return low<file.superStart+file.superCount&&supertypes[low].node==node&&supertypes[low].kind==kind;
}

static uint micro_next(device const Node* nodes,device const uint* children,thread MicroFrame& frame){
  if(frame.resume==1u){Node parent=nodes[frame.end];while(frame.next<parent.childCount){uint child=children[parent.childStart+frame.next];frame.next++;if(frame.named==0u||(nodes[child].flags&1u))return child;}return MICRO_NONE;}
  if(frame.resume==2u){while(frame.next<frame.end){uint node=frame.next;frame.next++;if(frame.named==0u||(nodes[node].flags&1u))return node;}return MICRO_NONE;}
  if(frame.resume==3u){uint node=frame.next;if(node==MICRO_NONE)return MICRO_NONE;frame.next=frame.named?nodes[node].nextNamed:nodes[node].nextAny;return node;}
  return MICRO_NONE;
}

kernel void label_micro(device const Node* nodes [[buffer(0)]],device const uint* children [[buffer(1)]],device const Supertype* supertypes [[buffer(2)]],device const File* files [[buffer(3)]],device const MicroPass* passes [[buffer(4)]],device const uint* words [[buffer(5)]],device const uint* items [[buffer(6)]],device atomic_uint* scratch [[buffer(7)]],device atomic_uint* flags [[buffer(8)]],device const uint* texts [[buffer(9)]],device atomic_uint* counts [[buffer(10)]],constant Shape& shape [[buffer(11)]],uint tid [[thread_position_in_grid]]){
  if(atomic_load_explicit(&flags[2],memory_order_relaxed)!=0u)return;
  if(tid>=shape.itemCount)return;
  uint low=0u,high=shape.targetCount;while(high-low>1u){uint middle=(low+high)/2u;if(passes[middle].itemStart<=tid)low=middle;else high=middle;}
  MicroPass pass=passes[low];
  uint start=items[tid];
  device atomic_uint* bits=scratch+pass.target*shape.words;
  thread MicroFrame stack[MICRO_FRAMES];uint depth=0u,steps=0u;
  MicroFrame path={0u,start,0u,0u,0u,0u,MICRO_NONE,MICRO_NONE,MICRO_NONE,MICRO_NONE,0u};
  #define MICRO_FAIL {atomic_fetch_or_explicit(&flags[0],2u,memory_order_relaxed);return;}
  while(true){
    bool alive=true;
    while(alive){
      if(++steps>shape.steps)MICRO_FAIL
      uint word=words[pass.wordStart+path.pc],op=word&255u,mode=(word>>8u)&255u,arg=word>>16u;
      bool named=mode==1u;Node node=nodes[path.node];
      switch(op){
        case 0u:alive=false;break;
        case 1u:alive=node.kind==arg;break;
        case 2u:alive=node.field==arg;break;
        case 3u:alive=(node.flags&mode)==mode&&(node.flags&arg)==0u;break;
        case 10u:alive=micro_class(supertypes,files,shape.fileCount,path.node,arg);break;
        case 4u:alive=((texts[(pass.textStart+arg)*shape.words+path.node/32u]>>(path.node%32u))&1u)!=0u;break;
        case 17u:{uint next=named?node.nextNamed:node.nextAny;if(next==MICRO_NONE)alive=false;else path.node=next;}break;
        case 18u:if(node.parent==MICRO_NONE)alive=false;else path.node=node.parent;break;
        case 16u:case 19u:case 20u:{
          MicroFrame frame=path;frame.pc=path.pc+1u;frame.named=named?1u:0u;
          if(op==16u){frame.resume=1u;frame.next=0u;frame.end=path.node;}
          else if(op==19u){frame.resume=2u;frame.next=path.node+1u;frame.end=node.descendantEnd;}
          else{frame.resume=3u;frame.next=named?node.nextNamed:node.nextAny;}
          uint candidate=micro_next(nodes,children,frame);
          if(candidate==MICRO_NONE){alive=false;break;}
          if(depth==MICRO_FRAMES)MICRO_FAIL
          stack[depth++]=frame;path.node=candidate;
        }break;
        case 21u:if(arg==0u)path.cursor0=path.node;else path.cursor1=path.node;break;
        case 22u:{uint saved=arg==0u?path.cursor0:path.cursor1;if(saved==MICRO_NONE)MICRO_FAIL path.node=saved;}break;
        case 32u:if(path.captured==0u)path.capture0=path.node;else if(path.captured==1u)path.capture1=path.node;else MICRO_FAIL path.captured++;break;
        case 48u:{if(depth==MICRO_FRAMES)MICRO_FAIL MicroFrame frame=path;frame.pc=arg;frame.resume=0u;stack[depth++]=frame;}break;
        case 49u:path.pc=arg;continue;
        case 64u:
          // As label_sink routes: 0 the start (once), 1 the captured nodes, 2 a tally by start,
          // 3 a tally by captured node, one each path.
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
    // The next frame with a path left.
    while(true){
      if(depth==0u)return;
      if(stack[depth-1u].resume==0u){path=stack[--depth];break;}
      uint candidate=micro_next(nodes,children,stack[depth-1u]);
      if(candidate==MICRO_NONE){depth--;continue;}
      path=stack[depth-1u];path.node=candidate;break;
    }
  }
  #undef MICRO_FAIL
}
