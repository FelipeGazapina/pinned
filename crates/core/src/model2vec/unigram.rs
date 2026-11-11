//! The Unigram model of `tokenizers`, with its vocabulary held flat. The crate's own `Unigram`
//! keeps a hash map per trie node and a `String` per token, about 540 MB for potion's 500,353
//! tokens. This one keeps the same tokens in about 14 MB and splits text the same way.

use serde::de::{Deserializer, Error, SeqAccess, Visitor};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tokenizers::models::unigram::UnigramError;
use tokenizers::{AddedToken, Model, Token, Trainer};

/// How far below the rarest token `tokenizers` scores an unknown character.
const UNKNOWN_PENALTY: f64 = 10.0;

#[derive(Deserialize)]
#[serde(try_from = "Fields")]
pub struct Unigram {
    /// Every token's bytes, in id order.
    bytes: Vec<u8>,
    /// Where each id's token ends in `bytes`.
    ends: Vec<u32>,
    scores: Vec<f64>,
    /// The ids in the byte order of their tokens, for prefix and exact search. A token listed
    /// twice keeps only its last id, as the crate's token map does.
    sorted: Vec<u32>,
    min_score: f64,
    unk_id: Option<u32>,
    byte_fallback: bool,
}

/// The `model` object of tokenizer.json.
#[derive(Deserialize)]
struct Fields {
    #[serde(rename = "type")]
    kind: Option<String>,
    vocab: Vocab,
    unk_id: Option<usize>,
    #[serde(default)]
    byte_fallback: bool,
}

/// The `[token, score]` pairs in id order, read without keeping a `String` per token.
#[derive(Default)]
struct Vocab {
    bytes: Vec<u8>,
    ends: Vec<u32>,
    scores: Vec<f64>,
}

impl<'de> Deserialize<'de> for Vocab {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Vocab, D::Error> {
        struct Pairs;
        impl<'de> Visitor<'de> for Pairs {
            type Value = Vocab;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a list of [token, score] pairs")
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Vocab, A::Error> {
                let mut vocab = Vocab::default();
                while let Some((token, score)) = seq.next_element::<(String, f64)>()? {
                    vocab.bytes.extend_from_slice(token.as_bytes());
                    vocab.ends.push(u32::try_from(vocab.bytes.len()).map_err(|_| A::Error::custom("the vocabulary is over 4 GB"))?);
                    vocab.scores.push(score);
                }
                Ok(vocab)
            }
        }
        deserializer.deserialize_seq(Pairs)
    }
}

impl TryFrom<Fields> for Unigram {
    type Error = String;

    fn try_from(Fields { kind, vocab, unk_id, byte_fallback }: Fields) -> Result<Unigram, String> {
        if let Some(kind) = kind.filter(|kind| *kind != "Unigram") {
            return Err(format!("a Unigram model, not {kind}"));
        }
        let Vocab { mut bytes, mut ends, mut scores } = vocab;
        let count = u32::try_from(ends.len()).map_err(|_| "the vocabulary has over 2^32 tokens")?;
        match unk_id {
            Some(_) if ends.is_empty() => return Err(UnigramError::EmptyVocabulary.to_string()),
            Some(id) if id >= ends.len() => return Err(UnigramError::UnkIdNotInVocabulary.to_string()),
            _ => {}
        }
        bytes.shrink_to_fit();
        ends.shrink_to_fit();
        scores.shrink_to_fit();
        let mut model = Unigram {
            bytes,
            ends,
            min_score: scores.iter().copied().fold(f64::INFINITY, f64::min),
            scores,
            sorted: Vec::new(),
            unk_id: unk_id.map(|id| id as u32),
            byte_fallback,
        };
        let mut sorted: Vec<u32> = (0..count).collect();
        // Equal tokens sort by descending id, so `dedup_by` keeps the last one.
        sorted.sort_unstable_by(|a, b| model.token(*a).cmp(model.token(*b)).then(b.cmp(a)));
        sorted.dedup_by(|a, b| model.token(*a) == model.token(*b));
        sorted.shrink_to_fit();
        model.sorted = sorted;
        Ok(model)
    }
}

impl Unigram {
    fn token(&self, id: u32) -> &[u8] {
        let id = id as usize;
        let start = id.checked_sub(1).map_or(0, |previous| self.ends[previous] as usize);
        &self.bytes[start..self.ends[id] as usize]
    }

