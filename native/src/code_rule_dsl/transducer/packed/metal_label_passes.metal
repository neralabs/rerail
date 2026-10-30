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
struct Shape { uint nodeCount,words,resultArena,taskCount,itemCount,targetCount; };
// How a program's results land: mode 0 roots, 1 captured nodes (into scratch label `slot`), 2 and
// 3 counts by root and by captured node (into count slot `slot`).
struct Route { uint mode,slot; };
// kind 1 complement of `of`, 2 count bounds of slot `slot`, 3 constrained rows from `rowStart`.
struct Op { uint kind,target,of,slot,minimum,maximum; };
struct Item { uint op,index; };
struct Row { uint root,captureStart,captureCount; };
struct Constraint { uint node,label; };

static void mark(device atomic_uint* bits,uint node){atomic_fetch_or_explicit(&bits[node/32u],1u<<(node%32u),memory_order_relaxed);}

kernel void label_sink(device const Arena* arenas [[buffer(0)]],device const Result* results [[buffer(1)]],device const FinalCapture* captures [[buffer(2)]],device const Status* statuses [[buffer(3)]],device const Route* routes [[buffer(4)]],device atomic_uint* scratch [[buffer(5)]],device atomic_uint* counts [[buffer(6)]],device atomic_uint* flags [[buffer(7)]],constant Shape& shape [[buffer(8)]],uint tid [[thread_position_in_grid]]){
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

kernel void label_derive(device const uint* labels [[buffer(0)]],device atomic_uint* scratch [[buffer(1)]],device uint* counts [[buffer(2)]],device const Op* ops [[buffer(3)]],device const Item* items [[buffer(4)]],device const Row* rows [[buffer(5)]],device const Constraint* constraints [[buffer(6)]],constant Shape& shape [[buffer(7)]],uint tid [[thread_position_in_grid]]){
  if(tid>=shape.itemCount)return;
  Item item=items[tid];Op op=ops[item.op];
  device atomic_uint* bits=scratch+op.target*shape.words;
  if(op.kind==1u){
    uint word=~labels[op.of*shape.words+item.index];
    uint last=shape.nodeCount%32u;
    if(item.index==shape.words-1u&&last!=0u)word&=(1u<<last)-1u;
    atomic_store_explicit(&bits[item.index],word,memory_order_relaxed);
  }else if(op.kind==2u){
    uint word=0u;
    for(uint bit=0u;bit<32u;bit++){uint node=item.index*32u+bit;if(node>=shape.nodeCount)break;uint count=counts[op.slot*shape.nodeCount+node];counts[op.slot*shape.nodeCount+node]=0u;if(count>=op.minimum&&count<=op.maximum)word|=1u<<bit;}
    atomic_store_explicit(&bits[item.index],word,memory_order_relaxed);
  }else{
    Row row=rows[item.index];
    for(uint at=0u;at<row.captureCount;at++){Constraint constraint=constraints[row.captureStart+at];if(((labels[constraint.label*shape.words+constraint.node/32u]>>(constraint.node%32u))&1u)==0u)return;}
    mark(bits,row.root);
  }
}

// Each of the level's targets takes its scratch bits; a change sets flags[1]; scratch is cleared.
kernel void label_settle(device uint* labels [[buffer(0)]],device uint* scratch [[buffer(1)]],device atomic_uint* flags [[buffer(2)]],device const uint* targets [[buffer(3)]],constant Shape& shape [[buffer(4)]],uint tid [[thread_position_in_grid]]){
  if(tid>=shape.targetCount*shape.words)return;
  uint at=targets[tid/shape.words]*shape.words+tid%shape.words;
  uint word=scratch[at];scratch[at]=0u;
  if(labels[at]!=word){labels[at]=word;atomic_fetch_or_explicit(&flags[1],1u,memory_order_relaxed);}
}
