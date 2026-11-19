// 16z's pieces decoded on the device (decisions 0336 §8, 0156 §2.1; E26's shape): one SIMD group a piece, every
// piece of an admission's 16z documents in a slab in one dispatch. A piece is raw or one LZ4 block whose matches
// stay inside it; it decodes to its array's bytes for its block's nodes (`fixed`) and then its side list's part
// (`side`), each where it lies in the document's format 16 tape in the slab. Every lane reads the same token; the
// lanes copy a literal run or a match 32 bytes at a time, a match closer than 32 bytes in rounds of its distance,
// straight into the slab (device memory: no threadgroup room, so no piece is too long). A piece that does not
// decode to its length, or reads or writes past its bounds, sets `bad` and stops: the admission refuses it.

struct PieceRun { uint src, src_len, lz4, fixed, fixed_len, side, side_len, pad; };

// Byte `p` of the piece's output: the array's part, then the side list's.
static inline uint piece_at(PieceRun r, uint p) { return p < r.fixed_len ? r.fixed + p : r.side + (p - r.fixed_len); }

// An LZ4 length past its 4-bit field: bytes of 255 and one below. `ok` is false past the piece's end.
static inline uint piece_ext(device const uchar* s, thread uint& ip, uint end, uint n, thread bool& ok) {
    uint c;
    do {
        if (ip >= end) { ok = false; return n; }
        c = s[ip++];
        n += c;
    } while (c == 255u);
    return n;
}

kernel void decode16z(device const PieceRun* runs [[buffer(0)]], device const uchar* in [[buffer(1)]],
                      device uchar* slab [[buffer(2)]], device atomic_uint* bad [[buffer(3)]],
                      constant uint& count [[buffer(4)]], uint tg [[threadgroup_position_in_grid]],
                      uint sg [[simdgroup_index_in_threadgroup]], uint groups [[simdgroups_per_threadgroup]],
                      uint lane [[thread_index_in_simdgroup]]) {
    uint id = tg * groups + sg;
    if (id >= count) return;
    PieceRun r = runs[id];
    device const uchar* s = in + r.src;
    uint total = r.fixed_len + r.side_len;
    if (r.lz4 == 0u) {
        if (r.src_len != total) { atomic_store_explicit(bad, 1u, memory_order_relaxed); return; }
        for (uint i = lane; i < total; i += 32u) slab[piece_at(r, i)] = s[i];
        return;
    }
    uint ip = 0u, op = 0u;
    bool ok = true;
    while (ok) {
        if (ip >= r.src_len) { ok = false; break; }
        uint tok = s[ip++];
        uint lit = tok >> 4;
        if (lit == 15u) lit = piece_ext(s, ip, r.src_len, lit, ok);
        if (!ok || ip + lit > r.src_len || op + lit > total) { ok = false; break; }
        for (uint i = lane; i < lit; i += 32u) slab[piece_at(r, op + i)] = s[ip + i];
        ip += lit;
        op += lit;
        // The last sequence is literals alone, and ends the block.
        if (ip == r.src_len) break;
        if (ip + 2u > r.src_len) { ok = false; break; }
        uint off = uint(s[ip]) | (uint(s[ip + 1u]) << 8);
        ip += 2u;
        uint ml = tok & 15u;
        if (ml == 15u) ml = piece_ext(s, ip, r.src_len, ml, ok);
        ml += 4u;
        if (!ok || off == 0u || off > op || op + ml > total) { ok = false; break; }
        simdgroup_barrier(mem_flags::mem_device);
        // Rounds of at most 32 bytes, and of the distance when it is shorter: each round reads only bytes written
        // before it.
        uint step = min(off, 32u);
        for (uint at = 0u; at < ml; at += step) {
            uint i = at + lane;
            if (lane < step && i < ml) slab[piece_at(r, op + i)] = slab[piece_at(r, op + i - off)];
            simdgroup_barrier(mem_flags::mem_device);
        }
        op += ml;
    }
    if (!ok || op != total) atomic_store_explicit(bad, 1u, memory_order_relaxed);
}
