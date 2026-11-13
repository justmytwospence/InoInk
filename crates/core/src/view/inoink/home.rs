//! The article list: collection selector, unread/all filter, search and sync status.

use crate::color::{BLACK, SEPARATOR_NORMAL, WHITE};
use crate::context::Context;
use crate::device::CURRENT_DEVICE;
use crate::font::Fonts;
use crate::framebuffer::{Framebuffer, UpdateMode};
use crate::geom::{divide, halves, CycleDir, Dir, Rectangle};
use crate::gesture::GestureEvent;
use crate::input::{ButtonCode, ButtonStatus, DeviceEvent};
use crate::unit::scale_by_dpi;
use crate::view::common::{locate, locate_by_id};
use crate::view::dialog::Dialog;
use crate::view::filler::Filler;
use crate::view::icon::Icon;
use crate::view::button::Button;
use crate::view::inoink::actions::{apply_article_action, undo_done, ArticleAction, InoInkEntry};
use crate::view::inoink::categories::{category_items, CategoryItem, CategoryRow};
use crate::view::inoink::group_header::GroupHeader;
use crate::view::inoink::manage;
use crate::view::inoink::page_progress::PageProgress;
use crate::view::inoink::row::{ArticleRow, RowMode};
use crate::view::inoink::state::{ReadingQueue, Tab};
use crate::view::inoink::tab_bar::TabBar;
use crate::settings::HeadlineTap;
use std::collections::BTreeSet;
use crate::view::keyboard::Keyboard;
use crate::view::label::Label;
use crate::view::menu::{Menu, MenuKind};
use crate::view::named_input::NamedInput;
use crate::view::notification::Notification;
use crate::view::top_bar::TopBar;
use crate::view::{Align, Bus, EntryId, EntryKind, Event, Hub, Id, RenderData, RenderQueue, View, ViewId, ID_FEEDER};
use crate::view::{BIG_BAR_HEIGHT, SMALL_BAR_HEIGHT, THICKNESS_MEDIUM};
use inoink::model::*;

/// One slot of the list page: a half-height header or a full-height article.
#[derive(Debug, Clone)]
enum Slot {
    Header(String),
    Item(usize),
    /// A category (an index into `cat_items`).
    Cat(usize),
    /// The "Mark N read" card at the end of an unread list.
    End,
}

#[derive(Debug, Clone)]
enum Prompt {
    Search,
    Entry(InoInkEntry),
}

pub struct InoInkHome {
    id: Id,
    rect: Rectangle,
    children: Vec<Box<dyn View>>,
    items: Vec<ArticleSummary>,
    /// Cursor that loads each page; page 0 starts at the top.
    cursors: Vec<Option<Cursor>>,
    page: usize,
    next: Option<Cursor>,
    layout: Vec<Slot>,
    search: Option<String>,
    prompt: Option<Prompt>,
    focus: Option<ViewId>,
    rows: usize,
    /// A frozen selection awaiting confirmation (count shown to the user).
    selection: Option<BulkSelection>,
    /// The article a row menu acts on.
    pending_row: Option<ArticleId>,
    cat_items: Vec<CategoryItem>,
    cat_pages: Vec<usize>,
    cat_page: usize,
    showing_cats: bool,
    row_mode: RowMode,
    /// Every article shown in this list since it was opened (for "Mark N read").
    seen: BTreeSet<ArticleId>,
    /// The page that only holds the end-of-list card.
    end_only: bool,
    /// The last page of articles had no room for the card: Next shows it.
    next_is_end: bool,
    end_count: usize,
    /// A spin tick is scheduled.
    /// Pages the current list takes (at least 1).
    total_pages: usize,
    /// More may be loaded from Inoreader after the last stored page.
    more_online: bool,
}

const TOP: usize = 0;
const TABS: usize = 2;
const LIST: usize = 4;
const FOOTER: usize = 6;

struct List {
    id: Id,
    rect: Rectangle,
    children: Vec<Box<dyn View>>,
}

impl View for List {
    fn handle_event(&mut self, evt: &Event, _hub: &Hub, bus: &mut Bus, _rq: &mut RenderQueue, _context: &mut Context) -> bool {
        if let Event::Gesture(GestureEvent::Swipe { dir, start, .. }) = *evt {
            if self.rect.includes(start) {
                match dir {
                    Dir::West => {
                        bus.push_back(Event::Page(CycleDir::Next));
                        return true;
                    }
                    Dir::East => {
                        bus.push_back(Event::Page(CycleDir::Previous));
                        return true;
                    }
                    _ => {}
                }
            }
        }
        false
    }
    fn render(&self, _fb: &mut dyn Framebuffer, _rect: Rectangle, _fonts: &mut Fonts) {}
    fn rect(&self) -> &Rectangle {
        &self.rect
    }
    fn rect_mut(&mut self) -> &mut Rectangle {
        &mut self.rect
    }
    fn children(&self) -> &Vec<Box<dyn View>> {
        &self.children
    }
    fn children_mut(&mut self) -> &mut Vec<Box<dyn View>> {
        &mut self.children
    }
    fn id(&self) -> Id {
        self.id
    }
}

struct Footer {
    id: Id,
    rect: Rectangle,
    children: Vec<Box<dyn View>>,
}

impl View for Footer {
    fn handle_event(&mut self, _evt: &Event, _hub: &Hub, _bus: &mut Bus, _rq: &mut RenderQueue, _context: &mut Context) -> bool {
        false
    }
    fn render(&self, _fb: &mut dyn Framebuffer, _rect: Rectangle, _fonts: &mut Fonts) {}
    fn rect(&self) -> &Rectangle {
        &self.rect
    }
    fn rect_mut(&mut self) -> &mut Rectangle {
        &mut self.rect
    }
    fn children(&self) -> &Vec<Box<dyn View>> {
        &self.children
    }
    fn children_mut(&mut self) -> &mut Vec<Box<dyn View>> {
        &mut self.children
    }
    fn id(&self) -> Id {
        self.id
    }
}

pub fn collection_title(context: &Context, c: &Collection) -> String {
    match c {
        Collection::Starred => "Read later".into(),
        Collection::Unread => "Unread".into(),
        Collection::All => "Newsfeed".into(),
        Collection::Feed(s) => context.inoink.store().and_then(|st| st.subscription(s.as_str()).ok().flatten()).map(|s| s.title).unwrap_or_else(|| "Feed".into()),
        Collection::Folder(n) => n.clone(),
        Collection::Tag(n) => format!("Tag: {n}"),
        Collection::SavedPages => "Saved pages".into(),
        Collection::Annotated => "Annotated".into(),
        Collection::Liked => "Liked".into(),
        Collection::Broadcast => "Broadcast".into(),
        Collection::Monitoring(s) => s.as_str().rsplit('/').next().unwrap_or("Monitoring").to_string(),
    }
}

