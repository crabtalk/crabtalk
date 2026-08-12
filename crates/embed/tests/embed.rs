//! Behavioural guard on the embedder. Needs a real model directory — set
//! `CRABTALK_EMBED_MODEL_DIR` to one holding config.json / tokenizer.json /
//! model.safetensors. Skipped (with a note) when unset. A drift in the model or
//! the pooling trips these.
use crabtalk_embed::{DIM, EmbedRole, Embedder, l2_normalize};

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

fn embed_normalized(e: &Embedder, text: &str) -> Vec<f32> {
    let mut v = e
        .embed(vec![text.to_string()], EmbedRole::Document)
        .unwrap()
        .pop()
        .unwrap();
    l2_normalize(&mut v);
    v
}

fn embedder() -> Option<Embedder> {
    match std::env::var("CRABTALK_EMBED_MODEL_DIR") {
        Ok(dir) => Some(Embedder::new(dir)),
        Err(_) => {
            eprintln!("skip: set CRABTALK_EMBED_MODEL_DIR to a model directory to run this test");
            None
        }
    }
}

#[test]
fn dims_and_normalization() {
    let Some(e) = embedder() else { return };
    let v = embed_normalized(&e, "hello world");
    assert_eq!(v.len(), DIM as usize);
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    assert!((norm - 1.0).abs() < 1e-5, "normalized norm was {norm}");
}

#[test]
fn semantic_ordering_holds() {
    let Some(e) = embedder() else { return };
    let cat = embed_normalized(&e, "cat");
    let dog = embed_normalized(&e, "dog");
    let qcd = embed_normalized(&e, "quantum chromodynamics");
    let cat_dog = cosine(&cat, &dog);
    let cat_qcd = cosine(&cat, &qcd);
    assert!(
        cat_dog > cat_qcd,
        "cat·dog ({cat_dog}) should exceed cat·qcd ({cat_qcd})"
    );
}
