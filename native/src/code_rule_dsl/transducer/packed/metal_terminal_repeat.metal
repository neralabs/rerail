struct RepeatParams {uint tasks,spans,programs,cells,results,captures,step,work_limit,domain_count;};
// The compact root domain preserves original semantic identities across gaps
// occupied by other execution classes. Both columns remain independently named.
uint repeat_semantic_task(device const uint4* domain,constant RepeatParams& p,uint dense) {
 uint lo=0u,hi=p.domain_count;
 while(lo<hi){uint mid=lo+(hi-lo)/2u;if(domain[mid].x<=dense)lo=mid+1u;else hi=mid;}
 if(!lo)return NONE;uint4 span=domain[lo-1u];return dense-span.x<span.z?span.y+dense-span.x:NONE;
}
uint repeat_dense_task(device const uint4* domain,constant RepeatParams& p,uint semantic) {
 uint lo=0u,hi=p.domain_count;
 while(lo<hi){uint mid=lo+(hi-lo)/2u;if(domain[mid].y<=semantic)lo=mid+1u;else hi=mid;}
 if(!lo)return NONE;uint4 span=domain[lo-1u];return semantic-span.y<span.z?span.x+semantic-span.y:NONE;
}
constant long NO_ORIGIN=(-9223372036854775807l-1l);
uint repeat_eval(View v,uint task,constant RepeatParams& p) {
 uint lo=0u,hi=p.spans;while(lo<hi){uint mid=lo+(hi-lo)/2u;if(v.spans[v.semantic_order[mid]].semantic_start<=task)lo=mid+1u;else hi=mid;}
 if(!lo)return NONE;Span span=v.spans[v.semantic_order[lo-1u]];
 return task-span.semantic_start<span.root_count?span.evaluation:NONE;
}
uint repeat_owner(device const uint4* offsets,uint count,uint value,uint component) {
 uint lo=0u,hi=count;while(lo<hi){uint mid=lo+(hi-lo)/2u;if(offsets[mid][component]<=value)lo=mid+1u;else hi=mid;}return lo-1u;
}
uint4 repeat_prefix(device const uint4* values,device const uint4* total,uint at,constant RepeatParams& p){return at==p.cells?total[0]:values[at];}
kernel void repeat_root_counts(device const uchar* data [[buffer(0)]],device const uint* roots [[buffer(1)]],device uint4* descriptors [[buffer(2)]],device uint4* counts [[buffer(3)]],device atomic_uint* status [[buffer(4)]],device atomic_uint* failures [[buffer(5)]],device const uint4* domain [[buffer(6)]],constant RepeatParams& p [[buffer(7)]],uint task [[thread_position_in_grid]]){
 if(task>=p.tasks)return;counts[task]=uint4(0u);descriptors[task]=uint4(0u);
 uint semantic=repeat_semantic_task(domain,p,task);if(semantic==NONE){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 View v=view(data);uint ei=repeat_eval(v,semantic,p);if(ei==NONE){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 Evaluation e=v.evaluations[ei];uint mode=repeat_mode(e.program);if(!mode){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 Program pg=v.programs[e.program];uint root=roots[select_root(v,v.selectors[e.selector],semantic-e.semantic_start)];
 uint2 root_path=repeat_root(v,e.program,pg,root);
 if(root_path.y>p.work_limit){atomic_store_explicit(failures+ei,1u,memory_order_relaxed);return;}
 if(!root_path.x)return;
 uint first=NONE,count=0u;
 if(mode<=2u){first=v.nodes[root].child_start;count=v.nodes[root].child_count;}
 else {
  if(ulong(root_path.y)+repeat_cost(v,e.program,pg,root).y>p.work_limit){atomic_store_explicit(failures+ei,1u,memory_order_relaxed);return;}
  if(!repeat_class(v,e.program,pg,root))return;
  uint parent=v.nodes[root].parent;
  if(parent==NONE)count=1u;
  else {Node n=v.nodes[parent];uint lo=n.child_start,hi=lo+n.child_count;while(lo<hi){uint mid=lo+(hi-lo)/2u;if(v.children[mid]<root)lo=mid+1u;else hi=mid;}
   if(lo>=n.child_start+n.child_count || v.children[lo]!=root){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
   first=lo;count=n.child_start+n.child_count-first;
  }
 }
 descriptors[task]=uint4(root,first,count,e.program);counts[task]=uint4(count,0u,0u,0u);
}
kernel void repeat_cells(device const uchar* data [[buffer(0)]],device const uint4* descriptors [[buffer(1)]],device const uint4* tasks [[buffer(2)]],device uint4* nodes [[buffer(3)]],device uint2* boundaries [[buffer(4)]],device uint4* values [[buffer(5)]],device uint4* costs [[buffer(6)]],constant RepeatParams& p [[buffer(7)]],uint i [[thread_position_in_grid]]){
 if(i>=p.cells)return;uint task=repeat_owner(tasks,p.tasks,i,0u),start=tasks[task].x;uint4 d=descriptors[task];
 View v=view(data);uint node=d.y==NONE?d.x:v.children[d.y+i-start];uint mode=repeat_mode(d.w);
 bool ignored=(mode==2u || mode==4u) && !(v.nodes[node].flags&1u) && !(mode>=3u && i==start);
 uint cls=ignored?0u:repeat_class(v,d.w,v.programs[d.w],node);bool bad=!cls && !ignored;
 nodes[i]=uint4(task,node,cls,uint(ignored));boundaries[i]=uint2(bad?i:start+d.z,bad?i+1u:start);
 uint4 cost=ignored?uint4(0u):repeat_cost(v,d.w,v.programs[d.w],node);costs[i]=cost;
 uint captures=cls?repeat_size(d.w,cls).x:0u;Node root=v.nodes[d.x],captured=v.nodes[node];
 // Root suffix captures follow every body capture with the same geometry.
 values[i]=uint4(uint(cls!=0u),captures,uint(root.source_start==captured.source_start && root.source_end==captured.source_end)*captures,cost.z);
}
kernel void repeat_origins(device const uint4* nodes [[buffer(0)]],device const uint4* descriptors [[buffer(1)]],device const uint4* tasks [[buffer(2)]],device const uint4* values [[buffer(3)]],device const uint4* costs [[buffer(4)]],device long* origins [[buffer(5)]],constant RepeatParams& p [[buffer(6)]],uint i [[thread_position_in_grid]]){
 if(i>=p.cells)return;uint4 cell=nodes[i];uint start=tasks[cell.x].x;
 origins[i]=cell.z && (repeat_mode(descriptors[cell.x].w)<=2u || i==start)?long(costs[i].x)-long(values[i].w)-long(costs[i].z):NO_ORIGIN;
}
kernel void repeat_neighbors(device const uint4* nodes [[buffer(0)]],device const uint4* descriptors [[buffer(1)]],device const uint4* tasks [[buffer(2)]],device const uint2* input [[buffer(3)]],device uint2* output [[buffer(4)]],device const long* origins [[buffer(5)]],device long* next_origins [[buffer(6)]],constant RepeatParams& p [[buffer(7)]],uint i [[thread_position_in_grid]]){
 if(i>=p.cells)return;uint task=nodes[i].x,start=tasks[task].x,end=start+descriptors[task].z;uint2 value=input[i];
 if(ulong(i)+p.step<end)value.x=min(value.x,input[i+p.step].x);
 long origin=origins[i];
 if(i-start>=p.step){if(i-p.step>=input[i].y)origin=max(origin,origins[i-p.step]);value.y=max(value.y,input[i-p.step].y);}output[i]=value;next_origins[i]=origin;
}
kernel void repeat_work(device const uchar* data [[buffer(0)]],device const uint4* nodes [[buffer(1)]],device const uint4* descriptors [[buffer(2)]],device const uint4* tasks [[buffer(3)]],device const uint4* values [[buffer(4)]],device const uint4* costs [[buffer(5)]],device const long* origins [[buffer(6)]],device atomic_uint* failures [[buffer(7)]],device atomic_uint* status [[buffer(8)]],device const uint4* domain [[buffer(9)]],constant RepeatParams& p [[buffer(10)]],uint i [[thread_position_in_grid]]){
 if(i>=p.cells)return;uint4 cell=nodes[i];if(cell.w)return;
 View v=view(data);uint4 d=descriptors[cell.x];uint start=tasks[cell.x].x;uint4 cost=costs[i];uint2 bounds=repeat_bounds(d.w);
 if((cell.z && (!cost.x || !cost.z)) || (!cell.z && (cost.x || cost.z))){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 bool failed=(repeat_mode(d.w)<=2u || i==start) && ulong(bounds.x)+cost.y>p.work_limit;
 if(cell.z && origins[i]!=NO_ORIGIN){
  long head=long(bounds.x)+long(values[i].w)+long(cost.z)+origins[i];
  if(head<long(bounds.x)){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
  failed=failed || ulong(head)+max(bounds.y,2u)>p.work_limit;
 }
 if(i>start && origins[i-1u]!=NO_ORIGIN && cost.w){
  long previous=long(bounds.x)+long(values[i].w)+origins[i-1u];
  if(previous<long(bounds.x)){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
  failed=failed || ulong(previous)+cost.w>p.work_limit;
 }
 if(failed){uint semantic=repeat_semantic_task(domain,p,cell.x),ei=repeat_eval(v,semantic,p);if(semantic==NONE || ei==NONE){atomic_store_explicit(status,2u,memory_order_relaxed);return;}atomic_store_explicit(failures+ei,1u,memory_order_relaxed);}
}
kernel void repeat_candidate_counts(device uint4* nodes [[buffer(0)]],device const uint4* descriptors [[buffer(1)]],device const uint4* tasks [[buffer(2)]],device const uint2* boundaries [[buffer(3)]],device const uint4* values [[buffer(4)]],device const uint4* total [[buffer(5)]],device uint4* raw [[buffer(6)]],device uint4* selected [[buffer(7)]],constant RepeatParams& p [[buffer(8)]],uint i [[thread_position_in_grid]]){
 if(i>=p.cells)return;raw[i]=selected[i]=uint4(0u);uint4 cell=nodes[i];if(!cell.z)return;
 uint4 d=descriptors[cell.x];uint start=tasks[cell.x].x;uint2 bounds=boundaries[i];uint mode=repeat_mode(d.w);
 uint4 end=repeat_prefix(values,total,bounds.x,p),left=repeat_prefix(values,total,bounds.y,p);
 if(mode>=3u && i!=start)return;
 nodes[i].w=end.z-values[i].z;
 raw[i]=uint4(end.x-values[i].x,0u,0u,0u);
 bool first=mode>=3u || values[i].x==left.x;
 if(first){uint3 caps=repeat_size(d.w,0u);selected[i]=uint4(1u,end.y-values[i].y+caps.x+caps.y,0u,0u);}
}
kernel void repeat_limit(device const uchar* data [[buffer(0)]],device const uint4* nodes [[buffer(1)]],device const uint4* tasks [[buffer(2)]],device const uint4* selected [[buffer(3)]],device const uint4* total [[buffer(4)]],device uint4* output [[buffer(5)]],device const uint* failures [[buffer(6)]],device const uint4* domain [[buffer(7)]],device atomic_uint* status [[buffer(8)]],constant RepeatParams& p [[buffer(9)]],uint i [[thread_position_in_grid]]){
 if(i>=p.cells)return;output[i]=uint4(0u);uint4 next=repeat_prefix(selected,total,i+1u,p);if(next.x==selected[i].x)return;
 if(atomic_load_explicit(status,memory_order_relaxed))return;
 View v=view(data);uint semantic=repeat_semantic_task(domain,p,nodes[i].x),ei=repeat_eval(v,semantic,p);
 if(semantic==NONE || ei==NONE){atomic_store_explicit(status,2u,memory_order_relaxed);return;}if(failures[ei])return;
 Evaluation e=v.evaluations[ei];uint dense=repeat_dense_task(domain,p,e.semantic_start);
 if(dense==NONE || dense>=p.tasks){atomic_store_explicit(status,2u,memory_order_relaxed);return;}uint first=tasks[dense].x;
 if(first>i){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 if(selected[i].x-selected[first].x<v.limits[e.program])output[i]=next-selected[i];
}
kernel void repeat_results(device const uchar* data [[buffer(0)]],device const uint4* nodes [[buffer(1)]],device const uint4* descriptors [[buffer(2)]],device const uint4* tasks [[buffer(3)]],device const uint4* raw [[buffer(4)]],device const uint4* selected [[buffer(5)]],device Result* output [[buffer(6)]],device const uint4* domain [[buffer(7)]],device atomic_uint* status [[buffer(8)]],constant RepeatParams& p [[buffer(9)]],uint i [[thread_position_in_grid]]){
 if(i>=p.cells)return;uint4 next=i+1u==p.cells?uint4(p.results,p.captures,0u,0u):selected[i+1u];if(next.x==selected[i].x)return;
 uint4 cell=nodes[i];uint semantic=repeat_semantic_task(domain,p,cell.x);
 if(semantic==NONE || cell.x>=p.tasks){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 uint4 d=descriptors[cell.x];View v=view(data);Program pg=v.programs[d.w];uint ordinal=raw[i].x-raw[tasks[cell.x].x].x;
 uint label=v.words[pg.structural_start+repeat_size(d.w,0u).z]>>16;
 output[selected[i].x]={semantic,ordinal,d.x,label,selected[i].y,next.y-selected[i].y,0u,0u,0u,0u};
}
kernel void repeat_captures(device const uchar* data [[buffer(0)]],device const uint4* nodes [[buffer(1)]],device const uint4* descriptors [[buffer(2)]],device const uint4* selected [[buffer(3)]],device const uint4* values [[buffer(4)]],device Capture* output [[buffer(5)]],constant RepeatParams& p [[buffer(6)]],uint i [[thread_position_in_grid]]){
 if(i>=p.captures)return;uint row=repeat_owner(selected,p.cells,i,1u),at=i-selected[row].y;uint4 head=nodes[row],d=descriptors[head.x];
 View v=view(data);Program pg=v.programs[d.w];uint3 rootcaps=repeat_size(d.w,0u);uint before=head.w;
 uint pc=NONE,node=d.x;
 if(at<rootcaps.x)pc=repeat_pc(d.w,0u,at);
 else if(at>=rootcaps.x+before && at<rootcaps.x+before+rootcaps.y)pc=repeat_pc(d.w,0u,rootcaps.x+at-rootcaps.x-before);
 else {uint local=at-rootcaps.x-(at>=rootcaps.x+before+rootcaps.y?rootcaps.y:0u),global=values[row].y+local;
  uint source=repeat_owner(values,p.cells,global,1u);uint4 cell=nodes[source];node=cell.y;pc=repeat_pc(d.w,cell.z,global-values[source].y);
 }
 Node captured=v.nodes[node];uint label=v.words[pg.structural_start+pc]>>16;
 output[i]={label,node,captured.source_start,captured.source_end,NONE,0u,0u,0u,1u};
}
