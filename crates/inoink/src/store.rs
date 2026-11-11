//! SQLite store: the single source of local truth.
//!
//! Server state (`remote_state`) is kept apart from pending local intent (`intents`).
//! The effective state shown in the UI is the intent when one exists, else the server state.
//! Every mutating method is one transaction, so a returned `Ok` means the change is durable.

use crate::error::{Error, Result};
use crate::model::*;
use crate::util::{now_secs, sha256_hex};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

const MIGRATIONS: &[&str] =
    &[include_str!("../migrations/0001.sql"), include_str!("../migrations/0002.sql"), include_str!("../migrations/0003.sql")];

/// Highest schema version this build understands.
pub const SCHEMA_VERSION: i64 = MIGRATIONS.len() as i64;

/// Maximum number of articles a single bulk selection may hold.
pub const BULK_LIMIT: usize = 1000;

const FLAG_KEYS: [&str; 4] = ["read", "starred", "like", "broadcast"];

/// An article as received from the API, ready to be stored.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ArticleRecord {
    pub id: u64,
    pub title: String,
    pub author: String,
    pub origin_stream: String,
    pub origin_title: String,
    pub canonical_url: String,
    pub published: i64,
    pub timestamp_usec: i64,
    pub summary_html: String,
    pub raw_json: String,
    /// All category strings returned with the article.
    pub categories: Vec<String>,
    pub excerpt: bool,
}

/// One edit-tag request's worth of work.
#[derive(Clone, Debug, PartialEq)]
pub struct OutboxBatch {
    pub add: Option<Membership>,
    pub remove: Option<Membership>,
    /// (article, membership, desired, generation) for every change carried by this batch.
    pub entries: Vec<(ArticleId, Membership, bool, i64)>,
}

