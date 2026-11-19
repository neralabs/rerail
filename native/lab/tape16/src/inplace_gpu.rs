//! E10 on Metal (0156, t38): the in-place questions as kernels, one thread a question, on each candidate's
//! one-buffer image (`inplace::GpuImage`). Every answer is checked against the CPU reader's.

use crate::inplace::{B, EfEf, ForLen, Gpu, GpuImage, Pos, PredGap, Reader, X, XF};
use metal::{Device, MTLResourceOptions, MTLSize};

pub const SOURCE: &str = r#"
#include <metal_stdlib>
using namespace metal;

inline ulong mask(uint w) { return w >= 64 ? ~0ul : ((1ul << w) - 1ul); }

inline ulong extract(device const ulong* words, ulong bit, uint w) {
    ulong k = bit >> 6;
    uint o = uint(bit & 63);
    ulong lo = words[k], hi = words[k + 1];
    return ((lo >> o) | ((hi << 1) << (63 - o))) & mask(w);
}

struct Rd {
    device const uchar* b;
    constant uint* p;
    uint c;
    uint val() { return p[c++]; }
    template <typename T> device const T* arr() { return (device const T*)(b + p[c++]); }
};

struct Packed {
    uint w; device const ulong* words;
    ulong get(uint i) const { return extract(words, ulong(i) * w, w); }
};
inline Packed rd_packed(thread Rd& r) { Packed x; x.w = r.val(); x.words = r.arr<ulong>(); return x; }

struct PFor {
    uint with_base, n; device const ulong* head; device const ulong* base; device const ulong* words;
    ulong get(uint i) const {
        uint b = i >> 8;
        ulong h = head[b];
        uint w = uint(h & 63), e = uint((h >> 12) & 511);
        ulong off = h >> 21;
        ulong v = extract(words, off + ulong(i & 255) * w, w);
        ulong bs = with_base ? base[b] : 0;
        ulong lim = (1ul << w) - e;
        if (v >= lim) {
            uint ew = uint((h >> 6) & 63);
            uint len = min(256u, n - b * 256);
            return bs + extract(words, off + ulong(len) * w + (v - lim) * ew, ew);
        }
        return bs + v;
    }
};
inline PFor rd_pfor(thread Rd& r) {
    PFor x;
    x.with_base = r.val(); x.n = r.val();
    x.head = r.arr<ulong>();
    if (x.with_base) { x.base = r.arr<ulong>(); } else { r.val(); x.base = 0; }
    x.words = r.arr<ulong>();
    return x;
}

struct Pred {
    device const uint* slope; device const ulong* base; PFor res;
    ulong get(uint i) const { uint b = i >> 8; return base[b] + ((ulong(i & 255) * slope[b]) >> 16) + res.get(i); }
};
inline Pred rd_pred(thread Rd& r) { Pred x; x.slope = r.arr<uint>(); x.base = r.arr<ulong>(); x.res = rd_pfor(r); return x; }

// The tables: total (256 i8), fwd and bwd (256 x 9), select8 (256 x 8).
struct Tab { device const uchar* t; };
inline int  t_total(Tab t, uint b) { return int(char(t.t[b])); }
inline uint t_fwd(Tab t, uint b, uint k) { return t.t[256 + b * 9 + k]; }
inline uint t_bwd(Tab t, uint b, uint k) { return t.t[256 + 2304 + b * 9 + k]; }
inline uint t_sel(Tab t, uint b, uint k) { return t.t[256 + 4608 + b * 8 + k]; }

inline uint select_in_word(ulong x, uint r, Tab t) {
    const ulong L8 = 0x0101010101010101ul, H8 = 0x8080808080808080ul;
    ulong s = x - ((x >> 1) & 0x5555555555555555ul);
    s = (s & 0x3333333333333333ul) + ((s >> 2) & 0x3333333333333333ul);
    s = (s + (s >> 4)) & 0x0f0f0f0f0f0f0f0ful;
    s *= L8;
    ulong geq = ((ulong(r) * L8 | H8) - s) & H8;
    uint place = popcount(geq) * 8;
    ulong rank = ulong(r) - (((s << 8) >> place) & 0xff);
    return place + t_sel(t, uint((x >> place) & 0xff), uint(rank));
}

