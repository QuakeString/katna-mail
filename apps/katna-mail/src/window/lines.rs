// SPDX-License-Identifier: GPL-3.0-or-later

//! The message list's lines as GPUI's `List` holds them: a page of at most
//! [`PAGE`] lines around the ones on show, not all of them. GPUI keeps
//! state per line and builds it again whole on each reset and each change
//! of width, which for a 70,000-line Inbox cost milliseconds at start and
//! on every frame of a resize. The page moves along before a frame whenever
//! the lines on show come near its edge, keeping the scroll position to the
//! pixel, so it never shows. Every index in and out of [`Lines`] counts all
//! of the list's lines (`MailWindow::entries`).

use std::cell::Cell;

use gpui::{ListAlignment, ListOffset, ListState, Pixels};
use katna_ui::px;

/// Most lines in the page. The page moves when the top line on show comes
/// within [`MARGIN`] lines of its edge, and centres it again.
const PAGE: usize = 1200;
const MARGIN: usize = 300;

pub(super) struct Lines {
    state: ListState,
    /// The page: the list's line at its start, and its number of lines.
    base: Cell<usize>,
    len: Cell<usize>,
    /// All of the list's lines, and their height until drawn.
    count: Cell<usize>,
    height: Cell<Pixels>,
    /// How tall the bar pinned over the list's top is: lines brought into
    /// view stop below it.
    head: Cell<Pixels>,
    /// A line GPUI brought into view from out of the page, put below the
    /// bar once drawn.
    reveal: Cell<Option<usize>>,
}

impl Lines {
    pub(super) fn new() -> Self {
        Self {
            state: ListState::new(0, ListAlignment::Top, px(400.0)),
            base: Cell::new(0),
            len: Cell::new(0),
            count: Cell::new(0),
            height: Cell::new(px(0.0)),
            head: Cell::new(px(0.0)),
            reveal: Cell::new(None),
        }
    }

    /// GPUI's state of the page, for the `list` element, which draws the
    /// page's line `ix` as the list's line [`Lines::line`]`(ix)`.
    pub(super) fn state(&self) -> &ListState {
        &self.state
    }

    /// The list's line that is the page's line `ix`.
    pub(super) fn line(&self, ix: usize) -> usize {
        self.base.get() + ix
    }

    /// `count` lines of `height` until drawn, scrolled to `top` if given,
    /// else to the start.
    pub(super) fn reset(&self, count: usize, height: Pixels, top: Option<ListOffset>) {
        self.count.set(count);
        self.height.set(height);
        let top = top.filter(|top| top.item_ix < count).unwrap_or(ListOffset {
            item_ix: 0,
            offset_in_item: px(0.0),
        });
        self.load(top.item_ix);
        self.state.scroll_to(self.local(top));
    }

    /// The bar pinned over the list's top is `head` tall.
    pub(super) fn set_head(&self, head: Pixels) {
        // The first line holds the room under the bar: drawn again at its
        // new height.
        if self.head.replace(head) != head && self.base.get() == 0 && self.len.get() > 0 {
            self.state.remeasure_items(0..1);
        }
    }

    /// Before a frame: moves the page along when the top line on show comes
    /// near its edge, and puts a line just brought into view below the bar.
    pub(super) fn follow(&self) {
        if let Some(ix) = self.reveal.get()
            && let Some(bounds) = self.bounds_for_item(ix)
        {
            self.reveal.set(None);
            let top = self.viewport_bounds().top() + self.head.get();
            if bounds.top() < top {
                self.state.scroll_by(bounds.top() - top);
            }
        }
        let top = self.state.logical_scroll_top().item_ix;
        let (base, len) = (self.base.get(), self.len.get());
        let near_start = base > 0 && top < MARGIN;
        let near_end = base + len < self.count.get() && len.saturating_sub(top) < MARGIN;
        if near_start || near_end {
            let top = self.logical_scroll_top();
            self.load(top.item_ix);
            self.state.scroll_to(self.local(top));
        }
    }

    /// Where the list is scrolled to.
    pub(super) fn logical_scroll_top(&self) -> ListOffset {
        let top = self.state.logical_scroll_top();
        ListOffset {
            item_ix: self.line(top.item_ix),
            offset_in_item: top.offset_in_item,
        }
    }

    /// Scrolls to `top`.
    pub(super) fn scroll_to(&self, top: ListOffset) {
        if !self.holds(top.item_ix) {
            self.load(top.item_ix);
        }
        self.state.scroll_to(self.local(top));
    }

    /// Scrolls just enough for line `ix` to be on show: at the bottom when
    /// it is below, at the top when above.
    pub(super) fn scroll_to_reveal_item(&self, ix: usize) {
        if ix >= self.count.get() {
            return;
        }
        // Drawn: just enough to show it whole between the bar and the
        // bottom.
        if let Some(bounds) = self.bounds_for_item(ix) {
            let view = self.viewport_bounds();
            let top = view.top() + self.head.get();
            if bounds.top() < top {
                self.state.scroll_by(bounds.top() - top);
            } else if bounds.bottom() > view.bottom() {
                self.state
                    .scroll_by((bounds.bottom() - view.bottom()).min(bounds.top() - top));
            }
            return;
        }
        self.reveal.set(Some(ix));
        if !self.holds(ix) {
            let below = ix > self.logical_scroll_top().item_ix;
            self.load(ix);
            // Scrolled to the far side of it, so GPUI reveals it from there.
            let from = if below { self.base.get() } else { ix + 1 };
            self.state.scroll_to(self.local(ListOffset {
                item_ix: from,
                offset_in_item: px(0.0),
            }));
        }
        self.state.scroll_to_reveal_item(ix - self.base.get());
    }

