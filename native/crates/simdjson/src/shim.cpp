// simdjson, and two ways to read a document with it:
//
// - `rerail_simdjson_structurals`: simdjson's DOM parser validates the document, and its structural
//   index is given back: the start of each structural byte (`{ } [ ] : ,`) and of each scalar (a
//   string's opening quote, a number's or a literal's first byte), in order.
// - `rerail_simdjson_v9np`: a stage 2 of ours. simdjson's stage 1 finds the index; simdjson's own
//   stage 2 walker (`json_iterator::walk_document`, which checks the document's structure) drives a
//   visitor that validates each scalar as simdjson's tape builder does (escapes, numbers, atoms) and
//   writes the engine's v9np tape at each node's preorder slot, into the tape's own buffers (the
//   caller's, grown through `grow`) at their stored widths, with the tape's code table. No DOM
//   tape, no string buffer, no number values, nothing written twice.
//
// The amalgamation is compiled in this translation unit: stage 2's walker and its scalar checks are
// internal to it.
#include "simdjson.h"
#include "simdjson.cpp"

#include <cstring>
#include <memory>
#include <string>

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
  // `size`, `off` and `tail`'s widths: 2, or 4 once a value needs it.
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
};

#if SIMDJSON_IMPLEMENTATION_ARM64
namespace simdjson {
namespace arm64 {
namespace {
namespace stage2 {

// A node's kind, by slot, and its field: as `rerail_simdjson`'s writer numbers them.
enum : uint32_t { DOCUMENT = 0, PAIR = 1, CONTENT = 2, ESCAPE = 3, OBJECT = 4, ARRAY = 5, STRING = 6, NUMBER = 7, TRUE_ = 8, FALSE_ = 9, NULL_ = 10, SLOTS = 11 };
enum : uint32_t { NO_FIELD = 0, KEY = 1, VALUE = 2 };
constexpr uint32_t STRIDE = 16;

// A number checked, its value not kept.
struct no_values {
  simdjson_inline void append_s64(int64_t) noexcept {}
  simdjson_inline void append_u64(uint64_t) noexcept {}
  simdjson_inline void append_double(double) noexcept {}
};


// The tape's buffers, in the sink's order.
enum : uint32_t { CODE = 0, SIZE = 1, OFF = 2, TAIL = 3, BASE = 4 };

// The bytes of `c` for `nodes` nodes, `size`, `off` and `tail` at `width`.
simdjson_inline size_t column_bytes(uint32_t c, uint32_t nodes, const uint32_t* width) {
  return c == CODE ? size_t(nodes) * 2 : c == BASE ? size_t((nodes + STRIDE - 1) / STRIDE) * 4 : size_t(nodes) * width[c - 1];
}

struct columns {
  uint8_t* col[5];
};

// The buffers made to hold `nodes` nodes at `width`, their first `n` (at `old`) kept. Out of line, and
// given values, not the writer: the writer's state stays in registers.
simdjson_never_inline columns grow_columns(rerail_v9np_sink* sink, uint32_t n, uint32_t nodes, uint32_t o0, uint32_t o1, uint32_t o2, uint32_t w0, uint32_t w1,
                                           uint32_t w2) noexcept {
  const uint32_t old[3] = {o0, o1, o2}, width[3] = {w0, w1, w2};
  size_t need[5];
  for (uint32_t c = 0; c < 5; c++) {
    sink->len[c] = column_bytes(c, n, old);
    need[c] = column_bytes(c, nodes, width);
  }
  sink->grow(sink, need);
  columns out;
  for (uint32_t c = 0; c < 5; c++) { out.col[c] = sink->col[c]; }
  return out;
}

// The visitor: each callback writes the columns of the nodes it starts or ends, at their slots in
// the tape's buffers. `SZ`, `OF` and `TL` are `size`, `off` and `tail`'s cells (`uint16_t` or
// `uint32_t`), stored unchecked; each column's values are or-ed together, and a 2-byte column that
// took a larger one is the caller's to walk again 4 bytes wide.
template <typename SZ, typename OF, typename TL>
struct v9np_writer {
  rerail_v9np_sink* sink;
  uint32_t shift;  // a byte order mark's length: simdjson's offsets start after it
  uint32_t len;    // the file's length, with the mark
  uint16_t* code;
  uint32_t* base;
  SZ* size;
  OF* off;
  TL* tail;
  uint32_t room = 0;  // the nodes the buffers hold
  uint32_t n = 0;
  uint32_t high_size = 0, high_off = 0, high_tail = 0;  // each column's values, or-ed
  uint16_t ids[SLOTS * 3];
  uint32_t block_base = 0, last_start = 0;
  // By depth: the open container's ord, an object's open pair, whether it is an object.
  uint32_t* container;
  uint32_t* pair;
  bool* object;
  uint32_t d = 0;
  // A value's field here: `value` in an object, none in an array or at the top.
  uint32_t vf = NO_FIELD;
  // The low half of a surrogate pair already checked with its high half.
  const uint8_t* low_checked = nullptr;

