//! What the user can do to an article or to the library, and how it is applied: every action
//! is committed to the local database first and only then announced to the sync worker.

use crate::context::Context;
use inoink::content::relocate;
use inoink::model::*;
use crate::settings::HeadlineTap;
use crate::view::inoink::state::{OpenMode, Tab};
use inoink::runtime::Command;
use std::path::PathBuf;

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum ArticleAction {
    ToggleStar,
    /// Mark read and remove the star. Undoable.
    Done,
    /// Mark read (or unread) without touching the star.
    ToggleRead,
    /// Mark read, leaving the star alone.
    MarkRead,
    ToggleLike,
    ToggleBroadcast,
    AddTag(String),
    RemoveTag(String),
    LoadOriginal,
    RefreshComments,
}

/// Which cached document of an item is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocKind {
    Article,
    Comments,
}

/// Menu choices of the list and management screens.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum InoInkEntry {
    Tab(Tab),
    /// Open a category (folder, feed, tag...) from the Categories tab.
    OpenCategory(Collection),
    /// Mark read what was seen in this list and not starred (the end-of-list card).
    MarkSeenRead,
    ConfirmMarkSeenRead,
    UnreadOnly(bool),
    ViewGroup(ArticleGroup),
    ViewSort(ArticleSort),
    CategorySort(CategorySort),
    SyncNow,
    Article(ArticleAction),
    UndoDone,
    PromptNewTag,
    /// Ask for the address of a feed to follow.
    Subscribe,
    PromptRenameFeed(String),
    MoveFeed { feed: String, add: Option<String>, remove: Option<String> },
    PromptNewFolderForFeed(String),
    Unsubscribe(String),
    ConfirmUnsubscribe(String),
    PromptRenameFolder(String),
    DeleteFolder(String),
    ConfirmDeleteFolder(String),
    PinFolder(String, bool),
    PromptNewMonitoring,
    DeleteMonitoring(String),
    ConfirmDeleteMonitoring(String),
    CacheBudget(u64),
    UnreadDays(i64),
    /// Show a frozen, counted selection of the current list before marking it read.
    MarkListRead,
    ConfirmMarkListRead,
    OpenDetails(ArticleId),
    SetHeadlineTap(HeadlineTap),
    SetQueueTitles(crate::settings::QueueTitles),
    /// A feed's choice of loading the original page's text for its articles.
    SetFullText(String, inoink::store::FullText),
    /// Open the previous or next article of the list (the titles row above the footer).
    QueueArticle(crate::geom::CycleDir),
    /// Open a linked page in a reading view.
    OpenLink(String),
    ShowQr(String),
    /// Follow a site or feed address in Inoreader.
    SubscribeUrl(String),
    /// Show the open article's address as a QR code.
    QrArticle,
    /// Send a link (address, title) to Karakeep.
    ShareLink(String, String),
    /// Send the open article's address to Karakeep.
    ShareArticle,
}

#[derive(Debug, Default)]
pub struct ActionOutcome {
    pub message: Option<String>,
    /// The reader should close and return to the list.
    pub close: bool,
}

pub const SAVE_FAILED: &str = "Could not save change";

fn flag_action(context: &mut Context, id: ArticleId, m: Membership) -> Result<ActionOutcome, String> {
    let store = context.inoink.store_mut().ok_or(SAVE_FAILED)?;
    let cur = store.effective(id, &m).map_err(|_| SAVE_FAILED)?;
    store.apply_intent(id, &m, !cur).map_err(|_| SAVE_FAILED)?;
    context.inoink.changed();
    Ok(ActionOutcome::default())
}

