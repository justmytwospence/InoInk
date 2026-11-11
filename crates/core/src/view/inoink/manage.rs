//! Menus and operations for subscriptions, folders, tags, monitoring and offline downloads.
//! Everything ordinary stays out of the way: these live in secondary menus only.

use crate::context::Context;
use crate::view::inoink::actions::{InoInkEntry, SAVE_FAILED};
use crate::view::{EntryId, EntryKind};
use inoink::model::*;
use serde_json::json;

const CHUNK: usize = 10;

/// Splits a long list into pages with a trailing "More" entry.
pub fn paged(mut entries: Vec<EntryKind>) -> Vec<EntryKind> {
    if entries.len() <= CHUNK + 1 {
        return entries;
    }
    let rest = entries.split_off(CHUNK);
    entries.push(EntryKind::More(paged(rest)));
    entries
}

fn cmd(label: &str, e: InoInkEntry) -> EntryKind {
    EntryKind::Command(label.to_string(), EntryId::InoInk(e))
}

fn count_suffix(context: &Context, stream: &str) -> String {
    let Some(store) = context.inoink.store() else { return String::new() };
    let max = store.get_meta("unread_max").ok().flatten().and_then(|v| v.parse::<i64>().ok());
    match store.unread_count(stream) {
        Ok(Some(n)) if n > 0 => match max {
            Some(m) if n >= m => format!(" ({m}+)"),
            _ => format!(" ({n})"),
        },
        _ => String::new(),
    }
}

/// The collection drawer: Starred, Unread, All, then folders, feeds, tags and the rest.
pub fn collection_entries(context: &Context) -> Vec<EntryKind> {
    let current = &context.inoink.collection;
    let radio = |label: String, c: Collection| EntryKind::RadioButton(label, EntryId::InoInk(InoInkEntry::Collection(c.clone())), *current == c);
    let mut entries = vec![
        radio("Starred".into(), Collection::Starred),
        radio(format!("Unread{}", count_suffix(context, STATE_READING_LIST)), Collection::Unread),
        radio("All articles".into(), Collection::All),
        EntryKind::Separator,
    ];
    let Some(store) = context.inoink.store() else { return entries };
    let subs = store.subscriptions().unwrap_or_default();
    let tags = store.tags().unwrap_or_default();
    let mut folders: Vec<String> = subs.iter().flat_map(|s| s.folders.clone()).collect();
    for t in &tags {
        if t.kind == "folder" {
            if let Some(n) = t.id.strip_prefix("user/-/label/") {
                folders.push(n.to_string());
            }
        }
    }
    folders.sort();
    folders.dedup();
    if !folders.is_empty() {
        let list = folders.iter().map(|f| radio(format!("{f}{}", count_suffix(context, &format!("user/-/label/{f}"))), Collection::Folder(f.clone()))).collect();
        entries.push(EntryKind::SubMenu("Folders".into(), paged(list)));
    }
    if !subs.is_empty() {
        let list = subs.iter().map(|s| radio(format!("{}{}", s.title, count_suffix(context, &s.id)), Collection::Feed(StreamId::new(&s.id)))).collect();
        entries.push(EntryKind::SubMenu("Feeds".into(), paged(list)));
    }
    let user_tags: Vec<&inoink::store::TagInfo> = tags.iter().filter(|t| t.kind == "tag" && t.id.starts_with("user/-/label/")).collect();
    if !user_tags.is_empty() {
        let list = user_tags.iter().map(|t| {
            let n = t.id.trim_start_matches("user/-/label/").to_string();
            radio(n.clone(), Collection::Tag(n))
        });
        entries.push(EntryKind::SubMenu("Tags".into(), paged(list.collect())));
    }
    let searches: Vec<&inoink::store::TagInfo> = tags.iter().filter(|t| t.kind == "active_search").collect();
    if !searches.is_empty() {
        let list = searches.iter().map(|t| radio(t.id.rsplit('/').next().unwrap_or("Search").to_string(), Collection::Monitoring(StreamId::new(&t.id))));
        entries.push(EntryKind::SubMenu("Monitoring".into(), paged(list.collect())));
    }
    entries.push(EntryKind::Separator);
    entries.push(radio("Saved pages".into(), Collection::SavedPages));
    entries.push(radio("Annotated".into(), Collection::Annotated));
    entries.push(radio("Liked".into(), Collection::Liked));
    entries.push(radio("Broadcast".into(), Collection::Broadcast));
    entries
}

