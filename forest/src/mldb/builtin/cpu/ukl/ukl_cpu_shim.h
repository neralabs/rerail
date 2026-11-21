/** ukl_cpu_shim.h                                     -*- C++ -*-
    Copyright (c) Jeremy Barnes.  All rights reserved.
    Used here under the Apache License, Version 2.0: MLDB's license is https://github.com/mldbai/mldb/blob/e3caed6fa44782d08eb640702fd60e9a2576d939/LICENSE
    This file is part of MLDB.

    UKL's shim for the CPU: a UKL kernel as C++ for the host's compiler, run a threadgroup at a time with each of
    its threads a stackful coroutine (Rerail, decision 0440 §3.5; upstream's CPU shim is block/ukl/ukl_cpu_shim.h,
    stackless coroutines over MLDB's C++ runtime, with every function holding a barrier marked SYNC_*).

    - **Every thread of a threadgroup is a coroutine with its own stack** (the host's, one a thread, with a guard
      page). A barrier, and every SIMD-group operation, suspends the thread; the group's scheduler runs each thread
      until it suspends or returns, then releases the waiters whose condition holds: a SIMD group's live lanes all at
      a SIMD-group operation, or every live thread at a threadgroup barrier. Calls need no marking: a barrier is
      reached in any function, as on a GPU.
    - **A SIMD group is 32 lanes**, as on Metal and CUDA (the library assumes it): an operation's values go through
      one slot a thread, between two suspensions.
    - **A thread that has returned no longer counts** at a barrier or in its SIMD group, as on a GPU (`if (i >= n)
      return;` before a barrier works).
    - **OpenCL's rule, checked** (`UKL_CPU_OPENCL_RULE`, `RERAIL_CPU_OPENCL_RULE=1`): a SIMD-group operation waits
      for every live thread of the threadgroup, as OpenCL's emulated SIMD groups do (decision 0440 §3.5). A kernel
      whose SIMD groups diverge at an operation fails here, naming its waits, where it would hang on such a device.
      `RERAIL_CPU_SIMD_WIDTH` runs SIMD groups of another width (a power of two up to 32).
    - **A barrier no thread can pass is an error here**: a SIMD group's lanes split between a SIMD-group operation
      and a threadgroup barrier (or another operation's wait) is reported (which thread waits where), and the
      launch fails, where a GPU would hang or give garbage. That is what the CPU backend is for: a kernel's bugs,
      found where a debugger reaches.
    - **No system headers**: the host passes the stacks, the threadgroup memory and the error buffer
      (`__ukl_cpu_launch`); the shim needs only the compiler's builtins.
    - **Arguments without reflection**, as on CUDA: beside each kernel, its rows as written (`Name__ukl_params`),
      their types' sizes, alignments and kinds (`Name__ukl_rows`), and each `ARGS` structure's fields
      (`Type__ukl_fields`) and layout (`Type__ukl_layout`). The kernel's entry takes a pointer to each argument's
      bytes, in its rows' order.
    - **A launch's threadgroups run together, up to its resident count** (`resident`, the host's): each group a
      coroutine of its own (its scheduler, its threads' stacks and its threadgroup memory), taking the launch's next
      group when its own ends. A thread that polls another group's progress (`ukl_poll_yield`, a grid barrier's poll)
      suspends; once every thread of its group waits so, the group yields to the next. So a persistent grid of up to
      `resident` groups meets its barriers here, and a larger one fails their bound, as on a GPU that holds fewer
      (decision 0468 §4). At a resident count of 1, groups run one after another. Groups take turns in order: a run
      is the same every time.
*/

#pragma once
#include "ukl_auto_macros.h"
#include "ukl_macros.h"

// 32 lanes, as on Metal and CUDA; another power of two up to 64 (`RERAIL_CPU_SIMD_WIDTH`) runs a kernel as a narrower
// device would, or a 64-lane one (AMD's).
#ifndef UKL_SIMD_WIDTH
#define UKL_SIMD_WIDTH 32
#endif

typedef unsigned int uint32_t;
typedef int int32_t;
typedef unsigned short uint16_t;
typedef short int16_t;
typedef unsigned char uint8_t;
typedef signed char int8_t;
typedef unsigned long long uint64_t;
typedef long long int64_t;
typedef unsigned int atomic_uint;
typedef int atomic_int;
typedef decltype(sizeof 0) size_t;

// `UKL_CPU_BACKEND` chose this backend (`ukl_shim.h`); a kernel's `#if UKL_CPU_BACKEND` is upstream's CPU runtime's code
// (MLDB's iostreams and printers), which this shim is not.
#undef UKL_CPU_BACKEND

// The GPU languages' built-in math the kernels call: Metal's and CUDA's `min`, `max`, `fma`, `sqrt`.
template <typename A, typename B> static inline auto min(A a, B b) -> decltype(a < b ? a : b) { return b < a ? b : a; }
template <typename A, typename B> static inline auto max(A a, B b) -> decltype(a < b ? a : b) { return a < b ? b : a; }
static inline float fma(float a, float b, float c) { return __builtin_fmaf(a, b, c); }
static inline double fma(double a, double b, double c) { return __builtin_fma(a, b, c); }
static inline float sqrt(float x) { return __builtin_sqrtf(x); }
static inline double sqrt(double x) { return __builtin_sqrt(x); }

#ifndef INFINITY
#  define INFINITY __builtin_inff()
#endif
#ifndef NAN
#  define NAN __builtin_nanf("")
#endif

// ---------------------------------------------------------------------------------------------------------------
// The coroutines: a context switch that saves the callee-saved registers on the stack it leaves.

// `__ukl_cpu_switch(save, to)`: saves the current stack pointer at `save`, continues on the stack `to`.
extern "C" void __ukl_cpu_switch(void** save, void* to);
// The first frame of a new coroutine returns into `__ukl_cpu_start`, which calls `__ukl_cpu_entry` with the
// coroutine's thread (held in a callee-saved register by its first frame).
extern "C" void __ukl_cpu_start();

