// simdjson, and two ways to read a document with it:
//
// - `rerail_simdjson_structurals`: simdjson's DOM parser validates the document, and its structural
//   index is given back: the start of each structural byte (`{ } [ ] : ,`) and of each scalar (a
//   string's opening quote, a number's or a literal's first byte), in order. Any CPU: simdjson picks
//   its implementation at run time.
// - `rerail_simdjson_v9np`: the engine's tape in one pass of stage 1 and one of a stage 2 of ours, by a
//   kernel (`kernel.inc`) compiled for each of simdjson's implementations this target has, each with
//   the CPU's SHA-256 instructions and without them:
//   - stage 1 is simdjson's (its scanner, UTF-8 checker and indexer), each 64-byte block also
//     hashed: the source's digest comes out with the index;
//   - stage 2 walks the index itself, its state in locals, checking the document as simdjson's
//     stage 2 does (structure, depth, escapes, numbers, atoms), and writes the v9np tape at each
//     node's preorder slot into the tape's own buffers (the caller's), at their stored widths, with
//     the tape's code table. No DOM tape, no string buffer, no number values, nothing copied after.
//
// Kernels by number (`rerail_simdjson_native::Kernel`), best first within a target:
//   0 arm64, SHA-256 instructions    1 arm64
//   2 icelake, SHA-NI                3 icelake
//   4 haswell, SHA-NI                5 haswell
//   6 westmere, SHA-NI               7 westmere
//
// The amalgamation is compiled in this translation unit: stage 1's parts and the scalar checks are
// internal to it.
#include "simdjson.h"
#include "simdjson.cpp"

// A lambda inlined where it is called.
#define simdjson_inline_lambda __attribute__((always_inline))

#include <array>
#include <cstring>
#include <memory>
#include <string>
#if SIMDJSON_IMPLEMENTATION_ARM64
#include <arm_neon.h>
#if defined(__linux__)
#include <sys/auxv.h>
#endif
#endif
#if SIMDJSON_IS_X86_64
#include <cpuid.h>
#include <immintrin.h>
#endif

#define RERAIL_STRINGIFY_(x) #x
#define RERAIL_STRINGIFY(x) RERAIL_STRINGIFY_(x)
// A region of functions compiled for the instructions `T` names (outside any namespace).
#if defined(__clang__)
#define RERAIL_TARGET_REGION(T) _Pragma(RERAIL_STRINGIFY(clang attribute push(__attribute__((target(T))), apply_to = function)))
#define RERAIL_UNTARGET_REGION _Pragma("clang attribute pop")
#else
#define RERAIL_TARGET_REGION(T) _Pragma("GCC push_options") _Pragma(RERAIL_STRINGIFY(GCC target(T)))
#define RERAIL_UNTARGET_REGION _Pragma("GCC pop_options")
#endif

namespace {
struct State {
  simdjson::dom::parser parser;
  // The source, copied with simdjson's padding after it.
  std::unique_ptr<char[]> buffer;
  size_t capacity = 0;
};
thread_local State state;

// The source, copied with simdjson's padding (zeros) after it.
const uint8_t* padded(std::unique_ptr<char[]>& buffer, size_t& capacity, const char* src, size_t len) {
  if (capacity < len + simdjson::SIMDJSON_PADDING) {
    capacity = len + simdjson::SIMDJSON_PADDING;
    buffer.reset(new char[capacity]);
  }
  std::memcpy(buffer.get(), src, len);
  std::memset(buffer.get() + len, 0, simdjson::SIMDJSON_PADDING);
  return reinterpret_cast<const uint8_t*>(buffer.get());
}
}  // namespace