struct Ef {
    uint l; Packed low; device const ulong* high; device const uint* samples;
    ulong get(uint i, Tab t) const {
        uint p = samples[i / 64];
        uint r = i % 64;
        uint w0 = p >> 6;
        ulong first = high[w0] & ~((1ul << (p & 63)) - 1ul);
        uint c0 = popcount(first), c1 = popcount(high[w0 + 1]), c2 = popcount(high[w0 + 2]), c3 = popcount(high[w0 + 3]);
        uint s1 = c0, s2 = c0 + c1, s3 = s2 + c2;
        uint w; ulong word;
        if (r < s3 + c3) {
            uint k = uint(r >= s1) + uint(r >= s2) + uint(r >= s3);
            w = w0 + k;
            word = k == 0 ? first : high[w0 + k];
            r -= k == 0 ? 0 : (k == 1 ? s1 : (k == 2 ? s2 : s3));
        } else {
            r -= s3 + c3;
            w = w0 + 4;
            word = high[w];
            for (;;) { uint c = popcount(word); if (r < c) break; r -= c; w++; word = high[w]; }
        }
        ulong pos = ulong(w) * 64 + select_in_word(word, r, t);
        return ((pos - i) << l) | low.get(i);
    }
};
inline Ef rd_ef(thread Rd& r) { Ef x; x.l = r.val(); x.low = rd_packed(r); x.high = r.arr<ulong>(); x.samples = r.arr<uint>(); return x; }

// ---- X ----
struct XC { uint n; Packed code; PFor size, back; };
inline XC rd_x(thread Rd& r) { XC x; x.n = r.val(); x.code = rd_packed(r); x.size = rd_pfor(r); x.back = rd_pfor(r); return x; }

// ---- XF: one slot a node, tag 0 an inner node's size, tag 1 the back distance of the node after a leaf ----
struct XFC {
    uint n; Packed code; PFor slot;
    uint end(uint i) const { ulong v = slot.get(i); ulong t = v & 1; return i + uint(((v >> 1) & (t - 1)) | t); }
    // parent + 1; 0: none.
    ulong parent1(uint i) const { if (i == 0) return 0; ulong v = slot.get(i - 1); ulong t = v & 1; return i - (((v >> 1) & (0 - t)) | (t ^ 1)) + 1; }
};
inline XFC rd_xf(thread Rd& r) { XFC x; x.n = r.val(); x.code = rd_packed(r); x.slot = rd_pfor(r); return x; }