impl InoInkHome {
    pub fn new(rect: Rectangle, hub: &Hub, rq: &mut RenderQueue, context: &mut Context) -> InoInkHome {
        let id = ID_FEEDER.next();
        let dpi = CURRENT_DEVICE.dpi;
        let (small_height, big_height) = (scale_by_dpi(SMALL_BAR_HEIGHT, dpi) as i32, scale_by_dpi(BIG_BAR_HEIGHT, dpi) as i32);
        let thickness = scale_by_dpi(THICKNESS_MEDIUM, dpi) as i32;
        let (small_thickness, big_thickness) = halves(thickness);
        let mut children: Vec<Box<dyn View>> = Vec::new();

        let title = String::new();
        let top = TopBar::new(rect![rect.min.x, rect.min.y, rect.max.x, rect.min.y + small_height - small_thickness], Event::Toggle(ViewId::SearchBar), title, context)
            .with_home_layout(vec![
                ("mark-read", Event::Select(EntryId::InoInk(InoInkEntry::MarkListRead))),
                ("sync", Event::Select(EntryId::InoInk(InoInkEntry::SyncNow))),
            ]);
        children.push(Box::new(top));
        children.push(Box::new(Filler::new(rect![rect.min.x, rect.min.y + small_height - small_thickness, rect.max.x, rect.min.y + small_height + big_thickness], BLACK)));

        let tabs_rect = rect![rect.min.x, rect.min.y + small_height + big_thickness, rect.max.x, rect.min.y + 2 * small_height - small_thickness];
        children.push(Box::new(TabBar::new(tabs_rect, context.inoink.tab, [String::new(), String::new(), String::new()])));
        children.push(Box::new(Filler::new(rect![rect.min.x, tabs_rect.max.y, rect.max.x, rect.min.y + 2 * small_height + big_thickness], BLACK)));

        let list_rect = rect![rect.min.x, rect.min.y + 2 * small_height + big_thickness, rect.max.x, rect.max.y - small_height - small_thickness];
        let rows = (((list_rect.height() as i32) + thickness) / big_height).max(1) as usize;
        children.push(Box::new(List { id: ID_FEEDER.next(), rect: list_rect, children: Vec::new() }));
        children.push(Box::new(Filler::new(rect![rect.min.x, rect.max.y - small_height - small_thickness, rect.max.x, rect.max.y - small_height + big_thickness], BLACK)));
        let footer_rect = rect![rect.min.x, rect.max.y - small_height + big_thickness, rect.max.x, rect.max.y];
        children.push(Box::new(Footer { id: ID_FEEDER.next(), rect: footer_rect, children: Vec::new() }));

        let mut home = InoInkHome {
            id,
            rect,
            children,
            items: Vec::new(),
            cursors: vec![None],
            page: 0,
            next: None,
            layout: Vec::new(),
            search: None,
            prompt: None,
            focus: None,
            rows,
            selection: None,
            pending_row: None,
            cat_items: Vec::new(),
            cat_pages: vec![0],
            cat_page: 0,
            showing_cats: false,
            row_mode: RowMode::OpenMenu,
            seen: BTreeSet::new(),
            end_only: false,
            next_is_end: false,
            end_count: 0,
            total_pages: 1,
            more_online: false,
        };
        home.load_page(hub, rq, context);
        home.update_sync_icon(rq, context);
        rq.add(RenderData::new(home.id, rect, UpdateMode::Full));
        home
    }

    /// The list of categories is showing (Categories tab, nothing opened, no search).
    fn showing_categories(&self, context: &Context) -> bool {
        self.search.is_none() && context.inoink.tab == Tab::Categories && context.inoink.show_categories
    }

    /// How rows answer taps and holds. In Read later and search results a tap opens and a hold
    /// shows the menu; elsewhere one of them saves for later (the setting says which).
    fn row_mode(&self, context: &Context) -> RowMode {
        if self.search.is_some() || context.inoink.tab == Tab::ReadLater {
            RowMode::OpenMenu
        } else if context.settings.inoink.list_tap == HeadlineTap::Star {
            RowMode::SaveOpen
        } else {
            RowMode::OpenSave
        }
    }

    fn query(&self, context: &Context, after: Option<Cursor>, limit: usize) -> ArticleQuery {
        if let Some(text) = &self.search {
            return ArticleQuery { collection: None, unread_only: false, search: Some(text.clone()), limit, after, group: ArticleGroup::None, sort: ArticleSort::Newest };
        }
        let o = Self::effective_options(context);
        ArticleQuery { collection: Some(context.inoink.collection.clone()), unread_only: o.unread_only, search: None, limit, after, group: o.group, sort: o.sort }
    }

    /// The filter, group and sort actually applied to the current list.
    fn effective_options(context: &Context) -> ViewOptions {
        context.inoink.view_options().effective(&context.inoink.collection)
    }

    /// An unread list is finished with a "Mark N read" card.
    fn end_card_applies(&self, context: &Context) -> bool {
        self.search.is_none() && Self::effective_options(context).unread_only && !matches!(context.inoink.collection, Collection::Starred)
    }

    /// Reloads the current page from the local database. Never touches the network.
    pub fn load_page(&mut self, hub: &Hub, rq: &mut RenderQueue, context: &mut Context) {
        self.showing_cats = self.showing_categories(context);
        self.row_mode = self.row_mode(context);
        if self.showing_cats {
            self.load_categories(rq, context);
        } else if !self.load_articles(hub, rq, context) {
            return;
        }
        self.update_chrome(rq, context);
        if let Some(notice) = context.inoink.notice.take() {
            let n = Notification::new(notice, hub, rq, context);
            self.children.push(Box::new(n));
        }
    }

    fn load_categories(&mut self, rq: &mut RenderQueue, context: &mut Context) {
        self.cat_items = category_items(context);
        let units = 2 * self.rows;
        let mut pages = vec![0usize];
        let mut used = 0;
        for (i, it) in self.cat_items.iter().enumerate() {
            // A header needs room for at least one entry under it.
            let (cost, need) = match it {
                CategoryItem::Header(_) => (1, 3),
                CategoryItem::Entry { .. } => (2, 2),
            };
            if used + need > units && used > 0 {
                pages.push(i);
                used = 0;
            }
            used += cost;
        }
        self.cat_pages = pages;
        self.cat_page = self.cat_page.min(self.cat_pages.len() - 1);
        self.total_pages = self.cat_pages.len().max(1);
        self.more_online = false;
        let start = self.cat_pages[self.cat_page];
        let end = self.cat_pages.get(self.cat_page + 1).copied().unwrap_or(self.cat_items.len());
        self.layout = (start..end)
            .map(|i| match &self.cat_items[i] {
                CategoryItem::Header(t) => Slot::Header(t.clone()),
                CategoryItem::Entry { .. } => Slot::Cat(i),
            })
            .collect();
        self.items.clear();
        self.next = None;
        self.next_is_end = false;
        self.rebuild_list(rq, context);
        self.rebuild_footer(rq, context);
    }

