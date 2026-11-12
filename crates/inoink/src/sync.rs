//! Synchronization engine: sends local intent, reconciles with the server, fetches missing
//! articles and caches content. It is plain blocking code driven by [`crate::runtime`]; every
//! step is bounded by the quota and can be cancelled between requests.

use crate::api::{ApiClient, Clock, ContentsParams, IdsParams, Priority, SubAction, SubscriptionEdit};
use crate::cache::{Cache, Control};
use crate::content::images::Fetcher;
use crate::error::{Error, Result};
use crate::model::*;
use crate::store::{OrgOp, OutboxBatch, Store};
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct SyncConfig {
    pub batch_size: usize,
    /// Pages of 1000 ids per membership listing before the listing is called incomplete.
    pub id_page_cap: usize,
    pub starred_fetch_pages: usize,
    pub unread_fetch_pages: usize,
    /// Automatic cycles do not refresh or reconcile more often than this.
    pub min_interval_secs: i64,
    pub cache_per_cycle: usize,
}

impl Default for SyncConfig {
    fn default() -> Self {
        SyncConfig {
            batch_size: crate::api::MAX_EDIT_IDS,
            id_page_cap: 10,
            starred_fetch_pages: 5,
            unread_fetch_pages: 3,
            min_interval_secs: 900,
            cache_per_cycle: 20,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    Launch,
    Wake,
    Network,
    /// A local change was committed.
    Local,
    /// The user asked for it.
    Explicit,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SyncError {
    Offline,
    NeedsSignIn,
    QuotaPaused { secs: u64 },
    Other(String),
}

impl SyncError {
    fn from(e: &Error) -> SyncError {
        match e {
            Error::Offline => SyncError::Offline,
            Error::NeedsAuthorization => SyncError::NeedsSignIn,
            Error::QuotaPaused { reset_after_secs } => SyncError::QuotaPaused { secs: *reset_after_secs },
            other => SyncError::Other(other.to_string()),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CycleReport {
    pub sent_batches: usize,
    pub rejected: usize,
    /// Changes dropped because Inoreader cannot apply them (older than the feed's first item).
    pub cannot_unread: Vec<ArticleId>,
    pub org_done: usize,
    pub refreshed_org: bool,
    pub reconciled: Vec<&'static str>,
    pub incomplete: Vec<&'static str>,
    pub fetched: usize,
    /// Listed on the server but not retrievable within the page caps.
    pub missing: usize,
    pub cached: usize,
    pub storage_paused: bool,
    pub skipped_interval: bool,
    pub error: Option<SyncError>,
}

pub struct SyncEngine {
    pub store: Store,
    api: ApiClient,
    cache: Cache,
    fetcher: Arc<dyn Fetcher>,
    cfg: SyncConfig,
    pub cancel: Arc<AtomicBool>,
    clock: Arc<dyn Clock>,
    sleep: Arc<dyn Fn(std::time::Duration) + Send + Sync>,
    free_space: Arc<dyn Fn() -> Option<u64> + Send + Sync>,
}

impl SyncEngine {
    pub fn new(
        store: Store,
        api: ApiClient,
        cache: Cache,
        fetcher: Arc<dyn Fetcher>,
        clock: Arc<dyn Clock>,
        cfg: SyncConfig,
    ) -> SyncEngine {
        SyncEngine {
            store,
            api,
            cache,
            fetcher,
            cfg,
            cancel: Arc::new(AtomicBool::new(false)),
            clock,
            sleep: Arc::new(std::thread::sleep),
            free_space: Arc::new(|| None),
        }
    }

    pub fn with_sleep(mut self, f: Arc<dyn Fn(std::time::Duration) + Send + Sync>) -> SyncEngine {
        self.sleep = f;
        self
    }

    pub fn with_free_space(mut self, f: Arc<dyn Fn() -> Option<u64> + Send + Sync>) -> SyncEngine {
        self.free_space = f;
        self
    }

    pub fn cache(&self) -> &Cache {
        &self.cache
    }

    pub fn fetcher(&self) -> Arc<dyn Fetcher> {
        self.fetcher.clone()
    }

    /// Retries one image the user tapped.
    pub fn retry_image(&mut self, id: ArticleId, name: &str, ctl: &Control, fetcher: &dyn Fetcher) -> Result<DownloadState> {
        self.cache.retry_asset(&mut self.store, fetcher, id, name, ctl)
    }

    fn control(&self) -> Control {
        let sleep = self.sleep.clone();
        let free = self.free_space.clone();
        Control { cancel: self.cancel.clone(), sleep: Box::new(move |d| sleep(d)), free_space: Box::new(move || free()) }
    }

    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    pub fn api(&self, prio: Priority) -> ApiClient {
        self.api.with_priority(prio)
    }

    /// Startup recovery: resend anything that was in flight, reconcile uncertain organization
    /// operations and clean the content cache.
    pub fn recover(&mut self) -> Result<()> {
        self.store.recover_in_flight()?;
        self.store.recover_org()?;
        self.cache.recover(&mut self.store)?;
        if let Some(q) = self.store.get_meta("quota")? {
            self.api.restore_quota(&q);
        }
        Ok(())
    }

    fn save_quota(&mut self) {
        let _ = self.store.set_meta("quota", &self.api.quota_json());
    }

    // ---- outbox ---------------------------------------------------------------------------

    /// Sends pending per-article changes. Stops at the first transport/quota/auth error,
    /// leaving everything unsent still pending.
    pub fn flush_outbox(&mut self, prio: Priority, rep: &mut CycleReport) -> Result<()> {
        let api = self.api.with_priority(prio);
        let batches = self.store.begin_send(self.cfg.batch_size)?;
        let mut stop: Option<Error> = None;
        for batch in batches {
            if stop.is_some() || self.cancelled() {
                self.store.release(&batch)?;
                continue;
            }
            for b in self.split_unsendable(batch, rep)? {
                if stop.is_some() {
                    self.store.release(&b)?;
                    continue;
                }
                match api.edit_tag(&b.articles(), b.add.as_ref(), b.remove.as_ref()) {
                    Ok(()) => {
                        self.store.acknowledge(&b)?;
                        rep.sent_batches += 1;
                    }
                    Err(Error::InvalidRequest(_)) => {
                        // The server refuses this request. It stays marked in flight (parked)
                        // so it is not hammered; recovery resends it after a restart.
                        rep.rejected += b.entries.len();
                    }
                    Err(e) => {
                        // Unknown outcomes are fine to resend: these are state setters.
                        self.store.release(&b)?;
                        stop = Some(e);
                    }
                }
            }
        }
        self.save_quota();
        match stop {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    /// Drops "mark unread" for articles older than their feed's first item (Inoreader ignores
    /// those silently) and re-batches the remaining changes of affected articles separately.
    fn split_unsendable(&mut self, batch: OutboxBatch, rep: &mut CycleReport) -> Result<Vec<OutboxBatch>> {
        let mut rejected_articles = BTreeSet::new();
        for (a, m, desired, gen) in &batch.entries {
            if *m == Membership::Read && !*desired {
                if let Some(rec) = self.store.article_record(*a)? {
                    let first = self.store.subscription(&rec.origin_stream)?.map(|s| s.first_item_usec).unwrap_or(0);
                    if first > 0 && rec.timestamp_usec < first {
                        self.store.release(&OutboxBatch {
                            add: None,
                            remove: Some(Membership::Read),
                            entries: vec![(*a, m.clone(), *desired, *gen)],
                        })?;
                        self.store.reject_intent(*a, m, *gen)?;
                        rejected_articles.insert(*a);
                        rep.cannot_unread.push(*a);
                    }
                }
            }
        }
        if rejected_articles.is_empty() {
            return Ok(vec![batch]);
        }
        let mut whole = OutboxBatch { add: batch.add.clone(), remove: batch.remove.clone(), entries: vec![] };
        let mut singles = Vec::new();
        for e in batch.entries {
            if e.1 == Membership::Read && !e.2 && rejected_articles.contains(&e.0) {
                continue; // dropped above
            }
            if rejected_articles.contains(&e.0) {
                let (add, remove) = if e.2 { (Some(e.1.clone()), None) } else { (None, Some(e.1.clone())) };
                singles.push(OutboxBatch { add, remove, entries: vec![e] });
            } else {
                whole.entries.push(e);
            }
        }
        let mut out = Vec::new();
        if !whole.entries.is_empty() {
            out.push(whole);
        }
        out.extend(singles);
        Ok(out)
    }

    // ---- organization operations ------------------------------------------------------------

    fn run_org_op(&mut self, api: &ApiClient, op: &OrgOp) -> Result<()> {
        let v: serde_json::Value = serde_json::from_str(&op.payload)?;
        let s = |k: &str| v[k].as_str().map(String::from);
        match op.kind.as_str() {
            "subscription_edit" => {
                let action = match v["action"].as_str().unwrap_or("edit") {
                    "subscribe" => SubAction::Subscribe,
                    "unsubscribe" => SubAction::Unsubscribe,
                    _ => SubAction::Edit,
                };
                api.edit_subscription(&SubscriptionEdit {
                    action,
                    stream: StreamId::new(&s("stream").unwrap_or_default()),
                    title: s("title"),
                    add_folder: s("add_folder"),
                    remove_folder: s("remove_folder"),
                })
            }
            "quickadd" => {
                let r = api.quickadd(&s("feed").unwrap_or_default())?;
                if r.num_results == 0 {
                    Err(Error::InvalidRequest("no feed was found at that address".into()))
                } else {
                    Ok(())
                }
            }
            "rename_tag" => api.rename_tag(&StreamId::new(&s("source").unwrap_or_default()), &s("dest").unwrap_or_default()),
            "disable_tag" => api.disable_tag(&StreamId::new(&s("stream").unwrap_or_default())),
            "stream_order" => {
                let ids = crate::api::split_sort_ids(&s("ordering").unwrap_or_default());
                api.set_stream_order(&StreamId::new(&s("stream").unwrap_or_default()), &ids)
            }
            "active_search_create" => {
                let spec = crate::api::ActiveSearchSpec {
                    term: s("term").unwrap_or_default(),
                    filter_type: match v["filter_type"].as_str() {
                        Some("folder") => crate::api::FilterType::Folder,
                        Some("subscription") => crate::api::FilterType::Subscription,
                        _ => crate::api::FilterType::Public,
                    },
                    filter_id: s("filter_id"),
                    article_ids: v["article_ids"]
                        .as_array()
                        .map(|a| a.iter().filter_map(|x| x.as_str().and_then(|x| ArticleId::parse(x).ok())).collect())
                        .unwrap_or_default(),
                    search_filters: s("search_filters"),
                    search_match: s("search_match"),
                    search_feed_popularity: s("search_feed_popularity"),
                };
                api.create_active_search(&spec).map(|_| ())
            }
            "active_search_delete" => api.delete_active_search(&StreamId::new(&s("id").unwrap_or_default())),
            other => Err(Error::InvalidRequest(format!("unknown operation {other}"))),
        }
    }

    /// Whether replaying `kind` after an unknown outcome could duplicate or break something.
    fn replay_safe(kind: &str) -> bool {
        matches!(kind, "subscription_edit" | "stream_order")
    }

    pub fn flush_org(&mut self, prio: Priority, rep: &mut CycleReport) -> Result<()> {
        let api = self.api.with_priority(prio);
        let ops = self.store.begin_org()?;
        let mut stop: Option<Error> = None;
        for op in ops {
            if stop.is_some() || self.cancelled() {
                self.store.set_org_state(op.id, "pending", "")?;
                continue;
            }
            match self.run_org_op(&api, &op) {
                Ok(()) => {
                    self.store.finish_org(op.id)?;
                    rep.org_done += 1;
                }
                Err(Error::InvalidRequest(m)) => self.store.set_org_state(op.id, "failed", &m)?,
                Err(Error::UnknownOutcome(m)) => {
                    if Self::replay_safe(&op.kind) {
                        self.store.set_org_state(op.id, "pending", "")?;
                    } else if op.kind == "active_search_create" {
                        self.store.set_org_state(op.id, "failed", "The outcome is unknown. Check Monitoring before creating it again.")?;
                    } else {
                        self.store.set_org_state(op.id, "needs_reconciliation", &m)?;
                    }
                    stop = Some(Error::Offline);
                }
                Err(e) => {
                    // Nothing reached the server (offline, quota, auth) or it reported a
                    // transient failure: try again later.
                    self.store.set_org_state(op.id, "pending", "")?;
                    stop = Some(e);
                }
            }
        }
        self.save_quota();
        match stop {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    /// Resolves operations whose outcome is unknown by inspecting authoritative listings.
    pub fn reconcile_org(&mut self, prio: Priority) -> Result<()> {
        let uncertain: Vec<OrgOp> = self.store.org_ops()?.into_iter().filter(|o| o.state == "needs_reconciliation").collect();
        if uncertain.is_empty() {
            return Ok(());
        }
        let api = self.api.with_priority(prio);
        let subs = api.subscriptions()?;
        let tags = api.tags()?;
        for op in uncertain {
            let v: serde_json::Value = serde_json::from_str(&op.payload)?;
            let has_tag = |id: &str| tags.iter().any(|t| t.id == normalize_user(id));
            let done = match op.kind.as_str() {
                "quickadd" => {
                    let feed = v["feed"].as_str().unwrap_or_default();
                    Some(subs.iter().any(|s| s.url == feed || s.id == format!("feed/{feed}") || s.id == feed))
                }
                "rename_tag" => {
                    let src = v["source"].as_str().unwrap_or_default();
                    let dest = StreamId::label(v["dest"].as_str().unwrap_or_default());
                    Some(has_tag(dest.as_str()) && !has_tag(src))
                }
                "disable_tag" => Some(!has_tag(v["stream"].as_str().unwrap_or_default())),
                "active_search_delete" => Some(!has_tag(v["id"].as_str().unwrap_or_default())),
                _ => None,
            };
            match done {
                Some(true) => self.store.finish_org(op.id)?,
                Some(false) => self.store.set_org_state(op.id, "pending", "")?,
                None => self.store.set_org_state(op.id, "failed", "The outcome is unknown.")?,
            }
        }
        Ok(())
    }

    // ---- refresh and reconciliation -----------------------------------------------------------

    /// Refreshes subscriptions, folders/tags, unread counts and ordering. Skipped while local
    /// organization changes are unacknowledged (they would be overwritten by older data).
    pub fn refresh_organization(&mut self, prio: Priority) -> Result<bool> {
        if self.store.org_pending()? {
            return Ok(false);
        }
        let api = self.api.with_priority(prio);
        let subs = api.subscriptions()?;
        self.store.replace_subscriptions(&subs)?;
        let tags = api.tags()?;
        self.store.replace_tags(&tags)?;
        let counts = api.unread_counts()?;
        self.store.set_unread_counts(&counts.counts)?;
        if let Some(max) = counts.max {
            self.store.set_meta("unread_max", &max.to_string())?;
        }
        let prefs = api.stream_preferences()?;
        for (stream, ordering) in prefs {
            self.store.set_stream_order(&stream, &ordering)?;
        }
        self.store.set_meta("org_refreshed", &self.clock.now().to_string())?;
        Ok(true)
    }

    fn list_ids(&mut self, api: &ApiClient, run: i64, params: IdsParams) -> Result<(Vec<ArticleId>, bool)> {
        let mut unknown = Vec::new();
        let mut p = params;
        for _ in 0..self.cfg.id_page_cap {
            if self.cancelled() {
                return Ok((unknown, false));
            }
            let page = api.item_ids(&p)?;
            let ids: Vec<ArticleId> = page.refs.iter().map(|r| r.id).collect();
            unknown.extend(self.store.reconcile_page(run, &ids, page.continuation.as_deref())?);
            match page.continuation {
                Some(c) => p.continuation = Some(c),
                None => return Ok((unknown, true)),
            }
        }
        Ok((unknown, false))
    }

    /// Reconciles one membership against its server listing. Absence is only acted on after a
    /// complete listing *and* a second complete listing that agrees.
    pub fn reconcile_membership(
        &mut self,
        prio: Priority,
        key: &Membership,
        listed_value: bool,
        params: IdsParams,
    ) -> Result<(Vec<ArticleId>, bool)> {
        let api = self.api.with_priority(prio);
        let run = self.store.begin_reconcile(key, listed_value)?;
        let (unknown, complete) = match self.list_ids(&api, run, params.clone()) {
            Ok(r) => r,
            Err(e) => {
                self.store.finish_reconciliation(run, false)?;
                return Err(e);
            }
        };
        let candidates = self.store.finish_reconciliation(run, complete)?;
        if !candidates.is_empty() {
            let run2 = self.store.begin_reconcile(key, listed_value)?;
            let second = match self.list_ids(&api, run2, params) {
                Ok((_, true)) => self.store.finish_reconciliation(run2, true)?,
                Ok(_) => {
                    self.store.finish_reconciliation(run2, false)?;
                    return Ok((unknown, false));
                }
                Err(e) => {
                    self.store.finish_reconciliation(run2, false)?;
                    return Err(e);
                }
            };
            let again: BTreeSet<ArticleId> = second.into_iter().collect();
            let confirmed: Vec<ArticleId> = candidates.into_iter().filter(|c| again.contains(c)).collect();
            self.store.resolve_candidates(run2, &confirmed)?;
        }
        Ok((unknown, complete))
    }

    /// Pages `stream` newest-first, storing every article, until all `wanted` ids are found or
    /// the page cap is hit. Returns how many wanted articles were not found.
    pub fn fetch_wanted(
        &mut self,
        prio: Priority,
        stream: &StreamId,
        exclude_read: bool,
        wanted: &[ArticleId],
        max_pages: usize,
        rep: &mut CycleReport,
    ) -> Result<usize> {
        let api = self.api.with_priority(prio);
        let mut remaining: BTreeSet<ArticleId> = wanted.iter().copied().collect();
        let mut cont: Option<String> = None;
        for _ in 0..max_pages {
            if remaining.is_empty() || self.cancelled() {
                break;
            }
            let start_seq = self.store.seq()?;
            let page = api.contents(
                stream,
                &ContentsParams {
                    count: 100,
                    exclude_read,
                    continuation: cont.clone(),
                    annotations: true,
                    summaries: true,
                    ..Default::default()
                },
            )?;
            for mut rec in page.items {
                self.strip_folder_labels(&mut rec)?;
                remaining.remove(&ArticleId(rec.id));
                self.store.upsert_article(&rec, start_seq)?;
                rep.fetched += 1;
            }
            match page.continuation {
                Some(c) => cont = Some(c),
                None => break,
            }
        }
        Ok(remaining.len())
    }

    /// Folder names inherited from the feed are not user tags; drop them from `categories`.
    fn strip_folder_labels(&self, rec: &mut crate::store::ArticleRecord) -> Result<()> {
        if let Some(sub) = self.store.subscription(&rec.origin_stream)? {
            rec.categories.retain(|c| match normalize_user(c).strip_prefix("user/-/label/") {
                Some(n) => !sub.folders.iter().any(|f| f == n),
                None => true,
            });
        }
        Ok(())
    }

    /// Explicit historical browsing: stores one page of `collection` and returns the next
    /// continuation token, if any.
    pub fn browse(&mut self, collection: &Collection, continuation: Option<String>) -> Result<Option<String>> {
        let api = self.api.with_priority(Priority::Explicit);
        let start_seq = self.store.seq()?;
        let page = api.contents(
            &collection.stream_id(),
            &ContentsParams {
                count: 100,
                exclude_read: matches!(collection, Collection::Unread),
                only_liked: false,
                continuation,
                annotations: true,
                summaries: true,
                ..Default::default()
            },
        )?;
        for mut rec in page.items {
            self.strip_folder_labels(&mut rec)?;
            self.store.upsert_article(&rec, start_seq)?;
        }
        self.save_quota();
        Ok(page.continuation)
    }

    /// Downloads content for the highest-priority uncached articles, then evicts to budget.
    pub fn cache_pending(&mut self, rep: &mut CycleReport) -> Result<()> {
        let plan = self.cache.plan_downloads(&mut self.store, self.clock.now())?;
        rep.storage_paused = plan.paused.is_some();
        let ctl = self.control();
        for id in plan.ids.into_iter().take(self.cfg.cache_per_cycle) {
            if self.cancelled() {
                break;
            }
            match self.cache.cache_article(&mut self.store, self.fetcher.as_ref(), id, &ctl) {
                Ok(_) => rep.cached += 1,
                Err(Error::Storage(_)) => {
                    rep.storage_paused = true;
                    break;
                }
                Err(_) => {}
            }
        }
        self.cache.evict_to_budget(&mut self.store)?;
        Ok(())
    }

    /// Fetches the original page for `id` and publishes it as a new content version when it
    /// holds more text than the feed provided. Returns a user-facing outcome.
    pub fn extract_original(&mut self, id: ArticleId) -> Result<std::result::Result<DownloadState, String>> {
        use crate::content::extract::{extract_original, Extraction};
        let rec = self.store.article_record(id)?.ok_or_else(|| Error::InvalidRequest("unknown article".into()))?;
        let feed_text = crate::content::prepare_article(
            &rec.summary_html,
            None,
            &crate::content::ArticleMeta::default(),
            &crate::content::AssetMap::new(),
        )
        .text
        .chars()
        .count();
        let limits = self.cache.config().limits.clone();
        match extract_original(self.fetcher.as_ref(), &rec.canonical_url, feed_text, &limits, &self.cancel)? {
            Extraction::Extracted { html, .. } => {
                let ctl = self.control();
                let st = self.cache.publish_extracted(&mut self.store, self.fetcher.as_ref(), id, &html, &rec.canonical_url, &ctl)?;
                Ok(Ok(st))
            }
            Extraction::Failed(why) => Ok(Err(why)),
        }
    }

    /// Re-downloads the comments of a Hacker News or Lobsters link post.
    pub fn refresh_comments(&mut self, id: ArticleId) -> Result<u32> {
        let ctl = self.control();
        self.cache.refresh_comments(&mut self.store, self.fetcher.as_ref(), id, &ctl)
    }

    /// Fetches one article's content now (the user opened it online).
    pub fn cache_now(&mut self, id: ArticleId) -> Result<DownloadState> {
        let ctl = self.control();
        self.cache.cache_article(&mut self.store, self.fetcher.as_ref(), id, &ctl)
    }

    // ---- the cycle ---------------------------------------------------------------------------

    fn due(&self) -> bool {
        let last = self.store.get_meta("last_cycle").ok().flatten().and_then(|v| v.parse::<i64>().ok()).unwrap_or(0);
        self.clock.now() - last >= self.cfg.min_interval_secs
    }

    /// One synchronization pass. Never panics on network trouble: problems end the pass and are
    /// reported; everything unsent stays pending.
    pub fn run_cycle(&mut self, reason: Reason) -> CycleReport {
        let mut rep = CycleReport::default();
        let prio = if reason == Reason::Explicit { Priority::Explicit } else { Priority::Background };
        if let Err(e) = self.cycle_inner(reason, prio, &mut rep) {
            rep.error = Some(SyncError::from(&e));
        }
        self.save_quota();
        rep
    }

    fn cycle_inner(&mut self, reason: Reason, prio: Priority, rep: &mut CycleReport) -> Result<()> {
        // Writes first: local intent is the most valuable thing to get to the server.
        self.reconcile_org(prio)?;
        self.flush_org(prio, rep)?;
        self.flush_outbox(prio, rep)?;
        if reason == Reason::Local {
            return Ok(());
        }
        if reason != Reason::Explicit && !self.due() {
            rep.skipped_interval = true;
            return self.cache_pending(rep);
        }
        if self.cancelled() {
            return Ok(());
        }
        self.store.set_meta("last_cycle", &self.clock.now().to_string())?;
        rep.refreshed_org = self.refresh_organization(prio)?;

        // Starred first: it must include old articles regardless of publication time.
        let mut starred = IdsParams::new(StreamId::new(STATE_STARRED));
        starred.only_starred = true;
        starred.direct_ids = false;
        let (unknown, complete) = self.reconcile_membership(prio, &Membership::Starred, true, starred)?;
        rep.reconciled.push("starred");
        if !complete {
            rep.incomplete.push("starred");
        }
        if !unknown.is_empty() {
            rep.missing += self.fetch_wanted(prio, &StreamId::new(STATE_STARRED), false, &unknown, self.cfg.starred_fetch_pages, rep)?;
        }

        // Unread: ids not marked read.
        let mut unread = IdsParams::new(StreamId::new(STATE_READING_LIST));
        unread.exclude_read = true;
        let (unknown, complete) = self.reconcile_membership(prio, &Membership::Read, false, unread)?;
        rep.reconciled.push("unread");
        if !complete {
            rep.incomplete.push("unread");
        }
        if !unknown.is_empty() {
            rep.missing += self.fetch_wanted(prio, &StreamId::new(STATE_READING_LIST), true, &unknown, self.cfg.unread_fetch_pages, rep)?;
        }
        self.cache_pending(rep)
    }
}
