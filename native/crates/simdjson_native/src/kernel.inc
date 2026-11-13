// The v9np kernel: simdjson's stage 1 with the source hashed in it, and our stage 2, which writes the
// tape. Written once, included by `shim.cpp` inside each of simdjson's implementations
// (`simdjson::<implementation>::{anonymous}`), in a target region with that implementation's
// instructions and the CPU's SHA-256 ones: its names (`simd`, `stage1`, `numberparsing`, …) are that
// implementation's. `RERAIL_IMPLEMENTATION` is its name.
//
// The kernel is the same in every implementation but for the vectors' width (`backslash_quote`); a
// kernel's SHA-256 is a template parameter (`H`: the CPU's instructions, or portable code).

namespace stage1 {

// The last `rest` bytes at `p` (fewer than 64), the padding and the length; the digest.
template <typename H>
void sha_finish(H& sha, const uint8_t* p, size_t rest, uint64_t total, uint8_t* out) {
  uint8_t last[128] = {0};
  std::memcpy(last, p, rest);
  last[rest] = 0x80;
  const size_t blocks = rest < 56 ? 1 : 2;
  const uint64_t bits = total * 8;
  for (int i = 0; i < 8; i++) { last[blocks * 64 - 1 - i] = uint8_t(bits >> (8 * i)); }
  for (size_t b = 0; b < blocks; b++) { sha.block(last + 64 * b); }
  uint32_t words[8];
  sha.words(words);
  for (int i = 0; i < 8; i++) {
    for (int j = 0; j < 4; j++) { out[4 * i + j] = uint8_t(words[i] >> (24 - 8 * j)); }
  }
}

// simdjson's stage 1 (`json_structural_indexer::index<64>`, the whole-document mode), each block of
// the source also hashed. `buf` and `len` are the document after a byte order mark; `src` and `total`
// the whole source, which is hashed.
//
// With `C` (JSONC), a `/` outside a string starts a comment: `//` to the line's end (its `\n` not
// included), `/*` to the next `*/`. Each is put in `comments` (its start and end in `buf`) and blanked
// in `buf` (the kernel's own copy), and its block scanned again from the state the block began in, so
// a quote in a comment starts no string. A `/` that starts no comment, or a `/*` never closed, is a
// `TAPE_ERROR`. Without `C`, `buf` is only read.
template <typename H, bool C>
simdjson_warn_unused error_code index_and_hash(dom_parser_implementation& parser, const uint8_t* buf, size_t len, const uint8_t* src, size_t total,
                                               uint8_t* digest, std::vector<uint32_t>& comments) noexcept {
  H sha;
  size_t hashed = 0;
  auto hash_rest = [&] {
    while (hashed + 64 <= total) {
      sha.block(src + hashed);
      hashed += 64;
    }
    sha_finish(sha, src + hashed, total - hashed, total, digest);
  };
  if (simdjson_unlikely(len > parser.capacity())) { return CAPACITY; }
  parser.buf = buf;
  parser.len = len;
  if (len == 0) {
    hash_rest();
    return EMPTY;
  }
  buf_block_reader<64> reader(buf, len);
  json_scanner scanner{};
  utf8_checker checker{};
  bit_indexer indexer(parser.structural_indexes.get());
  uint64_t prev_structurals = 0;
  uint64_t unescaped_chars_error = 0;
  uint8_t last[64];
  bool unreadable = false;  // C: a `/` that starts no comment
  // C: the comment at `at`, recorded and blanked; false where none starts there.
  auto blank = [&](size_t at) -> bool {
    size_t end = 0;
    if (buf[at + 1] == '/') {
      const void* nl = std::memchr(buf + at + 2, '\n', len - (at + 2));
      end = nl ? size_t(static_cast<const uint8_t*>(nl) - buf) : len;
    } else if (buf[at + 1] == '*') {
      // Each `*` after the opener, until one is followed by `/`.
      for (size_t i = at + 2; i < len;) {
        const void* star = std::memchr(buf + i, '*', len - i);
        if (star == nullptr) { break; }
        const size_t s = size_t(static_cast<const uint8_t*>(star) - buf);
        if (s + 1 < len && buf[s + 1] == '/') {
          end = s + 2;
          break;
        }
        i = s + 1;
      }
      if (end == 0) { return false; }
    } else {
      return false;
    }
    comments.push_back(uint32_t(at));
    comments.push_back(uint32_t(end));
    std::memset(const_cast<uint8_t*>(buf) + at, ' ', end - at);
    return true;
  };
  auto step = [&](const uint8_t* block) simdjson_inline_lambda {
    const json_scanner before = scanner;
    simd::simd8x64<uint8_t> in(block);
    json_block b = scanner.next(in);
    if constexpr (C) {
      const uint64_t slash = b.non_quote_outside_string(in.eq('/'));
      if (simdjson_unlikely(slash != 0) && !unreadable) {
        scanner = before;
        if (blank(reader.block_index() + size_t(__builtin_ctzll(slash)))) {
          if (block == last) { reader.get_remainder(last); }
        } else {
          unreadable = true;
        }
        return false;  // the block again
      }
    }
    (void)before;
    const uint64_t unescaped = in.lteq(0x1F);
    checker.check_next_input(in);
    indexer.write(uint32_t(reader.block_index() - 64), prev_structurals);
    prev_structurals = b.structural_start();
    unescaped_chars_error |= b.non_quote_inside_string(unescaped);
    reader.advance();
    // The block's bytes are in the cache: a block of the source is hashed with it.
    if (hashed + 64 <= total) {
      sha.block(src + hashed);
      hashed += 64;
    }
    return true;
  };
  while (reader.has_full_block()) {
    while (!step(reader.full_block())) {}
  }
  if (simdjson_unlikely(reader.get_remainder(last) == 0)) { return UNEXPECTED_ERROR; }
  while (!step(last)) {}
  hash_rest();

  indexer.write(uint32_t(reader.block_index() - 64), prev_structurals);
  const error_code error = scanner.finish();
  parser.n_structural_indexes = uint32_t(indexer.tail - parser.structural_indexes.get());
  parser.structural_indexes[parser.n_structural_indexes] = uint32_t(len);
  parser.structural_indexes[parser.n_structural_indexes + 1] = uint32_t(len);
  parser.structural_indexes[parser.n_structural_indexes + 2] = 0;
  parser.next_structural_index = 0;
  if (error != SUCCESS) { return error; }
  if (C && unreadable) { return TAPE_ERROR; }
  if (unescaped_chars_error) { return UNESCAPED_CHARS; }
  if (simdjson_unlikely(parser.n_structural_indexes == 0u)) { return EMPTY; }
  if (simdjson_unlikely(parser.structural_indexes[parser.n_structural_indexes - 1] > len)) { return UNEXPECTED_ERROR; }
  checker.check_eof();
  return checker.errors();
}

}  // namespace stage1