#if defined(__aarch64__)
#  define __UKL_CPU_FRAME 176
#  define __UKL_CPU_ARG_SLOT 0
#  define __UKL_CPU_RET_SLOT 11
#  if defined(__APPLE__)
#    define __UKL_SYM(x) "_" #x
#    define __UKL_HIDE(x) ".private_extern _" #x "\n"
#  else
#    define __UKL_SYM(x) #x
#    define __UKL_HIDE(x) ".hidden " #x "\n.type " #x ", %function\n"
#  endif
__asm__(".text\n.p2align 2\n.globl " __UKL_SYM(__ukl_cpu_switch) "\n" __UKL_HIDE(__ukl_cpu_switch)
        __UKL_SYM(__ukl_cpu_switch) ":\n"
        "  sub sp, sp, #176\n"
        "  stp x19, x20, [sp, #0]\n  stp x21, x22, [sp, #16]\n  stp x23, x24, [sp, #32]\n"
        "  stp x25, x26, [sp, #48]\n  stp x27, x28, [sp, #64]\n  stp x29, x30, [sp, #80]\n"
        "  stp d8, d9, [sp, #96]\n  stp d10, d11, [sp, #112]\n  stp d12, d13, [sp, #128]\n  stp d14, d15, [sp, #144]\n"
        "  mov x2, sp\n  str x2, [x0]\n  mov sp, x1\n"
        "  ldp x19, x20, [sp, #0]\n  ldp x21, x22, [sp, #16]\n  ldp x23, x24, [sp, #32]\n"
        "  ldp x25, x26, [sp, #48]\n  ldp x27, x28, [sp, #64]\n  ldp x29, x30, [sp, #80]\n"
        "  ldp d8, d9, [sp, #96]\n  ldp d10, d11, [sp, #112]\n  ldp d12, d13, [sp, #128]\n  ldp d14, d15, [sp, #144]\n"
        "  add sp, sp, #176\n  ret\n"
        ".globl " __UKL_SYM(__ukl_cpu_start) "\n" __UKL_HIDE(__ukl_cpu_start)
        __UKL_SYM(__ukl_cpu_start) ":\n"
        "  mov x0, x19\n  bl " __UKL_SYM(__ukl_cpu_entry) "\n  brk #0\n");
#elif defined(__x86_64__)
#  define __UKL_CPU_FRAME 56
#  define __UKL_CPU_ARG_SLOT 4
#  define __UKL_CPU_RET_SLOT 6
#  if defined(__APPLE__)
#    define __UKL_SYM(x) "_" #x
#    define __UKL_HIDE(x) ".private_extern _" #x "\n"
#  else
#    define __UKL_SYM(x) #x
#    define __UKL_HIDE(x) ".hidden " #x "\n.type " #x ", @function\n"
#  endif
// The frame: r15 r14 r13 r12 rbx rbp, then the return address.
__asm__(".text\n.p2align 4\n.globl " __UKL_SYM(__ukl_cpu_switch) "\n" __UKL_HIDE(__ukl_cpu_switch)
        __UKL_SYM(__ukl_cpu_switch) ":\n"
        "  pushq %rbp\n  pushq %rbx\n  pushq %r12\n  pushq %r13\n  pushq %r14\n  pushq %r15\n"
        "  movq %rsp, (%rdi)\n  movq %rsi, %rsp\n"
        "  popq %r15\n  popq %r14\n  popq %r13\n  popq %r12\n  popq %rbx\n  popq %rbp\n  ret\n"
        ".globl " __UKL_SYM(__ukl_cpu_start) "\n" __UKL_HIDE(__ukl_cpu_start)
        __UKL_SYM(__ukl_cpu_start) ":\n"
        "  movq %rbx, %rdi\n  call " __UKL_SYM(__ukl_cpu_entry) "\n  ud2\n");
#else
#  error "the CPU shim has no context switch for this architecture"
#endif

// What the host gives a launch (`compute_kernel_cpu.rs`'s `Launch`, field for field).
struct __ukl_cpu_launch {
    uint32_t grid[3];
    uint32_t block[3];
    unsigned char* shared;     // the threadgroup memory of the launch's `RWLOCAL` rows, `shared_bytes` of it
    uint64_t shared_bytes;
    unsigned char* stacks;     // one stack a thread of a threadgroup, `stack_bytes` apart, each with a guard below
    uint64_t stack_bytes;
    char* error;               // where a failure is described, `error_bytes` at most
    uint64_t error_bytes;
    unsigned char* sched;      // the scheduler's storage (`__ukl_cpu_sched`), `sched_bytes` of it, the host thread's
    uint64_t sched_bytes;
    void* (*tls)(void);        // the host thread's slot for the running scheduler and thread (`__ukl_cpu_tls`)
    // The groups run at once: past 1, `shared` holds `shared_bytes` for each, and `stacks` a thread's stack for each
    // thread of each, then one for each group's own coroutine.
    uint32_t resident;
    uint32_t pad;
};

enum { __UKL_RUN = 0, __UKL_AT_TG = 1, __UKL_AT_SG = 2, __UKL_DONE = 3, __UKL_YIELD = 4 };

struct __ukl_cpu_thread {
    void* sp;
    uint32_t state;
    uint32_t lid[3];
    uint32_t index;            // linear, in the threadgroup
    uint32_t lane;             // in its SIMD group
    uint32_t simd;             // its SIMD group
    uint32_t sgop;             // its SIMD-group operations so far: the parity is the bank of `slots` it writes
    void (*body)(void*);
    void* ctx;
    void** back;               // where the coroutine goes when it ends: its scheduler's saved stack
};

#define __UKL_CPU_MAX_THREADS 1024u
#define __UKL_CPU_MAX_RESIDENT 64u

struct __ukl_cpu_sched {
    void* sp;
    const __ukl_cpu_launch* launch;
    uint32_t tgid[3];
    uint32_t n;
    unsigned char* stacks;     // the group's threads' stacks
    unsigned char* shared;     // the group's `RWLOCAL` memory
    __ukl_cpu_thread* slot;    // the group's own coroutine, when groups run together
    __ukl_cpu_thread threads[__UKL_CPU_MAX_THREADS];
    // Two banks, a SIMD-group operation's values in the bank of its parity: a lane writes the next operation's
    // bank while the others still read this one's, so one wait an operation suffices (the next operation's wait
    // needs every lane past this one's reads).
    uint64_t slots[2 * __UKL_CPU_MAX_THREADS];
    // Who waits where, kept by the waits, so a release needs no scan of the threads: at the threadgroup's barrier,
    // done, and each SIMD group's lanes at its operation and done.
    uint32_t at_tg, done;
    uint32_t sg_at[__UKL_CPU_MAX_THREADS / UKL_SIMD_WIDTH], sg_done[__UKL_CPU_MAX_THREADS / UKL_SIMD_WIDTH];
};

