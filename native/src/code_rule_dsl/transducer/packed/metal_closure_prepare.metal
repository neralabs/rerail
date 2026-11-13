struct PrepareParams { uint capacity,cursors,repeats,captures,domains,entries,cells,nodes,children,regions,levels,data_bytes; };
uint prepare_groups(uint count) { return uint((ulong(count)+255ul)/256ul); }
uint prepare_column(device const uint* states,uint n,uint field,uint index) { return states[field*n+index]; }
bool prepare_guard(uint region,Node n) {
 switch(region) {
 // GENERATED_CLOSURE_GUARDS
 default: return false;
 }
}
kernel void prepare_initialize(device const uchar* data [[buffer(0)]],device const uint* counts [[buffer(1)]],device const uint* values [[buffer(2)]],device uint* lengths [[buffer(3)]],device uint* arguments [[buffer(4)]],device uint* status [[buffer(5)]],device uint* output [[buffer(6)]],constant PrepareParams& p [[buffer(7)]],uint tid [[thread_position_in_grid]]) {
 if(tid) return;
 device const Header* h=(device const Header*)data;
 uint n=counts[0];
 bool valid=n<=p.capacity && counts[1]<=p.captures && values[0]<=p.cursors && values[1]<=p.repeats && counts[2]==prepare_groups(n) && counts[3]==1u && counts[4]==1u;
 valid &= h->nodes%16u==0u && h->children%16u==0u && ulong(h->nodes)+ulong(p.nodes)*sizeof(Node)<=p.data_bytes && ulong(h->children)+ulong(p.children)*4ul<=p.data_bytes;
 status[0]=valid?0u:2u; lengths[0]=valid?n:0u; lengths[1]=valid?counts[1]:0u; lengths[2]=prepare_groups(lengths[0]);lengths[3]=lengths[4]=1u;
 uint count=lengths[0];for(uint level=0;level<p.levels;level++) {count=prepare_groups(count);arguments[level*3u]=max(1u,count);arguments[level*3u+1u]=arguments[level*3u+2u]=1u;}
 output[0]=output[1]=output[2]=0u;
}
kernel void prepare_project(device const uchar* data [[buffer(0)]],device const uint* states [[buffer(1)]],device const uint* lengths [[buffer(2)]],device const uint4* repeats [[buffer(3)]],device const uint* values [[buffer(4)]],device const uint4* descriptors [[buffer(5)]],device uint* projected [[buffer(6)]],device uint4* metadata [[buffer(7)]],device uint* regions [[buffer(8)]],device atomic_uint* status [[buffer(9)]],constant PrepareParams& p [[buffer(10)]],uint tid [[thread_position_in_grid]]) {
 uint n=lengths[0];if(tid>=n)return;
 uint pc=prepare_column(states,n,1u,tid),node=prepare_column(states,n,2u,tid),pg=prepare_column(states,n,4u,tid);
 uint low=0,high=p.regions;
 while(low<high) {uint mid=low+(high-low)/2;uint4 d=descriptors[mid];if(d.x<pg||(d.x==pg&&d.y<pc))low=mid+1u;else high=mid;}
 if(node>=p.nodes || prepare_column(states,n,3u,tid)>3u || prepare_column(states,n,5u,tid)!=0u || low==p.regions) {atomic_fetch_or_explicit(status,2u,memory_order_relaxed);return;}
 uint4 region=descriptors[low];uint start=prepare_column(states,n,8u,tid),count=prepare_column(states,n,9u,tid);
 if(region.x!=pg || region.y!=pc || count==0u || ulong(start)+count>values[1]) {atomic_fetch_or_explicit(status,2u,memory_order_relaxed);return;}
 if(repeats[start+count-1u].y!=region.z) {atomic_fetch_or_explicit(status,2u,memory_order_relaxed);return;}
 View v=view(data);uint parent=v.nodes[node].parent;
 uint4 m=uint4(0u,1u,0u,node);uint first=node;
 if(parent!=NONE) {
  if(parent>=p.nodes) {atomic_fetch_or_explicit(status,2u,memory_order_relaxed);return;}
  Node chain=v.nodes[parent];
  if(chain.child_count==0u || ulong(chain.child_start)+chain.child_count>p.children) {atomic_fetch_or_explicit(status,2u,memory_order_relaxed);return;}
  uint left=0,right=chain.child_count;
  while(left<right) {uint mid=left+(right-left)/2;if(v.children[chain.child_start+mid]<node)left=mid+1u;else right=mid;}
  if(left==chain.child_count || v.children[chain.child_start+left]!=node) {atomic_fetch_or_explicit(status,2u,memory_order_relaxed);return;}
  first=v.children[chain.child_start];m=uint4(chain.child_start,chain.child_count,left,NONE);
 }
 // Project onto the first sibling, not the parent: roots and their children
 // must never share a domain. Full original states and ordinals stay unchanged.
 for(uint field=0;field<13u;field++) projected[field*n+tid]=field==2u?first:prepare_column(states,n,field,tid);
 metadata[tid]=m;regions[tid]=low;
}
kernel void prepare_project_counts(device const uint* lengths [[buffer(0)]],device const uint* status [[buffer(1)]],device uint* output [[buffer(2)]],constant PrepareParams& p [[buffer(3)]],uint tid [[thread_position_in_grid]]) {
 if(tid)return;for(uint i=0;i<5u;i++)output[i]=i<3u&&status[0]!=0u?0u:lengths[i];
}
kernel void prepare_count(device const uint* counts [[buffer(0)]],device const uint* groups [[buffer(1)]],device const uint4* metadata [[buffer(2)]],device uint4* sizes [[buffer(3)]],constant PrepareParams& p [[buffer(4)]],uint tid [[thread_position_in_grid]]) {
 if(tid<counts[0]) {uint cells=metadata[tid].y;sizes[tid]=groups[tid]==tid?uint4(1u,cells,prepare_groups(cells),0u):uint4(0u);}
}
kernel void prepare_range_counts(device const uint4* total [[buffer(0)]],device const uint* lengths [[buffer(1)]],device const uint* groups [[buffer(2)]],device uint* status [[buffer(3)]],device uint* output [[buffer(4)]],device uint* arguments [[buffer(5)]],constant PrepareParams& p [[buffer(6)]],uint tid [[thread_position_in_grid]]) {
 if(tid)return;uint4 sum=total[0];
 if(groups[0]!=lengths[0])status[0]|=2u;
 if(sum.x>p.domains||sum.y>p.cells||lengths[0]>p.entries)status[0]|=1u;
 bool valid=status[0]==0u;
 output[0]=valid?sum.x:0u;output[1]=valid?lengths[0]:0u;output[2]=valid?sum.y:0u;
 arguments[0]=valid?sum.z:0u;arguments[1]=arguments[2]=1u;
}
kernel void prepare_pack(device const uint* counts [[buffer(0)]],device const uint* groups [[buffer(1)]],device const uint4* metadata [[buffer(2)]],device const uint4* offsets [[buffer(3)]],device const uint* status [[buffer(4)]],device uint2* domains [[buffer(5)]],device uint2* entries [[buffer(6)]],device uint4* work [[buffer(7)]],constant PrepareParams& p [[buffer(8)]],uint tid [[thread_position_in_grid]]) {
 if(tid>=counts[0]||status[0])return;
 uint representative=groups[tid];uint4 offset=offsets[representative];
 entries[tid]=uint2(offset.x,metadata[tid].z);
 if(tid==representative) {domains[offset.x]=uint2(offset.y,metadata[tid].y);work[offset.x]=uint4(offset.z,prepare_groups(metadata[tid].y),tid,0u);}
}
kernel void prepare_guards(device const uchar* data [[buffer(0)]],device const uint4* metadata [[buffer(1)]],device const uint* regions [[buffer(2)]],device const uint2* domains [[buffer(3)]],device const uint* counts [[buffer(4)]],device const uint4* work [[buffer(5)]],device uint* allowed [[buffer(6)]],device uint* nodes [[buffer(7)]],constant PrepareParams& p [[buffer(8)]],uint lane [[thread_index_in_threadgroup]],uint group [[threadgroup_position_in_grid]]) {
 uint low=0,high=counts[0];
 while(low<high) {uint mid=low+(high-low)/2;if(work[mid].x<=group)low=mid+1u;else high=mid;}
 if(low==0u)return;uint domain=low-1u;uint4 w=work[domain],m=metadata[w.z];
 uint offset=(group-w.x)*256u+lane;if(offset>=m.y)return;
 View v=view(data);uint node=m.w==NONE?v.children[m.x+offset]:m.w;
 uint address=domains[domain].x+offset;
 nodes[address]=node;allowed[address]=uint(prepare_guard(regions[w.z],v.nodes[node]));
}