// ---- B ----
struct BC {
    uint nbits; device const ulong* bits; device const uint* sb_abs; device const ulong* sb_rel;
    device const char* w_min; device const char* w_max; device const short* l2_min; device const short* l2_max;
    device const short* l2_total; uint nl2; uint n; Packed code;
    uint rank1(uint p) const {
        uint s = p >> 9, k = (p >> 6) & 7;
        ulong rel = sb_rel[s];
        uint within = k == 0 ? 0 : uint((rel >> (9 * (k - 1))) & 511);
        return sb_abs[s] + within + popcount(bits[p >> 6] & ((1ul << (p & 63)) - 1ul));
    }
    bool is_open(uint p) const { return p < nbits && ((bits[p >> 6] >> (p & 63)) & 1) == 1; }
    int word_total(uint w) const { int valid = int(min(nbits - w * 64, 64u)); return 2 * int(popcount(bits[w])) - valid; }
    uint findclose(uint p, Tab t) const {
        int need = 1;
        uint q = p + 1;
        while ((q & 7) != 0) { need += is_open(q) ? 1 : -1; if (need == 0) return q; q++; }
        for (;;) {
            while ((q & 63) != 0) {
                uint byte = uint((bits[q >> 6] >> (q & 63)) & 0xff);
                if (need <= 8) { uint f = t_fwd(t, byte, uint(need)); if (f != 8) return q + f; }
                need += t_total(t, byte);
                q += 8;
            }
            uint w = q >> 6;
            for (;;) {
                if ((w % 32) == 0 && w / 32 < nl2 && int(l2_min[w / 32]) > -need) { need += int(l2_total[w / 32]); w += 32; continue; }
                if (int(w_min[w]) <= -need) break;
                need += word_total(w);
                w++;
            }
            q = w * 64;
            uint byte = uint(bits[q >> 6] & 0xff);
            if (need <= 8) { uint f = t_fwd(t, byte, uint(need)); if (f != 8) return q + f; }
            need += t_total(t, byte);
            q += 8;
        }
    }
    // The enclosing open, or 0xffffffff.
    uint enclose(uint p, Tab t) const {
        int need = 1;
        if (p == 0) return 0xffffffffu;
        uint q = p;
        while ((q & 7) != 0) { q--; need -= is_open(q) ? 1 : -1; if (need == 0) return q; if (q == 0) return 0xffffffffu; }
        for (;;) {
            while ((q & 63) != 0) {
                uint byte = uint((bits[(q - 8) >> 6] >> ((q - 8) & 63)) & 0xff);
                if (need <= 8) { uint f = t_bwd(t, byte, uint(need)); if (f != 8) return q - 8 + f; }
                need -= t_total(t, byte);
                q -= 8;
                if (q == 0) return 0xffffffffu;
            }
            uint w = q >> 6;
            for (;;) {
                if (w == 0) return 0xffffffffu;
                if ((w % 32) == 0 && int(l2_max[w / 32 - 1]) < need) { need -= int(l2_total[w / 32 - 1]); w -= 32; continue; }
                if (int(w_max[w - 1]) >= need) break;
                need -= word_total(w - 1);
                w--;
            }
            q = w * 64;
            uint byte = uint((bits[(q - 8) >> 6] >> ((q - 8) & 63)) & 0xff);
            if (need <= 8) { uint f = t_bwd(t, byte, uint(need)); if (f != 8) return q - 8 + f; }
            need -= t_total(t, byte);
            q -= 8;
        }
    }
};
inline BC rd_b(thread Rd& r) {
    BC x;
    x.nbits = r.val(); x.bits = r.arr<ulong>(); x.sb_abs = r.arr<uint>(); x.sb_rel = r.arr<ulong>();
    x.w_min = r.arr<char>(); x.w_max = r.arr<char>(); x.l2_min = r.arr<short>(); x.l2_max = r.arr<short>();
    x.l2_total = r.arr<short>(); x.nl2 = r.val(); x.n = r.val(); x.code = rd_packed(r);
    return x;
}

// Questions: 0 code (in order), 1 code, 2 start, 3 end, 4 subtree end, 5 parent (+1; 0: none), 6 children.
#define ARGS device const uchar* buf [[buffer(0)]], constant uint* P [[buffer(1)]], device const uint* ids [[buffer(2)]], \
    device ulong* out [[buffer(3)]], constant uint& q [[buffer(4)]], constant uint& nq [[buffer(5)]], \
    device const uchar* tabs [[buffer(6)]], uint t [[thread_position_in_grid]]

kernel void x_forlen(ARGS) {
    if (t >= nq) return;
    Rd r = { buf, P, 0 };
    XC x = rd_x(r); PFor st = rd_pfor(r); PFor ln = rd_pfor(r);
    uint i = ids[t];
    ulong v = 0;
    switch (q) {
        case 0: case 1: v = x.code.get(i); break;
        case 2: v = st.get(i); break;
        case 3: v = st.get(i) + ln.get(i); break;
        case 4: v = i + x.size.get(i); break;
        case 5: { ulong b = x.back.get(i); v = b == 0 ? 0 : i - b + 1; break; }
        case 6: { uint pe = i + uint(x.size.get(i)); uint c = i + 1; uint k = 0; while (c < pe) { k++; c += uint(x.size.get(c)); } v = k; break; }
    }
    out[t] = v;
}

kernel void x_predgap(ARGS) {
    if (t >= nq) return;
    Rd r = { buf, P, 0 };
    XC x = rd_x(r); Pred st = rd_pred(r); PFor gap = rd_pfor(r);
    uint n = r.val(); ulong src = ulong(r.val()) | (ulong(r.val()) << 32);
    uint i = ids[t];
    ulong v = 0;
    switch (q) {
        case 0: case 1: v = x.code.get(i); break;
        case 2: v = st.get(i); break;
        case 3: { uint nx = i + uint(x.size.get(i)); ulong s = nx < n ? st.get(nx) : src; ulong z = gap.get(i); long d = long(z >> 1) ^ -long(z & 1); v = ulong(long(s) - d); break; }
        case 4: v = i + x.size.get(i); break;
        case 5: { ulong b = x.back.get(i); v = b == 0 ? 0 : i - b + 1; break; }
        case 6: { uint pe = i + uint(x.size.get(i)); uint c = i + 1; uint k = 0; while (c < pe) { k++; c += uint(x.size.get(c)); } v = k; break; }
    }
    out[t] = v;
}

