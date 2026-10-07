#!/usr/bin/env python3
"""Download and quantize the pinned Model2Vec artifact.

Requires model2vec==0.7.0 and huggingface-hub. Output is deterministic for the
pinned source revision and records hashes consumed by the native loader.
"""

from __future__ import annotations

import hashlib
import json
import shutil
import tempfile
from pathlib import Path

from huggingface_hub import snapshot_download
from model2vec import StaticModel

MODEL_ID = "minishlab/potion-base-8M"
REVISION = "bf8b056651a2c21b8d2565580b8569da283cab23"
OUTPUT = Path(__file__).resolve().parents[1] / "model"
FILES = ("config.json", "tokenizer.json", "model.safetensors")


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def main() -> None:
    with tempfile.TemporaryDirectory(prefix="rerail-model-") as temporary:
        snapshot = snapshot_download(
            repo_id=MODEL_ID,
            revision=REVISION,
            allow_patterns=list(FILES),
        )
        model = StaticModel.from_pretrained(snapshot, quantize_to="int8")
        quantized = Path(temporary) / "quantized"
        model.save_pretrained(quantized)

        OUTPUT.mkdir(parents=True, exist_ok=True)
        for name in FILES:
            shutil.copyfile(quantized / name, OUTPUT / name)

    manifest = {
        "formatVersion": 1,
        "modelId": MODEL_ID,
        "revision": REVISION,
        "license": "MIT",
        "dimensions": 256,
        "quantization": "int8",
        "quantizer": "model2vec==0.7.0",
        "files": {name: {"sha256": sha256(OUTPUT / name)} for name in FILES},
    }
    (OUTPUT / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")


if __name__ == "__main__":
    main()
