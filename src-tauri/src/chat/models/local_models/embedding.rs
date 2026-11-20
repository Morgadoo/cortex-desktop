//! Embeddings: a sentence-transformer (BERT) for semantic library search.

use super::*;

// Embeddings: a sentence-transformer (BERT) for semantic library search

pub(super) struct Embedder {
    pub(super) id: String,
    pub(super) model: candle_transformers::models::bert::BertModel,
    pub(super) tokenizer: tokenizers::Tokenizer,
}

pub(super) fn embedder() -> &'static Mutex<Option<Embedder>> {
    static EMBEDDER: OnceLock<Mutex<Option<Embedder>>> = OnceLock::new();
    EMBEDDER.get_or_init(|| Mutex::new(None))
}

/// Passages embedded per forward pass.
pub(super) const EMBED_BATCH: usize = 16;

/// Unit-length vectors for `texts`, in order: the mean of the token vectors (as
/// sentence-transformers does), normalized so a dot product is the cosine.
pub fn embed_texts(root: &Path, id: &str, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
    use candle_transformers::models::bert;
    if !installed(root, id) || entry(id).is_none_or(|e| e.kind != Kind::Embedding) {
        return Err(format!(
            "Download {} first (Settings → Providers → Built-in models).",
            entry(id).map_or(id, |e| e.name)
        ));
    }
    let mut guard = embedder().lock().map_err(|e| e.to_string())?;
    if guard.as_ref().is_none_or(|e| e.id != id) {
        *guard = None;
        let dir = model_dir(root, id)?;
        let config: bert::Config = serde_json::from_str(
            &std::fs::read_to_string(dir.join("config.json")).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        // SAFETY: the file is ours, downloaded and not changed while it is mapped.
        let vb = unsafe {
            candle_nn::VarBuilder::from_mmaped_safetensors(
                &[dir.join("model.safetensors")],
                bert::DTYPE,
                &Device::Cpu,
            )
        }
        .map_err(|e| e.to_string())?;
        let model = bert::BertModel::load(vb, &config).map_err(|e| e.to_string())?;
        let mut tokenizer = tokenizers::Tokenizer::from_file(dir.join("tokenizer.json"))
            .map_err(|e| e.to_string())?;
        tokenizer.with_padding(Some(tokenizers::PaddingParams {
            strategy: tokenizers::PaddingStrategy::BatchLongest,
            ..Default::default()
        }));
        tokenizer
            .with_truncation(Some(tokenizers::TruncationParams {
                max_length: 256,
                ..Default::default()
            }))
            .map_err(|e| e.to_string())?;
        *guard = Some(Embedder {
            id: id.to_string(),
            model,
            tokenizer,
        });
    }
    let e = guard.as_ref().ok_or("The embedding model is not loaded.")?;
    let mut out = Vec::with_capacity(texts.len());
    for chunk in texts.chunks(EMBED_BATCH) {
        let encodings = e
            .tokenizer
            .encode_batch(chunk.to_vec(), true)
            .map_err(|err| err.to_string())?;
        let ids: Vec<Tensor> = encodings
            .iter()
            .map(|enc| Tensor::new(enc.get_ids(), &Device::Cpu))
            .collect::<Result<_, _>>()
            .map_err(|err| err.to_string())?;
        let masks: Vec<Tensor> = encodings
            .iter()
            .map(|enc| Tensor::new(enc.get_attention_mask(), &Device::Cpu))
            .collect::<Result<_, _>>()
            .map_err(|err| err.to_string())?;
        let ids = Tensor::stack(&ids, 0).map_err(|err| err.to_string())?;
        let mask = Tensor::stack(&masks, 0).map_err(|err| err.to_string())?;
        let types = ids.zeros_like().map_err(|err| err.to_string())?;
        let hidden = e
            .model
            .forward(&ids, &types, Some(&mask))
            .map_err(|err| err.to_string())?;
        // Mean over the real tokens only.
        let mask_f = mask
            .to_dtype(candle_core::DType::F32)
            .and_then(|m| m.unsqueeze(2))
            .map_err(|err| err.to_string())?;
        let summed = hidden
            .broadcast_mul(&mask_f)
            .and_then(|h| h.sum(1))
            .map_err(|err| err.to_string())?;
        let counts = mask_f.sum(1).map_err(|err| err.to_string())?;
        let mean = summed
            .broadcast_div(&counts)
            .map_err(|err| err.to_string())?;
        let norm = mean
            .sqr()
            .and_then(|s| s.sum_keepdim(1))
            .and_then(|s| s.sqrt())
            .map_err(|err| err.to_string())?;
        let unit = mean.broadcast_div(&norm).map_err(|err| err.to_string())?;
        out.extend(unit.to_vec2::<f32>().map_err(|err| err.to_string())?);
    }
    Ok(out)
}
