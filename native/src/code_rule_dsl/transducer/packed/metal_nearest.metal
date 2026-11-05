#include <metal_stdlib>
using namespace metal;

// Nearest ancestors in a set by pointer jumping (decision 0038): a cell is (set, node). Each cell
// holds a target, at first its node's parent, and whether that target is final: the parent is in
// the set, or there is none. In a round, a cell whose target is not final takes its target's
// target, and its target's finality. A round doubles the distance a cell has looked up, so
// ceil(log2(depth)) rounds finish every cell. Rounds read one pair of buffers and write the
// other; `flags[round]` is set where a cell is still not final after that round.

constant uint NONE=0xFFFFFFFFu;

struct Args { uint nodes,cells,round,pad; };

kernel void nearest_jump(device const uint* target_in [[buffer(0)]],device const uchar* final_in [[buffer(1)]],
    device uint* target_out [[buffer(2)]],device uchar* final_out [[buffer(3)]],
    device atomic_uint* flags [[buffer(4)]],constant Args& args [[buffer(5)]],
    uint cell [[thread_position_in_grid]]){
  if(cell>=args.cells)return;
  uint t=target_in[cell];
  if(final_in[cell]!=0u){
    target_out[cell]=t;
    final_out[cell]=1u;
    return;
  }
  // The target is a node of the same set's block.
  uint at=(cell/args.nodes)*args.nodes+t;
  target_out[cell]=target_in[at];
  uchar f=final_in[at];
  final_out[cell]=f;
  if(f==0u)atomic_store_explicit(&flags[args.round],1u,memory_order_relaxed);
}
