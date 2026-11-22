//! The pickers of the two sections: the Newsfeed, folders and monitoring feeds; Read later, the
//! Archive, Web pages, Annotations and tags.

use crate::color::{BLACK, GRAY03, TEXT_INVERTED_HARD, TEXT_NORMAL};
use crate::context::Context;
use crate::device::CURRENT_DEVICE;
use crate::font::{font_from_style, Fonts, LIST_META, LIST_TITLE, LIST_TITLE_READ};
use crate::framebuffer::{Framebuffer, UpdateMode};
use crate::geom::{Point, Rectangle};
use crate::gesture::GestureEvent;
use crate::view::inoink::actions::InoInkEntry;
use crate::view::inoink::state::Tab;
use crate::view::{Bus, EntryId, Event, Hub, Id, RenderData, RenderQueue, View, ID_FEEDER};
use inoink::model::*;

/// One line of the category list.
#[derive(Clone, Debug, PartialEq)]
pub enum CategoryItem {
    Header(String),
    Entry { label: String, count: usize, target: Collection },
}

/// Everything a section's picker lists, its default view first. Feeds: the Newsfeed, the folders
/// in the chosen order, then the monitoring feeds. Saved: Read later, the Archive, Web pages and
/// Annotations, then the tags A-Z. Sections with nothing in them are left out.
pub fn category_items(context: &Context, tab: Tab) -> Vec<CategoryItem> {
    let mut items = Vec::new();
    let Some(store) = context.inoink.store() else { return items };
    let tags = store.tags().unwrap_or_default();
    let count = |c: Collection, unread: bool| {
        store.count_articles(&ArticleQuery { collection: Some(c), unread_only: unread, ..Default::default() }).unwrap_or(0)
    };
    match tab {
        Tab::Feeds => {
            items.push(CategoryItem::Entry { label: "Newsfeed".into(), count: count(Collection::All, true), target: Collection::All });
            let sort = context.inoink.category_sort();
            let counts = store.unread_by_folder().unwrap_or_default();
            let mut folders: Vec<(String, usize)> = store
                .folders_in_order()
                .unwrap_or_default()
                .into_iter()
                .map(|f| {
                    let n = counts.get(&f).copied().unwrap_or(0);
                    (f, n)
                })
                .collect();
            match sort {
                CategorySort::Inoreader => {}
                CategorySort::Name => folders.sort_by_key(|(f, _)| f.to_lowercase()),
                // Stable: ties keep Inoreader's order.
                CategorySort::MostUnread => folders.sort_by(|a, b| b.1.cmp(&a.1)),
            }
            if !folders.is_empty() {
                items.push(CategoryItem::Header("Folders".into()));
                for (f, count) in folders {
                    items.push(CategoryItem::Entry { label: f.clone(), count, target: Collection::Folder(f) });
                }
            }
            let monitoring: Vec<&inoink::store::TagInfo> = tags.iter().filter(|t| t.kind == "active_search").collect();
            if !monitoring.is_empty() {
                items.push(CategoryItem::Header("Monitoring feeds".into()));
                for t in monitoring {
                    let name = t.id.rsplit('/').next().unwrap_or("Search").to_string();
                    items.push(CategoryItem::Entry { label: name, count: 0, target: Collection::Monitoring(StreamId::new(&t.id)) });
                }
            }
        }
        Tab::Saved => {
            items.push(CategoryItem::Entry { label: "Read later".into(), count: count(Collection::Starred, false), target: Collection::Starred });
            items.push(CategoryItem::Entry { label: "Archive".into(), count: 0, target: Collection::Archived });
            items.push(CategoryItem::Entry { label: "Web pages".into(), count: 0, target: Collection::SavedPages });
            items.push(CategoryItem::Entry { label: "Annotations".into(), count: 0, target: Collection::Annotated });
            let mut user_tags: Vec<String> =
                tags.iter().filter(|t| t.kind == "tag").filter_map(|t| t.id.strip_prefix("user/-/label/").map(String::from)).collect();
            user_tags.sort_by_key(|t| t.to_lowercase());
            if !user_tags.is_empty() {
                items.push(CategoryItem::Header("Tags".into()));
                for t in user_tags {
                    items.push(CategoryItem::Entry { label: t.clone(), count: 0, target: Collection::Tag(t) });
                }
            }
        }
    }
    items
}

/// A tappable category: its name, and the number of unread articles on the device when known.
pub struct CategoryRow {
    id: Id,
    rect: Rectangle,
    children: Vec<Box<dyn View>>,
    label: String,
    count: usize,
    target: Collection,
    active: bool,
}

impl CategoryRow {
    pub fn new(rect: Rectangle, label: String, count: usize, target: Collection) -> CategoryRow {
        CategoryRow { id: ID_FEEDER.next(), rect, children: Vec::new(), label, count, target, active: false }
    }
}

impl View for CategoryRow {
    fn handle_event(&mut self, evt: &Event, _hub: &Hub, bus: &mut Bus, rq: &mut RenderQueue, _context: &mut Context) -> bool {
        match *evt {
            Event::Gesture(GestureEvent::Tap(c)) if self.rect.includes(c) => {
                self.active = true;
                rq.add(RenderData::new(self.id, self.rect, UpdateMode::Gui));
                bus.push_back(Event::Select(EntryId::InoInk(InoInkEntry::OpenCategory(self.target.clone()))));
                true
            }
            Event::Gesture(GestureEvent::HoldFingerShort(c, ..)) if self.rect.includes(c) => true,
            _ => false,
        }
    }

    fn render(&self, fb: &mut dyn Framebuffer, _rect: Rectangle, fonts: &mut Fonts) {
        let dpi = CURRENT_DEVICE.dpi;
        let scheme = if self.active { TEXT_INVERTED_HARD } else { TEXT_NORMAL };
        fb.draw_rectangle(&self.rect, scheme[0]);
        let padding = font_from_style(fonts, &LIST_TITLE, dpi).em() as i32;
        let mut right = self.rect.max.x - padding;
        if self.count > 0 {
            let font = font_from_style(fonts, &LIST_META, dpi);
            let plan = font.plan(&self.count.to_string(), None, None);
            let y = self.rect.min.y + (self.rect.height() as i32 + font.x_heights.0 as i32) / 2;
            font.render(fb, if self.active { scheme[2] } else { GRAY03 }, &plan, Point::new(right - plan.width, y));
            right -= plan.width + padding;
        }
        let style = if self.count > 0 { &*LIST_TITLE } else { &*LIST_TITLE_READ };
        let font = font_from_style(fonts, style, dpi);
        let x = self.rect.min.x + padding;
        let plan = font.plan(&self.label, Some((right - x).max(1)), None);
        let y = self.rect.min.y + (self.rect.height() as i32 + font.x_heights.0 as i32) / 2;
        font.render(fb, if self.active { scheme[1] } else { BLACK }, &plan, Point::new(x, y));
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
}