    /// Loads the article page. Returns false when it reloaded itself (a page past the end).
    fn load_articles(&mut self, hub: &Hub, rq: &mut RenderQueue, context: &mut Context) -> bool {
        self.next_is_end = false;
        let opts = Self::effective_options(context);
        let applies = self.end_card_applies(context);
        let units = 2 * self.rows - if context.inoink.last_undo.is_some() { 2 } else { 0 };
        if self.end_only {
            self.end_count = self.count_unmarked(context);
            if self.end_count == 0 || !applies {
                self.end_only = false;
                if self.page > 0 {
                    self.page -= 1;
                    self.cursors.truncate(self.page + 1);
                }
                self.load_page(hub, rq, context);
                return false;
            }
            self.items.clear();
            self.next = None;
            self.layout = vec![Slot::End];
            self.set_page_totals(self.page + 1, context);
            self.rebuild_list(rq, context);
            self.rebuild_footer(rq, context);
            return true;
        }
        let after = self.cursors.get(self.page).cloned().flatten();
        let q = self.query(context, after, self.rows);
        let page = context.inoink.store().and_then(|s| s.query_articles(&q).ok());
        let (items, next) = match page {
            Some(p) => (p.items, p.next),
            None => (Vec::new(), None),
        };
        // A page past the end (items were removed): step back.
        if items.is_empty() && self.page > 0 {
            self.page -= 1;
            self.cursors.truncate(self.page + 1);
            self.load_page(hub, rq, context);
            return false;
        }
        // Plan the page: headers cost one unit, articles two; a page has two units per row.
        let mut layout = Vec::new();
        let mut used = 0;
        let mut placed = 0;
        let mut last_header: Option<String> = None;
        for (i, item) in items.iter().enumerate() {
            let h = opts.group.header(item).filter(|_| self.search.is_none());
            let needs_header = h.is_some() && h != last_header;
            let cost = 2 + needs_header as usize;
            if used + cost > units && placed > 0 {
                break;
            }
            if needs_header {
                layout.push(Slot::Header(h.clone().unwrap_or_default()));
            }
            layout.push(Slot::Item(i));
            last_header = h;
            used += cost;
            placed += 1;
        }
        let truncated = placed < items.len();
        let mut items = items;
        items.truncate(placed);
        let next = if truncated { items.last().map(Cursor::from) } else { next };
        if self.search.is_none() {
            self.seen.extend(items.iter().map(|i| i.id));
        }
        if next.is_none() && applies {
            self.end_count = self.count_unmarked(context);
            if self.end_count > 0 {
                if used + 2 <= units {
                    layout.push(Slot::End);
                } else {
                    self.next_is_end = true;
                }
            }
        }
        self.layout = layout;
        self.items = items;
        self.next = next;
        let last = self.next.is_none() && !self.next_is_end;
        let total = if last { self.page + 1 } else { self.count_article_pages(context) };
        self.set_page_totals(total, context);
        self.rebuild_list(rq, context);
        self.rebuild_footer(rq, context);
        true
    }

    fn set_page_totals(&mut self, total: usize, context: &Context) {
        self.total_pages = total.max(self.page + 1);
        self.more_online = self.search.is_none() && context.inoink.handle.is_some() && self.can_browse(context);
    }

    /// How many pages the whole current list takes, laid out as `load_articles` lays out one
    /// page: a header costs one unit, an article two, a page has `2 * rows` units (two fewer
    /// while the undo bar shows) and holds at most `rows` articles; an unread list ends with a
    /// card that needs two units. Keep this in step with `load_articles`.
    fn count_article_pages(&self, context: &Context) -> usize {
        let Some(store) = context.inoink.store() else { return 1 };
        let opts = Self::effective_options(context);
        let units = (2 * self.rows).saturating_sub(if context.inoink.last_undo.is_some() { 2 } else { 0 });
        let q = ArticleQuery { limit: 0, after: None, ..self.query(context, None, 0) };
        let n = store.count_articles(&q).unwrap_or(0);
        let mut headers: Vec<Option<String>> = Vec::new();
        if !(self.search.is_some() || opts.group == ArticleGroup::None) {
            let mut scan = ArticleQuery { limit: 500, ..q.clone() };
            while headers.len() < PAGE_SCAN_LIMIT {
                let Ok(page) = store.query_articles(&scan) else { break };
                headers.extend(page.items.iter().map(|i| opts.group.header(i)));
                match page.next {
                    Some(c) => scan.after = Some(c),
                    None => break,
                }
            }
        }
        headers.resize(n.max(headers.len()), None);
        let (mut pages, mut i, mut used_last) = (0usize, 0usize, 0usize);
        while i < headers.len() {
            pages += 1;
            let (mut used, mut placed) = (0usize, 0usize);
            let mut last: Option<String> = None;
            while i < headers.len() && placed < self.rows {
                let h = &headers[i];
                let needs = h.is_some() && *h != last;
                let cost = 2 + needs as usize;
                if used + cost > units && placed > 0 {
                    break;
                }
                used += cost;
                placed += 1;
                last = h.clone();
                i += 1;
            }
            used_last = used;
        }
        if pages > 0 && self.end_card_applies(context) && used_last + 2 > units {
            pages += 1;
        }
        pages.max(1)
    }

    /// (page shown from 1, pages in the list, more can be loaded online).
    pub fn page_status(&self) -> (usize, usize, bool) {
        let page = if self.showing_cats { self.cat_page + 1 } else { self.page + 1 };
        (page, self.total_pages, self.more_online)
    }

    /// How many articles seen in this list are still unread and not saved.
    fn count_unmarked(&self, context: &Context) -> usize {
        self.unmarked(context).len()
    }

    fn unmarked(&self, context: &Context) -> Vec<ArticleId> {
        let Some(store) = context.inoink.store() else { return Vec::new() };
        self.seen
            .iter()
            .filter(|id| !store.effective(**id, &Membership::Read).unwrap_or(true) && !store.effective(**id, &Membership::Starred).unwrap_or(true))
            .copied()
            .collect()
    }

    /// Title, back/search icon and tab labels.
    fn update_chrome(&mut self, rq: &mut RenderQueue, context: &Context) {
        let in_category = context.inoink.tab == Tab::Categories && !context.inoink.show_categories;
        // The tabs already name the Newsfeed, Categories and Read later: the title only says
        // where you are inside Categories, or what you searched for.
        let title = if let Some(q) = &self.search {
            format!("Search: {q}")
        } else if in_category {
            format!("{} \u{25be}", collection_title(context, &context.inoink.collection))
        } else {
            String::new()
        };
        let icon = if self.search.is_some() || in_category { "back" } else { "search" };
        if let Some(top) = self.children[TOP].downcast_mut::<TopBar>() {
            top.update_title_label(&title, rq);
            top.update_root_icon(icon, rq);
        }
        let count = |c: Collection, unread: bool| {
            context.inoink.store().and_then(|s| s.count_articles(&ArticleQuery { collection: Some(c), unread_only: unread, ..Default::default() }).ok()).unwrap_or(0)
        };
        let label = |name: &str, n: usize| if n > 0 { format!("{name} ({n})") } else { name.to_string() };
        let labels = [label("Newsfeed", count(Collection::All, true)), "Categories".to_string(), label("Read later", count(Collection::Starred, false))];
        if let Some(tabs) = self.children[TABS].downcast_mut::<TabBar>() {
            tabs.update(context.inoink.tab, labels, rq);
        }
    }

    fn empty_message(&self, context: &Context) -> (String, String) {
        use inoink::runtime::SyncStatus::*;
        if self.search.is_some() {
            return ("No matches".into(), "Search covers articles downloaded to this device.".into());
        }
        match (&context.inoink.status, context.inoink.handle.is_some()) {
            (NeedsSignIn, _) | (_, false) if context.inoink.account == "local" && context.inoink.handle.is_none() && context.inoink.provision_error.is_some() || matches!(context.inoink.status, NeedsSignIn) => {
                ("Sign in required".into(), "Connect the Kobo to a computer and run inoink-cli authorize.".into())
            }
            (Syncing, _) => ("Syncing".into(), "Articles appear as they arrive.".into()),
            _ => {
                if self.showing_cats {
                    return ("No folders yet".into(), "Folders appear after the first sync.".into());
                }
                match context.inoink.collection {
                    Collection::Starred => ("Nothing in Read later".into(), "Tap a headline in the Newsfeed to save it here.".into()),
                    _ if Self::effective_options(context).unread_only => ("All caught up".into(), String::new()),
                    _ => ("Nothing here".into(), String::new()),
                }
            }
        }
    }

