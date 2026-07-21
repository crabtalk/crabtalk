# crabtalk-embed

Shared 384-dim text embedder (all-MiniLM-L6-v2) for Crabtalk, on Candle (pure
Rust, CPU). Cloud and desktop both embed through this crate, so a vector made on
one side is comparable to one made on the other **by construction** — same code,
same weights (pinned by sha256), same output.

## Model provenance

`MODEL_ID = "minilm-l6-v2-st"`, `DIM = 384`. The three model files are pinned by
sha256 in `src/lib.rs`. `config.json` and `tokenizer.json` are committed under
`model_cache/`; `model.safetensors` (~90 MB) is **not** in git.

## Getting the weights

- **`bundled` feature** — embeds the weights in the binary. `build.rs` stages
  them into `OUT_DIR`, reading from `CRABTALK_EMBED_MODEL_PATH` (a path to a
  `model.safetensors` whose sha256 matches the pin). Keeps the 90 MB out of git
  while still producing a self-contained binary.
- **`download` feature** — `ensure(dir)` downloads the pinned files into `dir` at
  runtime (verifying each hash); then load with `ModelSource::Dir(dir)`.

Both paths verify the same hashes, so the weights — and therefore the vectors —
are identical regardless of how they arrived.

## Use

```rust
// bundled:
let e = crabtalk_embed::Embedder::default();
let mut v = e.embed(vec!["hello".into()])?.pop().unwrap();
crabtalk_embed::l2_normalize(&mut v);
```
