//! Behavioural guard on the embedder. Runs only with the `bundled` feature (it
//! needs real weights). A drift in the model or the pooling trips these:
//! run with `--features bundled` and `CRABTALK_EMBED_MODEL_PATH` set.
#![cfg(feature = "bundled")]

use crabtalk_embed::{DIM, Embedder, l2_normalize};

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

fn embed_normalized(e: &Embedder, text: &str) -> Vec<f32> {
    let mut v = e.embed(vec![text.to_string()]).unwrap().pop().unwrap();
    l2_normalize(&mut v);
    v
}

#[test]
fn dims_and_normalization() {
    let e = Embedder::default();
    let v = embed_normalized(&e, "hello world");
    assert_eq!(v.len(), DIM as usize);
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    assert!((norm - 1.0).abs() < 1e-5, "normalized norm was {norm}");
}

#[test]
fn semantic_ordering_holds() {
    let e = Embedder::default();
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
