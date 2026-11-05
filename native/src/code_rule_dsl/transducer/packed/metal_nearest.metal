#include <metal_stdlib>
using namespace metal;

// Nearest ancestors in a set, in one dispatch (decisions 0038, 0040). A cell is (set, node) and
// holds a state: a target node and whether the target is final (bit 31).
//
// - Final: the target is the answer, the nearest strict ancestor in the set, or NONE.
// - Not final: the target is a strict ancestor not in the set, so the cell's answer is the
//   target's answer.
//
// A thread takes its target's current state. If that state is final, its target is the cell's
// answer. Otherwise the cell takes the target's target, still not final. Every state any thread
// writes keeps this meaning, so a thread may read another's state at any moment, before or
// after that one moves. Each step moves the target strictly up the tree, so a thread is done
// within its depth's steps, and it never waits on another thread. Threads that read cells
// already moved up skip ahead, which is the pointer jumping.

constant uint NONE=0xFFFFFFFFu;
constant uint FINAL=0x80000000u;

struct Args { uint nodes,cells,pad0,pad1; };

kernel void nearest_jump(device atomic_uint* state [[buffer(0)]],device atomic_uint* stats [[buffer(1)]],
    constant Args& args [[buffer(2)]],uint cell [[thread_position_in_grid]]){
  if(cell>=args.cells)return;
  uint v=atomic_load_explicit(&state[cell],memory_order_relaxed);
  // The target is a node of the same set's block.
  uint base=(cell/args.nodes)*args.nodes;
  uint steps=0u;
  while((v&FINAL)==0u&&steps<args.nodes){
    uint w=atomic_load_explicit(&state[base+v],memory_order_relaxed);
    v=w;
    steps++;
    atomic_store_explicit(&state[cell],v,memory_order_relaxed);
  }
  if(steps>0u)atomic_fetch_max_explicit(&stats[0],steps,memory_order_relaxed);
}