namespace stage2 {

// A node's kind, by slot, and its field: as `rerail_simdjson`'s writer numbers them.
enum : uint32_t { DOCUMENT = 0, PAIR = 1, CONTENT = 2, ESCAPE = 3, OBJECT = 4, ARRAY = 5, STRING = 6, NUMBER = 7, TRUE_ = 8, FALSE_ = 9, NULL_ = 10, COMMENT = 11, SLOTS = 12 };
enum : uint32_t { NO_FIELD = 0, KEY = 1, VALUE = 2 };
// The tape's buffers, in the sink's order.
enum : uint32_t { CODE = 0, SIZE = 1, OFF = 2, TAIL = 3, BASE = 4 };
constexpr uint32_t STRIDE = 16;

// A number checked as simdjson's `numberparsing::parse_number` checks it, its value not kept
// (`::no_values`); and where it ends.
simdjson_warn_unused simdjson_inline error_code number_end(const uint8_t* const src, const uint8_t*& end) noexcept {
  using namespace numberparsing;
  const bool negative = (*src == '-');
  const uint8_t* p = src + uint8_t(negative);
  const uint8_t* const start_digits = p;
  uint64_t i = 0;
  while (parse_digit(*p, i)) { p++; }
  size_t digit_count = size_t(p - start_digits);
  if (digit_count == 0 || ('0' == *start_digits && digit_count > 1)) { return NUMBER_ERROR; }
  int64_t exponent = 0;
  bool is_float = false;
  if ('.' == *p) {
    is_float = true;
    ++p;
    SIMDJSON_TRY(parse_decimal_after_separator(src, p, i, exponent));
    digit_count = int(p - start_digits);
  }
  if (('e' == *p) || ('E' == *p)) {
    is_float = true;
    ++p;
    SIMDJSON_TRY(parse_exponent(src, p, exponent));
  }
  end = p;
  if (is_float) {
    const bool dirty_end = jsoncharutils::is_not_structural_or_whitespace(*p);
    ::no_values w;
    SIMDJSON_TRY(write_float(src, negative, i, start_digits, digit_count, exponent, w));
    return dirty_end ? NUMBER_ERROR : SUCCESS;
  }
  const size_t longest_digit_count = negative ? 19 : 20;
  if (digit_count > longest_digit_count) { return BIGINT_ERROR; }
  if (digit_count == longest_digit_count) {
    if (negative) {
      if (i > uint64_t(INT64_MAX) + 1) { return BIGINT_ERROR; }
    } else if (src[0] != uint8_t('1') || i <= uint64_t(INT64_MAX)) {
      return BIGINT_ERROR;
    }
  }
  return jsoncharutils::is_not_structural_or_whitespace(*p) ? NUMBER_ERROR : SUCCESS;
}

// The 32 bytes at `p`: a bit for each `\\` (low half) and each `"` (high half), in the vectors this
// kernel has (`V`: 16, 32 or 64 bytes; `B`, their bits, a template's so that a branch not taken is not
// compiled).
template <typename V, typename B = simd::simd8x64<bool>>
simdjson_inline uint64_t backslash_quote(const uint8_t* p) {
  if constexpr (sizeof(V) == 16) {
    const V v0(p), v1(p + 16);
    return B(v0 == '\\', v1 == '\\', v0 == '"', v1 == '"').to_bitmask();
  } else if constexpr (sizeof(V) == 32) {
    const V v(p);
    return B(v == '\\', v == '"').to_bitmask();
  } else {
    const simd::simd8x64<uint8_t> in(p);
    return (in.eq('\\') & 0xffffffffu) | (in.eq('"') << 32);
  }
}

// The bytes of `c` for `nodes` nodes, `size`, `off` and `tail` at `width`.
simdjson_inline size_t column_bytes(uint32_t c, uint32_t nodes, const uint32_t* width) {
  return c == CODE ? size_t(nodes) * 2 : c == BASE ? size_t((nodes + STRIDE - 1) / STRIDE) * 4 : size_t(nodes) * width[c - 1];
}

// The tape's buffers made to hold `nodes` nodes at `width`, empty.
simdjson_never_inline void reserve_columns(rerail_v9np_sink* sink, uint32_t nodes, const uint32_t* width) noexcept {
  size_t need[5];
  for (uint32_t c = 0; c < 5; c++) {
    sink->len[c] = 0;
    need[c] = column_bytes(c, nodes, width);
  }
  sink->grow(sink, need);
}

// What a walk leaves: 0 and each 2-byte column's largest value fitting, or a column's bit (1 size,
// 2 off, 4 tail) where one did not, 8 where `size` is 4 bytes and its values fit 2; or simdjson's
// error code negated.
//
// The walk is simdjson's stage 2 (`json_iterator::walk_document`) with the tape builder's checks,
// each state a label, its state in locals. `SZ`, `OF` and `TL` are the widths of `size`, `off` and
// `tail`: each cell is stored unchecked, and each column's values or-ed together.
template <typename SZ, typename OF, typename TL, bool C>
simdjson_never_inline int walk(const uint8_t* const buf, const uint32_t* const index, const uint32_t count, const uint32_t shift, const uint32_t len,
                               rerail_v9np_sink* const sink, uint32_t* const container, uint32_t* const pair, bool* const object,
                               const uint32_t* const comments, const uint32_t comment_count) noexcept {
  const uint32_t widths[3] = {sizeof(SZ), sizeof(OF), sizeof(TL)};
  // A node starts at a byte of its own, but a pair (its key's quote) and the document (its value's
  // first byte); a pair takes four bytes or more. So the tape has at most this many nodes, and the
  // buffers are made for them once: no node checks for room.
  const uint32_t bound = len + len / 4 + 2;
  reserve_columns(sink, bound, widths);
  uint16_t* const code = reinterpret_cast<uint16_t*>(sink->col[CODE]);
  uint32_t* const base = reinterpret_cast<uint32_t*>(sink->col[BASE]);
  SZ* const size = reinterpret_cast<SZ*>(sink->col[SIZE]);
  OF* const off = reinterpret_cast<OF*>(sink->col[OFF]);
  TL* const tail = reinterpret_cast<TL*>(sink->col[TAIL]);
  uint32_t n = 0, block_base = 0, last_start = 0;
  uint32_t high_size = 0, high_off = 0, high_tail = 0;
  uint16_t ids[SLOTS * 3];
  std::memset(ids, 0xff, sizeof(ids));
  sink->codes = 0;

  // A node's code: numbered by first use.
  auto code_of = [&](uint32_t slot, uint32_t field) simdjson_inline_lambda -> uint16_t {
    const uint32_t key = slot * 3 + field;
    uint16_t id = ids[key];
    if (simdjson_unlikely(id == 0xffff)) {
      id = uint16_t(sink->codes++);
      ids[key] = id;
      sink->code_kind[id] = sink->kind[slot];
      sink->code_field[id] = field == KEY ? sink->key_field : field == VALUE ? sink->value_field : 0;
      // A value's place: the kinds a value can be, unless keyed.
      sink->code_value[id] = slot >= OBJECT && slot <= NULL_ && field != KEY;
    }
    return id;
  };
  // A node's start: its code, its offset from its block's base, the base of each block.
  auto open = [&](uint32_t slot, uint32_t field, uint32_t start) simdjson_inline_lambda -> uint32_t {
    code[n] = code_of(slot, field);
    if ((n % STRIDE) == 0) {
      block_base = start;
      base[n / STRIDE] = start;
    }
    const uint32_t o = start - block_base;
    high_off |= o;
    off[n] = OF(o);
    last_start = start;
    return n++;
  };
  // A node's end: its size and its tail (its end less its last descendant's start).
  auto close = [&](uint32_t ord, uint32_t end) simdjson_inline_lambda {
    const uint32_t t = end - last_start, z = n - ord;
    high_tail |= t;
    high_size |= z;
    tail[ord] = TL(t);
    size[ord] = SZ(z);
  };
  auto leaf = [&](uint32_t slot, uint32_t field, uint32_t start, uint32_t end) simdjson_inline_lambda {
    const uint32_t ord = open(slot, field, start);
    const uint32_t t = end - start;
    high_tail |= t;
    tail[ord] = TL(t);
    size[ord] = SZ(1);
  };
  auto at = [&](const uint8_t* p) simdjson_inline_lambda -> uint32_t { return uint32_t(p - buf) + shift; };

  // A string with escapes: its contents' runs and its escapes (`\` and the byte after it; a `\u`'s
  // digits are content), each escape checked as simdjson's `parse_string` checks it. Its end, or 0.
  auto escaped_string = [&](const uint8_t* v, uint32_t field) simdjson_inline_lambda -> uint32_t {
    const uint32_t ord = open(STRING, field, at(v));
    const uint8_t* p = v + 1;
    const uint8_t* from = p;
    const uint8_t* low_checked = nullptr;
    while (true) {
      const uint64_t bits = backslash_quote<simd::simd8<uint8_t>>(p);
      const uint32_t bs = uint32_t(bits), quote = uint32_t(bits >> 32);
      if (((bs - 1) & quote) != 0) {
        const uint8_t* q = p + __builtin_ctz(quote);
        if (q > from) { leaf(CONTENT, NO_FIELD, at(from), at(q)); }
        const uint32_t end = at(q) + 1;
        close(ord, end);
        return end;
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
            if (e[6] != '\\' || e[7] != 'u' || ((jsoncharutils::hex_to_u32_nocheck(e + 8) - 0xdc00) >> 10) != 0) { return 0; }
            low_checked = e + 6;
          } else if (cp > 0xffff || (cp >= 0xdc00 && cp <= 0xdfff)) {
            return 0;  // not four hex digits, or a lone low surrogate
          }
        }
      } else if (stringparsing::escape_map[c] == 0) {
        return 0;
      }
      if (e > from) { leaf(CONTENT, NO_FIELD, at(from), at(e)); }
      leaf(ESCAPE, NO_FIELD, at(e), at(e) + 2);
      from = p = e + 2;
    }
  };
  // A string: its end, or 0. One with no escape in its first 32 bytes, the most, is written as its
  // two nodes at once (the string and its content), or one where it is empty.
  auto string = [&](const uint8_t* v, uint32_t field) simdjson_inline_lambda -> uint32_t {
    const uint32_t s = at(v);
    const uint64_t bits = backslash_quote<simd::simd8<uint8_t>>(v + 1);
    const uint32_t bs = uint32_t(bits), quote = uint32_t(bits >> 32);
    if (simdjson_likely(((bs - 1) & quote) != 0)) {
      const uint32_t q = __builtin_ctz(quote);
      if (q == 0) {
        leaf(STRING, field, s, s + 2);
        return s + 2;
      }
      const uint32_t ord = open(STRING, field, s);
      const uint32_t content = open(CONTENT, NO_FIELD, s + 1);
      high_tail |= q + 1;
      tail[content] = TL(q);
      size[content] = SZ(1);
      tail[ord] = TL(q + 1);
      size[ord] = SZ(2);
      return s + q + 2;
    }
    return escaped_string(v, field);
  };
  // A scalar in an array or an object: its end, or 0 and `error`.
  auto primitive = [&](const uint8_t* v, uint32_t field, error_code& error) simdjson_inline_lambda -> uint32_t {
    const uint32_t s = at(v);
    if (*v == '"') {
      const uint32_t end = string(v, field);
      if (simdjson_unlikely(end == 0)) { error = STRING_ERROR; }
      return end;
    }
    // As simdjson's `visit_primitive`: a byte below `0` is a number's too, refused by its check.
    if (((*v - '0') < 10) || (*v == '-')) {
      const uint8_t* e = v;
      error = number_end(v, e);
      if (simdjson_unlikely(error != SUCCESS)) { return 0; }
      leaf(NUMBER, field, s, at(e));
      return at(e);
    }
    uint32_t slot, length;
    switch (*v) {
      case 't':
        if (!atomparsing::is_valid_true_atom(v)) { error = T_ATOM_ERROR; return 0; }
        slot = TRUE_, length = 4;
        break;
      case 'f':
        if (!atomparsing::is_valid_false_atom(v)) { error = F_ATOM_ERROR; return 0; }
        slot = FALSE_, length = 5;
        break;
      case 'n':
        if (!atomparsing::is_valid_null_atom(v)) { error = N_ATOM_ERROR; return 0; }
        slot = NULL_, length = 4;
        break;
      default:
        error = TAPE_ERROR;
        return 0;
    }
    leaf(slot, field, s, s + length);
    return s + length;
  };
  // The document a scalar alone: an atom checked against the bytes left, a number from a copy
  // followed by spaces (as simdjson's tape builder does: the padding is zeros).
  auto root_primitive = [&](const uint8_t* v) simdjson_inline_lambda -> error_code {
    const size_t rest = size_t(len - shift) - size_t(v - buf);
    const uint32_t s = at(v);
    switch (*v) {
      case '"': return string(v, NO_FIELD) ? SUCCESS : STRING_ERROR;
      case 't':
        if (!atomparsing::is_valid_true_atom(v, rest)) { return T_ATOM_ERROR; }
        leaf(TRUE_, NO_FIELD, s, s + 4);
        return SUCCESS;
      case 'f':
        if (!atomparsing::is_valid_false_atom(v, rest)) { return F_ATOM_ERROR; }
        leaf(FALSE_, NO_FIELD, s, s + 5);
        return SUCCESS;
      case 'n':
        if (!atomparsing::is_valid_null_atom(v, rest)) { return N_ATOM_ERROR; }
        leaf(NULL_, NO_FIELD, s, s + 4);
        return SUCCESS;
      default: {
        std::unique_ptr<uint8_t[]> copy(new (std::nothrow) uint8_t[rest + SIMDJSON_PADDING]);
        if (copy.get() == nullptr) { return MEMALLOC; }
        std::memcpy(copy.get(), v, rest);
        std::memset(copy.get() + rest, ' ', SIMDJSON_PADDING);
        const uint8_t* e = copy.get();
        SIMDJSON_TRY(number_end(copy.get(), e));
        leaf(NUMBER, NO_FIELD, s, s + uint32_t(e - copy.get()));
        return SUCCESS;
      }
    }
  };

  const uint32_t* next = index;
  uint32_t d = 0;      // the open containers
  uint32_t vf = NO_FIELD;  // a value's field here: `value` in an object
  uint32_t end;
  error_code error = SUCCESS;
  const uint8_t* v;
  object[0] = false;

  // A file's comments (`C`: `comments` holds each one's start and end, before the byte order mark's
  // shift, in order). Each is a leaf of the lowest node holding the tokens on either side of it, as
  // tree-sitter places an extra: written where the walk stands when it reads the token after it.
  uint32_t ci = 0;
  auto gap = [&](uint32_t before) simdjson_inline_lambda {
    if constexpr (C) {
      while (ci < comment_count && comments[2 * ci] < before) {
        leaf(COMMENT, NO_FIELD, comments[2 * ci] + shift, comments[2 * ci + 1] + shift);
        ci++;
      }
    }
  };
  // An empty object or array at `v`, its closer the next token: a leaf, or a node holding the comments
  // inside it.
  auto empty = [&](uint32_t slot, uint32_t field) simdjson_inline_lambda {
    const uint32_t start = at(v);
    end = *next + shift + 1;
    if (C && ci < comment_count && comments[2 * ci] < *next) {
      const uint32_t ord = open(slot, field, start);
      gap(*next);
      close(ord, end);
    } else {
      leaf(slot, field, start, end);
    }
    next++;
  };

  // The document starts at its first token, a comment's included.
  open(DOCUMENT, NO_FIELD, ((C && comment_count != 0 && comments[0] < index[0]) ? comments[0] : index[0]) + shift);
  gap(*next);
  v = buf + *next++;
  // The outer object or array closed at the document's end (simdjson's check).
  if ((*v == '{' && buf[index[count - 1]] != '}') || (*v == '[' && buf[index[count - 1]] != ']')) { return -int(TAPE_ERROR); }
  switch (*v) {
    case '{':
      if (buf[*next] == '}') {
        empty(OBJECT, NO_FIELD);
        goto document_end;
      }
      goto object_begin;
    case '[':
      if (buf[*next] == ']') {
        empty(ARRAY, NO_FIELD);
        goto document_end;
      }
      goto array_begin;
    default:
      error = root_primitive(v);
      if (error) { return -int(error); }
      goto document_end;
  }