// The running scheduler and thread: in the host thread's one slot (`__ukl_cpu_launch::tls`), never a thread-local of
// the library's. macOS gives each loaded image's thread-locals a pthread key of their own, and a process runs out of
// them (about 512) after a few hundred kernel libraries: a thread-local's first use then aborts.
struct __ukl_cpu_tls {
    __ukl_cpu_sched* sched;
    __ukl_cpu_thread* self;
};
static void* (*__ukl_cpu_tls_of)(void);  // the launch's: the same function in every launch
#define __ukl_sched (((__ukl_cpu_tls*)__ukl_cpu_tls_of())->sched)
#define __ukl_self (((__ukl_cpu_tls*)__ukl_cpu_tls_of())->self)

// Thread `t` of `s` stops (it waits, or is done): straight on to the group's next runnable thread after it, else back
// to the scheduler. A pass of the group's runnable threads is so one switch a thread, in their order.
static inline void __ukl_cpu_pass(__ukl_cpu_sched* s, __ukl_cpu_thread* t)
{
    for (uint32_t j = t->index + 1; j < s->n; j++)
        if (s->threads[j].state == __UKL_RUN) {
            __ukl_self = &s->threads[j];
            __ukl_cpu_switch(&t->sp, s->threads[j].sp);
            return;
        }
    __ukl_cpu_switch(&t->sp, s->sp);
}

extern "C" __attribute__((used, visibility("hidden"))) void __ukl_cpu_entry(__ukl_cpu_thread* t)
{
    t->body(t->ctx);
    t->state = __UKL_DONE;
    __ukl_cpu_sched* s = __ukl_sched;
    // A group's thread hands on; a coroutine of the launch's own (a resident group's slot) goes back.
    if (s && t >= s->threads && t < s->threads + s->n) {
        s->done++;
        s->sg_done[t->simd]++;
        __ukl_cpu_pass(s, t);
    } else {
        __ukl_cpu_switch(&t->sp, *t->back);
    }
    __builtin_trap();
}

// Makes `t` a coroutine that runs `body(ctx)` on the stack below `top`, and goes to `*back` when it ends.
static inline void __ukl_cpu_prepare(__ukl_cpu_thread* t, unsigned char* top_bytes, void (*body)(void*), void* ctx, void** back)
{
    t->state = __UKL_RUN;
    t->body = body;
    t->ctx = ctx;
    t->back = back;
    // The top of the stack, 16-aligned, with its first frame: a switch into it returns into `__ukl_cpu_start` with
    // the coroutine in the register the frame's slot fills. On both, `__ukl_cpu_start` begins with the stack
    // 16-aligned, as a call needs it.
    uint64_t top = ((uint64_t)top_bytes) & ~(uint64_t)15;
    void** frame = (void**)(top - __UKL_CPU_FRAME);
    for (int k = 0; k < __UKL_CPU_FRAME / 8; k++)
        frame[k] = 0;
    frame[__UKL_CPU_ARG_SLOT] = (void*)t;
    frame[__UKL_CPU_RET_SLOT] = (void*)&__ukl_cpu_start;
    t->sp = frame;
}

// Suspends the calling thread at a barrier of kind `kind` until the scheduler releases it.
static inline void __ukl_cpu_wait(uint32_t kind)
{
    __ukl_cpu_thread* t = __ukl_self;
    __ukl_cpu_sched* s = __ukl_sched;
    t->state = kind;
    if (kind == __UKL_AT_TG)
        s->at_tg++;
    else if (kind == __UKL_AT_SG)
        s->sg_at[t->simd]++;
    __ukl_cpu_pass(s, t);
}

// Appends `s` to the launch's error.
static inline void __ukl_cpu_say(const __ukl_cpu_launch* L, const char* s)
{
    uint64_t at = 0;
    while (at + 1 < L->error_bytes && L->error[at])
        at++;
    for (; *s && at + 1 < L->error_bytes; s++)
        L->error[at++] = *s;
    if (L->error_bytes)
        L->error[at] = 0;
}

static inline void __ukl_cpu_say_u(const __ukl_cpu_launch* L, uint64_t v)
{
    char b[24];
    int n = 0;
    do {
        b[n++] = (char)('0' + v % 10);
        v /= 10;
    } while (v);
    char s[24];
    for (int i = 0; i < n; i++)
        s[i] = b[n - 1 - i];
    s[n] = 0;
    __ukl_cpu_say(L, s);
}

// Where a waiting thread waits, for a report: its two innermost return addresses (the operation's site and its
// caller's), from the frame its switch saved. 0 where the architecture's frame is not read here.
static inline uint64_t __ukl_cpu_where(const __ukl_cpu_thread* t)
{
#if defined(__aarch64__)
    void** frame = (void**)t->sp;
    void** fp = (void**)frame[10];
    return (uint64_t)frame[11] * 31u + (fp ? (uint64_t)fp[1] : 0u);
#else
    (void)t;
    return 0;
#endif
}

// What a SIMD group's lanes must share to meet at an operation. Under `UKL_CPU_SIMD_STRICT`
// (`RERAIL_CPU_SIMD_STRICT=1`, compiled without optimization, so no operation is duplicated): the whole chain of
// return addresses, the same operation reached by the same path, as a GPU's SIMD group meets one; a group whose
// lanes wait at two (some skipped an operation, or left an item early) fails the launch. Else nothing: any
// operations meet.
static inline uint64_t __ukl_cpu_site(const __ukl_cpu_thread* t)
{
#if defined(UKL_CPU_SIMD_STRICT) && defined(__aarch64__)
    void** frame = (void**)t->sp;
    uint64_t h = (uint64_t)frame[11];
    void** fp = (void**)frame[10];
    for (int k = 0; k < 64 && fp != 0; k++) {
        h = h * 1000003u + (uint64_t)fp[1];
        fp = (void**)fp[0];
    }
    return h;
#else
    (void)t;
    return 0;
#endif
}