kernel void xf_forlen(ARGS) {
    if (t >= nq) return;
    Rd r = { buf, P, 0 };
    XFC x = rd_xf(r); PFor st = rd_pfor(r); PFor ln = rd_pfor(r);
    uint i = ids[t];
    ulong v = 0;
    switch (q) {
        case 0: case 1: v = x.code.get(i); break;
        case 2: v = st.get(i); break;
        case 3: v = st.get(i) + ln.get(i); break;
        case 4: v = x.end(i); break;
        case 5: v = x.parent1(i); break;
        case 6: { uint pe = x.end(i); uint c = i + 1; uint k = 0; while (c < pe) { k++; c = x.end(c); } v = k; break; }
    }
    out[t] = v;
}

kernel void xf_predgap(ARGS) {
    if (t >= nq) return;
    Rd r = { buf, P, 0 };
    XFC x = rd_xf(r); Pred st = rd_pred(r); PFor gap = rd_pfor(r);
    uint n = r.val(); ulong src = ulong(r.val()) | (ulong(r.val()) << 32);
    uint i = ids[t];
    ulong v = 0;
    switch (q) {
        case 0: case 1: v = x.code.get(i); break;
        case 2: v = st.get(i); break;
        case 3: { uint nx = x.end(i); ulong s = nx < n ? st.get(nx) : src; ulong z = gap.get(i); long d = long(z >> 1) ^ -long(z & 1); v = ulong(long(s) - d); break; }
        case 4: v = x.end(i); break;
        case 5: v = x.parent1(i); break;
        case 6: { uint pe = x.end(i); uint c = i + 1; uint k = 0; while (c < pe) { k++; c = x.end(c); } v = k; break; }
    }
    out[t] = v;
}

kernel void b_forlen(ARGS) {
    if (t >= nq) return;
    Rd r = { buf, P, 0 };
    BC b = rd_b(r); PFor st = rd_pfor(r); PFor ln = rd_pfor(r);
    Tab tb = { tabs };
    uint p = ids[t];
    ulong v = 0;
    switch (q) {
        case 0: case 1: v = b.code.get(b.rank1(p)); break;
        case 2: v = st.get(b.rank1(p)); break;
        case 3: { uint k = b.rank1(p); v = st.get(k) + ln.get(k); break; }
        case 4: v = b.findclose(p, tb) + 1; break;
        case 5: { uint e = b.enclose(p, tb); v = e == 0xffffffffu ? 0 : ulong(e) + 1; break; }
        case 6: { uint c = p + 1; uint k = 0; while (b.is_open(c)) { k++; c = b.findclose(c, tb) + 1; } v = k; break; }
    }
    out[t] = v;
}

kernel void b_efef(ARGS) {
    if (t >= nq) return;
    Rd r = { buf, P, 0 };
    BC b = rd_b(r); Ef st = rd_ef(r); Ef en = rd_ef(r);
    Tab tb = { tabs };
    uint p = ids[t];
    ulong v = 0;
    switch (q) {
        case 0: case 1: v = b.code.get(b.rank1(p)); break;
        case 2: v = st.get(b.rank1(p), tb); break;
        case 3: { uint c = b.findclose(p, tb); v = en.get(c - b.rank1(c), tb); break; }
        case 4: v = b.findclose(p, tb) + 1; break;
        case 5: { uint e = b.enclose(p, tb); v = e == 0xffffffffu ? 0 : ulong(e) + 1; break; }
        case 6: { uint c = p + 1; uint k = 0; while (b.is_open(c)) { k++; c = b.findclose(c, tb) + 1; } v = k; break; }
    }
    out[t] = v;
}
// ---- F16 (E29): format 16 read in place, as stored (0156 §1–2) ----
inline ulong rd_v64(thread Rd& r) { ulong lo = r.val(); return lo | (ulong(r.val()) << 32); }

