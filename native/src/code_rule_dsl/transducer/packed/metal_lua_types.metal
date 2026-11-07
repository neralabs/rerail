// Private continuation values retain one checked meaning across shared jobs.
// Byte offsets are relative to this request's source span.
struct LuaControl {
 uint request_id,item_count,table_offset,frame_words,capture_slots;
 uint depth,attempt,actions,initial_actions,action;
 uint child_at,push,pop,reset,source_at,chunk,reference_at;
};
struct LuaStackHeader {
 uint magic,generation,depth,attempt,actions,value,budget_owner,failure_owner;
};
// Phases: ENTER, ATOM_SCAN, ATOM_CHOICE, WAIT_ATOM, WAIT_SINGLE,
// BALANCED_SCAN, BACKREF_SCAN, RETURN_FALSE.
struct LuaFrame {
 uint at,phase,minimum,maximum,choice,scan,balance_depth,reference_start;
};
// Atom repetitions stay in immutable code. Only backreferences use reference_start.
// Tags: NONE, OPEN, RANGE, POSITION. Every position uses source byte offsets.
struct LuaCapture {uint tag,start,end;};
static_assert(sizeof(LuaStackHeader)==8u*sizeof(uint),"Lua header word contract");
static_assert(sizeof(LuaFrame)==8u*sizeof(uint),"Lua frame word contract");
static_assert(sizeof(LuaCapture)==3u*sizeof(uint),"Lua capture word contract");
constant uint LUA_ARENA_MAGIC=0x4c554132u;
device LuaFrame* lua_frame(device uint* arena,uint base,uint depth,uint stride){
 return (device LuaFrame*)(arena+base+8u+depth*stride);
}
device LuaCapture* lua_captures(device LuaFrame* frame){
 return (device LuaCapture*)((device uint*)frame+8u);
}
device const LuaCapture* lua_captures(device const LuaFrame* frame){
 return (device const LuaCapture*)((device const uint*)frame+8u);
}
