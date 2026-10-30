// A recursive group's sweep (06-first-slice S4, O25): one thread per file runs the group's work in
// node order (preorder on even rounds, reverse preorder on odd ones or where the round's word has
// bit 31), each node's in the order of its labels (`local_levels`). A task's results mark its label
// at once (`SWEEP_MARK` in `run_task`), so later work of the same sweep reads it; a complement op
// sets its label at the node from the label it complements. A recursion that runs one way settles
// in one sweep; mixed directions alternate. Without complements labels only grow, so the sweeps
// reach the rounds' fixpoint; with them every walk moves forward in preorder (`matcher::lower`),
// so a reverse sweep computes each node from final verdicts. The sweep's words follow the labels in
// the prefix: the round's word (bits 0..16 the round, 16..31 the group), a task failed, then per
// group whether its even and its odd rounds changed a label, which the host reads at the end; a
// round after one that changed nothing does nothing. The order follows the tasks: per file the
// start of its entries (and the end), then entries {task index or 0x80000000 | op, node} by (node, level), then the ops'
// {target, of}.
#define SWEEP_TASK(T) {uint sweepTask=(T);lane.generation+=2u;device Evaluation& evaluation=evaluations[tasks[sweepTask].evaluation];for(uint kind=0u;kind<=9u;kind++)lane.quotas[kind].limit=evaluation.quotas[kind].limit;for(uint kind=1u;kind<=9u;kind++)atomic_store_explicit(&lane.quotas[kind].used,0u,memory_order_relaxed);run_task(tasks,header,files,nodes,programs,words,longs,children,supertypes,pending,captures,cursors,repeats,schedulerStorage,results,statuses,values,strings,nfaStates,nfaData,nfaRanges,luaInstructions,luaMembers,assertions,assertionData,stringBytes,source,valueWork,lineStarts,finalCaptures,outputStorage,sweepTask,lane,evaluation);Status sweepStatus=statuses[sweepTask];if(sweepStatus.error!=0u)failed=true;else if((sweepStatus.reserved&0x80000000u)!=0u)changed=true;}
#define SWEEP_ENTRY(E) {uint item=list[2u*(E)];if((item&0x80000000u)==0u)SWEEP_TASK(item)else{uint op=item&0x7fffffffu,node=list[2u*(E)+1u],bit=1u<<(node%32u);device atomic_uint* target=&labelBits[ops[2u*op]*labelWords+node/32u];bool set=(atomic_load_explicit(&labelBits[ops[2u*op+1u]*labelWords+node/32u],memory_order_relaxed)&bit)==0u;uint old=set?atomic_fetch_or_explicit(target,bit,memory_order_relaxed):atomic_fetch_and_explicit(target,~bit,memory_order_relaxed);if(((old&bit)!=0u)!=set)changed=true;}}
kernel void label_sweep(/*PARAMETERS*/uint tid [[thread_position_in_grid]],uint lanesCount [[threads_per_grid]]){
 device uint* head=(device uint*)supertypes;
 device atomic_uint* sweepFlags=(device atomic_uint*)(head+2u+(head[0]&0x7fffffffu)*head[1]);
 uint roundWord=atomic_load_explicit(&sweepFlags[0],memory_order_relaxed),round=roundWord&0xffffu;
 device atomic_uint* changes=sweepFlags+2u+2u*((roundWord>>16u)&0x7fffu);
 bool reverse=(roundWord>>31u)!=0u||round%2u!=0u;
 uint labelWords=head[1];device atomic_uint* labelBits=(device atomic_uint*)(head+2u);
 if(round>0u&&atomic_load_explicit(&changes[(round-1u)%2u],memory_order_relaxed)==0u)return;
 if(tid>=header.fileCount)return;
 device GlobalArenas& globalArenas=*((device GlobalArenas*)schedulerStorage);
 device Evaluation* evaluations=(device Evaluation*)(schedulerStorage+sizeof(GlobalArenas));
 device Evaluation& lane=evaluations[header.evaluationCount+tid];
 uint ignored=0u;
 if((lane.pending.limit&&ensure_lazy_block(lane.pending,globalArenas.arenas[0],ignored))||
    (lane.capture.limit&&ensure_lazy_block(lane.capture,globalArenas.arenas[1],ignored))||
    (lane.cursor.limit&&ensure_lazy_block(lane.cursor,globalArenas.arenas[2],ignored))||
    (lane.repeat.limit&&ensure_lazy_block(lane.repeat,globalArenas.arenas[3],ignored))||
    (lane.valueState.limit&&ensure_lazy_block(lane.valueState,globalArenas.arenas[8],ignored))||
    (lane.candidate.limit&&ensure_lazy_block(lane.candidate,globalArenas.arenas[9],ignored))||
    (lane.queryBucket.limit&&ensure_lazy_block(lane.queryBucket,globalArenas.arenas[4],ignored))||
    (lane.queryMember.limit&&ensure_lazy_block(lane.queryMember,globalArenas.arenas[5],ignored))||
    (lane.queuedState.limit&&ensure_lazy_block(lane.queuedState,globalArenas.arenas[6],ignored))||
    (lane.greedyEntry.limit&&ensure_lazy_block(lane.greedyEntry,globalArenas.arenas[7],ignored))){
  atomic_fetch_or_explicit(&sweepFlags[1],1u,memory_order_relaxed);
  return;
 }
 device const uint* order=(device const uint*)(tasks+header.taskCount);
 device const uint* list=order+header.fileCount+1u;
 device const uint* ops=list+2u*order[header.fileCount];
 bool changed=false,failed=false;
 for(uint file=tid;file<header.fileCount;file+=lanesCount){
  uint start=order[file],end=order[file+1u];
  if(!reverse){for(uint at=start;at<end;at++)SWEEP_ENTRY(at)}
  else{uint at=end;while(at>start){uint first=at-1u;uint node=list[2u*first+1u];while(first>start&&list[2u*first-1u]==node)first--;for(uint entry=first;entry<at;entry++)SWEEP_ENTRY(entry);at=first;}}
 }
 if(failed)atomic_fetch_or_explicit(&sweepFlags[1],1u,memory_order_relaxed);
 if(changed)atomic_fetch_or_explicit(&changes[round%2u],1u,memory_order_relaxed);
}
