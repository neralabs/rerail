# Embedding model

The static embedding model conversation rules use for semantic similarity: `minishlab/potion-base-8M` at the revision
in `manifest.json`, with 256 dimensions, quantized to int8 by model2vec 0.7.0, under the MIT licence. It is shipped
inside the package and loaded by the native engine (`native/src/model.rs`) from `src/engine/package-root.ts`'s
`model/` path.

- `manifest.json` pins the source revision and the SHA-256 of each file. The engine checks those hashes when it loads
  the model.
- `model.safetensors` is stored in Git LFS (`.gitattributes`). Install git-lfs before cloning (see CONTRIBUTING.md).
- The files are produced by `scripts/generate/model/prepare_model.py` (see `scripts/generate/README.md`).
