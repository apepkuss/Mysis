use ndarray::{Array1, Array2, ArrayView3, Axis};
use ort::session::Session;
use ort::value::TensorRef;
use std::path::Path;
use tokenizers::Tokenizer;

pub struct Embedder {
    session: Session,
    tokenizer: Tokenizer,
    dim: usize,
}

impl Embedder {
    /// 加载 ONNX 模型和 tokenizer
    pub fn new(model_dir: &str, dim: usize) -> Result<Self, String> {
        let model_path = Path::new(model_dir).join("model.onnx");
        let tokenizer_path = Path::new(model_dir).join("tokenizer.json");

        if !model_path.exists() {
            return Err(format!("model not found: {}", model_path.display()));
        }
        if !tokenizer_path.exists() {
            return Err(format!("tokenizer not found: {}", tokenizer_path.display()));
        }

        let session = Session::builder()
            .map_err(|e| format!("failed to create session builder: {e}"))?
            .commit_from_file(&model_path)
            .map_err(|e| format!("failed to load ONNX model: {e}"))?;

        let tokenizer = Tokenizer::from_file(&tokenizer_path)
            .map_err(|e| format!("failed to load tokenizer: {e}"))?;

        Ok(Self {
            session,
            tokenizer,
            dim,
        })
    }

    /// 文本 → 嵌入向量（384 维）
    pub fn embed(&mut self, text: &str) -> Result<Vec<f32>, String> {
        let encoding = self
            .tokenizer
            .encode(text, true)
            .map_err(|e| format!("tokenize failed: {e}"))?;

        let ids = encoding.get_ids();
        let attention_mask = encoding.get_attention_mask();
        let token_type_ids = encoding.get_type_ids();
        let seq_len = ids.len();

        // 构建输入张量 [1, seq_len]，使用 (shape, slice) 元组
        let ids_data: Vec<i64> = ids.iter().map(|&x| x as i64).collect();
        let mask_data: Vec<i64> = attention_mask.iter().map(|&x| x as i64).collect();
        let type_data: Vec<i64> = token_type_ids.iter().map(|&x| x as i64).collect();

        let ids_tensor = TensorRef::from_array_view(([1, seq_len], ids_data.as_slice()))
            .map_err(|e| format!("tensor error: {e}"))?;
        let mask_tensor = TensorRef::from_array_view(([1, seq_len], mask_data.as_slice()))
            .map_err(|e| format!("tensor error: {e}"))?;
        let type_tensor = TensorRef::from_array_view(([1, seq_len], type_data.as_slice()))
            .map_err(|e| format!("tensor error: {e}"))?;

        let outputs = self
            .session
            .run(ort::inputs![
                "input_ids" => ids_tensor,
                "attention_mask" => mask_tensor,
                "token_type_ids" => type_tensor,
            ])
            .map_err(|e| format!("inference failed: {e}"))?;

        // 输出 shape: [1, seq_len, dim]
        let (output_shape, output_data) = outputs[0]
            .try_extract_tensor::<f32>()
            .map_err(|e| format!("extract output failed: {e}"))?;

        let output_view = ArrayView3::<f32>::from_shape(
            (
                output_shape[0] as usize,
                output_shape[1] as usize,
                output_shape[2] as usize,
            ),
            output_data,
        )
        .map_err(|e| format!("output reshape failed: {e}"))?;

        // Mean pooling over attention mask
        let mask_f32: Array2<f32> = Array2::from_shape_vec(
            (1, seq_len),
            attention_mask.iter().map(|&x| x as f32).collect(),
        )
        .map_err(|e| format!("mask shape error: {e}"))?;

        // batch 0: [seq_len, dim]
        let embeddings = output_view.index_axis(Axis(0), 0);
        let mask_expanded = mask_f32
            .index_axis(Axis(0), 0)
            .insert_axis(Axis(1))
            .broadcast((seq_len, self.dim))
            .ok_or("broadcast failed")?
            .to_owned();

        // masked mean pooling
        let masked = &embeddings.to_owned() * &mask_expanded;
        let summed = masked.sum_axis(Axis(0));
        let mask_sum = mask_expanded.sum_axis(Axis(0)).mapv(|v| v.max(1e-9));
        let pooled = &summed / &mask_sum;

        // L2 normalize
        let norm = pooled.dot(&pooled).sqrt().max(1e-9);
        let normalized = pooled.mapv(|v| v / norm);

        Ok(normalized.to_vec())
    }
}

/// 余弦相似度
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return 0.0;
    }
    let a = Array1::from_vec(a.to_vec());
    let b = Array1::from_vec(b.to_vec());

    let dot = a.dot(&b);
    let norm_a = a.dot(&a).sqrt();
    let norm_b = b.dot(&b).sqrt();

    if norm_a < 1e-9 || norm_b < 1e-9 {
        return 0.0;
    }
    dot / (norm_a * norm_b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cosine_similarity_identical() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert!((sim - 1.0).abs() < 1e-6);
    }

    #[test]
    fn cosine_similarity_orthogonal() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![0.0, 1.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert!(sim.abs() < 1e-6);
    }

    #[test]
    fn cosine_similarity_opposite() {
        let a = vec![1.0, 0.0];
        let b = vec![-1.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert!((sim - (-1.0)).abs() < 1e-6);
    }

    #[test]
    fn cosine_similarity_mismatched_len() {
        let a = vec![1.0, 0.0];
        let b = vec![1.0, 0.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert_eq!(sim, 0.0);
    }

    // 以下测试需要模型文件，标记为 ignore
    #[test]
    #[ignore]
    fn embed_produces_correct_dim() {
        let model_dir = format!(
            "{}/.mysis/models/bge-small-zh-v1.5",
            std::env::var("HOME").unwrap()
        );
        let mut embedder = Embedder::new(&model_dir, 384).unwrap();
        let vec = embedder.embed("测试文本").unwrap();
        assert_eq!(vec.len(), 384);
    }

    #[test]
    #[ignore]
    fn similar_sentences_high_similarity() {
        let model_dir = format!(
            "{}/.mysis/models/bge-small-zh-v1.5",
            std::env::var("HOME").unwrap()
        );
        let mut embedder = Embedder::new(&model_dir, 384).unwrap();
        let a = embedder.embed("太热了").unwrap();
        let b = embedder.embed("温度偏高").unwrap();
        let sim = cosine_similarity(&a, &b);
        assert!(sim > 0.6, "expected > 0.6, got {sim}");
    }

    #[test]
    #[ignore]
    fn unrelated_sentences_low_similarity() {
        let model_dir = format!(
            "{}/.mysis/models/bge-small-zh-v1.5",
            std::env::var("HOME").unwrap()
        );
        let mut embedder = Embedder::new(&model_dir, 384).unwrap();
        let a = embedder.embed("太热了").unwrap();
        let b = embedder.embed("客厅灯").unwrap();
        let sim = cosine_similarity(&a, &b);
        assert!(sim < 0.5, "expected < 0.5, got {sim}");
    }
}
