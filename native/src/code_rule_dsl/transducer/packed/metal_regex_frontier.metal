// One broad kernel selects checked state storage for each request.
void lua_frontier_advance(
 device const uchar*,device const uint*,device RegexRequest*,device const uint*,
 device uint*,threadgroup uint4&,device atomic_uint*,device atomic_uint*,device atomic_uint*,
 constant RegexParams&,uint,uint,threadgroup RegexRequest&,threadgroup LuaControl&,threadgroup uint*,threadgroup atomic_uint*);

// A bounded claim cannot wrap or assign an active slot twice.
uint regex_claim(device atomic_uint* queue,uint active) {
 uint id=atomic_load_explicit(queue,memory_order_relaxed);
 while(id<active)if(atomic_compare_exchange_weak_explicit(queue,&id,id+1u,memory_order_relaxed,memory_order_relaxed))return id;
 return NONE;
}
kernel void regex_frontier_advance(
 device const uchar* data [[buffer(0)]],device const uint* code [[buffer(1)]],
 device RegexRequest* requests [[buffer(2)]],device const uint* ids [[buffer(3)]],
 device uint* current [[buffer(4)]],device atomic_uint* first_rank [[buffer(5)]],
 device atomic_uint* first_unit [[buffer(6)]],device uint* sorted [[buffer(7)]],
 device uint* next_ids [[buffer(8)]],device atomic_uint* status [[buffer(9)]],
 device atomic_uint* work [[buffer(10)]],device atomic_uint* failures [[buffer(11)]],
 device uint2* captures [[buffer(12)]],device uint2* next_captures [[buffer(13)]],
 device const uchar* external_bytes [[buffer(14)]],device uint* deletion [[buffer(15)]],
 device atomic_uint* queue [[buffer(16)]],constant RegexParams& p [[buffer(17)]],uint worker [[threadgroup_position_in_grid]],uint tid [[thread_index_in_threadgroup]]) {
 if(worker>=p.workers){if(!tid)atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 threadgroup RegexRequest r;
 threadgroup uint request_id,states,initial_at;
 threadgroup atomic_uint matched[2];
 threadgroup uint costs[256];
 threadgroup LuaControl lua_control;
 threadgroup uint engine,local_state,saved_scratch;
 threadgroup uint local_current[256],local_sorted[256];
 threadgroup atomic_uint local_rank[256],local_unit[256];
 threadgroup uint active_slot;
 threadgroup uint4 pending;
 for(;;){
  if(!tid){active_slot=atomic_load_explicit(status,memory_order_relaxed)?NONE:regex_claim(queue,p.active);pending=uint4(0u);}
  threadgroup_barrier(mem_flags::mem_threadgroup);
  if(active_slot==NONE)return;
  uint group=active_slot;
  do {
   if(tid==0u){
    engine=0u;uint id=ids[group];
    if(id<p.requests && p.code_words>=4u){uint value=requests[id].value;ulong descriptor=ulong(code[1])+ulong(value)*4ul;
     if(value<code[0] && descriptor+4ul<=p.code_words && code[uint(descriptor)+3u]==1u)engine=1u;
    }
   }
   threadgroup_barrier(mem_flags::mem_threadgroup);
   if(engine){lua_frontier_advance(data,code,requests,ids,current,pending,status,work,failures,p,group,tid,r,lua_control,costs,matched);break;}
   if(tid==0u){
    local_state=0u;pending=uint4(0u);request_id=ids[group];
    if(request_id>=p.requests){atomic_store_explicit(status,2u,memory_order_relaxed);r.status=4u;}
    else {
     r=requests[request_id];initial_at=r.mode==2u?r.progress:r.at;View v=view(data);
     uint source_size=p.external_source?p.source_bytes:v.header->source_bytes;
     device const uchar* source=p.external_source?external_bytes:v.source;
     bool continuation=r.mode==2u?
      (r.phase<=2u && ulong(r.progress)==ulong(p.generation)*p.unit_bytes
       && r.strip_cursor<=r.strip_limit && r.strip_limit<=r.source_length && ulong(r.strip_next)<=ulong(r.source_length)+1ul):
      (r.phase==uint(p.generation!=0u) && ulong(r.at)==ulong(r.search_start)+ulong(p.generation)*p.unit_bytes
       && (r.phase==0u?r.used==0u:r.used!=0u) && !r.progress && !r.strip_cursor && !r.strip_limit && !r.strip_next);
     bool valid=(!p.scratch_plan || (!p.external_source && !r.mode)) && p.scratch_plan<=1u && (!p.scratch_plan || p.unit_bytes==REGEX_LOCAL_BYTES) && p.abort_evaluation<=1u && p.unit_bytes!=0u && p.unit_bytes<=65536u && p.code_words>=4u && r.value<code[0] && r.status==0u && continuation && p.external_source<=1u && p.owner_work_limit<=REGEX_WORK_LIMIT
      && (p.external_source?r.mode==2u:r.mode<=1u)
      && (r.mode || !r.search_start) && r.search_start<=r.at && r.at<=r.source_length && r.owner<p.budget_owners && r.evaluation<p.evaluations
      && r.budget<=p.owner_work_limit && r.used<=r.budget && r.source_start<=source_size
      && r.source_length<=source_size-r.source_start
      && ((r.match_start==NONE && r.match_end==NONE) || (r.mode && r.phase && r.match_start<=r.match_end && r.match_start>=r.search_start && r.match_end<=r.at));
     if(valid){
      uint descriptor=code[1]+r.value*4u;
      valid=descriptor<=p.code_words && 4u<=p.code_words-descriptor;
      if(valid){states=code[descriptor];uint width=code[descriptor+1u];uint table=code[descriptor+2u];
       local_state=uint(p.scratch_plan==1u && regex_workgroup_predicate(code,r.value,width,r.source_length,r.mode));
       uint physical=local_state?0u:width;
       valid=code[descriptor+3u]==0u && states!=0u && width==r.width && width>=states && (width&(width-1u))==0u
        && (local_state || width<=p.auxiliary_width)
        && r.scratch<=p.scratch_slots && physical<=p.scratch_slots-r.scratch
        && table<=p.code_words && 100u<=p.code_words-table;
      }
      if(valid){device const uchar* bytes=source+r.source_start;
       valid=(!r.source_length || ((bytes[0]&0xc0u)!=0x80u))
        && (r.source_start+r.source_length==source_size || (source[r.source_start+r.source_length]&0xc0u)!=0x80u);
      }
     }
     if(!valid){atomic_store_explicit(status,2u,memory_order_relaxed);r.status=4u;}
    }
   }
   threadgroup_barrier(mem_flags::mem_threadgroup);
   if(r.status!=0u)break;
   if(local_state){
    if(!tid){saved_scratch=r.scratch;r.scratch=0u;}
    threadgroup_barrier(mem_flags::mem_threadgroup);
    regex_nfa_advance(data,code,local_current,local_rank,local_unit,local_sorted,status,work,failures,captures,next_captures,external_bytes,deletion,p,tid,r,states,matched,costs,0u);
    threadgroup_barrier(mem_flags::mem_threadgroup);
    if(!tid){
     if(!r.status){r.status=4u;atomic_store_explicit(status,2u,memory_order_relaxed);}
     r.scratch=saved_scratch;
    }
    threadgroup_barrier(mem_flags::mem_threadgroup);
   }else regex_nfa_advance(data,code,current,first_rank,first_unit,sorted,status,work,failures,captures,next_captures,external_bytes,deletion,p,tid,r,states,matched,costs,worker*p.auxiliary_width);
   if(tid==0u){requests[request_id]=r;pending=uint4(uint(r.status==0u),0u,0u,uint(r.status==0u && (r.mode==2u?r.progress:r.at)<=initial_at));}

  }while(false);
  // The same worker owns its auxiliary pool slice until every lane retires.
  threadgroup_barrier(mem_flags::mem_threadgroup|mem_flags::mem_device);
  if(!tid){
   if(pending.x>1u || pending.y || pending.z || pending.w)atomic_store_explicit(status,2u,memory_order_relaxed);
   else if(pending.x){uint at=atomic_fetch_add_explicit(queue+1u,1u,memory_order_relaxed);
    if(at>=p.active)atomic_store_explicit(status,2u,memory_order_relaxed);else next_ids[at]=ids[group];}
  }
  threadgroup_barrier(mem_flags::mem_threadgroup|mem_flags::mem_device);
 }
}
