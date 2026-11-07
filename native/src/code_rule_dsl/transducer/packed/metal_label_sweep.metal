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
// round after one that changed nothing does nothing.
// The work at a node (decision 0033) is the group's entries in order, with no per-node list: a
// program runs where its rule admits the node's code and its evaluation for the file exists (one
// per program and file with roots), a complement sets its label from the label it complements.
// The entries (in place of the tasks): their count, then per entry {kind (0 a program, 1 a
// complement), program or target, -, the rule's word} or {1, target, of, -}; then the rules, each
// {kind count or NONE (any kind), kinds, test count, tests (op << 16 | argument)}.
inline bool sweep_admits(device const uint* rule,device const DeviceCode& code){
 uint kinds=rule[0],at=1u;
 if(kinds!=0xffffffffu){bool found=false;for(uint k=0u;k<kinds;k++)if(rule[1u+k]==code.kind){found=true;break;}if(!found)return false;at+=kinds;}
 for(uint k=0u,tests=rule[at];k<tests;k++){
  uint test=rule[at+1u+k],op=test>>16u,arg=test&0xffffu;
  if(op==1u){if(code.kind!=arg)return false;}
  else{bool found=false;for(uint m=0u;m<code.classCount;m++)if(code.classes[m]==arg){found=true;break;}if(!found)return false;}
 }
 return true;
}
// The evaluation of (file, program), or NONE: evaluations are ordered by file, then program.
inline uint sweep_evaluation(device const Evaluation* evaluations,uint count,uint file,uint program){uint low=0u,high=count;while(low<high){uint middle=(low+high)/2u;uint f=evaluations[middle].file,p=evaluations[middle].program;if(f<file||(f==file&&p<program))low=middle+1u;else high=middle;}return low<count&&evaluations[low].file==file&&evaluations[low].program==program?low:0xffffffffu;}
#define SWEEP_TASK(E,NODE) {uint sweepEvaluation=(E);lane.generation+=2u;device Evaluation& evaluation=evaluations[sweepEvaluation];for(uint kind=0u;kind<=9u;kind++)lane.quotas[kind].limit=evaluation.quotas[kind].limit;for(uint kind=1u;kind<=9u;kind++)atomic_store_explicit(&lane.quotas[kind].used,0u,memory_order_relaxed);Task own={sweepEvaluation,0u,evaluation.file,evaluation.program,(NODE),resident_work(slabs,doc,evaluation.workLimit,(NODE)-doc.nodeBase)};device Status& sweepStatus=statuses[tid];run_task(own,header,files,nodes,programs,words,longs,children,supertypes,pending,captures,cursors,repeats,schedulerStorage,results,sweepStatus,values,strings,nfaStates,nfaData,nfaRanges,luaInstructions,luaMembers,assertions,assertionData,stringBytes,source,valueWork,lineStarts,finalCaptures,outputStorage,lane,evaluation);if(sweepStatus.error!=0u)failed=true;else if((sweepStatus.reserved&0x80000000u)!=0u)changed=true;}
#define SWEEP_NODE(LOCAL) {uint sweepLocal=(LOCAL),node=doc.nodeBase+sweepLocal;device const DeviceCode& code=tape_code(slabs,doc,sweepLocal);for(uint e=0u;e<entryCount;e++){device const uint* entry=entries+4u*e;if(entry[0]==0u){if(!sweep_admits(sweep+entry[3],code))continue;uint at=sweep_evaluation(evaluations,header.evaluationCount,file,entry[1]);if(at!=0xffffffffu)SWEEP_TASK(at,node)}else{uint bit=1u<<(node%32u);device atomic_uint* target=&labelBits[entry[1]*labelWords+node/32u];bool set=(atomic_load_explicit(&labelBits[entry[2]*labelWords+node/32u],memory_order_relaxed)&bit)==0u;uint old=set?atomic_fetch_or_explicit(target,bit,memory_order_relaxed):atomic_fetch_and_explicit(target,~bit,memory_order_relaxed);if(((old&bit)!=0u)!=set)changed=true;}}}
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
 device const uint* sweep=(device const uint*)tasks;
 uint entryCount=sweep[0];device const uint* entries=sweep+1u;
 Slabs slabs={nodes,children,source,lineStarts};
 bool changed=false,failed=false;
 for(uint file=tid;file<header.fileCount;file+=lanesCount){
  DocView doc=files[file];
  if(!reverse){for(uint local=0u;local<doc.nodes;local++)SWEEP_NODE(local)}
  else{for(uint local=doc.nodes;local>0u;local--)SWEEP_NODE(local-1u)}
 }
 if(failed)atomic_fetch_or_explicit(&sweepFlags[1],1u,memory_order_relaxed);
 if(changed)atomic_fetch_or_explicit(&changes[round%2u],1u,memory_order_relaxed);
}
