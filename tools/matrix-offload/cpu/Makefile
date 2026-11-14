# CPU matrix-offload probes. Binaries go to $(OUT) (default: ./bin, gitignored).
# -mcpu=apple-m4 enables SME/SME2 intrinsics without enabling plain (non-streaming) SVE; +cssc makes
# __builtin_popcountll a scalar CNT (FEAT_CSSC is present) instead of fmov + NEON cnt + addv.
# Do NOT use -march=armv9.2-a: it emits SVE outside streaming mode, which faults on Apple silicon.
CC ?= clang
CFLAGS ?= -O2 -mcpu=apple-m4+cssc -Wall -Wno-unused-function
OUT ?= bin
PROBES = c1_streaming c2_bmopa c3_mopa c4_bitmatmul c5_intersect c6_bitmap c7_bandwidth
WORKLOADS = w1_prefilter w2_eqjoin w3_closure w4_hist w5_cooc

all: $(addprefix $(OUT)/,$(PROBES)) $(addprefix $(OUT)/,$(WORKLOADS))

$(OUT)/%: %.c bench.h mo.h
	@mkdir -p $(OUT)
	$(CC) $(CFLAGS) $< -o $@ -lpthread

clean:
	rm -rf $(OUT)
.PHONY: all clean