/// Applies an article action. On error nothing has changed and the message is user-facing.
pub fn apply_article_action(context: &mut Context, id: ArticleId, action: &ArticleAction) -> Result<ActionOutcome, String> {
    match action {
        ArticleAction::ToggleStar => flag_action(context, id, Membership::Starred),
        ArticleAction::ToggleRead => flag_action(context, id, Membership::Read),
        ArticleAction::MarkRead => {
            let store = context.inoink.store_mut().ok_or(SAVE_FAILED)?;
            store.apply_intent(id, &Membership::Read, true).map_err(|_| SAVE_FAILED)?;
            context.inoink.changed();
            Ok(ActionOutcome::default())
        }
        ArticleAction::ToggleLike => flag_action(context, id, Membership::Like),
        ArticleAction::ToggleBroadcast => flag_action(context, id, Membership::Broadcast),
        ArticleAction::AddTag(name) | ArticleAction::RemoveTag(name) => {
            let name = name.trim();
            if name.is_empty() || name.starts_with("user/") || name.chars().any(|c| c.is_control()) {
                return Err("Enter a tag name".into());
            }
            let add = matches!(action, ArticleAction::AddTag(_));
            let store = context.inoink.store_mut().ok_or(SAVE_FAILED)?;
            store.apply_intent(id, &Membership::Label(name.to_string()), add).map_err(|_| SAVE_FAILED)?;
            context.inoink.changed();
            Ok(ActionOutcome::default())
        }
        ArticleAction::Done => {
            let title = {
                let store = context.inoink.store().ok_or(SAVE_FAILED)?;
                store.article_record(id).ok().flatten().map(|r| r.title).unwrap_or_default()
            };
            let store = context.inoink.store_mut().ok_or(SAVE_FAILED)?;
            let undo = store.done(id).map_err(|_| SAVE_FAILED)?;
            context.inoink.last_undo = Some((undo, title));
            context.inoink.changed();
            // The Undo bar at the top of the list is the confirmation; no notification covers it.
            Ok(ActionOutcome { message: None, close: true })
        }
        ArticleAction::LoadOriginal => {
            if context.inoink.handle.is_none() {
                return Err("Sign in to load the original page".into());
            }
            context.inoink.submit(Command::ExtractOriginal(id));
            Ok(ActionOutcome { message: Some("Loading the original page".into()), close: false })
        }
        ArticleAction::RefreshComments => {
            if context.inoink.handle.is_none() {
                return Err("Sign in to refresh the comments".into());
            }
            context.inoink.opening_comments = Some(id);
            context.inoink.submit(Command::RefreshComments(id));
            Ok(ActionOutcome { message: Some("Refreshing the comments".into()), close: false })
        }
    }
}

/// Reverts the most recent Done.
pub fn undo_done(context: &mut Context) -> Result<ActionOutcome, String> {
    let Some((undo, _)) = context.inoink.last_undo.clone() else {
        return Ok(ActionOutcome::default());
    };
    let store = context.inoink.store_mut().ok_or(SAVE_FAILED)?;
    store.undo(undo).map_err(|_| SAVE_FAILED)?;
    context.inoink.last_undo = None;
    context.inoink.changed();
    Ok(ActionOutcome { message: Some("Restored".into()), close: false })
}

/// What is needed to show an article.
#[derive(Debug, Clone)]
pub struct OpenArticle {
    pub id: ArticleId,
    /// Absolute path of the published HTML (the document root is its directory).
    pub path: PathBuf,
    pub version: i64,
    pub content_hash: String,
    /// Text offset to resume at (relocated when the content version changed).
    pub offset: Option<usize>,
    pub kind: DocKind,
}

pub enum OpenResult {
    Ready(Box<OpenArticle>),
    /// Content is being fetched; the article opens when it arrives.
    Fetching,
    /// Nothing cached and no way to fetch it.
    Unavailable(String),
}

/// Finds the published content for an article and takes a lease on it so the version stays
/// until the reader closes.
pub fn open_article(context: &mut Context, id: ArticleId) -> OpenResult {
    let at_end = std::mem::take(&mut context.inoink.open_at_end);
    let version = context.inoink.store().and_then(|s| s.latest_version(id).ok().flatten());
    let Some(v) = version else {
        return fetch_for_open(context, id);
    };
    let path = context.inoink.cache.version_path(id, v.version);
    if !path.is_file() {
        return fetch_for_open(context, id);
    }
    let mut offset = None;
    if let Some(store) = context.inoink.store_mut() {
        let _ = store.lease(id, v.version);
        if at_end {
            offset = Some(usize::MAX);
        } else if let Ok(Some(pos)) = store.position(id) {
            offset = if pos.content_hash == v.hash {
                Some(pos.offset as usize)
            } else {
                // The content changed (for example images arrived): find the same text again.
                let html = std::fs::read_to_string(&path).unwrap_or_default();
                relocate(&html, &pos.anchor, pos.offset as usize).or(Some((pos.offset as usize).min(html.len())))
            };
        }
    }
    OpenResult::Ready(Box::new(OpenArticle { id, path, version: v.version, content_hash: v.hash, offset, kind: DocKind::Article }))
}

