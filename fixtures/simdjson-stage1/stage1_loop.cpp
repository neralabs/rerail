// simdjson's stage 1 loop, built alone: the loop of `json_structural_indexer::index<64>` (arm64's
// step size) over a document's full blocks, with `step<64>` and `next()` written out as they
// are there, in a function of its own. In simdjson that loop is inlined into
// `dom_parser_implementation::stage1` with the reader's tail, `finish()` and the buffer checks;
// here it is the function's only loop, so its machine code can be set beside tier 1's hot loop
// (`scripts/dev/simdjson-stage1-loop`, docs/parser-generators/39).
//
// Built against the vendored amalgamation (native/vendor/simdjson-5.0.5), with the flags
// `rerail-simdjson-native` builds it with.
#include "simdjson.h"
#include "simdjson.cpp"

using namespace simdjson::arm64;
using namespace simdjson::arm64::stage1;

// Returns the state a caller would read after the loop, so none of it is dead.
extern "C" __attribute__((noinline)) uint64_t rerail_stage1_loop(const uint8_t *buf, size_t len, uint32_t *indexes, uint32_t **tail) {
  json_scanner scanner{};
  utf8_checker checker{};
  bit_indexer indexer(indexes);
  uint64_t prev_structurals = 0;
  uint64_t unescaped_chars_error = 0;
  buf_block_reader<64> reader(buf, len);
  while (reader.has_full_block()) {
    // step<64>
    simd::simd8x64<uint8_t> in(reader.full_block());
    json_block block = scanner.next(in);
    // next()
    uint64_t unescaped = in.lteq(0x1F);
    checker.check_next_input(in);
    indexer.write(uint32_t(reader.block_index() - 64), prev_structurals);
    prev_structurals = block.structural_start();
    unescaped_chars_error |= block.non_quote_inside_string(unescaped);
    reader.advance();
  }
  *tail = indexer.tail;
  return prev_structurals ^ unescaped_chars_error ^ uint64_t(scanner.finish()) ^ uint64_t(checker.errors());
}
