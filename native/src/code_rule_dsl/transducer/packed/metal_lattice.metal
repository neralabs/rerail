// A flat lattice's fixpoint (`logic::native::lattice_fixpoint`'s graph, list step 17) on the
// device work queue (`metal_work_queue.metal`, prepended): an item is a cell to evaluate. A
// worker takes a cell, evaluates it from its inputs' current values, and where its value rose,
// pushes its dependents, which any idle worker takes up. The system is monotone over a finite
// lattice, so this chaotic order reaches the one least fixpoint.
//
// A cell's state only rises: 0 BOT, 1 writing, 2 a constant (in `values`), 3 TOP. A constant is
// published as 0 -> 1, the value written and fenced, 1 -> 2; a reader that sees 1 reads BOT
// (the writer pushes the cell's dependents once it is published), and a raise that finds 1
// pushes its cell again rather than wait. So no value falls and none is read half written.

constant long TOP=4611686018427387904L;
constant long BOT=(-9223372036854775807L-1L);
constant long BIG=1099511627776L;

struct Args { uint cells,capacity,polls,pad; };
struct Bin { uint op,left,right,pad; };

static long lub(long a,long b){ if(a==BOT)return b; if(b==BOT)return a; return a==b?a:TOP; }
static long transfer(uint op,long a,long b){
  if(a==TOP||b==TOP)return TOP;
  if(op==43u)return a+b;
  if(op==45u)return a-b;
  if(op==42u&&abs(a)<BIG&&abs(b)<BIG)return a*b;
  return TOP;
}
static void fence(){atomic_thread_fence(mem_flags::mem_device,memory_order_seq_cst,thread_scope_device);}

static long read_cell(device atomic_uint* state,coherent(device) device long* values,uint cell){
  uint s=atomic_load_explicit(&state[cell],memory_order_relaxed);
  if(s==3u)return TOP;
  if(s!=2u)return BOT;
  fence();
  return values[cell];
}

// 0: unchanged; 1: risen; 2: another worker is publishing it, evaluate again later.
static uint raise(device atomic_uint* state,coherent(device) device long* values,uint cell,long value){
  if(value==BOT)return 0u;
  if(value==TOP)return atomic_fetch_max_explicit(&state[cell],3u,memory_order_relaxed)==3u?0u:1u;
  uint seen=0u;
  bool claimed=false;
  for(uint attempt=0u;attempt<WQ_TURNS;attempt++){
    claimed=atomic_compare_exchange_weak_explicit(&state[cell],&seen,1u,memory_order_relaxed,memory_order_relaxed);
    if(claimed||seen!=0u)break;
  }
  if(!claimed&&seen==0u)return 2u;
  if(claimed){
    values[cell]=value;
    fence();
    uint writing=1u;
    // Fails only where TOP was raised meanwhile, which stands.
    for(uint attempt=0u;attempt<WQ_TURNS;attempt++){
      if(atomic_compare_exchange_weak_explicit(&state[cell],&writing,2u,memory_order_relaxed,memory_order_relaxed)||writing!=1u)break;
    }
    return 1u;
  }
  if(seen==3u)return 0u;
  if(seen==1u)return 2u;
  fence();
  if(values[cell]==value)return 0u;
  return atomic_fetch_max_explicit(&state[cell],3u,memory_order_relaxed)==3u?0u:1u;
}

// Queues `cell` unless it is queued already.
static void enqueue(WorkQueue q,device atomic_uint* queued,uint cell){
  uint bit=1u<<(cell%32u);
  if((atomic_fetch_or_explicit(&queued[cell/32u],bit,memory_order_relaxed)&bit)==0u)wq_push(q,cell);
}

kernel void lattice_queue(device atomic_uint* state [[buffer(0)]],coherent(device) device long* values [[buffer(1)]],
    device const long* seeds [[buffer(2)]],
    device const uint* copyStart [[buffer(3)]],device const uint* copies [[buffer(4)]],
    device const uint* binStart [[buffer(5)]],device const uint* binIndex [[buffer(6)]],device const Bin* bins [[buffer(7)]],
    device const uint* outStart [[buffer(8)]],device const uint* outs [[buffer(9)]],
    device atomic_uint* queued [[buffer(10)]],device atomic_uint* slots [[buffer(11)]],device atomic_uint* control [[buffer(12)]],
    device atomic_uint* stats [[buffer(13)]],constant Args& args [[buffer(14)]]){
  WorkQueue q={slots,control,args.capacity};
  uint polls=0u,visits=0u,retries=0u,cell=0u;
  for(uint turn=0u;turn<WQ_TURNS;turn++){
    if(!wq_take(q,cell)){
      if(wq_idle(q,polls,args.polls))break;
      continue;
    }
    polls=0u;
    atomic_fetch_and_explicit(&queued[cell/32u],~(1u<<(cell%32u)),memory_order_relaxed);
    fence();
    visits++;
    long value=seeds[cell];
    for(uint e=copyStart[cell];e<copyStart[cell+1u];e++)value=lub(value,read_cell(state,values,copies[e]));
    for(uint e=binStart[cell];e<binStart[cell+1u];e++){
      Bin bin=bins[binIndex[e]];
      long a=read_cell(state,values,bin.left),b=read_cell(state,values,bin.right);
      if(a!=BOT&&b!=BOT)value=lub(value,transfer(bin.op,a,b));
    }
    uint risen=raise(state,values,cell,value);
    if(risen==1u){
      fence();
      for(uint e=outStart[cell];e<outStart[cell+1u];e++)enqueue(q,queued,outs[e]);
    }else if(risen==2u){
      retries++;
      enqueue(q,queued,cell);
    }
    wq_done(q);
  }
  atomic_fetch_add_explicit(&stats[0],visits,memory_order_relaxed);
  atomic_fetch_add_explicit(&stats[1],retries,memory_order_relaxed);
}