/// Queues a background download of a neighbour of the open article when it is not cached yet
/// (once per session), so swiping to it does not wait for the network.
pub fn prefetch(context: &mut Context, id: ArticleId) {
    if context.inoink.handle.is_none() || !context.inoink.prefetched.insert(id) {
        return;
    }
    let cached = context
        .inoink
        .store()
        .and_then(|s| s.latest_version(id).ok().flatten())
        .map_or(false, |v| context.inoink.cache.version_path(id, v.version).is_file());
    if !cached {
        context.inoink.submit(Command::CacheNow(id));
    }
}

/// Opens the cached comments page of a link post (fetching it when it is missing).
pub fn open_comments(context: &mut Context, id: ArticleId) -> OpenResult {
    let path = context.inoink.cache.comments_path(id);
    if !path.is_file() {
        if context.inoink.handle.is_none() {
            return OpenResult::Unavailable("The comments are not available offline".into());
        }
        context.inoink.opening_comments = Some(id);
        context.inoink.submit(Command::RefreshComments(id));
        return OpenResult::Fetching;
    }
    let row = context.inoink.store().and_then(|s| s.discussion(id).ok().flatten()).unwrap_or_default();
    let hash = format!("comments:{}", row.fetched);
    let offset = if row.pos_hash == hash {
        Some(row.pos_offset as usize)
    } else if row.pos_anchor.is_empty() {
        None
    } else {
        let html = std::fs::read_to_string(&path).unwrap_or_default();
        relocate(&html, &row.pos_anchor, row.pos_offset as usize)
    };
    OpenResult::Ready(Box::new(OpenArticle { id, path, version: 0, content_hash: hash, offset, kind: DocKind::Comments }))
}

pub fn open_document(context: &mut Context, id: ArticleId, kind: DocKind) -> OpenResult {
    match kind {
        DocKind::Article => open_article(context, id),
        DocKind::Comments => open_comments(context, id),
    }
}

fn fetch_for_open(context: &mut Context, id: ArticleId) -> OpenResult {
    if context.inoink.handle.is_none() {
        return OpenResult::Unavailable("This article is not available offline".into());
    }
    context.inoink.opening = Some((id, OpenMode::Push));
    context.inoink.opening_at = Some(std::time::Instant::now());
    context.inoink.submit(Command::CacheNow(id));
    OpenResult::Fetching
}

pub fn release_article(context: &mut Context, id: ArticleId, version: i64) {
    if version == 0 {
        return;
    }
    if let Some(store) = context.inoink.store_mut() {
        let _ = store.release_lease(id, version);
    }
}

/// Queues a link for Karakeep. It is sent as soon as the network allows.
pub fn share_url(context: &mut Context, url: &str, title: &str) -> Result<String, String> {
    if context.inoink.share.karakeep.is_none() {
        return Err("Karakeep is not set up. See the setup guide.".into());
    }
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("Only web addresses can be shared".into());
    }
    let store = context.inoink.store_mut().ok_or(SAVE_FAILED)?;
    store.enqueue_share(inoink::share::KARAKEEP, url, title).map_err(|_| SAVE_FAILED.to_string())?;
    context.inoink.changed();
    Ok("Sending to Karakeep".into())
}

/// What a sync pass reports about shares: the first refusal, or how many were saved.
pub fn share_message(rep: &inoink::sync::CycleReport) -> Option<String> {
    if let Some(e) = rep.shares.iter().find_map(|s| s.error.as_ref()) {
        return Some(format!("Karakeep: {e}"));
    }
    match rep.shares.len() {
        0 => None,
        1 => {
            let t = inoink::util::shorten(&rep.shares[0].title, 40);
            Some(format!("Saved to Karakeep: {}", if t.is_empty() { "link" } else { &t }))
        }
        n => Some(format!("Saved {n} links to Karakeep")),
    }
}