    fn rebuild_list(&mut self, rq: &mut RenderQueue, context: &Context) {
        let dpi = CURRENT_DEVICE.dpi;
        let big_height = scale_by_dpi(BIG_BAR_HEIGHT, dpi) as i32;
        let thickness = scale_by_dpi(THICKNESS_MEDIUM, dpi) as i32;
        let (small_thickness, big_thickness) = halves(thickness);
        let empty = self.empty_message(context);
        let rows = self.rows;
        let items = self.items.clone();
        let cats = self.cat_items.clone();
        let row_mode = self.row_mode;
        let end_count = self.end_count;
        let layout = self.layout.clone();
        let list = self.children[LIST].downcast_mut::<List>().unwrap();
        list.children.clear();
        let rect = list.rect;
        let total_units = 2 * rows as i32;
        let unit_h = divide(rect.height() as i32, total_units);
        let span = |from: i32, n: i32| -> i32 { unit_h[from as usize..(from + n) as usize].iter().sum() };
        let mut y = rect.min.y;
        let mut unit = 0i32;
        if !self.showing_cats {
            if let Some((_, title)) = context.inoink.last_undo.as_ref() {
                // The undo bar takes the first row until superseded.
                let h = span(0, 2);
                let r = rect![rect.min.x, y, rect.max.x, y + h - small_thickness];
                let label = format!("Done: {}", inoink::util::shorten(title, 40));
                let lbl = Label::new(rect![r.min.x, r.min.y, r.max.x - r.height() as i32 * 2, r.max.y], label, Align::Left(r.height() as i32 / 3));
                list.children.push(Box::new(lbl));
                list.children.push(Box::new(Label::new(rect![r.max.x - r.height() as i32 * 2, r.min.y, r.max.x, r.max.y], "Undo".to_string(), Align::Center).event(Some(Event::Select(EntryId::InoInk(InoInkEntry::UndoDone))))));
                list.children.push(Box::new(Filler::new(rect![rect.min.x, r.max.y, rect.max.x, r.max.y + thickness], SEPARATOR_NORMAL)));
                y += h;
                unit = 2;
            }
        }
        for slot in &layout {
            let n = match slot {
                Slot::Header(_) => 1,
                Slot::Item(_) | Slot::Cat(_) | Slot::End => 2,
            };
            if unit + n > total_units {
                break;
            }
            let h = span(unit, n);
            let y_min = y + if y > rect.min.y { big_thickness } else { 0 };
            let last = unit + n >= total_units;
            let y_max = y + h - if last { 0 } else { small_thickness };
            match slot {
                Slot::Header(text) => {
                    list.children.push(Box::new(GroupHeader::new(rect![rect.min.x, y_min, rect.max.x, y_max + if last { 0 } else { small_thickness }], text.clone())));
                }
                Slot::Item(i) => {
                    list.children.push(Box::new(ArticleRow::new(rect![rect.min.x, y_min, rect.max.x, y_max], items[*i].clone(), *i, row_mode)));
                    if !last {
                        list.children.push(Box::new(Filler::new(rect![rect.min.x, y_max, rect.max.x, y_max + thickness], SEPARATOR_NORMAL)));
                    }
                }
                Slot::Cat(i) => {
                    if let CategoryItem::Entry { label, count, target } = &cats[*i] {
                        list.children.push(Box::new(CategoryRow::new(rect![rect.min.x, y_min, rect.max.x, y_max], label.clone(), *count, target.clone())));
                        if !last {
                            list.children.push(Box::new(Filler::new(rect![rect.min.x, y_max, rect.max.x, y_max + thickness], SEPARATOR_NORMAL)));
                        }
                    }
                }
                Slot::End => {
                    let r = rect![rect.min.x, y_min, rect.max.x, y_max];
                    list.children.push(Box::new(Filler::new(r, WHITE)));
                    let half = r.height() as i32 / 2;
                    list.children.push(Box::new(Label::new(rect![r.min.x, r.min.y, r.max.x, r.min.y + half], "End of unread articles".to_string(), Align::Center)));
                    let w = r.width() as i32 / 3;
                    let cx = (r.min.x + r.max.x) / 2;
                    let pad = half / 6;
                    list.children.push(Box::new(Button::new(rect![cx - w, r.min.y + half + pad, cx + w, r.max.y - pad], Event::Select(EntryId::InoInk(InoInkEntry::MarkSeenRead)), format!("Mark {end_count} read"))));
                }
            }
            y += h;
            unit += n;
        }
        if layout.is_empty() {
            let cy = rect.min.y + rect.height() as i32 / 3;
            let h = big_height;
            list.children.push(Box::new(Filler::new(rect![rect.min.x, y, rect.max.x, rect.max.y], WHITE)));
            list.children.push(Box::new(Label::new(rect![rect.min.x, cy, rect.max.x, cy + h], empty.0, Align::Center)));
            if !empty.1.is_empty() {
                list.children.push(Box::new(Label::new(rect![rect.min.x, cy + h, rect.max.x, cy + 2 * h], empty.1, Align::Center)));
            }
        } else if y < rect.max.y {
            list.children.push(Box::new(Filler::new(rect![rect.min.x, y, rect.max.x, rect.max.y], WHITE)));
        }
        rq.add(RenderData::new(list.id, list.rect, UpdateMode::Partial));
    }

    fn rebuild_footer(&mut self, rq: &mut RenderQueue, context: &Context) {
        let (prev, next, page) = if self.showing_cats {
            (self.cat_page > 0, self.cat_page + 1 < self.cat_pages.len(), self.cat_page + 1)
        } else {
            (self.page > 0, self.next.is_some() || self.next_is_end, self.page + 1)
        };
        let (total, plus) = (self.total_pages, if self.more_online { "+" } else { "" });
        let opts = Self::effective_options(context);
        let view_text = if self.search.is_some() {
            String::new()
        } else if self.showing_cats {
            format!("{} \u{25be}", context.inoink.category_sort().short())
        } else {
            format!("{} \u{b7} {} \u{25be}", if opts.unread_only { "Unread" } else { "All" }, opts.sort.short())
        };
        let footer = self.children[FOOTER].downcast_mut::<Footer>().unwrap();
        let rect = footer.rect;
        let side = rect.height() as i32;
        footer.children.clear();
        let prev_rect = rect![rect.min, rect.min + side];
        if prev {
            footer.children.push(Box::new(Icon::new("arrow-left", prev_rect, Event::Page(CycleDir::Previous))));
        } else {
            footer.children.push(Box::new(Filler::new(prev_rect, WHITE)));
        }
        let next_rect = rect![rect.max - side, rect.max];
        if next {
            footer.children.push(Box::new(Icon::new("arrow-right", next_rect, Event::Page(CycleDir::Next))));
        } else {
            footer.children.push(Box::new(Filler::new(next_rect, WHITE)));
        }
        // Between the arrows: the page bar with its count above the view choice. With one page
        // there is nothing to count, and the view choice takes the whole height.
        let dpi = CURRENT_DEVICE.dpi;
        let pad = scale_by_dpi(12.0, dpi) as i32;
        let x0 = rect.min.x + side;
        let x1 = rect.max.x - side;
        let mid = rect.min.y + rect.height() as i32 / 2;
        let view_rect = if total > 1 { rect![x0, mid, x1, rect.max.y] } else { rect![x0, rect.min.y, x1, rect.max.y] };
        if total > 1 {
            let count_w = scale_by_dpi(150.0, dpi) as i32;
            footer.children.push(Box::new(PageProgress::new(rect![x0 + pad, rect.min.y, x1 - count_w, mid], page - 1, total)));
            footer.children.push(Box::new(Label::new(rect![x1 - count_w, rect.min.y, x1, mid], format!("{page} / {total}{plus}"), Align::Center)));
        }
        if view_text.is_empty() {
            footer.children.push(Box::new(Filler::new(view_rect, WHITE)));
        } else {
            footer.children.push(Box::new(Label::new(view_rect, view_text, Align::Center).event(Some(Event::ToggleNear(ViewId::SortMenu, view_rect)))));
        }
        rq.add(RenderData::new(footer.id, footer.rect, UpdateMode::Gui));
    }