// The tape's buffers, as the caller owns them: `code`, `size`, `off`, `tail` and `base`, each with
// its bytes used (`len`) and allocated (`cap`). `grow` makes each buffer hold `need[i]` bytes, keeping
// its first `len[i]`, and sets `col` and `cap` again.
struct rerail_v9np_sink {
  uint8_t* col[5];
  size_t len[5];
  size_t cap[5];
  // `size`, `off` and `tail`'s widths: 2 or 4.
  uint8_t width[3];
  void* ctx;
  void (*grow)(rerail_v9np_sink* sink, const size_t* need);
  // The kind of each slot (document, pair, string_content, escape_sequence, object, array, string,
  // number, true, false, null), and the `key` and `value` fields.
  uint16_t kind[11];
  uint16_t key_field;
  uint16_t value_field;
  // The tape's codes, in first-use order: kind, field, and whether in a value's place.
  uint32_t codes;
  uint16_t code_kind[33];
  uint16_t code_field[33];
  uint8_t code_value[33];
  // The source's SHA-256.
  uint8_t digest[32];
};

// --- SHA-256 ------------------------------------------------------------------------------------------
//
// Three of the same shape: `block(p)` hashes a 64-byte block, `words(w)` gives the state. The CPU's
// instructions (ARMv8's SHA2, x86's SHA-NI), or portable code where a CPU has neither.

