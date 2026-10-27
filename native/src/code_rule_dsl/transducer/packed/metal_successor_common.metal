// Every variable value copy and comparison is a cell-parallel work domain.
struct SuccessorParams {
 uint capacity,output,tasks,spans,programs,nodes,cursors,repeats,captures,digits;
 uint out_cursors,out_repeats,out_captures,out_digits,signatures,pairs,work,active_slots,steps,task_slots,step_words,input_jobs;
};
struct Step {
 uint pc,node,begin,end,cursor_keep,repeat_keep,error,input_parent,path_work,logical_step;
 // Input validation checks kinds before this narrowing. Finite signatures bound
 // edits to 32, branches to 64, instructions to 256, and offsets to 240 words.
 ushort repeat_offset,capture_offset,branch_offset,units;
 uchar kind,live,expansion,named,cursor_add,repeat_add,capture_add;
 uchar boundary,query_order,branch_count;
 uchar padding[6];
};
static_assert(sizeof(Step)==SUCCESSOR_HEADER_WORDS*4u,"finite successor header layout");
// SUCCESSOR_LAYOUTS
uint successor_step_words(uint4 shape) {
 uint capture=(SUCCESSOR_HEADER_WORDS+2u*shape.x+shape.y+3u)&~3u;
 return (capture+4u*shape.z+shape.w+3u)&~3u;
}
uint successor_requirement(View v,Program pg,uint pc) {uint op=v.words[pg.structural_start+pc]&255u;return op==17u?1u:(op==7u?2u:(op==8u?3u:0u));}

uint sc(device const uint* s,uint n,uint field,uint i) {return s[field*n+i];}
uint owner(device const uint4* offsets,uint n,uint index,uint component) {
 uint lo=0u,hi=n;
 while(lo<hi) {uint mid=lo+(hi-lo)/2u;if(offsets[mid][component]<=index)lo=mid+1u;else hi=mid;}
 return lo-1u;
}
bool geo_less(uint4 a,uint4 b) {return a.z<b.z || (a.z==b.z && a.w>b.w);}
bool capture_edit_less(uint4 a,uint4 b,uint query_order) {
 return geo_less(a,b) || (!query_order && a.z==b.z && a.w==b.w && a.y<b.y);
}
uint cursor_lower(device const uint4* arena,uint start,uint count,uint slot) {
 uint lo=0u,hi=count;while(lo<hi) {uint mid=lo+(hi-lo)/2u;if(arena[start+mid].x<slot)lo=mid+1u;else hi=mid;}return lo;
}
// STEP_HELPERS

bool query_kind(uint kind) {return frame_query(kind) && !frame_pending(kind);}
