//! A project's memory: facts its threads save and search, one SQLite database per project.
//! Search fuses a full-text ranking with a vector ranking, so a Portuguese question finds an
//! English memory; without vectors it is word search alone.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
pub struct Memory {
    pub id: i64,
    pub title: String,
    pub text: String,
    /// What sort of fact it is, such as a preference, a decision or a convention.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// Unix seconds.
    pub created: i64,
    pub updated: i64,
}

/// A memory someone asked to save: trimmed, with a title and a text.
#[derive(Clone, Debug, PartialEq)]
pub struct Draft {
    title: String,
    text: String,
    kind: Option<String>,
}

impl Draft {
    pub fn new(title: &str, text: &str, kind: Option<&str>) -> Result<Draft, String> {
        let (title, text) = (title.trim(), text.trim());
        if title.is_empty() || text.is_empty() {
            return Err("a memory needs a title and a text".into());
        }
        let kind = kind.map(str::trim).filter(|k| !k.is_empty()).map(str::to_owned);
        Ok(Draft { title: title.to_owned(), text: text.to_owned(), kind })
    }

    fn passage(&self) -> String {
        passage(&self.title, &self.text)
    }
}

/// What the vectors are made of: the title leads, since it names the subject.
fn passage(title: &str, text: &str) -> String {
    format!("{title}: {text}")
}

#[derive(Clone, Debug, PartialEq)]
pub enum Saved {
    Added(Memory),
    Updated(Memory),
    /// Nothing was saved: this memory already says nearly the same.
    Duplicate(Memory),
}

/// Turns text into a unit-length vector.
pub trait Embedder: Send + Sync {
    /// Names the model, so a row embedded by another model is embedded again rather than compared.
    fn model(&self) -> &str;
    fn embed(&self, text: &str) -> Vec<f32>;
}

/// Cosine at or above which a new memory repeats an old one. potion-multilingual-128M puts
/// same-language paraphrases at 0.92 to 0.95 and related but different facts below 0.7.
const DUPLICATE: f32 = 0.9;
/// How many rows each ranking hands to the fusion.
const ARM: usize = 40;
/// The least cosine a vector match needs. potion-multilingual-128M scores a relevant memory
/// 0.21 to 0.6 and an unrelated one around 0.14, and gives a bare "ok, continue" up to 0.39
/// against something. `memory_search` is asked on purpose and keeps the weak matches.
const SEARCH_FLOOR: f32 = 0.2;
/// Each prompt is searched without being asked, so only closer matches are added to it.
const RECALL_FLOOR: f32 = 0.3;
const RECALL_LIMIT: usize = 5;
/// About 400 tokens.
const RECALL_CHARS: usize = 1600;
const RECALL_TEXT_CHARS: usize = 400;
/// How long a failed model load waits before the next memory asks for it again.
const RETRY: Duration = Duration::from_secs(600);
/// Reciprocal Rank Fusion's constant, as in the paper and in coffee-fueled-dev/agent.
const RRF_K: f64 = 60.0;

const SCHEMA: &str = "
CREATE TABLE memories(
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    title TEXT NOT NULL,
    text TEXT NOT NULL,
    kind TEXT,
    created INTEGER NOT NULL,
    updated INTEGER NOT NULL,
    embedding BLOB,
    model TEXT
);
CREATE VIRTUAL TABLE memories_fts USING fts5(title, text, content='memories', content_rowid='id', tokenize='unicode61 remove_diacritics 2');
CREATE TRIGGER memories_ai AFTER INSERT ON memories BEGIN
    INSERT INTO memories_fts(rowid, title, text) VALUES (new.id, new.title, new.text);
END;
CREATE TRIGGER memories_ad AFTER DELETE ON memories BEGIN
    INSERT INTO memories_fts(memories_fts, rowid, title, text) VALUES ('delete', old.id, old.title, old.text);
END;
CREATE TRIGGER memories_au AFTER UPDATE OF title, text ON memories BEGIN
    INSERT INTO memories_fts(memories_fts, rowid, title, text) VALUES ('delete', old.id, old.title, old.text);
    INSERT INTO memories_fts(rowid, title, text) VALUES (new.id, new.title, new.text);
END;
";