impl OutboxBatch {
    pub fn articles(&self) -> Vec<ArticleId> {
        let mut set = BTreeSet::new();
        for (a, ..) in &self.entries {
            set.insert(*a);
        }
        set.into_iter().collect()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Subscription {
    pub id: String,
    pub title: String,
    pub url: String,
    pub html_url: String,
    pub sort_id: String,
    pub first_item_usec: i64,
    pub folders: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TagInfo {
    pub id: String,
    pub kind: String,
    pub sort_id: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OrgOp {
    pub id: i64,
    pub kind: String,
    pub payload: String,
    pub state: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ContentVersion {
    pub version: i64,
    pub hash: String,
    pub raw_hash: String,
    pub dir: String,
    pub bytes: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AssetRow {
    pub name: String,
    pub url: String,
    pub state: String,
    pub bytes: i64,
    pub tries: i64,
}

pub struct Store {
    conn: Connection,
    account: String,
    path: Option<PathBuf>,
}

fn flag_expr(key: &str) -> String {
    format!(
        "COALESCE((SELECT desired FROM intents WHERE account=a.account AND article=a.id AND key='{key}'), \
         (SELECT present FROM remote_state WHERE account=a.account AND article=a.id AND key='{key}'), 0)"
    )
}

fn next_seq(tx: &Transaction) -> Result<i64> {
    tx.execute(
        "INSERT INTO meta(key,value) VALUES('seq','1') \
         ON CONFLICT(key) DO UPDATE SET value=CAST(value AS INTEGER)+1",
        [],
    )?;
    Ok(tx.query_row("SELECT CAST(value AS INTEGER) FROM meta WHERE key='seq'", [], |r| r.get(0))?)
}

fn next_generation(tx: &Transaction) -> Result<i64> {
    tx.execute(
        "INSERT INTO meta(key,value) VALUES('generation','1') \
         ON CONFLICT(key) DO UPDATE SET value=CAST(value AS INTEGER)+1",
        [],
    )?;
    Ok(tx.query_row("SELECT CAST(value AS INTEGER) FROM meta WHERE key='generation'", [], |r| r.get(0))?)
}

fn effective_tx(tx: &Transaction, account: &str, article: ArticleId, m: &Membership) -> Result<bool> {
    let key = m.key();
    let v: Option<i64> = tx
        .query_row("SELECT desired FROM intents WHERE account=?1 AND article=?2 AND key=?3", params![account, article.to_db(), key], |r| {
            r.get(0)
        })
        .optional()?;
    if let Some(v) = v {
        return Ok(v != 0);
    }
    let v: Option<i64> = tx
        .query_row(
            "SELECT present FROM remote_state WHERE account=?1 AND article=?2 AND key=?3",
            params![account, article.to_db(), key],
            |r| r.get(0),
        )
        .optional()?;
    Ok(v.unwrap_or(0) != 0)
}

fn set_intent_tx(tx: &Transaction, account: &str, article: ArticleId, m: &Membership, desired: bool) -> Result<CommitReceipt> {
    let key = m.key();
    let a = article.to_db();
    let known: Option<i64> = tx
        .query_row("SELECT present FROM remote_state WHERE account=?1 AND article=?2 AND key=?3", params![account, a, key], |r| r.get(0))
        .optional()?;
    let existing: Option<Option<i64>> = tx
        .query_row("SELECT sent_generation FROM intents WHERE account=?1 AND article=?2 AND key=?3", params![account, a, key], |r| {
            r.get::<_, Option<i64>>(0)
        })
        .optional()?;
    let generation = next_generation(tx)?;
    let in_flight = matches!(existing, Some(Some(_)));
    let matches_remote = known.map(|k| (k != 0) == desired).unwrap_or(false);
    if matches_remote && !in_flight {
        // Collapses an unsent change back to the known server state: nothing to send.
        tx.execute("DELETE FROM intents WHERE account=?1 AND article=?2 AND key=?3", params![account, a, key])?;
        return Ok(CommitReceipt { generation, pending: false });
    }
    tx.execute(
        "INSERT INTO intents(account,article,key,desired,generation,updated) VALUES(?1,?2,?3,?4,?5,?6) \
         ON CONFLICT(account,article,key) DO UPDATE SET desired=excluded.desired, \
         generation=excluded.generation, updated=excluded.updated",
        params![account, a, key, desired as i64, generation, now_secs()],
    )?;
    Ok(CommitReceipt { generation, pending: true })
}

fn observe_tx(tx: &Transaction, account: &str, article: ArticleId, key: &str, present: bool, start_seq: i64) -> Result<bool> {
    let a = article.to_db();
    let existing: Option<i64> = tx
        .query_row("SELECT seq FROM remote_state WHERE account=?1 AND article=?2 AND key=?3", params![account, a, key], |r| r.get(0))
        .optional()?;
    if let Some(seq) = existing {
        if seq > start_seq {
            // Something newer than this observation's request (an acknowledgement) already landed.
            return Ok(false);
        }
    }
    let seq = next_seq(tx)?;
    tx.execute(
        "INSERT INTO remote_state(account,article,key,present,seq) VALUES(?1,?2,?3,?4,?5) \
         ON CONFLICT(account,article,key) DO UPDATE SET present=excluded.present, seq=excluded.seq",
        params![account, a, key, present as i64, seq],
    )?;
    // An unsent intent that now equals the server state is redundant.
    tx.execute(
        "DELETE FROM intents WHERE account=?1 AND article=?2 AND key=?3 AND sent_generation IS NULL AND desired=?4",
        params![account, a, key, present as i64],
    )?;
    Ok(true)
}

impl Store {
    pub fn open(path: &Path, account: &str) -> Result<Store> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        let mut store = Store { conn, account: account.to_string(), path: Some(path.to_path_buf()) };
        store.configure(true)?;
        store.migrate()?;
        Ok(store)
    }

    pub fn open_in_memory(account: &str) -> Result<Store> {
        let conn = Connection::open_in_memory()?;
        let mut store = Store { conn, account: account.to_string(), path: None };
        store.configure(false)?;
        store.migrate()?;
        Ok(store)
    }

    /// Another handle on the same database for a different account.
    pub fn for_account(&self, account: &str) -> Result<Store> {
        let path = self.path.clone().ok_or_else(|| Error::Storage("in-memory store".into()))?;
        Store::open(&path, account)
    }

    fn configure(&mut self, wal: bool) -> Result<()> {
        self.conn.busy_timeout(std::time::Duration::from_secs(5))?;
        self.conn.pragma_update(None, "foreign_keys", "ON")?;
        self.conn.pragma_update(None, "synchronous", "FULL")?;
        if wal {
            self.conn.pragma_update(None, "journal_mode", "WAL")?;
        }
        Ok(())
    }

    pub fn account(&self) -> &str {
        &self.account
    }

    pub fn schema_version(&self) -> Result<i64> {
        Ok(self.conn.pragma_query_value(None, "user_version", |r| r.get(0))?)
    }

    /// Applies pending migrations, each in its own transaction. Takes a consistent backup first
    /// when upgrading an existing database, and refuses a database from a newer build.
    pub fn migrate(&mut self) -> Result<()> {
        let current = self.schema_version()?;
        if current > SCHEMA_VERSION {
            return Err(Error::Storage(format!("database schema {current} is newer than supported {SCHEMA_VERSION}")));
        }
        if current == SCHEMA_VERSION {
            return Ok(());
        }
        if current > 0 {
            if let Some(path) = &self.path {
                let backup = path.with_extension(format!("bak-v{current}"));
                let _ = std::fs::remove_file(&backup);
                let quoted = backup.to_string_lossy().replace('\'', "''");
                self.conn.execute_batch(&format!("VACUUM INTO '{quoted}'"))?;
            }
        }
        let upgraded_from = current;
        for (i, sql) in MIGRATIONS.iter().enumerate() {
            let version = i as i64 + 1;
            if version <= current {
                continue;
            }
            let tx = self.conn.transaction()?;
            tx.execute_batch(sql)?;
            tx.pragma_update(None, "user_version", version)?;
            tx.commit()?;
        }
        if upgraded_from > 0 && upgraded_from < 3 {
            self.backfill_words()?;
        }
        Ok(())
    }

    /// Recomputes every article's word count from the best text held (document text when
    /// downloaded, else the feed summary).
    pub fn backfill_words(&mut self) -> Result<usize> {
        let account = self.account.clone();
        let tx = self.conn.transaction()?;
        let rows: Vec<(i64, String)> = {
            let mut stmt = tx.prepare("SELECT id, summary_html FROM articles WHERE account=?1")?;
            let r = stmt.query_map([&account], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<std::result::Result<_, _>>()?;
            r
        };
        for (id, summary) in &rows {
            let body: Option<String> = tx
                .query_row("SELECT body FROM article_fts WHERE account=?1 AND article=?2", params![account, id], |r| r.get(0))
                .optional()?;
            let words = match body {
                Some(b) => crate::content::word_count(&b),
                None => crate::content::word_count_html(summary),
            };
            tx.execute("UPDATE articles SET words=?3 WHERE account=?1 AND id=?2", params![account, id, words as i64])?;
        }
        tx.commit()?;
        Ok(rows.len())
    }

    /// Folds the WAL into the main file so the database file alone is complete (before USB
    /// export). The caller must then drop every handle.
    pub fn checkpoint(&self) -> Result<()> {
        let _ = self.conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()));
        Ok(())
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    pub fn get_meta(&self, key: &str) -> Result<Option<String>> {
        let k = format!("{}:{key}", self.account);
        Ok(self.conn.query_row("SELECT value FROM meta WHERE key=?1", [k], |r| r.get(0)).optional()?)
    }

    pub fn set_meta(&mut self, key: &str, value: &str) -> Result<()> {
        let k = format!("{}:{key}", self.account);
        self.conn
            .execute("INSERT INTO meta(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![k, value])?;
        Ok(())
    }

    /// Current global sequence number. Take it *before* issuing a request whose result will be
    /// passed to [`Store::upsert_article`] or the reconcile functions.
    pub fn seq(&self) -> Result<i64> {
        let v: Option<i64> = self.conn.query_row("SELECT CAST(value AS INTEGER) FROM meta WHERE key='seq'", [], |r| r.get(0)).optional()?;
        Ok(v.unwrap_or(0))
    }

    // ---- articles -------------------------------------------------------------------------

    /// Stores an article observed on the server. `start_seq` is [`Store::seq`] from before the
    /// request; server flags older than a later acknowledgement are not applied.
    /// Returns true when the article was new.
    pub fn upsert_article(&mut self, rec: &ArticleRecord, start_seq: i64) -> Result<bool> {
        let account = self.account.clone();
        let tx = self.conn.transaction()?;
        let id = ArticleId(rec.id);
        let existed: bool = tx
            .query_row("SELECT 1 FROM articles WHERE account=?1 AND id=?2", params![account, id.to_db()], |_| Ok(true))
            .optional()?
            .unwrap_or(false);
        let content_hash = sha256_hex(rec.summary_html.as_bytes());
        let words = crate::content::word_count_html(&rec.summary_html) as i64;
        tx.execute(
            "INSERT INTO articles(account,id,title,author,origin_stream,origin_title,canonical_url,published,\
             timestamp_usec,summary_html,content_hash,raw_json,excerpt,words) \
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14) \
             ON CONFLICT(account,id) DO UPDATE SET title=excluded.title, author=excluded.author, \
             origin_stream=excluded.origin_stream, origin_title=excluded.origin_title, \
             canonical_url=excluded.canonical_url, published=excluded.published, \
             timestamp_usec=excluded.timestamp_usec, summary_html=excluded.summary_html, \
             content_hash=excluded.content_hash, excerpt=excluded.excerpt, \
             words=CASE WHEN EXISTS(SELECT 1 FROM content_versions WHERE account=articles.account AND article=articles.id) \
             THEN articles.words ELSE excluded.words END",
            params![
                account,
                id.to_db(),
                rec.title,
                rec.author,
                rec.origin_stream,
                rec.origin_title,
                rec.canonical_url,
                rec.published,
                rec.timestamp_usec,
                rec.summary_html,
                content_hash,
                rec.raw_json,
                rec.excerpt as i64,
                words
            ],
        )?;
        // raw_json is immutable once stored: only set it when it is still the placeholder.
        tx.execute(
            "UPDATE articles SET raw_json=?3 WHERE account=?1 AND id=?2 AND raw_json='{}'",
            params![account, id.to_db(), rec.raw_json],
        )?;

        tx.execute("DELETE FROM article_streams WHERE account=?1 AND article=?2", params![account, id.to_db()])?;
        let mut labels = BTreeSet::new();
        let mut flags: BTreeMap<String, bool> = FLAG_KEYS.iter().map(|k| (k.to_string(), false)).collect();
        for cat in &rec.categories {
            let norm = normalize_user(cat);
            tx.execute(
                "INSERT OR IGNORE INTO article_streams(account,article,stream) VALUES(?1,?2,?3)",
                params![account, id.to_db(), norm],
            )?;
            if let Some(m) = Membership::from_tag_id(cat) {
                match &m {
                    Membership::Label(_) => {
                        labels.insert(m.key());
                    }
                    _ => {
                        flags.insert(m.key(), true);
                    }
                }
            }
        }
        for (key, present) in &flags {
            observe_tx(&tx, &account, id, key, *present, start_seq)?;
        }
        // Labels no longer listed are cleared; listed ones are set.
        let known: Vec<String> = {
            let mut stmt = tx.prepare("SELECT key FROM remote_state WHERE account=?1 AND article=?2 AND key LIKE 'label:%'")?;
            let rows = stmt.query_map(params![account, id.to_db()], |r| r.get::<_, String>(0))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        for key in known {
            if !labels.contains(&key) {
                observe_tx(&tx, &account, id, &key, false, start_seq)?;
            }
        }
        for key in &labels {
            observe_tx(&tx, &account, id, key, true, start_seq)?;
        }
        tx.commit()?;
        Ok(!existed)
    }

    pub fn has_article(&self, id: ArticleId) -> Result<bool> {
        Ok(self
            .conn
            .query_row("SELECT 1 FROM articles WHERE account=?1 AND id=?2", params![self.account, id.to_db()], |_| Ok(true))
            .optional()?
            .unwrap_or(false))
    }

    /// The raw API object stored with the article.
    pub fn raw_json(&self, id: ArticleId) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row("SELECT raw_json FROM articles WHERE account=?1 AND id=?2", params![self.account, id.to_db()], |r| r.get(0))
            .optional()?)
    }

    pub fn article_record(&self, id: ArticleId) -> Result<Option<ArticleRecord>> {
        Ok(self
            .conn
            .query_row(
                "SELECT title,author,origin_stream,origin_title,canonical_url,published,timestamp_usec,\
                 summary_html,raw_json,excerpt FROM articles WHERE account=?1 AND id=?2",
                params![self.account, id.to_db()],
                |r| {
                    Ok(ArticleRecord {
                        id: id.0,
                        title: r.get(0)?,
                        author: r.get(1)?,
                        origin_stream: r.get(2)?,
                        origin_title: r.get(3)?,
                        canonical_url: r.get(4)?,
                        published: r.get(5)?,
                        timestamp_usec: r.get(6)?,
                        summary_html: r.get(7)?,
                        raw_json: r.get(8)?,
                        categories: Vec::new(),
                        excerpt: r.get::<_, i64>(9)? != 0,
                    })
                },
            )
            .optional()?)
    }

    pub fn effective(&self, article: ArticleId, m: &Membership) -> Result<bool> {
        let key = m.key();
        let v: Option<i64> = self
            .conn
            .query_row(
                "SELECT COALESCE((SELECT desired FROM intents WHERE account=?1 AND article=?2 AND key=?3),\
                 (SELECT present FROM remote_state WHERE account=?1 AND article=?2 AND key=?3))",
                params![self.account, article.to_db(), key],
                |r| r.get(0),
            )
            .optional()?
            .flatten();
        Ok(v.unwrap_or(0) != 0)
    }

    /// Tags currently on an article (effective state), by bare name.
    pub fn effective_labels(&self, article: ArticleId) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT k FROM (\
               SELECT key AS k, COALESCE((SELECT desired FROM intents i WHERE i.account=r.account AND i.article=r.article AND i.key=r.key), r.present) AS v \
               FROM remote_state r WHERE r.account=?1 AND r.article=?2 AND r.key LIKE 'label:%' \
               UNION \
               SELECT key, desired FROM intents WHERE account=?1 AND article=?2 AND key LIKE 'label:%') WHERE v=1 ORDER BY k",
        )?;
        let rows = stmt.query_map(params![self.account, article.to_db()], |r| r.get::<_, String>(0))?;
        let mut out = Vec::new();
        for r in rows {
            if let Some(n) = r?.strip_prefix("label:") {
                out.push(n.to_string());
            }
        }
        out.dedup();
        Ok(out)
    }

    /// Server-known (not effective) state, if ever observed.
    pub fn remote(&self, article: ArticleId, m: &Membership) -> Result<Option<bool>> {
        let v: Option<i64> = self
            .conn
            .query_row(
                "SELECT present FROM remote_state WHERE account=?1 AND article=?2 AND key=?3",
                params![self.account, article.to_db(), m.key()],
                |r| r.get(0),
            )
            .optional()?;
        Ok(v.map(|v| v != 0))
    }

    pub fn query_articles(&self, q: &ArticleQuery) -> Result<ArticlePage> {
        let limit = if q.limit == 0 { 50 } else { q.limit };
        let mut sql = String::from("SELECT a.id, a.title, a.origin_title, a.published, a.timestamp_usec, a.download_state, ");
        sql.push_str(&flag_expr("read"));
        sql.push_str(" AS is_read, ");
        sql.push_str(&flag_expr("starred"));
        sql.push_str(" AS is_starred, a.words FROM articles a WHERE a.account=?1");
        let mut args: Vec<rusqlite::types::Value> = vec![self.account.clone().into()];
        let mut push = |sql: &mut String, cond: &str, vals: Vec<rusqlite::types::Value>| {
            sql.push_str(" AND ");
            sql.push_str(cond);
            args.extend(vals);
        };
        let label_expr = |key_placeholder: &str| {
            format!(
                "COALESCE((SELECT desired FROM intents WHERE account=a.account AND article=a.id AND key={key_placeholder}), \
                 (SELECT present FROM remote_state WHERE account=a.account AND article=a.id AND key={key_placeholder}), 0)=1"
            )
        };
        match &q.collection {
            None | Some(Collection::All) => {}
            Some(Collection::Starred) => push(&mut sql, &format!("{}=1", flag_expr("starred")), vec![]),
            Some(Collection::Unread) => push(&mut sql, &format!("{}=0", flag_expr("read")), vec![]),
            Some(Collection::Liked) => push(&mut sql, &format!("{}=1", flag_expr("like")), vec![]),
            Some(Collection::Broadcast) => push(&mut sql, &format!("{}=1", flag_expr("broadcast")), vec![]),
            Some(Collection::Feed(s)) => push(&mut sql, "a.origin_stream=?", vec![s.0.clone().into()]),
            Some(Collection::Folder(n)) => push(
                &mut sql,
                "a.origin_stream IN (SELECT feed FROM subscription_folders WHERE account=a.account AND folder=?)",
                vec![n.clone().into()],
            ),
            Some(Collection::Tag(n)) => {
                let key = format!("label:{n}");
                push(&mut sql, &label_expr("?"), vec![key.clone().into(), key.into()])
            }
            Some(Collection::SavedPages) => push(
                &mut sql,
                "EXISTS (SELECT 1 FROM article_streams WHERE account=a.account AND article=a.id AND stream=?)",
                vec![STATE_SAVED.to_string().into()],
            ),
            Some(Collection::Annotated) => push(
                &mut sql,
                "EXISTS (SELECT 1 FROM article_streams WHERE account=a.account AND article=a.id AND stream=?)",
                vec![STATE_ANNOTATED.to_string().into()],
            ),
            Some(Collection::Monitoring(s)) => push(
                &mut sql,
                "EXISTS (SELECT 1 FROM article_streams WHERE account=a.account AND article=a.id AND stream=?)",
                vec![s.0.clone().into()],
            ),
        }
        if q.unread_only && !matches!(q.collection, Some(Collection::Unread)) {
            push(&mut sql, &format!("{}=0", flag_expr("read")), vec![]);
        }
        if let Some(text) = &q.search {
            match fts_query(text) {
                Some(m) => push(
                    &mut sql,
                    "a.id IN (SELECT article FROM article_fts WHERE account=? AND article_fts MATCH ?)",
                    vec![self.account.clone().into(), m.into()],
                ),
                None => return Ok(ArticlePage { items: vec![], next: None }),
            }
        }
        if let Some((ts, id)) = q.after {
            push(&mut sql, "(a.timestamp_usec, a.id) < (?, ?)", vec![ts.into(), id.to_db().into()]);
        }
        sql.push_str(" ORDER BY a.timestamp_usec DESC, a.id DESC LIMIT ?");
        args.push((limit as i64 + 1).into());
        // Bare `?` placeholders are numbered after `?1`, so `args` binds in order.
        let mut stmt = self.conn.prepare(&sql)?;
        let mut rows = stmt.query(rusqlite::params_from_iter(args.iter()))?;
        let mut items = Vec::new();
        while let Some(r) = rows.next()? {
            items.push(ArticleSummary {
                id: ArticleId::from_db(r.get(0)?),
                title: r.get(1)?,
                source: r.get(2)?,
                published: r.get(3)?,
                timestamp_usec: r.get(4)?,
                download: DownloadState::parse(&r.get::<_, String>(5)?),
                read: r.get::<_, i64>(6)? != 0,
                starred: r.get::<_, i64>(7)? != 0,
                words: r.get::<_, i64>(8)?.max(0) as u32,
            });
        }
        let next = if items.len() > limit {
            items.truncate(limit);
            items.last().map(|l| (l.timestamp_usec, l.id))
        } else {
            None
        };
        Ok(ArticlePage { items, next })
    }
}

/// Turns user text into a safe FTS5 MATCH expression: every whitespace-separated token becomes
/// a quoted phrase, so FTS operators and syntax in the input are inert.
pub fn fts_query(text: &str) -> Option<String> {
    let tokens: Vec<String> = text.split_whitespace().map(|t| format!("\"{}\"", t.replace('"', "\"\""))).collect();
    if tokens.is_empty() {
        None
    } else {
        Some(tokens.join(" "))
    }
}

// ---- local intent, Done/undo, bulk, positions ---------------------------------------------

impl Store {
    /// Persists a desired state for one membership. Returns once the transaction is committed.
    pub fn apply_intent(&mut self, article: ArticleId, m: &Membership, desired: bool) -> Result<CommitReceipt> {
        let account = self.account.clone();
        let tx = self.conn.transaction()?;
        let r = set_intent_tx(&tx, &account, article, m, desired)?;
        tx.commit()?;
        Ok(r)
    }

    /// Done: mark read and remove the star in one transaction, recording an undo entry.
    pub fn done(&mut self, article: ArticleId) -> Result<UndoId> {
        let account = self.account.clone();
        let tx = self.conn.transaction()?;
        let prev_read = effective_tx(&tx, &account, article, &Membership::Read)?;
        let prev_starred = effective_tx(&tx, &account, article, &Membership::Starred)?;
        set_intent_tx(&tx, &account, article, &Membership::Read, true)?;
        set_intent_tx(&tx, &account, article, &Membership::Starred, false)?;
        tx.execute(
            "INSERT INTO undos(account,article,prev_read,prev_starred,created) VALUES(?1,?2,?3,?4,?5)",
            params![account, article.to_db(), prev_read as i64, prev_starred as i64, now_secs()],
        )?;
        let id = tx.last_insert_rowid();
        tx.commit()?;
        Ok(UndoId(id))
    }

    /// Undo a Done by durably restoring the previous read and starred values.
    pub fn undo(&mut self, id: UndoId) -> Result<()> {
        let account = self.account.clone();
        let tx = self.conn.transaction()?;
        let row: Option<(i64, i64, i64, i64)> = tx
            .query_row("SELECT article,prev_read,prev_starred,used FROM undos WHERE id=?1 AND account=?2", params![id.0, account], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
            })
            .optional()?;
        let (article, prev_read, prev_starred, used) = row.ok_or_else(|| Error::InvalidRequest("unknown undo".into()))?;
        if used != 0 {
            return Err(Error::InvalidRequest("already undone".into()));
        }
        let article = ArticleId::from_db(article);
        set_intent_tx(&tx, &account, article, &Membership::Read, prev_read != 0)?;
        set_intent_tx(&tx, &account, article, &Membership::Starred, prev_starred != 0)?;
        tx.execute("UPDATE undos SET used=1 WHERE id=?1", params![id.0])?;
        tx.commit()?;
        Ok(())
    }

    /// The latest unused undo for an article, if any.
    pub fn latest_undo(&self, article: ArticleId) -> Result<Option<UndoId>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id FROM undos WHERE account=?1 AND article=?2 AND used=0 ORDER BY id DESC LIMIT 1",
                params![self.account, article.to_db()],
                |r| r.get(0),
            )
            .optional()?
            .map(UndoId))
    }

    /// Freezes the articles currently matching `query` (at most [`BULK_LIMIT`]) into an immutable
    /// selection. Articles arriving later are never part of it.
    pub fn freeze_selection(&mut self, query: &ArticleQuery) -> Result<BulkSelection> {
        let mut ids = Vec::new();
        let mut q = query.clone();
        q.limit = 200;
        loop {
            let page = self.query_articles(&q)?;
            ids.extend(page.items.iter().map(|i| i.id));
            match page.next {
                Some(c) if ids.len() < BULK_LIMIT => q.after = Some(c),
                _ => break,
            }
        }
        ids.truncate(BULK_LIMIT);
        self.freeze_ids(&ids)
    }

    pub fn freeze_ids(&mut self, ids: &[ArticleId]) -> Result<BulkSelection> {
        if ids.len() > BULK_LIMIT {
            return Err(Error::InvalidRequest(format!("selection larger than {BULK_LIMIT}")));
        }
        let account = self.account.clone();
        let tx = self.conn.transaction()?;
        tx.execute("INSERT INTO bulk_scopes(account,created) VALUES(?1,?2)", params![account, now_secs()])?;
        let scope = tx.last_insert_rowid();
        let mut articles = Vec::new();
        for id in ids {
            let n = tx.execute(
                "INSERT OR IGNORE INTO bulk_items(scope,article) SELECT ?1, id FROM articles WHERE account=?2 AND id=?3",
                params![scope, account, id.to_db()],
            )?;
            if n > 0 {
                articles.push(*id);
            }
        }
        tx.commit()?;
        Ok(BulkSelection { id: scope, articles })
    }

    /// Applies `action` to exactly the frozen selection.
    pub fn apply_bulk(&mut self, sel: &BulkSelection, action: BulkAction) -> Result<usize> {
        let account = self.account.clone();
        let tx = self.conn.transaction()?;
        let ids: Vec<i64> = {
            let mut stmt = tx.prepare(
                "SELECT i.article FROM bulk_items i JOIN bulk_scopes s ON s.id=i.scope \
                 WHERE i.scope=?1 AND s.account=?2 ORDER BY i.article",
            )?;
            let rows = stmt.query_map(params![sel.id, account], |r| r.get(0))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        let (m, v) = match action {
            BulkAction::MarkRead => (Membership::Read, true),
            BulkAction::MarkUnread => (Membership::Read, false),
            BulkAction::Star => (Membership::Starred, true),
            BulkAction::Unstar => (Membership::Starred, false),
        };
        for id in &ids {
            set_intent_tx(&tx, &account, ArticleId::from_db(*id), &m, v)?;
        }
        tx.commit()?;
        Ok(ids.len())
    }

    pub fn save_position(&mut self, article: ArticleId, pos: &ReadingPosition) -> Result<()> {
        self.conn.execute(
            "INSERT INTO positions(account,article,content_hash,offset,anchor,font_size,updated) \
             VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(account,article) DO UPDATE SET \
             content_hash=excluded.content_hash, offset=excluded.offset, anchor=excluded.anchor, \
             font_size=excluded.font_size, updated=excluded.updated",
            params![
                self.account,
                article.to_db(),
                pos.content_hash,
                pos.offset as i64,
                pos.anchor,
                pos.font_size.map(|f| f as f64),
                now_secs()
            ],
        )?;
        Ok(())
    }

    pub fn position(&self, article: ArticleId) -> Result<Option<ReadingPosition>> {
        Ok(self
            .conn
            .query_row(
                "SELECT content_hash,offset,anchor,font_size FROM positions WHERE account=?1 AND article=?2",
                params![self.account, article.to_db()],
                |r| {
                    Ok(ReadingPosition {
                        content_hash: r.get(0)?,
                        offset: r.get::<_, i64>(1)? as u64,
                        anchor: r.get(2)?,
                        font_size: r.get::<_, Option<f64>>(3)?.map(|f| f as f32),
                    })
                },
            )
            .optional()?)
    }
}