    /// Where line `ix` was drawn in the last frame, if it was.
    pub(super) fn bounds_for_item(&self, ix: usize) -> Option<gpui::Bounds<Pixels>> {
        self.holds(ix)
            .then(|| self.state.bounds_for_item(ix - self.base.get()))
            .flatten()
    }

    pub(super) fn viewport_bounds(&self) -> gpui::Bounds<Pixels> {
        self.state.viewport_bounds()
    }

    pub(super) fn scroll_by(&self, distance: Pixels) {
        self.state.scroll_by(distance);
    }

    /// How far the list is scrolled from its start, in pixels: lines before
    /// the page count at their height until drawn.
    pub(super) fn scrolled(&self) -> Pixels {
        let before = self.height.get() * self.base.get() as f32;
        before - self.state.scroll_px_offset_for_scrollbar().y
    }

    /// Lines drawn again at their new height, as after a change of density
    /// or scale.
    pub(super) fn remeasure(&self) {
        self.state.remeasure();
    }

    /// Line `ix` has gone from the list.
    pub(super) fn remove(&self, ix: usize) {
        let (base, len) = (self.base.get(), self.len.get());
        if ix >= self.count.get() {
            return;
        }
        self.count.set(self.count.get() - 1);
        if ix < base {
            self.base.set(base - 1);
        } else if ix < base + len {
            self.state.splice(ix - base..ix - base + 1, 0);
            self.len.set(len - 1);
        }
    }

    fn holds(&self, ix: usize) -> bool {
        (self.base.get()..self.base.get() + self.len.get()).contains(&ix)
    }

    /// Makes the page the lines around line `around`.
    fn load(&self, around: usize) {
        let count = self.count.get();
        let base = around
            .saturating_sub(PAGE / 2)
            .min(count.saturating_sub(PAGE));
        let len = (count - base).min(PAGE);
        self.base.set(base);
        self.len.set(len);
        self.state.reset_with_uniform_height(len, self.height.get());
    }

    /// `top` in the page's lines.
    fn local(&self, top: ListOffset) -> ListOffset {
        ListOffset {
            item_ix: top.item_ix.saturating_sub(self.base.get()),
            offset_in_item: top.offset_in_item,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(item_ix: usize) -> ListOffset {
        ListOffset {
            item_ix,
            offset_in_item: px(0.0),
        }
    }

    /// `ListOffset` has no `PartialEq`.
    fn place(top: ListOffset) -> (usize, Pixels) {
        (top.item_ix, top.offset_in_item)
    }

    #[test]
    fn the_page_follows_the_scrolling_and_keeps_the_place() {
        let lines = Lines::new();
        lines.reset(70_000, px(40.0), None);
        assert_eq!((lines.base.get(), lines.len.get()), (0, PAGE));
        assert_eq!(lines.logical_scroll_top().item_ix, 0);

        // Near the page's end: it moves, the top line and offset stay.
        let top = ListOffset {
            item_ix: PAGE - 10,
            offset_in_item: px(7.0),
        };
        lines.scroll_to(top);
        assert_eq!(lines.base.get(), 0, "still in the page");
        lines.follow();
        assert_eq!(lines.base.get(), PAGE - 10 - PAGE / 2);
        assert_eq!(place(lines.logical_scroll_top()), place(top));
        assert_eq!(lines.line(PAGE / 2), PAGE - 10);

        // A jump far away loads the page there; the end has no page after it.
        lines.scroll_to(at(69_990));
        assert_eq!((lines.base.get(), lines.len.get()), (70_000 - PAGE, PAGE));
        lines.follow();
        assert_eq!(lines.base.get(), 70_000 - PAGE, "nothing after the end");
        assert_eq!(place(lines.logical_scroll_top()), place(at(69_990)));

        // Back up to the start.
        lines.scroll_to_reveal_item(3);
        assert_eq!(lines.base.get(), 0);
        assert_eq!(place(lines.logical_scroll_top()), place(at(3)));

        // Kept in place across a reset, as after a reload.
        lines.reset(70_000, px(40.0), Some(at(40_000)));
        assert_eq!(place(lines.logical_scroll_top()), place(at(40_000)));
        assert_eq!(lines.scrolled(), px(40.0 * 40_000.0));
    }

    #[test]
    fn removed_lines_leave_the_page_in_step() {
        let lines = Lines::new();
        lines.reset(5_000, px(40.0), Some(at(3_000)));
        let base = lines.base.get();
        lines.remove(10);
        assert_eq!(lines.base.get(), base - 1, "a line before the page");
        assert_eq!(place(lines.logical_scroll_top()), place(at(2_999)));
        lines.remove(3_500);
        assert_eq!(lines.len.get(), PAGE - 1, "a line in the page");
        lines.remove(4_990);
        assert_eq!(lines.count.get(), 4_997);
        assert_eq!(lines.len.get(), PAGE - 1, "a line after the page");
    }

    #[test]
    fn a_short_list_is_one_page() {
        let lines = Lines::new();
        lines.reset(30, px(40.0), None);
        assert_eq!((lines.base.get(), lines.len.get()), (0, 30));
        lines.scroll_to_reveal_item(29);
        lines.follow();
        assert_eq!(lines.base.get(), 0);
        lines.reset(0, px(40.0), None);
        assert_eq!(lines.len.get(), 0);
        lines.scroll_to_reveal_item(0);
    }
}