pub struct Store {
    db: Connection,
}

fn sql(e: rusqlite::Error) -> String {
    format!("memory database: {e}")
}

impl Store {
    /// Opens the database at `file`, creating and migrating it as needed.
    pub fn open(file: &Path) -> Result<Store, String> {
        let db = Connection::open(file).map_err(sql)?;
        db.pragma_update(None, "journal_mode", "WAL").map_err(sql)?;
        let version: i64 = db.pragma_query_value(None, "user_version", |r| r.get(0)).map_err(sql)?;
        if version < 1 {
            db.execute_batch(&format!("BEGIN; {SCHEMA} PRAGMA user_version = 1; COMMIT;")).map_err(sql)?;
        }
        Ok(Store { db })
    }

    /// Adds `draft`, or with `id` replaces that memory's title, text and kind. A new memory that
    /// repeats an old one, by title or by meaning, is not added.
    pub fn save(&self, draft: &Draft, id: Option<i64>, vectors: Option<&dyn Embedder>, now: i64) -> Result<Saved, String> {
        let vector = vectors.map(|e| (e.model(), e.embed(&draft.passage())));
        let (model, blob) = vector.as_ref().map(|(m, v)| (Some(*m), Some(to_blob(v)))).unwrap_or_default();
        if let Some(id) = id {
            let changed = self
                .db
                .execute(
                    "UPDATE memories SET title = ?2, text = ?3, kind = ?4, updated = ?5, embedding = ?6, model = ?7 WHERE id = ?1",
                    params![id, draft.title, draft.text, draft.kind, now, blob, model],
                )
                .map_err(sql)?;
            if changed == 0 {
                return Err(format!("no memory #{id}"));
            }
            return Ok(Saved::Updated(self.get(id)?));
        }
        if let Some(old) = self.duplicate(draft, vector.as_ref().map(|(m, v)| (*m, v.as_slice())))? {
            return Ok(Saved::Duplicate(self.get(old)?));
        }
        self.db
            .execute(
                "INSERT INTO memories(title, text, kind, created, updated, embedding, model) VALUES (?1, ?2, ?3, ?4, ?4, ?5, ?6)",
                params![draft.title, draft.text, draft.kind, now, blob, model],
            )
            .map_err(sql)?;
        Ok(Saved::Added(self.get(self.db.last_insert_rowid())?))
    }

    fn duplicate(&self, draft: &Draft, vector: Option<(&str, &[f32])>) -> Result<Option<i64>, String> {
        let same_title = self.db.query_row("SELECT id FROM memories WHERE title = ?1 COLLATE NOCASE LIMIT 1", [&draft.title], |r| r.get(0)).optional().map_err(sql)?;
        if same_title.is_some() {
            return Ok(same_title);
        }
        let Some((model, vector)) = vector else { return Ok(None) };
        Ok(self.by_meaning(model, vector, DUPLICATE)?.first().copied())
    }

    pub fn forget(&self, id: i64) -> Result<Memory, String> {
        let memory = self.get(id)?;
        self.db.execute("DELETE FROM memories WHERE id = ?1", [id]).map_err(sql)?;
        Ok(memory)
    }

    pub fn get(&self, id: i64) -> Result<Memory, String> {
        self.db
            .query_row("SELECT id, title, text, kind, created, updated FROM memories WHERE id = ?1", [id], row)
            .optional()
            .map_err(sql)?
            .ok_or_else(|| format!("no memory #{id}"))
    }

    pub fn count(&self) -> Result<u64, String> {
        self.db.query_row("SELECT count(*) FROM memories", [], |r| r.get::<_, i64>(0)).map(|n| n as u64).map_err(sql)
    }

    /// The memories that best match `query`, best first. An empty query lists the latest ones.
    /// `floor` drops vector matches less similar than it, since every memory has some similarity.
    pub fn search(&self, query: &str, vectors: Option<&dyn Embedder>, floor: f32, limit: usize) -> Result<Vec<Memory>, String> {
        if query.trim().is_empty() {
            return self.recent(limit);
        }
        let words = self.by_words(query)?;
        let meaning = match vectors {
            Some(e) => self.by_meaning(e.model(), &e.embed(query), floor)?,
            None => vec![],
        };
        fuse(&[words, meaning], limit).into_iter().map(|id| self.get(id)).collect()
    }