namespace {

const uint32_t SHA_K[64] = {
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be,
    0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa,
    0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85,
    0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3,
    0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f,
    0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2};
const uint32_t SHA_H[8] = {0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19};

// A number's value, not kept (`kernel.inc`'s `number_end`): outside the kernels' target regions, so
// simdjson's `write_float`, compiled for its implementation's instructions alone, inlines it.
struct no_values {
  simdjson_inline void append_s64(int64_t) noexcept {}
  simdjson_inline void append_u64(uint64_t) noexcept {}
  simdjson_inline void append_double(double) noexcept {}
};

// Portable: FIPS 180-4's rounds, unrolled, the schedule a ring of 16 words, the eight working variables
// renamed from round to round instead of moved.
struct sha256_portable {
  uint32_t h[8];
  sha256_portable() { std::memcpy(h, SHA_H, sizeof(h)); }
  static simdjson_inline uint32_t rotr(uint32_t x, int n) { return (x >> n) | (x << (32 - n)); }
  simdjson_inline void block(const uint8_t* p) {
    uint32_t w[16];
    std::memcpy(w, p, sizeof(w));
    for (int i = 0; i < 16; i++) { w[i] = __builtin_bswap32(w[i]); }
    uint32_t a = h[0], b = h[1], c = h[2], d = h[3], e = h[4], f = h[5], g = h[6], k = h[7];
#define RERAIL_SCHEDULE(i)                                                                                  \
  (w[(i) & 15] += (rotr(w[((i) - 2) & 15], 17) ^ rotr(w[((i) - 2) & 15], 19) ^ (w[((i) - 2) & 15] >> 10)) + \
                  w[((i) - 7) & 15] + (rotr(w[((i) - 15) & 15], 7) ^ rotr(w[((i) - 15) & 15], 18) ^ (w[((i) - 15) & 15] >> 3)))
#define RERAIL_ROUND(a, b, c, d, e, f, g, h, i, W)                                                    \
  {                                                                                                   \
    const uint32_t t1 = h + (rotr(e, 6) ^ rotr(e, 11) ^ rotr(e, 25)) + (g ^ (e & (f ^ g))) + SHA_K[i] + (W); \
    d += t1;                                                                                          \
    h = t1 + (rotr(a, 2) ^ rotr(a, 13) ^ rotr(a, 22)) + ((a & b) | (c & (a | b)));                    \
  }
#define RERAIL_EIGHT(i, W)                  \
  RERAIL_ROUND(a, b, c, d, e, f, g, k, i + 0, W(i + 0)) \
  RERAIL_ROUND(k, a, b, c, d, e, f, g, i + 1, W(i + 1)) \
  RERAIL_ROUND(g, k, a, b, c, d, e, f, i + 2, W(i + 2)) \
  RERAIL_ROUND(f, g, k, a, b, c, d, e, i + 3, W(i + 3)) \
  RERAIL_ROUND(e, f, g, k, a, b, c, d, i + 4, W(i + 4)) \
  RERAIL_ROUND(d, e, f, g, k, a, b, c, i + 5, W(i + 5)) \
  RERAIL_ROUND(c, d, e, f, g, k, a, b, i + 6, W(i + 6)) \
  RERAIL_ROUND(b, c, d, e, f, g, k, a, i + 7, W(i + 7))
#define RERAIL_LOADED(i) w[(i) & 15]
    RERAIL_EIGHT(0, RERAIL_LOADED)
    RERAIL_EIGHT(8, RERAIL_LOADED)
    for (int i = 16; i < 64; i += 16) {
      RERAIL_EIGHT(i, RERAIL_SCHEDULE)
      RERAIL_EIGHT(i + 8, RERAIL_SCHEDULE)
    }
#undef RERAIL_LOADED
#undef RERAIL_EIGHT
#undef RERAIL_ROUND
#undef RERAIL_SCHEDULE
    h[0] += a, h[1] += b, h[2] += c, h[3] += d, h[4] += e, h[5] += f, h[6] += g, h[7] += k;
  }
  void words(uint32_t* out) const { std::memcpy(out, h, sizeof(h)); }
};

#if SIMDJSON_IMPLEMENTATION_ARM64
#if defined(__clang__)
#define RERAIL_ARM_SHA "sha2"
#else
#define RERAIL_ARM_SHA "+sha2"
#endif
// ARMv8's SHA2 instructions: four rounds an instruction pair, the schedule four words at a time.
struct sha256_arm {
  uint32x4_t abcd, efgh;
  sha256_arm() {
    abcd = vld1q_u32(SHA_H);
    efgh = vld1q_u32(SHA_H + 4);
  }
  __attribute__((target(RERAIL_ARM_SHA))) simdjson_inline void block(const uint8_t* p) {
    uint32x4_t m0 = vreinterpretq_u32_u8(vrev32q_u8(vld1q_u8(p)));
    uint32x4_t m1 = vreinterpretq_u32_u8(vrev32q_u8(vld1q_u8(p + 16)));
    uint32x4_t m2 = vreinterpretq_u32_u8(vrev32q_u8(vld1q_u8(p + 32)));
    uint32x4_t m3 = vreinterpretq_u32_u8(vrev32q_u8(vld1q_u8(p + 48)));
    uint32x4_t s0 = abcd, s1 = efgh;
#define RERAIL_SHA_ROUNDS(M, I)                                    \
  {                                                                \
    const uint32x4_t k = vaddq_u32(M, vld1q_u32(SHA_K + 4 * (I))); \
    const uint32x4_t before = s0;                                  \
    s0 = vsha256hq_u32(s0, s1, k);                                 \
    s1 = vsha256h2q_u32(s1, before, k);                            \
  }
#define RERAIL_SHA_SCHEDULE(A, B, C, D) A = vsha256su1q_u32(vsha256su0q_u32(A, B), C, D)
    for (int i = 0; i < 12; i += 4) {
      RERAIL_SHA_ROUNDS(m0, i);
      RERAIL_SHA_SCHEDULE(m0, m1, m2, m3);
      RERAIL_SHA_ROUNDS(m1, i + 1);
      RERAIL_SHA_SCHEDULE(m1, m2, m3, m0);
      RERAIL_SHA_ROUNDS(m2, i + 2);
      RERAIL_SHA_SCHEDULE(m2, m3, m0, m1);
      RERAIL_SHA_ROUNDS(m3, i + 3);
      RERAIL_SHA_SCHEDULE(m3, m0, m1, m2);
    }
    RERAIL_SHA_ROUNDS(m0, 12);
    RERAIL_SHA_ROUNDS(m1, 13);
    RERAIL_SHA_ROUNDS(m2, 14);
    RERAIL_SHA_ROUNDS(m3, 15);
#undef RERAIL_SHA_ROUNDS
#undef RERAIL_SHA_SCHEDULE
    abcd = vaddq_u32(s0, abcd);
    efgh = vaddq_u32(s1, efgh);
  }
  void words(uint32_t* out) const {
    vst1q_u32(out, abcd);
    vst1q_u32(out + 4, efgh);
  }
};

// Whether this CPU has the SHA2 instructions: every Apple CPU; elsewhere as the kernel says.
bool cpu_has_sha() {
#if defined(__APPLE__)
  return true;
#elif defined(__linux__)
  return (getauxval(AT_HWCAP) & (1ul << 6)) != 0;  // HWCAP_SHA2
#else
  return false;
#endif
}
#endif

#if SIMDJSON_IS_X86_64
#define RERAIL_NI_SHA "sha,sse4.1"
// x86's SHA-NI: two rounds an instruction, the state as ABEF and CDGH, the schedule four words at a
// time (`sha256msg1`, `sha256msg2`).
struct sha256_ni {
  __m128i abef, cdgh;
  __attribute__((target(RERAIL_NI_SHA))) sha256_ni() {
    const __m128i cdab = _mm_shuffle_epi32(_mm_loadu_si128(reinterpret_cast<const __m128i*>(SHA_H)), 0xB1);
    const __m128i efgh = _mm_shuffle_epi32(_mm_loadu_si128(reinterpret_cast<const __m128i*>(SHA_H + 4)), 0x1B);
    abef = _mm_alignr_epi8(cdab, efgh, 8);
    cdgh = _mm_blend_epi16(efgh, cdab, 0xF0);
  }
  __attribute__((target(RERAIL_NI_SHA))) simdjson_inline void block(const uint8_t* p) {
    const __m128i swap = _mm_set_epi64x(0x0c0d0e0f08090a0bULL, 0x0405060700010203ULL);
    __m128i m[4];
    for (int i = 0; i < 4; i++) { m[i] = _mm_shuffle_epi8(_mm_loadu_si128(reinterpret_cast<const __m128i*>(p + 16 * i)), swap); }
    __m128i s0 = abef, s1 = cdgh;
    for (int g = 0; g < 16; g++) {
      __m128i& w = m[g % 4];
      if (g >= 4) {
        // W[i..i+3] from W[i-16..i-1]: m[g % 4] holds the words 16 back, m[(g + 3) % 4] the last four.
        const __m128i prev = m[(g + 3) % 4];
        w = _mm_sha256msg2_epu32(_mm_add_epi32(_mm_sha256msg1_epu32(w, m[(g + 1) % 4]), _mm_alignr_epi8(prev, m[(g + 2) % 4], 4)), prev);
      }
      __m128i k = _mm_add_epi32(w, _mm_loadu_si128(reinterpret_cast<const __m128i*>(SHA_K + 4 * g)));
      s1 = _mm_sha256rnds2_epu32(s1, s0, k);
      k = _mm_shuffle_epi32(k, 0x0E);
      s0 = _mm_sha256rnds2_epu32(s0, s1, k);
    }
    abef = _mm_add_epi32(s0, abef);
    cdgh = _mm_add_epi32(s1, cdgh);
  }
  __attribute__((target(RERAIL_NI_SHA))) void words(uint32_t* out) const {
    const __m128i feba = _mm_shuffle_epi32(abef, 0x1B);
    const __m128i dchg = _mm_shuffle_epi32(cdgh, 0xB1);
    _mm_storeu_si128(reinterpret_cast<__m128i*>(out), _mm_blend_epi16(feba, dchg, 0xF0));      // DCBA
    _mm_storeu_si128(reinterpret_cast<__m128i*>(out + 4), _mm_alignr_epi8(dchg, feba, 8));  // HGFE
  }
};

// Whether this CPU has SHA-NI (CPUID leaf 7, EBX bit 29).
bool cpu_has_sha() {
  unsigned a = 0, b = 0, c = 0, d = 0;
  return __get_cpuid_count(7, 0, &a, &b, &c, &d) && (b & (1u << 29)) != 0;
}
#endif

}  // namespace

