// Input rows already have complete, validated logical ordering.
uint terminal_evaluation(View v,uint task,uint spans){
 uint lo=0u,hi=spans;while(lo<hi){uint mid=lo+(hi-lo)/2u;if(v.spans[v.semantic_order[mid]].semantic_start<=task)lo=mid+1u;else hi=mid;}
 if(!lo)return NONE;Span span=v.spans[v.semantic_order[lo-1u]];
 return task-span.semantic_start<span.root_count?span.evaluation:NONE;
}
kernel void evaluation_failure_first(device const uchar* data [[buffer(0)]],device const uint* s [[buffer(1)]],device const uint* h [[buffer(2)]],device atomic_uint* winners [[buffer(3)]],device atomic_uint* status [[buffer(4)]],constant TerminalParams& p [[buffer(5)]],uint i [[thread_position_in_grid]]){
 if(atomic_load_explicit(status,memory_order_relaxed) || i>=h[0] || !(frame_operand_failure(tc(s,h[0],5u,i)) || frame_work_failure(tc(s,h[0],5u,i))))return;
 View v=view(data);uint evaluation=terminal_evaluation(v,s[i],p.spans);
 if(evaluation==NONE){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 atomic_fetch_min_explicit(winners+evaluation,i,memory_order_relaxed);
}
kernel void evaluation_failure_records(device const uint* s [[buffer(0)]],device const uint* h [[buffer(1)]],device const uint* winners [[buffer(2)]],device uint2* output [[buffer(3)]],device atomic_uint* status [[buffer(4)]],constant uint& evaluations [[buffer(5)]],uint i [[thread_position_in_grid]]){
 if(i>=evaluations)return;output[i]=uint2(0u);if(atomic_load_explicit(status,memory_order_relaxed))return;
 uint row=winners[i];if(row==NONE)return;
 if(row>=h[0] || !(frame_operand_failure(tc(s,h[0],5u,row)) || frame_work_failure(tc(s,h[0],5u,row)))){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 output[i]=uint2(frame_work_failure(tc(s,h[0],5u,row))?2u:1u,tc(s,h[0],1u,row));
}
