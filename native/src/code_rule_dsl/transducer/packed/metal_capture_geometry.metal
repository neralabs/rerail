// Captures are ordered by byte start and reverse byte end. Equal geometry retains
// capture order. A multiset comparison would lose the oracle's ordered tie semantics.
bool geometry_less(uint4 a,uint4 b) { return a.z<b.z || (a.z==b.z && a.w>b.w); }
uint geometry_begin(device const uint4* captures,uint start,uint count,uint4 value) {
 uint low=0u,high=count;
 while(low<high) { uint mid=low+(high-low)/2u; if(geometry_less(captures[start+mid],value)) low=mid+1u; else high=mid; }
 return low;
}
