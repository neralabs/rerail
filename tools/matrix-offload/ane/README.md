# ANE microprobes

Core ML ML programs built with coremltools (dev-only Python) and run, timed and placement-checked from a
Swift harness. Results: `RESULTS-ane.md`.

Files:

- `gen.py`: builds a model from its name (grammar in the docstring), e.g. `mm_64_8192_8192_p1_mm`,
  `chain_2048_2048_16_p1_clip`, `eq_4096_4096_cast`.
- `anebench.swift`: compiles (.mlpackage -> .mlmodelc), loads, prints the MLComputePlan placement per op,
  and times `prediction` calls (wall min/median/p90, host CPU per call). Options: `--iters N --warm W`,
  `--iosurface` (CVPixelBuffer-backed inputs and output backings), `--batch B` (batch API),
  `--in DIR` / `--out DIR` (raw input/output files), `--plan` / `--plan-only`, `--cu ane|cpu|gpu|all`,
  `--gpu-busy SECONDS` (memory-bound Metal load in parallel).
- `probe.sh LOG "ARGS" NAME...`: builds missing models, runs anebench on each, appends to `out/LOG.log`.
- `exact.py`: inputs and checkers for the counting (A4) and equality (A5) exactness probes.

Setup (paths: `RUNS` = scratch folder for models and logs, `PY` = Python with coremltools 9 and numpy):

```sh
export RUNS=/path/to/ane-runs PY=/path/to/cmlenv/bin/python
mkdir -p $RUNS/bin && swiftc -O -parse-as-library anebench.swift -o $RUNS/bin/anebench
```

Probes (each command stays under 2 minutes; build each 128 MB-weight model in its own run, since the first
ANE compile of such a model takes 10-30 s; use bash, not zsh, for the word-splitting loops):

```sh
# A1 dispatch floor and batch API
./probe.sh a1 "--plan --iosurface --iters 3000 --warm 100" add_1 mm_256_1024_1024_fp16_mm mm_64_2048_2048_fp16_mm
./probe.sh a1 "--iters 50 --warm 5 --batch 64" add_1 mm_256_1024_1024_fp16_mm
# A2 storage sweep (one model per run for 8192x8192 and 16384x4096)
for s in fp16 int8 p4 p2 p1; do ./probe.sh a2 "--plan --iosurface --iters 300 --warm 20" mm_1_8192_8192_${s}_mm; done
for s in fp16 int8 p4 p2 p1; do ./probe.sh a2 "--plan --iosurface --iters 300 --warm 20" mm_1_4096_4096_${s}_mm mm_64_4096_4096_${s}_mm; done
for s in fp16 int8 p4 p2 p1; do ./probe.sh a2 "--plan --iosurface --iters 60 --warm 5" mm_4096_4096_4096_${s}_mm; done
# A3 dynamic matmul, conv, linear
./probe.sh a3 "--plan --iosurface --iters 60 --warm 5" dmm_4096_4096_4096 mm_4096_4096_4096_fp16_conv mm_4096_4096_4096_p2_conv mm_4096_4096_4096_fp16_lin dconv_2048_2048_2048
# A4 counting exactness (K = 4096, 16384, 32768, 65536)
K=16384; $PY -I exact.py mkcnt $RUNS/io/cnt$K 256 $K 256
./probe.sh a4 "--plan --iters 20 --in $RUNS/io/cnt$K --out $RUNS/io/cnt$K/out" cnt_256_${K}_256
$PY -I exact.py chkcnt $RUNS/io/cnt$K 256 $K 256
# A5 equality counts (append "two" for two-digit keys; model prefix eq2_)
$PY -I exact.py mkeq $RUNS/io/eq4096 4096 4096 one
./probe.sh a5 "--plan --iters 100 --in $RUNS/io/eq4096 --out $RUNS/io/eq4096/out" eq_4096_4096_cast
$PY -I exact.py chkeq $RUNS/io/eq4096 4096 4096 one
# A6 fused chains (clip = sparse A, clipd = dense A, acc = accumulate form)
for k in 1 4 16; do ./probe.sh a6 "--plan --iosurface --iters 40" chain_2048_2048_${k}_p1_clip chain_2048_2048_${k}_fp16_clip; done
# A7 gather / topk / argsort placement
./probe.sh a7 "--plan --iters 200" gat_65536_64_4096 gnd_65536_64_4096 topk_16384_64 sort_16384
# A8 with a busy GPU
./probe.sh a8 "--iosurface --iters 300 --gpu-busy 40" mm_1_8192_8192_p1_mm
```

Core ML caches the ANE-compiled bundles in `~/Library/Caches/anebench/com.apple.e5rt.e5bundlecache`, about
1 GB after the full sweep. Delete it to force cold loads. Its `weights.bin` sizes show the stored weight
format (uint1/2/4, int8 or fp16).
