// Cooperative Lua continuations share the predicate request frontier.
bool lua_mask(device const uint* instruction,uchar byte){return (instruction[4u+uint(byte)/32u]&(1u<<(uint(byte)&31u)))!=0u;}
void lua_invalid(device atomic_uint* status,threadgroup RegexRequest& r){atomic_store_explicit(status,2u,memory_order_relaxed);r.status=4u;}
bool lua_charge(device atomic_uint* work,device atomic_uint* failures,constant RegexParams& p,threadgroup RegexRequest& r,uint amount){
 uint room=r.budget-r.used,charged=min(amount,room+1u),previous=regex_charge(work,r.owner,charged,p.owner_work_limit);
 r.used+=min(charged,p.owner_work_limit+1u-min(previous,p.owner_work_limit+1u));
 if(amount>room || previous>p.owner_work_limit || charged>p.owner_work_limit-previous){r.status=3u;if(p.abort_evaluation)atomic_store_explicit(failures+r.evaluation,2u,memory_order_relaxed);return false;}return true;
}
bool lua_frame_valid(device const LuaFrame* f,device const uint* table,uint depth,uint top,uint n){
 uint items=table[4],caps=table[0],at=f->at,phase=f->phase;
 if(at>n || phase>7u || (depth<top && phase!=3u && phase!=4u) || (depth==top && (phase==3u || phase==4u)))return false;
 bool idle=!(f->minimum || f->maximum || f->choice || f->scan || f->balance_depth || f->reference_start);
 if(phase==0u && !idle)return false;
 if(depth==items){if(phase!=0u)return false;}
 else {
  device const uint* instruction=table+8u+depth*12u;uint kind=instruction[0];
  if(phase==0u){}
  else if(phase==1u || phase==2u || phase==3u){
   uint rep=instruction[1],minimum=uint(rep==0u || rep==4u);
   if(kind!=0u || f->minimum!=minimum || f->scan<at || f->scan>n || f->balance_depth || f->reference_start)return false;
   if(phase==1u){if(f->maximum || f->choice)return false;}
   else if(f->maximum<minimum || f->maximum>n-at || (rep<=1u && f->maximum>1u) || (f->choice!=NONE && (f->choice<minimum || f->choice>f->maximum)))return false;
  }else if(phase==5u){
   if(kind!=2u || f->scan<=at || f->scan>n || !f->balance_depth || f->balance_depth>f->scan-at || f->minimum || f->maximum || f->choice || f->reference_start)return false;
  }else if(phase==6u){
   if(kind!=6u)return false;device const LuaCapture* capture=lua_captures(f)+instruction[1];
   if(capture->tag!=2u || capture->end<capture->start || capture->end>at || f->scan<at || capture->end-capture->start>n-at || f->scan-at>capture->end-capture->start || f->reference_start!=capture->start || f->minimum || f->maximum || f->choice || f->balance_depth)return false;
  }else if(phase==4u || phase==7u){
   if(kind==0u || !idle)return false;
  }
 }
 for(uint slot=0u;slot<caps;slot++){
  device const LuaCapture* c=lua_captures(f)+slot;uint tag=c->tag;
  if(tag>3u || ((tag==0u || tag==3u) && (c->start || c->end)) || (tag==1u && (c->start>at || c->end)) || (tag==2u && (c->start>c->end || c->end>at)))return false;
 }
 return true;
}
void lua_frontier_advance(
 device const uchar* data,device const uint* code,device RegexRequest* requests,device const uint* ids,
 device uint* arena,threadgroup uint4& pending,device atomic_uint* status,device atomic_uint* work,device atomic_uint* failures,
 constant RegexParams& p,uint group,uint tid,threadgroup RegexRequest& r,threadgroup LuaControl& s,threadgroup uint* lanes,threadgroup atomic_uint* reduced){
 if(tid==0u){
  pending=uint4(0u);s.request_id=ids[group];r.status=4u;
  if(s.request_id>=p.requests)atomic_store_explicit(status,2u,memory_order_relaxed);
  else {
   r=requests[s.request_id];View v=view(data);
   bool valid=p.abort_evaluation<=1u && p.unit_actions && p.unit_actions<=65536u && r.value<code[0] && r.status==0u
    && r.phase==uint(p.generation!=0u) && !r.mode && !r.search_start && r.at<=r.source_length
    && r.owner<p.budget_owners && r.evaluation<p.evaluations && r.budget<=REGEX_WORK_LIMIT && r.used<=r.budget
    && r.source_start<=v.header->source_bytes && r.source_length<=v.header->source_bytes-r.source_start
    && r.match_start==NONE && r.match_end==NONE && (r.phase?r.used!=0u:(!r.used && !r.at));
   uint descriptor=code[1]+r.value*4u;
   if(valid){
    s.item_count=code[descriptor];s.table_offset=code[descriptor+2u];
    valid=code[descriptor+3u]==1u && r.width==code[descriptor+1u] && r.width && !(r.width&(r.width-1u))
     && r.scratch<=p.scratch_slots && r.width<=p.scratch_slots-r.scratch && s.table_offset<=p.code_words && 8u<=p.code_words-s.table_offset;
   }
   if(valid){
    device const uint* table=code+s.table_offset;s.frame_words=table[2];s.capture_slots=table[0];
    valid=s.capture_slots<=32u && table[1]<=3u && s.frame_words==8u+3u*s.capture_slots && table[4]==s.item_count && !table[5] && !table[6] && !table[7]
     && ulong(table[3])==8ul+(ulong(s.item_count)+1ul)*s.frame_words && table[3]<=r.width && r.width/2u<table[3]
     && ulong(s.item_count)*12ul<=p.code_words-s.table_offset-8u;
   }
   if(valid && p.generation){
    device const LuaStackHeader* h=(device const LuaStackHeader*)(arena+r.scratch);s.depth=h->depth;s.attempt=h->attempt;s.actions=h->actions;
    valid=h->magic==LUA_ARENA_MAGIC && h->generation==p.generation && s.depth<=s.item_count && s.attempt==r.at && s.attempt<=r.source_length
     && s.actions!=0u && h->value==r.value && h->budget_owner==r.owner && h->failure_owner==r.evaluation;
   }else if(valid){s.depth=s.attempt=s.actions=0u;}
   s.initial_actions=s.actions;if(!valid)lua_invalid(status,r);
  }
 }
 threadgroup_barrier(mem_flags::mem_threadgroup);
 if(r.status!=0u)return;
 if(!p.generation){for(uint i=tid;i<r.width;i+=256u)arena[r.scratch+i]=0u;}
 else {for(uint d=tid;d<=s.depth;d+=256u)if(!lua_frame_valid(lua_frame(arena,r.scratch,d,s.frame_words),code+s.table_offset,d,s.depth,r.source_length))atomic_store_explicit(status,2u,memory_order_relaxed);}
 threadgroup_barrier(mem_flags::mem_device);
 if(tid==0u && atomic_load_explicit(status,memory_order_relaxed))r.status=4u;
 threadgroup_barrier(mem_flags::mem_threadgroup);
 if(r.status!=0u)return;
 View v=view(data);device const uchar* bytes=v.source+r.source_start;
 for(uint step=0u;step<p.unit_actions;step++){
  if(tid==0u){
   s.action=NONE;s.push=s.pop=s.reset=0u;
   uint failure=p.abort_evaluation?atomic_load_explicit(failures+r.evaluation,memory_order_relaxed):0u;
   if(failure==2u)r.status=5u;
   else if(failure || atomic_load_explicit(status,memory_order_relaxed))lua_invalid(status,r);
   if(!r.status){
    uint depth=s.depth;device LuaFrame* f=lua_frame(arena,r.scratch,depth,s.frame_words);uint at=f->at,phase=f->phase;
    device const uint* instruction=code+s.table_offset+8u+min(depth,s.item_count)*12u;
    uint kind=depth==s.item_count?7u:instruction[0];
    if(phase==0u){
     if(lua_charge(work,failures,p,r,1u)){
      if(kind==7u){if(!(code[s.table_offset+1u]&2u) || at==r.source_length)r.status=2u;else s.pop=1u;}
      else if(kind==0u){f->phase=1u;f->minimum=uint(instruction[1]==0u || instruction[1]==4u);f->scan=at;}
      else if(kind==1u){
       uchar previous=at?bytes[at-1u]:0u,current=at<r.source_length?bytes[at]:0u;
       if(!lua_mask(instruction,previous) && lua_mask(instruction,current)){f->phase=4u;s.child_at=at;s.push=1u;}else s.pop=1u;
      }else if(kind==2u){if(at<r.source_length && bytes[at]==instruction[1]){f->phase=5u;f->scan=at+1u;f->balance_depth=1u;}else s.pop=1u;}
      else if(kind>=3u && kind<=6u){
       device LuaCapture* capture=lua_captures(f)+instruction[1];
       if(kind==3u){capture->tag=1u;capture->start=at;capture->end=0u;}
       else if(kind==4u){if(capture->tag!=1u)s.pop=1u;else{capture->tag=2u;capture->end=at;}}
       else if(kind==5u){capture->tag=3u;capture->start=capture->end=0u;}
       else {
        if(capture->tag!=2u || capture->start>capture->end || capture->end>at || capture->end-capture->start>r.source_length-at)s.pop=1u;
        else if(capture->start!=capture->end){f->phase=6u;f->scan=at;f->reference_start=capture->start;}
       }
       if(!s.pop && (kind!=6u || capture->start==capture->end)){f->phase=4u;s.child_at=at;s.push=1u;}
      }else lua_invalid(status,r);
     }
    }else if(phase==1u || phase==5u || phase==6u){
     s.action=phase;
     if(phase==6u){device const LuaCapture* capture=lua_captures(f)+instruction[1];s.source_at=f->scan;s.chunk=min(256u,f->at+(capture->end-capture->start)-f->scan);s.reference_at=f->reference_start+(f->scan-f->at);}
     else {s.source_at=f->scan;s.chunk=min(256u,r.source_length-f->scan);if(phase==1u && instruction[1]<=1u)s.chunk=min(s.chunk,1u);}
     atomic_store_explicit(reduced,NONE,memory_order_relaxed);
    }else if(phase==2u){
     uint choice=f->choice;if(choice==NONE)s.pop=1u;
     else {s.child_at=at+choice;s.push=1u;f->choice=instruction[1]==3u?(choice==f->maximum?NONE:choice+1u):(choice==f->minimum?NONE:choice-1u);f->phase=3u;}
    }else if(phase==7u)s.pop=1u;
    else lua_invalid(status,r);
   }
  }
  threadgroup_barrier(mem_flags::mem_threadgroup|mem_flags::mem_device);
  if(r.status)break;
  uint depth=s.depth;device LuaFrame* f=lua_frame(arena,r.scratch,depth,s.frame_words);device const uint* instruction=code+s.table_offset+8u+min(depth,s.item_count)*12u;
  if(s.action==1u){
   if(tid<s.chunk && !lua_mask(instruction,bytes[s.source_at+tid]))atomic_fetch_min_explicit(reduced,tid,memory_order_relaxed);
   threadgroup_barrier(mem_flags::mem_threadgroup);
   if(tid==0u){
    uint first=atomic_load_explicit(reduced,memory_order_relaxed);f->scan+=first==NONE?s.chunk:first;
    if(first!=NONE || f->scan==r.source_length || instruction[1]<=1u){
     uint available=f->scan-f->at;f->maximum=instruction[1]==0u?1u:(instruction[1]==1u?min(available,1u):available);
     if(available<f->minimum)s.pop=1u;else{f->choice=instruction[1]==3u?f->minimum:f->maximum;f->phase=2u;}
    }
   }
  }else if(s.action==5u){
   int delta=0;if(tid<s.chunk){uchar byte=bytes[s.source_at+tid];delta=byte==instruction[1]?1:(byte==instruction[2]?-1:0);}
   lanes[tid]=uint(delta);threadgroup_barrier(mem_flags::mem_threadgroup);
   for(uint offset=1u;offset<256u;offset<<=1u){uint add=tid>=offset?lanes[tid-offset]:0u;threadgroup_barrier(mem_flags::mem_threadgroup);lanes[tid]+=add;threadgroup_barrier(mem_flags::mem_threadgroup);}
   if(tid<s.chunk && long(f->balance_depth)+long(int(lanes[tid]))==0l)atomic_fetch_min_explicit(reduced,tid,memory_order_relaxed);
   threadgroup_barrier(mem_flags::mem_threadgroup);
   if(tid==0u){
    uint first=atomic_load_explicit(reduced,memory_order_relaxed),amount=first==NONE?s.chunk:first+1u;
    if(lua_charge(work,failures,p,r,amount)){
     if(first!=NONE){s.child_at=s.source_at+amount;s.push=1u;f->scan=f->balance_depth=0u;f->phase=4u;}
     else if(!amount || s.source_at+amount==r.source_length)s.pop=1u;
     else {f->scan+=amount;f->balance_depth=uint(long(f->balance_depth)+long(int(lanes[amount-1u])));}
    }
   }
  }else if(s.action==6u){
   if(tid<s.chunk && bytes[s.reference_at+tid]!=bytes[s.source_at+tid])atomic_fetch_min_explicit(reduced,tid,memory_order_relaxed);
   threadgroup_barrier(mem_flags::mem_threadgroup);
   if(tid==0u){
    if(atomic_load_explicit(reduced,memory_order_relaxed)!=NONE)s.pop=1u;
    else {f->scan+=s.chunk;device const LuaCapture* capture=lua_captures(f)+instruction[1];if(f->scan-f->at==capture->end-capture->start){s.child_at=f->scan;s.push=1u;f->scan=f->reference_start=0u;f->phase=4u;}}
   }
  }
  threadgroup_barrier(mem_flags::mem_threadgroup|mem_flags::mem_device);
  if(r.status)break;
  if(s.push){
   device uint* child=(device uint*)lua_frame(arena,r.scratch,depth+1u,s.frame_words);
   for(uint i=tid;i<s.frame_words;i+=256u)child[i]=i<8u?(i==0u?s.child_at:0u):((device const uint*)f)[i];
   threadgroup_barrier(mem_flags::mem_device);
   if(tid==0u)s.depth++;
  }else if(s.pop){
   if(tid==0u){
    if(s.depth){s.depth--;device LuaFrame* parent=lua_frame(arena,r.scratch,s.depth,s.frame_words);if(parent->phase==3u)parent->phase=2u;else if(parent->phase==4u)parent->phase=7u;else lua_invalid(status,r);}
    else if((code[s.table_offset+1u]&1u) || s.attempt==r.source_length)r.status=1u;
    else{s.attempt++;s.reset=1u;}
   }
   threadgroup_barrier(mem_flags::mem_threadgroup);
   if(s.reset){device uint* first=arena+r.scratch+8u;for(uint i=tid;i<s.frame_words;i+=256u)first[i]=i==0u?s.attempt:0u;threadgroup_barrier(mem_flags::mem_device);}
  }
  threadgroup_barrier(mem_flags::mem_threadgroup|mem_flags::mem_device);
  if(tid==0u){if(s.actions==NONE)lua_invalid(status,r);else s.actions++;}
  threadgroup_barrier(mem_flags::mem_threadgroup);
  if(r.status)break;
 }
 if(tid==0u){
  r.at=s.attempt;if(!r.status){
   if(s.actions<=s.initial_actions || p.generation==NONE)lua_invalid(status,r);
   else {r.phase=1u;device LuaStackHeader* h=(device LuaStackHeader*)(arena+r.scratch);h->magic=LUA_ARENA_MAGIC;h->generation=p.generation+1u;h->depth=s.depth;h->attempt=s.attempt;h->actions=s.actions;h->value=r.value;h->budget_owner=r.owner;h->failure_owner=r.evaluation;}
  }
  requests[s.request_id]=r;pending=uint4(uint(!r.status),0u,0u,uint(!r.status && s.actions<=s.initial_actions));
 }
}
