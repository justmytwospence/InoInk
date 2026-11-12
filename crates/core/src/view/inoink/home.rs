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
use crate::view::inoink::actions::{apply_article_action, undo_done, ArticleAction, InoInkEntry};
use crate::view::inoink::group_header::GroupHeader;
use crate::view::inoink::manage;
use crate::view::inoink::row::ArticleRow;
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
}

const TOP: usize = 0;
const LIST: usize = 2;
const FOOTER: usize = 4;

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
        Collection::Starred => "Starred".into(),
        Collection::Unread => "Unread".into(),
        Collection::All => "All articles".into(),
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

        let title = collection_title(context, &context.inoink.collection.clone());
        let top = TopBar::new(rect![rect.min.x, rect.min.y, rect.max.x, rect.min.y + small_height - small_thickness], Event::Toggle(ViewId::SearchBar), title, context);
        children.push(Box::new(top));
        children.push(Box::new(Filler::new(rect![rect.min.x, rect.min.y + small_height - small_thickness, rect.max.x, rect.min.y + small_height + big_thickness], BLACK)));

        let list_rect = rect![rect.min.x, rect.min.y + small_height + big_thickness, rect.max.x, rect.max.y - small_height - small_thickness];
        let rows = (((list_rect.height() as i32) + thickness) / big_height).max(1) as usize;
        children.push(Box::new(List { id: ID_FEEDER.next(), rect: list_rect, children: Vec::new() }));
        children.push(Box::new(Filler::new(rect![rect.min.x, rect.max.y - small_height - small_thickness, rect.max.x, rect.max.y - small_height + big_thickness], BLACK)));
        let footer_rect = rect![rect.min.x, rect.max.y - small_height + big_thickness, rect.max.x, rect.max.y];
        children.push(Box::new(Footer { id: ID_FEEDER.next(), rect: footer_rect, children: Vec::new() }));

        let mut home = InoInkHome { id, rect, children, items: Vec::new(), cursors: vec![None], page: 0, next: None, layout: Vec::new(), search: None, prompt: None, focus: None, rows, selection: None, pending_row: None };
        home.load_page(hub, rq, context);
        rq.add(RenderData::new(home.id, rect, UpdateMode::Full));
        home
    }

    fn query(&self, context: &Context, after: Option<Cursor>, limit: usize) -> ArticleQuery {
        let v = context.inoink.view_options();
        ArticleQuery { collection: Some(context.inoink.collection.clone()), unread_only: context.inoink.unread_only, search: self.search.clone(), limit, after, group: v.group, sort: v.sort }
    }

    /// The group and sort actually applied to the current list.
    fn effective_options(context: &Context) -> ViewOptions {
        ViewOptions { unread_only: context.inoink.unread_only, ..context.inoink.view_options() }.effective(&context.inoink.collection)
    }

    /// Reloads the current page from the local database. Never touches the network.
    pub fn load_page(&mut self, hub: &Hub, rq: &mut RenderQueue, context: &mut Context) {
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
            return self.load_page(hub, rq, context);
        }
        // Plan the page: headers cost one unit, articles two; a page has two units per row.
        let opts = Self::effective_options(context);
        let units = 2 * self.rows - if context.inoink.last_undo.is_some() { 2 } else { 0 };
        let mut layout = Vec::new();
        let mut used = 0;
        let mut placed = 0;
        let mut last_header: Option<String> = None;
        for (i, item) in items.iter().enumerate() {
            let h = opts.group.header(item);
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
        self.layout = layout;
        self.items = items;
        self.next = next;
        self.rebuild_list(rq, context);
        self.rebuild_footer(rq, context);
        let title = if let Some(q) = &self.search { format!("Search: {q}") } else { collection_title(context, &context.inoink.collection.clone()) };
        if let Some(top) = self.children[TOP].downcast_mut::<TopBar>() {
            top.update_title_label(&title, rq);
        }
        if let Some(notice) = context.inoink.notice.take() {
            let n = Notification::new(notice, hub, rq, context);
            self.children.push(Box::new(n));
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
            _ => match context.inoink.collection {
                Collection::Starred => ("No starred articles".into(), "Star an article to keep it here.".into()),
                Collection::Unread => ("All caught up".into(), String::new()),
                _ => ("Nothing here".into(), String::new()),
            },
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
        let list = self.children[LIST].downcast_mut::<List>().unwrap();
        list.children.clear();
        let rect = list.rect;
        let total_units = 2 * rows as i32;
        let unit_h = divide(rect.height() as i32, total_units);
        let span = |from: i32, n: i32| -> i32 { unit_h[from as usize..(from + n) as usize].iter().sum() };
        let mut y = rect.min.y;
        let _ = big_height;
        let mut unit = 0i32;
        if let Some((_, title)) = context.inoink.last_undo.as_ref() {
            // The undo bar takes the first row until superseded.
            let h = span(0, 2);
            let r = rect![rect.min.x, y, rect.max.x, y + h - small_thickness];
            let label = format!("Done: {}", title.chars().take(40).collect::<String>());
            let lbl = Label::new(rect![r.min.x, r.min.y, r.max.x - r.height() as i32 * 2, r.max.y], label, Align::Left(r.height() as i32 / 3));
            list.children.push(Box::new(lbl));
            list.children.push(Box::new(Label::new(rect![r.max.x - r.height() as i32 * 2, r.min.y, r.max.x, r.max.y], "Undo".to_string(), Align::Center).event(Some(Event::Select(EntryId::InoInk(InoInkEntry::UndoDone))))));
            list.children.push(Box::new(Filler::new(rect![rect.min.x, r.max.y, rect.max.x, r.max.y + thickness], SEPARATOR_NORMAL)));
            y += h;
            unit = 2;
        }
        for slot in &self.layout {
            let n = match slot {
                Slot::Header(_) => 1,
                Slot::Item(_) => 2,
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
                    list.children.push(Box::new(ArticleRow::new(rect![rect.min.x, y_min, rect.max.x, y_max], items[*i].clone(), *i)));
                    if !last {
                        list.children.push(Box::new(Filler::new(rect![rect.min.x, y_max, rect.max.x, y_max + thickness], SEPARATOR_NORMAL)));
                    }
                }
            }
            y += h;
            unit += n;
        }
        if items.is_empty() {
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
        let prev = self.page > 0;
        let next = self.next.is_some();
        let page = self.page + 1;
        let status = context.inoink.status_text();
        let filter = if context.inoink.unread_only || matches!(context.inoink.collection, Collection::Unread) { "Unread" } else { "All" };
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
        let inner = rect.width() as i32 - 2 * side;
        let filter_w = inner / 6;
        let view_w = inner / 4;
        let page_w = inner / 6;
        let status_w = inner - filter_w - view_w - page_w;
        let sort_text = format!("{} \u{25be}", Self::effective_options(context).sort.short());
        let x0 = rect.min.x + side;
        footer.children.push(Box::new(Label::new(rect![x0, rect.min.y, x0 + filter_w, rect.max.y], filter.to_string(), Align::Center).event(Some(Event::Select(EntryId::InoInk(InoInkEntry::UnreadOnly(filter != "Unread")))))));
        let view_rect = rect![x0 + filter_w, rect.min.y, x0 + filter_w + view_w, rect.max.y];
        footer.children.push(Box::new(Label::new(view_rect, sort_text, Align::Center).event(Some(Event::ToggleNear(ViewId::SortMenu, view_rect)))));
        let sx = x0 + filter_w + view_w;
        footer.children.push(Box::new(Label::new(rect![sx, rect.min.y, sx + status_w, rect.max.y], status, Align::Center).event(Some(Event::Select(EntryId::InoInk(InoInkEntry::SyncNow))))));
        footer.children.push(Box::new(Label::new(rect![sx + status_w, rect.min.y, rect.max.x - side, rect.max.y], format!("Page {page}"), Align::Center)));
        let next_rect = rect![rect.max - side, rect.max];
        if next {
            footer.children.push(Box::new(Icon::new("arrow-right", next_rect, Event::Page(CycleDir::Next))));
        } else {
            footer.children.push(Box::new(Filler::new(next_rect, WHITE)));
        }
        rq.add(RenderData::new(footer.id, footer.rect, UpdateMode::Gui));
    }

    fn go(&mut self, dir: CycleDir, hub: &Hub, rq: &mut RenderQueue, context: &mut Context) {
        match dir {
            CycleDir::Next => {
                if let Some(c) = self.next.clone() {
                    self.page += 1;
                    self.cursors.truncate(self.page);
                    self.cursors.push(Some(c));
                    self.load_page(hub, rq, context);
                } else if matches!(context.inoink.collection, Collection::All | Collection::Feed(_) | Collection::Folder(_) | Collection::Tag(_) | Collection::SavedPages) && context.inoink.handle.is_some() && self.search.is_none() && Self::effective_options(context) == ViewOptions::default() {
                    // End of what is stored: browse further back through history online.
                    context.inoink.submit(inoink::runtime::Command::Browse { collection: context.inoink.collection.clone(), continuation: context.inoink.store().and_then(|s| s.get_meta(&browse_key(&context.inoink.collection)).ok().flatten()) });
                    let n = Notification::new("Loading older articles".to_string(), hub, rq, context);
                    self.children.push(Box::new(n));
                }
            }
            CycleDir::Previous => {
                if self.page > 0 {
                    self.page -= 1;
                    self.cursors.truncate(self.page + 1);
                    self.load_page(hub, rq, context);
                }
            }
        }
    }

    fn reset(&mut self, hub: &Hub, rq: &mut RenderQueue, context: &mut Context) {
        self.page = 0;
        self.cursors = vec![None];
        self.load_page(hub, rq, context);
    }

    fn notify(&mut self, text: &str, hub: &Hub, rq: &mut RenderQueue, context: &mut Context) {
        let n = Notification::new(text.to_string(), hub, rq, context);
        self.children.push(Box::new(n));
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
            EntryKind::Command(if item.starred { "Unstar" } else { "Star" }.to_string(), e(ArticleAction::ToggleStar)),
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
            InoInkEntry::Collection(c) => {
                let unread = context.inoink.unread_only;
                context.inoink.set_view(c.clone(), unread);
                self.search = None;
                self.reset(hub, rq, context);
            }
            InoInkEntry::UnreadOnly(v) => {
                let c = context.inoink.collection.clone();
                if matches!(c, Collection::Unread) && !*v {
                    context.inoink.set_view(Collection::All, false);
                } else {
                    context.inoink.set_view(c, *v);
                }
                self.reset(hub, rq, context);
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
            InoInkEntry::SyncNow => {
                if context.inoink.handle.is_some() {
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
                context.settings.inoink.headline_tap = *h;
                context.inoink.settings.headline_tap = *h;
                self.reset(hub, rq, context);
            }
            InoInkEntry::SetContinueOnSwipe(b) => {
                context.settings.inoink.continue_on_swipe = *b;
                context.inoink.settings.continue_on_swipe = *b;
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
                let entries = manage::collection_entries(context);
                self.toggle_menu(ViewId::TitleMenu, rect, MenuKind::DropDown, entries, rq, context);
                true
            }
            Event::ToggleNear(ViewId::SortMenu, rect) => {
                let stored = context.inoink.view_options();
                let mut entries = vec![EntryKind::Message("Group by".to_string(), None)];
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
                } else {
                    self.show_prompt("Search downloaded articles", Prompt::Search, hub, rq, context);
                }
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
                if let inoink::runtime::Update::Cached(id, _) = u {
                    if context.inoink.opening == Some(*id) {
                        context.inoink.opening = None;
                        hub.send(Event::OpenArticle(*id)).ok();
                    }
                }
                if let inoink::runtime::Update::CacheFailed(id, _) = u {
                    if context.inoink.opening == Some(*id) {
                        context.inoink.opening = None;
                        self.notify("This article could not be downloaded", hub, rq, context);
                        hub.send(Event::Invalid(Default::default())).ok();
                    }
                }
                if let inoink::runtime::Update::Browsed { collection, next } = u {
                    if let Some(store) = context.inoink.store_mut() {
                        let _ = store.set_meta(&browse_key(collection), next.as_deref().unwrap_or(""));
                    }
                }
                if refresh {
                    self.load_page(hub, rq, context);
                } else {
                    self.rebuild_footer(rq, context);
                }
                true
            }
            Event::Reseed => {
                self.load_page(hub, rq, context);
                if let Some(top) = self.children[TOP].downcast_mut::<TopBar>() {
                    top.reseed(rq, context);
                }
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
