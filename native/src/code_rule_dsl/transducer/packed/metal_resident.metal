#include <metal_stdlib>
using namespace metal;
constant uint NONE=0xffffffffu;
// Finished operand failures retain the structural PC and full logical position.
constant uint FRAME_QUERY_OPERAND_FAILURE=8u,FRAME_ORDINARY_OPERAND_FAILURE=9u;
constant uint FRAME_QUERY_PREDICATE=10u,FRAME_ORDINARY_PREDICATE=11u;
constant uint FRAME_QUERY_WORK_FAILURE=12u,FRAME_ORDINARY_WORK_FAILURE=13u;
constant uint PATH_WORK_LIMIT=1048576u;
bool frame_predicate(uint kind){return kind==FRAME_QUERY_PREDICATE || kind==FRAME_ORDINARY_PREDICATE;}
bool frame_work_failure(uint kind){return kind==FRAME_QUERY_WORK_FAILURE || kind==FRAME_ORDINARY_WORK_FAILURE;}
bool frame_query(uint kind){return kind==0u || kind==1u || kind==4u || kind==6u || kind==FRAME_QUERY_OPERAND_FAILURE || kind==FRAME_QUERY_PREDICATE || kind==FRAME_QUERY_WORK_FAILURE;}
bool frame_pending(uint kind){return kind==6u || kind==7u;}
bool frame_terminal(uint kind){return kind==1u || kind==3u;}
bool frame_operand_failure(uint kind){return kind==FRAME_QUERY_OPERAND_FAILURE || kind==FRAME_ORDINARY_OPERAND_FAILURE;}
bool frame_finished(uint kind){return frame_terminal(kind) || frame_operand_failure(kind) || frame_work_failure(kind);}
bool operand_failure_opcode(uint op){return op==0x16u || op==0x22u || op==0x23u;}
struct Header {
 uint nodes,files,roots,selectors,selector_data,evaluations,spans,blocks,words,programs,longs;
 uint relations,fields,supertypes,semantic_order,span_positions,eval_first,limits,children;
 uint root_count,field_count,supertype_count;
 uint source,strings,string_bytes,line_starts,output_words,accepted_limits;
 uint span_count,evaluation_count,program_count,source_bytes,output_word_count;
 uint values,assertions;
 uint unique_fields,unique_kinds,unique_field_count,unique_kind_count;
 uint named_order;
 uint value_prefixes;
};
static_assert(sizeof(Header)==41u*4u,"resident header layout");
struct NamedOrder {uint sibling,preorder;};
static_assert(sizeof(NamedOrder)==8u,"named range ordering layout");
struct Node { uint parent,next_any,next_named,child_start,child_count,descendant_end,source_start,source_end,kind,field,flags; };
struct File { uint node_start,node_count,source_start,source_length,line_start,line_count,supertype_start,supertype_count; };
struct Root { uint file,node,ordinal; };
struct Selector { uint kind,file,data_start,data_count,root_count,reserved; };
struct Evaluation { uint evaluation,file,program,signature,work_class,route,selector,root_count,semantic_start; };
struct Span { uint evaluation,block,selector_offset,root_count,lane_start,semantic_start; };
struct Block { uint span_start,span_count,signature,route,logical_count,reserved; };
struct Program { uint structural_start,structural_count,long_start,long_count,string_start,string_count,value_start,value_count,capture_start,capture_count,output_start,output_count,label_start,label_count,maximum_cursor_slot,maximum_lua_capture_slot,flags; };
struct Assertion { uint kind,first,second,data_start,data_count,flags; };
struct Value { uint kind,operand,auxiliary,state_start,state_count; };
struct Result { uint task_ordinal,local_ordinal,start,label,capture_start,capture_count,property_start,property_count,dynamic_start,dynamic_count; };
struct Capture { uint label,node,start,end,text_start,text_length,property_start,property_count,retained; };
struct Property {uint kind,owner,key,value,key_start,key_length,value_start,value_length;};
constant uint REGISTER_ALL=0u,REGISTER_SHARED=1u,REGISTER_CONSTANT=2u;
struct Params { uint first,count,blocks,limited,eval_first,eval_count,result_capacity,capture_capacity,output_class; };
struct View {
 device const Header* header; device const Node* nodes; device const File* files; device const Root* roots;
 device const Selector* selectors; device const uint* selector_data; device const Evaluation* evaluations;
 device const Span* spans; device const Block* blocks; device const uint* words; device const Program* programs;
 device const ulong* longs; device const uint2* relations; device const uint2* fields; device const uint2* supertypes;
 device const uint* semantic_order; device const uint* span_positions; device const uint* eval_first;
 device const uint* limits; device const uint* children;
 device const uchar* source;device const uint2* strings;device const uchar* string_bytes;
 device const uint* line_starts;device const uint* output_words;device const uint* accepted_limits;
 device const Value* values;device const Assertion* assertions;
 device const uint* unique_fields;device const uint* unique_kinds;
 device const NamedOrder* named_order;
 device const uint* value_prefixes;
};
View view(device const uchar* data) {
 device const Header* h=(device const Header*)data;
 return {h,(device const Node*)(data+h->nodes),(device const File*)(data+h->files),(device const Root*)(data+h->roots),
 (device const Selector*)(data+h->selectors),(device const uint*)(data+h->selector_data),(device const Evaluation*)(data+h->evaluations),
 (device const Span*)(data+h->spans),(device const Block*)(data+h->blocks),(device const uint*)(data+h->words),(device const Program*)(data+h->programs),
 (device const ulong*)(data+h->longs),(device const uint2*)(data+h->relations),(device const uint2*)(data+h->fields),(device const uint2*)(data+h->supertypes),
 (device const uint*)(data+h->semantic_order),(device const uint*)(data+h->span_positions),(device const uint*)(data+h->eval_first),
 (device const uint*)(data+h->limits),(device const uint*)(data+h->children),
 data+h->source,(device const uint2*)(data+h->strings),data+h->string_bytes,
 (device const uint*)(data+h->line_starts),(device const uint*)(data+h->output_words),(device const uint*)(data+h->accepted_limits),(device const Value*)(data+h->values),(device const Assertion*)(data+h->assertions),
 (device const uint*)(data+h->unique_fields),(device const uint*)(data+h->unique_kinds),(device const NamedOrder*)(data+h->named_order),(device const uint*)(data+h->value_prefixes)};
}
uint unique_child(View v,uint parent,uint key,uint by_kind) {
 device const uint* ids=by_kind?v.unique_kinds:v.unique_fields;
 uint count=by_kind?v.header->unique_kind_count:v.header->unique_field_count,lo=0u,hi=count;
 while(lo<hi) {
  uint mid=lo+(hi-lo)/2u;Node child=v.nodes[ids[mid]];uint value=by_kind?child.kind:child.field;
  if(child.parent<parent || (child.parent==parent && value<key))lo=mid+1u;else hi=mid;
 }
 if(lo==count)return NONE;
 Node child=v.nodes[ids[lo]];
 return child.parent==parent && (by_kind?child.kind:child.field)==key?ids[lo]:NONE;
}
// At most 64 source bytes per finite operation. Larger predicates suspend
// before this helper and belong to the shared value frontier.
uint finite_text(View v,Program pg,uint operand,uint node) {
 Value value=v.values[pg.value_start+operand];
 if(value.kind>1u) return NONE;
 uint2 literal=v.strings[pg.string_start+value.operand];
 if(literal.y>64u) return NONE;
 Node n=v.nodes[node];uint length=n.source_end-n.source_start;
 if(length<literal.y || (value.kind==0u && length!=literal.y)) return 0u;
 bool equal=true;
 for(uint i=0u;i<64u;i++) if(i<literal.y)
  equal=equal && v.source[n.source_start+i]==v.string_bytes[literal.x+i];
 return uint(equal);
}

