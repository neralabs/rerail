#include <metal_stdlib>
using namespace metal;
struct SavedColumnParams { uint4 counts; uint groups,words; };
uint saved_fields(uint arena) { return arena==0u?10u:(arena==1u?9u:(arena==2u?8u:4u)); }
uint saved_arena(constant SavedColumnParams& p,thread uint& group) {
 for(uint arena=0u;arena<4u;arena++) {
  uint count=((p.counts[arena]+255u)/256u)*saved_fields(arena);
  if(group<count)return arena;group-=count;
 }
 return 4u;
}
kernel void saved_column_bounds(
 device const uint* rows [[buffer(0)]],device const uint* caps [[buffer(1)]],
 device const uint* props [[buffer(2)]],device const uint* layout [[buffer(3)]],
 device uint2* minima [[buffer(4)]],device uint4* sizes [[buffer(5)]],
 constant SavedColumnParams& p [[buffer(6)]],uint g [[threadgroup_position_in_grid]],uint lane [[thread_index_in_threadgroup]]) {
 threadgroup uint2 values[256];uint local=g,arena=saved_arena(p,local);
 uint fields=saved_fields(arena),row=(local/fields)*256u+lane,field=local%fields;
 device const uint* source=arena==0u?rows:(arena==1u?caps:(arena==2u?props:layout));
 uint value=row<p.counts[arena]?source[row*fields+field]:0u;
 values[lane]=row<p.counts[arena]?uint2(value):uint2(0xffffffffu,0u);
 threadgroup_barrier(mem_flags::mem_threadgroup);
 for(uint step=128u;step;step/=2u){
  if(lane<step){values[lane].x=min(values[lane].x,values[lane+step].x);values[lane].y=max(values[lane].y,values[lane+step].y);}
  threadgroup_barrier(mem_flags::mem_threadgroup);
 }
 if(!lane){uint bits=values[0].y==values[0].x?0u:32u-clz(values[0].y-values[0].x);
  minima[g]=uint2(values[0].x,bits);uint count=min(256u,p.counts[arena]-(local/fields)*256u);
  sizes[g]=uint4(((count*bits+255u)/256u)*8u,0u,0u,0u);
 }
}
kernel void saved_column_pack(
 device const uint* rows [[buffer(0)]],device const uint* caps [[buffer(1)]],
 device const uint* props [[buffer(2)]],device const uint* layout [[buffer(3)]],
 device const uint2* minima [[buffer(4)]],device const uint4* offsets [[buffer(5)]],
 device uint2* headers [[buffer(6)]],device uint* output [[buffer(7)]],
 device atomic_uint* status [[buffer(8)]],constant SavedColumnParams& p [[buffer(9)]],uint i [[thread_position_in_grid]]) {
 if(i<p.groups){uint2 h=minima[i];uint at=offsets[i].x;
  if(h.y>32u || at>p.words || at%8u || at>500000000u){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
  headers[i]=uint2(h.x,at*4u+(h.y&31u)+((h.y>>5u)<<31u));
 }
 if(i>=p.words)return;
 uint lo=0u,hi=p.groups;
 while(lo<hi){uint mid=lo+(hi-lo)/2u;if(offsets[mid].x<=i)lo=mid+1u;else hi=mid;}
 if(!lo){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 uint group=lo-1u,local=group,arena=saved_arena(p,local),bits=minima[group].y;
 if(arena>=4u || !bits || bits>32u){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 uint fields=saved_fields(arena),count=min(256u,p.counts[arena]-(local/fields)*256u);
 uint need=((count*bits+255u)/256u)*8u;
 if(i-offsets[group].x>=need){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 device const uint* source=arena==0u?rows:(arena==1u?caps:(arena==2u?props:layout));
 uint bit=(i-offsets[group].x)*32u,first=bit/bits,word=0u;
 for(uint j=first;j<first+32u;j++){
  int shift=int(j*bits)-int(bit);if(shift>=32)break;uint delta=0u;
  if(j<count){uint value=source[((local/fields)*256u+j)*fields+local%fields];
   if(value<minima[group].x || (bits<32u && value-minima[group].x>=(1u<<bits))){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
   delta=value-minima[group].x;
  }
  word|=shift<0?delta>>uint(-shift):delta<<uint(shift);
 }
 output[i]=word;
}
