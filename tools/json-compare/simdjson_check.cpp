// simdjson, as the oracle and the yardstick for the Bitlex JSON forms
// (docs/parser-generators/21-json-against-simdjson.md).
//
//   simdjson_check verdict FILE...        a line per file: 1 (accepted) or 0 and the error
//   simdjson_check bench ITERS FILE...    a line per file: bytes, stage 1 and whole-parse CPU
//                                         nanoseconds per run (the median of ITERS runs)
//   simdjson_check run MODE ITERS FILE... MODE none, stage1 or parse, ITERS times over every
//                                         file: for counting instructions with `time -l`
//
// Build: ./build.sh (clang++ -O3 against Homebrew's simdjson).
#include <simdjson.h>
#include <algorithm>
#include <cstdio>
#include <cstring>
#include <ctime>
#include <string>
#include <vector>

static double cpu_ns() {
  timespec t;
  clock_gettime(CLOCK_THREAD_CPUTIME_ID, &t);
  return double(t.tv_sec) * 1e9 + double(t.tv_nsec);
}

static simdjson::padded_string load(const char *path) {
  auto r = simdjson::padded_string::load(path);
  if (r.error()) {
    std::fprintf(stderr, "%s: %s\n", path, simdjson::error_message(r.error()));
    std::exit(2);
  }
  return std::move(r).value_unsafe();
}

static bool stage1(simdjson::dom::parser &p, const simdjson::padded_string &s) {
  if (p.allocate(s.size() < 64 ? 64 : s.size()) != simdjson::SUCCESS) return false;
  return p.implementation->stage1(reinterpret_cast<const uint8_t *>(s.data()), s.size(), simdjson::stage1_mode::regular) == simdjson::SUCCESS;
}

int main(int argc, char **argv) {
  if (argc < 3) {
    std::fprintf(stderr, "usage: simdjson_check verdict|bench|run ...\n");
    return 2;
  }
  std::string mode = argv[1];
  simdjson::dom::parser parser;
  if (mode == "verdict") {
    for (int i = 2; i < argc; i++) {
      auto s = load(argv[i]);
      auto doc = parser.parse(s);
      if (doc.error()) std::printf("0 %s %s\n", argv[i], simdjson::error_message(doc.error()));
      else std::printf("1 %s\n", argv[i]);
    }
    return 0;
  }
  if (mode == "bench") {
    int iters = std::atoi(argv[2]);
    std::printf("file\tbytes\tstage1_ns\tparse_ns\n");
    for (int i = 3; i < argc; i++) {
      auto s = load(argv[i]);
      std::vector<double> one, all;
      for (int k = 0; k < iters; k++) {
        double t = cpu_ns();
        bool ok = stage1(parser, s);
        one.push_back(cpu_ns() - t);
        t = cpu_ns();
        auto doc = parser.parse(s);
        all.push_back(cpu_ns() - t);
        if (!ok || doc.error()) std::fprintf(stderr, "%s: refused\n", argv[i]);
      }
      std::sort(one.begin(), one.end());
      std::sort(all.begin(), all.end());
      std::printf("%s\t%zu\t%.0f\t%.0f\n", argv[i], s.size(), one[one.size() / 2], all[all.size() / 2]);
    }
    return 0;
  }
  if (mode == "run") {
    std::string what = argv[2];
    int iters = std::atoi(argv[3]);
    std::vector<simdjson::padded_string> files;
    for (int i = 4; i < argc; i++) files.push_back(load(argv[i]));
    size_t ok = 0;
    for (int k = 0; k < iters; k++)
      for (auto &s : files) {
        if (what == "stage1") ok += stage1(parser, s);
        else if (what == "parse") ok += !parser.parse(s).error();
      }
    std::printf("%zu\n", ok);
    return 0;
  }
  return 2;
}
