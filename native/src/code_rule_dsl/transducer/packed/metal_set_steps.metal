#include <metal_stdlib>
using namespace metal;

// A wave's node sets on the device (decision 0024 §1): slots of `words` 64-bit words over the
// wave's node space in one buffer, a step a dispatch (metal_set_steps.rs).
struct StepArgs { uint op, out, a, b, words, len, files, pad; };

// Mask of bits [low, high) of a word, 0 <= low <= high <= 64.
static ulong bits_between(uint low, uint high) {
  ulong upper = high >= 64u ? ~0ul : ((1ul << high) - 1ul);
  ulong lower = low >= 64u ? 0ul : (~0ul << low);
  return upper & lower;
}

// One word of a slot: 0 clear, 1 every node of the space, 2 copy, 3 or, 4 and, 5 and-not.
kernel void set_word(device ulong* values [[buffer(0)]], constant StepArgs& s [[buffer(1)]], uint w [[thread_position_in_grid]]) {
  if (w >= s.words) return;
  device ulong* out = values + ulong(s.out) * s.words;
  device const ulong* a = values + ulong(s.a) * s.words;
  device const ulong* b = values + ulong(s.b) * s.words;
  ulong value;
  switch (s.op) {
    case 0u: value = 0ul; break;
    case 1u: { uint start = w * 64u; value = start >= s.len ? 0ul : bits_between(0u, min(64u, s.len - start)); } break;
    case 2u: value = a[w]; break;
    case 3u: value = a[w] | b[w]; break;
    case 4u: value = a[w] & b[w]; break;
    case 5u: value = a[w] & ~b[w]; break;
    default: value = out[w]; break;
  }
  out[w] = value;
}

// Whether slot `a` has a node in each file: `flags[file]`.
kernel void set_file_any(device const ulong* values [[buffer(0)]], constant StepArgs& s [[buffer(1)]], device const uint2* files [[buffer(2)]], device uint* flags [[buffer(3)]], uint file [[thread_position_in_grid]]) {
  if (file >= s.files) return;
  device const ulong* test = values + ulong(s.a) * s.words;
  uint node = files[file].x, end = files[file].x + files[file].y, any = 0u;
  while (node < end && any == 0u) {
    uint word = node / 64u, upto = min(end, (word + 1u) * 64u);
    if ((test[word] & bits_between(node - word * 64u, upto - word * 64u)) != 0ul) any = 1u;
    node = upto;
  }
  flags[file] = any;
}

// Slot `out`'s nodes kept only in files flagged by `set_file_any`.
kernel void set_file_keep(device ulong* values [[buffer(0)]], constant StepArgs& s [[buffer(1)]], device const uint2* files [[buffer(2)]], device const uint* flags [[buffer(3)]], uint w [[thread_position_in_grid]]) {
  if (w >= s.words) return;
  device ulong* out = values + ulong(s.out) * s.words;
  ulong value = out[w];
  if (value == 0ul) return;
  uint first = w * 64u, last = min(first + 64u, s.len);
  // The last file starting at or before the word's first node.
  uint low = 0u, high = s.files;
  while (high - low > 1u) { uint middle = (low + high) / 2u; if (files[middle].x <= first) low = middle; else high = middle; }
  ulong keep = 0ul;
  for (uint file = low; file < s.files && files[file].x < last; file++) {
    if (flags[file] == 0u) continue;
    uint start = max(files[file].x, first), end = min(files[file].x + files[file].y, last);
    if (start < end) keep |= bits_between(start - first, end - first);
  }
  out[w] = value & keep;
}
