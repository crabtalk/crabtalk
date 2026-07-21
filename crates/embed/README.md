# crabtalk-embed

Shared 384-dim text embedder (multilingual-e5-small) for Crabtalk, on Candle
(pure Rust). Cloud and desktop both embed through this crate, so a vector made on
one side is comparable to one made on the other **by construction** — same code,
same weights (pinned by sha256), same output. The model is multilingual
(cross-lingual retrieval) and asymmetric: queries and documents take different
prefixes, so a native-language prompt matches source text in any language.

## Model provenance

`MODEL_ID = "multilingual-e5-small"`, `DIM = 384`. The three model files
(`config.json`, `tokenizer.json`, `model.safetensors`) are listed in `FILES` and
pinned by sha256 in `src/lib.rs`. The crate bundles **no** weights and carries
**no** HTTP client: you supply a directory holding those files — fetch them from
the canonical `intfloat/multilingual-e5-small` repo — and each is verified
against its pin on load.

## Use

```rust
use crabtalk_embed::{EmbedRole, Embedder, l2_normalize};

let e = Embedder::new(".model");
// A retrieval query…
let mut q = e.embed(vec!["hello".into()], EmbedRole::Query)?.pop().unwrap();
l2_normalize(&mut q);
// …matched against documents embedded with EmbedRole::Document.
```

The model loads and verifies lazily on the first `embed()`, then caches for the
embedder's lifetime. A wrong or corrupt file fails with `EmbedError::Hash`.
Inference runs on CPU in fp32.