// ---- outbox ---------------------------------------------------------------------------------

impl Store {
    /// Marks the oldest unsent intents as in flight and groups them into edit-tag batches of at
    /// most `max_articles` articles. An article's changes are paired (one add with one remove)
    /// so that a Done is a single request.
    pub fn begin_send(&mut self, max_articles: usize) -> Result<Vec<OutboxBatch>> {
        let account = self.account.clone();
        let tx = self.conn.transaction()?;
        let rows: Vec<(i64, String, bool, i64)> = {
            let mut stmt = tx.prepare(
                "SELECT article,key,desired,generation FROM intents \
                 WHERE account=?1 AND sent_generation IS NULL ORDER BY updated, article, key",
            )?;
            let rows = stmt.query_map(params![account], |r| Ok((r.get(0)?, r.get(1)?, r.get::<_, i64>(2)? != 0, r.get(3)?)))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        type Changes = Vec<(Membership, i64)>;
        let mut per_article: BTreeMap<i64, (Changes, Changes)> = BTreeMap::new();
        for (a, key, desired, gen) in &rows {
            let m = Membership::from_key(key)?;
            let e = per_article.entry(*a).or_default();
            if *desired {
                e.0.push((m, *gen));
            } else {
                e.1.push((m, *gen));
            }
        }
        type Key = (Option<Membership>, Option<Membership>);
        let mut groups: BTreeMap<Key, Vec<(ArticleId, Membership, bool, i64)>> = BTreeMap::new();
        for (a, (adds, removes)) in per_article {
            let article = ArticleId::from_db(a);
            let n = adds.len().max(removes.len());
            for i in 0..n {
                let add = adds.get(i).cloned();
                let rem = removes.get(i).cloned();
                let key = (add.as_ref().map(|x| x.0.clone()), rem.as_ref().map(|x| x.0.clone()));
                let g = groups.entry(key).or_default();
                if let Some((m, gen)) = add {
                    g.push((article, m, true, gen));
                }
                if let Some((m, gen)) = rem {
                    g.push((article, m, false, gen));
                }
            }
        }
        let mut batches = Vec::new();
        for ((add, remove), entries) in groups {
            let mut chunk: Vec<(ArticleId, Membership, bool, i64)> = Vec::new();
            let mut seen = BTreeSet::new();
            for e in entries {
                if !seen.contains(&e.0) && seen.len() == max_articles.max(1) {
                    batches.push(OutboxBatch { add: add.clone(), remove: remove.clone(), entries: std::mem::take(&mut chunk) });
                    seen.clear();
                }
                seen.insert(e.0);
                chunk.push(e);
            }
            if !chunk.is_empty() {
                batches.push(OutboxBatch { add: add.clone(), remove: remove.clone(), entries: chunk });
            }
        }
        for b in &batches {
            for (a, m, desired, gen) in &b.entries {
                tx.execute(
                    "UPDATE intents SET sent_generation=?4, sent_desired=?5 WHERE account=?1 AND article=?2 AND key=?3",
                    params![account, a.to_db(), m.key(), gen, *desired as i64],
                )?;
            }
        }
        tx.commit()?;
        Ok(batches)
    }

    /// The server acknowledged the batch. Only the sent generation is cleared; a newer desired
    /// value written while the request was in flight stays pending.
    pub fn acknowledge(&mut self, batch: &OutboxBatch) -> Result<()> {
        let account = self.account.clone();
        let tx = self.conn.transaction()?;
        for (a, m, desired, gen) in &batch.entries {
            let key = m.key();
            let seq = next_seq(&tx)?;
            tx.execute(
                "INSERT INTO remote_state(account,article,key,present,seq) VALUES(?1,?2,?3,?4,?5) \
                 ON CONFLICT(account,article,key) DO UPDATE SET present=excluded.present, seq=excluded.seq",
                params![account, a.to_db(), key, *desired as i64, seq],
            )?;
            let cur: Option<(i64, i64)> = tx
                .query_row(
                    "SELECT generation,desired FROM intents WHERE account=?1 AND article=?2 AND key=?3 AND sent_generation=?4",
                    params![account, a.to_db(), key, gen],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            if let Some((cur_gen, cur_desired)) = cur {
                if cur_gen == *gen || (cur_desired != 0) == *desired {
                    tx.execute("DELETE FROM intents WHERE account=?1 AND article=?2 AND key=?3", params![account, a.to_db(), key])?;
                } else {
                    tx.execute(
                        "UPDATE intents SET sent_generation=NULL, sent_desired=NULL WHERE account=?1 AND article=?2 AND key=?3",
                        params![account, a.to_db(), key],
                    )?;
                }
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// The server cannot apply this change (for example marking an article older than the feed's
    /// first item unread). Drops the intent, reverting the effective state to the server's.
    pub fn reject_intent(&mut self, article: ArticleId, m: &Membership, generation: i64) -> Result<bool> {
        let n = self.conn.execute(
            "DELETE FROM intents WHERE account=?1 AND article=?2 AND key=?3 AND generation=?4",
            params![self.account, article.to_db(), m.key(), generation],
        )?;
        Ok(n > 0)
    }

    /// The batch failed (or its outcome is unknown); state setters are safe to send again.
    pub fn release(&mut self, batch: &OutboxBatch) -> Result<()> {
        let account = self.account.clone();
        let tx = self.conn.transaction()?;
        for (a, m, _, gen) in &batch.entries {
            tx.execute(
                "UPDATE intents SET sent_generation=NULL, sent_desired=NULL \
                 WHERE account=?1 AND article=?2 AND key=?3 AND sent_generation=?4",
                params![account, a.to_db(), m.key(), gen],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Startup recovery: anything left in flight by an interrupted process is resent.
    pub fn recover_in_flight(&mut self) -> Result<usize> {
        Ok(self.conn.execute(
            "UPDATE intents SET sent_generation=NULL, sent_desired=NULL WHERE account=?1 AND sent_generation IS NOT NULL",
            params![self.account],
        )?)
    }

    pub fn pending_count(&self) -> Result<usize> {
        Ok(self.conn.query_row("SELECT COUNT(*) FROM intents WHERE account=?1", params![self.account], |r| r.get::<_, i64>(0))? as usize)
    }

    pub fn pending_for(&self, article: ArticleId) -> Result<Vec<(Membership, bool)>> {
        let mut stmt = self.conn.prepare("SELECT key,desired FROM intents WHERE account=?1 AND article=?2 ORDER BY key")?;
        let rows = stmt.query_map(params![self.account, article.to_db()], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? != 0)))?;
        let mut out = Vec::new();
        for r in rows {
            let (k, v) = r?;
            out.push((Membership::from_key(&k)?, v));
        }
        Ok(out)
    }
}

// ---- reconciliation -------------------------------------------------------------------------

impl Store {
    /// Starts reconciling one membership against a server listing. `listed_value` is the value
    /// implied for ids that appear in the listing (e.g. starred=true for the starred stream, or
    /// read=false for an unread listing).
    pub fn begin_reconcile(&mut self, m: &Membership, listed_value: bool) -> Result<i64> {
        let seq = self.seq()?;
        self.conn.execute(
            "INSERT INTO sync_runs(account,key,listed_value,start_seq) VALUES(?1,?2,?3,?4)",
            params![self.account, m.key(), listed_value as i64, seq],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn run_start_seq(&self, run: i64) -> Result<i64> {
        Ok(self.conn.query_row("SELECT start_seq FROM sync_runs WHERE id=?1", params![run], |r| r.get(0))?)
    }

    /// Records one page of a listing. Returns the ids not known locally (to be fetched), and
    /// rejects a continuation token already seen in this run (a loop).
    pub fn reconcile_page(&mut self, run: i64, ids: &[ArticleId], continuation: Option<&str>) -> Result<Vec<ArticleId>> {
        let account = self.account.clone();
        let tx = self.conn.transaction()?;
        let (key, listed, start_seq): (String, i64, i64) = tx
            .query_row(
                "SELECT key,listed_value,start_seq FROM sync_runs WHERE id=?1 AND account=?2 AND complete=0 AND abandoned=0",
                params![run, account],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .map_err(|_| Error::InvalidRequest("unknown or finished run".into()))?;
        if let Some(tok) = continuation {
            let n = tx.execute("INSERT OR IGNORE INTO sync_tokens(run,token) VALUES(?1,?2)", params![run, tok])?;
            if n == 0 {
                tx.execute("UPDATE sync_runs SET abandoned=1 WHERE id=?1", params![run])?;
                tx.commit()?;
                return Err(Error::Data("continuation loop".into()));
            }
        }
        let mut unknown = Vec::new();
        for id in ids {
            tx.execute("INSERT OR IGNORE INTO sync_seen(run,article) VALUES(?1,?2)", params![run, id.to_db()])?;
            let known: bool = tx
                .query_row("SELECT 1 FROM articles WHERE account=?1 AND id=?2", params![account, id.to_db()], |_| Ok(true))
                .optional()?
                .unwrap_or(false);
            if known {
                observe_tx(&tx, &account, *id, &key, listed != 0, start_seq)?;
            } else {
                unknown.push(*id);
            }
        }
        tx.commit()?;
        Ok(unknown)
    }

    /// Ends a run. An incomplete run (failure, quota stop, page cap) abandons the comparison:
    /// no absence is ever inferred from it. A complete run returns the *candidates*: articles
    /// whose last-known value equals the listed value but were not listed. The caller verifies
    /// them with a second pass and passes the confirmed ones to [`Store::resolve_candidates`].
    pub fn finish_reconciliation(&mut self, run: i64, complete: bool) -> Result<Vec<ArticleId>> {
        let account = self.account.clone();
        if !complete {
            self.conn.execute("UPDATE sync_runs SET abandoned=1 WHERE id=?1", params![run])?;
            return Ok(Vec::new());
        }
        let (key, listed, start_seq): (String, i64, i64) = self.conn.query_row(
            "SELECT key,listed_value,start_seq FROM sync_runs WHERE id=?1 AND account=?2",
            params![run, account],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        self.conn.execute("UPDATE sync_runs SET complete=1 WHERE id=?1", params![run])?;
        let mut stmt = self.conn.prepare(
            "SELECT article FROM remote_state WHERE account=?1 AND key=?2 AND present=?3 AND seq<=?4 \
             AND article NOT IN (SELECT article FROM sync_seen WHERE run=?5)",
        )?;
        let rows = stmt.query_map(params![account, key, listed, start_seq, run], |r| r.get::<_, i64>(0))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(ArticleId::from_db(r?));
        }
        Ok(out)
    }

    /// Applies the opposite of the listed value to candidates confirmed absent by a second pass.
    /// Articles stay in the database: leaving a stream never deletes them.
    pub fn resolve_candidates(&mut self, run: i64, confirmed: &[ArticleId]) -> Result<usize> {
        let account = self.account.clone();
        let tx = self.conn.transaction()?;
        let (key, listed, start_seq): (String, i64, i64) = tx
            .query_row(
                "SELECT key,listed_value,start_seq FROM sync_runs WHERE id=?1 AND account=?2 AND complete=1",
                params![run, account],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .map_err(|_| Error::InvalidRequest("run is not complete".into()))?;
        let mut n = 0;
        for id in confirmed {
            if observe_tx(&tx, &account, *id, &key, listed == 0, start_seq)? {
                n += 1;
            }
        }
        tx.commit()?;
        Ok(n)
    }

    /// Ids of all articles currently known with the given last-known value.
    pub fn articles_with(&self, m: &Membership, present: bool) -> Result<Vec<ArticleId>> {
        let mut stmt = self.conn.prepare("SELECT article FROM remote_state WHERE account=?1 AND key=?2 AND present=?3 ORDER BY article")?;
        let rows = stmt.query_map(params![self.account, m.key(), present as i64], |r| r.get::<_, i64>(0))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(ArticleId::from_db(r?));
        }
        Ok(out)
    }

    /// Applies an observed server value for one membership from a verification request.
    pub fn observe(&mut self, article: ArticleId, m: &Membership, present: bool, start_seq: i64) -> Result<bool> {
        let account = self.account.clone();
        let tx = self.conn.transaction()?;
        let r = observe_tx(&tx, &account, article, &m.key(), present, start_seq)?;
        tx.commit()?;
        Ok(r)
    }
}

// ---- organization, counts, org ops ----------------------------------------------------------

impl Store {
    /// Replaces the subscription list. Only call with a complete, successfully parsed listing.
    pub fn replace_subscriptions(&mut self, subs: &[Subscription]) -> Result<()> {
        let account = self.account.clone();
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM subscriptions WHERE account=?1", params![account])?;
        tx.execute("DELETE FROM subscription_folders WHERE account=?1", params![account])?;
        for s in subs {
            tx.execute(
                "INSERT INTO subscriptions(account,id,title,url,html_url,sort_id,first_item_usec) VALUES(?1,?2,?3,?4,?5,?6,?7)",
                params![account, s.id, s.title, s.url, s.html_url, s.sort_id, s.first_item_usec],
            )?;
            for f in &s.folders {
                tx.execute("INSERT OR IGNORE INTO subscription_folders(account,feed,folder) VALUES(?1,?2,?3)", params![account, s.id, f])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn subscriptions(&self) -> Result<Vec<Subscription>> {
        let mut stmt = self.conn.prepare(
            "SELECT id,title,url,html_url,sort_id,first_item_usec FROM subscriptions WHERE account=?1 ORDER BY title COLLATE NOCASE, id",
        )?;
        let rows = stmt.query_map(params![self.account], |r| {
            Ok(Subscription {
                id: r.get(0)?,
                title: r.get(1)?,
                url: r.get(2)?,
                html_url: r.get(3)?,
                sort_id: r.get(4)?,
                first_item_usec: r.get(5)?,
                folders: Vec::new(),
            })
        })?;
        let mut subs: Vec<Subscription> = rows.collect::<rusqlite::Result<_>>()?;
        for s in &mut subs {
            let mut st = self.conn.prepare("SELECT folder FROM subscription_folders WHERE account=?1 AND feed=?2 ORDER BY folder")?;
            let rows = st.query_map(params![self.account, s.id], |r| r.get::<_, String>(0))?;
            s.folders = rows.collect::<rusqlite::Result<_>>()?;
        }
        Ok(subs)
    }

    pub fn subscription(&self, id: &str) -> Result<Option<Subscription>> {
        Ok(self.subscriptions()?.into_iter().find(|s| s.id == id))
    }

    pub fn replace_tags(&mut self, tags: &[TagInfo]) -> Result<()> {
        let account = self.account.clone();
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM tags WHERE account=?1", params![account])?;
        for t in tags {
            tx.execute(
                "INSERT OR REPLACE INTO tags(account,id,kind,sort_id) VALUES(?1,?2,?3,?4)",
                params![account, t.id, t.kind, t.sort_id],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn tags(&self) -> Result<Vec<TagInfo>> {
        let mut stmt = self.conn.prepare("SELECT id,kind,sort_id FROM tags WHERE account=?1 ORDER BY id")?;
        let rows = stmt.query_map(params![self.account], |r| Ok(TagInfo { id: r.get(0)?, kind: r.get(1)?, sort_id: r.get(2)? }))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn set_unread_counts(&mut self, counts: &[(String, i64, i64)]) -> Result<()> {
        let account = self.account.clone();
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM unread_counts WHERE account=?1", params![account])?;
        for (s, c, newest) in counts {
            tx.execute(
                "INSERT OR REPLACE INTO unread_counts(account,stream,count,newest_usec) VALUES(?1,?2,?3,?4)",
                params![account, s, c, newest],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn unread_count(&self, stream: &str) -> Result<Option<i64>> {
        Ok(self
            .conn
            .query_row("SELECT count FROM unread_counts WHERE account=?1 AND stream=?2", params![self.account, stream], |r| r.get(0))
            .optional()?)
    }

    pub fn set_stream_order(&mut self, stream: &str, ordering: &str) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO stream_order(account,stream,ordering) VALUES(?1,?2,?3)",
            params![self.account, stream, ordering],
        )?;
        Ok(())
    }

    pub fn stream_order(&self, stream: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row("SELECT ordering FROM stream_order WHERE account=?1 AND stream=?2", params![self.account, stream], |r| r.get(0))
            .optional()?)
    }

    /// Queues an organization operation and applies its effect to the local copy in the same
    /// transaction, so the UI reflects it immediately. Kinds and JSON payloads:
    /// `subscription_edit` {action: edit|subscribe|unsubscribe, stream, title?, add_folder?, remove_folder?},
    /// `quickadd` {feed}, `rename_tag` {source, dest}, `disable_tag` {stream},
    /// `stream_order` {stream, ordering}, `active_search_create` {...spec}, `active_search_delete` {id}.
    pub fn enqueue_org(&mut self, kind: &str, payload: &str) -> Result<i64> {
        let account = self.account.clone();
        let v: serde_json::Value = serde_json::from_str(payload).map_err(|_| Error::InvalidRequest("bad payload".into()))?;
        let tx = self.conn.transaction()?;
        match kind {
            "subscription_edit" => {
                let stream = v["stream"].as_str().unwrap_or_default();
                match v["action"].as_str().unwrap_or("edit") {
                    "unsubscribe" => {
                        tx.execute("DELETE FROM subscriptions WHERE account=?1 AND id=?2", params![account, stream])?;
                        tx.execute("DELETE FROM subscription_folders WHERE account=?1 AND feed=?2", params![account, stream])?;
                    }
                    _ => {
                        if let Some(t) = v["title"].as_str() {
                            tx.execute("UPDATE subscriptions SET title=?3 WHERE account=?1 AND id=?2", params![account, stream, t])?;
                        }
                        if let Some(f) = v["add_folder"].as_str() {
                            tx.execute(
                                "INSERT OR IGNORE INTO subscription_folders(account,feed,folder) VALUES(?1,?2,?3)",
                                params![account, stream, f],
                            )?;
                        }
                        if let Some(f) = v["remove_folder"].as_str() {
                            tx.execute(
                                "DELETE FROM subscription_folders WHERE account=?1 AND feed=?2 AND folder=?3",
                                params![account, stream, f],
                            )?;
                        }
                    }
                }
            }
            "rename_tag" => {
                let src = v["source"].as_str().unwrap_or_default();
                let dest = v["dest"].as_str().unwrap_or_default();
                if let (Some(s), true) = (src.strip_prefix("user/-/label/"), !dest.is_empty()) {
                    tx.execute("UPDATE subscription_folders SET folder=?3 WHERE account=?1 AND folder=?2", params![account, s, dest])?;
                    tx.execute("UPDATE tags SET id=?3 WHERE account=?1 AND id=?2", params![account, src, format!("user/-/label/{dest}")])?;
                    tx.execute(
                        "UPDATE remote_state SET key=?3 WHERE account=?1 AND key=?2",
                        params![account, format!("label:{s}"), format!("label:{dest}")],
                    )?;
                }
            }
            "disable_tag" => {
                let stream = v["stream"].as_str().unwrap_or_default();
                if let Some(s) = stream.strip_prefix("user/-/label/") {
                    tx.execute("DELETE FROM subscription_folders WHERE account=?1 AND folder=?2", params![account, s])?;
                    tx.execute("DELETE FROM remote_state WHERE account=?1 AND key=?2", params![account, format!("label:{s}")])?;
                }
                tx.execute("DELETE FROM tags WHERE account=?1 AND id=?2", params![account, stream])?;
            }
            "stream_order" => {
                tx.execute(
                    "INSERT OR REPLACE INTO stream_order(account,stream,ordering) VALUES(?1,?2,?3)",
                    params![account, v["stream"].as_str().unwrap_or_default(), v["ordering"].as_str().unwrap_or_default()],
                )?;
            }
            "quickadd" | "active_search_create" => {}
            "active_search_delete" => {
                tx.execute("DELETE FROM tags WHERE account=?1 AND id=?2", params![account, v["id"].as_str().unwrap_or_default()])?;
            }
            other => return Err(Error::InvalidRequest(format!("unknown operation {other}"))),
        }
        tx.execute("INSERT INTO org_ops(account,kind,payload,created) VALUES(?1,?2,?3,?4)", params![account, kind, payload, now_secs()])?;
        let id = tx.last_insert_rowid();
        tx.commit()?;
        Ok(id)
    }

    /// True while any organization operation has not been acknowledged.
    pub fn org_pending(&self) -> Result<bool> {
        Ok(self.conn.query_row("SELECT COUNT(*) FROM org_ops WHERE account=?1 AND state<>'failed'", params![self.account], |r| {
            r.get::<_, i64>(0)
        })? > 0)
    }

    /// Operations still to be sent, oldest first, marking them in flight.
    pub fn begin_org(&mut self) -> Result<Vec<OrgOp>> {
        let account = self.account.clone();
        let tx = self.conn.transaction()?;
        let ops: Vec<OrgOp> = {
            let mut stmt = tx.prepare("SELECT id,kind,payload,state FROM org_ops WHERE account=?1 AND state='pending' ORDER BY id")?;
            let rows =
                stmt.query_map(params![account], |r| Ok(OrgOp { id: r.get(0)?, kind: r.get(1)?, payload: r.get(2)?, state: r.get(3)? }))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        for op in &ops {
            tx.execute("UPDATE org_ops SET state='in_flight' WHERE id=?1", params![op.id])?;
        }
        tx.commit()?;
        Ok(ops)
    }

    /// Drops a failed (or unwanted) operation; the next refresh restores the server's view.
    pub fn discard_org(&mut self, id: i64) -> Result<()> {
        self.finish_org(id)
    }

    pub fn finish_org(&mut self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM org_ops WHERE id=?1 AND account=?2", params![id, self.account])?;
        Ok(())
    }

    pub fn set_org_state(&mut self, id: i64, state: &str, error: &str) -> Result<()> {
        self.conn.execute("UPDATE org_ops SET state=?3, error=?4 WHERE id=?1 AND account=?2", params![id, self.account, state, error])?;
        Ok(())
    }

    /// Startup recovery: operations in flight at a crash have an unknown outcome and must be
    /// reconciled against server listings before any retry of a non-idempotent kind.
    pub fn recover_org(&mut self) -> Result<usize> {
        Ok(self
            .conn
            .execute("UPDATE org_ops SET state='needs_reconciliation' WHERE account=?1 AND state='in_flight'", params![self.account])?)
    }

    pub fn org_ops(&self) -> Result<Vec<OrgOp>> {
        let mut stmt = self.conn.prepare("SELECT id,kind,payload,state FROM org_ops WHERE account=?1 ORDER BY id")?;
        let rows =
            stmt.query_map(params![self.account], |r| Ok(OrgOp { id: r.get(0)?, kind: r.get(1)?, payload: r.get(2)?, state: r.get(3)? }))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
}

// ---- content bookkeeping --------------------------------------------------------------------

impl Store {
    pub fn set_download_state(&mut self, article: ArticleId, state: DownloadState) -> Result<()> {
        self.conn.execute(
            "UPDATE articles SET download_state=?3 WHERE account=?1 AND id=?2",
            params![self.account, article.to_db(), state.as_str()],
        )?;
        Ok(())
    }

    pub fn download_state(&self, article: ArticleId) -> Result<DownloadState> {
        let s: Option<String> = self
            .conn
            .query_row("SELECT download_state FROM articles WHERE account=?1 AND id=?2", params![self.account, article.to_db()], |r| {
                r.get(0)
            })
            .optional()?;
        Ok(s.map(|s| DownloadState::parse(&s)).unwrap_or(DownloadState::NotDownloaded))
    }

    pub fn set_pinned(&mut self, article: ArticleId, pinned: bool) -> Result<()> {
        self.conn
            .execute("UPDATE articles SET pinned=?3 WHERE account=?1 AND id=?2", params![self.account, article.to_db(), pinned as i64])?;
        Ok(())
    }

    /// Publishes a new content version and indexes its text, in one transaction.
    #[allow(clippy::too_many_arguments)]
    pub fn publish_version(
        &mut self,
        article: ArticleId,
        raw_hash: &str,
        hash: &str,
        dir: &str,
        bytes: i64,
        title: &str,
        body_text: &str,
        state: DownloadState,
    ) -> Result<ContentVersion> {
        let account = self.account.clone();
        let tx = self.conn.transaction()?;
        let version: i64 = tx.query_row(
            "SELECT COALESCE(MAX(version),0)+1 FROM content_versions WHERE account=?1 AND article=?2",
            params![account, article.to_db()],
            |r| r.get(0),
        )?;
        tx.execute(
            "INSERT INTO content_versions(account,article,version,hash,raw_hash,dir,bytes,created) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
            params![account, article.to_db(), version, hash, raw_hash, dir, bytes, now_secs()],
        )?;
        tx.execute("DELETE FROM article_fts WHERE account=?1 AND article=?2", params![account, article.to_db()])?;
        tx.execute(
            "INSERT INTO article_fts(account,article,title,body) VALUES(?1,?2,?3,?4)",
            params![account, article.to_db(), title, body_text],
        )?;
        tx.execute(
            "UPDATE articles SET download_state=?3, words=?4 WHERE account=?1 AND id=?2",
            params![account, article.to_db(), state.as_str(), crate::content::word_count(body_text) as i64],
        )?;
        tx.commit()?;
        Ok(ContentVersion { version, hash: hash.into(), raw_hash: raw_hash.into(), dir: dir.into(), bytes })
    }

    pub fn latest_version(&self, article: ArticleId) -> Result<Option<ContentVersion>> {
        Ok(self
            .conn
            .query_row(
                "SELECT version,hash,raw_hash,dir,bytes FROM content_versions WHERE account=?1 AND article=?2 ORDER BY version DESC LIMIT 1",
                params![self.account, article.to_db()],
                |r| Ok(ContentVersion { version: r.get(0)?, hash: r.get(1)?, raw_hash: r.get(2)?, dir: r.get(3)?, bytes: r.get(4)? }),
            )
            .optional()?)
    }

    pub fn versions(&self, article: ArticleId) -> Result<Vec<ContentVersion>> {
        let mut stmt = self
            .conn
            .prepare("SELECT version,hash,raw_hash,dir,bytes FROM content_versions WHERE account=?1 AND article=?2 ORDER BY version")?;
        let rows = stmt.query_map(params![self.account, article.to_db()], |r| {
            Ok(ContentVersion { version: r.get(0)?, hash: r.get(1)?, raw_hash: r.get(2)?, dir: r.get(3)?, bytes: r.get(4)? })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn delete_version(&mut self, article: ArticleId, version: i64) -> Result<()> {
        self.conn.execute(
            "DELETE FROM content_versions WHERE account=?1 AND article=?2 AND version=?3",
            params![self.account, article.to_db(), version],
        )?;
        Ok(())
    }

    /// Drops all cached content bookkeeping for an article (eviction). The article row, its
    /// raw API object, positions and pending intents are kept.
    pub fn clear_content(&mut self, article: ArticleId) -> Result<()> {
        let account = self.account.clone();
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM content_versions WHERE account=?1 AND article=?2", params![account, article.to_db()])?;
        tx.execute("DELETE FROM assets WHERE account=?1 AND article=?2", params![account, article.to_db()])?;
        tx.execute("DELETE FROM article_fts WHERE account=?1 AND article=?2", params![account, article.to_db()])?;
        tx.execute("UPDATE articles SET download_state='not_downloaded' WHERE account=?1 AND id=?2", params![account, article.to_db()])?;
        tx.commit()?;
        Ok(())
    }

    pub fn lease(&mut self, article: ArticleId, version: i64) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO leases(account,article,version) VALUES(?1,?2,?3)",
            params![self.account, article.to_db(), version],
        )?;
        Ok(())
    }

    pub fn release_lease(&mut self, article: ArticleId, version: i64) -> Result<()> {
        self.conn.execute(
            "DELETE FROM leases WHERE account=?1 AND article=?2 AND version=?3",
            params![self.account, article.to_db(), version],
        )?;
        Ok(())
    }

    pub fn is_leased(&self, article: ArticleId) -> Result<bool> {
        Ok(self
            .conn
            .query_row("SELECT 1 FROM leases WHERE account=?1 AND article=?2 LIMIT 1", params![self.account, article.to_db()], |_| Ok(true))
            .optional()?
            .unwrap_or(false))
    }

    /// Leases do not survive a restart.
    pub fn clear_leases(&mut self) -> Result<()> {
        self.conn.execute("DELETE FROM leases WHERE account=?1", params![self.account])?;
        Ok(())
    }

    pub fn set_asset(&mut self, article: ArticleId, name: &str, url: &str, state: &str, bytes: i64) -> Result<()> {
        self.conn.execute(
            "INSERT INTO assets(account,article,name,url,state,bytes,tries) VALUES(?1,?2,?3,?4,?5,?6,?7) \
             ON CONFLICT(account,article,name) DO UPDATE SET state=excluded.state, bytes=excluded.bytes, \
             tries=assets.tries+CASE WHEN excluded.state='failed' THEN 1 ELSE 0 END",
            params![self.account, article.to_db(), name, url, state, bytes, if state == "failed" { 1 } else { 0 }],
        )?;
        Ok(())
    }

    pub fn assets(&self, article: ArticleId) -> Result<Vec<AssetRow>> {
        let mut stmt = self.conn.prepare("SELECT name,url,state,bytes,tries FROM assets WHERE account=?1 AND article=?2 ORDER BY name")?;
        let rows = stmt.query_map(params![self.account, article.to_db()], |r| {
            Ok(AssetRow { name: r.get(0)?, url: r.get(1)?, state: r.get(2)?, bytes: r.get(3)?, tries: r.get(4)? })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Cached bytes per article for budgeting and eviction, with protection flags.
    pub fn cache_usage(&self) -> Result<Vec<CacheEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT a.id, a.timestamp_usec, a.pinned, \
             COALESCE((SELECT SUM(bytes) FROM content_versions v WHERE v.account=a.account AND v.article=a.id),0) + \
             COALESCE((SELECT SUM(bytes) FROM assets s WHERE s.account=a.account AND s.article=a.id AND s.state='done'),0), \
             COALESCE((SELECT present FROM remote_state WHERE account=a.account AND article=a.id AND key='starred'),0), \
             COALESCE((SELECT 1 FROM intents WHERE account=a.account AND article=a.id LIMIT 1),0), \
             COALESCE((SELECT 1 FROM leases WHERE account=a.account AND article=a.id LIMIT 1),0), \
             COALESCE((SELECT desired FROM intents WHERE account=a.account AND article=a.id AND key='starred'),NULL) \
             FROM articles a WHERE a.account=?1 AND a.download_state<>'not_downloaded' ORDER BY a.timestamp_usec ASC",
        )?;
        let rows = stmt.query_map(params![self.account], |r| {
            let remote_star: i64 = r.get(4)?;
            let intent_star: Option<i64> = r.get(7)?;
            Ok(CacheEntry {
                article: ArticleId::from_db(r.get(0)?),
                timestamp_usec: r.get(1)?,
                pinned: r.get::<_, i64>(2)? != 0,
                bytes: r.get(3)?,
                starred: intent_star.map(|v| v != 0).unwrap_or(remote_star != 0),
                has_pending: r.get::<_, i64>(5)? != 0,
                leased: r.get::<_, i64>(6)? != 0,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
}

/// Why an article is a download candidate, in priority order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum CandidateKind {
    Starred,
    Pinned,
    RecentUnread,
}

impl Store {
    /// Articles worth caching that are not cached yet: starred, pinned-folder, then unread
    /// newer than `unread_cutoff_usec`. Newest first within each kind.
    pub fn download_candidates(&self, unread_cutoff_usec: i64, limit: usize) -> Result<Vec<(ArticleId, CandidateKind)>> {
        let mut stmt = self.conn.prepare(
            "SELECT a.id, \
             CASE WHEN COALESCE((SELECT desired FROM intents WHERE account=a.account AND article=a.id AND key='starred'),\
                                 (SELECT present FROM remote_state WHERE account=a.account AND article=a.id AND key='starred'),0)=1 THEN 0 \
                  WHEN a.pinned=1 THEN 1 ELSE 2 END AS kind \
             FROM articles a WHERE a.account=?1 AND a.download_state='not_downloaded' AND (kind<2 OR \
               (COALESCE((SELECT desired FROM intents WHERE account=a.account AND article=a.id AND key='read'),\
                         (SELECT present FROM remote_state WHERE account=a.account AND article=a.id AND key='read'),0)=0 \
                AND a.timestamp_usec>=?2)) \
             ORDER BY kind, a.timestamp_usec DESC, a.id DESC LIMIT ?3",
        )?;
        let rows = stmt.query_map(params![self.account, unread_cutoff_usec, limit as i64], |r| {
            let kind = match r.get::<_, i64>(1)? {
                0 => CandidateKind::Starred,
                1 => CandidateKind::Pinned,
                _ => CandidateKind::RecentUnread,
            };
            Ok((ArticleId::from_db(r.get(0)?), kind))
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Marks articles of feeds in the given folders as pinned (and unpins all others).
    pub fn sync_pins(&mut self, folders: &[String]) -> Result<()> {
        let account = self.account.clone();
        let tx = self.conn.transaction()?;
        tx.execute("UPDATE articles SET pinned=0 WHERE account=?1 AND pinned=1", params![account])?;
        for f in folders {
            tx.execute(
                "UPDATE articles SET pinned=1 WHERE account=?1 AND origin_stream IN \
                 (SELECT feed FROM subscription_folders WHERE account=?1 AND folder=?2)",
                params![account, f],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn downloaded_articles(&self) -> Result<Vec<ArticleId>> {
        let mut stmt = self.conn.prepare("SELECT id FROM articles WHERE account=?1 AND download_state<>'not_downloaded' ORDER BY id")?;
        let rows = stmt.query_map(params![self.account], |r| r.get::<_, i64>(0))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(ArticleId::from_db(r?));
        }
        Ok(out)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CacheEntry {
    pub article: ArticleId,
    pub timestamp_usec: i64,
    pub pinned: bool,
    pub bytes: i64,
    pub starred: bool,
    pub has_pending: bool,
    pub leased: bool,
}