    fn go(&mut self, dir: CycleDir, hub: &Hub, rq: &mut RenderQueue, context: &mut Context) {
        if self.showing_cats {
            match dir {
                CycleDir::Next if self.cat_page + 1 < self.cat_pages.len() => self.cat_page += 1,
                CycleDir::Previous if self.cat_page > 0 => self.cat_page -= 1,
                _ => return,
            }
            self.load_page(hub, rq, context);
            return;
        }
        match dir {
            CycleDir::Next => {
                if let Some(c) = self.next.clone() {
                    self.page += 1;
                    self.cursors.truncate(self.page);
                    self.cursors.push(Some(c));
                    self.load_page(hub, rq, context);
                } else if self.next_is_end {
                    self.end_only = true;
                    self.page += 1;
                    self.cursors.truncate(self.page);
                    self.cursors.push(None);
                    self.load_page(hub, rq, context);
                } else if !self.end_only && self.search.is_none() && context.inoink.handle.is_some() && self.can_browse(context) {
                    // End of what is stored: browse further back through history online.
                    context.inoink.submit(inoink::runtime::Command::Browse { collection: context.inoink.collection.clone(), continuation: context.inoink.store().and_then(|s| s.get_meta(&browse_key(&context.inoink.collection)).ok().flatten()) });
                    let n = Notification::new("Loading older articles".to_string(), hub, rq, context);
                    self.children.push(Box::new(n));
                }
            }
            CycleDir::Previous => {
                if self.end_only {
                    self.end_only = false;
                }
                if self.page > 0 {
                    self.page -= 1;
                    self.cursors.truncate(self.page + 1);
                    self.load_page(hub, rq, context);
                }
            }
        }
    }

    /// Browsing older history online only makes sense for the plain "everything, newest
    /// first" views.
    fn can_browse(&self, context: &Context) -> bool {
        let o = Self::effective_options(context);
        matches!(context.inoink.collection, Collection::All | Collection::Feed(_) | Collection::Folder(_) | Collection::Tag(_) | Collection::SavedPages) && !o.unread_only && o.group == ArticleGroup::None && o.sort == ArticleSort::Newest
    }

    fn reset(&mut self, hub: &Hub, rq: &mut RenderQueue, context: &mut Context) {
        self.page = 0;
        self.cursors = vec![None];
        self.cat_page = 0;
        self.seen.clear();
        self.end_only = false;
        self.next_is_end = false;
        self.load_page(hub, rq, context);
    }

    /// While a sync runs, the sync button shows its "syncing" form (with a dot); it is drawn once
    /// per change and never animated, so e-ink leaves no ghost.
    fn update_sync_icon(&mut self, rq: &mut RenderQueue, context: &Context) {
        let syncing = matches!(context.inoink.status, inoink::runtime::SyncStatus::Syncing);
        let name = if syncing { "sync-0" } else { "sync" };
        let Some(top) = self.children[TOP].downcast_mut::<TopBar>() else { return };
        for icon in top.children_mut().iter_mut().filter_map(|c| c.downcast_mut::<Icon>()) {
            if (icon.name == "sync" || icon.name == "sync-0") && icon.name != name {
                icon.name = name.to_string();
                rq.add(RenderData::new(icon.id(), icon.rect, UpdateMode::Gui));
            }
        }
    }

    fn notify(&mut self, text: &str, hub: &Hub, rq: &mut RenderQueue, context: &mut Context) {
        let n = Notification::new(text.to_string(), hub, rq, context);
        self.children.push(Box::new(n));
    }

    /// The articles on screen, in order.
    pub fn visible_articles(&self) -> Vec<ArticleId> {
        self.items.iter().map(|i| i.id).collect()
    }

    /// The headers and categories on screen, in order.
    pub fn category_labels(&self) -> Vec<String> {
        self.layout
            .iter()
            .filter_map(|s| match s {
                Slot::Header(t) => Some(t.clone()),
                Slot::Cat(i) => match &self.cat_items[*i] {
                    CategoryItem::Entry { label, .. } => Some(label.clone()),
                    _ => None,
                },
                _ => None,
            })
            .collect()
    }

    // ---- menus ------------------------------------------------------------------------------

    fn toggle_menu(&mut self, id: ViewId, rect: Rectangle, kind: MenuKind, entries: Vec<EntryKind>, rq: &mut RenderQueue, context: &mut Context) {
        if let Some(i) = locate_by_id(self, id) {
            rq.add(RenderData::expose(*self.child(i).rect(), UpdateMode::Gui));
            self.children.remove(i);
            return;
        }
        if entries.is_empty() {
            return;
        }
        let menu = Menu::new(rect, id, kind, entries, context);
        rq.add(RenderData::new(menu.id(), *menu.rect(), UpdateMode::Gui));
        self.children.push(Box::new(menu));
    }

    /// The title menu: inside a folder or feed, switch between the folder and its feeds.
    fn title_menu_entries(&self, context: &Context) -> Vec<EntryKind> {
        if self.search.is_some() || self.showing_cats {
            return Vec::new();
        }
        let Some(store) = context.inoink.store() else { return Vec::new() };
        let subs = store.subscriptions().unwrap_or_default();
        let folder = match &context.inoink.collection {
            Collection::Folder(f) => f.clone(),
            Collection::Feed(s) => match subs.iter().find(|x| x.id == s.0).and_then(|x| x.folders.first().cloned()) {
                Some(f) => f,
                None => return Vec::new(),
            },
            _ => return Vec::new(),
        };
        let current = &context.inoink.collection;
        let mut entries = vec![EntryKind::RadioButton(format!("All in {folder}"), EntryId::InoInk(InoInkEntry::OpenCategory(Collection::Folder(folder.clone()))), *current == Collection::Folder(folder.clone()))];
        for s in subs.iter().filter(|s| s.folders.contains(&folder)) {
            let c = Collection::Feed(StreamId::new(&s.id));
            entries.push(EntryKind::RadioButton(s.title.clone(), EntryId::InoInk(InoInkEntry::OpenCategory(c.clone())), *current == c));
        }
        manage::paged(entries)
    }

    fn close_menus(&mut self, rq: &mut RenderQueue) {
        for id in [ViewId::TitleMenu, ViewId::MainMenu, ViewId::BookMenu, ViewId::ManageMenu, ViewId::SortMenu] {
            if let Some(i) = locate_by_id(self, id) {
                rq.add(RenderData::expose(*self.child(i).rect(), UpdateMode::Gui));
                self.children.remove(i);
            }
        }
    }