// --- The kernels: `kernel.inc` in each implementation ------------------------------------------------

#if SIMDJSON_IMPLEMENTATION_ARM64
RERAIL_TARGET_REGION(RERAIL_ARM_SHA)
namespace simdjson {
namespace arm64 {
namespace {
#define RERAIL_IMPLEMENTATION "arm64"
#include "kernel.inc"
#undef RERAIL_IMPLEMENTATION
}  // namespace
}  // namespace arm64
}  // namespace simdjson
RERAIL_UNTARGET_REGION
#endif

#if SIMDJSON_IMPLEMENTATION_ICELAKE
RERAIL_TARGET_REGION("avx512f,avx512dq,avx512cd,avx512bw,avx512vbmi,avx512vbmi2,avx512vl,avx2,bmi,bmi2,pclmul,lzcnt,popcnt," RERAIL_NI_SHA)
namespace simdjson {
namespace icelake {
namespace {
#define RERAIL_IMPLEMENTATION "icelake"
#include "kernel.inc"
#undef RERAIL_IMPLEMENTATION
}  // namespace
}  // namespace icelake
}  // namespace simdjson
RERAIL_UNTARGET_REGION
#endif

#if SIMDJSON_IMPLEMENTATION_HASWELL
RERAIL_TARGET_REGION("avx2,bmi,bmi2,pclmul,lzcnt,popcnt," RERAIL_NI_SHA)
namespace simdjson {
namespace haswell {
namespace {
#define RERAIL_IMPLEMENTATION "haswell"
#include "kernel.inc"
#undef RERAIL_IMPLEMENTATION
}  // namespace
}  // namespace haswell
}  // namespace simdjson
RERAIL_UNTARGET_REGION
#endif