  static constexpr uint32_t W0 = sizeof(SZ), W1 = sizeof(OF), W2 = sizeof(TL);
  simdjson_inline void reserve(uint32_t nodes) noexcept {
    const columns c = grow_columns(sink, n, nodes, W0, W1, W2, W0, W1, W2);
    code = reinterpret_cast<uint16_t*>(c.col[CODE]);
    base = reinterpret_cast<uint32_t*>(c.col[BASE]);
    size = reinterpret_cast<SZ*>(c.col[SIZE]);
    off = reinterpret_cast<OF*>(c.col[OFF]);
    tail = reinterpret_cast<TL*>(c.col[TAIL]);
    room = nodes;
  }

  simdjson_inline uint32_t open(uint32_t slot, uint32_t field, uint32_t start) noexcept {
    if (simdjson_unlikely(n == room)) { reserve(room + room / 2 + 64); }
    const uint32_t key = slot * 3 + field;
    uint16_t id = ids[key];
    if (simdjson_unlikely(id == 0xffff)) {
      id = uint16_t(sink->codes++);
      ids[key] = id;
      sink->code_kind[id] = sink->kind[slot];
      sink->code_field[id] = field == KEY ? sink->key_field : field == VALUE ? sink->value_field : 0;
      // A value's place: the kinds a value can be, unless keyed.
      sink->code_value[id] = slot >= OBJECT && field != KEY;
    }
    code[n] = id;
    if ((n % STRIDE) == 0) {
      block_base = start;
      base[n / STRIDE] = start;
    }
    const uint32_t o = start - block_base;
    high_off |= o;
    off[n] = OF(o);
    last_start = start;
    return n++;
  }
  // A node with children ended.
  simdjson_inline void close(uint32_t ord, uint32_t end) noexcept {
    const uint32_t t = end - last_start, z = n - ord;
    high_tail |= t;
    high_size |= z;
    tail[ord] = TL(t);
    size[ord] = SZ(z);
  }
  // A leaf: its size is 1.
  simdjson_inline void leaf(uint32_t slot, uint32_t field, uint32_t start, uint32_t end) noexcept {
    const uint32_t ord = open(slot, field, start);
    const uint32_t t = end - start;
    high_tail |= t;
    tail[ord] = TL(t);
    size[ord] = SZ(1);
  }
  // A value's field: an object's member is its pair's `value`.
  simdjson_inline uint32_t value_field() const noexcept { return vf; }
  // A value ended at `end`: the pair it is the value of ends there too.
  simdjson_inline void value_end(uint32_t end) noexcept {
    if (vf == VALUE) { close(pair[d], end); }
  }
  simdjson_inline uint32_t at(json_iterator& iter, const uint8_t* p) const noexcept { return uint32_t(p - iter.buf) + shift; }
  // The offset of the structural `back` entries before the iterator's next.
  simdjson_inline uint32_t structural(json_iterator& iter, int back) const noexcept { return iter.next_structural[-back] + shift; }