    fn recent(&self, limit: usize) -> Result<Vec<Memory>, String> {
        let mut statement = self.db.prepare("SELECT id, title, text, kind, created, updated FROM memories ORDER BY updated DESC, id DESC LIMIT ?1").map_err(sql)?;
        let rows = statement.query_map([limit as i64], row).map_err(sql)?;
        rows.collect::<Result<_, _>>().map_err(sql)
    }

    fn by_words(&self, query: &str) -> Result<Vec<i64>, String> {
        let Some(query) = fts_query(query) else { return Ok(vec![]) };
        let mut statement = self.db.prepare("SELECT rowid FROM memories_fts WHERE memories_fts MATCH ?1 ORDER BY rank LIMIT ?2").map_err(sql)?;
        let rows = statement.query_map(params![query, ARM as i64], |r| r.get(0)).map_err(sql)?;
        rows.collect::<Result<_, _>>().map_err(sql)
    }

    /// An exact scan: a project keeps hundreds of memories, not millions.
    fn by_meaning(&self, model: &str, query: &[f32], floor: f32) -> Result<Vec<i64>, String> {
        let mut statement = self.db.prepare("SELECT id, embedding FROM memories WHERE model = ?1 AND embedding IS NOT NULL").map_err(sql)?;
        let rows = statement.query_map([model], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Vec<u8>>(1)?))).map_err(sql)?;
        let mut scored = vec![];
        for row in rows {
            let (id, blob) = row.map_err(sql)?;
            let similarity: f32 = from_blob(&blob).zip(query).map(|(a, b)| a * b).sum();
            if similarity >= floor {
                scored.push((similarity, id));
            }
        }
        scored.sort_by(|a, b| b.0.total_cmp(&a.0).then(b.1.cmp(&a.1)));
        Ok(scored.into_iter().take(ARM).map(|(_, id)| id).collect())
    }

    /// Embeds every memory that has no vector from `vectors`' model, and says how many it did.
    pub fn embed_missing(&self, vectors: &dyn Embedder) -> Result<usize, String> {
        let mut statement = self.db.prepare("SELECT id, title, text FROM memories WHERE model IS NOT ?1 OR embedding IS NULL").map_err(sql)?;
        let rows = statement.query_map([vectors.model()], |r| Ok((r.get::<_, i64>(0)?, passage(&r.get::<_, String>(1)?, &r.get::<_, String>(2)?)))).map_err(sql)?;
        let missing: Vec<(i64, String)> = rows.collect::<Result<_, _>>().map_err(sql)?;
        for (id, passage) in &missing {
            self.db
                .execute("UPDATE memories SET embedding = ?2, model = ?3 WHERE id = ?1", params![id, to_blob(&vectors.embed(passage)), vectors.model()])
                .map_err(sql)?;
        }
        Ok(missing.len())
    }
}

fn row(r: &rusqlite::Row) -> rusqlite::Result<Memory> {
    Ok(Memory { id: r.get(0)?, title: r.get(1)?, text: r.get(2)?, kind: r.get(3)?, created: r.get(4)?, updated: r.get(5)? })
}

fn to_blob(vector: &[f32]) -> Vec<u8> {
    vector.iter().flat_map(|x| x.to_le_bytes()).collect()
}

fn from_blob(blob: &[u8]) -> impl Iterator<Item = f32> + '_ {
    blob.as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b))
}

/// Words too common to say what a prompt is about. Matching them ranks every memory that uses
/// them, which drowns the vector ranking in the fusion.
const STOP_WORDS: &[&str] = &[
    "about", "all", "an", "and", "any", "are", "as", "at", "be", "been", "but", "by", "can", "could", "did", "do", "does", "for", "from", "had",
    "has", "have", "how", "if", "in", "into", "is", "it", "its", "me", "my", "need", "not", "of", "on", "or", "our", "should", "so", "some",
    "than", "that", "the", "them", "then", "there", "they", "this", "to", "use", "was", "we", "were", "what", "when", "where", "which", "who", "why",
    "will", "with", "would", "you", "your", "ao", "com", "como", "da", "das", "de", "do", "dos", "ela", "ele", "em", "esse", "essa",
    "esta", "este", "eu", "faco", "fazer", "foi", "isso", "mais", "mas", "na", "nas", "no", "nos", "num", "numa", "onde", "os", "para", "pela",
    "pelo", "por", "posso", "qual", "quais", "quando", "que", "quero", "se", "sem", "ser", "sobre", "sao", "tem", "um", "uma", "voce",
];

