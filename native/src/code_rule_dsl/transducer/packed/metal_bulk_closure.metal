#include <metal_stdlib>
using namespace metal;
constant uint CLOSURE_NONE=0xffffffffu;
struct ClosureParams { uint domains,entries,cells,output,cell_levels,entry_levels; };
uint closure_groups(uint n) { return max(1u,uint((ulong(n)+255ul)/256ul)); }
uint closure_level(uint n,uint level) {
 for(uint i=0u;i<level;i++) n=uint((ulong(n)+255ul)/256ul);
 return n;
}
uint2 closure_combine(uint2 left,uint2 right,bool sum) {
 if(sum) return uint2(uint(min(ulong(left.x)+right.x,0xfffffffful)),0u);
 return uint2(right.y?right.x:min(left.x,right.x),left.y|right.y);
}
kernel void closure_scan(device uint2* values [[buffer(0)]],device uint2* totals [[buffer(1)]],
 device const uint* lengths [[buffer(2)]],constant uint3& p [[buffer(3)]],
 uint tid [[thread_position_in_grid]],uint lane [[thread_index_in_threadgroup]],uint group [[threadgroup_position_in_grid]]) {
 uint n=closure_level(lengths[p.x],p.y);
 uint2 identity=uint2(p.z?0u:CLOSURE_NONE,0u);
 threadgroup uint2 work[256];
 work[lane]=tid<n?values[tid]:identity;
 threadgroup_barrier(mem_flags::mem_threadgroup);
 for(uint step=1u;step<256u;step<<=1u) {
  uint2 left=lane>=step?work[lane-step]:identity;
  threadgroup_barrier(mem_flags::mem_threadgroup);
  work[lane]=closure_combine(left,work[lane],p.z!=0u);
  threadgroup_barrier(mem_flags::mem_threadgroup);
 }
 if(tid<n) values[tid]=work[lane];
 if(lane==255u) totals[group]=work[lane];
}
kernel void closure_add_carries(device uint2* values [[buffer(0)]],device const uint2* totals [[buffer(1)]],
 device const uint* lengths [[buffer(2)]],constant uint3& p [[buffer(3)]],uint tid [[thread_position_in_grid]]) {
 uint n=closure_level(lengths[p.x],p.y),block=tid/256u;
 if(tid<n && block) values[tid]=closure_combine(totals[block-1u],values[tid],p.z!=0u);
}
kernel void closure_initialize(device const uint* counts [[buffer(0)]],device uint* lengths [[buffer(1)]],
 device uint* arguments [[buffer(2)]],device uint* status [[buffer(3)]],device uint* output [[buffer(4)]],
 constant ClosureParams& p [[buffer(5)]],uint tid [[thread_position_in_grid]]) {
 if(tid) return;
 bool valid=counts[0]<=p.domains && counts[1]<=p.entries && counts[2]<=p.cells
  && ((counts[0]==0u)==(counts[2]==0u));
 for(uint i=0u;i<3u;i++) lengths[i]=valid?counts[i]:0u;
 status[0]=valid?0u:2u;
 for(uint i=0u;i<p.cell_levels;i++) {
  arguments[i*3u]=closure_groups(closure_level(lengths[2],i)); arguments[i*3u+1u]=arguments[i*3u+2u]=1u;
 }
 for(uint i=0u;i<p.entry_levels;i++) {
  uint at=(p.cell_levels+i)*3u;
  arguments[at]=closure_groups(closure_level(lengths[1],i)); arguments[at+1u]=arguments[at+2u]=1u;
 }
 uint at=(p.cell_levels+p.entry_levels)*3u;
 arguments[at]=closure_groups(lengths[0]); arguments[at+1u]=arguments[at+2u]=1u;
 output[0]=output[1]=0u; output[2]=output[3]=1u;
}
kernel void closure_initialize_cells(device const uint* allowed [[buffer(0)]],device uint* seeds [[buffer(1)]],
 device uint* heads [[buffer(2)]],device const uint* lengths [[buffer(3)]],device atomic_uint* status [[buffer(4)]],
 constant ClosureParams& p [[buffer(5)]],uint tid [[thread_position_in_grid]]) {
 if(tid>=lengths[2]) return;
 seeds[tid]=CLOSURE_NONE; heads[tid]=0u;
 if(allowed[tid]>1u) atomic_fetch_or_explicit(status,2u,memory_order_relaxed);
}
kernel void closure_domains(device const uint2* domains [[buffer(0)]],device atomic_uint* heads [[buffer(1)]],
 device const uint* lengths [[buffer(2)]],device atomic_uint* status [[buffer(3)]],
 constant ClosureParams& p [[buffer(4)]],uint tid [[thread_position_in_grid]]) {
 if(tid>=lengths[0]) return;
 uint2 d=domains[tid]; ulong end=ulong(d.x)+d.y;
 ulong expected=tid?ulong(domains[tid-1u].x)+domains[tid-1u].y:0ul;
 if(d.y==0u || d.x!=expected || end>lengths[2] || (tid+1u==lengths[0] && end!=lengths[2])) {
  atomic_fetch_or_explicit(status,2u,memory_order_relaxed); return;
 }
 atomic_store_explicit(heads+d.x,1u,memory_order_relaxed);
}
kernel void closure_entries(device const uint2* domains [[buffer(0)]],device const uint2* entries [[buffer(1)]],
 device atomic_uint* seeds [[buffer(2)]],device const uint* lengths [[buffer(3)]],device atomic_uint* status [[buffer(4)]],
 constant ClosureParams& p [[buffer(5)]],uint tid [[thread_position_in_grid]]) {
 if(tid>=lengths[1] || atomic_load_explicit(status,memory_order_relaxed)) return;
 uint2 e=entries[tid];
 if(e.x>=lengths[0]) { atomic_fetch_or_explicit(status,2u,memory_order_relaxed); return; }
 uint2 d=domains[e.x];
 if(e.y>=d.y || ulong(d.x)+e.y>=lengths[2]) { atomic_fetch_or_explicit(status,2u,memory_order_relaxed); return; }
 atomic_fetch_min_explicit(seeds+d.x+e.y,tid,memory_order_relaxed);
}
kernel void closure_runs(device const uint* allowed [[buffer(0)]],device const uint* seeds [[buffer(1)]],
 device const uint* heads [[buffer(2)]],device uint2* runs [[buffer(3)]],device const uint* lengths [[buffer(4)]],
 device const atomic_uint* status [[buffer(5)]],constant ClosureParams& p [[buffer(6)]],uint tid [[thread_position_in_grid]]) {
 if(tid<lengths[2] && !atomic_load_explicit(status,memory_order_relaxed)) runs[tid]=uint2(seeds[tid],heads[tid] || !allowed[tid]);
}
kernel void closure_ends(device const uint2* owners [[buffer(0)]],device const uint* heads [[buffer(1)]],
 device uint2* ends [[buffer(2)]],device const uint* lengths [[buffer(3)]],device const atomic_uint* status [[buffer(4)]],
 constant ClosureParams& p [[buffer(5)]],uint tid [[thread_position_in_grid]]) {
 uint n=lengths[2]; if(tid>=n || atomic_load_explicit(status,memory_order_relaxed)) return;
 uint at=n-1u-tid; bool last=at+1u==n || heads[at+1u];
 ends[tid]=uint2(last || owners[at].x!=owners[at+1u].x?at:CLOSURE_NONE,last);
}
kernel void closure_count(device const uint2* domains [[buffer(0)]],device const uint2* entries [[buffer(1)]],
 device const uint2* owners [[buffer(2)]],device uint2* counts [[buffer(3)]],device const uint* lengths [[buffer(4)]],
 device const atomic_uint* status [[buffer(5)]],constant ClosureParams& p [[buffer(6)]],uint tid [[thread_position_in_grid]]) {
 if(tid>=lengths[1] || atomic_load_explicit(status,memory_order_relaxed)) return;
 uint2 e=entries[tid]; uint at=domains[e.x].x+e.y;
 counts[tid]=uint2(owners[at].x==tid,0u);
}
kernel void closure_boundary(device const uint2* totals [[buffer(0)]],device uint* output [[buffer(1)]],
 device atomic_uint* status [[buffer(2)]],constant ClosureParams& p [[buffer(3)]],uint tid [[thread_position_in_grid]]) {
 if(tid) return;
 uint n=totals[0].x;
 if(n>p.output) atomic_fetch_or_explicit(status,1u,memory_order_relaxed);
 if(atomic_load_explicit(status,memory_order_relaxed)) n=0u;
 output[0]=n; output[1]=uint((ulong(n)+255ul)/256ul); output[2]=output[3]=1u;
}
kernel void closure_scatter(device const uint2* domains [[buffer(0)]],device const uint2* entries [[buffer(1)]],
 device const uint2* owners [[buffer(2)]],device const uint2* ends [[buffer(3)]],device const uint2* offsets [[buffer(4)]],
 device uint2* output [[buffer(5)]],device const uint* counts [[buffer(6)]],device const uint* lengths [[buffer(7)]],
 constant ClosureParams& p [[buffer(8)]],uint tid [[thread_position_in_grid]]) {
 if(tid>=lengths[1] || !counts[0]) return;
 uint2 e=entries[tid]; uint at=domains[e.x].x+e.y;
 if(owners[at].x==tid) output[offsets[tid].x-1u]=uint2(tid,ends[lengths[2]-1u-at].x);
}