  simdjson_warn_unused simdjson_inline error_code visit_document_start(json_iterator& iter) noexcept {
    open(DOCUMENT, NO_FIELD, iter.next_structural[0] + shift);
    return SUCCESS;
  }
  simdjson_warn_unused simdjson_inline error_code visit_document_end(json_iterator&) noexcept {
    close(0, len);
    return SUCCESS;
  }
  simdjson_inline void start_container(json_iterator& iter, uint32_t slot, bool is_object) noexcept {
    const uint32_t ord = open(slot, value_field(), structural(iter, 1));
    d++;
    container[d] = ord;
    object[d] = is_object;
    vf = is_object ? VALUE : NO_FIELD;
  }
  simdjson_inline void end_container(json_iterator& iter) noexcept {
    const uint32_t end = structural(iter, 1) + 1;
    close(container[d], end);
    d--;
    vf = object[d] ? VALUE : NO_FIELD;
    value_end(end);
  }
  simdjson_inline void empty_container(json_iterator& iter, uint32_t slot) noexcept {
    const uint32_t end = structural(iter, 1) + 1;
    leaf(slot, value_field(), structural(iter, 2), end);
    value_end(end);
  }
  simdjson_warn_unused simdjson_inline error_code visit_object_start(json_iterator& iter) noexcept { start_container(iter, OBJECT, true); return SUCCESS; }
  simdjson_warn_unused simdjson_inline error_code visit_array_start(json_iterator& iter) noexcept { start_container(iter, ARRAY, false); return SUCCESS; }
  simdjson_warn_unused simdjson_inline error_code visit_object_end(json_iterator& iter) noexcept { end_container(iter); return SUCCESS; }
  simdjson_warn_unused simdjson_inline error_code visit_array_end(json_iterator& iter) noexcept { end_container(iter); return SUCCESS; }
  simdjson_warn_unused simdjson_inline error_code visit_empty_object(json_iterator& iter) noexcept { empty_container(iter, OBJECT); return SUCCESS; }
  simdjson_warn_unused simdjson_inline error_code visit_empty_array(json_iterator& iter) noexcept { empty_container(iter, ARRAY); return SUCCESS; }
  simdjson_warn_unused simdjson_inline error_code increment_count(json_iterator&) noexcept { return SUCCESS; }

  simdjson_warn_unused simdjson_inline error_code visit_key(json_iterator& iter, const uint8_t* key) noexcept {
    pair[d] = open(PAIR, NO_FIELD, at(iter, key));
    return string(iter, key, KEY);
  }
  simdjson_warn_unused simdjson_inline error_code visit_primitive(json_iterator& iter, const uint8_t* value) noexcept { return iter.visit_primitive(*this, value); }
  simdjson_warn_unused simdjson_inline error_code visit_root_primitive(json_iterator& iter, const uint8_t* value) noexcept { return iter.visit_root_primitive(*this, value); }

  // A string: its contents' runs and its escapes (`\` and the byte after it; a `\u`'s digits are
  // content), each escape checked as simdjson's `parse_string` checks it, without unescaping.
  simdjson_warn_unused simdjson_inline error_code string(json_iterator& iter, const uint8_t* value, uint32_t field) noexcept {
    const uint32_t ord = open(STRING, field, at(iter, value));
    const uint8_t* p = value + 1;
    const uint8_t* from = p;
    while (true) {
      // 32 bytes at a time: the padding is past the closing quote.
      simd::simd8<uint8_t> v0(p);
      simd::simd8<uint8_t> v1(p + 16);
      const uint64_t bits = simd::simd8x64<bool>(v0 == '\\', v1 == '\\', v0 == '"', v1 == '"').to_bitmask();
      const uint32_t bs = uint32_t(bits), quote = uint32_t(bits >> 32);
      if (((bs - 1) & quote) != 0) {
        const uint8_t* q = p + __builtin_ctz(quote);
        if (q > from) { leaf(CONTENT, NO_FIELD, at(iter, from), at(iter, q)); }
        const uint32_t end = at(iter, q) + 1;
        close(ord, end);
        if (field != KEY) { value_end(end); }
        return SUCCESS;
      }
      if (bs == 0) {
        p += 32;
        continue;
      }
      const uint8_t* e = p + __builtin_ctz(bs);
      const uint8_t c = e[1];
      if (c == 'u') {
        if (e == low_checked) {
          low_checked = nullptr;
        } else {
          const uint32_t cp = jsoncharutils::hex_to_u32_nocheck(e + 2);
          if (cp >= 0xd800 && cp < 0xdc00) {
            if (e[6] != '\\' || e[7] != 'u' || ((jsoncharutils::hex_to_u32_nocheck(e + 8) - 0xdc00) >> 10) != 0) { return STRING_ERROR; }
            low_checked = e + 6;
          } else if (cp > 0xffff || (cp >= 0xdc00 && cp <= 0xdfff)) {
            return STRING_ERROR;  // not four hex digits, or a lone low surrogate
          }
        }
      } else if (stringparsing::escape_map[c] == 0) {
        return STRING_ERROR;
      }
      if (e > from) { leaf(CONTENT, NO_FIELD, at(iter, from), at(iter, e)); }
      leaf(ESCAPE, NO_FIELD, at(iter, e), at(iter, e) + 2);
      from = p = e + 2;
    }
  }
  simdjson_warn_unused simdjson_inline error_code visit_string(json_iterator& iter, const uint8_t* value, bool key = false) noexcept {
    return string(iter, value, key ? KEY : value_field());
  }
  simdjson_warn_unused simdjson_inline error_code visit_root_string(json_iterator& iter, const uint8_t* value) noexcept { return string(iter, value, NO_FIELD); }