/// An FTS5 query matching any word of `text` longer than a letter. Each word is quoted, so no
/// input is read as FTS syntax; nothing is left when `text` has no searchable word.
fn fts_query(text: &str) -> Option<String> {
    let mut words: Vec<String> = vec![];
    for word in text.split(|c: char| !c.is_alphanumeric()).map(str::to_lowercase) {
        let plain = fold(&word);
        if plain.chars().count() > 1 && !STOP_WORDS.contains(&plain.as_str()) && !words.contains(&word) {
            words.push(word);
        }
    }
    (!words.is_empty()).then(|| words.iter().take(32).map(|w| format!("\"{w}\"")).collect::<Vec<_>>().join(" OR "))
}

/// The word without the accents Portuguese puts on stop words, to look it up in `STOP_WORDS`.
fn fold(word: &str) -> String {
    word.chars()
        .map(|c| match c {
            'á' | 'à' | 'â' | 'ã' => 'a',
            'é' | 'ê' => 'e',
            'í' => 'i',
            'ó' | 'ô' | 'õ' => 'o',
            'ú' => 'u',
            'ç' => 'c',
            c => c,
        })
        .collect()
}

/// Reciprocal Rank Fusion: each ranking gives an id `1 / (60 + rank)`, and the sums order the ids.
fn fuse(rankings: &[Vec<i64>], limit: usize) -> Vec<i64> {
    let mut scores: HashMap<i64, f64> = HashMap::new();
    for ranking in rankings {
        for (rank, id) in ranking.iter().enumerate() {
            *scores.entry(*id).or_default() += 1.0 / (RRF_K + rank as f64 + 1.0);
        }
    }
    let mut ids: Vec<(f64, i64)> = scores.into_iter().map(|(id, score)| (score, id)).collect();
    ids.sort_by(|a, b| b.0.total_cmp(&a.0).then(b.1.cmp(&a.1)));
    ids.into_iter().take(limit).map(|(_, id)| id).collect()
}

/// Builds the vector model. The first time, that downloads it, which can take minutes.
pub type Loader = Arc<dyn Fn() -> Result<Arc<dyn Embedder>, String> + Send + Sync>;

enum Vectors {
    /// No project with a memory asked for the model yet.
    Idle,
    Loading,
    Ready(Arc<dyn Embedder>),
    Failed(Instant),
}

/// Whether a search also matches by meaning, as the Memory panel shows it.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum VectorSearch {
    Ready,
    /// The model is downloading or loading; until then, search is by words.
    Loading,
    /// By words only: the model is not needed yet, or failed to load.
    Off,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
pub struct Status {
    pub count: u64,
    pub vectors: VectorSearch,
}

/// Every project's memory on this machine, and the vector model they share. The machine is the
/// only writer: sessions reach it through its socket.
pub struct Memories {
    /// Where each project's database lives; none keeps no memory.
    dir: Option<PathBuf>,
    load: Option<Loader>,
    stores: Mutex<HashMap<PathBuf, Arc<Mutex<Store>>>>,
    vectors: Mutex<Vectors>,
}

impl Memories {
    /// Without `load`, search is by words only.
    pub fn new(dir: Option<PathBuf>, load: Option<Loader>) -> Arc<Memories> {
        Arc::new(Memories { dir, load, stores: Mutex::default(), vectors: Mutex::new(Vectors::Idle) })
    }

