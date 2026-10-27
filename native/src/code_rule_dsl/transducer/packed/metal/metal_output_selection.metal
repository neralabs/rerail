// The maximal adjacent suffix follows capture order, rather than source length.
kernel void output_selection_initialize(device uint2* selected [[buffer(0)]],constant OutputParams& p [[buffer(1)]],uint i [[thread_position_in_grid]]) {
 if(i<p.n)selected[i]=uint2(NONE,0u);
}
kernel void output_selection_counts(device const uchar* data [[buffer(0)]],device const Capture* caps [[buffer(1)]],device const uint4* meta [[buffer(2)]],device const uint* owners [[buffer(3)]],device atomic_uint* selected [[buffer(4)]],device uint4* flags [[buffer(5)]],device const uint* status [[buffer(6)]],constant OutputParams& p [[buffer(7)]],uint i [[thread_position_in_grid]]) {
 if(i>=p.c)return;flags[i]=uint4(0u);if(status[0])return;uint owner=owners[i];if(!meta[owner].w || !caps[i].retained)return;
 View v=view(data);Program pg=v.programs[meta[owner].y];if(p.step>=pg.output_count)return;
 device const uint* op=v.output_words+pg.output_start+p.step*6u;if(op[0]!=4u && op[0]!=6u)return;
 if(caps[i].label==(op[0]==4u?op[2]:op[1]))atomic_fetch_min_explicit(selected+owner*2u,i,memory_order_relaxed);
 if(op[0]==4u && caps[i].label==op[1])flags[i].x=1u;
}
bool output_selected(device const uint4* offsets,device const uint4* total,uint n,uint i) {
 return output_prefix(offsets,total,n,i+1u).x==offsets[i].x+1u;
}
kernel void output_selection_indices(device const uint4* offsets [[buffer(0)]],device const uint4* total [[buffer(1)]],device uint* indices [[buffer(2)]],device const uint* status [[buffer(3)]],constant OutputParams& p [[buffer(4)]],uint i [[thread_position_in_grid]]) {
 if(!status[0] && i<p.c && output_selected(offsets,total,p.c,i))indices[offsets[i].x]=i;
}
kernel void output_selection_breaks(device const uchar* data [[buffer(0)]],device const Capture* caps [[buffer(1)]],device const uint4* meta [[buffer(2)]],device const uint* owners [[buffer(3)]],device atomic_uint* selected [[buffer(4)]],device const uint4* offsets [[buffer(5)]],device const uint4* total [[buffer(6)]],device const uint* indices [[buffer(7)]],device atomic_uint* status [[buffer(8)]],constant OutputParams& p [[buffer(9)]],uint i [[thread_position_in_grid]]) {
 if(atomic_load_explicit(status,memory_order_relaxed) || i>=p.c || !output_selected(offsets,total,p.c,i))return;
 uint owner=owners[i],target=atomic_load_explicit(selected+owner*2u,memory_order_relaxed);if(target==NONE)return;
 if(target>=p.c || owners[target]!=owner || !caps[target].retained){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 uint rank=offsets[i].x,next=rank+1u<total[0].x?indices[rank+1u]:NONE;
 if(next==NONE || owners[next]!=owner)next=target;
 View v=view(data);File f=v.files[meta[owner].z];
 if(output_line(v,f,caps[i].end)+1u<output_line(v,f,caps[next].start))atomic_fetch_max_explicit(selected+owner*2u+1u,i+1u,memory_order_relaxed);
}
kernel void output_selection_apply(device Capture* caps [[buffer(0)]],device const uint* owners [[buffer(1)]],device const uint2* selected [[buffer(2)]],device const uint4* offsets [[buffer(3)]],device const uint4* total [[buffer(4)]],device const uint* status [[buffer(5)]],constant OutputParams& p [[buffer(6)]],uint i [[thread_position_in_grid]]) {
 if(!status[0] && i<p.c && output_selected(offsets,total,p.c,i) && i+1u<=selected[owners[i]].y)caps[i].retained=0u;
}