  simdjson_inline void scalar(json_iterator& iter, uint32_t slot, const uint8_t* value, uint32_t length) noexcept {
    const uint32_t start = at(iter, value);
    leaf(slot, value_field(), start, start + length);
    value_end(start + length);
  }
  // A scalar's end: the next structural's offset, less the blanks before it.
  simdjson_inline uint32_t scalar_length(json_iterator& iter, const uint8_t* value) const noexcept {
    const uint8_t* q = iter.buf + iter.next_structural[0];
    while (q > value + 1 && (q[-1] == ' ' || q[-1] == '\n' || q[-1] == '\r' || q[-1] == '\t')) { q--; }
    return uint32_t(q - value);
  }
  simdjson_warn_unused simdjson_inline error_code visit_number(json_iterator& iter, const uint8_t* value) noexcept {
    no_values w;
    SIMDJSON_TRY(numberparsing::parse_number(value, w));
    scalar(iter, NUMBER, value, scalar_length(iter, value));
    return SUCCESS;
  }
  // The document a number alone: checked from a copy followed by spaces, as simdjson's tape builder does.
  simdjson_warn_unused simdjson_inline error_code visit_root_number(json_iterator& iter, const uint8_t* value) noexcept {
    const size_t rest = iter.remaining_len();
    std::unique_ptr<uint8_t[]> copy(new (std::nothrow) uint8_t[rest + SIMDJSON_PADDING]);
    if (copy.get() == nullptr) { return MEMALLOC; }
    std::memcpy(copy.get(), value, rest);
    std::memset(copy.get() + rest, ' ', SIMDJSON_PADDING);
    no_values w;
    SIMDJSON_TRY(numberparsing::parse_number(copy.get(), w));
    scalar(iter, NUMBER, value, scalar_length(iter, value));
    return SUCCESS;
  }
  simdjson_warn_unused simdjson_inline error_code visit_true_atom(json_iterator& iter, const uint8_t* value) noexcept {
    if (!atomparsing::is_valid_true_atom(value)) { return T_ATOM_ERROR; }
    scalar(iter, TRUE_, value, 4);
    return SUCCESS;
  }
  simdjson_warn_unused simdjson_inline error_code visit_false_atom(json_iterator& iter, const uint8_t* value) noexcept {
    if (!atomparsing::is_valid_false_atom(value)) { return F_ATOM_ERROR; }
    scalar(iter, FALSE_, value, 5);
    return SUCCESS;
  }
  simdjson_warn_unused simdjson_inline error_code visit_null_atom(json_iterator& iter, const uint8_t* value) noexcept {
    if (!atomparsing::is_valid_null_atom(value)) { return N_ATOM_ERROR; }
    scalar(iter, NULL_, value, 4);
    return SUCCESS;
  }
  simdjson_warn_unused simdjson_inline error_code visit_root_true_atom(json_iterator& iter, const uint8_t* value) noexcept {
    if (!atomparsing::is_valid_true_atom(value, iter.remaining_len())) { return T_ATOM_ERROR; }
    scalar(iter, TRUE_, value, 4);
    return SUCCESS;
  }
  simdjson_warn_unused simdjson_inline error_code visit_root_false_atom(json_iterator& iter, const uint8_t* value) noexcept {
    if (!atomparsing::is_valid_false_atom(value, iter.remaining_len())) { return F_ATOM_ERROR; }
    scalar(iter, FALSE_, value, 5);
    return SUCCESS;
  }
  simdjson_warn_unused simdjson_inline error_code visit_root_null_atom(json_iterator& iter, const uint8_t* value) noexcept {
    if (!atomparsing::is_valid_null_atom(value, iter.remaining_len())) { return N_ATOM_ERROR; }
    scalar(iter, NULL_, value, 4);
    return SUCCESS;
  }
};

}  // namespace stage2
}  // namespace
}  // namespace arm64
}  // namespace simdjson

