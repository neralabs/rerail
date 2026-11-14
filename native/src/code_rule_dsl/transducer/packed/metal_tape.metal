// A resident tape read in place (PIAT §20.5; decision 0032): a batch binds up to four slabs and a
// table of its documents (`DocView`, `residency::slabs`); each document's v9np columns, parents,
// codes, source and line starts are read from its slab. Node indices here are document-local; a
// batch's numbering is `doc.nodeBase + local`. NONE is u32's max.
// kidStart, kids: each node's first child in its document's child lists, and the lists (the
// interpreter's children by index); childBase, sourceBase, lineBase: the document's place in the
// batch's numbering of children, source bytes and lines.
// codeStart, byCode: each code's first entry in byCode, and the nodes of every code in node order
// (a pass's roots: `RootSeg`).
struct DocView{uint slab,nodes,nodeBase,widths,code,size,base,off,tail,parent,codes,source,sourceLen,lines,lineCount,codeCount,kidStart,kids,kidCount,childBase,sourceBase,lineBase,codeStart,byCode;};
static_assert(sizeof(DocView)==96u,"document view layout");
// `tape::resident::DeviceCode`: flags are named, extra<<1, missing<<2, error<<3.
struct DeviceCode{ushort kind,field;uchar flags,classCount;ushort reserved;ushort classes[8];};
static_assert(sizeof(DeviceCode)==24u,"device code layout");
constant uint TAPE_NONE=0xffffffffu;
struct Slabs{device const uchar* s0;device const uchar* s1;device const uchar* s2;device const uchar* s3;};
inline device const uchar* tape_slab(thread const Slabs& slabs,uint slab){return slab==0u?slabs.s0:slab==1u?slabs.s1:slab==2u?slabs.s2:slabs.s3;}
inline uint tape_cell(device const uchar* column,uint width,uint i){return width==2u?uint(((device const ushort*)column)[i]):((device const uint*)column)[i];}
inline uint tape_size(thread const Slabs& s,DocView d,uint i){return tape_cell(tape_slab(s,d.slab)+d.size,d.widths&15u,i);}
inline device const DeviceCode& tape_code(thread const Slabs& s,DocView d,uint i){device const uchar* slab=tape_slab(s,d.slab);return ((device const DeviceCode*)(slab+d.codes))[((device const ushort*)(slab+d.code))[i]];}
inline uint tape_kind(thread const Slabs& s,DocView d,uint i){return tape_code(s,d,i).kind;}
inline uint tape_field(thread const Slabs& s,DocView d,uint i){return tape_code(s,d,i).field;}
inline uint tape_flags(thread const Slabs& s,DocView d,uint i){return tape_code(s,d,i).flags;}
inline bool tape_named(thread const Slabs& s,DocView d,uint i){return (tape_flags(s,d,i)&1u)!=0u;}
inline bool tape_has_class(thread const Slabs& s,DocView d,uint i,uint kind){device const DeviceCode& c=tape_code(s,d,i);for(uint k=0u;k<c.classCount;k++){if(c.classes[k]==kind)return true;if(c.classes[k]>kind)return false;}return false;}
inline uint tape_start(thread const Slabs& s,DocView d,uint i){device const uchar* slab=tape_slab(s,d.slab);return ((device const uint*)(slab+d.base))[i>>4]+tape_cell(slab+d.off,(d.widths>>4)&15u,i);}
inline uint tape_end(thread const Slabs& s,DocView d,uint i){device const uchar* slab=tape_slab(s,d.slab);uint from=(d.widths&4096u)!=0u?tape_start(s,d,i):tape_start(s,d,i+tape_size(s,d,i)-1u);return from+tape_cell(slab+d.tail,(d.widths>>8)&15u,i);}
inline uint tape_parent(thread const Slabs& s,DocView d,uint i){return ((device const uint*)(tape_slab(s,d.slab)+d.parent))[i];}
// The node after `i`'s subtree, while it is inside `i`'s parent.
inline uint tape_next(thread const Slabs& s,DocView d,uint i){uint p=tape_parent(s,d,i);if(p==TAPE_NONE)return TAPE_NONE;uint j=i+tape_size(s,d,i);return j<p+tape_size(s,d,p)?j:TAPE_NONE;}
inline uint tape_next_named(thread const Slabs& s,DocView d,uint i){uint j=tape_next(s,d,i);while(j!=TAPE_NONE&&!tape_named(s,d,j))j=tape_next(s,d,j);return j;}
inline uint tape_first_child(thread const Slabs& s,DocView d,uint i){return tape_size(s,d,i)>1u?i+1u:TAPE_NONE;}
inline uint tape_child_count(thread const Slabs& s,DocView d,uint i){uint n=0u;for(uint c=tape_first_child(s,d,i);c!=TAPE_NONE;c=tape_next(s,d,c))n++;return n;}
// The `k`-th child (from 0), by sibling hops.
inline uint tape_child(thread const Slabs& s,DocView d,uint i,uint k){uint c=tape_first_child(s,d,i);while(c!=TAPE_NONE&&k>0u){c=tape_next(s,d,c);k--;}return c;}
inline uchar tape_byte(thread const Slabs& s,DocView d,uint at){return (tape_slab(s,d.slab)+d.source)[at];}
inline device const uchar* tape_source(thread const Slabs& s,DocView d){return tape_slab(s,d.slab)+d.source;}
inline uint tape_line_start(thread const Slabs& s,DocView d,uint line){return ((device const uint*)(tape_slab(s,d.slab)+d.lines))[line];}
// A pass's roots (`label_passes::Roots`, decision 0033): at `at` in `segs`, a header (x: the
// segment count, y: the roots) and its segments, each a document's run of roots: x the document,
// y where they start (in its nodes by code, or, bit 31 set, in `segs` as listed local nodes), z
// how many, w the first root's index in the pass. Segments ascend by document, then index.
struct RootSeg{uint doc,from,count,first;};
constant uint ROOT_LISTED=0x80000000u;
// The `i`-th root's segment, by binary search over the pass's segments.
inline uint root_segment(device const uint4* segs,uint at,uint i){uint low=at+1u,high=at+1u+segs[at].x;while(high-low>1u){uint middle=(low+high)/2u;if(segs[middle].w<=i)low=middle;else high=middle;}return low;}
// The node `k` of a segment (document-local).
inline uint root_local(thread const Slabs& s,DocView d,device const uint4* segs,uint4 seg,uint k){
  if((seg.y&ROOT_LISTED)!=0u)return ((device const uint*)segs)[(seg.y&~ROOT_LISTED)+k];
  return ((device const uint*)(tape_slab(s,d.slab)+d.byCode))[seg.y+k];
}
// The first segment at or after document `doc` in the pass at `at`.
inline uint root_doc_segment(device const uint4* segs,uint at,uint doc){uint low=at+1u,high=at+1u+segs[at].x;while(low<high){uint middle=(low+high)/2u;if(segs[middle].x<doc)low=middle+1u;else high=middle;}return low;}

