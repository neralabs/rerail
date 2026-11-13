// simdjson's DOM parser, one per thread, and the structural index it leaves: the start of each
// structural byte (`{ } [ ] : ,`) and of each scalar (a string's opening quote, a number's or a
// literal's first byte), in order. The parse validates the whole document (UTF-8, strings and
// their escapes, numbers, nesting), so an index is only given for a file simdjson accepts.
#include "simdjson.h"
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
}  // namespace

extern "C" {

// 0 and the index (valid until this thread's next call), or simdjson's error code.
int rerail_simdjson_structurals(const char* src, size_t len, const uint32_t** indexes, uint32_t* count) {
  State& s = state;
  if (s.capacity < len + simdjson::SIMDJSON_PADDING) {
    s.capacity = len + simdjson::SIMDJSON_PADDING;
    s.buffer.reset(new char[s.capacity]);
  }
  std::memcpy(s.buffer.get(), src, len);
  std::memset(s.buffer.get() + len, 0, simdjson::SIMDJSON_PADDING);
  simdjson::dom::element root;
  auto error = s.parser.parse(s.buffer.get(), len, false).get(root);
  if (error) {
    return int(error);
  }
  *indexes = s.parser.implementation->structural_indexes.get();
  *count = s.parser.implementation->n_structural_indexes;
  return 0;
}

// Frees this thread's parser and buffer when they hold more than `keep` bytes of source.
void rerail_simdjson_release_above(size_t keep) {
  State& s = state;
  if (s.capacity > keep + simdjson::SIMDJSON_PADDING || s.parser.capacity() > keep) {
    s.parser = simdjson::dom::parser();
    s.buffer.reset();
    s.capacity = 0;
  }
}

const char* rerail_simdjson_error_message(int code) {
  return simdjson::error_message(simdjson::error_code(code));
}

const char* rerail_simdjson_implementation() {
  static const std::string name = simdjson::get_active_implementation()->name();
  return name.c_str();
}
}
