// Stable per-frame indexes preserve occurrence pairing without moving captures.
kernel void predicate_group_init(device const CaptureSpan* rows [[buffer(0)]],device uint* ids [[buffer(1)]],device atomic_uint* maximum [[buffer(2)]],constant CaptureParams& p [[buffer(3)]],uint i [[thread_position_in_grid]]) {
 if(i<p.c)ids[i]=i;if(i<p.n)atomic_fetch_max_explicit(maximum,argument_span(rows,i,p).capture_count,memory_order_relaxed);
}
bool predicate_group_less(device const ArgumentCapture* caps,uint a,uint b,constant CaptureParams& p){uint left=argument_capture(caps,a,p).label,right=argument_capture(caps,b,p).label;return left<right || (left==right && a<b);}
kernel void predicate_group_merge(device const CaptureSpan* rows [[buffer(0)]],device const ArgumentCapture* caps [[buffer(1)]],device const uint* owners [[buffer(2)]],device const uint* ids [[buffer(3)]],device uint* output [[buffer(4)]],constant CaptureParams& p [[buffer(5)]],uint i [[thread_position_in_grid]]) {
 if(i>=p.c)return;CaptureSpan row=argument_span(rows,owners[i],p);uint local=i-row.capture_start;
 uint base=row.capture_start+(local/p.width/2u)*p.width*2u;
 uint end_row=row.capture_start+row.capture_count,middle=uint(min(ulong(end_row),ulong(base)+p.width)),end=uint(min(ulong(end_row),ulong(base)+2ul*p.width));
 bool left=i<middle;uint begin=left?middle:base,lo=begin,hi=left?end:middle,a=ids[i];
 while(lo<hi){uint mid=lo+(hi-lo)/2u;if(predicate_group_less(caps,ids[mid],a,p))lo=mid+1u;else hi=mid;}
 output[base+i-(left?base:middle)+lo-begin]=a;
}
kernel void predicate_group_inverse(device const uint* ids [[buffer(0)]],device uint* ranks [[buffer(1)]],constant CaptureParams& p [[buffer(2)]],uint i [[thread_position_in_grid]]) {if(i<p.c)ranks[ids[i]]=i;}
uint predicate_group_start(device const ArgumentCapture* caps,device const uint* ids,CaptureSpan row,uint label,constant CaptureParams& p) {
 uint lo=row.capture_start,hi=lo+row.capture_count;while(lo<hi){uint mid=lo+(hi-lo)/2u;if(argument_capture(caps,ids[mid],p).label<label)lo=mid+1u;else hi=mid;}return lo;
}
uint predicate_partner(device const ArgumentCapture* caps,device const uint* ids,device const uint* ranks,CaptureSpan row,uint i,uint label,constant CaptureParams& p) {
 uint first=predicate_group_start(caps,ids,row,argument_capture(caps,i,p).label,p),right=predicate_group_start(caps,ids,row,label,p);
 ulong at=ulong(right)+ranks[i]-first;
 return at<ulong(row.capture_start)+row.capture_count && argument_capture(caps,ids[uint(at)],p).label==label?ids[uint(at)]:NONE;
}
// Kind sets are immutable compiler tables. Ancestors use subtree interval maxima.
bool predicate_kind(device const uint* code,uint set,uint kind) {
 uint at=code[4]+set*3u,lo=code[at],hi=lo+code[at+1u],end=hi;
 while(lo<hi){uint mid=lo+(hi-lo)/2u;if(code[mid]<kind)lo=mid+1u;else hi=mid;}return lo<end && code[lo]==kind;
}
kernel void predicate_ancestor_init(device const uchar* data [[buffer(0)]],device const uint* code [[buffer(1)]],device uint* values [[buffer(2)]],constant uint2& p [[buffer(3)]],uint i [[thread_position_in_grid]]) {
 if(ulong(i)>=ulong(p.x)*p.y)return;View v=view(data);uint domain=i/p.x,node=i%p.x,set=code[code[5]+domain];
 values[i]=predicate_kind(code,set,v.nodes[node].kind)?v.nodes[node].descendant_end:0u;
}
kernel void predicate_ancestor_scan(device uint* values [[buffer(0)]],device uint* sums [[buffer(1)]],constant uint2& p [[buffer(2)]],uint lane [[thread_index_in_threadgroup]],uint group [[threadgroup_position_in_grid]]) {
 uint groups=(p.x+255u)/256u,domain=group/groups,base=(group%groups)*256u,at=base+lane;
 threadgroup uint slots[256];slots[lane]=domain<p.y && at<p.x?values[domain*p.x+at]:0u;threadgroup_barrier(mem_flags::mem_threadgroup);
 for(uint step=1u;step<256u;step<<=1u){uint add=lane>=step?slots[lane-step]:0u;threadgroup_barrier(mem_flags::mem_threadgroup);slots[lane]=max(slots[lane],add);threadgroup_barrier(mem_flags::mem_threadgroup);}
 if(domain<p.y && at<p.x)values[domain*p.x+at]=slots[lane];if(domain<p.y && lane==255u)sums[group]=slots[lane];
}
kernel void predicate_ancestor_add(device uint* values [[buffer(0)]],device const uint* sums [[buffer(1)]],constant uint2& p [[buffer(2)]],uint i [[thread_position_in_grid]]) {
 if(ulong(i)>=ulong(p.x)*p.y)return;uint at=i%p.x,group=at/256u;if(group)values[i]=max(values[i],sums[(i/p.x)*((p.x+255u)/256u)+group-1u]);
}
kernel void capture_predicate_kind(device const uchar* data [[buffer(0)]],device const ArgumentCapture* caps [[buffer(1)]],device const uint4* meta [[buffer(2)]],device const uint* owners [[buffer(3)]],device atomic_uint* counts [[buffer(4)]],device const uint* code [[buffer(5)]],device const uint* ancestors [[buffer(6)]],device atomic_uint* status [[buffer(7)]],device const PredicateSpec* specs [[buffer(8)]],constant CaptureParams& p [[buffer(9)]],uint i [[thread_position_in_grid]]) {
 if(i>=p.c || atomic_load_explicit(status,memory_order_relaxed))return;uint owner=owners[i];if(!meta[owner].w)return;View v=view(data);Program pg=v.programs[meta[owner].y];
 PredicateSpec a=argument_spec(v,meta,specs,owner,p);uint index=a.index;if((a.kind!=3u && a.kind!=5u) || argument_capture(caps,i,p).label!=a.first)return;
 uint set=code[code[3]+index],node=argument_capture(caps,i,p).node;bool found;
 if(a.kind==3u)found=predicate_kind(code,set,v.nodes[node].kind);
 else if(a.flags&4u){uint domain=code[code[4]+set*3u+2u];if(domain>=code[2]){atomic_store_explicit(status,2u,memory_order_relaxed);return;}found=node && ancestors[domain*p.nodes+node-1u]>node;}
 else {uint parent=v.nodes[node].parent;found=parent!=NONE && predicate_kind(code,set,v.nodes[parent].kind);}
 atomic_fetch_add_explicit(counts+owner*4u,1u,memory_order_relaxed);
 bool answer=a.kind==5u?found:found^bool(a.flags&1u);if(answer)atomic_fetch_add_explicit(counts+owner*4u+1u,1u,memory_order_relaxed);
}
