// Capture arguments have no public result ordinals, properties, or output storage.
struct ArgumentCapture {uint node,label,start,end;};
struct CaptureSpan {uint capture_start,capture_count;};
struct PredicateSpec {uint kind,first,second,data_start,data_count,flags,index,reserved;};
struct CaptureParams {uint n,c,step,nodes,pairs,windows,units,abort_evaluation,width,layout;};
CaptureSpan argument_span(device const CaptureSpan* rows,uint i,constant CaptureParams& p) {
 if(p.layout==1u){Result row=((device const Result*)rows)[i];return {row.capture_start,row.capture_count};}
 return rows[i];
}
ArgumentCapture argument_capture(device const ArgumentCapture* captures,uint i,constant CaptureParams& p) {
 if(p.layout==1u){Capture cap=((device const Capture*)captures)[i];return {cap.node,cap.label,cap.start,cap.end};}
 return captures[i];
}
PredicateSpec query_spec(View v,uint4 metadata,uint step) {
 Program pg=v.programs[metadata.y];if(!(pg.flags&1u) || step>=pg.capture_count)return {NONE,0u,0u,0u,0u,0u,NONE,0u};
 uint index=pg.capture_start+step;Assertion a=v.assertions[index];return {a.kind,a.first,a.second,a.data_start,a.data_count,a.flags,index,0u};
}
PredicateSpec argument_spec(View v,device const uint4* metadata,device const PredicateSpec* specs,uint i,constant CaptureParams& p) {
 if(p.layout==1u)return query_spec(v,metadata[i],p.step);
 return specs[i];
}
bool capture_metadata_valid(View v,uint4 m) {
 if(m.x>=v.header->evaluation_count || m.y>=v.header->program_count || m.w>1u)return false;
 Evaluation e=v.evaluations[m.x];return e.program==m.y && e.file==m.z;
}
bool capture_spec_matches(PredicateSpec s,Assertion a,uint index) {
 return s.kind==a.kind && s.first==a.first && s.second==a.second && s.data_start==a.data_start && s.data_count==a.data_count && s.flags==a.flags && s.index==index && !s.reserved;
}
template<bool Query> void capture_arguments_validate(device const uchar* data,device const CaptureSpan* spans,device const ArgumentCapture* captures,device const uint4* meta,device const uint* owners,device const PredicateSpec* specs,device atomic_uint* status,constant CaptureParams& p,uint i) {
 if(atomic_load_explicit(status,memory_order_relaxed))return;View v=view(data);bool valid=p.layout<=1u && (Query || p.layout==0u);
 if(!valid){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 if(i<p.n){
  uint4 m=meta[i];valid=capture_metadata_valid(v,m);CaptureSpan span=argument_span(spans,i,p);
  valid=valid && span.capture_start<=p.c && span.capture_count<=p.c-span.capture_start;
  if(!i)valid=valid && !span.capture_start;
  if(i+1u<p.n)valid=valid && span.capture_start+span.capture_count==argument_span(spans,i+1u,p).capture_start;
  else valid=valid && span.capture_start+span.capture_count==p.c;
  if(valid){Program pg=v.programs[m.y];PredicateSpec s=argument_spec(v,meta,specs,i,p);
   if(Query){
    if((pg.flags&1u) && p.step<pg.capture_count){uint index=pg.capture_start+p.step;valid=capture_spec_matches(s,v.assertions[index],index);}
    else valid=s.kind==NONE && !s.first && !s.second && !s.data_start && !s.data_count && !s.flags && s.index==NONE && !s.reserved;
   }else if(s.index==NONE){
    valid=s.kind==1u && !s.first && s.second<pg.value_count && !s.data_start && !s.data_count && !s.flags && !s.reserved && span.capture_count==1u;
   }else{
    valid=!(pg.flags&1u) && s.index>=pg.capture_start && s.index-pg.capture_start<pg.capture_count;
    if(valid)valid=capture_spec_matches(s,v.assertions[s.index],s.index);
   }
  }
 }
 if(i<p.c && valid){
  uint owner=owners[i];valid=owner<p.n;
  if(valid){uint4 m=meta[owner];CaptureSpan span=argument_span(spans,owner,p);valid=capture_metadata_valid(v,m) && span.capture_start<=i && i-span.capture_start<span.capture_count;
   if(valid){ArgumentCapture cap=argument_capture(captures,i,p);File f=v.files[m.z];Program pg=v.programs[m.y];PredicateSpec s=argument_spec(v,meta,specs,owner,p);
    valid=cap.node<p.nodes && cap.node>=f.node_start && cap.node-f.node_start<f.node_count;
    if(valid){Node node=v.nodes[cap.node];valid=cap.start==node.source_start && cap.end==node.source_end && (cap.label<pg.label_count || (!Query && s.index==NONE && cap.label==0u));}
   }
  }
 }
 if(!p.n && p.c)valid=false;
 if(!valid)atomic_store_explicit(status,2u,memory_order_relaxed);
}
kernel void capture_query_arguments_validate(device const uchar* data [[buffer(0)]],device const CaptureSpan* spans [[buffer(1)]],device const ArgumentCapture* captures [[buffer(2)]],device const uint4* meta [[buffer(3)]],device const uint* owners [[buffer(4)]],device const PredicateSpec* specs [[buffer(5)]],device atomic_uint* status [[buffer(6)]],constant CaptureParams& p [[buffer(7)]],uint i [[thread_position_in_grid]]) {capture_arguments_validate<true>(data,spans,captures,meta,owners,specs,status,p,i);}
kernel void capture_inline_arguments_validate(device const uchar* data [[buffer(0)]],device const CaptureSpan* spans [[buffer(1)]],device const ArgumentCapture* captures [[buffer(2)]],device const uint4* meta [[buffer(3)]],device const uint* owners [[buffer(4)]],device const PredicateSpec* specs [[buffer(5)]],device atomic_uint* status [[buffer(6)]],constant CaptureParams& p [[buffer(7)]],uint i [[thread_position_in_grid]]) {capture_arguments_validate<false>(data,spans,captures,meta,owners,specs,status,p,i);}