namespace {
// A thread's v9np stage 2: stage 1's buffers (no DOM document) and the depth stacks.
struct V9np {
  std::unique_ptr<simdjson::internal::dom_parser_implementation> stage1;
  std::unique_ptr<char[]> buffer;
  size_t buffer_capacity = 0;
  std::unique_ptr<uint32_t[]> stacks;  // container and pair, by depth
  std::unique_ptr<bool[]> objects;
};
thread_local V9np v9np;
// Walks made again at other widths, for a measurement.
thread_local uint64_t rewalks = 0;

// One walk at the given widths: 0, or a column's bit (1 size, 2 off, 4 tail) for each 2-byte column
// that took a larger value, or simdjson's error code negated.
template <typename SZ, typename OF, typename TL>
int walk(simdjson::arm64::dom_parser_implementation& p, rerail_v9np_sink* sink, uint32_t shift, uint32_t len) {
  using namespace simdjson::arm64::stage2;
  V9np& s = v9np;
  v9np_writer<SZ, OF, TL> w;
  w.sink = sink;
  w.shift = shift;
  w.len = len;
  sink->codes = 0;
  std::memset(w.ids, 0xff, sizeof(w.ids));
  w.container = s.stacks.get();
  w.pair = s.stacks.get() + simdjson::DEFAULT_MAX_DEPTH + 1;
  w.object = s.objects.get();
  w.object[0] = false;
  // About a node a structural, and half again for pairs, contents and escapes.
  const uint32_t structurals = p.n_structural_indexes;
  w.reserve(structurals + structurals / 2 + 16);
  json_iterator iter(p, 0);
  const simdjson::error_code error = iter.walk_document<false, false>(w);
  if (error) { return -int(error); }
  const uint32_t widths[3] = {w.W0, w.W1, w.W2};
  for (uint32_t c = 0; c < 5; c++) { sink->len[c] = column_bytes(c, w.n, widths); }
  for (int c = 0; c < 3; c++) { sink->width[c] = uint8_t(widths[c]); }
  return (w.W0 == 2 && w.high_size > 0xffff ? 1 : 0) | (w.W1 == 2 && w.high_off > 0xffff ? 2 : 0) | (w.W2 == 2 && w.high_tail > 0xffff ? 4 : 0) |
         // A 4-byte column whose values fit 2: walked again narrower.
         (w.W0 == 4 && w.high_size <= 0xffff ? 8 : 0);
}

// A walk at the widths `wide` names (bit 1 size, 2 off, 4 tail: 4 bytes).
inline int walk_at(uint32_t wide, simdjson::arm64::dom_parser_implementation& p, rerail_v9np_sink* sink, uint32_t shift, uint32_t len) {
  switch (wide) {
    case 0: return walk<uint16_t, uint16_t, uint16_t>(p, sink, shift, len);
    case 1: return walk<uint32_t, uint16_t, uint16_t>(p, sink, shift, len);
    case 2: return walk<uint16_t, uint32_t, uint16_t>(p, sink, shift, len);
    case 3: return walk<uint32_t, uint32_t, uint16_t>(p, sink, shift, len);
    case 4: return walk<uint16_t, uint16_t, uint32_t>(p, sink, shift, len);
    case 5: return walk<uint32_t, uint16_t, uint32_t>(p, sink, shift, len);
    case 6: return walk<uint16_t, uint32_t, uint32_t>(p, sink, shift, len);
    default: return walk<uint32_t, uint32_t, uint32_t>(p, sink, shift, len);
  }
}
}  // namespace
#endif

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