// Runs one threadgroup's threads, each `f()`, to their ends; 0, or 1 with the launch's error written.
template <typename F> static int __ukl_cpu_group(__ukl_cpu_sched* s, F& f)
{
    const __ukl_cpu_launch* L = s->launch;
    uint32_t n = s->n;
    for (uint32_t i = 0; i < n; i++) {
        __ukl_cpu_thread* t = &s->threads[i];
        t->index = i;
        t->lid[0] = i % L->block[0];
        t->lid[1] = i / L->block[0] % L->block[1];
        t->lid[2] = i / (L->block[0] * L->block[1]);
        t->lane = i % UKL_SIMD_WIDTH;
        t->simd = i / UKL_SIMD_WIDTH;
        t->sgop = 0;
        __ukl_cpu_prepare(t, s->stacks + (uint64_t)(i + 1) * L->stack_bytes, [](void* c) { (*(F*)c)(); }, &f, &s->sp);
    }
    s->at_tg = 0;
    s->done = 0;
    for (uint32_t k = 0; k < __UKL_CPU_MAX_THREADS / UKL_SIMD_WIDTH; k++) {
        s->sg_at[k] = 0;
        s->sg_done[k] = 0;
    }
    for (;;) {
        // A pass: the first runnable thread, which hands on to the others in order (`__ukl_cpu_pass`); the last
        // comes back here. A thread is made runnable only here, so none is left after it.
        for (uint32_t i = 0; i < n; i++) {
            if (s->threads[i].state != __UKL_RUN)
                continue;
            __ukl_self = &s->threads[i];
            __ukl_cpu_switch(&s->sp, s->threads[i].sp);
            break;
        }
        // Every thread waits or is done: release a SIMD group whose live lanes all wait at a SIMD-group operation,
        // else every thread if all live ones wait at a threadgroup barrier. Under OpenCL's rule (`UKL_CPU_OPENCL_RULE`:
        // its SIMD groups emulated through threadgroup memory, whose only wait is the threadgroup's barrier), a SIMD
        // group is released only when every live thread of the threadgroup waits at a SIMD-group operation.
        bool released = false;
#if defined(UKL_CPU_OPENCL_RULE)
        {
            uint32_t live = 0, at = 0;
            for (uint32_t i = 0; i < n; i++) {
                live += s->threads[i].state != __UKL_DONE;
                at += s->threads[i].state == __UKL_AT_SG;
            }
            if (live && at == live) {
                for (uint32_t i = 0; i < n; i++)
                    if (s->threads[i].state == __UKL_AT_SG)
                        s->threads[i].state = __UKL_RUN;
                released = true;
            }
        }
#else
        for (uint32_t g = 0; g * UKL_SIMD_WIDTH < n; g++) {
            uint32_t width = n - g * UKL_SIMD_WIDTH < UKL_SIMD_WIDTH ? n - g * UKL_SIMD_WIDTH : UKL_SIMD_WIDTH;
            uint32_t at = s->sg_at[g], live = width - s->sg_done[g];
            if (at == 0 || at != live)
                continue;
            // Under `UKL_CPU_SIMD_STRICT`, every lane at the same operation (`__ukl_cpu_site`).
#if defined(UKL_CPU_SIMD_STRICT)
            for (uint32_t i = g * UKL_SIMD_WIDTH, first = n; i < n && i < (g + 1) * UKL_SIMD_WIDTH; i++) {
                if (s->threads[i].state != __UKL_AT_SG)
                    continue;
                if (first == n)
                    first = i;
                else if (__ukl_cpu_site(&s->threads[i]) != __ukl_cpu_site(&s->threads[first]))
                    at = 0;
            }
#endif
            if (live && at == live) {
                for (uint32_t i = g * UKL_SIMD_WIDTH; i < n && i < (g + 1) * UKL_SIMD_WIDTH; i++)
                    if (s->threads[i].state == __UKL_AT_SG)
                        s->threads[i].state = __UKL_RUN;
                s->sg_at[g] = 0;
                released = true;
            }
        }
#endif
        if (released)
            continue;
#if defined(UKL_CPU_OPENCL_RULE)
        uint32_t live = 0, at = 0;
        for (uint32_t i = 0; i < n; i++) {
            live += s->threads[i].state != __UKL_DONE;
            at += s->threads[i].state == __UKL_AT_TG;
        }
#else
        uint32_t live = n - s->done, at = s->at_tg;
#endif
        if (!live)
            return 0;
        if (at == live) {
            for (uint32_t i = 0; i < n; i++)
                if (s->threads[i].state == __UKL_AT_TG)
                    s->threads[i].state = __UKL_RUN;
            s->at_tg = 0;
            continue;
        }
        // Threads that poll another group's progress: the group yields to the others (when groups run together),
        // then polls again.
        bool polls = false;
        for (uint32_t i = 0; i < n; i++)
            if (s->threads[i].state == __UKL_YIELD) {
                s->threads[i].state = __UKL_RUN;
                polls = true;
            }
        if (polls) {
            if (s->slot) {
                __ukl_cpu_switch(&s->slot->sp, *s->slot->back);
                __ukl_sched = s;
            }
            continue;
        }
        // No barrier can be passed: say who waits where.
        __ukl_cpu_say(L, "threadgroup (");
        for (int d = 0; d < 3; d++) {
            __ukl_cpu_say_u(L, s->tgid[d]);
            __ukl_cpu_say(L, d < 2 ? ", " : ")");
        }
        __ukl_cpu_say(L, ": no barrier can be passed:");
#if defined(__aarch64__)
        // Where the threads wait: the first thread at each site (four at most), its return addresses, from its saved frame up its frame
        // pointers, as offsets past `__ukl_cpu_switch` (for `atos` on the library with its symbols).
        uint64_t seen[4] = {0, 0, 0, 0};
        for (uint32_t i = 0, sites = 0; i < n && sites < 4; i++) {
            if (s->threads[i].state == __UKL_DONE)
                continue;
            uint64_t site = __ukl_cpu_where(&s->threads[i]);
            bool known = false;
            for (uint32_t k = 0; k < sites; k++)
                known = known || seen[k] == site;
            if (known)
                continue;
            seen[sites++] = site;
            __ukl_cpu_say(L, " thread ");
            __ukl_cpu_say_u(L, i);
            __ukl_cpu_say(L, "'s frames past the switch:");
            void** frame = (void**)s->threads[i].sp;
            uint64_t base = (uint64_t)(void*)&__ukl_cpu_switch;
            uint64_t ret = (uint64_t)frame[11];
            void** fp = (void**)frame[10];
            for (int k = 0; k < 16 && ret != 0; k++) {
                __ukl_cpu_say(L, " ");
                __ukl_cpu_say_u(L, ret - base);
                if (fp == 0)
                    break;
                ret = (uint64_t)fp[1];
                fp = (void**)fp[0];
            }
            __ukl_cpu_say(L, ";");
        }
#endif
        for (uint32_t i = 0; i < n; i++) {
            uint32_t st = s->threads[i].state;
            if (st == __UKL_DONE)
                continue;
            __ukl_cpu_say(L, " thread ");
            __ukl_cpu_say_u(L, i);
            __ukl_cpu_say(L, st == __UKL_AT_TG ? " at a threadgroup barrier;" : " at a SIMD-group operation;");
        }
        return 1;
    }
}