// A side list in parts of 1024 nodes (4 blocks of 256): each part the blocks' u16 ends, the offsets, the values.
struct PartsC {
    uint vw, blocks; device const uint* base; device const uchar* b;
    ulong get(uint i) const { uint vb; uint k = find(i, vb); return value(vb, k); }
    // Entry k's value, its block's values starting at byte vb.
    ulong value(uint vb, uint k) const { return extract((device const ulong*)b, ulong(vb + k * vw) * 8, vw * 8); }
    // The entry of node i (listed) or of the first listed node after it in its block; vb: the block's values.
    uint find(uint i, thread uint& vb) const {
        uint blk = i >> 8, p = blk >> 2, j = blk & 3;
        uint at = base[p];
        uint nsub = min(4u, blocks - p * 4);
        device const ulong* w = (device const ulong*)b;
        uint s = j == 0 ? 0 : uint(extract(w, ulong(at + 2 * (j - 1)) * 8, 16));
        uint e = uint(extract(w, ulong(at + 2 * j) * 8, 16));
        uint total = uint(extract(w, ulong(at + 2 * (nsub - 1)) * 8, 16));
        uint offs = at + 2 * nsub;
        uint key = i & 255;
        while (s < e) { uint m = (s + e) >> 1; if (uint(b[offs + m]) < key) s = m + 1; else e = m; }
        vb = offs + total;
        return s;
    }
};
inline PartsC rd_parts(thread Rd& r) { PartsC x; x.vw = r.val(); x.blocks = r.val(); x.base = r.arr<uint>(); x.b = r.arr<uchar>(); return x; }

// Codes: a byte below the cutoff is the number; past it, the prefix and the side list's low part.
struct R8 {
    uint cut, per; device const uchar* b; PartsC parts;
    uint get(uint i) const { uint v = b[i]; return v < cut ? v : cut + (v - cut) * per + uint(parts.get(i)); }
};
inline R8 rd_r8(thread Rd& r) { R8 x; x.cut = r.val(); x.per = r.val(); x.b = r.arr<uchar>(); x.parts = rd_parts(r); return x; }

// A whole-byte-width array; a value too wide is all ones, listed in its block's side list.
struct EscC {
    uint w; device const ulong* words; uint nf; uint fw[3]; ulong fm[3]; ulong low; PartsC parts;
    ulong field(uint i) const {
        switch (w) {
            case 8: return ((device const uchar*)words)[i];
            case 16: return ((device const ushort*)words)[i];
            case 32: return ((device const uint*)words)[i];
            default: return extract(words, ulong(i) * w, w);
        }
    }
    ulong get(uint i) const { ulong v = field(i); return v != mask(w) ? v : parts.get(i); }
    // The sum of from..to (one block, under K values): each word's fields folded into their sum, then each
    // escape's value in place of its all-ones field.
    ulong sum(uint from, uint to) const {
        uint left = to > from ? to - from : 0, per = 64 / w, f = from;
        ulong s = 0, any = 0;
        while (left > 0) {
            uint t = min(left, per), tw = t * w;
            ulong v = extract(words, ulong(f) * w, tw);
            ulong fmk = mask(tw), c = ~v & fmk;
            any |= ~(((c & low) + low) | c | low) & fmk;
            for (uint k = 0; k < nf; k++) v = (v & fm[k]) + ((v >> fw[k]) & fm[k]);
            s += v; f += t; left -= t;
        }
        if (any != 0) {
            ulong esc = mask(w);
            for (uint j = from; j < to; j++) if (field(j) == esc) s += parts.get(j) - esc;
        }
        return s;
    }
};
inline EscC rd_esc(thread Rd& r) {
    EscC x; x.w = r.val(); x.words = r.arr<ulong>(); x.nf = r.val();
    for (uint k = 0; k < 3; k++) { x.fw[k] = r.val(); x.fm[k] = rd_v64(r); }
    x.low = rd_v64(r); x.parts = rd_parts(r);
    return x;
}