object_begin:
  if (++d >= DEFAULT_MAX_DEPTH) { return -int(DEPTH_ERROR); }
  container[d] = open(OBJECT, vf, at(v));
  object[d] = true;
  vf = VALUE;
  gap(*next);
  v = buf + *next++;
  if (*v != '"') { return -int(TAPE_ERROR); }
object_key:
  pair[d] = open(PAIR, NO_FIELD, at(v));
  if (simdjson_unlikely(string(v, KEY) == 0)) { return -int(STRING_ERROR); }
  gap(*next);
  if (simdjson_unlikely(buf[*next++] != ':')) { return -int(TAPE_ERROR); }
  gap(*next);
  v = buf + *next++;
  switch (*v) {
    case '{':
      if (buf[*next] == '}') {
        empty(OBJECT, VALUE);
        close(pair[d], end);
        break;
      }
      goto object_begin;
    case '[':
      if (buf[*next] == ']') {
        empty(ARRAY, VALUE);
        close(pair[d], end);
        break;
      }
      goto array_begin;
    default:
      end = primitive(v, VALUE, error);
      if (simdjson_unlikely(end == 0)) { return -int(error); }
      close(pair[d], end);
  }
object_continue:
  gap(*next);
  switch (buf[*next++]) {
    case ',':
      gap(*next);
      v = buf + *next++;
      // JSONC (`C`): a trailing comma, passed over as TypeScript reads it.
      if (C && *v == '}') { goto scope_end; }
      if (simdjson_unlikely(*v != '"')) { return -int(TAPE_ERROR); }
      goto object_key;
    case '}': goto scope_end;
    default: return -int(TAPE_ERROR);
  }