    fn row_menu(&mut self, rect: Rectangle, index: usize, rq: &mut RenderQueue, context: &mut Context) {
        let Some(item) = self.items.get(index).cloned() else { return };
        let e = |a: ArticleAction| EntryId::InoInk(InoInkEntry::Article(a));
        self.pending_row = Some(item.id);
        let entries = vec![
            EntryKind::Command(if item.starred { "Remove from Read later" } else { "Save for later" }.to_string(), e(ArticleAction::ToggleStar)),
            EntryKind::Command(if item.read { "Mark unread" } else { "Mark read" }.to_string(), e(ArticleAction::ToggleRead)),
            EntryKind::Command("Done".to_string(), e(ArticleAction::Done)),
            EntryKind::Command("Details".to_string(), EntryId::InoInk(InoInkEntry::OpenDetails(item.id))),
        ];
        self.toggle_menu(ViewId::BookMenu, rect, MenuKind::Contextual, entries, rq, context);
    }

    // ---- prompts ----------------------------------------------------------------------------

    fn show_prompt(&mut self, title: &str, prompt: Prompt, hub: &Hub, rq: &mut RenderQueue, context: &mut Context) {
        self.close_prompt(hub, rq, context);
        let input = NamedInput::new(title.to_string(), ViewId::PromptDialog, ViewId::PromptInput, 24, context);
        rq.add(RenderData::new(input.id(), *input.rect(), UpdateMode::Gui));
        hub.send(Event::Focus(Some(ViewId::PromptInput))).ok();
        self.children.push(Box::new(input));
        self.prompt = Some(prompt);
    }

    fn close_prompt(&mut self, hub: &Hub, rq: &mut RenderQueue, context: &mut Context) {
        if let Some(i) = locate_by_id(self, ViewId::PromptDialog) {
            rq.add(RenderData::expose(*self.child(i).rect(), UpdateMode::Gui));
            self.children.remove(i);
        }
        self.toggle_keyboard(false, hub, rq, context);
        self.prompt = None;
    }

    fn toggle_keyboard(&mut self, enable: bool, hub: &Hub, rq: &mut RenderQueue, context: &mut Context) {
        if let Some(index) = locate::<Keyboard>(self) {
            if enable {
                return;
            }
            let mut rect = *self.child(index).rect();
            rect.absorb(self.child(index - 1).rect());
            self.children.drain(index - 1..=index);
            rq.add(RenderData::expose(rect, UpdateMode::Gui));
            context.kb_rect = Rectangle::default();
            hub.send(Event::Focus(None)).ok();
        } else if enable {
            let dpi = CURRENT_DEVICE.dpi;
            let (small_height, big_height) = (scale_by_dpi(SMALL_BAR_HEIGHT, dpi) as i32, scale_by_dpi(BIG_BAR_HEIGHT, dpi) as i32);
            let thickness = scale_by_dpi(THICKNESS_MEDIUM, dpi) as i32;
            let (_, big_thickness) = halves(thickness);
            let mut kb_rect = rect![self.rect.min.x, self.rect.max.y - (small_height + 3 * big_height) + big_thickness, self.rect.max.x, self.rect.max.y];
            let keyboard = Keyboard::new(&mut kb_rect, false, context);
            let separator = Filler::new(rect![self.rect.min.x, kb_rect.min.y - thickness, self.rect.max.x, kb_rect.min.y], BLACK);
            rq.add(RenderData::new(separator.id(), *separator.rect(), UpdateMode::Gui));
            rq.add(RenderData::new(keyboard.id(), *keyboard.rect(), UpdateMode::Gui));
            self.children.push(Box::new(separator));
            self.children.push(Box::new(keyboard));
        }
    }

    fn confirm(&mut self, text: String, entry: InoInkEntry, rq: &mut RenderQueue, context: &mut Context) {
        let d = Dialog::new(ViewId::ConfirmDialog, Some(Event::Select(EntryId::InoInk(entry))), text, context);
        rq.add(RenderData::new(d.id(), *d.rect(), UpdateMode::Gui));
        self.children.push(Box::new(d));
    }

    // ---- entries ------------------------------------------------------------------------------