    /// The project's store. A project that never saved a memory has no database, and only `create` makes one.
    fn store(self: &Arc<Self>, project: &Path, create: bool) -> Result<Option<Arc<Mutex<Store>>>, String> {
        let dir = self.dir.as_ref().ok_or("this machine keeps no memory")?;
        let mut stores = self.stores.lock().unwrap();
        if let Some(store) = stores.get(project) {
            return Ok(Some(store.clone()));
        }
        // A hash names the file, since a path can't, and the canonical path makes it stable.
        let hash: String = Sha256::digest(project.as_os_str().as_encoded_bytes())[..8].iter().map(|b| format!("{b:02x}")).collect();
        let file = dir.join(format!("{hash}.sqlite"));
        if !create && !file.exists() {
            return Ok(None);
        }
        std::fs::create_dir_all(dir).map_err(|e| format!("could not make {}: {e}", dir.display()))?;
        let store = Arc::new(Mutex::new(Store::open(&file)?));
        stores.insert(project.to_owned(), store.clone());
        drop(stores);
        if store.lock().unwrap().count()? > 0 {
            if let Some(vectors) = self.vectors() {
                let store = store.clone();
                std::thread::spawn(move || store.lock().unwrap().embed_missing(&*vectors));
            }
        }
        Ok(Some(store))
    }

    /// The vector model, once loaded. The first call starts loading it in the background, and
    /// embeds every open store's memories when it is ready.
    fn vectors(self: &Arc<Self>) -> Option<Arc<dyn Embedder>> {
        let load = self.load.clone()?;
        let mut state = self.vectors.lock().unwrap();
        match &*state {
            Vectors::Ready(vectors) => return Some(vectors.clone()),
            Vectors::Loading => return None,
            Vectors::Failed(at) if at.elapsed() < RETRY => return None,
            Vectors::Idle | Vectors::Failed(_) => *state = Vectors::Loading,
        }
        let me = self.clone();
        std::thread::spawn(move || match load() {
            Ok(vectors) => {
                *me.vectors.lock().unwrap() = Vectors::Ready(vectors.clone());
                let stores: Vec<_> = me.stores.lock().unwrap().values().cloned().collect();
                for store in stores {
                    if let Err(e) = store.lock().unwrap().embed_missing(&*vectors) {
                        eprintln!("pinned: could not embed memories: {e}");
                    }
                }
            }
            Err(e) => {
                eprintln!("pinned: memory search is by words only: {e}");
                *me.vectors.lock().unwrap() = Vectors::Failed(Instant::now());
            }
        });
        None
    }

    /// The model if it is loaded, without asking for it.
    fn ready(&self) -> Option<Arc<dyn Embedder>> {
        match &*self.vectors.lock().unwrap() {
            Vectors::Ready(vectors) => Some(vectors.clone()),
            _ => None,
        }
    }

    pub fn search(self: &Arc<Self>, project: &Path, query: &str, limit: usize) -> Result<Vec<Memory>, String> {
        match self.store(project, false)? {
            Some(store) => store.lock().unwrap().search(query, self.ready().as_deref(), SEARCH_FLOOR, limit),
            None => Ok(vec![]),
        }
    }

    /// The few memories closest to `prompt`, written for the thread to read with it, or nothing.
    pub fn recall(self: &Arc<Self>, project: &Path, prompt: &str) -> String {
        if prompt.trim().is_empty() {
            return String::new();
        }
        let Ok(Some(store)) = self.store(project, false) else { return String::new() };
        let found = store.lock().unwrap().search(prompt, self.ready().as_deref(), RECALL_FLOOR, RECALL_LIMIT).unwrap_or_default();
        let mut lines: Vec<String> = vec![];
        for m in found {
            let line = format!("[{}] {}: {}", m.id, m.title, clip(&m.text, RECALL_TEXT_CHARS));
            if !lines.is_empty() && lines.iter().map(String::len).sum::<usize>() + line.len() > RECALL_CHARS {
                break;
            }
            lines.push(line);
        }
        if lines.is_empty() {
            return String::new();
        }
        format!("Project memory pinned found for this prompt, by id (memory_search finds more; memory_save with an id corrects one):\n{}", lines.join("\n"))
    }

    /// Saving a project's first memory is what asks for the vector model.
    pub fn save(self: &Arc<Self>, project: &Path, draft: &Draft, id: Option<i64>) -> Result<Saved, String> {
        let store = self.store(project, true)?.ok_or("this machine keeps no memory")?;
        let vectors = self.vectors();
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
        let saved = store.lock().unwrap().save(draft, id, vectors.as_deref(), now);
        saved
    }