bool has_pair(device const uint2* pairs,uint count,uint node,uint key) {
 uint low=0,high=count;
 while(low<high) { uint mid=low+(high-low)/2; uint2 pair=pairs[mid]; if(pair.x<node || (pair.x==node && pair.y<key)) low=mid+1; else high=mid; }
 return low<count && all(pairs[low]==uint2(node,key));
}
bool at_edge(View v,uint node,uint last,uint named) {
 uint parent=v.nodes[node].parent;
 if(parent==NONE) return true;
 Node p=v.nodes[parent];
 uint edge=named?(last?v.relations[parent].y:v.relations[parent].x):(p.child_count?v.children[p.child_start+(last?p.child_count-1u:0u)]:NONE);
 return node==edge;
}
bool capture_less(View v,uint2 a,uint2 b,bool query) {
 Node left=v.nodes[a.x],right=v.nodes[b.x];
 return left.source_start<right.source_start || (left.source_start==right.source_start && (left.source_end>right.source_end || (left.source_end==right.source_end && !query && a.y<b.y)));
}
uint select_root(View v,Selector selector,uint offset) {
 device const uint* data=v.selector_data+selector.data_start;
 if(selector.kind==1u) return data[0]+offset;
 if(selector.kind==4u) return data[offset];
 if(selector.kind==2u) {
  uint low=0,high=selector.data_count/3u;
  while(low<high) { uint mid=low+(high-low)/2; if(data[mid*3u+2u]<=offset) low=mid+1u; else high=mid; }
  uint run=low-1u; return data[run*3u]+offset-data[run*3u+2u];
 }
 uint blocks=data[1],low=0,high=blocks;
 while(low<high) { uint mid=low+(high-low)/2; if(data[2u+mid]<=offset) low=mid+1u; else high=mid; }
 uint block=low-1u,remaining=offset-data[2u+block];
 uint words=selector.data_count-2u-blocks;
 for(uint word=block*32u;word<min((block+1u)*32u,words);word++) {
  uint bits=data[2u+blocks+word],count=popcount(bits);
  if(remaining<count) { for(uint i=0;i<remaining;i++) bits&=bits-1u; return data[0]+word*32u+ctz(bits); }
  remaining-=count;
 }
 return NONE;
}
kernel void register_roots(device const uchar* data [[buffer(0)]],device uint* resolved [[buffer(1)]],uint tid [[thread_position_in_grid]]) {
 View v=view(data); if(tid<v.header->root_count) { Root root=v.roots[tid]; resolved[tid]=v.files[root.file].node_start+root.node; }
}
struct Lane { uint span,position,node,ordinal; Program pg; };
Lane lane_input(View v,Block block,uint lane,device const uint* resolved,Params p) {
 Lane out={NONE,NONE,0u,0u,{}};
 if(lane>=block.logical_count) return out;
 uint low=0,high=block.span_count;
 while(low<high) { uint mid=low+(high-low)/2; if(v.spans[block.span_start+mid].lane_start<=lane) low=mid+1u; else high=mid; }
 uint id=block.span_start+low-1u,position=v.span_positions[id];
 if(position<p.first || position-p.first>=p.count) return out;
 Span span=v.spans[id]; Evaluation evaluation=v.evaluations[span.evaluation];
 uint local=lane-span.lane_start;
 out.span=id; out.position=position-p.first;
 out.node=resolved[select_root(v,v.selectors[evaluation.selector],span.selector_offset+local)];
 out.ordinal=span.semantic_start+local; out.pg=v.programs[evaluation.program];
 return out;
}