    fn select(&mut self, entry: &InoInkEntry, hub: &Hub, rq: &mut RenderQueue, context: &mut Context) {
        match entry {
            InoInkEntry::Tab(t) => {
                self.search = None;
                context.inoink.set_tab(*t);
                self.reset(hub, rq, context);
            }
            InoInkEntry::OpenCategory(c) => {
                self.search = None;
                context.inoink.open_collection(c.clone());
                self.reset(hub, rq, context);
            }
            InoInkEntry::UnreadOnly(v) => {
                let mut o = context.inoink.view_options();
                o.unread_only = *v;
                context.inoink.set_view_options(o);
                self.reset(hub, rq, context);
            }
            InoInkEntry::MarkSeenRead => {
                let mut ids = self.unmarked(context);
                ids.truncate(inoink::store::BULK_LIMIT);
                let frozen = context.inoink.store_mut().and_then(|s| s.freeze_ids(&ids).ok());
                match frozen {
                    Some(sel) if !sel.articles.is_empty() => {
                        let n = sel.articles.len();
                        self.selection = Some(sel);
                        self.confirm(format!("Mark {n} articles read? Starred articles stay unread."), InoInkEntry::ConfirmMarkSeenRead, rq, context);
                    }
                    _ => self.notify("Nothing to mark", hub, rq, context),
                }
            }
            InoInkEntry::ConfirmMarkSeenRead => {
                if let (Some(sel), Some(store)) = (self.selection.take(), context.inoink.store_mut()) {
                    match store.apply_bulk(&sel, BulkAction::MarkRead) {
                        Ok(n) => {
                            context.inoink.changed();
                            self.notify(&format!("Marked {n} read"), hub, rq, context);
                        }
                        Err(_) => self.notify(crate::view::inoink::actions::SAVE_FAILED, hub, rq, context),
                    }
                    self.reset(hub, rq, context);
                }
            }
            InoInkEntry::ViewGroup(g) => {
                let mut v = context.inoink.view_options();
                v.group = *g;
                context.inoink.set_view_options(v);
                self.reset(hub, rq, context);
            }
            InoInkEntry::ViewSort(so) => {
                let mut v = context.inoink.view_options();
                v.sort = *so;
                context.inoink.set_view_options(v);
                self.reset(hub, rq, context);
            }
            InoInkEntry::CategorySort(s) => {
                context.inoink.set_category_sort(*s);
                self.reset(hub, rq, context);
            }
            InoInkEntry::SyncNow => {
                if context.inoink.handle.is_some() {
                    // The sync button shows the progress.
                    context.inoink.submit(inoink::runtime::Command::SyncNow);
                } else {
                    self.notify("Sign in to sync. See the setup guide.", hub, rq, context);
                }
            }
            InoInkEntry::UndoDone => match undo_done(context) {
                Ok(o) => {
                    if let Some(m) = o.message {
                        self.notify(&m, hub, rq, context);
                    }
                    self.load_page(hub, rq, context);
                }
                Err(m) => self.notify(&m, hub, rq, context),
            },
            InoInkEntry::Article(a) => {
                // From the row menu: acts on the long-pressed article.
                if let Some(id) = self.pending_row.take() {
                    let r = apply_article_action(context, id, a);
                    match r {
                        Ok(o) => {
                            if let Some(m) = o.message {
                                self.notify(&m, hub, rq, context);
                            }
                        }
                        Err(m) => self.notify(&m, hub, rq, context),
                    }
                    self.load_page(hub, rq, context);
                }
            }
            InoInkEntry::Subscribe => self.show_prompt("Feed address", Prompt::Entry(entry.clone()), hub, rq, context),
            InoInkEntry::PromptRenameFeed(_) => self.show_prompt("New name", Prompt::Entry(entry.clone()), hub, rq, context),
            InoInkEntry::PromptNewFolderForFeed(_) => self.show_prompt("Folder name", Prompt::Entry(entry.clone()), hub, rq, context),
            InoInkEntry::PromptRenameFolder(_) => self.show_prompt("New folder name", Prompt::Entry(entry.clone()), hub, rq, context),
            InoInkEntry::PromptNewMonitoring => self.show_prompt("Words to watch", Prompt::Entry(entry.clone()), hub, rq, context),
            InoInkEntry::Unsubscribe(f) => self.confirm("Unfollow this feed? Its articles stay.".into(), InoInkEntry::ConfirmUnsubscribe(f.clone()), rq, context),
            InoInkEntry::DeleteFolder(f) => self.confirm("Delete this folder? Its feeds are kept.".into(), InoInkEntry::ConfirmDeleteFolder(f.clone()), rq, context),
            InoInkEntry::DeleteMonitoring(m) => self.confirm("Delete this monitoring search?".into(), InoInkEntry::ConfirmDeleteMonitoring(m.clone()), rq, context),
            InoInkEntry::MarkListRead => {
                let q = ArticleQuery { limit: 0, after: None, ..self.query(context, None, 0) };
                let frozen = context.inoink.store_mut().and_then(|s| s.freeze_selection(&q).ok());
                match frozen {
                    Some(sel) if !sel.articles.is_empty() => {
                        let n = sel.articles.len();
                        self.selection = Some(sel);
                        let more = if n >= inoink::store::BULK_LIMIT { " (the first 1000)" } else { "" };
                        self.confirm(format!("Mark {n} article(s){more} read? Stars are kept."), InoInkEntry::ConfirmMarkListRead, rq, context);
                    }
                    _ => self.notify("Nothing to mark", hub, rq, context),
                }
            }
            InoInkEntry::ConfirmMarkListRead => {
                if let (Some(sel), Some(store)) = (self.selection.take(), context.inoink.store_mut()) {
                    match store.apply_bulk(&sel, BulkAction::MarkRead) {
                        Ok(n) => {
                            context.inoink.changed();
                            self.notify(&format!("Marked {n} read"), hub, rq, context);
                        }
                        Err(_) => self.notify(crate::view::inoink::actions::SAVE_FAILED, hub, rq, context),
                    }
                    self.load_page(hub, rq, context);
                }
            }
            InoInkEntry::OpenDetails(id) => {
                hub.send(Event::OpenDetails(*id)).ok();
            }
            InoInkEntry::SetHeadlineTap(h) => {
                context.settings.inoink.list_tap = *h;
                context.inoink.settings.list_tap = *h;
                self.reset(hub, rq, context);
            }
            InoInkEntry::SetQueueTitles(t) => {
                context.settings.inoink.queue_titles = *t;
                context.inoink.settings.queue_titles = *t;
            }
            other => match manage::apply(context, other, None) {
                Ok(m) => {
                    if let Some(m) = m {
                        self.notify(&m, hub, rq, context);
                    }
                    self.load_page(hub, rq, context);
                }
                Err(m) => self.notify(&m, hub, rq, context),
            },
        }
    }

    fn submit(&mut self, text: &str, hub: &Hub, rq: &mut RenderQueue, context: &mut Context) {
        let Some(prompt) = self.prompt.clone() else { return };
        self.close_prompt(hub, rq, context);
        match prompt {
            Prompt::Search => {
                let t = text.trim();
                self.search = if t.is_empty() { None } else { Some(t.to_string()) };
                self.reset(hub, rq, context);
            }
            Prompt::Entry(entry) => match manage::apply(context, &entry, Some(text)) {
                Ok(m) => {
                    if let Some(m) = m {
                        self.notify(&m, hub, rq, context);
                    }
                    self.load_page(hub, rq, context);
                }
                Err(m) => self.notify(&m, hub, rq, context),
            },
        }
    }
}

/// Grouped lists read at most this many summaries to count their pages.
const PAGE_SCAN_LIMIT: usize = 3000;

pub fn browse_key(c: &Collection) -> String {
    format!("browse:{}", serde_json::to_string(c).unwrap_or_default())
}

fn remove_child(home: &mut InoInkHome, id: ViewId, rq: &mut RenderQueue) {
    if let Some(i) = locate_by_id(home, id) {
        rq.add(RenderData::expose(*home.child(i).rect(), UpdateMode::Gui));
        home.children.remove(i);
    }
}

