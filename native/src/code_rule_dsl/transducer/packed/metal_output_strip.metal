// Shared Strip snapshots, selected requests, and exact tiled byte reconstruction.
struct StripParams {uint n,c,step,source_bytes,old_bytes,code_words,work_limit,requests;};
uint4 strip_prefix(device const uint4* offsets,device const uint4* total,uint n,uint i){return i<n?offsets[i]:total[0];}
bool strip_selected(View v,device const Capture* caps,device const uint4* meta,device const uint* owners,uint i,constant StripParams& p){
 uint owner=owners[i];if(owner>=p.n || !meta[owner].w || !caps[i].retained)return false;
 Program pg=v.programs[meta[owner].y];if(p.step>=pg.output_count)return false;
 device const uint* op=v.output_words+pg.output_start+p.step*6u;return op[0]==3u && op[1]==caps[i].label;
}
kernel void strip_capture_sizes(device const uchar* data [[buffer(0)]],device const Capture* caps [[buffer(1)]],device const uint4* meta [[buffer(2)]],device const uint* owners [[buffer(3)]],device const uint4* text [[buffer(4)]],device const uint* code [[buffer(5)]],device uint4* sizes [[buffer(6)]],device atomic_uint* status [[buffer(7)]],constant StripParams& p [[buffer(8)]],uint i [[thread_position_in_grid]]){
 if(i>=p.c)return;sizes[i]=uint4(0u);if(atomic_load_explicit(status,memory_order_relaxed))return;
 uint owner=owners[i];if(owner>=p.n){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 if(!meta[owner].w || !caps[i].retained)return;View v=view(data);uint4 t=text[i];
 if(t.z>3u || t.w || (t.z && (t.x>(t.z>=2u?p.old_bytes:v.header->source_bytes) || t.y>(t.z>=2u?p.old_bytes:v.header->source_bytes)-t.x))){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 bool selected=strip_selected(v,caps,meta,owners,i,p);uint length=selected?(t.z?t.y:caps[i].end-caps[i].start):(t.z>=2u?t.y:0u);
 uint width=0u;if(selected){Program pg=v.programs[meta[owner].y];uint value=v.output_words[pg.output_start+p.step*6u+2u];
  if(p.code_words<4u || value>=code[0] || ulong(code[1])+ulong(value)*4ul+4ul>p.code_words){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
  uint descriptor=code[1]+value*4u;width=code[descriptor+1u];
  if(code[descriptor+3u] || !width || (width&(width-1u))){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 }
 sizes[i]=uint4(length,width,uint(selected),0u);
}
kernel void strip_snapshot_bytes(device const uchar* data [[buffer(0)]],device const Capture* caps [[buffer(1)]],device const uint4* text [[buffer(2)]],device const uint4* offsets [[buffer(3)]],device const uint4* total [[buffer(4)]],device const uchar* old_bytes [[buffer(5)]],device uchar* output [[buffer(6)]],device uint* mask [[buffer(7)]],device atomic_uint* status [[buffer(8)]],constant StripParams& p [[buffer(9)]],uint i [[thread_position_in_grid]]){
 if(i>=p.source_bytes || atomic_load_explicit(status,memory_order_relaxed))return;
 if(total[0].x!=p.source_bytes){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 uint lo=0u,hi=p.c;while(lo<hi){uint mid=lo+(hi-lo)/2u;if(offsets[mid].x<=i)lo=mid+1u;else hi=mid;}
 if(!lo){atomic_store_explicit(status,2u,memory_order_relaxed);return;}uint cap=lo-1u;uint4 t=text[cap];View v=view(data);
 uint start=t.z?t.x:caps[cap].start,length=t.z?t.y:caps[cap].end-caps[cap].start,at=i-offsets[cap].x;
 uint bound=t.z>=2u?p.old_bytes:v.header->source_bytes;
 if(at>=length || start>bound || length>bound-start){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 uchar byte=t.z>=2u?old_bytes[start+at]:v.source[start+at];
 if((t.z==1u || t.z==3u) && byte>='A' && byte<='Z')byte+=('a'-'A');output[i]=byte;mask[i]=1u;
}
kernel void strip_request_scatter(device const uchar* data [[buffer(0)]],device const Capture* caps [[buffer(1)]],device const uint4* meta [[buffer(2)]],device const uint* owners [[buffer(3)]],device const uint4* offsets [[buffer(4)]],device const uint4* total [[buffer(5)]],device RegexRequest* requests [[buffer(6)]],device uint* ids [[buffer(7)]],device const uint* status [[buffer(8)]],constant StripParams& p [[buffer(9)]],uint i [[thread_position_in_grid]]){
 if(i>=p.c || status[0])return;View v=view(data);if(!strip_selected(v,caps,meta,owners,i,p))return;
 uint owner=owners[i];Program pg=v.programs[meta[owner].y];uint value=v.output_words[pg.output_start+p.step*6u+2u];
 uint4 first=offsets[i],last=strip_prefix(offsets,total,p.c,i+1u);
 requests[first.z]={value,first.x,last.x-first.x,0u,p.work_limit,first.y,last.y-first.y,0u,0u,0u,0u,NONE,NONE,2u,owner,meta[owner].x,0u,0u,0u,0u};ids[first.z]=first.z;
}
kernel void strip_results(device const RegexRequest* requests [[buffer(0)]],device uint4* meta [[buffer(1)]],device const uint* failures [[buffer(2)]],device atomic_uint* status [[buffer(3)]],constant StripParams& p [[buffer(4)]],uint i [[thread_position_in_grid]]){
 if(atomic_load_explicit(status,memory_order_relaxed))return;
 if(i<p.requests && requests[i].status!=1u && requests[i].status!=3u && requests[i].status!=5u)atomic_store_explicit(status,2u,memory_order_relaxed);
 if(i<p.n && failures[meta[i].x])meta[i].w=0u;
}
kernel void strip_mask_counts(device const uint* mask [[buffer(0)]],device uint4* counts [[buffer(1)]],device atomic_uint* status [[buffer(2)]],constant StripParams& p [[buffer(3)]],uint group [[threadgroup_position_in_grid]],uint lane [[thread_index_in_threadgroup]]){
 if(group>=(p.source_bytes+255u)/256u)return;
 uint i=group*256u+lane,keep=i<p.source_bytes?mask[i]:0u;if(keep>1u){atomic_store_explicit(status,2u,memory_order_relaxed);keep=0u;}
 threadgroup uint sums[256];sums[lane]=keep;threadgroup_barrier(mem_flags::mem_threadgroup);
 for(uint width=128u;width;width>>=1u){if(lane<width)sums[lane]+=sums[lane+width];threadgroup_barrier(mem_flags::mem_threadgroup);}
 if(!lane)counts[group]=uint4(sums[0],0u,0u,0u);
}
kernel void strip_publish_bytes(device const uchar* source [[buffer(0)]],device const uint* mask [[buffer(1)]],device const uint4* offsets [[buffer(2)]],device uchar* output [[buffer(3)]],device const uint* status [[buffer(4)]],constant StripParams& p [[buffer(5)]],uint group [[threadgroup_position_in_grid]],uint lane [[thread_index_in_threadgroup]]){
 if(group>=(p.source_bytes+255u)/256u || status[0])return;
 uint i=group*256u+lane,keep=i<p.source_bytes?mask[i]:0u;threadgroup uint ranks[256];ranks[lane]=keep;threadgroup_barrier(mem_flags::mem_threadgroup);
 for(uint width=1u;width<256u;width<<=1u){uint value=ranks[lane]+(lane>=width?ranks[lane-width]:0u);threadgroup_barrier(mem_flags::mem_threadgroup);ranks[lane]=value;threadgroup_barrier(mem_flags::mem_threadgroup);}
 if(keep)output[offsets[group].x+ranks[lane]-1u]=source[i];
}
uint strip_kept_before(uint at,device const uint* mask,device const uint4* offsets,device const uint4* total,constant StripParams& p){
 if(at==p.source_bytes)return total[0].x;uint tile=at/256u,count=offsets[tile].x;
 for(uint i=tile*256u;i<at;i++)count+=mask[i];return count;
}
kernel void strip_publish_captures(device const uchar* data [[buffer(0)]],device const Capture* caps [[buffer(1)]],device const uint4* meta [[buffer(2)]],device const uint* owners [[buffer(3)]],device uint4* text [[buffer(4)]],device const uint4* source_offsets [[buffer(5)]],device const uint4* source_total [[buffer(6)]],device const uint* mask [[buffer(7)]],device const uint4* offsets [[buffer(8)]],device const uint4* total [[buffer(9)]],device atomic_uint* status [[buffer(10)]],constant StripParams& p [[buffer(11)]],uint i [[thread_position_in_grid]]){
 if(i>=p.c || atomic_load_explicit(status,memory_order_relaxed))return;uint owner=owners[i];if(owner>=p.n){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 if(!meta[owner].w || !caps[i].retained){text[i]=uint4(0u);return;}View v=view(data);
 if(text[i].z<2u && !strip_selected(v,caps,meta,owners,i,p))return;
 uint first=source_offsets[i].x,last=strip_prefix(source_offsets,source_total,p.c,i+1u).x;
 if(first>last || last>p.source_bytes){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 uint start=strip_kept_before(first,mask,offsets,total,p),end=strip_kept_before(last,mask,offsets,total,p);
 if(start>end || end>total[0].x){atomic_store_explicit(status,2u,memory_order_relaxed);return;}text[i]=uint4(start,end-start,2u,0u);
}
