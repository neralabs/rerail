// The same ordered NFA operations serve device and workgroup state.
uint regex_owner_rank(device const uint* unit,threadgroup const RegexRequest& r,REGEX_ADDRESS const uint* current) {
 uint owner=unit[0];
 if(owner==NONE)return r.phase==0u?0u:NONE;
 return r.phase==1u?current[r.scratch+owner]:NONE;
}
bool regex_unit_enabled(device const uint* unit,threadgroup const RegexRequest& r,REGEX_ADDRESS const uint* current,device const uchar* bytes) {
 if(regex_owner_rank(unit,r,current)==NONE)return false;
 if(unit[1]==1u || unit[0]==NONE)return true;
 uchar byte=bytes[r.at-1u];return byte>=unit[2] && byte<=unit[3];
}
bool regex_unit_executed(device const uint* units,uint i,threadgroup const RegexRequest& r,REGEX_ADDRESS const uint* current,REGEX_ADDRESS const uint* first_rank,REGEX_ADDRESS const uint* first_unit,uint temporary_base) {
 device const uint* unit=units+i*8u;
 if(unit[1]==1u || unit[5]==NONE)return true;
 uint parent=unit[5],state=units[parent*8u+4u];
 return first_rank[temporary_base+state]==regex_owner_rank(unit,r,current) && first_unit[temporary_base+state]==parent;
}
uint2 regex_state_key(uint state,uint states,device const uint* units,uint base,REGEX_ADDRESS const uint* first_rank,REGEX_ADDRESS const uint* first_unit,uint2 cutoff) {
 if(state>=states || first_rank[base+state]==NONE)return uint2(NONE);
 uint unit=first_unit[base+state];
 uint2 key=uint2(first_rank[base+state],unit);
 return units[unit*8u+7u]==0u && regex_pair_less(key,cutoff)?key:uint2(NONE);
}
void regex_nfa_advance(device const uchar* data,device const uint* code,
 REGEX_ADDRESS uint* current,REGEX_ADDRESS atomic_uint* first_rank,
 REGEX_ADDRESS atomic_uint* first_unit,REGEX_ADDRESS uint* sorted,
 device atomic_uint* status,device atomic_uint* work,device atomic_uint* failures,
 device uint2* captures,device uint2* next_captures,device const uchar* external_bytes,
 device uint* deletion,constant RegexParams& p,uint tid,threadgroup RegexRequest& r,
 uint states,threadgroup atomic_uint* matched,threadgroup uint* costs,uint temporary_base) {
 if(r.phase==0u)for(uint i=tid;i<r.width;i+=256u)current[r.scratch+i]=NONE;
 threadgroup_barrier(mem_flags::mem_device|mem_flags::mem_threadgroup);
 View v=view(data);device const uchar* bytes=(p.external_source?external_bytes:v.source)+r.source_start;
 for(uint step=0u;step<p.unit_bytes;step++) {
  if(tid==0u){uint failure=p.abort_evaluation?atomic_load_explicit(failures+r.evaluation,memory_order_relaxed):0u;
   if(r.mode==2u){if(r.progress==NONE){r.status=4u;atomic_store_explicit(status,2u,memory_order_relaxed);}else r.progress++;}
   if(failure==2u)r.status=5u;
   else if(failure || atomic_load_explicit(status,memory_order_relaxed)){r.status=4u;atomic_store_explicit(status,2u,memory_order_relaxed);}
  }
  threadgroup_barrier(mem_flags::mem_threadgroup);
  if(r.status!=0u)break;
  if(r.mode==2u && r.phase==2u){
   uint count=min(256u,r.strip_limit-r.strip_cursor);
   if(tid<count)deletion[r.source_start+r.strip_cursor+tid]=0u;
   threadgroup_barrier(mem_flags::mem_device|mem_flags::mem_threadgroup);
   if(!tid){r.strip_cursor+=count;if(r.strip_cursor==r.strip_limit){
    if(r.strip_next>r.source_length)r.status=1u;
    else {r.search_start=r.strip_next;r.at=r.strip_next;r.phase=0u;r.match_start=r.match_end=NONE;}
   }}
   threadgroup_barrier(mem_flags::mem_threadgroup);
   if(r.status)break;
   if(!r.phase)for(uint i=tid;i<r.width;i+=256u)current[r.scratch+i]=NONE;
   threadgroup_barrier(mem_flags::mem_device|mem_flags::mem_threadgroup);continue;
  }
  uint context=regex_context(bytes,r.source_length,r.at,code);
  uint table=code[code[1]+r.value*4u+2u];
  if(context==NONE){if(tid==0u){r.status=4u;atomic_store_explicit(status,2u,memory_order_relaxed);}threadgroup_barrier(mem_flags::mem_threadgroup);break;}
  uint unit_start=code[table+context*2u],unit_count=code[table+context*2u+1u];
  if(unit_start>p.code_words || ulong(unit_count)*8ul>p.code_words-unit_start){if(tid==0u){r.status=4u;atomic_store_explicit(status,2u,memory_order_relaxed);}threadgroup_barrier(mem_flags::mem_threadgroup);break;}
  device const uint* units=code+unit_start;
  for(uint i=tid;i<r.width;i+=256u){atomic_store_explicit(first_rank+temporary_base+i,NONE,memory_order_relaxed);atomic_store_explicit(first_unit+temporary_base+i,NONE,memory_order_relaxed);sorted[temporary_base+i]=i;}
  if(tid==0u){atomic_store_explicit(matched,NONE,memory_order_relaxed);atomic_store_explicit(matched+1u,NONE,memory_order_relaxed);}
  threadgroup_barrier(mem_flags::mem_device|mem_flags::mem_threadgroup);
  for(uint i=tid;i<unit_count;i+=256u){device const uint* unit=units+i*8u;
   if(unit[1]==0u && regex_unit_enabled(unit,r,current,bytes))atomic_fetch_min_explicit(first_rank+temporary_base+unit[4],regex_owner_rank(unit,r,current),memory_order_relaxed);
  }
  threadgroup_barrier(mem_flags::mem_device|mem_flags::mem_threadgroup);
  for(uint i=tid;i<unit_count;i+=256u){device const uint* unit=units+i*8u;
   if(unit[1]==0u && regex_unit_enabled(unit,r,current,bytes) && atomic_load_explicit(first_rank+temporary_base+unit[4],memory_order_relaxed)==regex_owner_rank(unit,r,current))atomic_fetch_min_explicit(first_unit+temporary_base+unit[4],i,memory_order_relaxed);
  }
  threadgroup_barrier(mem_flags::mem_device|mem_flags::mem_threadgroup);
  REGEX_ADDRESS const uint* ranks=(REGEX_ADDRESS const uint*)first_rank;REGEX_ADDRESS const uint* firsts=(REGEX_ADDRESS const uint*)first_unit;
  for(uint i=tid;i<unit_count;i+=256u){device const uint* unit=units+i*8u;
   if(unit[7]==5u && regex_unit_enabled(unit,r,current,bytes) && regex_unit_executed(units,i,r,current,ranks,firsts,temporary_base))atomic_fetch_min_explicit(matched,regex_owner_rank(unit,r,current),memory_order_relaxed);
  }
  threadgroup_barrier(mem_flags::mem_threadgroup);
  uint match_rank=atomic_load_explicit(matched,memory_order_relaxed);
  for(uint i=tid;i<unit_count;i+=256u){device const uint* unit=units+i*8u;
   if(unit[7]==5u && regex_unit_enabled(unit,r,current,bytes) && regex_owner_rank(unit,r,current)==match_rank && regex_unit_executed(units,i,r,current,ranks,firsts,temporary_base))atomic_fetch_min_explicit(matched+1u,i,memory_order_relaxed);
  }
  threadgroup_barrier(mem_flags::mem_threadgroup);
  uint2 match=uint2(match_rank,atomic_load_explicit(matched+1u,memory_order_relaxed));
  uint cost=0u;
  for(uint i=tid;i<unit_count;i+=256u){device const uint* unit=units+i*8u;
   if(regex_unit_enabled(unit,r,current,bytes) && regex_unit_executed(units,i,r,current,ranks,firsts,temporary_base)
    && (r.mode || !regex_pair_less(match,uint2(regex_owner_rank(unit,r,current),i))))cost++;
  }
  costs[tid]=cost;threadgroup_barrier(mem_flags::mem_threadgroup);
  for(uint width=128u;width; width>>=1u){if(tid<width)costs[tid]+=costs[tid+width];threadgroup_barrier(mem_flags::mem_threadgroup);}
  if(tid==0u){uint room=r.budget-r.used,charged=min(costs[0],room+1u);uint previous=regex_charge(work,r.owner,charged,p.owner_work_limit);
   r.used+=min(charged,p.owner_work_limit+1u-min(previous,p.owner_work_limit+1u));
   if(costs[0]>room || previous>p.owner_work_limit || charged>p.owner_work_limit-previous){r.status=3u;if(p.abort_evaluation)atomic_store_explicit(failures+r.evaluation,2u,memory_order_relaxed);}
   else {
    if(match.x!=NONE){
     if(!r.mode)r.status=2u;
     else {uint2 tags=regex_capture_tags(units+match.y*8u,r,captures);r.match_start=tags.x;r.match_end=tags.y;
      if(tags.x==NONE || tags.y==NONE || tags.x>tags.y || tags.x<r.search_start || tags.y>r.at){r.status=4u;atomic_store_explicit(status,2u,memory_order_relaxed);}
     }
    }
    if(!r.status && r.at==r.source_length)r.status=r.match_start==NONE?1u:2u;
   }
  }
  threadgroup_barrier(mem_flags::mem_threadgroup|mem_flags::mem_device);
  if(r.mode==2u && r.status==2u){
   if(!tid)regex_strip_match(r,bytes,status);
   threadgroup_barrier(mem_flags::mem_threadgroup);if(r.status)break;continue;
  }
  if(r.status!=0u)break;
  uint2 cutoff=r.mode?match:uint2(NONE);
  if(r.mode)for(uint state=tid;state<states;state+=256u){
   if(regex_state_key(state,states,units,temporary_base,ranks,firsts,cutoff).x!=NONE)
    next_captures[r.scratch+state]=regex_capture_tags(units+firsts[temporary_base+state]*8u,r,captures);
  }
  threadgroup_barrier(mem_flags::mem_device|mem_flags::mem_threadgroup);
  for(uint width=2u;width<=r.width;width<<=1u){
   for(uint distance=width>>1u;distance;distance>>=1u){
    for(uint i=tid;i<r.width;i+=256u){uint j=i^distance;if(j>i){uint a=sorted[temporary_base+i],b=sorted[temporary_base+j];uint2 ak=regex_state_key(a,states,units,temporary_base,ranks,firsts,cutoff),bk=regex_state_key(b,states,units,temporary_base,ranks,firsts,cutoff);bool less=regex_pair_less(ak,bk)||(all(ak==bk)&&a<b);if(less!=((i&width)==0u)){sorted[temporary_base+i]=b;sorted[temporary_base+j]=a;}}}
    threadgroup_barrier(mem_flags::mem_device|mem_flags::mem_threadgroup);
   }
   if(width==r.width)break;
  }
  for(uint i=tid;i<r.width;i+=256u)current[r.scratch+i]=NONE;
  threadgroup_barrier(mem_flags::mem_device|mem_flags::mem_threadgroup);
  for(uint i=tid;i<r.width;i+=256u){uint state=sorted[temporary_base+i];if(regex_state_key(state,states,units,temporary_base,ranks,firsts,cutoff).x!=NONE){current[r.scratch+state]=i;if(r.mode)captures[r.scratch+state]=next_captures[r.scratch+state];}}
  threadgroup_barrier(mem_flags::mem_device|mem_flags::mem_threadgroup);
  if(tid==0u){if(regex_state_key(sorted[temporary_base],states,units,temporary_base,ranks,firsts,cutoff).x==NONE)r.status=r.match_start==NONE?1u:2u;else{r.at++;r.phase=1u;}}
  threadgroup_barrier(mem_flags::mem_threadgroup);
  if(r.mode==2u && r.status==2u){
   if(!tid)regex_strip_match(r,bytes,status);
   threadgroup_barrier(mem_flags::mem_threadgroup);if(r.status)break;continue;
  }
  if(r.status!=0u)break;
 }
}
