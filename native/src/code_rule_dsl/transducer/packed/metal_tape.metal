// A resident tape read in place (PIAT §20.5; decisions 0032, 0293): a batch binds up to four slabs and a
// table of its documents (`DocView`, `residency::slabs`); each document's tape, codes, source and line
// starts are read from its slab. The tape is read where it lies, format 15's columns or format 16's
// arrays (a 16z tape is laid out as format 16 where it is placed): nothing per node is derived beside
// it. A parent is a search (format 15: back over at most 64 nodes, else a descent from the root;
// format 16: back over the siblings' slots), a child or sibling a hop over a subtree. Node indices
// here are document-local; a batch's numbering is `doc.nodeBase + local`. NONE is u32's max.
// sourceBase, lineBase: the document's place in the batch's numbering of source bytes and lines.
// Format 15: widths are `size`'s, `off`'s and `tail`'s at bits 0, 4, 8, bit 12 tails from the node's
// own start; col: code, size, base, off, tail. Format 16: widths are the slots', gaps' and lengths'
// at bits 0, 4, 8, the checkpoints' at 12, the side lists' values' at 16, 20, 24, 28; col: codes,
// slots, gaps, lengths, checkpoints; side, parts: each side list and its part starts.
struct DocView{uint slab,nodes,nodeBase,format,widths;uint col[5];uint side[4],parts[4];uint cut,per,codes,source,sourceLen,lines,lineCount,codeCount,sourceBase,lineBase,tape;};
static_assert(sizeof(DocView)==116u,"document view layout");
// `tape::resident::DeviceCode`: flags are named, extra<<1, missing<<2, error<<3.
struct DeviceCode{ushort kind,field;uchar flags,classCount;ushort reserved;ushort classes[8];};
static_assert(sizeof(DeviceCode)==24u,"device code layout");
constant uint TAPE_NONE=0xffffffffu;
struct Slabs{device const uchar* s0;device const uchar* s1;device const uchar* s2;device const uchar* s3;};
inline device const uchar* tape_slab(thread const Slabs& slabs,uint slab){return slab==0u?slabs.s0:slab==1u?slabs.s1:slab==2u?slabs.s2:slabs.s3;}

// ---- format 15's columns
inline uint tape_cell(device const uchar* column,uint width,uint i){return width==2u?uint(((device const ushort*)column)[i]):((device const uint*)column)[i];}

// ---- format 16's arrays: the reads `tape_format::f16::View` makes (K = 8, BLOCK = 256, PART = 1024;
// a change there is a change here). A value too wide for its array is all ones, and is read from its
// side list: the node's part, its 256-node block's offsets, a search for its low byte.
constant uint F16_K=8u,F16_BLOCK=256u,F16_PART=1024u,F16_SUB=4u;
inline ulong f16_word(device const uchar* a,uint at){ulong v=0ul;for(uint b=0u;b<8u;b++)v|=ulong(a[at+b])<<(8u*b);return v;}
inline ulong f16_mask(uint w){return w>=8u?~0ul:((1ul<<(8u*w))-1ul);}
// Width `k`: 0 slots, 1 gaps, 2 lengths, 3 checkpoints, 4 + j side list j's values.
inline uint f16_width(DocView d,uint k){return (d.widths>>(4u*k))&15u;}
inline uint f16_u16(device const uchar* b,uint at){return uint(b[at])|uint(b[at+1u])<<8u;}
inline ulong f16_side(device const uchar* slab,DocView d,uint k,uint i){
  device const uchar* b=slab+d.side[k];uint p=i/F16_PART,at=((device const uint*)(slab+d.parts[k]))[p];
  uint blocks=(d.nodes+F16_BLOCK-1u)/F16_BLOCK,nsub=min(F16_SUB,blocks-p*F16_SUB),j=(i/F16_BLOCK)%F16_SUB;
  uint lo=j==0u?0u:f16_u16(b,at+2u*(j-1u)),hi=f16_u16(b,at+2u*j),offs=at+2u*nsub;uchar low=uchar(i%F16_BLOCK);
  while(lo<hi){uint m=(lo+hi)/2u;if(b[offs+m]<low)lo=m+1u;else hi=m;}
  uint vw=f16_width(d,4u+k);
  return f16_word(b,offs+f16_u16(b,at+2u*(nsub-1u))+lo*vw)&f16_mask(vw);
}
// Value `i` of array `a` (0 slots, 1 gaps, 2 lengths).
inline ulong f16_get(device const uchar* slab,DocView d,uint a,uint i){
  uint w=f16_width(d,a);device const uchar* arr=slab+d.col[1u+a];
  ulong v=w==1u?ulong(arr[i]):(f16_word(arr,i*w)&f16_mask(w));
  return v!=f16_mask(w)?v:f16_side(slab,d,a+1u,i);
}

