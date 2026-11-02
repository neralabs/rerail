#include <metal_stdlib>
using namespace metal;

// A flat lattice's fixpoint (`logic::native::lattice_fixpoint`'s graph, list step 17): one
// threadgroup per component (a file's cells: no edge leaves it), which runs its semi-naive waves
// itself until one changes nothing. A wave evaluates the dirty cells from the last wave's values
// (`next`), then applies the changes and marks their dependents dirty.

constant long TOP=4611686018427387904L;
constant long BOT=(-9223372036854775807L-1L);
constant long BIG=1099511627776L;

struct Args { uint cells,components,maxWaves,pad; };
struct Bin { uint op,left,right,pad; };

static long lub(long a,long b){ if(a==BOT)return b; if(b==BOT)return a; return a==b?a:TOP; }
static long transfer(uint op,long a,long b){
  if(a==TOP||b==TOP)return TOP;
  if(op==43u)return a+b;
  if(op==45u)return a-b;
  if(op==42u&&abs(a)<BIG&&abs(b)<BIG)return a*b;
  return TOP;
}
static bool test(device atomic_uint* bits,uint cell){return (atomic_load_explicit(&bits[cell/32u],memory_order_relaxed)>>(cell%32u)&1u)!=0u;}
static void set(device atomic_uint* bits,uint cell){atomic_fetch_or_explicit(&bits[cell/32u],1u<<(cell%32u),memory_order_relaxed);}
static void clear(device atomic_uint* bits,uint cell){atomic_fetch_and_explicit(&bits[cell/32u],~(1u<<(cell%32u)),memory_order_relaxed);}

kernel void lattice_flat(device long* values [[buffer(0)]],device const long* seeds [[buffer(1)]],
    device const uint* copyStart [[buffer(2)]],device const uint* copies [[buffer(3)]],
    device const uint* binStart [[buffer(4)]],device const uint* binIndex [[buffer(5)]],device const Bin* bins [[buffer(6)]],
    device const uint* outStart [[buffer(7)]],device const uint* outs [[buffer(8)]],
    device const uint* order [[buffer(9)]],device const uint* ranges [[buffer(10)]],
    device atomic_uint* dirty [[buffer(11)]],device atomic_uint* pending [[buffer(12)]],device long* next [[buffer(13)]],
    device atomic_uint* stats [[buffer(14)]],constant Args& args [[buffer(15)]],
    uint component [[threadgroup_position_in_grid]],uint lane [[thread_index_in_threadgroup]],uint width [[threads_per_threadgroup]]){
  if(component>=args.components)return;
  threadgroup atomic_uint changed;
  uint low=ranges[component],high=ranges[component+1u],waves=0u,visits=0u;
  while(true){
    if(lane==0u)atomic_store_explicit(&changed,0u,memory_order_relaxed);
    threadgroup_barrier(mem_flags::mem_threadgroup|mem_flags::mem_device);
    for(uint at=low+lane;at<high;at+=width){
      uint cell=order[at];
      if(!test(dirty,cell))continue;
      clear(dirty,cell);
      visits++;
      long value=seeds[cell];
      for(uint e=copyStart[cell];e<copyStart[cell+1u];e++)value=lub(value,values[copies[e]]);
      for(uint e=binStart[cell];e<binStart[cell+1u];e++){
        Bin bin=bins[binIndex[e]];
        long a=values[bin.left],b=values[bin.right];
        if(a!=BOT&&b!=BOT)value=lub(value,transfer(bin.op,a,b));
      }
      if(value!=values[cell]){next[cell]=value;set(pending,cell);atomic_store_explicit(&changed,1u,memory_order_relaxed);}
    }
    threadgroup_barrier(mem_flags::mem_threadgroup|mem_flags::mem_device);
    bool more=atomic_load_explicit(&changed,memory_order_relaxed)!=0u;
    if(more){
      for(uint at=low+lane;at<high;at+=width){
        uint cell=order[at];
        if(!test(pending,cell))continue;
        clear(pending,cell);
        values[cell]=next[cell];
        for(uint e=outStart[cell];e<outStart[cell+1u];e++)set(dirty,outs[e]);
      }
      waves++;
    }
    threadgroup_barrier(mem_flags::mem_threadgroup|mem_flags::mem_device);
    if(!more||waves>=args.maxWaves)break;
  }
  atomic_fetch_add_explicit(&stats[1],visits,memory_order_relaxed);
  if(lane==0u){
    atomic_fetch_max_explicit(&stats[0],waves,memory_order_relaxed);
    if(waves>=args.maxWaves)atomic_store_explicit(&stats[2],1u,memory_order_relaxed);
  }
}
