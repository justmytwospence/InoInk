//! Menus and operations for subscriptions, folders, tags, monitoring and offline downloads.
//! Everything ordinary stays out of the way: these live in secondary menus only.

use crate::context::Context;
use crate::device::CURRENT_DEVICE;
use crate::settings::{ButtonScheme, HeadlineTap, RotationLock};
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
    entries
}

/// The main menu: status and the everyday actions first, then Settings, Manage and Status.
pub fn main_menu_entries(context: &Context) -> Vec<EntryKind> {
    let mut entries = Vec::new();
    if let Some(e) = context.inoink.provision_error.clone() {
        entries.push(EntryKind::Message(format!("Provisioning failed: {e}"), None));
    }
    entries.push(EntryKind::Message(context.inoink.status_text(), None));
    entries.push(cmd("Sync now", InoInkEntry::SyncNow));
    entries.push(cmd("Mark this list read...", InoInkEntry::MarkListRead));
    entries.push(cmd("Follow a feed...", InoInkEntry::Subscribe));
    entries.push(EntryKind::Separator);
    entries.push(EntryKind::SubMenu("Settings".into(), settings_entries(context)));
    entries.push(EntryKind::SubMenu("Manage".into(), manage_entries(context)));
    entries.push(EntryKind::SubMenu("Status".into(), status_entries(context)));
    entries.push(EntryKind::Separator);
    entries.push(EntryKind::Command("About InoInk".into(), EntryId::About));
    let quit = if std::env::var("PLATFORM").as_deref() == Ok("mt8113t-ntx") { "Quit (the Kobo restarts)" } else { "Quit" };
    entries.push(EntryKind::Command(quit.into(), EntryId::Quit));
    entries
}

fn settings_entries(context: &Context) -> Vec<EntryKind> {
    let s = &context.settings.inoink;
    let radio = |label: &str, e: InoInkEntry, on: bool| EntryKind::RadioButton(label.to_string(), EntryId::InoInk(e), on);
    let mut entries = vec![
        EntryKind::SubMenu(
            "Headlines".into(),
            vec![
                radio("Tap opens, hold saves for later", InoInkEntry::SetHeadlineTap(HeadlineTap::Open), s.list_tap == HeadlineTap::Open),
                radio("Tap saves for later, hold opens", InoInkEntry::SetHeadlineTap(HeadlineTap::Star), s.list_tap == HeadlineTap::Star),
            ],
        ),
        EntryKind::SubMenu(
            "At the end of an article".into(),
            vec![radio("Swipe again to continue", InoInkEntry::SetContinueOnSwipe(true), s.continue_on_swipe), radio("Wait for a tap", InoInkEntry::SetContinueOnSwipe(false), !s.continue_on_swipe)],
        ),
    ];
    entries.push(EntryKind::SubMenu(
        "Offline storage".into(),
        [256u64, 512, 1024, 2048].iter().map(|b| EntryKind::RadioButton(format!("{b} MB"), EntryId::InoInk(InoInkEntry::CacheBudget(*b)), s.cache_budget_mb == *b)).collect(),
    ));
    entries.push(EntryKind::SubMenu(
        "Keep unread for".into(),
        [7i64, 30, 90].iter().map(|d| EntryKind::RadioButton(format!("{d} days"), EntryId::InoInk(InoInkEntry::UnreadDays(*d)), s.recent_unread_days == *d)).collect(),
    ));
    if CURRENT_DEVICE.has_page_turn_buttons() {
        let b = context.settings.button_scheme;
        entries.push(EntryKind::SubMenu(
            "Page-turn buttons".into(),
            vec![
                EntryKind::RadioButton(ButtonScheme::Natural.to_string(), EntryId::SetButtonScheme(ButtonScheme::Natural), b == ButtonScheme::Natural),
                EntryKind::RadioButton(ButtonScheme::Inverted.to_string(), EntryId::SetButtonScheme(ButtonScheme::Inverted), b == ButtonScheme::Inverted),
            ],
        ));
    }
    if CURRENT_DEVICE.has_gyroscope() {
        let lock = context.settings.rotation_lock;
        entries.push(EntryKind::SubMenu(
            "Rotation".into(),
            vec![
                EntryKind::RadioButton("Auto".into(), EntryId::SetRotationLock(None), lock.is_none()),
                EntryKind::Separator,
                EntryKind::RadioButton("Portrait".into(), EntryId::SetRotationLock(Some(RotationLock::Portrait)), lock == Some(RotationLock::Portrait)),
                EntryKind::RadioButton("Landscape".into(), EntryId::SetRotationLock(Some(RotationLock::Landscape)), lock == Some(RotationLock::Landscape)),
                EntryKind::RadioButton("Ignore".into(), EntryId::SetRotationLock(Some(RotationLock::Current)), lock == Some(RotationLock::Current)),
            ],
        ));
    }
    entries.push(EntryKind::CheckBox("Invert colors".into(), EntryId::ToggleInverted, context.fb.inverted()));
    entries
}

fn status_entries(context: &Context) -> Vec<EntryKind> {
    let mut entries = vec![
        EntryKind::Message(context.inoink.status_text(), None),
        EntryKind::Message(format!("Changes waiting to send: {}", context.inoink.pending()), None),
    ];
    if let Some(q) = context.inoink.store().and_then(|s| s.get_meta("quota").ok().flatten()) {
        let q = inoink::api::Quota::from_json(&q);
        let r1 = q.remaining(inoink::api::Zone::One);
        let r2 = q.remaining(inoink::api::Zone::Two);
        let est = if r1.estimated { " (estimated)" } else { "" };
        entries.push(EntryKind::Message(format!("Inoreader today: {} of {} reads, {} of {} writes left{est}", r1.remaining, r1.limit, r2.remaining, r2.limit), None));
    }
    let karakeep = if context.inoink.share.karakeep.is_none() {
        "Karakeep: not set up".to_string()
    } else {
        let (waiting, failed) = context.inoink.store().and_then(|s| s.share_counts().ok()).unwrap_or((0, 0));
        format!("Karakeep: {waiting} waiting, {failed} failed")
    };
    entries.push(EntryKind::Message(karakeep, None));
    entries.push(EntryKind::Message(format!("Version {}", inoink::VERSION), None));
    entries
}

/// Applies a management entry that changes data. Returns a user-facing message on success.
pub fn apply(context: &mut Context, entry: &InoInkEntry, prompt_text: Option<&str>) -> Result<Option<String>, String> {
    let enqueue = |context: &mut Context, kind: &str, payload: serde_json::Value| -> Result<(), String> {
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