    /// The tokens `text` starts with, shortest first, as (length, id).
    fn prefixes<'a>(&'a self, text: &'a [u8]) -> impl Iterator<Item = (usize, u32)> + 'a {
        // `sorted[lo..hi]` holds the tokens that start with `text[..depth]`, and one equal to it
        // sorts first.
        let (mut lo, mut hi, mut depth) = (0, self.sorted.len(), 0);
        std::iter::from_fn(move || {
            while lo < hi && depth < text.len() {
                if self.token(self.sorted[lo]).len() == depth {
                    lo += 1;
                }
                let range = &self.sorted[lo..hi];
                let next = text[depth];
                let byte = |id: &u32| self.token(*id)[depth];
                hi = lo + range.partition_point(|id| byte(id) <= next);
                lo += range.partition_point(|id| byte(id) < next);
                depth += 1;
                if lo < hi && self.token(self.sorted[lo]).len() == depth {
                    return Some((depth, self.sorted[lo]));
                }
            }
            None
        })
    }

    /// The byte ranges of the pieces on the best-scoring path through `sentence`, with each run of
    /// unknown pieces as one range. This is the crate's `Unigram::encode_optimized`.
    fn split(&self, sentence: &str) -> tokenizers::Result<Vec<(usize, usize)>> {
        #[derive(Clone)]
        struct Best {
            id: u32,
            score: f64,
            start: Option<usize>,
        }
        let unknown_score = self.min_score - UNKNOWN_PENALTY;
        let mut best = vec![Best { id: 0, score: 0.0, start: None }; sentence.len() + 1];
        for (start, c) in sentence.char_indices() {
            let here = best[start].score;
            let mut whole_char = false;
            for (len, id) in self.prefixes(&sentence.as_bytes()[start..]) {
                let score = self.scores[id as usize] + here;
                let node = &mut best[start + len];
                if node.start.is_none() || score > node.score {
                    *node = Best { id, score, start: Some(start) };
                }
                whole_char |= len == c.len_utf8();
            }
            if !whole_char {
                let score = unknown_score + here;
                let node = &mut best[start + c.len_utf8()];
                if node.start.is_none() || score > node.score {
                    *node = Best {
                        id: self.unk_id.ok_or(UnigramError::MissingUnkId)?,
                        score,
                        start: Some(start),
                    };
                }
            }
        }
        let mut pieces = Vec::new();
        let mut end = sentence.len();
        // Where the run of unknown pieces being walked back over ends.
        let mut unknown_end = None;
        while let Some(start) = best[end].start {
            if Some(best[end].id) == self.unk_id {
                unknown_end.get_or_insert(end);
            } else {
                if let Some(unknown_end) = unknown_end.take() {
                    pieces.push((end, unknown_end));
                }
                pieces.push((start, end));
            }
            end = start;
        }
        if let Some(unknown_end) = unknown_end {
            pieces.push((end, unknown_end));
        }
        pieces.reverse();
        Ok(pieces)
    }
}

impl Model for Unigram {
    type Trainer = Untrainable;

    /// The crate's `Unigram::tokenize`. A piece's id is looked up by its text, since a fused run
    /// of unknown pieces can itself be a token.
    fn tokenize(&self, sentence: &str) -> tokenizers::Result<Vec<Token>> {
        let mut tokens = Vec::new();
        for (start, end) in self.split(sentence)? {
            let piece = &sentence[start..end];
            if let Some(id) = self.token_to_id(piece) {
                tokens.push(Token::new(id, piece.to_owned(), (start, end)));
                continue;
            }
            if self.byte_fallback {
                let spelled: Option<Vec<Token>> = piece
                    .bytes()
                    .map(|byte| {
                        let value = format!("<0x{byte:02X}>");
                        self.token_to_id(&value).map(|id| Token::new(id, value, (start, end)))
                    })
                    .collect();
                if let Some(spelled) = spelled {
                    tokens.extend(spelled);
                    continue;
                }
            }
            let unknown = self.unk_id.ok_or(UnigramError::MissingUnkId)?;
            tokens.push(Token::new(unknown, piece.to_owned(), (start, end)));
        }
        Ok(tokens)
    }

    fn token_to_id(&self, token: &str) -> Option<u32> {
        let at = self.sorted.binary_search_by(|id| self.token(*id).cmp(token.as_bytes())).ok()?;
        Some(self.sorted[at])
    }

