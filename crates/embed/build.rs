//! Stages the pinned `model.safetensors` into `OUT_DIR` when the `bundled`
//! feature is on, so `include_bytes!` can embed it without the ~90 MB blob
//! living in git. The weights are read from `CRABTALK_EMBED_MODEL_PATH` and
//! verified against the pinned hash before they're baked in.
use sha2::{Digest, Sha256};
use std::env;
use std::fmt::Write;
use std::fs;
use std::path::PathBuf;

/// Keep in sync with `MODEL_SHA256` in `src/lib.rs`.
const MODEL_SHA256: &str = "53aa51172d142c89d9012cce15ae4d6cc0ca6895895114379cacb4fab128d9db";

fn main() {
    println!("cargo:rerun-if-env-changed=CRABTALK_EMBED_MODEL_PATH");
    if env::var_os("CARGO_FEATURE_BUNDLED").is_none() {
        return;
    }
    let out = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR")).join("model.safetensors");
    if out.exists() && sha256_hex(&fs::read(&out).unwrap()) == MODEL_SHA256 {
        return;
    }
    let src = env::var_os("CRABTALK_EMBED_MODEL_PATH").unwrap_or_else(|| {
        panic!(
            "the `bundled` feature needs the model weights: set CRABTALK_EMBED_MODEL_PATH to a \
             model.safetensors with sha256 {MODEL_SHA256} (all-MiniLM-L6-v2)"
        )
    });
    let src = PathBuf::from(src);
    println!("cargo:rerun-if-changed={}", src.display());
    let bytes = fs::read(&src)
        .unwrap_or_else(|e| panic!("read CRABTALK_EMBED_MODEL_PATH ({}): {e}", src.display()));
    let got = sha256_hex(&bytes);
    assert_eq!(
        got, MODEL_SHA256,
        "CRABTALK_EMBED_MODEL_PATH sha256 mismatch: got {got}, expected {MODEL_SHA256}"
    );
    fs::write(&out, &bytes).expect("stage model.safetensors into OUT_DIR");
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut s = String::with_capacity(digest.len() * 2);
    for b in digest {
        let _ = write!(s, "{b:02x}");
    }
    s
}
