// Instantiate these same operations for device frames and register frames.
STEP_ADDRESS Step& step_at(STEP_ADDRESS uint* arena,uint word) {return *((STEP_ADDRESS Step*)(arena+word));}
STEP_ADDRESS const Step& step_at(STEP_ADDRESS const uint* arena,uint word) {return *((STEP_ADDRESS const Step*)(arena+word));}
STEP_ADDRESS uint2* step_cursors(STEP_ADDRESS Step& x) {return (STEP_ADDRESS uint2*)((STEP_ADDRESS uint*)&x+SUCCESSOR_HEADER_WORDS);}
STEP_ADDRESS const uint2* step_cursors(STEP_ADDRESS const Step& x) {return (STEP_ADDRESS const uint2*)((STEP_ADDRESS const uint*)&x+SUCCESSOR_HEADER_WORDS);}
STEP_ADDRESS uint* step_repeats(STEP_ADDRESS Step& x) {return (STEP_ADDRESS uint*)&x+x.repeat_offset;}
STEP_ADDRESS const uint* step_repeats(STEP_ADDRESS const Step& x) {return (STEP_ADDRESS const uint*)&x+x.repeat_offset;}
STEP_ADDRESS uint4* step_captures(STEP_ADDRESS Step& x) {return (STEP_ADDRESS uint4*)((STEP_ADDRESS uint*)&x+x.capture_offset);}
STEP_ADDRESS const uint4* step_captures(STEP_ADDRESS const Step& x) {return (STEP_ADDRESS const uint4*)((STEP_ADDRESS const uint*)&x+x.capture_offset);}
STEP_ADDRESS uint* step_branches(STEP_ADDRESS Step& x) {return (STEP_ADDRESS uint*)&x+x.branch_offset;}
STEP_ADDRESS const uint* step_branches(STEP_ADDRESS const Step& x) {return (STEP_ADDRESS const uint*)&x+x.branch_offset;}
__attribute__((noinline)) bool successor_charge(STEP_ADDRESS Step& x) {
 if(x.units==SUCCESSOR_UNIT_INSTRUCTIONS)return false;
 if(x.path_work>=PATH_WORK_LIMIT){x.path_work=PATH_WORK_LIMIT+1u;x.kind=frame_query(x.kind)?FRAME_QUERY_WORK_FAILURE:FRAME_ORDINARY_WORK_FAILURE;x.live=1u;x.expansion=0u;return false;}
 x.path_work++;x.units++;return true;
}
void save_cursor(device const uint4* arena,uint start,uint slot,STEP_ADDRESS Step& x) {
 x.cursor_keep=cursor_lower(arena,start,x.cursor_keep,slot);
 while(x.cursor_add>0u && step_cursors(x)[x.cursor_add-1u].x>=slot)x.cursor_add--;
 step_cursors(x)[x.cursor_add++]=uint2(slot,x.node);
}
void restore_cursor(device const uint4* arena,uint start,uint slot,STEP_ADDRESS Step& x) {
 uint node=NONE,at=cursor_lower(arena,start,x.cursor_keep,slot);
 if(at<x.cursor_keep && arena[start+at].x==slot)node=arena[start+at].y;
 for(uint j=0u;j<PREFIX_EDIT_CAPACITY;j++)if(j<x.cursor_add && step_cursors(x)[j].x==slot)node=step_cursors(x)[j].y;
 if(node==NONE)x.error=x.pc;else x.node=node;
}
void repeat_top(device const uint4* arena,uint start,uint expected,uint pop,STEP_ADDRESS Step& x) {
 uint value=x.repeat_add?step_repeats(x)[x.repeat_add-1u]:(x.repeat_keep?arena[start+x.repeat_keep-1u].y:NONE);
 if(value!=expected){x.error=x.pc;return;}
 if(pop){if(x.repeat_add)x.repeat_add--;else x.repeat_keep--;}
}
void expand_nodes(View v,uint op,uint named,STEP_ADDRESS Step& x) {
 Node node=v.nodes[x.node];x.named=named;x.expansion=op;
 if(op==3u){x.begin=node.child_start;x.end=node.child_start+node.child_count;}
 else if(op==4u){x.begin=x.node+1u;x.end=node.descendant_end;}
 else if(node.parent==NONE){x.begin=x.end=0u;}
 else {
  Node parent=v.nodes[node.parent];uint lo=parent.child_start,hi=lo+parent.child_count;
  while(lo<hi){uint mid=lo+(hi-lo)/2u;if(v.children[mid]<=x.node)lo=mid+1u;else hi=mid;}
  x.begin=lo;x.end=parent.child_start+parent.child_count;
 }
}
void initial_child(View v,Program pg,uint node_count,uint guard,uint selector,uint named,uint maximum,STEP_ADDRESS Step& x) {
 Node parent=v.nodes[x.node];
 if(parent.child_count>maximum || x.branch_count>=64u || selector>3u){x.boundary=2u;return;}
 uint child=selector<2u?unique_child(v,x.node,v.words[pg.structural_start+x.pc+guard]>>16,selector):
  (selector==2u?(parent.child_count?v.children[parent.child_start]:NONE):v.relations[x.node].x);
 if(child==NONE){x.live=0u;return;}
 if(child>=node_count || v.nodes[child].parent!=x.node){x.boundary=2u;return;}
 if(named && !(v.nodes[child].flags&1u)){x.live=0u;return;}
 uint lo=parent.child_start,hi=lo+parent.child_count;
 while(lo<hi){uint mid=lo+(hi-lo)/2u;if(v.children[mid]<child)lo=mid+1u;else hi=mid;}
 if(lo>=parent.child_start+parent.child_count || v.children[lo]!=child){x.boundary=2u;return;}
 uint rank=named?v.named_order[child].sibling:lo-parent.child_start;
 step_branches(x)[x.branch_count++]=rank;x.node=child;
}
