bool mime_equals(View v,uint start,uint length,constant char* expected,uint count) {
 if(length!=count)return false;
 for(uint k=0u;k<22u;k++)if(k<count && v.source[start+k]!=uchar(expected[k]))return false;
 return true;
}
uint mime_value_length(uint4 snapshot) {
 if(snapshot.z==0u)return snapshot.y;if(snapshot.z==1u)return 4u;if(snapshot.z==2u)return 10u;return NONE;
}
// Snapshots are checked before any byte publication.
uchar mime_byte(View v,uint4 snapshot,uint at) {
 if(at<8u)return uchar("language"[at]);at-=8u;
 if(snapshot.z==0u)return v.source[snapshot.x+at];
 if(snapshot.z==1u)return uchar("json"[at]);
 if(snapshot.z==2u)return uchar("javascript"[at]);
 return 0u;
}
kernel void output_mime_snapshot(device const uchar* data [[buffer(0)]],device Result* rows [[buffer(1)]],device const Capture* caps [[buffer(2)]],device uint4* meta [[buffer(3)]],device const uint2* selected [[buffer(4)]],device uint4* snapshots [[buffer(5)]],device uint4* work [[buffer(6)]],device atomic_uint* status [[buffer(7)]],device const uint* owners [[buffer(8)]],constant OutputParams& p [[buffer(9)]],uint i [[thread_position_in_grid]]) {
 if(i>=p.n)return;work[i]=uint4(0u);if(atomic_load_explicit(status,memory_order_relaxed) || !meta[i].w)return;
 View v=view(data);Program pg=v.programs[meta[i].y];if(p.step>=pg.output_count || v.output_words[pg.output_start+p.step*6u]!=6u)return;
 uint target=selected[i].x;if(target==NONE){meta[i].w=0u;return;}
 if(target>=p.c || owners[target]!=i || !caps[target].retained || caps[target].label!=v.output_words[pg.output_start+p.step*6u+1u]){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 uint start=caps[target].start,length=caps[target].end-start,mode=0u;
 if(mime_equals(v,start,length,"importmap",9u))mode=1u;
 else if(mime_equals(v,start,length,"module",6u) || mime_equals(v,start,length,"application/ecmascript",22u) || mime_equals(v,start,length,"text/ecmascript",15u))mode=2u;
 snapshots[i*p.max_ops+p.step]=uint4(start,length,mode,++rows[i].dynamic_count);
 if(mode==0u)work[i].x=length;
}
kernel void output_mime_arguments(device const uint4* total [[buffer(0)]],device uint* arguments [[buffer(1)]],device atomic_uint* status [[buffer(2)]],constant OutputParams& p [[buffer(3)]],uint i [[thread_position_in_grid]]) {
 (void)p;if(i)return;arguments[0]=0u;arguments[1]=arguments[2]=1u;if(atomic_load_explicit(status,memory_order_relaxed))return;
 if(total[0].x==NONE){atomic_store_explicit(status,1u,memory_order_relaxed);return;}
 arguments[0]=total[0].x/256u+uint(total[0].x%256u!=0u);
}
kernel void output_mime_slashes(device const uchar* data [[buffer(0)]],device const uint4* snapshots [[buffer(1)]],device const uint4* offsets [[buffer(2)]],device const uint4* total [[buffer(3)]],device atomic_uint* slashes [[buffer(4)]],device const uint* status [[buffer(5)]],constant OutputParams& p [[buffer(6)]],uint i [[thread_position_in_grid]]) {
 if(status[0] || i>=total[0].x)return;uint lo=0u,hi=p.n;
 while(lo<hi){uint mid=lo+(hi-lo)/2u;if(offsets[mid].x<=i)lo=mid+1u;else hi=mid;}
 uint owner=lo-1u,slot=owner*p.max_ops+p.step,at=i-offsets[owner].x;View v=view(data);
 if(v.source[snapshots[slot].x+at]=='/')atomic_fetch_max_explicit(slashes+slot,at+1u,memory_order_relaxed);
}
kernel void output_mime_finish(device uint4* snapshots [[buffer(0)]],device const uint* slashes [[buffer(1)]],device atomic_uint* status [[buffer(2)]],constant OutputParams& p [[buffer(3)]],uint i [[thread_position_in_grid]]) {
 if(i>=p.n || atomic_load_explicit(status,memory_order_relaxed))return;
 uint slot=i*p.max_ops+p.step;uint4 snapshot=snapshots[slot];if(!snapshot.w)return;
 if(snapshot.z>2u || (snapshot.z==0u && slashes[slot]>snapshot.y)){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 if(snapshot.z==0u){snapshot.x+=slashes[slot];snapshot.y-=slashes[slot];snapshots[slot]=snapshot;}
}
kernel void output_dynamic_sizes(device const uint4* snapshots [[buffer(0)]],device const uint4* meta [[buffer(1)]],device uint4* counts [[buffer(2)]],device atomic_uint* status [[buffer(3)]],device const uchar* data [[buffer(4)]],constant OutputParams& p [[buffer(5)]],uint i [[thread_position_in_grid]]) {
 if(i>=p.dynamic_slots)return;counts[i]=uint4(0u);
 if(atomic_load_explicit(status,memory_order_relaxed) || !meta[i/p.max_ops].w)return;
 View v=view(data);Program pg=v.programs[meta[i/p.max_ops].y];uint step=i%p.max_ops;
 bool required=step<pg.output_count && v.output_words[pg.output_start+step*6u]==6u;
 if(required!=(snapshots[i].w!=0u)){atomic_store_explicit(status,2u,memory_order_relaxed);return;}if(!required)return;
 uint length=mime_value_length(snapshots[i]);if(length==NONE || length>NONE-8u){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 counts[i]=uint4(1u,8u+length,0u,0u);
}