// 0 and the tape written to `sink`, simdjson's error code, or -1 where this build has no v9np stage
// 2 (not arm64).
int rerail_simdjson_v9np(const char* src, size_t len, rerail_v9np_sink* sink) {
#if SIMDJSON_IMPLEMENTATION_ARM64
  using namespace simdjson::arm64::stage2;
  static const bool arm64 = simdjson::get_active_implementation()->name() == "arm64";
  if (!arm64 || len >= 0xffffffffu / 2) { return -1; }
  V9np& s = v9np;
  const uint8_t* buf = padded(s.buffer, s.buffer_capacity, src, len);
  const uint32_t shift = (len >= 3 && std::memcmp(buf, "\xEF\xBB\xBF", 3) == 0) ? 3 : 0;
  const size_t body = len - shift;
  if (!s.stage1 || s.stage1->capacity() < body) {
    s.stage1.reset();
    auto error = simdjson::get_active_implementation()->create_dom_parser_implementation(body < 64 ? 64 : body, simdjson::DEFAULT_MAX_DEPTH, s.stage1);
    if (error) { return int(error); }
  }
  if (!s.stacks) {
    s.stacks.reset(new uint32_t[2 * (simdjson::DEFAULT_MAX_DEPTH + 1)]);
    s.objects.reset(new bool[simdjson::DEFAULT_MAX_DEPTH + 1]);
  }
  auto error = s.stage1->stage1(buf + shift, body, simdjson::stage1_mode::regular);
  if (error) { return int(error); }
  auto& p = static_cast<simdjson::arm64::dom_parser_implementation&>(*s.stage1);
  // `size` 4 bytes wide where the nodes, about one and a quarter a structural, pass 2 bytes; `off`
  // and `tail` 2. A prediction a walk shows wrong is walked again at the widths it showed (rare: a
  // node count near 65,536, or a node 65,536 bytes from its block's start or its last descendant).
  uint32_t wide = p.n_structural_indexes > 52000 ? 1 : 0;
  for (int attempt = 0; attempt < 3; attempt++) {
    const int r = walk_at(wide, p, sink, shift, uint32_t(len));
    if (r < 0) { return -r; }
    if (r == 0) { return 0; }
    rewalks++;
    wide = (wide | (r & 7)) & ~((r & 8) ? 1u : 0u);
  }
  return int(simdjson::UNEXPECTED_ERROR);
#else
  (void)src;
  (void)len;
  (void)sink;
  return -1;
#endif
}

// This thread's walks made again at other widths.
uint64_t rerail_simdjson_v9np_rewalks() {
#if SIMDJSON_IMPLEMENTATION_ARM64
  return rewalks;
#else
  return 0;
#endif
}

// Stage 1 alone (a measurement's): the structural index's length, or simdjson's error code negated.
int64_t rerail_simdjson_stage1(const char* src, size_t len) {
#if SIMDJSON_IMPLEMENTATION_ARM64
  V9np& s = v9np;
  const uint8_t* buf = padded(s.buffer, s.buffer_capacity, src, len);
  if (!s.stage1 || s.stage1->capacity() < len) {
    s.stage1.reset();
    auto error = simdjson::get_active_implementation()->create_dom_parser_implementation(len < 64 ? 64 : len, simdjson::DEFAULT_MAX_DEPTH, s.stage1);
    if (error) { return -int64_t(error); }
  }
  auto error = s.stage1->stage1(buf, len, simdjson::stage1_mode::regular);
  return error ? -int64_t(error) : int64_t(s.stage1->n_structural_indexes);
#else
  (void)src;
  (void)len;
  return -1;
#endif
}

// Frees this thread's parsers and buffers when they hold more than `keep` bytes of source.
void rerail_simdjson_release_above(size_t keep) {
  State& s = state;
  if (s.capacity > keep + simdjson::SIMDJSON_PADDING || s.parser.capacity() > keep) {
    s.parser = simdjson::dom::parser();
    s.buffer.reset();
    s.capacity = 0;
  }
#if SIMDJSON_IMPLEMENTATION_ARM64
  V9np& v = v9np;
  if (v.buffer_capacity > keep + simdjson::SIMDJSON_PADDING) {
    v.stage1.reset();
    v.buffer.reset();
    v.buffer_capacity = 0;
  }
#endif
}

const char* rerail_simdjson_error_message(int code) {
  return simdjson::error_message(simdjson::error_code(code));
}

const char* rerail_simdjson_implementation() {
  static const std::string name = simdjson::get_active_implementation()->name();
  return name.c_str();
}
}