// The threadgroup `g` of the launch's linear order (x fastest).
static inline void __ukl_cpu_tgid(const __ukl_cpu_launch* L, uint32_t g, uint32_t* tgid)
{
    tgid[0] = g % L->grid[0];
    tgid[1] = g / L->grid[0] % L->grid[1];
    tgid[2] = g / (L->grid[0] * L->grid[1]);
}

// A call of `f(k)` as a coroutine's body.
template <typename F> struct __ukl_cpu_call {
    F* f;
    uint32_t k;
    static void run(void* c)
    {
        __ukl_cpu_call* a = (__ukl_cpu_call*)c;
        (*a->f)(a->k);
    }
};

// Runs every threadgroup of the launch, each `group(sched)` (which declares the group's storage and runs its
// threads); 0, or 1 with the error written. Up to `resident` groups at once, each a coroutine of its own.
template <typename G> static int __ukl_cpu_run(const __ukl_cpu_launch* L, G&& group)
{
    uint32_t n = L->block[0] * L->block[1] * L->block[2];
    if (n > __UKL_CPU_MAX_THREADS) {
        __ukl_cpu_say(L, "more threads in a threadgroup than the CPU backend runs (1024)");
        return 1;
    }
    if (L->sched_bytes < sizeof(__ukl_cpu_sched)) {
        __ukl_cpu_say(L, "the host gave the scheduler too little storage");
        return 1;
    }
    __atomic_store_n(&__ukl_cpu_tls_of, L->tls, __ATOMIC_RELAXED);
    uint32_t total = L->grid[0] * L->grid[1] * L->grid[2];
    uint32_t resident = L->resident < 1u ? 1u : L->resident;
    resident = resident > __UKL_CPU_MAX_RESIDENT ? __UKL_CPU_MAX_RESIDENT : resident;
    resident = resident > total ? total : resident;
    __ukl_cpu_sched* outer = __ukl_sched;
    __ukl_cpu_thread* outer_self = __ukl_self;
    int r = 0;
    if (resident <= 1u) {
        // One group at a time: its scheduler in the host's storage. Groups that run together keep theirs on their
        // own coroutines' stacks.
        __ukl_cpu_sched& sched = *(__ukl_cpu_sched*)L->sched;
        __ukl_sched = &sched;
        sched.launch = L;
        sched.n = n;
        sched.stacks = L->stacks;
        sched.shared = L->shared;
        sched.slot = 0;
        for (uint32_t g = 0; g < total && !r; g++) {
            __ukl_cpu_tgid(L, g, sched.tgid);
            r = group(&sched);
        }
    } else {
        // Each slot a coroutine that runs the launch's next group until none is left, its scheduler on its stack.
        void* top = 0;
        uint32_t next = 0;
        __ukl_cpu_thread slots[__UKL_CPU_MAX_RESIDENT];
        auto slot = [&](uint32_t k) {
            __ukl_cpu_sched s;
            s.launch = L;
            s.n = n;
            s.slot = &slots[k];
            s.stacks = L->stacks + (uint64_t)k * n * L->stack_bytes;
            s.shared = L->shared + (uint64_t)k * L->shared_bytes;
            while (!r && next < total) {
                __ukl_cpu_tgid(L, next++, s.tgid);
                __ukl_sched = &s;
                int e = group(&s);
                r = r ? r : e;
            }
        };
        __ukl_cpu_call<decltype(slot)> calls[__UKL_CPU_MAX_RESIDENT];
        unsigned char* own = L->stacks + (uint64_t)resident * n * L->stack_bytes;
        for (uint32_t k = 0; k < resident; k++) {
            calls[k].f = &slot;
            calls[k].k = k;
            __ukl_cpu_prepare(&slots[k], own + (uint64_t)(k + 1) * L->stack_bytes, &__ukl_cpu_call<decltype(slot)>::run, &calls[k], &top);
        }
        for (bool any = true; any && !r;) {
            any = false;
            for (uint32_t k = 0; k < resident && !r; k++) {
                if (slots[k].state == __UKL_DONE)
                    continue;
                any = true;
                __ukl_cpu_switch(&top, slots[k].sp);
            }
        }
    }
    __ukl_sched = outer;
    __ukl_self = outer_self;
    return r;
}

// ---------------------------------------------------------------------------------------------------------------
// The SIMD group's exchange: each lane's value into its slot, then each lane's read of another's.

// Each lane's `v` into its slot of the operation's bank, then one wait for the SIMD group: the group's slots, lane
// 0's first.
static inline const uint64_t* __ukl_cpu_put(uint64_t v)
{
    __ukl_cpu_thread* t = __ukl_self;
    __ukl_cpu_sched* s = __ukl_sched;
    uint64_t* bank = s->slots + (uint64_t)(t->sgop++ & 1u) * __UKL_CPU_MAX_THREADS;
    bank[t->index] = v;
    __ukl_cpu_wait(__UKL_AT_SG);
    return bank + (uint64_t)t->simd * UKL_SIMD_WIDTH;
}

// Whether lane `l` of the calling thread's SIMD group took part (is in the group and not done).
static inline bool __ukl_cpu_in(uint32_t l)
{
    uint32_t i = __ukl_self->simd * UKL_SIMD_WIDTH + l;
    return i < __ukl_sched->n && __ukl_sched->threads[i].state != __UKL_DONE;
}

static inline uint64_t __ukl_cpu_exchange(uint64_t v, uint32_t from_lane)
{
    uint32_t l = from_lane % UKL_SIMD_WIDTH;
    const uint64_t* g = __ukl_cpu_put(v);
    return __ukl_self->simd * UKL_SIMD_WIDTH + l < __ukl_sched->n ? g[l] : v;
}

// The lanes whose `b` holds, lane `l` at bit `l` (64 bits: the widest group).
static inline uint64_t __ukl_cpu_ballot(bool b)
{
    const uint64_t* g = __ukl_cpu_put(b);
    uint64_t m = 0;
    for (uint32_t l = 0; l < UKL_SIMD_WIDTH; l++)
        if (__ukl_cpu_in(l) && g[l])
            m |= (uint64_t)1 << l;
    return m;
}

