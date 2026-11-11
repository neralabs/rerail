// simdjson's own stage 1, as the reference for the transducer's block program
// (`native/src/code_rule_dsl/transducer/packed/block_tests.rs`, `stage1_*`).
//
// For each file: its length, the number of structural indexes stage 1 finds, and an FNV-1a
// 64-bit hash of the indexes as little-endian u32s. With `--time`, also stage 1's CPU time
// (user + system) per 64-byte block, best of 5 runs of enough repetitions to take ~1 s.
//
// Build and run (Homebrew simdjson):
//   clang++ -std=c++17 -O2 -I/opt/homebrew/include stage1_indexes.cpp \
//     -L/opt/homebrew/lib -lsimdjson -o /tmp/stage1_indexes
//   /tmp/stage1_indexes [--time] FILE...   (paths relative to the repository root)
#include <simdjson.h>
#include <sys/resource.h>
#include <cstdio>
#include <cstring>

static double cpu_seconds() {
  rusage u{};
  getrusage(RUSAGE_SELF, &u);
  return u.ru_utime.tv_sec + u.ru_stime.tv_sec + (u.ru_utime.tv_usec + u.ru_stime.tv_usec) * 1e-6;
}

int main(int argc, char** argv) {
  bool time = false;
  for (int i = 1; i < argc; i++) {
    if (std::strcmp(argv[i], "--time") == 0) { time = true; continue; }
    simdjson::padded_string json;
    if (simdjson::padded_string::load(argv[i]).get(json)) { std::fprintf(stderr, "cannot read %s\n", argv[i]); return 1; }
    simdjson::dom::parser parser;
    if (parser.allocate(json.size())) { std::fprintf(stderr, "cannot allocate\n"); return 1; }
    auto* impl = parser.implementation.get();
    auto bytes = reinterpret_cast<const uint8_t*>(json.data());
    if (auto err = impl->stage1(bytes, json.size(), simdjson::stage1_mode::regular)) {
      std::fprintf(stderr, "%s: stage 1 failed: %s\n", argv[i], simdjson::error_message(err));
      return 1;
    }
    uint64_t hash = 0xcbf29ce484222325ull;
    for (uint32_t k = 0; k < impl->n_structural_indexes; k++) {
      uint32_t v = impl->structural_indexes[k];
      for (int b = 0; b < 4; b++) { hash ^= (v >> (8 * b)) & 0xff; hash *= 0x100000001b3ull; }
    }
    std::printf("%s %zu %u %016llx\n", argv[i], json.size(), impl->n_structural_indexes, (unsigned long long)hash);
    if (time) {
      size_t reps = 1 + (size_t)(1e9 / (json.size() + 1));  // ~1 GB of input per run
      double best = 1e30;
      for (int run = 0; run < 5; run++) {
        double t0 = cpu_seconds();
        for (size_t r = 0; r < reps; r++) { (void)impl->stage1(bytes, json.size(), simdjson::stage1_mode::regular); }
        double t = cpu_seconds() - t0;
        if (t < best) best = t;
      }
      double blocks = (double)reps * (double)((json.size() + 63) / 64);
      std::printf("# %s: simdjson %s stage 1: %.3f ns per 64-byte block (CPU time, best of 5)\n", argv[i],
                  simdjson::get_active_implementation()->name().c_str(), best / blocks * 1e9);
    }
  }
  return 0;
}
