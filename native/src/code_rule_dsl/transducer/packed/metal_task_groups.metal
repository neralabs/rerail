// Current-batch task groups preserve complete uint task identities. Hashes only
// choose a starting slot. These tables never retain a previous generation.
uint active_task_hash(uint key) {
 key^=key>>16u;key*=0x7feb352du;key^=key>>15u;key*=0x846ca68bu;return key^(key>>16u);
}
uint active_task_slot(device atomic_uint* keys,uint stride,uint capacity,uint task) {
 if(!capacity)return NONE;
 uint slot=active_task_hash(task)%capacity,visited=0u;
 while(visited<capacity) {
  uint key=atomic_load_explicit(keys+slot*stride,memory_order_relaxed);
  if(key==task)return slot;
  if(key==NONE) {
   uint expected=NONE;
   if(atomic_compare_exchange_weak_explicit(keys+slot*stride,&expected,task,memory_order_relaxed,memory_order_relaxed) || expected==task)return slot;
   if(expected==NONE)continue;
  }
  visited++;slot=slot+1u==capacity?0u:slot+1u;
 }
 return NONE;
}
uint find_active_task_slot(device const uint* keys,uint stride,uint capacity,uint task) {
 if(!capacity)return NONE;
 uint slot=active_task_hash(task)%capacity;
 for(uint visited=0u;visited<capacity;visited++) {
  uint key=keys[slot*stride];if(key==task)return slot;if(key==NONE)return NONE;
  slot=slot+1u==capacity?0u:slot+1u;
 }
 return NONE;
}