impl View for InoInkHome {
    fn handle_event(&mut self, evt: &Event, hub: &Hub, _bus: &mut Bus, rq: &mut RenderQueue, context: &mut Context) -> bool {
        match *evt {
            Event::Page(dir) => {
                self.go(dir, hub, rq, context);
                true
            }
            Event::Device(DeviceEvent::Button { code: ButtonCode::Backward, status: ButtonStatus::Pressed, .. }) => {
                self.go(CycleDir::Previous, hub, rq, context);
                true
            }
            Event::Device(DeviceEvent::Button { code: ButtonCode::Forward, status: ButtonStatus::Pressed, .. }) => {
                self.go(CycleDir::Next, hub, rq, context);
                true
            }
            Event::ToggleNear(ViewId::TitleMenu, rect) => {
                let entries = self.title_menu_entries(context);
                self.toggle_menu(ViewId::TitleMenu, rect, MenuKind::DropDown, entries, rq, context);
                true
            }
            Event::ToggleNear(ViewId::SortMenu, rect) if self.showing_cats => {
                let current = context.inoink.category_sort();
                let mut entries = vec![EntryKind::Message("Sort by".to_string(), None)];
                for s in CategorySort::ALL {
                    entries.push(EntryKind::RadioButton(s.label().to_string(), EntryId::InoInk(InoInkEntry::CategorySort(s)), s == current));
                }
                // The menu opens upward from the footer and lists its entries bottom to top.
                entries.reverse();
                self.toggle_menu(ViewId::SortMenu, rect, MenuKind::DropDown, entries, rq, context);
                true
            }
            Event::ToggleNear(ViewId::SortMenu, rect) => {
                let stored = context.inoink.view_options();
                let mut entries = vec![
                    EntryKind::Message("Show".to_string(), None),
                    EntryKind::RadioButton("Unread only".to_string(), EntryId::InoInk(InoInkEntry::UnreadOnly(true)), stored.unread_only),
                    EntryKind::RadioButton("All articles".to_string(), EntryId::InoInk(InoInkEntry::UnreadOnly(false)), !stored.unread_only),
                    EntryKind::Separator,
                    EntryKind::Message("Group by".to_string(), None),
                ];
                for g in ArticleGroup::ALL {
                    entries.push(EntryKind::RadioButton(g.label().to_string(), EntryId::InoInk(InoInkEntry::ViewGroup(g)), g == stored.group));
                }
                entries.push(EntryKind::Separator);
                entries.push(EntryKind::Message("Sort by".to_string(), None));
                for so in ArticleSort::ALL {
                    if so == ArticleSort::Source && stored.group == ArticleGroup::Source {
                        continue;
                    }
                    entries.push(EntryKind::RadioButton(so.label().to_string(), EntryId::InoInk(InoInkEntry::ViewSort(so)), so == stored.sort));
                }
                // The menu opens upward from the footer and lists its entries bottom to top.
                entries.reverse();
                self.toggle_menu(ViewId::SortMenu, rect, MenuKind::DropDown, entries, rq, context);
                true
            }
            Event::ToggleNear(ViewId::MainMenu, rect) => {
                let entries = manage::main_menu_entries(context);
                self.toggle_menu(ViewId::MainMenu, rect, MenuKind::DropDown, entries, rq, context);
                true
            }
            Event::ToggleNear(ViewId::BatteryMenu, rect) => {
                crate::view::common::toggle_battery_menu(self, rect, None, rq, context);
                true
            }
            Event::ToggleNear(ViewId::ClockMenu, rect) => {
                crate::view::common::toggle_clock_menu(self, rect, None, rq, context);
                true
            }
            Event::ToggleBookMenu(rect, index) => {
                self.row_menu(rect, index, rq, context);
                true
            }
            Event::Toggle(ViewId::SearchBar) => {
                if self.prompt.is_some() {
                    self.close_prompt(hub, rq, context);
                } else if self.search.is_some() {
                    self.search = None;
                    self.reset(hub, rq, context);
                } else if context.inoink.tab == Tab::Categories && !context.inoink.show_categories {
                    context.inoink.show_category_list();
                    self.reset(hub, rq, context);
                } else {
                    self.show_prompt("Search downloaded articles", Prompt::Search, hub, rq, context);
                }
                true
            }
            Event::ArticleAction(id, ArticleAction::ToggleStar) => {
                match apply_article_action(context, id, &ArticleAction::ToggleStar) {
                    Ok(_) => {
                        let starred = context.inoink.store().and_then(|s| s.effective(id, &Membership::Starred).ok()).unwrap_or(false);
                        for i in self.items.iter_mut().filter(|i| i.id == id) {
                            i.starred = starred;
                        }
                        if let Some(list) = self.children[LIST].downcast_mut::<List>() {
                            for row in list.children.iter_mut().filter_map(|c| c.downcast_mut::<ArticleRow>()) {
                                if row.item.id == id {
                                    row.set_starred(starred, rq);
                                }
                            }
                        }
                        if self.layout.iter().any(|s| matches!(s, Slot::End)) {
                            // The "Mark N read" card counts what saving just changed.
                            self.end_count = self.count_unmarked(context);
                            if self.end_count == 0 {
                                self.load_page(hub, rq, context);
                            } else {
                                self.rebuild_list(rq, context);
                            }
                        }
                        self.update_chrome(rq, context);
                    }
                    Err(m) => self.notify(&m, hub, rq, context),
                }
                true
            }
            Event::OpenArticle(id) => {
                let ids = context.inoink.store().and_then(|s| s.query_ids(&self.query(context, None, 0), inoink::store::BULK_LIMIT).ok()).unwrap_or_default();
                let label = if self.search.is_some() { "Search".to_string() } else { collection_title(context, &context.inoink.collection) };
                let read_later = self.search.is_none() && context.inoink.collection == Collection::Starred;
                context.inoink.queue = Some(ReadingQueue { ids, read_later, label });
                hub.send(Event::OpenArticle(id)).ok();
                true
            }
            Event::Select(EntryId::InoInk(ref entry)) => {
                self.close_menus(rq);
                let entry = entry.clone();
                self.select(&entry, hub, rq, context);
                true
            }
            Event::Close(id @ (ViewId::TitleMenu | ViewId::MainMenu | ViewId::BookMenu | ViewId::ManageMenu | ViewId::SortMenu | ViewId::BatteryMenu | ViewId::ClockMenu | ViewId::ConfirmDialog)) => {
                remove_child(self, id, rq);
                true
            }
            Event::Close(ViewId::PromptDialog) => {
                self.close_prompt(hub, rq, context);
                true
            }
            Event::Focus(v) => {
                self.focus = v;
                if v.is_some() {
                    self.toggle_keyboard(true, hub, rq, context);
                }
                true
            }
            Event::Submit(ViewId::PromptInput, ref text) => {
                let t = text.clone();
                self.submit(&t, hub, rq, context);
                true
            }
            Event::Sync(ref u) => {
                let refresh = context.inoink.apply(u);
                if let inoink::runtime::Update::Browsed { collection, next } = u {
                    if let Some(store) = context.inoink.store_mut() {
                        let _ = store.set_meta(&browse_key(collection), next.as_deref().unwrap_or(""));
                    }
                }
                if refresh {
                    self.load_page(hub, rq, context);
                }
                self.update_sync_icon(rq, context);
                true
            }
            Event::Reseed => {
                self.load_page(hub, rq, context);
                if let Some(top) = self.children[TOP].downcast_mut::<TopBar>() {
                    top.reseed(rq, context);
                }
                self.update_sync_icon(rq, context);
                // Coming back from the reader (or a menu): repaint everything.
                rq.add(RenderData::new(self.id, self.rect, UpdateMode::Full));
                true
            }
            Event::ClockTick => {
                if let Some(top) = self.children[TOP].downcast_mut::<TopBar>() {
                    top.update_clock_label(rq);
                }
                true
            }
            Event::BatteryTick => {
                if let Some(top) = self.children[TOP].downcast_mut::<TopBar>() {
                    top.update_battery_widget(rq, context);
                }
                true
            }
            Event::ToggleFrontlight | Event::FrontlightChanged => {
                if let Some(top) = self.children[TOP].downcast_mut::<TopBar>() {
                    top.update_frontlight_icon(rq, context);
                }
                true
            }
            Event::NetworkChanged => {
                if let Some(top) = self.children[TOP].downcast_mut::<TopBar>() {
                    top.update_wifi_icon(rq, context);
                }
                true
            }
            _ => false,
        }
    }

    fn render(&self, _fb: &mut dyn Framebuffer, _rect: Rectangle, _fonts: &mut Fonts) {}

    fn might_rotate(&self) -> bool {
        self.prompt.is_none()
    }

    fn resize(&mut self, rect: Rectangle, hub: &Hub, rq: &mut RenderQueue, context: &mut Context) {
        // Rotation: rebuild the whole layout for the new size.
        let fresh = InoInkHome::new(rect, hub, rq, context);
        self.children = fresh.children;
        self.rows = fresh.rows;
        self.rect = rect;
        self.page = 0;
        self.cursors = vec![None];
        self.cat_page = 0;
        self.end_only = false;
        self.load_page(hub, rq, context);
        rq.add(RenderData::new(self.id, rect, UpdateMode::Full));
    }

    fn rect(&self) -> &Rectangle {
        &self.rect
    }

    fn rect_mut(&mut self) -> &mut Rectangle {
        &mut self.rect
    }

    fn children(&self) -> &Vec<Box<dyn View>> {
        &self.children
    }

    fn children_mut(&mut self) -> &mut Vec<Box<dyn View>> {
        &mut self.children
    }

    fn id(&self) -> Id {
        self.id
    }

    fn view_id(&self) -> Option<ViewId> {
        Some(ViewId::Home)
    }
}
