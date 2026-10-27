// Check the complete initial request arena before allocation or semantic work.
kernel void value_frontier_preflight(
 device const uchar* data [[buffer(0)]],device const uint* code [[buffer(1)]],
 device const RegexRequest* requests [[buffer(2)]],device const uint* ids [[buffer(3)]],
 device atomic_uint* status [[buffer(4)]],device atomic_uint* modes [[buffer(5)]],device const uchar* external_bytes [[buffer(6)]],device const uint* deletion [[buffer(7)]],constant RegexParams& p [[buffer(8)]],
 uint i [[thread_position_in_grid]]){
 if(atomic_load_explicit(status,memory_order_relaxed))return;
 if(p.external_source && i<p.source_bytes && deletion[i]!=1u)atomic_store_explicit(status,2u,memory_order_relaxed);
 if(i>=p.requests)return;
 bool valid=p.scratch_plan<=1u && (!p.scratch_plan || p.unit_bytes==REGEX_LOCAL_BYTES) && !p.generation && p.active==p.requests && p.unit_bytes && p.unit_bytes<=65536u
  && p.unit_actions && p.unit_actions<=65536u && p.abort_evaluation<=1u && p.code_words>=4u && p.external_source<=1u && p.owner_work_limit<=REGEX_WORK_LIMIT;
 if(valid)valid=code[1]==4u && code[0]<=(p.code_words-4u)/4u && ulong(code[2])+ulong(code[3])*2ul<=p.code_words;
 RegexRequest r=requests[i];View v=view(data);uint physical=r.width,nfa_width=0u;
 uint source_size=p.external_source?p.source_bytes:v.header->source_bytes;
 device const uchar* source=p.external_source?external_bytes:v.source;
 valid=valid && (!p.scratch_plan || (!p.external_source && !r.mode)) && ids[i]==i && !r.status && !r.phase && r.at==r.search_start && !r.used && (p.external_source?r.mode==2u:r.mode<=1u) && !r.progress && !r.strip_cursor && !r.strip_limit && !r.strip_next && (r.mode || !r.search_start) && r.search_start<=r.source_length
  && r.match_start==NONE && r.match_end==NONE && r.owner<p.budget_owners && r.evaluation<p.evaluations
  && r.budget<=p.owner_work_limit && r.source_start<=source_size
  && r.source_length<=source_size-r.source_start && r.value<code[0]
  && r.width && !(r.width&(r.width-1u)) && r.scratch<=p.scratch_slots;
 if(valid){
  ulong descriptor=ulong(code[1])+ulong(r.value)*4ul;valid=descriptor+4ul<=p.code_words;
  if(valid){
   uint item_count=code[uint(descriptor)],width=code[uint(descriptor)+1u],table=code[uint(descriptor)+2u],engine=code[uint(descriptor)+3u];
   valid=width==r.width && table<=p.code_words;
   if(valid && p.scratch_plan==1u && regex_workgroup_predicate(code,r.value,width,r.source_length,r.mode))physical=0u;
   if(valid && engine==0u){
    valid=item_count && width>=item_count && 100u<=p.code_words-table;
    if(valid && physical)nfa_width=width;
    if(valid){device const uchar* bytes=source+r.source_start;
     valid=(!r.source_length || (bytes[0]&0xc0u)!=0x80u)
      && (r.source_start+r.source_length==source_size || (source[r.source_start+r.source_length]&0xc0u)!=0x80u);
    }
   }else if(valid && engine==1u){
    valid=!r.mode && 8u<=p.code_words-table;
    if(valid){
     uint captures=code[table],flags=code[table+1u],stride=code[table+2u],slots=code[table+3u];
     valid=captures<=32u && flags<=3u && stride==8u+captures*3u && code[table+4u]==item_count
      && !code[table+5u] && !code[table+6u] && !code[table+7u]
      && ulong(slots)==8ul+(ulong(item_count)+1ul)*stride && slots<=width && width/2u<slots
      && ulong(item_count)*12ul<=p.code_words-table-8u;
    }
   }else valid=false;
  }
 }
 if(valid && p.external_source && i+1u<p.requests)valid=ulong(r.source_start)+r.source_length<=requests[i+1u].source_start;
 if(valid){
  ulong end=ulong(r.scratch)+physical;
  valid=physical<=p.scratch_slots-r.scratch;
  valid=valid && (i || !r.scratch) && (i+1u<p.requests?end==requests[i+1u].scratch:end==p.scratch_slots);
 }
 if(!valid)atomic_store_explicit(status,2u,memory_order_relaxed);
 else {
  if(r.mode)atomic_fetch_or_explicit(modes,1u,memory_order_relaxed);
  if(nfa_width)atomic_fetch_max_explicit(modes+1u,nfa_width,memory_order_relaxed);
 }
}