// The SIMD group's reduction of its live lanes' `v` (`op` 0 sum, 1 minimum, 2 maximum), in one wait.
static inline uint32_t __ukl_cpu_reduce(uint32_t v, uint32_t op)
{
    const uint64_t* g = __ukl_cpu_put(v);
    uint32_t r = op == 0u ? 0u : op == 1u ? 0xffffffffu : 0u;
    for (uint32_t l = 0; l < UKL_SIMD_WIDTH; l++) {
        if (!__ukl_cpu_in(l))
            continue;
        uint32_t x = (uint32_t)g[l];
        r = op == 0u ? r + x : op == 1u ? (x < r ? x : r) : (x > r ? x : r);
    }
    return r;
}

// The sum of the live lanes' `v` below this lane (`inclusive`: and its own), in one wait.
static inline uint32_t __ukl_cpu_prefix(uint32_t v, bool inclusive)
{
    uint32_t self = __ukl_self->lane;
    const uint64_t* g = __ukl_cpu_put(v);
    uint32_t r = 0;
    for (uint32_t l = 0; l < self; l++)
        if (__ukl_cpu_in(l))
            r += (uint32_t)g[l];
    return inclusive ? r + v : r;
}

// The inclusive segmented scan (`add`: a sum, else a maximum) of the live lanes' `v` up to this lane, from the last
// lane at or before it whose `f` is set; whether there is one in `*fin`. In one wait.
static inline uint32_t __ukl_cpu_seg(bool add, uint32_t f, uint32_t v, uint32_t* fin)
{
    uint32_t self = __ukl_self->lane;
    const uint64_t* g = __ukl_cpu_put(((uint64_t)(f != 0u) << 32) | v);
    uint32_t r = 0, h = 0;
    for (uint32_t l = 0; l <= self; l++) {
        if (!__ukl_cpu_in(l))
            continue;
        uint32_t x = (uint32_t)g[l];
        if (g[l] >> 32) {
            r = x;
            h = 1;
        } else {
            r = add ? r + x : (x > r ? x : r);
        }
    }
    *fin = h;
    return r;
}

// ---------------------------------------------------------------------------------------------------------------
// UKL's macros, as upstream's kernels (the forest's) use them.

#define __constant
#define __kernel
#define __global
#define __local

#define ukl_assert(x)
#define ukl_assert_equal(x, y)
#define ukl_assert_not_equal(x, y)
#define ukl_assert_less(x, y)
#define ukl_assert_less_equal(x, y)
#define ukl_assert_greater(x, y)
#define ukl_assert_greater_equal(x, y)

#define ROBUFFER(Type) const Type *
#define RWBUFFER(Type) Type *
#define WOBUFFER(Type) Type *
#define RWLOCAL(Type) Type *
#define CONSTBUFFER(Type) const Type *

#define CAST_RWBUFFER(Buf, Type) reinterpret_cast<Type *>(Buf)
#define CAST_ROBUFFER(Buf, Type) reinterpret_cast<const Type *>(Buf)
#define CAST_RWLOCAL(Buf, Type) reinterpret_cast<Type *>(Buf)
#define CAST_ROLOCAL(Buf, Type) reinterpret_cast<const Type *>(Buf)

#define UKL_LOCAL_PTR(Type) Type *
#define UKL_LOCAL_PTR_CAST(Type) (Type *)
#define UKL_GLOBAL_PTR(Type) Type *
#define UKL_GLOBAL_PTR_CAST(Type) (Type *)

#define ukl_simdgroup_barrier() __ukl_cpu_wait(__UKL_AT_SG)
#define ukl_threadgroup_barrier() __ukl_cpu_wait(__UKL_AT_TG)
#define printf(...)

// Functions: a barrier is reached in any of them, so the SYNC_* ones are ordinary calls too.
#define UKL_FUNCTION(Version, Return, Name) Return Name
#define SYNC_FUNCTION(Version, Return, Name) Return Name
#define SYNC_RETURN(...) return __VA_ARGS__
#define SYNC_CALL(Fn, ...) Fn(__VA_ARGS__)

template<typename T> static inline T ukl_atom_load(T * p) { return __atomic_load_n(p, __ATOMIC_RELAXED); }
template<typename T, typename V> static inline void ukl_atom_store(T * p, V v) { __atomic_store_n(p, (T)v, __ATOMIC_RELAXED); }
#define atom_load(x) ukl_atom_load(x)
#define atom_store(x, y) ukl_atom_store(x, y)
#define atom_add(x, y) __atomic_fetch_add(x, y, __ATOMIC_RELAXED)
#define atom_sub(x, y) __atomic_fetch_sub(x, y, __ATOMIC_RELAXED)
#define atom_or(x, y) __atomic_fetch_or(x, y, __ATOMIC_RELAXED)
#define atom_inc(x) __atomic_fetch_add(x, 1, __ATOMIC_RELAXED)
#define atom_dec(x) __atomic_fetch_sub(x, 1, __ATOMIC_RELAXED)
#define atom_load_local atom_load
#define atom_store_local atom_store
#define atom_add_local atom_add
#define atom_sub_local atom_sub
#define atom_or_local atom_or
#define atom_inc_local atom_inc
#define atom_dec_local atom_dec

#define __UKL_CPU_GID(n) (__ukl_sched->tgid[n] * __ukl_sched->launch->block[n] + __ukl_self->lid[n])
#define get_global_id(n) __UKL_CPU_GID(n)
#define get_global_size(n) (__ukl_sched->launch->grid[n] * __ukl_sched->launch->block[n])
#define get_local_id(n) (__ukl_self->lid[n])
#define get_local_size(n) (__ukl_sched->launch->block[n])

#define KERNEL_RETURN() return

#define ukl_vec_el(expr, el) (expr).el

#define ukl_popcount __builtin_popcount
#define ukl_clz(x) ((x) == 0 ? 32 : __builtin_clz(x))

static inline uint32_t extract_bits(uint32_t val, uint16_t offset, uint32_t bits)
{
    return (val >> offset) & (bits >= 32 ? 0xffffffffu : ((1u << bits) - 1u));
}

static inline int32_t reinterpretFloatAsInt(float val)
{
    int32_t r;
    __builtin_memcpy(&r, &val, 4);
    return r;
}

static inline float reinterpretIntAsFloat(int32_t val)
{
    float r;
    __builtin_memcpy(&r, &val, 4);
    return r;
}

// Upstream's SIMD-group operations (`simd_lane` and `tmp` are the signature Metal's and the CPU's share), written for
// groups of 32 lanes at most: their ballot is 32 bits.
static inline uint32_t ukl_simdgroup_ballot(bool val, uint16_t simd_lane, atomic_uint * tmp)
{
    return (uint32_t)__ukl_cpu_ballot(val);
}

