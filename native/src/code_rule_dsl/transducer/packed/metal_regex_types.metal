// Shared cooperative NFA closure. No lane owns a private NFA interpreter.
struct RegexRequest {
 uint value,source_start,source_length,search_start,budget,scratch,width,at;
 uint phase,status,used,match_start,match_end,mode,owner,evaluation;
 uint progress,strip_cursor,strip_limit,strip_next;
};
struct RegexParams {uint requests,active,scratch_slots,code_words,budget_owners,evaluations,generation,unit_bytes,abort_evaluation,unit_actions,external_source,source_bytes,owner_work_limit,scratch_plan,auxiliary_width,workers;};
constant uint REGEX_WORK_LIMIT=1048576u;

uint regex_category(device const uchar* bytes,uint n,uint at,device const uint* code) {
 if(at>=n)return 0u;uchar byte=bytes[at];
 if(byte=='\n')return 1u;if(byte=='\r')return 2u;
 if(byte<128u)return ((byte>='A'&&byte<='Z')||(byte>='a'&&byte<='z')||(byte>='0'&&byte<='9')||byte=='_')?3u:4u;
 uint width=byte<0xe0u?2u:byte<0xf0u?3u:4u;
 if(byte<0xc2u || byte>0xf4u || width>n-at)return NONE;
 uint scalar=byte&((1u<<(7u-width))-1u);
 for(uint j=1u;j<4u;j++)if(j<width){uchar tail=bytes[at+j];if((tail&0xc0u)!=0x80u)return NONE;scalar=(scalar<<6u)|(tail&63u);}
 uint lo=0u,hi=code[3];device const uint* ranges=code+code[2];
 while(lo<hi){uint mid=lo+(hi-lo)/2u;if(ranges[mid*2u+1u]<scalar)lo=mid+1u;else hi=mid;}
 return lo<code[3] && ranges[lo*2u]<=scalar?5u:6u;
}
uint regex_context(device const uchar* bytes,uint n,uint at,device const uint* code) {
 if(at>n)return NONE;
 if(at<n && (bytes[at]&0xc0u)==0x80u)return 49u;
 uint previous=0u;
 if(at){uint start=at-1u;for(uint j=0u;j<3u;j++)if(start && (bytes[start]&0xc0u)==0x80u)start--;
  previous=regex_category(bytes,n,start,code);
 }
 uint current=regex_category(bytes,n,at,code);
 return previous==NONE || current==NONE?NONE:previous*7u+current;
}
bool regex_pair_less(uint2 a,uint2 b){return a.x<b.x || (a.x==b.x && a.y<b.y);}
uint2 regex_capture_tags(device const uint* unit,threadgroup const RegexRequest& r,device const uint2* captures){
 uint2 tags=uint2(NONE);if(unit[0]!=NONE)tags=captures[r.scratch+unit[0]];
 if(unit[6]&1u)tags.x=r.at;if(unit[6]&2u)tags.y=r.at;return tags;
}
uint regex_charge(device atomic_uint* work,uint owner,uint amount,uint limit) {
 uint previous=atomic_load_explicit(work+owner,memory_order_relaxed);
 while(previous<=limit){
  uint next=min(limit+1u,previous+amount);
  if(atomic_compare_exchange_weak_explicit(work+owner,&previous,next,memory_order_relaxed,memory_order_relaxed))return previous;
 }
 return previous;
}

// Deletion and restart retain the original snapshot and the complete work owner.
void regex_strip_match(threadgroup RegexRequest& r,device const uchar* bytes,device atomic_uint* status){
 uint first=r.match_start,last=r.match_end;
 bool valid=first!=NONE && first<=last && last<=r.source_length
  && (last==r.source_length || (bytes[last]&0xc0u)!=0x80u);
 if(!valid){r.status=4u;atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 r.strip_cursor=first;r.strip_limit=last;r.strip_next=last;
 if(first==last){uint width=1u;if(last<r.source_length){uchar lead=bytes[last];width=lead<0x80u?1u:lead<0xe0u?2u:lead<0xf0u?3u:4u;}
  if(ulong(last)+width>ulong(r.source_length)+1ul){r.status=4u;atomic_store_explicit(status,2u,memory_order_relaxed);return;}
  r.strip_next=last+width;
 }
 r.phase=2u;r.status=0u;
}

// A predicate over fewer than one byte chunk must finish before local state
// can retire. Lua and range-producing requests always keep device state.
constant uint REGEX_LOCAL_WIDTH=256u,REGEX_LOCAL_BYTES=4096u;
bool regex_workgroup_predicate(device const uint* code,uint value,uint width,uint length,uint mode) {
 return !mode && width<=REGEX_LOCAL_WIDTH && length<REGEX_LOCAL_BYTES
  && code[code[1]+value*4u+3u]==0u;
}
