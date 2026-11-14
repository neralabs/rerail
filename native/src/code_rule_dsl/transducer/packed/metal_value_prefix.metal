// Reject only when every member of the first positive predicate fails cheaply.
// Every survivor still executes the shared value frontier and its exact budget.
__attribute__((noinline)) uint register_prefix_label(View v,Program pg,uint captures) {
 if(!(pg.flags&1u) || !pg.capture_count)return NONE;
 Assertion a=v.assertions[pg.capture_start];if(a.kind!=1u || (a.flags&1u))return NONE;
 device const uint* proof=v.value_prefixes+(pg.value_start+a.second)*9u;
 if(!proof[8] || ulong(proof[8])*captures>PATH_WORK_LIMIT)return NONE;
 return a.first;
}
__attribute__((noinline)) bool register_prefix_possible(View v,Program pg,uint node) {
 Assertion a=v.assertions[pg.capture_start];
 device const uint* proof=v.value_prefixes+(pg.value_start+a.second)*9u;
 Node n=v.nodes[node];if(n.source_start==n.source_end)return false;
 uint byte=v.source[n.source_start];return (proof[byte/32u]&(1u<<(byte%32u)))!=0u;
}
