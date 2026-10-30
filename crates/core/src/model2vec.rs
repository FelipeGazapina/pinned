//! potion-multilingual-128M, a Model2Vec model: a vector per token, averaged. It runs in pure
//! Rust with no inference library, and the rows are read from disk as tokens need them, so the
//! 512 MB matrix is never held in memory.

use crate::memory::Embedder;
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

const REPO: &str = "minishlab/potion-multilingual-128M";
const REVISION: &str = "73908c3438cf03b6a01bcb9611d62b23d0726f08";
/// Each file and the SHA-256 Hugging Face lists for it at `REVISION`.
const FILES: [(&str, &str); 2] = [
    ("tokenizer.json", "19f1909063da3cfe3bd83a782381f040dccea475f4816de11116444a73e1b6a1"),
    ("model.safetensors", "14b5eb39cb4ce5666da8ad1f3dc6be4346e9b2d601c073302fa0a31bf7943397"),
];
/// Model2Vec reads at most this many tokens of a text.
const MAX_TOKENS: usize = 512;

/// The model's folder under `models`, downloading whatever file is missing. A download goes to a
/// `.part` file and is renamed only once its checksum matches, so an interrupted one starts over.
pub fn fetch(models: &Path) -> Result<PathBuf, String> {
    let dir = models.join(format!("potion-multilingual-128M-{}", &REVISION[..7]));
    std::fs::create_dir_all(&dir).map_err(|e| format!("could not make {}: {e}", dir.display()))?;
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(3600))
        .build()
        .map_err(|e| e.to_string())?;
    for (name, sha) in FILES {
        let file = dir.join(name);
        if file.exists() {
            continue;
        }
        let url = format!("https://huggingface.co/{REPO}/resolve/{REVISION}/{name}");
        let mut response = client.get(&url).send().and_then(|r| r.error_for_status()).map_err(|e| format!("could not download {url}: {e}"))?;
        let part = dir.join(format!("{name}.part"));
        let mut out = File::create(&part).map_err(|e| format!("could not write {}: {e}", part.display()))?;
        let mut hasher = Sha256::new();
        let mut buf = vec![0; 1 << 16];
        loop {
            let n = response.read(&mut buf).map_err(|e| format!("could not download {url}: {e}"))?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            out.write_all(&buf[..n]).map_err(|e| format!("could not write {}: {e}", part.display()))?;
        }
        let got: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
        if got != sha {
            let _ = std::fs::remove_file(&part);
            return Err(format!("{url} has checksum {got}, not {sha}"));
        }
        std::fs::rename(&part, &file).map_err(|e| format!("could not write {}: {e}", file.display()))?;
    }
    Ok(dir)
}

pub struct Model2Vec {
    tokenizer: tokenizers::Tokenizer,
    rows: Mutex<File>,
    /// Where the embedding matrix starts in `model.safetensors`.
    start: u64,
    dims: usize,
    unknown: Option<u32>,
    name: String,
}

impl Model2Vec {
    /// Loads the model `fetch` put in `dir`.
    pub fn load(dir: &Path) -> Result<Model2Vec, String> {
        let tokenizer = tokenizers::Tokenizer::from_file(dir.join("tokenizer.json")).map_err(|e| format!("could not read the tokenizer: {e}"))?;
        let path = dir.join("model.safetensors");
        let mut file = File::open(&path).map_err(|e| format!("could not open {}: {e}", path.display()))?;
        // safetensors: an 8-byte little-endian header length, a JSON header, then the tensors.
        let mut len = [0; 8];
        file.read_exact(&mut len).map_err(|e| e.to_string())?;
        let mut header = vec![0; u64::from_le_bytes(len) as usize];
        file.read_exact(&mut header).map_err(|e| e.to_string())?;
        let header: serde_json::Value = serde_json::from_slice(&header).map_err(|e| format!("bad safetensors header: {e}"))?;
        let embeddings = &header["embeddings"];
        let (Some(dims), Some(offset), Some("F32")) = (embeddings["shape"][1].as_u64(), embeddings["data_offsets"][0].as_u64(), embeddings["dtype"].as_str()) else {
            return Err(format!("{} has no f32 embeddings matrix", path.display()));
        };
        Ok(Model2Vec {
            unknown: tokenizer.token_to_id("<unk>"),
            tokenizer,
            rows: Mutex::new(file),
            start: 8 + u64::from_le_bytes(len) + offset,
            dims: dims as usize,
            name: format!("potion-multilingual-128M@{}", &REVISION[..7]),
        })
    }
}

impl Embedder for Model2Vec {
    fn model(&self) -> &str {
        &self.name
    }

    /// The mean of the text's token rows, scaled to unit length. Unknown tokens carry no meaning
    /// and are left out, as Model2Vec does.
    fn embed(&self, text: &str) -> Vec<f32> {
        let mut sum = vec![0.0f32; self.dims];
        let Ok(encoding) = self.tokenizer.encode_fast(text, false) else { return sum };
        let mut row = vec![0u8; self.dims * 4];
        let mut rows = self.rows.lock().unwrap();
        for &id in encoding.get_ids().iter().filter(|id| Some(**id) != self.unknown).take(MAX_TOKENS) {
            let at = self.start + id as u64 * row.len() as u64;
            if rows.seek(SeekFrom::Start(at)).and_then(|_| rows.read_exact(&mut row)).is_err() {
                continue;
            }
            for (s, b) in sum.iter_mut().zip(row.as_chunks::<4>().0) {
                *s += f32::from_le_bytes(*b);
            }
        }
        let norm = sum.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-12);
        sum.iter_mut().for_each(|x| *x /= norm);
        sum
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Downloads the real model (530 MB) into `PINNED_MODELS`, or the temp folder, on first run.
    #[test]
    #[ignore]
    fn the_real_model_finds_the_same_fact_across_portuguese_and_english() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let models = std::env::var_os("PINNED_MODELS").map_or_else(|| std::env::temp_dir().join("pinned-models"), PathBuf::from);
        let model = Model2Vec::load(&fetch(&models).unwrap()).unwrap();
        let memories = [
            "Deploy do hub: O hub faz deploy sozinho no Railway a partir da branch railway-hub.",
            "Package manager: Use pnpm, not npm. The lockfile is pnpm-lock.yaml.",
            "Autenticação: O login usa magic link por e-mail; não existe senha.",
            "Rate limiting: The public API allows 60 requests per minute per account.",
            "Fuso horário: Datas são guardadas em UTC e mostradas no fuso America/Sao_Paulo.",
        ];
        let vectors: Vec<Vec<f32>> = memories.iter().map(|m| model.embed(m)).collect();
        let nearest = |query: &str| {
            let q = model.embed(query);
            let cos = |v: &Vec<f32>| v.iter().zip(&q).map(|(a, b)| a * b).sum::<f32>();
            (0..memories.len()).max_by(|a, b| cos(&vectors[*a]).total_cmp(&cos(&vectors[*b]))).unwrap()
        };
        assert_eq!(nearest("how does the hub get deployed?"), 0);
        assert_eq!(nearest("qual gerenciador de pacotes devo usar?"), 1);
        assert_eq!(nearest("how do users sign in?"), 2);
        assert_eq!(nearest("quantas requisições por minuto a API aceita?"), 3);
        assert_eq!(nearest("what timezone do we show dates in?"), 4);
        assert_eq!(model.model(), "potion-multilingual-128M@73908c3");
        let unit: f32 = vectors[0].iter().map(|x| x * x).sum();
        assert!((unit - 1.0).abs() < 1e-4, "{unit}");
    }
}
