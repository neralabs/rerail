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
struct Shape { uint nodeCount,words,resultArena,taskCount,itemCount,targetCount,round,finish,recursive,lastLevel; };
// flags: [0] a task failed, [2] the group settled (skip), [3] a group did not settle within its
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