fn feed_menu(context: &Context, s: &inoink::store::Subscription, folders: &[String]) -> EntryKind {
    let mut move_entries = Vec::new();
    for f in folders {
        let inside = s.folders.contains(f);
        let entry = if inside {
            InoInkEntry::MoveFeed { feed: s.id.clone(), add: None, remove: Some(f.clone()) }
        } else {
            InoInkEntry::MoveFeed { feed: s.id.clone(), add: Some(f.clone()), remove: None }
        };
        move_entries.push(EntryKind::CheckBox(f.clone(), EntryId::InoInk(entry), inside));
    }
    move_entries.push(EntryKind::Separator);
    move_entries.push(cmd("New folder...", InoInkEntry::PromptNewFolderForFeed(s.id.clone())));
    let _ = context;
    EntryKind::SubMenu(
        s.title.clone(),
        vec![cmd("Rename...", InoInkEntry::PromptRenameFeed(s.id.clone())), EntryKind::SubMenu("Folders".into(), paged(move_entries)), cmd("Unfollow", InoInkEntry::Unsubscribe(s.id.clone()))],
    )
}

/// Management: subscriptions, folders, monitoring and offline downloads.
pub fn manage_entries(context: &Context) -> Vec<EntryKind> {
    let mut entries = vec![cmd("Follow a feed...", InoInkEntry::Subscribe)];
    if let Some(store) = context.inoink.store() {
        let subs = store.subscriptions().unwrap_or_default();
        let mut folders: Vec<String> = subs.iter().flat_map(|s| s.folders.clone()).collect();
        folders.sort();
        folders.dedup();
        if !subs.is_empty() {
            let list = subs.iter().map(|s| feed_menu(context, s, &folders)).collect();
            entries.push(EntryKind::SubMenu("Feeds".into(), paged(list)));
        }
        if !folders.is_empty() {
            let list = folders
                .iter()
                .map(|f| {
                    let pinned = context.settings.inoink.pinned_folders.contains(f);
                    EntryKind::SubMenu(
                        f.clone(),
                        vec![
                            cmd("Rename...", InoInkEntry::PromptRenameFolder(f.clone())),
                            EntryKind::CheckBox("Keep offline".into(), EntryId::InoInk(InoInkEntry::PinFolder(f.clone(), !pinned)), pinned),
                            cmd("Delete folder", InoInkEntry::DeleteFolder(f.clone())),
                        ],
                    )
                })
                .collect();
            entries.push(EntryKind::SubMenu("Folders".into(), paged(list)));
        }
        let searches: Vec<_> = store.tags().unwrap_or_default().into_iter().filter(|t| t.kind == "active_search").collect();
        let mut mon = vec![cmd("New monitoring search...", InoInkEntry::PromptNewMonitoring)];
        for t in searches {
            let name = t.id.rsplit('/').next().unwrap_or("search").to_string();
            mon.push(EntryKind::SubMenu(name, vec![cmd("Delete", InoInkEntry::DeleteMonitoring(t.id.clone()))]));
        }
        entries.push(EntryKind::SubMenu("Monitoring".into(), paged(mon)));
    }
    entries.push(EntryKind::Separator);
    let budget = context.settings.inoink.cache_budget_mb;
    entries.push(EntryKind::SubMenu(
        "Offline storage".into(),
        [256u64, 512, 1024, 2048].iter().map(|b| EntryKind::RadioButton(format!("{b} MB"), EntryId::InoInk(InoInkEntry::CacheBudget(*b)), budget == *b)).collect(),
    ));
    let days = context.settings.inoink.recent_unread_days;
    entries.push(EntryKind::SubMenu(
        "Keep unread for".into(),
        [7i64, 30, 90].iter().map(|d| EntryKind::RadioButton(format!("{d} days"), EntryId::InoInk(InoInkEntry::UnreadDays(*d)), days == *d)).collect(),
    ));
    entries
}