    pub fn forget(self: &Arc<Self>, project: &Path, id: i64) -> Result<Memory, String> {
        self.store(project, false)?.ok_or_else(|| format!("no memory #{id}"))?.lock().unwrap().forget(id)
    }

    pub fn status(self: &Arc<Self>, project: &Path) -> Result<Status, String> {
        let count = match self.store(project, false)? {
            Some(store) => store.lock().unwrap().count()?,
            None => 0,
        };
        let vectors = match &*self.vectors.lock().unwrap() {
            Vectors::Ready(_) => VectorSearch::Ready,
            Vectors::Loading => VectorSearch::Loading,
            Vectors::Idle | Vectors::Failed(_) => VectorSearch::Off,
        };
        Ok(Status { count, vectors })
    }
}

fn clip(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((cut, _)) => format!("{}…", &text[..cut]),
        None => text.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("pinned-memory-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Puts a text on the axis of the first topic it mentions, so tests pick which memories are alike.
    struct Topics(&'static str);

    impl Embedder for Topics {
        fn model(&self) -> &str {
            self.0
        }

        fn embed(&self, text: &str) -> Vec<f32> {
            let text = text.to_lowercase();
            let topics = [&["deploy", "ship", "publica"][..], &["test", "teste"], &["login", "senha", "auth"]];
            let mut v = vec![0.0; topics.len() + 1];
            v[topics.iter().position(|words| words.iter().any(|w| text.contains(w))).unwrap_or(topics.len())] = 1.0;
            v
        }
    }

    fn draft(title: &str, text: &str) -> Draft {
        Draft::new(title, text, None).unwrap()
    }

    fn titles(memories: Result<Vec<Memory>, String>) -> Vec<String> {
        memories.unwrap().into_iter().map(|m| format!("#{} {}", m.id, m.title)).collect()
    }

    #[test]
    fn memories_are_saved_updated_and_forgotten_and_survive_a_reopen() {
        let dir = temp("store");
        let file = dir.join("app.sqlite");
        let store = Store::open(&file).unwrap();
        assert_eq!(
            store.save(&Draft::new(" Package manager ", " Use pnpm, not npm. ", Some(" convention ")).unwrap(), None, None, 100),
            Ok(Saved::Added(Memory { id: 1, title: "Package manager".into(), text: "Use pnpm, not npm.".into(), kind: Some("convention".into()), created: 100, updated: 100 }))
        );
        assert!(matches!(store.save(&draft("Deploys", "The hub deploys from railway-hub."), None, None, 101), Ok(Saved::Added(Memory { id: 2, .. }))));
        assert_eq!(
            store.save(&draft("Deploys", "The hub deploys itself from version tags on main."), Some(2), None, 150),
            Ok(Saved::Updated(Memory { id: 2, title: "Deploys".into(), text: "The hub deploys itself from version tags on main.".into(), kind: None, created: 101, updated: 150 }))
        );
        assert_eq!(store.save(&draft("Gone", "x"), Some(9), None, 151), Err("no memory #9".into()));
        assert_eq!(Draft::new(" ", "text", None), Err("a memory needs a title and a text".into()));
        assert_eq!(store.forget(2).map(|m| m.title), Ok("Deploys".into()));
        assert_eq!(store.forget(2), Err("no memory #2".into()));
        drop(store);

        let store = Store::open(&file).unwrap();
        assert_eq!(titles(store.search("", None, 0.0, 8)), ["#1 Package manager"]);
        assert!(matches!(store.save(&draft("Tests", "Run cargo test --workspace."), None, None, 200), Ok(Saved::Added(Memory { id: 3, .. }))), "a forgotten id came back");
        assert_eq!(titles(store.search(" ", None, 0.0, 8)), ["#3 Tests", "#1 Package manager"]);
        assert_eq!(titles(store.search("deploys", None, 0.0, 8)), Vec::<String>::new(), "the forgotten memory is still in the word index");
        assert_eq!(store.count(), Ok(2));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn any_text_searches_as_plain_words() {
        let store = Store::open(Path::new(":memory:")).unwrap();
        for (title, text) in [("Deploy do hub", "O hub faz deploy no Railway, a partir da branch railway-hub."), ("Proximity", "NEAR(a b) is FTS5 syntax; title:x too.")] {
            store.save(&draft(title, text), None, None, 1).unwrap();
        }
        assert_eq!(titles(store.search("\"", None, 0.0, 8)), Vec::<String>::new());
        assert_eq!(titles(store.search("*", None, 0.0, 8)), Vec::<String>::new());
        assert_eq!(titles(store.search("NEAR(", None, 0.0, 8)), ["#2 Proximity"]);
        assert_eq!(titles(store.search("-railway", None, 0.0, 8)), ["#1 Deploy do hub"]);
        assert_eq!(titles(store.search("title:x AND OR NOT", None, 0.0, 8)), ["#2 Proximity"]);
        assert_eq!(titles(store.search("como faço o deploy do hub?", None, 0.0, 8)), ["#1 Deploy do hub"]);
        assert_eq!(titles(store.search("Railway's \"branch\"", None, 0.0, 8)), ["#1 Deploy do hub"]);
        assert_eq!(fts_query("How do I DEPLOY o hub? -now *"), Some("\"deploy\" OR \"hub\" OR \"now\"".into()));
        assert_eq!(fts_query("como é que faço?"), None);
    }

    #[test]
    fn word_and_vector_rankings_are_fused_by_reciprocal_rank() {
        assert_eq!(fuse(&[vec![1, 2, 3], vec![3, 1]], 3), [1, 3, 2]);
        assert_eq!(fuse(&[vec![4], vec![]], 8), [4]);
        assert_eq!(fuse(&[vec![5, 6], vec![6, 5]], 8), [6, 5], "a tie goes to the newer memory");

        let store = Store::open(Path::new(":memory:")).unwrap();
        let vectors = Topics("topics-1");
        for (title, text) in [("Railway", "The service deploys from the railway-hub branch."), ("Tests", "Run cargo test before each commit."), ("Login", "Users sign in with a magic link."), ("Hub logs", "The hub writes JSON logs; hub errors go to Sentry.")] {
            store.save(&draft(title, text), None, Some(&vectors), 1).unwrap();
        }
        assert_eq!(titles(store.search("onde publica o hub?", Some(&vectors), 0.5, 8)), ["#1 Railway", "#4 Hub logs"]);
        assert_eq!(titles(store.search("onde publica o hub?", None, 0.5, 8)), ["#4 Hub logs", "#1 Railway"]);
        assert_eq!(titles(store.search("esqueci a senha", Some(&vectors), 0.5, 8)), ["#3 Login"]);
        assert_eq!(titles(store.search("esqueci a senha", Some(&vectors), 0.5, 1)), ["#3 Login"]);
    }

    #[test]
    fn a_memory_that_repeats_another_is_not_added() {
        let store = Store::open(Path::new(":memory:")).unwrap();
        let vectors = Topics("topics-1");
        store.save(&draft("Deploys", "The hub deploys from railway-hub."), None, None, 1).unwrap();
        let repeat = store.save(&draft("deploys", "Something else entirely."), None, None, 2).unwrap();
        assert!(matches!(repeat, Saved::Duplicate(Memory { id: 1, .. })), "{repeat:?}");
        assert_eq!(store.embed_missing(&vectors), Ok(1));
        let repeat = store.save(&draft("Shipping", "We ship the hub on every tag."), None, Some(&vectors), 3).unwrap();
        assert!(matches!(repeat, Saved::Duplicate(Memory { id: 1, .. })), "{repeat:?}");
        assert!(matches!(store.save(&draft("Tests", "Run cargo test."), None, Some(&vectors), 4), Ok(Saved::Added(Memory { id: 2, .. }))));
        assert!(matches!(store.save(&draft("Shipping", "We ship the hub on every tag."), Some(1), Some(&vectors), 5), Ok(Saved::Updated(Memory { id: 1, .. }))));
        assert_eq!(store.count(), Ok(2));
    }

    fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
        let start = Instant::now();
        while !done() {
            assert!(start.elapsed() < Duration::from_secs(5), "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn a_project_gets_vector_search_once_its_first_memory_asks_for_the_model() {
        let dir = temp("registry");
        let (go, wait) = std::sync::mpsc::channel::<()>();
        let wait = Mutex::new(wait);
        let load: Loader = Arc::new(move || {
            wait.lock().unwrap().recv().unwrap();
            Ok(Arc::new(Topics("topics-1")) as Arc<dyn Embedder>)
        });
        let memories = Memories::new(Some(dir.clone()), Some(load));
        let app = Path::new("/work/app");
        assert_eq!(memories.status(app), Ok(Status { count: 0, vectors: VectorSearch::Off }));
        assert_eq!(memories.search(app, "login", 8), Ok(vec![]));
        assert_eq!(memories.recall(app, "how do users log in?"), "");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0, "reading a project's memory made a database");

        assert!(matches!(memories.save(app, &draft("Login", "Users sign in with a magic link; there is no password."), None), Ok(Saved::Added(Memory { id: 1, .. }))));
        memories.save(app, &draft("Tests", "Run cargo test before each commit."), None).unwrap();
        assert_eq!(memories.status(app), Ok(Status { count: 2, vectors: VectorSearch::Loading }));
        assert_eq!(memories.search(app, "esqueci a senha", 8), Ok(vec![]), "matched by meaning before the model loaded");
        go.send(()).unwrap();
        wait_for("the model", || memories.status(app).unwrap().vectors == VectorSearch::Ready);
        wait_for("the memories' vectors", || memories.search(app, "esqueci a senha", 8).unwrap().len() == 1);
        assert_eq!(
            memories.recall(app, "esqueci a senha do login"),
            "Project memory pinned found for this prompt, by id (memory_search finds more; memory_save with an id corrects one):\n[1] Login: Users sign in with a magic link; there is no password."
        );
        assert_eq!(memories.recall(app, "  "), "");
        assert_eq!(memories.forget(app, 1).map(|m| m.title), Ok("Login".into()));
        assert_eq!(memories.recall(app, "esqueci a senha do login"), "");
        assert_eq!(memories.forget(Path::new("/work/other"), 3), Err("no memory #3".into()));

        let reopened = Memories::new(Some(dir.clone()), None);
        assert_eq!(titles(reopened.search(app, "", 8)), ["#2 Tests"]);
        assert_eq!(Memories::new(None, None).save(app, &draft("Tests", "x"), None), Err("this machine keeps no memory".into()));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_model_that_fails_to_load_leaves_word_search() {
        let dir = temp("failed-model");
        let memories = Memories::new(Some(dir.clone()), Some(Arc::new(|| Err("offline".into()))));
        let app = Path::new("/work/app");
        memories.save(app, &draft("Deploys", "The hub deploys from railway-hub."), None).unwrap();
        wait_for("the failure", || memories.status(app).unwrap().vectors == VectorSearch::Off);
        assert_eq!(titles(memories.search(app, "where does the hub deploy from?", 8)), ["#1 Deploys"]);
        let long = memories.save(app, &draft("Long", &"word ".repeat(500)), None).unwrap();
        assert!(matches!(long, Saved::Added(Memory { id: 2, .. })));
        assert_eq!(memories.recall(app, "word").lines().nth(1), Some(format!("[2] Long: {}…", "word ".repeat(80)).as_str()));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn memories_saved_before_the_model_or_under_another_get_vectors_later() {
        let store = Store::open(Path::new(":memory:")).unwrap();
        store.save(&draft("Login", "Users sign in with a magic link."), None, None, 1).unwrap();
        store.save(&draft("Tests", "Run cargo test before each commit."), None, Some(&Topics("old-model")), 2).unwrap();
        let vectors = Topics("new-model");
        assert_eq!(titles(store.search("esqueci a senha", Some(&vectors), 0.5, 8)), Vec::<String>::new());
        assert_eq!(store.embed_missing(&vectors), Ok(2));
        assert_eq!(store.embed_missing(&vectors), Ok(0));
        assert_eq!(titles(store.search("esqueci a senha", Some(&vectors), 0.5, 8)), ["#1 Login"]);
        assert_eq!(titles(store.search("quebrou o teste", Some(&vectors), 0.5, 8)), ["#2 Tests"]);
    }
}
