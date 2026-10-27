// Test-only scalar work accounting. No semantic records cross to the host.
kernel void successor_profile_expansion(
 device const uchar* data [[buffer(0)]],device const uint* input [[buffer(1)]],device const uint* header [[buffer(2)]],
 device const uint* steps [[buffer(3)]],device const uint4* offsets [[buffer(4)]],device const uint4* total [[buffer(5)]],
 device atomic_uint* profile [[buffer(6)]],constant SuccessorParams& p [[buffer(7)]],uint i [[thread_position_in_grid]]) {
 if(i>=p.steps)return;
 uint4 start=offsets[i],end=i+1u<p.steps?offsets[i+1u]:total[0];
 device const Step& x=step_at(steps,start.z);View v=view(data);
 uint program=input_scalar(v,input,header[0],4u,x.input_parent),base=program*8u;
 atomic_fetch_add_explicit(profile+base,1u,memory_order_relaxed);
 atomic_fetch_add_explicit(profile+base+1u,uint(x.live!=0u),memory_order_relaxed);
 atomic_fetch_add_explicit(profile+base+2u,end.x-start.x,memory_order_relaxed);
 atomic_fetch_add_explicit(profile+base+3u,end.y-start.y,memory_order_relaxed);
 atomic_fetch_add_explicit(profile+base+4u,end.w-start.w,memory_order_relaxed);
 atomic_fetch_add_explicit(profile+base+5u,end.z-start.z,memory_order_relaxed);
 atomic_fetch_max_explicit(profile+base+6u,end.x-start.x,memory_order_relaxed);
 atomic_fetch_add_explicit(profile+base+7u,uint(x.live && (frame_pending(x.kind) || x.expansion==2u)),memory_order_relaxed);
}