    fn id_to_token(&self, id: u32) -> Option<String> {
        ((id as usize) < self.ends.len()).then(|| String::from_utf8_lossy(self.token(id)).into_owned())
    }

    fn get_vocab(&self) -> HashMap<String, u32> {
        self.sorted.iter().map(|&id| (String::from_utf8_lossy(self.token(id)).into_owned(), id)).collect()
    }

    fn get_vocab_size(&self) -> usize {
        self.ends.len()
    }

    fn save(&self, _folder: &Path, _prefix: Option<&str>) -> tokenizers::Result<Vec<PathBuf>> {
        Err("this Unigram model only loads; it cannot be saved".into())
    }

    fn get_trainer(&self) -> Untrainable {
        Untrainable
    }
}

/// `Model` asks for a trainer, but this model only loads one that is already trained.
pub struct Untrainable;

impl Trainer for Untrainable {
    type Model = Unigram;

    fn should_show_progress(&self) -> bool {
        false
    }

    fn train(&self, _model: &mut Unigram) -> tokenizers::Result<Vec<AddedToken>> {
        Err("this Unigram model cannot be trained".into())
    }

    fn feed<I, S, F>(&mut self, _iterator: I, _process: F) -> tokenizers::Result<()>
    where
        I: Iterator<Item = S> + Send,
        S: AsRef<str> + Send,
        F: Fn(&str) -> tokenizers::Result<Vec<String>> + Sync,
    {
        Err("this Unigram model cannot be trained".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokenizers::models::unigram::Unigram as CrateUnigram;

    fn model(vocab: &[(&str, f64)], unk_id: Option<usize>, byte_fallback: bool) -> Unigram {
        let json = serde_json::json!({ "type": "Unigram", "unk_id": unk_id, "vocab": vocab, "byte_fallback": byte_fallback });
        serde_json::from_value(json).unwrap()
    }

    fn assert_tokens(model: &Unigram, text: &str, expected: &[(u32, &str, (usize, usize))]) {
        let got: Vec<(u32, String, (usize, usize))> = model.tokenize(text).unwrap().into_iter().map(|t| (t.id, t.value, t.offsets)).collect();
        let expected: Vec<(u32, String, (usize, usize))> = expected.iter().map(|(id, value, offsets)| (*id, value.to_string(), *offsets)).collect();
        assert_eq!(got, expected, "{text:?}");
    }

    #[test]
    fn runs_of_unknown_characters_fuse_into_one_token() {
        let model = model(
            &[("<unk>", 0.0), ("a", 0.0), ("b", 0.0), ("c", 0.0), ("d", 0.0), ("cd", 1.0), ("ab", 2.0), ("abc", 5.0), ("abcd", 10.0)],
            Some(0),
            false,
        );
        assert_tokens(&model, "abcdacdxx", &[(8, "abcd", (0, 4)), (1, "a", (4, 5)), (5, "cd", (5, 7)), (0, "xx", (7, 9))]);
    }

    #[test]
    fn a_tie_keeps_the_path_that_reached_it_first() {
        let one_start = model(&[("<unk>", 0.0), ("a", -1.0), ("b", -1.0), ("ab", -2.0)], Some(0), false);
        assert_tokens(&one_start, "ab", &[(3, "ab", (0, 2))]);
        let two_starts = model(&[("<unk>", 0.0), ("a", -1.0), ("bc", -1.0), ("ab", -1.0), ("c", -1.0)], Some(0), false);
        assert_tokens(&two_starts, "abc", &[(1, "a", (0, 1)), (2, "bc", (1, 3))]);
    }

    #[test]
    fn a_token_listed_twice_takes_its_last_id_and_score() {
        let model = model(&[("<unk>", 0.0), ("ab", -3.0), ("a", -1.0), ("b", -1.0), ("a", -5.0)], Some(0), false);
        assert_tokens(&model, "ab", &[(1, "ab", (0, 2))]);
        assert_tokens(&model, "ba", &[(3, "b", (0, 1)), (4, "a", (1, 2))]);
        assert_eq!((model.get_vocab_size(), model.id_to_token(2).as_deref()), (5, Some("a")));
    }

    #[test]
    fn offsets_count_the_bytes_of_multibyte_characters() {
        // é and ß share their first byte, so the search for ß narrows to é before it fails.
        let model = model(&[("<unk>", 0.0), ("東", -1.0), ("京", -1.0), ("東京", -1.5), ("é", -1.0)], Some(0), false);
        assert_tokens(&model, "東京éß🙂東", &[(3, "東京", (0, 6)), (4, "é", (6, 8)), (0, "ß🙂", (8, 14)), (1, "東", (14, 17))]);
    }

    #[test]
    fn empty_text_has_no_tokens() {
        let model = model(&[("<unk>", 0.0), ("a", -1.0)], Some(0), false);
        assert_tokens(&model, "", &[]);
        assert_tokens(&model, "a", &[(1, "a", (0, 1))]);
    }

    #[test]
    fn a_fused_unknown_run_that_is_itself_a_token_takes_that_tokens_id() {
        // With scores this high, two unknown characters outscore the token "ab".
        let model = model(&[("<unk>", 100.0), ("ab", 100.0)], Some(0), false);
        assert_tokens(&model, "ab", &[(1, "ab", (0, 2))]);
    }

    #[test]
    fn byte_fallback_spells_an_unknown_piece_in_byte_tokens() {
        let model = model(&[("<unk>", 0.0), ("<0xC3>", -0.01), ("<0xA9>", -0.03)], Some(0), true);
        assert_tokens(&model, "é", &[(1, "<0xC3>", (0, 2)), (2, "<0xA9>", (0, 2))]);
        assert_tokens(&model, "?é", &[(0, "?é", (0, 3))]);
    }

    #[test]
    fn an_unknown_character_without_an_unknown_token_is_an_error() {
        let model = model(&[("a", -1.0)], None, false);
        assert_tokens(&model, "a", &[(0, "a", (0, 1))]);
        assert_eq!(model.tokenize("ab").unwrap_err().to_string(), "Encountered an unknown token but `unk_id` is missing");
    }

    #[test]
    fn tokenizes_generated_text_as_the_crates_unigram_does() {
        // é and ß share a leading byte, as do 東 and 京; x is in no vocabulary.
        let alphabet = ['a', 'b', 'c', 'é', 'ß', '東', '京', '🙂', '▁', 'x'];
        let mut seed = 0x9E37_79B9_7F4A_7C15u64;
        let mut below = |n: usize| {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            (seed >> 33) as usize % n
        };
        for round in 0..60 {
            let mut vocab: Vec<(String, f64)> = Vec::new();
            for _ in 0..below(40) + 1 {
                let token: String = (0..below(4) + 1).map(|_| alphabet[below(alphabet.len() - 1)]).collect();
                // Few distinct scores, so paths tie. Positive ones let unknown runs outscore tokens.
                vocab.push((token, [-3.0, -2.0, -1.0, -0.5, 0.0, 2.0][below(6)]));
                if below(8) == 0 {
                    let again = vocab[below(vocab.len())].0.clone();
                    vocab.push((again, -1.5));
                }
            }
            let byte_fallback = round % 4 == 3;
            if byte_fallback {
                vocab.extend(["<0xC3>", "<0x9F>", "<0x78>", "<0xF0>"].map(|b| (b.to_string(), -4.0)));
            }
            let unk_id = (round % 5 != 4).then(|| below(vocab.len()));
            let theirs = CrateUnigram::from(vocab.clone(), unk_id, byte_fallback).unwrap();
            let json = serde_json::json!({ "type": "Unigram", "unk_id": unk_id, "vocab": vocab, "byte_fallback": byte_fallback });
            let ours: Unigram = serde_json::from_value(json).unwrap();
            assert_eq!(ours.get_vocab(), theirs.get_vocab(), "round {round}");
            assert_eq!(ours.get_vocab_size(), theirs.get_vocab_size(), "round {round}");
            for id in 0..vocab.len() as u32 + 1 {
                assert_eq!(ours.id_to_token(id), theirs.id_to_token(id), "round {round}");
            }
            for _ in 0..80 {
                let text: String = (0..below(12)).map(|_| alphabet[below(alphabet.len())]).collect();
                match (ours.tokenize(&text), theirs.tokenize(&text)) {
                    (Ok(ours), Ok(theirs)) => assert_eq!(ours, theirs, "round {round}, {text:?}"),
                    (Err(ours), Err(theirs)) => assert_eq!(ours.to_string(), theirs.to_string(), "round {round}, {text:?}"),
                    (ours, theirs) => panic!("round {round}, {text:?}: ours {ours:?}, the crate's {theirs:?}"),
                }
            }
        }
    }
}
