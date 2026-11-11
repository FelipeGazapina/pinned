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
use tokenizers::{DecoderWrapper, NormalizerWrapper, PostProcessorWrapper, PreTokenizerWrapper, TokenizerImpl};

mod unigram;

/// The crate's tokenizer with its Unigram model swapped for ours, which needs 14 MB, not 540.
type Tokenizer = TokenizerImpl<unigram::Unigram, NormalizerWrapper, PreTokenizerWrapper, PostProcessorWrapper, DecoderWrapper>;

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
    tokenizer: Tokenizer,
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
        let tokenizer = Tokenizer::from_file(dir.join("tokenizer.json")).map_err(|e| format!("could not read the tokenizer: {e}"))?;
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

    /// The process's resident size in MB, as `ps` reports it.
    fn resident_mb() -> f64 {
        let out = crate::process::command("ps").args(["-o", "rss=", "-p", &std::process::id().to_string()]).output().unwrap();
        String::from_utf8_lossy(&out.stdout).trim().parse::<f64>().unwrap() / 1024.0
    }

    /// Lines of this repository's README and core sources, hand-written lines in other scripts,
    /// and lines of random characters from those scripts.
    fn corpus() -> Vec<String> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut files = vec![root.join("../../README.md")];
        for entry in std::fs::read_dir(root.join("src")).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|e| e == "rs") {
                files.push(path);
            }
        }
        let mut lines: Vec<String> = files.iter().flat_map(|f| std::fs::read_to_string(f).unwrap().lines().map(str::to_owned).collect::<Vec<_>>()).collect();
        let written = [
            "Não existe senha: o login usa um link mágico enviado por e-mail.",
            "Ação, coração, pão, órgão, açúcar, avó, vovô, pôr do sol às 18h.",
            "Qual gerenciador de pacotes devo usar neste repositório?",
            "¿Dónde está la estación de tren? Mañana llegaré temprano, señor.",
            "El niño comió paella y bebió horchata en Valencia.",
            "我们在周五部署了新版本，用户反馈很好。",
            "北京和上海的天气今天都很晴朗。",
            "東京で新しいブランチをデプロイしました。ありがとうございます！",
            "カタカナとひらがなと漢字を混ぜた文章です。ｶﾀｶﾅ半角もあります。",
            "مرحبا بالعالم، كيف حالك اليوم؟",
            "اللغة العربية تكتب من اليمين إلى اليسار ١٢٣٤٥",
            "שלום עולם, מה שלומך?",
            "Привет, мир! Как дела у команды сегодня?",
            "Γειά σου κόσμε, τι κάνεις;",
            "한국어 문장도 토큰으로 나눠야 합니다.",
            "हिन्दी में लिखा गया वाक्य।",
            "ภาษาไทยไม่มีช่องว่างระหว่างคำ",
            "Deploy 🚀 no Railway via branch railway-hub ✅ ok 👍🏽",
            "👩‍💻👨‍👩‍👧‍👦🇧🇷🇯🇵❤️‍🔥 emoji only",
            "mixed: café 東京 مرحبا Привет 🙂 naïve résumé",
            "Ｆｕｌｌｗｉｄｔｈ ＡＢＣ １２３ and ﬁ ligature and ① and ℌ and Å and ㍿",
            "e\u{301} combining accent vs é precomposed",
            "[UNK] [PAD] <unk> <pad> ▁ literal specials",
            "a\u{a0}b non-breaking, a\u{3000}b ideographic, a\u{200b}b zero-width",
            "tabs\tand\tnewlines\r\nmixed",
            "\u{0}\u{1}\u{7f} control characters",
            "",
            " ",
            "   ",
            "\t",
            "\n",
            "  leading spaces",
            "trailing spaces   ",
            "double  spaces  between  words",
            "x",
            "ß",
        ];
        lines.extend(written.iter().map(|s| s.to_string()));
        lines.push(written.join(" "));
        lines.push("abc".repeat(5000));
        let chars: Vec<char> = written.iter().flat_map(|s| s.chars()).collect();
        let mut seed = 0x2545_F491_4F6C_DD1Du64;
        let mut below = |n: usize| {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            (seed >> 33) as usize % n
        };
        for _ in 0..3000 {
            let len = below(40);
            lines.push((0..len).map(|_| chars[below(chars.len())]).collect());
        }
        lines.sort();
        lines.dedup();
        lines
    }

    /// Reads the model from `PINNED_MODELS` like the test above. Run it alone, so the resident
    /// sizes it prints are this test's.
    #[test]
    #[ignore]
    fn the_flat_unigram_tokenizes_as_the_crates_does_on_the_real_model() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let models = std::env::var_os("PINNED_MODELS").map_or_else(|| std::env::temp_dir().join("pinned-models"), PathBuf::from);
        let dir = fetch(&models).unwrap();
        let start = resident_mb();
        let model = Model2Vec::load(&dir).unwrap();
        let with_ours = resident_mb();
        let theirs = tokenizers::Tokenizer::from_file(dir.join("tokenizer.json")).unwrap();
        let with_both = resident_mb();
        let (ours_mb, theirs_mb) = (with_ours - start, with_both - with_ours);
        eprintln!("resident: {start:.1} MB before, {with_ours:.1} MB after ours (+{ours_mb:.1}), {with_both:.1} MB after the crate's too (+{theirs_mb:.1})");
        assert_eq!(model.unknown, theirs.token_to_id("<unk>"));
        let lines = corpus();
        let mut mismatches = 0;
        for line in &lines {
            let ours = model.tokenizer.encode_fast(line.as_str(), false).unwrap();
            let expected = theirs.encode_fast(line.as_str(), false).unwrap();
            if ours.get_ids() != expected.get_ids() || ours.get_tokens() != expected.get_tokens() {
                if mismatches < 10 {
                    eprintln!("{line:?}: ours {:?}, the crate's {:?}", ours.get_tokens(), expected.get_tokens());
                }
                mismatches += 1;
            }
        }
        eprintln!("compared {} lines, {mismatches} mismatches", lines.len());
        assert_eq!(mismatches, 0);
    }
}