struct F16C {
    uint n; R8 code; EscC slot, gap, len; Packed ck;
    // Subtree end: an inner node's slot is its size << 1; a leaf's is odd.
    uint end(uint i) const { ulong v = slot.get(i); return (v & 1) == 0 ? i + uint(v >> 1) : i + 1; }
    // Parent + 1 (0: none): the node before is the parent (tag 0), or a leaf naming the parent distance
    // (tag 3) or its previous sibling (tag 1); at most K − 1 sibling steps.
    ulong parent1(uint i) const {
        if (i == 0) return 0;
        uint j = i;
        for (;;) {
            ulong v = slot.get(j - 1);
            if ((v & 1) == 0) return j;
            if ((v & 2) != 0) return j - uint(v >> 2) + 1;
            j -= uint(v >> 2);
        }
    }
    // A checkpoint every 8 nodes, the gaps after it.
    ulong start(uint i) const { uint c = i >> 3; return ck.get(c) + gap.sum((c << 3) + 1, i + 1); }
};
inline F16C rd_f16(thread Rd& r) {
    F16C x; x.n = r.val(); x.code = rd_r8(r); x.slot = rd_esc(r); x.gap = rd_esc(r); x.len = rd_esc(r); x.ck = rd_packed(r);
    return x;
}

kernel void f16_q(ARGS) {
    if (t >= nq) return;
    Rd r = { buf, P, 0 };
    F16C x = rd_f16(r);
    uint i = ids[t];
    ulong v = 0;
    switch (q) {
        case 0: case 1: v = x.code.get(i); break;
        case 2: v = x.start(i); break;
        case 3: v = x.start(i) + x.len.get(i); break;
        case 4: v = x.end(i); break;
        case 5: v = x.parent1(i); break;
        case 6: { uint pe = x.end(i); uint c = i + 1; uint k = 0; while (c < pe) { k++; c = x.end(c); } v = k; break; }
    }
    out[t] = v;
}
"#;


pub const QUESTIONS: [&str; 7] = ["code, in order (a sweep)", "code at random", "start at random", "end at random", "subtree end at random", "parent at random", "children of a node, at random"];

/// The CPU reader's answers to every question for `ids` (the kernel's encoding: parent + 1, 0 for none).
pub fn answers<R: Reader>(r: &R, ids: &[usize], order: &[usize]) -> Vec<Vec<u64>> {
    let ans = |q: usize, id: usize| -> u64 {
        match q {
            0 | 1 => u64::from(r.code(id)),
            2 => r.start(id),
            3 => r.end(id),
            4 => r.subtree_end(id) as u64,
            5 => r.parent(id).map_or(0, |p| p as u64 + 1),
            _ => {
                let pe = r.subtree_end(id);
                let (mut c, mut k) = (r.first_child(id), 0u64);
                while let Some(x) = c {
                    k += 1;
                    c = r.next_sibling(x, pe);
                }
                k
            }
        }
    };
    (0..QUESTIONS.len()).map(|q| {
        let list: Vec<usize> = if q == 0 { ids.to_vec() } else { order.iter().map(|&p| ids[p]).collect() };
        list.iter().map(|&id| ans(q, id)).collect()
    }).collect()
}

pub struct GpuRow {
    pub bytes: usize,
    /// GPU ns a question (the best of three warm dispatches), per question.
    pub ns: Vec<f64>,
    pub correct: Vec<bool>,
}