#if SIMDJSON_IMPLEMENTATION_WESTMERE
RERAIL_TARGET_REGION("sse4.2,pclmul,popcnt," RERAIL_NI_SHA)
namespace simdjson {
namespace westmere {
namespace {
#define RERAIL_IMPLEMENTATION "westmere"
#include "kernel.inc"
#undef RERAIL_IMPLEMENTATION
}  // namespace
}  // namespace westmere
}  // namespace simdjson
RERAIL_UNTARGET_REGION
#endif

namespace {
// A kernel: its implementation, whether it hashes with the CPU's instructions, and its entry points.
struct Kernel {
  const char* implementation;
  bool sha;
  int (*tape)(const char*, size_t, rerail_v9np_sink*);
  int64_t (*stage1)(const char*, size_t, int);
  void (*release_above)(size_t);
  uint64_t (*rewalks)();
};
constexpr uint32_t KERNELS = 8;

#define RERAIL_KERNEL(ns, H, with_sha) \
  Kernel { #ns, with_sha, &simdjson::ns::rerail_v9np::tape<H>, &simdjson::ns::rerail_v9np::stage1_only<H>, &simdjson::ns::rerail_v9np::release_above, &simdjson::ns::rerail_v9np::walked_again }
const Kernel* kernel(uint32_t k) {
  static const Kernel table[KERNELS] = {
#if SIMDJSON_IMPLEMENTATION_ARM64
      RERAIL_KERNEL(arm64, sha256_arm, true), RERAIL_KERNEL(arm64, sha256_portable, false),
#else
      Kernel{}, Kernel{},
#endif
#if SIMDJSON_IMPLEMENTATION_ICELAKE
      RERAIL_KERNEL(icelake, sha256_ni, true), RERAIL_KERNEL(icelake, sha256_portable, false),
#else
      Kernel{}, Kernel{},
#endif
#if SIMDJSON_IMPLEMENTATION_HASWELL
      RERAIL_KERNEL(haswell, sha256_ni, true), RERAIL_KERNEL(haswell, sha256_portable, false),
#else
      Kernel{}, Kernel{},
#endif
#if SIMDJSON_IMPLEMENTATION_WESTMERE
      RERAIL_KERNEL(westmere, sha256_ni, true), RERAIL_KERNEL(westmere, sha256_portable, false),
#else
      Kernel{}, Kernel{},
#endif
  };
  return k < KERNELS && table[k].tape != nullptr ? &table[k] : nullptr;
}
#undef RERAIL_KERNEL

// Whether kernel `k` is in this build and runs on this CPU: simdjson's check of its implementation's
// instructions, and the SHA-256 instructions where it hashes with them.
bool runs(uint32_t k) {
  static const auto supported = [] {
    std::array<bool, KERNELS> ok{};
    for (uint32_t i = 0; i < KERNELS; i++) {
      const Kernel* kern = kernel(i);
      if (kern == nullptr) { continue; }
      const simdjson::implementation* impl = simdjson::get_available_implementations()[kern->implementation];
      ok[i] = impl != nullptr && impl->supported_by_runtime_system() && (!kern->sha || cpu_has_sha());
    }
    return ok;
  }();
  return k < KERNELS && supported[k];
}
}  // namespace

extern "C" {

// 0 and the index (valid until this thread's next call), or simdjson's error code.
int rerail_simdjson_structurals(const char* src, size_t len, const uint32_t** indexes, uint32_t* count) {
  State& s = state;
  const uint8_t* buf = padded(s.buffer, s.capacity, src, len);
  simdjson::dom::element root;
  auto error = s.parser.parse(buf, len, false).get(root);
  if (error) {
    return int(error);
  }
  *indexes = s.parser.implementation->structural_indexes.get();
  *count = s.parser.implementation->n_structural_indexes;
  return 0;
}

// Whether kernel `k` is in this build and runs on this CPU.
int rerail_simdjson_kernel_runs(uint32_t k) { return runs(k) ? 1 : 0; }

// 0 and the tape and the source's digest written to `sink` by kernel `k`, simdjson's error code, or -1
// where the kernel does not run here.
int rerail_simdjson_v9np(uint32_t k, const char* src, size_t len, rerail_v9np_sink* sink) {
  if (!runs(k) || len >= 0xffffffffu / 2) { return -1; }
  return kernel(k)->tape(src, len, sink);
}

// Kernel `k`'s walks made again at other widths, on this thread.
uint64_t rerail_simdjson_v9np_rewalks(uint32_t k) { return runs(k) ? kernel(k)->rewalks() : 0; }

// Stage 1 alone (a measurement's), kernel `k`'s implementation's or, `hashed`, ours with the digest:
// the structural index's length, or simdjson's error code negated; -1 where the kernel does not run.
int64_t rerail_simdjson_stage1(uint32_t k, const char* src, size_t len, int hashed) {
  if (!runs(k)) { return -1; }
  return kernel(k)->stage1(src, len, hashed);
}

// Frees this thread's parsers and buffers when they hold more than `keep` bytes of source.
void rerail_simdjson_release_above(size_t keep) {
  State& s = state;
  if (s.capacity > keep + simdjson::SIMDJSON_PADDING || s.parser.capacity() > keep) {
    s.parser = simdjson::dom::parser();
    s.buffer.reset();
    s.capacity = 0;
  }
  for (uint32_t k = 0; k < KERNELS; k += 2) {
    if (const Kernel* kern = kernel(k)) { kern->release_above(keep); }
  }
}

const char* rerail_simdjson_error_message(int code) {
  return simdjson::error_message(simdjson::error_code(code));
}

// The implementation simdjson's DOM parser chose for this CPU.
const char* rerail_simdjson_implementation() {
  static const std::string name = simdjson::get_active_implementation()->name();
  return name.c_str();
}
}
