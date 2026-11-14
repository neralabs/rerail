// Virtual expansion rows own bounded register frames, never global candidates.
struct RangeParams {uint roots,work,blocks,tasks,spans,programs,nodes,states,cursors,repeats,captures,digits,work_class;};
bool range_root(View v,device const uint* s,device const uint* h,constant RangeParams& p,uint i,thread uint& task,thread uint& program,thread uint& root) {
 if(h[0]!=p.roots || h[1] || h[2]!=(p.roots+255u)/256u || h[3]!=1u || h[4]!=1u || i>=p.roots || v.header->program_count!=p.programs || v.header->span_count!=p.spans)return false;
 task=s[i];
 if(task>=p.tasks)return false;
 uint lo=0u,hi=p.spans;
 while(lo<hi){uint mid=lo+(hi-lo)/2u;if(v.spans[v.semantic_order[mid]].semantic_start<=task)lo=mid+1u;else hi=mid;}
 if(!lo)return false;
 Span span=v.spans[v.semantic_order[lo-1u]];Evaluation e=v.evaluations[span.evaluation];File file=v.files[e.file];
 if(task-span.semantic_start>=span.root_count || e.program>=p.programs)return false;
 uint index=select_root(v,v.selectors[e.selector],span.selector_offset+task-span.semantic_start);
 if(index>=v.header->root_count)return false;
 Root selected=v.roots[index];
 if(selected.file!=e.file || selected.node>=file.node_count)return false;
 root=file.node_start+selected.node;program=e.program;
 return root<p.nodes;

}
void range_init(View v,Program pg,uint signature,uint root,thread Step& x) {
 x={};uint4 shape=range_layout(signature);if(any(shape==uint4(NONE))){x.boundary=2u;return;}
 x.repeat_offset=SUCCESSOR_HEADER_WORDS+2u*shape.x;x.capture_offset=(x.repeat_offset+shape.y+3u)&~3u;x.branch_offset=x.capture_offset+4u*shape.z;
 x.node=root;x.kind=(pg.flags&2u)?0u:2u;x.live=1u;x.error=NONE;x.query_order=(pg.flags&1u)?1u:(pg.capture_count?2u:0u);
}
void range_error(device atomic_uint* status,thread Step& x) {atomic_store_explicit(status,2u,memory_order_relaxed);x.live=0u;}
uint range_work(View v,uint program,uint signature,uint root,constant RangeParams& p,thread Step& x,device atomic_uint* status) {
 Program pg=v.programs[program];range_init(v,pg,signature,root,x);
 if(x.boundary){range_error(status,x);return 0u;}
 uint complete=range_complete(signature);
 if(complete>1u || p.work_class>2u){range_error(status,x);return 0u;}
 if((p.work_class==1u && complete) || (p.work_class==2u && !complete)){x.live=0u;return 0u;}
 range_prefix(v,pg,signature,p.nodes,x);
 if(x.boundary || x.error!=NONE || x.cursor_keep || x.repeat_keep){range_error(status,x);return 0u;}
 if(!x.live)return 0u;
 uint variable=range_variable(signature);if(variable==NONE){range_error(status,x);return 0u;}
 if(!variable)return 1u;
 if(x.expansion<3u || x.expansion>5u || x.end<x.begin){range_error(status,x);return 0u;}
 return x.end-x.begin;
}
void range_run(View v,uint program,uint signature,uint root,uint choice,constant RangeParams& p,thread Step& x,device atomic_uint* status) {
 uint work=range_work(v,program,signature,root,p,x,status);if(!x.live)return;
 if(choice>=work){range_error(status,x);return;}
 Program pg=v.programs[program];
 if(range_variable(signature)) {
  uint expansion=x.expansion,begin=x.begin,anchor=x.node;
  uint node=expansion==4u?begin+choice:v.children[begin+choice];
  if(node>=p.nodes){range_error(status,x);return;}
  if(x.named && !(v.nodes[node].flags&1u)){x.live=0u;return;}
  uint rank=choice;
  if(x.named) {
   uint before=expansion==4u?v.named_order[begin].preorder:(expansion==5u?v.named_order[anchor].sibling+uint((v.nodes[anchor].flags&1u)!=0u):0u);
   uint ordinal=expansion==4u?v.named_order[node].preorder:v.named_order[node].sibling;
   if(ordinal<before){range_error(status,x);return;}rank=ordinal-before;
  }
  if(x.branch_count>=64u){range_error(status,x);return;}
  step_branches(x)[x.branch_count++]=rank;x.node=node;x.expansion=0u;
  range_suffix(v,pg,signature,p.nodes,x);
 }
 if(x.boundary>1u || x.cursor_keep || x.repeat_keep){range_error(status,x);return;}
 if(x.error!=NONE){x.pc=x.error;x.kind=frame_query(x.kind)?FRAME_QUERY_OPERAND_FAILURE:FRAME_ORDINARY_OPERAND_FAILURE;x.error=NONE;x.live=1u;x.expansion=0u;}
 if(x.boundary){x.kind=frame_query(x.kind)?FRAME_QUERY_PREDICATE:FRAME_ORDINARY_PREDICATE;x.live=1u;x.expansion=0u;}
 if(x.pc>=pg.structural_count){x.live=0u;return;}
 if(x.expansion || x.cursor_add>32u || x.repeat_add>32u || x.capture_add>32u || x.branch_count>64u){range_error(status,x);return;}
 if(x.live && x.query_order!=2u)for(uint a=1u;a<x.capture_add;a++){uint4 value=step_captures(x)[a];uint b=a;while(b && capture_edit_less(value,step_captures(x)[b-1u],x.query_order)){step_captures(x)[b]=step_captures(x)[b-1u];b--;}step_captures(x)[b]=value;}
}
uint range_owner(device const uint* starts,uint count,uint row) {uint lo=0u,hi=count;while(lo<hi){uint mid=lo+(hi-lo)/2u;if(starts[mid]<=row)lo=mid+1u;else hi=mid;}return lo-1u;}
kernel void initial_range_root_counts(device const uchar* data [[buffer(0)]],device const uint* seeds [[buffer(1)]],device const uint* header [[buffer(2)]],device uint* starts [[buffer(3)]],device uint4* blocks [[buffer(4)]],device atomic_uint* status [[buffer(5)]],device const uint* units [[buffer(6)]],constant RangeParams& p [[buffer(7)]],uint i [[thread_position_in_grid]],uint group [[threadgroup_position_in_grid]],uint lane [[thread_index_in_threadgroup]]) {
 threadgroup uint counts[256];uint count=0u;View v=view(data);
 if(i<p.roots && !atomic_load_explicit(status,memory_order_relaxed)) {
  uint task,program,root;
  if(!range_root(v,seeds,header,p,i,task,program,root))atomic_store_explicit(status,2u,memory_order_relaxed);
  else {thread uint4 arena[(RANGE_FRAME_WORDS+3u)/4u];thread Step& x=step_at((thread uint*)arena,0u);count=range_work(v,program,units[program],root,p,x,status);}
 }
 counts[lane]=count;threadgroup_barrier(mem_flags::mem_threadgroup);
 for(uint step=1u;step<256u;step<<=1u){uint add=lane>=step?counts[lane-step]:0u;threadgroup_barrier(mem_flags::mem_threadgroup);ulong sum=ulong(counts[lane])+add;counts[lane]=uint(min(sum,0xfffffffful));threadgroup_barrier(mem_flags::mem_threadgroup);}
 if(i<p.roots)starts[i]=lane?counts[lane-1u]:0u;
 if(!lane)blocks[group]=uint4(counts[255],0u,0u,0u);
}
kernel void initial_range_root_offsets(device uint* starts [[buffer(0)]],device const uint4* blocks [[buffer(1)]],device const uint4* total [[buffer(2)]],device atomic_uint* status [[buffer(3)]],constant RangeParams& p [[buffer(4)]],uint i [[thread_position_in_grid]]) {
 if(total[0].x==NONE){if(!i)atomic_store_explicit(status,1u,memory_order_relaxed);return;}
 if(i<p.roots)starts[i]+=blocks[i/256u].x;
 if(i==p.roots)starts[i]=total[0].x;
}
struct RangeOutput {device uint* columns;device uint* spans;device uint* path_work;device uint4* cursors;device uint4* repeats;device uint4* captures;device uint* digits;};
void range_store(View v,uint task,uint program,thread const Step& x,uint at,uint4 values,uint digits,RangeOutput out,constant RangeParams& p,device atomic_uint* status) {
 if(at>=p.states || values.y+x.cursor_add>p.cursors || values.z+x.repeat_add>p.repeats || values.w+x.capture_add>p.captures || digits+x.branch_count>p.digits){atomic_store_explicit(status,1u,memory_order_relaxed);return;}
 uint n=p.states;Program pg=v.programs[program];
 out.columns[at]=task;out.columns[n+at]=x.pc;out.columns[2u*n+at]=x.node;out.columns[3u*n+at]=successor_requirement(v,pg,x.pc);out.columns[4u*n+at]=program;out.columns[5u*n+at]=x.kind;
 out.columns[6u*n+at]=values.y;out.columns[7u*n+at]=x.cursor_add;out.columns[8u*n+at]=values.z;out.columns[9u*n+at]=x.repeat_add;out.columns[10u*n+at]=values.w;out.columns[11u*n+at]=x.capture_add;
 out.spans[at]=digits;out.spans[n+at]=x.branch_count;out.spans[2u*n+at]=digits+x.branch_count;out.spans[3u*n+at]=0u;out.spans[4u*n+at]=digits+x.branch_count;out.spans[5u*n+at]=0u;out.path_work[at]=x.path_work;
 for(uint j=0u;j<x.cursor_add;j++)out.cursors[values.y+j]=uint4(step_cursors(x)[j],0u,0u);
 for(uint j=0u;j<x.repeat_add;j++)out.repeats[values.z+j]=uint4(j,step_repeats(x)[j],0u,0u);
 for(uint j=0u;j<x.capture_add;j++)out.captures[values.w+j]=step_captures(x)[j];
 for(uint j=0u;j<x.branch_count;j++)out.digits[digits+j]=step_branches(x)[j];
}
// Count and scatter use the same pure bounded frame function. Only block totals
// remain global; the scatter retains its frame across the cooperative scan.
kernel void initial_range_counts(device const uchar* data [[buffer(0)]],device const uint* seeds [[buffer(1)]],device const uint* header [[buffer(2)]],device const uint* starts [[buffer(3)]],device uint4* blocks [[buffer(4)]],device uint4* digit_blocks [[buffer(5)]],device atomic_uint* status [[buffer(6)]],device const uint* units [[buffer(7)]],constant RangeParams& p [[buffer(8)]],uint i [[thread_position_in_grid]],uint group [[threadgroup_position_in_grid]],uint lane [[thread_index_in_threadgroup]]) {
 if(group>=p.blocks)return;threadgroup uint4 counts[256];threadgroup uint digits[256];
 uint4 value=uint4(0u);uint length=0u;View v=view(data);
 if(i<p.work && !atomic_load_explicit(status,memory_order_relaxed)) {
  uint owner=range_owner(starts,p.roots,i),task,program,root;
  if(!range_root(v,seeds,header,p,owner,task,program,root))atomic_store_explicit(status,2u,memory_order_relaxed);
  else {thread uint4 arena[(RANGE_FRAME_WORDS+3u)/4u];thread Step& x=step_at((thread uint*)arena,0u);range_run(v,program,units[program],root,i-starts[owner],p,x,status);if(x.live){value=uint4(1u,x.cursor_add,x.repeat_add,x.capture_add);length=x.branch_count;}}
 }
 counts[lane]=value;digits[lane]=length;threadgroup_barrier(mem_flags::mem_threadgroup);
 for(uint step=1u;step<256u;step<<=1u){uint4 add=lane>=step?counts[lane-step]:uint4(0u);uint d=lane>=step?digits[lane-step]:0u;threadgroup_barrier(mem_flags::mem_threadgroup);counts[lane]+=add;digits[lane]+=d;threadgroup_barrier(mem_flags::mem_threadgroup);}
 if(!lane){blocks[group]=counts[255];digit_blocks[group]=uint4(digits[255],0u,0u,0u);}
}
kernel void initial_range_scatter(device const uchar* data [[buffer(0)]],device const uint* seeds [[buffer(1)]],device const uint* header [[buffer(2)]],device const uint* starts [[buffer(3)]],device const uint4* blocks [[buffer(4)]],device const uint4* digit_blocks [[buffer(5)]],device uint* columns [[buffer(6)]],device uint* spans [[buffer(7)]],device uint* path_work [[buffer(8)]],device uint4* cursors [[buffer(9)]],device uint4* repeats [[buffer(10)]],device uint4* captures [[buffer(11)]],device uint* output_digits [[buffer(12)]],device atomic_uint* status [[buffer(13)]],device const uint* units [[buffer(14)]],constant RangeParams& p [[buffer(15)]],uint i [[thread_position_in_grid]],uint group [[threadgroup_position_in_grid]],uint lane [[thread_index_in_threadgroup]]) {
 if(group>=p.blocks)return;threadgroup uint4 counts[256];threadgroup uint digits[256];
 thread uint4 arena[(RANGE_FRAME_WORDS+3u)/4u];thread Step& x=step_at((thread uint*)arena,0u);x={};
 uint task=0u,program=0u,root=0u;View v=view(data);
 if(i<p.work && !atomic_load_explicit(status,memory_order_relaxed)) {
  uint owner=range_owner(starts,p.roots,i);
  if(!range_root(v,seeds,header,p,owner,task,program,root))atomic_store_explicit(status,2u,memory_order_relaxed);
  else range_run(v,program,units[program],root,i-starts[owner],p,x,status);
 }
 uint4 value=x.live?uint4(1u,x.cursor_add,x.repeat_add,x.capture_add):uint4(0u);uint length=x.live?x.branch_count:0u;
 counts[lane]=value;digits[lane]=length;threadgroup_barrier(mem_flags::mem_threadgroup);
 for(uint step=1u;step<256u;step<<=1u){uint4 add=lane>=step?counts[lane-step]:uint4(0u);uint d=lane>=step?digits[lane-step]:0u;threadgroup_barrier(mem_flags::mem_threadgroup);counts[lane]+=add;digits[lane]+=d;threadgroup_barrier(mem_flags::mem_threadgroup);}
 if(!x.live || atomic_load_explicit(status,memory_order_relaxed))return;
 uint4 offset=blocks[group]+(lane?counts[lane-1u]:uint4(0u));uint digit=digit_blocks[group].x+(lane?digits[lane-1u]:0u);
 range_store(v,task,program,x,offset.x,offset,digit,{columns,spans,path_work,cursors,repeats,captures,output_digits},p,status);
}
kernel void initial_range_headers(device uint* states [[buffer(0)]],device uint* values [[buffer(1)]],device uint* control [[buffer(2)]],device const uint* status [[buffer(3)]],constant RangeParams& p [[buffer(4)]],uint i [[thread_position_in_grid]]) {
 if(i || status[0])return;states[0]=p.states;states[1]=p.captures;states[2]=(p.states+255u)/256u;states[3]=states[4]=1u;values[0]=p.cursors;values[1]=p.repeats;control[0]=p.digits;
}
