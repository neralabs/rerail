// The checked terminal projection owns task-ordered rows and contiguous raw
// ordinals. A maximal anchor can remove only rows that it exactly contains.
struct AnchorParams {uint capacity,captures,output;};
uint anchor_column(device const uint* s,uint n,uint f,uint i){return s[f*n+i];}
kernel void terminal_anchor_elect(device const uint* s [[buffer(0)]],
 device const uint* h [[buffer(1)]],device uint* anchors [[buffer(2)]],
 device atomic_uint* status [[buffer(3)]],constant AnchorParams& p [[buffer(4)]],uint i [[thread_position_in_grid]]) {
 uint n=h[0];
 if(n>p.capacity || h[1]>p.captures || h[2]!=uint((ulong(n)+255ul)/256ul) || h[3]!=1u || h[4]!=1u){if(!i)atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 if(i>=n || (i && s[i-1u]==s[i]))return;
 uint lo=i+1u,hi=n;
 while(lo<hi){uint mid=lo+(hi-lo)/2u;if(s[mid]<=s[i])lo=mid+1u;else hi=mid;}
 uint best=i;
 for(uint row=i+1u;row<lo;row++)if(anchor_column(s,n,9u,row)>anchor_column(s,n,9u,best))best=row;
 anchors[i]=best;
}
kernel void terminal_anchor_flags(device const uint* s [[buffer(0)]],
 device const uint* h [[buffer(1)]],device const uint4* captures [[buffer(2)]],
 device const uint* anchors [[buffer(3)]],device uint4* flags [[buffer(4)]],
 device atomic_uint* status [[buffer(5)]],constant AnchorParams& p [[buffer(6)]],uint i [[thread_position_in_grid]]) {
 if(i>=p.capacity)return;
 flags[i]=uint4(0u);
 uint n=h[0];if(n>p.capacity || h[1]>p.captures || i>=n)return;
 uint lo=0u,hi=i;
 while(lo<hi){uint mid=lo+(hi-lo)/2u;if(s[mid]<s[i])lo=mid+1u;else hi=mid;}
 uint first=lo,best=anchors[first],ordinal=anchor_column(s,n,10u,i),scope=anchor_column(s,n,1u,i);
 bool valid=(!i || s[i-1u]<=s[i]) && best>=first && best<n && s[best]==s[i] && ordinal==i-first;
 valid &= scope==0u || (ordinal!=0xffffffffu && scope==ordinal+1u);
 valid &= (scope==0u)==(anchor_column(s,n,1u,first)==0u);
 for(uint f=2u;f<8u;f++)valid &= anchor_column(s,n,f,i)==0u;
 uint start=anchor_column(s,n,8u,i),count=anchor_column(s,n,9u,i);
 uint other_start=valid?anchor_column(s,n,8u,best):0u,other_count=valid?anchor_column(s,n,9u,best):0u;
 valid &= ulong(start)+count<=h[1] && ulong(other_start)+other_count<=h[1];
 if(!valid){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 bool contained=scope==0u && best!=i;
 for(uint entry=0u;entry<count;entry++){
  uint4 value=captures[start+entry];
  valid &= value.x!=0xffffffffu && value.y<=65535u && value.z<=value.w;
  if(entry)valid &= !geometry_less(value,captures[start+entry-1u]);
  if(contained){
   uint rank=entry-geometry_begin(captures,start,count,value);
   uint target=geometry_begin(captures,other_start,other_count,value)+rank;
   if(target>=other_count)contained=false;
   else contained=all(captures[other_start+target]==value);
  }
 }
 if(!valid)atomic_store_explicit(status,2u,memory_order_relaxed);
 flags[i].x=uint(valid && !contained);
}
kernel void terminal_anchor_scatter(device const uint* s [[buffer(0)]],
 device const uint* h [[buffer(1)]],device const uint4* offsets [[buffer(2)]],
 device const uint4* total [[buffer(3)]],device uint* output [[buffer(4)]],
 device uint* counts [[buffer(5)]],device const uint* status [[buffer(6)]],
 constant AnchorParams& p [[buffer(7)]],uint i [[thread_position_in_grid]]) {
 uint n=total[0].x;bool valid=status[0]==0u && n<=p.output;
 if(!i){counts[0]=valid?n:0u;counts[1]=valid?h[1]:0u;counts[2]=valid?uint((ulong(n)+255ul)/256ul):0u;counts[3]=counts[4]=1u;}
 if(!valid || i>=h[0] || offsets[i].x==(i+1u<h[0]?offsets[i+1u].x:n))return;
 for(uint f=0u;f<11u;f++)output[f*n+offsets[i].x]=s[f*h[0]+i];
}