/// Applies a management entry that changes data. Returns a user-facing message on success.
pub fn apply(context: &mut Context, entry: &InoInkEntry, prompt_text: Option<&str>) -> Result<Option<String>, String> {
    let mut enqueue = |context: &mut Context, kind: &str, payload: serde_json::Value| -> Result<(), String> {
        let store = context.inoink.store_mut().ok_or(SAVE_FAILED)?;
        store.enqueue_org(kind, &payload.to_string()).map_err(|_| SAVE_FAILED.to_string())?;
        context.inoink.changed();
        Ok(())
    };
    match entry {
        InoInkEntry::Subscribe => {
            let feed = prompt_text.unwrap_or("").trim();
            if feed.is_empty() {
                return Err("Enter the address of a feed or site".into());
            }
            enqueue(context, "quickadd", json!({"feed": feed}))?;
            Ok(Some("Following. It appears after the next sync.".into()))
        }
        InoInkEntry::PromptRenameFeed(feed) => {
            let t = prompt_text.unwrap_or("").trim();
            if t.is_empty() {
                return Err("Enter a name".into());
            }
            enqueue(context, "subscription_edit", json!({"action": "edit", "stream": feed, "title": t}))?;
            Ok(None)
        }
        InoInkEntry::MoveFeed { feed, add, remove } => {
            enqueue(context, "subscription_edit", json!({"action": "edit", "stream": feed, "add_folder": add, "remove_folder": remove}))?;
            Ok(None)
        }
        InoInkEntry::PromptNewFolderForFeed(feed) => {
            let t = prompt_text.unwrap_or("").trim();
            if t.is_empty() || t.contains('/') {
                return Err("Enter a folder name".into());
            }
            enqueue(context, "subscription_edit", json!({"action": "edit", "stream": feed, "add_folder": t}))?;
            Ok(None)
        }
        InoInkEntry::ConfirmUnsubscribe(feed) => {
            enqueue(context, "subscription_edit", json!({"action": "unsubscribe", "stream": feed}))?;
            Ok(Some("Unfollowed. Its articles stay until removed.".into()))
        }
        InoInkEntry::PromptRenameFolder(folder) => {
            let t = prompt_text.unwrap_or("").trim();
            if t.is_empty() || t.contains('/') {
                return Err("Enter a folder name".into());
            }
            enqueue(context, "rename_tag", json!({"source": StreamId::label(folder).0, "dest": t}))?;
            Ok(None)
        }
        InoInkEntry::ConfirmDeleteFolder(folder) => {
            enqueue(context, "disable_tag", json!({"stream": StreamId::label(folder).0}))?;
            Ok(Some("Folder deleted. Its feeds are kept.".into()))
        }
        InoInkEntry::PinFolder(folder, pin) => {
            let pins = &mut context.settings.inoink.pinned_folders;
            pins.retain(|f| f != folder);
            if *pin {
                pins.push(folder.clone());
            }
            Ok(None)
        }
        InoInkEntry::PromptNewMonitoring => {
            let t = prompt_text.unwrap_or("").trim();
            if t.is_empty() {
                return Err("Enter words to watch for".into());
            }
            enqueue(context, "active_search_create", json!({"term": t, "filter_type": "public", "search_filters": "all", "search_match": "all_words"}))?;
            Ok(Some("Monitoring search requested.".into()))
        }
        InoInkEntry::ConfirmDeleteMonitoring(id) => {
            enqueue(context, "active_search_delete", json!({"id": id}))?;
            Ok(None)
        }
        InoInkEntry::CacheBudget(mb) => {
            context.settings.inoink.cache_budget_mb = *mb;
            Ok(Some(format!("Offline storage: {mb} MB (applies after InoInk restarts)")))
        }
        InoInkEntry::UnreadDays(d) => {
            context.settings.inoink.recent_unread_days = *d;
            Ok(None)
        }
        _ => Ok(None),
    }
}
