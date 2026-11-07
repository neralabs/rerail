// Instantiate one bounded operation body for device and private GPU frames.
uint4 successor_frame(View v,Program pg,device const uint* s,uint n,device const uint4* cursors,device const uint4* repeats,device const uint* spans,device const uint4* sites,device const uint* path_work,constant SuccessorParams& p,uint i,uint variant,uint signature,device atomic_uint* status,STEP_ADDRESS Step& x) {
 uint4 layout=successor_layout(signature);
 x.repeat_offset=SUCCESSOR_HEADER_WORDS+2u*layout.x;
 x.capture_offset=(x.repeat_offset+layout.y+3u)&~3u;x.branch_offset=x.capture_offset+4u*layout.z;
 x.input_parent=i;x.pc=input_scalar(v,s,n,1u,i);x.node=input_scalar(v,s,n,2u,i);x.kind=input_scalar(v,s,n,5u,i);x.live=1u;x.error=NONE;x.cursor_keep=input_scalar(v,s,n,7u,i);x.repeat_keep=input_scalar(v,s,n,9u,i);x.query_order=(pg.flags&1u)?1u:(pg.capture_count?2u:0u);x.path_work=input_work(path_work,i);
 uint cs=input_scalar(v,s,n,6u,i),rs=input_scalar(v,s,n,8u,i);
 if(signature<p.signatures) {
  if(x.kind==4u || x.kind==5u) {x.live=uint((v.nodes[x.node].flags&1u)!=0u);x.kind=x.kind==4u?0u:2u;}
  if(x.live)switch(signature) {
   // SUCCESSOR_CASES
  }
 }
 if(x.boundary>1u){atomic_store_explicit(status,2u,memory_order_relaxed);return uint4(0u);}
 if(x.error!=NONE) {x.pc=x.error;x.kind=frame_query(x.kind)?FRAME_QUERY_OPERAND_FAILURE:FRAME_ORDINARY_OPERAND_FAILURE;x.error=NONE;x.live=1u;x.expansion=0u;}
 if(x.boundary){x.kind=frame_query(x.kind)?FRAME_QUERY_PREDICATE:FRAME_ORDINARY_PREDICATE;x.live=1u;x.expansion=0u;}
 // Ordinary candidates without pending offers preserve every occurrence.
 // Their comparison domain is empty; the completed producer owns this proof.
 // Bounded insertion sort concerns only the finite region's new captures.
 if(x.query_order!=2u)for(uint a=1u;a<x.capture_add;a++) {uint4 value=step_captures(x)[a];uint b=a;while(b>0u && capture_edit_less(value,step_captures(x)[b-1u],x.query_order)) {step_captures(x)[b]=step_captures(x)[b-1u];b--;}step_captures(x)[b]=value;}
 uint count=x.live?(x.expansion==0u?1u:(x.expansion<=2u?2u:x.end-x.begin)):0u;
 // A GREEDY fork contributes one query candidate and one pending offer.
 // Named expansion can overcount losing candidates but never undercounts keys.
 uint keys=frame_query(x.kind) && !frame_pending(x.kind)?(x.expansion==2u?uint(x.live):count):0u;
 uint needs=uint(keys!=0u)|uint(x.live && (frame_pending(x.kind) || x.expansion==2u))*2u;
 if(needs)atomic_fetch_or_explicit(status+3u,needs,memory_order_relaxed);
 uint digits=input_control(spans,n,1u,i)+x.branch_count+uint(x.expansion!=0u);
 if(x.expansion==2u)digits=3u*input_control(spans,n,1u,i)+2u;
 else if(frame_pending(x.kind))digits+=input_control(spans,n,3u,i)+input_control(spans,n,5u,i);
 uint width=max(max(x.cursor_keep+x.cursor_add,x.repeat_keep+x.repeat_add),max(input_scalar(v,s,n,11u,i)+x.capture_add,digits));
 return uint4(count,uint(min(ulong(count)*width,0xfffffffful)),successor_step_words(layout),keys);
}