array_begin:
  if (++d >= DEFAULT_MAX_DEPTH) { return -int(DEPTH_ERROR); }
  container[d] = open(ARRAY, vf, at(v));
  object[d] = false;
  vf = NO_FIELD;
array_value:
  gap(*next);
  v = buf + *next++;
  switch (*v) {
    case '{':
      if (buf[*next] == '}') {
        empty(OBJECT, NO_FIELD);
        break;
      }
      goto object_begin;
    case '[':
      if (buf[*next] == ']') {
        empty(ARRAY, NO_FIELD);
        break;
      }
      goto array_begin;
    default:
      if (simdjson_unlikely(primitive(v, NO_FIELD, error) == 0)) { return -int(error); }
  }
array_continue:
  gap(*next);
  switch (buf[*next++]) {
    case ',':
      // JSONC (`C`): a trailing comma, passed over as TypeScript reads it.
      if constexpr (C) {
        gap(*next);
        if (buf[*next] == ']') {
          next++;
          goto scope_end;
        }
      }
      goto array_value;
    case ']': goto scope_end;
    default: return -int(TAPE_ERROR);
  }

scope_end:
  // The container's closer is the structural just read; the pair it is the value of ends with it.
  end = next[-1] + shift + 1;
  close(container[d], end);
  d--;
  if (d == 0) { goto document_end; }
  if (object[d]) {
    vf = VALUE;
    close(pair[d], end);
    goto object_continue;
  }
  vf = NO_FIELD;
  goto array_continue;