// ---- either format
inline uint tape_size(thread const Slabs& s,DocView d,uint i){
  device const uchar* slab=tape_slab(s,d.slab);
  if(d.format==16u){ulong v=f16_get(slab,d,0u,i);return (v&1ul)==0ul?uint(v>>1):1u;}
  return max(tape_cell(slab+d.col[1],d.widths&15u,i),1u);
}
// The node's code's number in its document's (format 16: its language's) table.
inline uint tape_number(device const uchar* slab,DocView d,uint i){
  if(d.format==16u){uint b=slab[d.col[0]+i];return b<d.cut?b:d.cut+(b-d.cut)*d.per+uint(f16_side(slab,d,0u,i));}
  return uint(((device const ushort*)(slab+d.col[0]))[i]);
}
inline device const DeviceCode& tape_code(thread const Slabs& s,DocView d,uint i){device const uchar* slab=tape_slab(s,d.slab);return ((device const DeviceCode*)(slab+d.codes))[tape_number(slab,d,i)];}
inline uint tape_kind(thread const Slabs& s,DocView d,uint i){return tape_code(s,d,i).kind;}
inline uint tape_field(thread const Slabs& s,DocView d,uint i){return tape_code(s,d,i).field;}
inline uint tape_flags(thread const Slabs& s,DocView d,uint i){return tape_code(s,d,i).flags;}
inline bool tape_named(thread const Slabs& s,DocView d,uint i){return (tape_flags(s,d,i)&1u)!=0u;}
inline bool tape_has_class(thread const Slabs& s,DocView d,uint i,uint kind){device const DeviceCode& c=tape_code(s,d,i);for(uint k=0u;k<c.classCount;k++){if(c.classes[k]==kind)return true;if(c.classes[k]>kind)return false;}return false;}
inline uint tape_start(thread const Slabs& s,DocView d,uint i){
  device const uchar* slab=tape_slab(s,d.slab);
  if(d.format==16u){uint c=i/F16_K,cw=f16_width(d,3u);ulong at=f16_word(slab+d.col[4],c*cw)&f16_mask(cw);for(uint j=c*F16_K+1u;j<=i;j++)at+=f16_get(slab,d,1u,j);return uint(at);}
  return ((device const uint*)(slab+d.col[2]))[i>>4]+tape_cell(slab+d.col[3],(d.widths>>4)&15u,i);
}
inline uint tape_end(thread const Slabs& s,DocView d,uint i){
  device const uchar* slab=tape_slab(s,d.slab);
  if(d.format==16u)return tape_start(s,d,i)+uint(f16_get(slab,d,2u,i));
  uint from=(d.widths&4096u)!=0u?tape_start(s,d,i):tape_start(s,d,i+tape_size(s,d,i)-1u);return from+tape_cell(slab+d.col[4],(d.widths>>8)&15u,i);
}
// The parent. Format 16: the node before is the parent (an inner node), or a leaf naming the previous
// sibling or, every K-th sibling, the parent. Format 15: the nearest node before whose subtree holds
// `i`, back over at most 64 nodes, else by a descent from the root over the siblings' subtrees.
inline uint tape_parent(thread const Slabs& s,DocView d,uint i){
  if(i==0u)return TAPE_NONE;
  if(d.format==16u){
    device const uchar* slab=tape_slab(s,d.slab);uint j=i;
    while(j>0u){ulong v=f16_get(slab,d,0u,j-1u);if((v&1ul)==0ul)return j-1u;if((v&2ul)!=0ul)return j-uint(v>>2);j-=uint(v>>2);}
    return TAPE_NONE;
  }
  for(uint j=i,k=0u;j>0u&&k<64u;j--,k++)if(j-1u+tape_size(s,d,j-1u)>i)return j-1u;
  uint at=0u;
  while(true){uint c=at+1u;while(c+tape_size(s,d,c)<=i)c+=tape_size(s,d,c);if(c==i)return at;at=c;}
}
// The node after `i`'s subtree while it is before `end` (its parent's subtree's end).
inline uint tape_next_in(thread const Slabs& s,DocView d,uint i,uint end){uint j=i+tape_size(s,d,i);return j<end?j:TAPE_NONE;}
inline uint tape_end_of(thread const Slabs& s,DocView d,uint i){return i+tape_size(s,d,i);}
// The node after `i`'s subtree, while it is inside `i`'s parent.
inline uint tape_next(thread const Slabs& s,DocView d,uint i){uint p=tape_parent(s,d,i);if(p==TAPE_NONE)return TAPE_NONE;return tape_next_in(s,d,i,tape_end_of(s,d,p));}
inline uint tape_next_named_in(thread const Slabs& s,DocView d,uint i,uint end){uint j=tape_next_in(s,d,i,end);while(j!=TAPE_NONE&&!tape_named(s,d,j))j=tape_next_in(s,d,j,end);return j;}
inline uint tape_next_named(thread const Slabs& s,DocView d,uint i){uint p=tape_parent(s,d,i);if(p==TAPE_NONE)return TAPE_NONE;return tape_next_named_in(s,d,i,tape_end_of(s,d,p));}
inline uint tape_first_child(thread const Slabs& s,DocView d,uint i){return tape_size(s,d,i)>1u?i+1u:TAPE_NONE;}
inline uint tape_child_count(thread const Slabs& s,DocView d,uint i){uint n=0u,end=tape_end_of(s,d,i);for(uint c=i+1u;c<end;c+=tape_size(s,d,c))n++;return n;}
// The `k`-th child (from 0), by hops over its elder siblings' subtrees.
inline uint tape_child(thread const Slabs& s,DocView d,uint i,uint k){uint end=tape_end_of(s,d,i);for(uint c=i+1u;c<end;c+=tape_size(s,d,c)){if(k==0u)return c;k--;}return TAPE_NONE;}
inline uchar tape_byte(thread const Slabs& s,DocView d,uint at){return (tape_slab(s,d.slab)+d.source)[at];}
inline device const uchar* tape_source(thread const Slabs& s,DocView d){return tape_slab(s,d.slab)+d.source;}
inline uint tape_line_start(thread const Slabs& s,DocView d,uint line){return ((device const uint*)(tape_slab(s,d.slab)+d.lines))[line];}
// A pass's roots (`label_passes::Roots`; decisions 0033, 0293 §10), with no list of them. In
// `roots`: word 0 the batch's file count F, words 1..=F+1 each file's first group of 64 nodes
// (groups start at each document's first node) and the total. A rule at word `at`: its kind (0
// over codes, 1 listed); over codes, a bit per file it runs in (ceil(F/32) words), then its test
// (`code_admits`); listed, its nodes as a row of bits over the batch's nodes. A thread takes a
// pass and a group, finds the group's roots as a 64-bit mask, and walks from each.
constant uint ROOT_GROUP=64u;
inline uint roots_groups(device const uint* roots){return roots[1u+roots[0]];}
inline uint roots_first_group(device const uint* roots,uint file){return roots[1u+file];}
inline uint roots_file_groups(device const uint* roots,uint file){return roots[2u+file]-roots[1u+file];}
// The file holding the batch's group `group`.
inline uint roots_file(device const uint* roots,uint group){uint low=0u,high=roots[0];while(high-low>1u){uint middle=(low+high)/2u;if(roots[1u+middle]<=group)low=middle;else high=middle;}return low;}
// Whether a code passes a rule over codes: {kind count or NONE, kinds, test count, tests (op << 16 | argument)}.
inline bool code_admits(device const uint* rule,device const DeviceCode& code){
 uint kinds=rule[0],at=1u;
 if(kinds!=0xffffffffu){bool found=false;for(uint k=0u;k<kinds;k++)if(rule[1u+k]==code.kind){found=true;break;}if(!found)return false;at+=kinds;}
 for(uint k=0u,tests=rule[at];k<tests;k++){
  uint test=rule[at+1u+k],op=test>>16u,arg=test&0xffffu;
  if(op==1u){if(code.kind!=arg)return false;}
  else{bool found=false;for(uint m=0u;m<code.classCount;m++)if(code.classes[m]==arg){found=true;break;}if(!found)return false;}
 }
 return true;
}
// The roots of the rule at `at` among document `d`'s nodes 64*group.. (file `file` of the batch), as bits.
inline ulong root_mask(thread const Slabs& s,DocView d,device const uint* roots,uint at,uint file,uint group){
 uint from=group*ROOT_GROUP;if(from>=d.nodes)return 0ul;
 uint count=min(ROOT_GROUP,d.nodes-from);
 ulong mask=0ul;
 if(roots[at]!=0u){
  device const uint* row=roots+at+1u;uint b=d.nodeBase+from,w=b/32u,sh=b%32u;
  mask=ulong(row[w])|(ulong(row[w+1u])<<32u);
  if(sh!=0u)mask=(mask>>sh)|(ulong(row[w+2u])<<(64u-sh));
 }else{
  uint files=roots[0];
  if(((roots[at+1u+file/32u]>>(file%32u))&1u)==0u)return 0ul;
  device const uint* rule=roots+at+1u+(files+31u)/32u;
  for(uint k=0u;k<count;k++)if(code_admits(rule,tape_code(s,d,from+k)))mask|=1ul<<k;
 }
 return count==ROOT_GROUP?mask:(mask&((1ul<<count)-1ul));
}
// The lowest set bit of a mask.
inline uint root_ctz(ulong m){uint low=uint(m);return low!=0u?ctz(low):32u+ctz(uint(m>>32u));}
// Each root of a mask, document-local: `ROOTS_EACH(mask, group, local) { ... }`. One loop over the
// mask (a fixed two-trip loop over its halves was unrolled, the body compiled twice).
#define ROOTS_EACH(MASK,GROUP,LOCAL) for(ulong rootBits=(MASK);rootBits!=0ul;rootBits&=rootBits-1ul)for(uint LOCAL=(GROUP)*ROOT_GROUP+root_ctz(rootBits),rootOnce=0u;rootOnce==0u;rootOnce=1u)
