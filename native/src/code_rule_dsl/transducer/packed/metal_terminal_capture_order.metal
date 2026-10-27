// Ordinary assertions consume emission order. Terminal containment consumes geometry order.
struct CaptureOrderParams {uint capacity,count,captures,width;};
uint capture_order_row(device const uint4* offsets,uint n,uint i){uint lo=0u,hi=n;while(lo<hi){uint mid=lo+(hi-lo)/2u;if(offsets[mid].x<=i)lo=mid+1u;else hi=mid;}return lo?lo-1u:NONE;}
kernel void terminal_capture_sizes(device const uint* states [[buffer(0)]],device const uint* header [[buffer(1)]],device uint4* sizes [[buffer(2)]],device atomic_uint* maximum [[buffer(3)]],device const uint* status [[buffer(4)]],constant CaptureOrderParams& p [[buffer(5)]],uint i [[thread_position_in_grid]]) {
 if(i>=p.capacity)return;sizes[i]=uint4(0u);if(status[0] || i>=header[0])return;uint count=states[11u*header[0]+i];sizes[i].x=count;atomic_fetch_max_explicit(maximum,count,memory_order_relaxed);
}
kernel void terminal_capture_project(device const uint* states [[buffer(0)]],device const uint* header [[buffer(1)]],device const uint4* captures [[buffer(2)]],device const uint4* offsets [[buffer(3)]],device uint* output [[buffer(4)]],device uint* output_header [[buffer(5)]],device uint4* output_captures [[buffer(6)]],device atomic_uint* status [[buffer(7)]],constant CaptureOrderParams& p [[buffer(8)]],uint i [[thread_position_in_grid]]) {
 if(atomic_load_explicit(status,memory_order_relaxed))return;
 if(!i){output_header[0]=p.count;output_header[1]=p.captures;output_header[2]=(p.count+255u)/256u;output_header[3]=output_header[4]=1u;}
 if(i<p.count){for(uint column=0u;column<12u;column++)output[column*p.count+i]=states[column*header[0]+i];output[10u*p.count+i]=offsets[i].x;}
 if(i<p.captures){uint row=capture_order_row(offsets,p.count,i);if(row>=header[0]){atomic_store_explicit(status,2u,memory_order_relaxed);return;}uint local=i-offsets[row].x;
  if(local>=states[11u*header[0]+row]){atomic_store_explicit(status,2u,memory_order_relaxed);return;}output_captures[i]=captures[states[10u*header[0]+row]+local];}
}
bool terminal_capture_less(uint4 a,uint4 b){return a.z<b.z || (a.z==b.z && (a.w>b.w || (a.w==b.w && a.y<b.y)));}
kernel void terminal_capture_merge(device const uchar* data [[buffer(0)]],device const uint* states [[buffer(1)]],device const uint4* offsets [[buffer(2)]],device const uint4* captures [[buffer(3)]],device uint4* output [[buffer(4)]],device const uint* status [[buffer(5)]],constant CaptureOrderParams& p [[buffer(6)]],uint i [[thread_position_in_grid]]) {
 if(i>=p.captures || status[0])return;uint row=capture_order_row(offsets,p.count,i);View v=view(data);Program pg=v.programs[states[4u*p.count+row]];
 if((pg.flags&1u) || !pg.capture_count || states[5u*p.count+row]!=3u){output[i]=captures[i];return;}
 uint start=states[10u*p.count+row],end_row=start+states[11u*p.count+row],base=start+((i-start)/p.width/2u)*p.width*2u;
 uint middle=uint(min(ulong(end_row),ulong(base)+p.width)),end=uint(min(ulong(end_row),ulong(base)+2ul*p.width));
 bool left=i<middle;uint begin=left?middle:base,lo=begin,hi=left?end:middle;uint4 value=captures[i];
 while(lo<hi){uint mid=lo+(hi-lo)/2u;uint4 other=captures[mid];bool before=terminal_capture_less(other,value) || (!left && !terminal_capture_less(value,other));if(before)lo=mid+1u;else hi=mid;}
 output[base+i-(left?base:middle)+lo-begin]=value;
}