document_end:
  // More than one value, or bytes after it.
  if (next != index + count) { return -int(TAPE_ERROR); }
  gap(UINT32_MAX);
  close(0, len);
  for (uint32_t c = 0; c < 5; c++) { sink->len[c] = column_bytes(c, n, widths); }
  for (int c = 0; c < 3; c++) { sink->width[c] = uint8_t(widths[c]); }
  return (sizeof(SZ) == 2 && high_size > 0xffff ? 1 : 0) | (sizeof(OF) == 2 && high_off > 0xffff ? 2 : 0) | (sizeof(TL) == 2 && high_tail > 0xffff ? 4 : 0) |
         (sizeof(SZ) == 4 && high_size <= 0xffff ? 8 : 0);
}

}  // namespace stage2

namespace rerail_v9np {

// A thread's kernel: stage 1's buffers (no DOM document), the padded source and the depth stacks.
struct Thread {
  std::unique_ptr<simdjson::internal::dom_parser_implementation> stage1;
  std::unique_ptr<char[]> buffer;
  size_t buffer_capacity = 0;
  std::unique_ptr<uint32_t[]> stacks;  // container and pair, by depth
  std::unique_ptr<bool[]> objects;
  std::vector<uint32_t> comments;  // JSONC: each comment's start and end
};
thread_local Thread thread;
// Walks made again at other widths, for a measurement.
thread_local uint64_t rewalks = 0;

// A walk at the widths `wide` names (bit 1 size, 2 off, 4 tail: 4 bytes).
template <bool C>
int walk_at(uint32_t wide, const uint8_t* buf, const uint32_t* index, uint32_t count, uint32_t shift, uint32_t len, rerail_v9np_sink* sink,
            const std::vector<uint32_t>& comments) {
  const uint32_t* cm = comments.data();
  const uint32_t nc = uint32_t(comments.size() / 2);
  using namespace stage2;
  Thread& s = thread;
  uint32_t* container = s.stacks.get();
  uint32_t* pair = s.stacks.get() + simdjson::DEFAULT_MAX_DEPTH + 1;
  bool* object = s.objects.get();
  switch (wide) {
    case 0: return walk<uint16_t, uint16_t, uint16_t, C>(buf, index, count, shift, len, sink, container, pair, object, cm, nc);
    case 1: return walk<uint32_t, uint16_t, uint16_t, C>(buf, index, count, shift, len, sink, container, pair, object, cm, nc);
    case 2: return walk<uint16_t, uint32_t, uint16_t, C>(buf, index, count, shift, len, sink, container, pair, object, cm, nc);
    case 3: return walk<uint32_t, uint32_t, uint16_t, C>(buf, index, count, shift, len, sink, container, pair, object, cm, nc);
    case 4: return walk<uint16_t, uint16_t, uint32_t, C>(buf, index, count, shift, len, sink, container, pair, object, cm, nc);
    case 5: return walk<uint32_t, uint16_t, uint32_t, C>(buf, index, count, shift, len, sink, container, pair, object, cm, nc);
    case 6: return walk<uint16_t, uint32_t, uint32_t, C>(buf, index, count, shift, len, sink, container, pair, object, cm, nc);
    default: return walk<uint32_t, uint32_t, uint32_t, C>(buf, index, count, shift, len, sink, container, pair, object, cm, nc);
  }
}

// This thread's stage 1 parser, this implementation's, made to hold `body` bytes.
simdjson::error_code ready(Thread& s, size_t body) {
  if (!s.stage1 || s.stage1->capacity() < body) {
    s.stage1.reset();
    const simdjson::implementation* impl = simdjson::get_available_implementations()[RERAIL_IMPLEMENTATION];
    if (impl == nullptr) { return simdjson::UNSUPPORTED_ARCHITECTURE; }
    auto error = impl->create_dom_parser_implementation(body < 64 ? 64 : body, simdjson::DEFAULT_MAX_DEPTH, s.stage1);
    if (error) { return error; }
  }
  if (!s.stacks) {
    s.stacks.reset(new uint32_t[2 * (simdjson::DEFAULT_MAX_DEPTH + 1)]);
    s.objects.reset(new bool[simdjson::DEFAULT_MAX_DEPTH + 1]);
  }
  return simdjson::SUCCESS;
}

// 0 and the tape and the source's digest written to `sink`, or simdjson's error code. With `C`, the
// source is read as JSONC: its comments are the tape's too (`index_and_hash`, `walk`).
template <typename H, bool C>
int tape(const char* src, size_t len, rerail_v9np_sink* sink) {
  Thread& s = thread;
  const uint8_t* buf = ::padded(s.buffer, s.buffer_capacity, src, len);
  const uint32_t shift = (len >= 3 && std::memcmp(buf, "\xEF\xBB\xBF", 3) == 0) ? 3 : 0;
  const size_t body = len - shift;
  auto error = ready(s, body);
  if (error) { return int(error); }
  auto& p = static_cast<dom_parser_implementation&>(*s.stage1);
  s.comments.clear();
  // The source is hashed as it was given: with `C`, `buf`'s comments are blanked.
  error = stage1::index_and_hash<H, C>(p, buf + shift, body, reinterpret_cast<const uint8_t*>(src), len, sink->digest, s.comments);
  if (error) { return int(error); }
  // `size` 4 bytes wide where the nodes, about one and a quarter a structural, pass 2 bytes; `off`
  // and `tail` 2. A prediction a walk shows wrong is walked again at the widths it showed (rare: a
  // node count near 65,536, or a node 65,536 bytes from its block's start or its last descendant).
  const uint32_t count = p.n_structural_indexes;
  uint32_t wide = count > 52000 ? 1 : 0;
  for (int attempt = 0; attempt < 3; attempt++) {
    const int r = walk_at<C>(wide, buf + shift, p.structural_indexes.get(), count, shift, uint32_t(len), sink, s.comments);
    if (r < 0) { return -r; }
    if (r == 0) { return 0; }
    rewalks++;
    wide = (wide | uint32_t(r & 7)) & ~((r & 8) ? 1u : 0u);
  }
  return int(simdjson::UNEXPECTED_ERROR);
}

// Stage 1 alone (a measurement's): simdjson's, or, `hashed`, ours with the digest. The structural
// index's length, or simdjson's error code negated.
template <typename H>
int64_t stage1_only(const char* src, size_t len, int hashed) {
  Thread& s = thread;
  const uint8_t* buf = ::padded(s.buffer, s.buffer_capacity, src, len);
  auto error = ready(s, len);
  if (error) { return -int64_t(error); }
  if (hashed) {
    uint8_t digest[32];
    auto& p = static_cast<dom_parser_implementation&>(*s.stage1);
    error = stage1::index_and_hash<H, false>(p, buf, len, buf, len, digest, s.comments);
  } else {
    error = s.stage1->stage1(buf, len, simdjson::stage1_mode::regular);
  }
  return error ? -int64_t(error) : int64_t(s.stage1->n_structural_indexes);
}

// Frees this thread's buffers when they hold more than `keep` bytes of source.
void release_above(size_t keep) {
  Thread& s = thread;
  if (s.buffer_capacity > keep + simdjson::SIMDJSON_PADDING) {
    s.stage1.reset();
    s.buffer.reset();
    s.buffer_capacity = 0;
  }
}

uint64_t walked_again() { return rewalks; }

}  // namespace rerail_v9np