static inline uint32_t ukl_simdgroup_sum(uint32_t val, uint16_t simd_lane, atomic_uint * tmp)
{
    for (uint32_t d = UKL_SIMD_WIDTH / 2; d > 0; d >>= 1)
        val += (uint32_t)__ukl_cpu_exchange(val, __ukl_self->lane ^ d);
    return val;
}

static inline uint32_t ukl_prefix_exclusive_sum_bitmask(uint32_t bits, uint16_t n)
{
    return __builtin_popcount(bits & ((1u << n) - 1u));
}

static inline uint32_t ukl_simdgroup_prefix_exclusive_sum_bools(bool val, uint16_t simd_lane, atomic_uint * tmp)
{
    return ukl_prefix_exclusive_sum_bitmask((uint32_t)__ukl_cpu_ballot(val), simd_lane);
}

static inline uint32_t ukl_simdgroup_broadcast_first(uint32_t val, uint16_t simd_lane, atomic_uint * tmp)
{
    uint64_t live = __ukl_cpu_ballot(true);
    return (uint32_t)__ukl_cpu_exchange(val, (uint32_t)__builtin_ctzll(live));
}

// ---------------------------------------------------------------------------------------------------------------
// A kernel's parameters: what its body sees (inner), how the entry reads each argument from the pointers the host
// passes (outer), and the threadgroup's own storage.

#define GEN_INNER_PARAMETER_RWBUFFER(Type) Type *
#define GEN_INNER_PARAMETER_ROBUFFER(Type) const Type *
#define GEN_INNER_PARAMETER_WOBUFFER(Type) Type *
#define GEN_INNER_PARAMETER_LITERAL(Type) Type
#define GEN_INNER_PARAMETER_RWLOCAL(Type) Type *
#define GEN_INNER_PARAMETER_TUNEABLE(Type) Type
#define GEN_INNER_PARAMETER_GID0(Type) const Type
#define GEN_INNER_PARAMETER_GID1(Type) const Type
#define GEN_INNER_PARAMETER_GID2(Type) const Type
#define GEN_INNER_PARAMETER_GSZ0(Type) const Type
#define GEN_INNER_PARAMETER_GSZ1(Type) const Type
#define GEN_INNER_PARAMETER_GSZ2(Type) const Type
#define GEN_INNER_PARAMETER_LID0(Type) const Type
#define GEN_INNER_PARAMETER_LID1(Type) const Type
#define GEN_INNER_PARAMETER_LID2(Type) const Type
#define GEN_INNER_PARAMETER_LSZ0(Type) const Type
#define GEN_INNER_PARAMETER_LSZ1(Type) const Type
#define GEN_INNER_PARAMETER_LSZ2(Type) const Type
#define GEN_INNER_PARAMETER_ARGS(Type) const Type &
#define GEN_INNER_PARAMETER_CONSTBUFFER(Type) const Type *
#define GEN_INNER_PARAMETER_TGID0(Type) const Type
#define GEN_INNER_PARAMETER_TGID1(Type) const Type
#define GEN_INNER_PARAMETER_TGID2(Type) const Type
#define GEN_INNER_PARAMETER_LANE(Type) const Type
#define GEN_INNER_PARAMETER_NLANES(Type) const Type
#define GEN_INNER_PARAMETER_SHARED(Type) Type *

#define GEN_INNER_PARAMETER2(Access, Type, Name, Bounds) CALL2_A(GEN_INNER_PARAMETER_, Access, Type) Name

#define PASS_INNER_ARG_RWBUFFER(Type, Name) Name
#define PASS_INNER_ARG_ROBUFFER(Type, Name) Name
#define PASS_INNER_ARG_WOBUFFER(Type, Name) Name
#define PASS_INNER_ARG_LITERAL(Type, Name) Name
#define PASS_INNER_ARG_RWLOCAL(Type, Name) reinterpret_cast<Type *>(__ukl_s->shared + Name)
#define PASS_INNER_ARG_TUNEABLE(Type, Name) Name
#define PASS_INNER_ARG_GID0(Type, Name) __UKL_CPU_GID(0)
#define PASS_INNER_ARG_GID1(Type, Name) __UKL_CPU_GID(1)
#define PASS_INNER_ARG_GID2(Type, Name) __UKL_CPU_GID(2)
#define PASS_INNER_ARG_GSZ0(Type, Name) get_global_size(0)
#define PASS_INNER_ARG_GSZ1(Type, Name) get_global_size(1)
#define PASS_INNER_ARG_GSZ2(Type, Name) get_global_size(2)
#define PASS_INNER_ARG_LID0(Type, Name) (__ukl_self->lid[0])
#define PASS_INNER_ARG_LID1(Type, Name) (__ukl_self->lid[1])
#define PASS_INNER_ARG_LID2(Type, Name) (__ukl_self->lid[2])
#define PASS_INNER_ARG_LSZ0(Type, Name) (__ukl_L->block[0])
#define PASS_INNER_ARG_LSZ1(Type, Name) (__ukl_L->block[1])
#define PASS_INNER_ARG_LSZ2(Type, Name) (__ukl_L->block[2])
#define PASS_INNER_ARG_ARGS(Type, Name) Name
#define PASS_INNER_ARG_CONSTBUFFER(Type, Name) Name
#define PASS_INNER_ARG_TGID0(Type, Name) (__ukl_sched->tgid[0])
#define PASS_INNER_ARG_TGID1(Type, Name) (__ukl_sched->tgid[1])
#define PASS_INNER_ARG_TGID2(Type, Name) (__ukl_sched->tgid[2])
#define PASS_INNER_ARG_LANE(Type, Name) (__ukl_self->lane)
#define PASS_INNER_ARG_NLANES(Type, Name) ((uint32_t)UKL_SIMD_WIDTH)
#define PASS_INNER_ARG_SHARED(Type, Name) Name

#define PASS_INNER_ARG2(Access, Type, Name, Bounds) CALL2_A(PASS_INNER_ARG_, Access, Type, Name)

