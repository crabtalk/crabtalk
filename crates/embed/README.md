# crabtalk-embed

Shared 384-dim text embedder (all-MiniLM-L6-v2) for Crabtalk, on Candle (pure
Rust, CPU). Cloud and desktop both embed through this crate, so a vector made on
one side is comparable to one made on the other **by construction** — same code,
same weights (pinned by sha256), same output.

## Model provenance

`MODEL_ID = "minilm-l6-v2-st"`, `DIM = 384`. The three model files are pinned by
sha256 in `src/lib.rs`. The crate bundles **no** weights and carries **no** HTTP
client: you supply a directory holding `config.json`, `tokenizer.json`, and
`model.safetensors`, and each is verified against its pin on load.

## Use

```rust
use crabtalk_embed::{Embedder, l2_normalize};

// `dir` holds config.json / tokenizer.json / model.safetensors — fetch them
// however you like (they're on the canonical sentence-transformers repo).
let e = Embedder::new(".model");
let mut v = e.embed(vec!["hello".into()])?.pop().unwrap();
l2_normalize(&mut v);
```

The model loads and verifies lazily on the first `embed()`, then caches for the
embedder's lifetime. A wrong or corrupt file fails with `EmbedError::Hash`.
