#include <metal_stdlib>
using namespace metal;
bool publication_tile_less(uint4 a,uint4 b) {
 return a.x<b.x || (a.x==b.x && (a.y<b.y || (a.y==b.y && a.z<b.z)));
}
kernel void publication_sort_tiles(device const uint* header [[buffer(0)]],device const uint4* metadata [[buffer(1)]],device const uint* input [[buffer(2)]],device uint* output [[buffer(3)]],device atomic_uint* status [[buffer(4)]],constant uint& capacity [[buffer(5)]],uint group [[threadgroup_position_in_grid]],uint lane [[thread_index_in_threadgroup]]) {
 uint first=group*256u,n=header[0];threadgroup atomic_uint invalid;
 if(!lane){uint prior=atomic_load_explicit(status,memory_order_relaxed);if(!prior && n>capacity)atomic_store_explicit(status,2u,memory_order_relaxed);atomic_store_explicit(&invalid,uint(prior || n>capacity || first>=n),memory_order_relaxed);}
 threadgroup_barrier(mem_flags::mem_threadgroup);if(atomic_load_explicit(&invalid,memory_order_relaxed))return;
 uint at=first+lane;threadgroup uint ids[256];threadgroup uint4 keys[256];
 ids[lane]=at<n?input[at]:0xffffffffu;
 if(at<n && ids[lane]>=n){atomic_store_explicit(&invalid,1u,memory_order_relaxed);atomic_store_explicit(status,2u,memory_order_relaxed);}
 threadgroup_barrier(mem_flags::mem_threadgroup);if(atomic_load_explicit(&invalid,memory_order_relaxed))return;
 keys[lane]=at<n?metadata[ids[lane]]:uint4(0xffffffffu);
 threadgroup_barrier(mem_flags::mem_threadgroup);
 for(uint width=2u;width<=256u;width*=2u)for(uint distance=width/2u;distance;distance/=2u){
  uint other=lane^distance,a=ids[lane],b=ids[other];uint4 ka=keys[lane],kb=keys[other];
  bool less=b!=0xffffffffu && (a==0xffffffffu || publication_tile_less(kb,ka) || (!publication_tile_less(ka,kb) && b<a));
  bool ascending=(lane&width)==0u;bool take=((lane&distance)==0u)==ascending?less:!less;
  uint selected=take?b:a;uint4 key=take?kb:ka;
  threadgroup_barrier(mem_flags::mem_threadgroup);ids[lane]=selected;keys[lane]=key;threadgroup_barrier(mem_flags::mem_threadgroup);
 }
 if(at<n)output[at]=ids[lane];
}