/// Runs each question over every node (in order for the sweep, in `order` for the rest) with `kernel`.
pub fn run(kernel: &str, image: &GpuImage, ids: &[usize], order: &[usize], want: &[Vec<u64>]) -> Result<GpuRow, String> {
    let device = Device::system_default().ok_or("no Metal device")?;
    let lib = device.new_library_with_source(SOURCE, &metal::CompileOptions::new()).map_err(|e| e.to_string())?;
    let f = lib.get_function(kernel, None).map_err(|e| e.to_string())?;
    let pipe = device.new_compute_pipeline_state_with_function(&f).map_err(|e| e.to_string())?;
    let opts = MTLResourceOptions::StorageModeShared;
    let buf = |b: &[u8]| device.new_buffer_with_data(b.as_ptr().cast(), b.len().max(1) as u64, opts);
    let u32s = |v: &[u32]| -> Vec<u8> { v.iter().flat_map(|x| x.to_le_bytes()).collect() };
    let mut tabs = crate::inplace::byte_tables_bytes();
    tabs.extend(crate::inplace::select8_bytes());
    let (bimg, bpar, btab) = (buf(&image.bytes), buf(&u32s(&image.params)), buf(&tabs));
    let n = ids.len();
    let in_order: Vec<u32> = ids.iter().map(|&i| i as u32).collect();
    let random: Vec<u32> = order.iter().map(|&p| ids[p] as u32).collect();
    let (bseq, brand) = (buf(&u32s(&in_order)), buf(&u32s(&random)));
    let bout = device.new_buffer((n * 8).max(8) as u64, opts);
    let queue = device.new_command_queue();
    let (mut ns, mut correct) = (Vec::new(), Vec::new());
    for (q, want) in want.iter().enumerate() {
        let mut best = f64::MAX;
        for _ in 0..4 {
            let cmd = queue.new_command_buffer();
            let enc = cmd.new_compute_command_encoder();
            enc.set_compute_pipeline_state(&pipe);
            enc.set_buffer(0, Some(&bimg), 0);
            enc.set_buffer(1, Some(&bpar), 0);
            enc.set_buffer(2, Some(if q == 0 { &bseq } else { &brand }), 0);
            enc.set_buffer(3, Some(&bout), 0);
            let (qq, nq) = (q as u32, n as u32);
            enc.set_bytes(4, 4, (&qq as *const u32).cast());
            enc.set_bytes(5, 4, (&nq as *const u32).cast());
            enc.set_buffer(6, Some(&btab), 0);
            let w = pipe.thread_execution_width();
            enc.dispatch_threads(MTLSize::new(n as u64, 1, 1), MTLSize::new(w, 1, 1));
            enc.end_encoding();
            cmd.commit();
            cmd.wait_until_completed();
            best = best.min(crate::gpu::command_seconds(cmd));
        }
        ns.push(best * 1e9 / n as f64);
        // SAFETY: shared storage of n u64s, and the GPU is done with it.
        let got = unsafe { std::slice::from_raw_parts(bout.contents().cast::<u64>(), n) };
        correct.push(got == &want[..]);
    }
    Ok(GpuRow { bytes: image.bytes.len(), ns, correct })
}

/// The candidates with kernels, on one forest: (name, row).
pub fn all(tree: &crate::tree::Tree, codes: &crate::inplace::Codes, order: &[usize]) -> Result<Vec<(&'static str, GpuRow)>, String> {
    fn one<R: Reader>(name: &'static str, kernel: &str, r: &R, image: GpuImage, order: &[usize], n: usize) -> Result<(&'static str, GpuRow), String> {
        let ids: Vec<usize> = (0..n).map(|p| r.id_of(p)).collect();
        let want = answers(r, &ids, order);
        Ok((name, run(kernel, &image, &ids, order, &want)?))
    }
    let n = tree.len();
    let x = X::<ForLen>::new(tree, codes);
    let m = X::<PredGap>::new(tree, codes);
    let b = B::<ForLen>::new(tree, codes);
    let be = B::<EfEf>::new(tree, codes);
    let xf = XF::<ForLen>::new(tree, codes);
    let mf = XF::<PredGap>::new(tree, codes);
    // Format 16 (E29) on the same forest, its choices learned on it as one language; the kernel answers a
    // code with its number (a rule tests numbers), so the check maps the reader's codes to numbers.
    let cols = crate::inplace::Cols::of_tree(tree, codes);
    let lang = crate::f16::F16Lang::learn(codes, codes.table.len(), std::iter::once(&cols)).marked(0);
    let f = crate::f16::F16::new(&cols, &lang);
    drop(cols);
    eprintln!("F16 on the forest: widths (slot, gap, length, checkpoint) {:?} bits, cutoff {}, past the cutoff / escaped (codes, slots, gaps, lengths) {:?}", f.widths(), lang.cut, f.escapes());
    let f16 = {
        let ids: Vec<usize> = (0..n).collect();
        let mut want = answers(&f, &ids, order);
        for q in 0..2 {
            for v in want[q].iter_mut() {
                *v = u64::from(lang.num[*v as usize]);
            }
        }
        ("F16", run("f16_q", &f.gpu_image(), &ids, order, &want)?)
    };
    Ok(vec![
        one("X", "x_forlen", &x, x.gpu_image(), order, n)?,
        one("M", "x_predgap", &m, m.gpu_image(), order, n)?,
        one("B", "b_forlen", &b, b.gpu_image(), order, n)?,
        one("BE", "b_efef", &be, be.gpu_image(), order, n)?,
        one("XF", "xf_forlen", &xf, xf.gpu_image(), order, n)?,
        one("MF", "xf_predgap", &mf, mf.gpu_image(), order, n)?,
        f16,
    ])
}

#[allow(dead_code)]
fn _uses<P: Pos + Gpu>() {}