// The entry's reads of its arguments: argument `k` is the k-th row that takes one, read from `__ukl_args[k]`.
#define GEN_OUTER_LOAD_RWBUFFER(Type, Name) Type * Name = *(Type * const *)__ukl_args[__ukl_k++];
#define GEN_OUTER_LOAD_ROBUFFER(Type, Name) const Type * Name = *(const Type * const *)__ukl_args[__ukl_k++];
#define GEN_OUTER_LOAD_WOBUFFER(Type, Name) Type * Name = *(Type * const *)__ukl_args[__ukl_k++];
#define GEN_OUTER_LOAD_CONSTBUFFER(Type, Name) const Type * Name = *(const Type * const *)__ukl_args[__ukl_k++];
#define GEN_OUTER_LOAD_ARGS(Type, Name) const Type & Name = *(const Type *)__ukl_args[__ukl_k++];
#define GEN_OUTER_LOAD_LITERAL(Type, Name) Type Name = *(const Type *)__ukl_args[__ukl_k++];
#define GEN_OUTER_LOAD_TUNEABLE(Type, Name) Type Name = *(const Type *)__ukl_args[__ukl_k++];
#define GEN_OUTER_LOAD_RWLOCAL(Type, Name) uint32_t Name = *(const uint32_t *)__ukl_args[__ukl_k++];
#define GEN_OUTER_LOAD_GID0(Type, Name)
#define GEN_OUTER_LOAD_GID1(Type, Name)
#define GEN_OUTER_LOAD_GID2(Type, Name)
#define GEN_OUTER_LOAD_GSZ0(Type, Name)
#define GEN_OUTER_LOAD_GSZ1(Type, Name)
#define GEN_OUTER_LOAD_GSZ2(Type, Name)
#define GEN_OUTER_LOAD_LID0(Type, Name)
#define GEN_OUTER_LOAD_LID1(Type, Name)
#define GEN_OUTER_LOAD_LID2(Type, Name)
#define GEN_OUTER_LOAD_LSZ0(Type, Name)
#define GEN_OUTER_LOAD_LSZ1(Type, Name)
#define GEN_OUTER_LOAD_LSZ2(Type, Name)
#define GEN_OUTER_LOAD_TGID0(Type, Name)
#define GEN_OUTER_LOAD_TGID1(Type, Name)
#define GEN_OUTER_LOAD_TGID2(Type, Name)
#define GEN_OUTER_LOAD_LANE(Type, Name)
#define GEN_OUTER_LOAD_NLANES(Type, Name)
#define GEN_OUTER_LOAD_SHARED(Type, Name)

#define GEN_OUTER_LOAD2(Access, Type, Name, Bounds) CALL2_A(GEN_OUTER_LOAD_, Access, Type, Name)

// A threadgroup's own storage (`SHARED` rows): one array a threadgroup, which its threads share.
#define GEN_SHARED_DECL_SHARED(Type, Name, Bounds) Type Name[Bounds];

// What the host binds a kernel by: as the CUDA shim's.
template<typename T> struct ukl_type_code { static constexpr unsigned long long value = __is_class(T) ? 's' : 'u'; };
template<> struct ukl_type_code<float> { static constexpr unsigned long long value = 'f'; };
template<> struct ukl_type_code<double> { static constexpr unsigned long long value = 'd'; };
template<> struct ukl_type_code<bool> { static constexpr unsigned long long value = 'b'; };
template<> struct ukl_type_code<char> { static constexpr unsigned long long value = 'i'; };
template<> struct ukl_type_code<signed char> { static constexpr unsigned long long value = 'i'; };
template<> struct ukl_type_code<short> { static constexpr unsigned long long value = 'i'; };
template<> struct ukl_type_code<int> { static constexpr unsigned long long value = 'i'; };
template<> struct ukl_type_code<long> { static constexpr unsigned long long value = 'i'; };
template<> struct ukl_type_code<long long> { static constexpr unsigned long long value = 'i'; };

#define GEN_ROW_LAYOUT2(Access, Type, Name, Bounds) sizeof(Type), alignof(Type), ukl_type_code<Type>::value

// An `ARGS` structure's fields' names, and a function that writes its size and alignment, then each field's
// offset, size and kind (`Type__ukl_layout`, as CUDA's layout kernel does).
#define __UKL_FIELD_LAYOUT(Type, Field) \
    (unsigned long long)((const char *)&__ukl_v.Field - (const char *)&__ukl_v), sizeof(__ukl_v.Field), \
    ukl_type_code<decltype(__ukl_v.Field)>::value
#define UKL_REFLECT_STRUCT(Type, ...) \
    extern "C" const char Type ## __ukl_fields[] = #__VA_ARGS__; \
    extern "C" void Type ## __ukl_layout(unsigned long long * __ukl_out) \
    { \
        Type __ukl_v; \
        const unsigned long long __ukl_l[] = \
            { sizeof(Type), alignof(Type), __UKL_FOREACH_CTX(__UKL_FIELD_LAYOUT, Type, __VA_ARGS__) }; \
        for (unsigned __ukl_i = 0; __ukl_i < sizeof(__ukl_l) / sizeof(__ukl_l[0]); ++__ukl_i) \
            __ukl_out[__ukl_i] = __ukl_l[__ukl_i]; \
    }

// The kernel: its rows and their layouts, its body (`Name_inner`), and its entry, which reads its arguments and
// runs every threadgroup: 0, or 1 with the launch's error written.
// The rows expanded first: a kernel may give them as a macro (a grid's shared rows), which the body stringizes and
// counts.
#define DEFINE_KERNEL2(Library, Name, ...) __UKL_DEFINE_KERNEL2(Library, Name, __VA_ARGS__)
#define __UKL_DEFINE_KERNEL2(Library, Name, ...) \
    extern "C" const char Name ## __ukl_params[] = #__VA_ARGS__; \
    extern "C" const unsigned long long Name ## __ukl_rows[] = \
        { __UKL_FOREACH_QUAD(GEN_ROW_LAYOUT2, __VA_ARGS__) }; \
    static inline void Name ## _inner( __UKL_FOREACH_QUAD(GEN_INNER_PARAMETER2, __VA_ARGS__) ); \
    extern "C" int Name (void * const * __ukl_args, const __ukl_cpu_launch * __ukl_L) \
    { \
        unsigned __ukl_k = 0; \
        (void)__ukl_k; \
        __UKL_FOREACH_QUAD_NOSEP(GEN_OUTER_LOAD2, __VA_ARGS__) \
        return __ukl_cpu_run(__ukl_L, [&](__ukl_cpu_sched * __ukl_s) { \
            __UKL_FOREACH_QUAD_NOSEP(GEN_SHARED_DECL2, __VA_ARGS__) \
            auto __ukl_body = [&]() { Name ## _inner ( __UKL_FOREACH_QUAD(PASS_INNER_ARG2, __VA_ARGS__)); }; \
            return __ukl_cpu_group(__ukl_s, __ukl_body); \
        }); \
    } \
    static inline void Name ## _inner( __UKL_FOREACH_QUAD(GEN_INNER_PARAMETER2, __VA_ARGS__) )
