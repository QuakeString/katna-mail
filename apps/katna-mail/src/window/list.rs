// SPDX-License-Identifier: GPL-3.0-or-later

//! The list card: the toolbar (select, refresh, more; or actions on the
//! ticked lines), the inbox tabs and the lines, one row each, or three
//! stacked lines when the list is narrow.

use katna_ui::WindowDrag;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::{
    Animation, AnimationExt, AnyElement, Context, Div, FontWeight, HighlightStyle, ListOffset,
    SharedString, SpringAnimation, SpringConfig, Stateful, StyledText, anchored, deferred, div,
    ease_out_quint, list, point, prelude::*, relative, rgba,
};
use katna_core::config::Density;
use katna_i18n::tr;
use katna_ui::Ripple;
use katna_ui::motion::{self, Spring, lerp};
use katna_ui::px;
use katna_ui::tokens::duration;

/// The lift of the line under the pointer: critically damped and slower
/// than other hover feedback, so it rises and settles without a jolt.
const ROW_LIFT: SpringConfig = SpringConfig::new(500.0, 44.7, 1.0);
/// How strongly a tick box, star or marker that is off shows while the
/// pointer is not over its line.
const OFF_REST: f32 = 0.3;
/// How long the quick actions of a line take to fade in.
const ACTIONS_IN: Duration = Duration::from_millis(160);
/// The attachment chips under a line: their line's extra height, their
/// height, widest size and spacing, and the "+N" button's size.
const CHIPS_LINE: f32 = 40.0;
pub(super) const CHIP_HEIGHT: f32 = 30.0;
const CHIP_WIDTH: f32 = 184.0;
const CHIP_GAP: f32 = 8.0;
const MORE_SIZE: f32 = 30.0;
/// The Back to top button's size.
const TO_TOP_SIZE: f32 = 40.0;

use super::attachments::kind_badge;
use super::folder_pick::{PickFrom, PickMode};
use super::layout::FAB_SIZE;
use super::mail_drag::MailDrag;
use super::reader::Squeeze;
use super::{Act, LIST_CONTEXT, Listing, MailWindow, Menu, READER_CONTEXT, Reload, STACKED_BELOW};
use crate::data::{EntryKey, Row, RowFile};
use crate::format;
use crate::sidebar::Role;
use crate::theme::{Theme, fade, mix};
use crate::widgets::{
    TOOLBAR_HEIGHT, card_outline, elevation, icon, icon_button, icon_button_colored, menu,
    menu_item, menu_item_icon, tip, toolbar,
};
use gpui::DragMoveEvent;

/// The inbox tabs' pill bar: its height and inset, and each tab's height,
/// padding (with labels, and icons only), icon (larger with no label),
/// gaps and text; the counts'
/// badges; and the room the list's top row keeps for its buttons on the
/// left (more with lines ticked) and its "1–50 of N" and arrows on the
/// right, and a row of its own's padding.
const TABS_HEIGHT: f32 = 36.0;
const TABS_INSET: f32 = 3.0;
const TAB_HEIGHT: f32 = TABS_HEIGHT - 2.0 * TABS_INSET;
const TAB_PAD: f32 = 10.0;
const TAB_PAD_ICONS: f32 = 9.0;
const TAB_ICON: f32 = 16.0;
const TAB_ICON_ALONE: f32 = 19.0;
const TAB_GAP: f32 = 5.0;
const TAB_SPACING: f32 = 2.0;
const TAB_TEXT: f32 = 13.0;
const BADGE_HEIGHT: f32 = 18.0;
const BADGE_PAD: f32 = 6.0;
const BADGE_TEXT: f32 = 11.0;
/// The open tab's quiet count: how much of the highlight's text color tints
/// its badge and colors its number.
const CHIP_QUIET_BG: f32 = 0.12;
const CHIP_QUIET_TEXT: f32 = 0.7;
const TOP_ROW_LEFT: f32 = 152.0;
const TOP_ROW_LEFT_CHECKED: f32 = 340.0;
const TOP_ROW_RIGHT: f32 = 185.0;
const TABS_ROW_PAD: f32 = 12.0;
/// A phone's list bar with the tabs in it: the room on each side, and the
/// room the More button at the end of their pill takes.
const PHONE_TABS_SIDE: f32 = 8.0;
const PHONE_TABS_MORE: f32 = TAB_SPACING + TAB_HEIGHT + TABS_INSET;

/// The gap around the list toolbar's select pill: the toolbar's height
/// less the pill's, halved, so its left end sits as far in as its top.
const SELECT_PILL_GAP: f32 = (TOOLBAR_HEIGHT - 40.0) / 2.0;

/// An inbox tab's unread chip: how much of it shows, folding away once
/// the tab has nothing unread; how quiet it is, faint on the open tab and
/// in color on the others, changing slowly so a click doesn't flash it;
/// and the width and count it last had, kept while it folds away.
pub(super) struct TabChip {
    shown: Spring,
    quiet: Spring,
    /// 1 while its tab is open: the label of a folded bar grows in and
    /// out with it.
    on: Spring,
    width: f32,
    count: u64,
}

/// Where the inbox tabs go (see [`MailWindow::tabs_fit`]).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum TabsFit {
    TopRow,
    Row,
}

/// How far the inbox tabs fold to fit their room, in steps the fold
/// glides through: every label; then only the open tab's, the others'
/// icons growing as their labels go; then no counts; then no label at
/// all. No tab is ever hidden.
const FOLD_LABELS: f32 = 0.0;
const FOLD_OPEN_LABEL: f32 = 1.0;
const FOLD_NO_COUNTS: f32 = 2.0;
/// How long the list must be still after scrolling before the line under
/// the pointer shows its hover again.
const SCROLL_SETTLE: Duration = Duration::from_millis(150);
const FOLD_ICONS: f32 = 3.0;

/// The line just opened and how far below the list's top it was, kept
/// there while the reading pane opens (`MailWindow::keep_opened_line`).
#[derive(Debug, Clone, Copy)]
pub(super) struct KeepLine {
    pub ix: usize,
    pub top: gpui::Pixels,
    /// Put back in place; the next frame only nudges it fully into view.
    pub placed: bool,
}

/// What Read, Unread, Starred or Unstarred in the select menu ticked: the
/// matching lines on screen, then, from the banner's link, every matching
/// line of the list, as Gmail does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Picked {
    pub pick: Pick,
    /// Matching lines on screen, ticked first.
    pub on_screen: usize,
    /// Every matching line of the list, loaded or not, for the link.
    pub all: Vec<EntryKey>,
    /// `all` is ticked, not only the lines on screen.
    pub whole: bool,
}

impl Picked {
    /// How many lines are ticked while this pick still holds.
    fn ticked(&self) -> usize {
        if self.whole {
            self.all.len()
        } else {
            self.on_screen
        }
    }
}

/// What the select menu ticks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Pick {
    All,
    None,
    Read,
    Unread,
    Starred,
    Unstarred,
}

impl MailWindow {
    /// Height of one line of the list.
    pub(super) fn row_height(&self) -> f32 {
        let compact = self.config.mail.density == Density::Compact;
        match (self.stacked(), compact) {
            (true, false) => 76.0,
            (true, true) => 64.0,
            (false, false) => 40.0,
            (false, true) => 32.0,
        }
    }

    /// How much attachment chips under a line add to its height.
    fn chips_extra(&self, has_chips: bool) -> f32 {
        match (has_chips, self.stacked()) {
            (false, _) => 0.0,
            (true, false) => CHIPS_LINE,
            (true, true) => CHIPS_LINE - 4.0,
        }
    }

    /// The height line `ix` is drawn at, chips included.
    fn line_height_at(&mut self, ix: usize) -> f32 {
        let Some(entry) = self.entries.get(ix).copied() else {
            return self.row_height();
        };
        let folder = self.listed_folder();
        let has_chips = match &mut self.mail {
            Ok(mail) => mail
                .rows(&[entry], folder, self.show_recipients)
                .pop()
                .flatten()
                .is_some_and(|r| !r.files.is_empty()),
            Err(_) => false,
        };
        // The first line holds the room under the bar pinned over the top.
        let head = if ix == 0 { self.list_head.get() } else { 0.0 };
        self.row_height() + self.chips_extra(has_chips) + head
    }

    /// Keeps the line just opened where it was on screen while the reading
    /// pane opens beside the list, or scrolls just enough to show it whole
    /// when it no longer fits there. The lines may change height (two
    /// columns become three stacked lines), so this takes two frames: one
    /// puts the line where it was, counting the new line height, and the
    /// next, with the lines drawn at their real height, nudges it fully
    /// into view.
    fn keep_opened_line(&mut self, cx: &mut Context<Self>) {
        let Some(keep) = self.keep_line.take() else {
            return;
        };
        if keep.ix >= self.entries.len() || self.selected != Some(keep.ix) {
            return;
        }
        let view = self.list_state.viewport_bounds();
        if keep.placed {
            if let Some(bounds) = self.list_state.bounds_for_item(keep.ix) {
                if bounds.bottom() > view.bottom() {
                    self.list_state.scroll_by(
                        (bounds.bottom() - view.bottom()).min(bounds.top() - view.top()),
                    );
                } else if bounds.top() < view.top() {
                    self.list_state.scroll_by(bounds.top() - view.top());
                }
            } else {
                self.list_state.scroll_to_reveal_item(keep.ix);
            }
            return;
        }
        let height = px(self.line_height_at(keep.ix));
        let mut left = keep.top.min(view.size.height - height).max(px(0.0));
        // Walk up the lines above it, each at its own height (lines with
        // attachment chips are taller), to the one cut by the list's top.
        let mut ix = keep.ix;
        let mut offset_in_item = px(0.0);
        while left > px(0.0) && ix > 0 {
            ix -= 1;
            let above = px(self.line_height_at(ix));
            if above >= left {
                offset_in_item = above - left;
                left = px(0.0);
            } else {
                left -= above;
            }
        }
        self.list_state.scroll_to(ListOffset {
            item_ix: ix,
            offset_in_item,
        });
        self.keep_line = Some(KeepLine {
            placed: true,
            ..keep
        });
        cx.notify();
    }

    /// The list scrolled: hover stays off the lines passing under the
    /// pointer until it has been still for [`SCROLL_SETTLE`], then the line
    /// under the pointer lights up.
    pub(super) fn hold_hover_while_scrolling(&mut self, cx: &mut Context<Self>) {
        self.list_scrolling = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SCROLL_SETTLE).await;
            this.update(cx, |this, cx| {
                this.list_scrolling = None;
                cx.notify();
            })
            .ok();
        }));
    }

    /// Whether lines show as three stacked lines: the list is narrow.
    fn stacked(&self) -> bool {
        self.list_width_of(self.cards_target) < STACKED_BELOW
    }

    /// The list card's width once the reading pane is where it is going.
    pub(super) fn list_width(&self) -> f32 {
        self.list_width_of(self.cards_width)
    }

    /// The list card's width beside the reading pane in `cards` width.
    fn list_width_of(&self, cards: f32) -> f32 {
        let open = self.split() && self.reading && self.reader.is_some();
        if open {
            let pane = (cards - super::SPLIT_GAP) * self.config.mail.reading_pane_share;
            cards - pane - super::SPLIT_GAP
        } else {
            cards
        }
    }

    pub(super) fn render_list_card(&mut self, th: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let two_pane_reading = !self.split() && self.reading;
        let inner = if self.slides() {
            self.render_sliding(th, cx)
        } else {
            let (toolbar, body) = if two_pane_reading {
                (
                    self.render_reader_toolbar(th, cx),
                    self.render_reader(th, cx),
                )
            } else {
                self.render_list_parts(th, cx)
            };
            div()
                .size_full()
                .flex()
                .flex_col()
                .relative()
                .map(|d| {
                    if two_pane_reading {
                        d.child(toolbar).child(fade_in(body, self.card_seq))
                    } else {
                        // The bar floats over the top of the lines.
                        d.child(fade_in(body, self.card_seq)).child(toolbar)
                    }
                })
                .into_any_element()
        };
        // Beside a conversation, the list keeps its own keys: Up and Down
        // move in it, and the conversation has the keys once it is clicked,
        // or Tab or Enter goes to it.
        let reading_context = self.reading && !self.split();
        let (radius, outline) = (
            self.layout.shape.card_radius(),
            self.layout.shape.card_outline(),
        );
        let (shadow, edge) = self.card_edges(self.card_keys(false), outline);
        let card = div()
            .id("card")
            .key_context(if reading_context {
                READER_CONTEXT
            } else {
                LIST_CONTEXT
            })
            .track_focus(&self.list_focus)
            .size_full()
            .flex()
            .flex_col()
            .relative()
            .overflow_hidden()
            .map(|d| {
                let fill = if reading_context && self.chat_shown() {
                    th.chat_pane()
                } else {
                    th.pane()
                };
                crate::widgets::card(d, th, fill, radius, shadow)
            })
            .p(px(outline))
            // GPUI clips to rectangles, so the lines stop short of the
            // rounded bottom corners rather than showing square ones.
            .pb(px(radius.max(outline)))
            .on_action(cx.listener(Self::select_next))
            .on_action(cx.listener(Self::select_previous))
            .on_action(cx.listener(Self::select_first))
            .on_action(cx.listener(Self::select_last))
            .on_action(cx.listener(Self::page_down))
            .on_action(cx.listener(Self::page_up))
            .on_action(cx.listener(Self::open_message))
            .on_action(cx.listener(Self::close_message))
            .on_action(cx.listener(Self::scroll_down))
            .on_action(cx.listener(Self::scroll_up))
            .on_action(cx.listener(Self::scroll_page_down))
            .on_action(cx.listener(Self::scroll_page_up))
            .on_action(cx.listener(Self::archive))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::report_spam))
            .on_action(cx.listener(Self::mark_read))
            .on_action(cx.listener(Self::mark_unread))
            .on_action(cx.listener(Self::toggle_star))
            .on_action(cx.listener(Self::add_to_tasks))
            .on_action(cx.listener(Self::mark_important))
            .on_action(cx.listener(Self::toggle_mute))
            .on_action(cx.listener(Self::mark_not_important))
            .on_action(cx.listener(Self::toggle_check))
            .on_action(cx.listener(Self::open_context_menu_key))
            .on_action(cx.listener(Self::summarize_key))
            .child(inner)
            .children(card_outline(th, radius, edge));
        card.into_any_element()
    }

    /// The list's toolbar and its tabs, banner and lines.
    fn render_list_parts(
        &mut self,
        th: &Theme,
        cx: &mut Context<Self>,
    ) -> (AnyElement, AnyElement) {
        // The inbox tabs have a row of their own, centred, when the top row
        // has no room for them. A phone keeps them in its list bar, but
        // while lines are ticked that bar holds the actions.
        let phone_bar = self.layout.shape.is_phone() && self.checked.is_empty();
        let tabs = self
            .shows_tabs()
            .then(|| self.tabs_fit())
            .filter(|fit| *fit != TabsFit::TopRow && !phone_bar)
            .map(|_| {
                div()
                    .flex_none()
                    .h(px(TOOLBAR_HEIGHT))
                    .px(px(TABS_ROW_PAD))
                    .flex()
                    .items_center()
                    .justify_center()
                    .border_b_1()
                    .border_color(rgba(th.divider))
                    .child(self.render_tabs(None, th, cx))
            });
        let banner = self.render_select_banner(th, cx);
        let list = self.render_list(th, cx);
        // A phone's toolbar slides up out of sight as the list moves on.
        let toolbar = self.render_list_toolbar(th, cx);
        let rows = self.layout.shape.rows;
        let toolbar = if self.layout.shape.is_phone() && rows < 0.999 {
            div()
                .flex_none()
                .h(px(TOOLBAR_HEIGHT * rows))
                .overflow_hidden()
                .child(div().mt(px(-TOOLBAR_HEIGHT * (1.0 - rows))).child(toolbar))
                .into_any_element()
        } else {
            toolbar
        };
        // The bar, the tabs and the banner stay at the top while the lines
        // scroll under them, frosted when Blur and Frosted headers are on,
        // as the open mail's subject does.
        let under = katna_ui::unpx(self.list_state.scrolled()) > 0.5;
        let head = crate::widgets::pinned_head(
            div()
                .flex()
                .flex_col()
                .child(toolbar)
                .children(tabs)
                .children(banner),
            th.pane(),
            under,
            self.config.experimental.frosted_headers,
            self.list_head.clone(),
            th,
        );
        (
            head,
            div()
                .size_full()
                .relative()
                .child(list)
                .children(self.render_list_top(th, cx))
                .child(self.tour_mark(super::tour::Spot::List))
                .into_any_element(),
        )
    }

    /// The round button that takes the list back to its top, once it is
    /// a screen down: at the list's bottom right, above a phone's Compose
    /// button and any note at the bottom of the window.
    fn render_list_top(&self, th: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let t = self.layout.to_top_t();
        if t <= 0.001 {
            return None;
        }
        let shape = self.layout.shape;
        let snackbar = self
            .snackbar
            .as_ref()
            .map_or(0.0, |s| s.shown.value().clamp(0.0, 1.0));
        let above = 16.0 + shape.phone * (FAB_SIZE + 16.0) + 64.0 * snackbar;
        // On a phone it stands centred over the Compose button.
        let right = 16.0 + shape.phone * (FAB_SIZE - TO_TOP_SIZE) / 2.0;
        Some(
            div()
                .absolute()
                .right(px(right))
                .bottom(px(above + lerp(-12.0, 0.0, t)))
                .opacity(t)
                .child(
                    div()
                        .id("list-top")
                        // The line under it takes no hover or click.
                        .occlude()
                        .relative()
                        .overflow_hidden()
                        .size(px(TO_TOP_SIZE))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .bg(rgba(th.raised))
                        .cursor_pointer()
                        .keeps_press()
                        .shadow(elevation(th, 2.0))
                        .hover(|s| s.shadow(elevation(th, 3.0)))
                        .tooltip(tip(tr!("list-back-to-top"), th))
                        .on_mouse_move(|_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(|this, _, _, cx| this.glide_list_to_top(cx)))
                        .child(Ripple::new(("list-top", 0_usize), rgba(th.ripple)).centered())
                        .child(icon("arrow-up", th.text, 22.0)),
                )
                .into_any_element(),
        )
    }

    /// The list with the open conversation sliding in over it from the
    /// right, as on a phone; the list drifts left and dims beneath.
    fn render_sliding(&mut self, th: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let page = self.layout.shape.page;
        let shown = page.clamp(0.0, 1.0);
        let has_reader = self.reader.is_some();
        // On a card the blur shows through, the list fades out under the
        // conversation rather than showing through it.
        let see_through = th.pane_tint < 100;
        let list = (shown < 0.999 || !has_reader).then(|| {
            let (toolbar, body) = self.render_list_parts(th, cx);
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .left(relative(-0.25 * shown))
                .w_full()
                .flex()
                .flex_col()
                .child(fade_in(body, self.card_seq))
                .child(toolbar)
                .when(see_through && has_reader, |d| d.opacity(1.0 - shown))
                .when(has_reader && shown > 0.001, |d| {
                    d.child(
                        div()
                            .absolute()
                            .top_0()
                            .left_0()
                            .size_full()
                            .bg(rgba(fade(th.shadow, 0.5 * shown))),
                    )
                })
        });
        let reader = (has_reader && page > 0.001).then(|| {
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .left(relative(1.0 - page))
                .w_full()
                .flex()
                .flex_col()
                .when(!see_through, |d| d.bg(rgba(th.surface)))
                .when(shown < 0.999, |d| {
                    d.shadow(crate::widgets::elevation(th, 2.0))
                })
                .child(self.render_reader_toolbar(th, cx))
                .child(div().flex_1().min_h_0().child(self.render_reader(th, cx)))
        });
        div()
            .relative()
            .size_full()
            .overflow_hidden()
            .children(list)
            .children(reader)
            .into_any_element()
    }

    // Toolbar

    fn render_list_toolbar(&mut self, th: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let count = self.entries.len();
        let checked = self.checked.len();
        let phone = self.layout.shape.is_phone();
        if phone && checked == 0 {
            return self.render_phone_list_bar(th, cx);
        }
        let page_checked = checked > 0
            && (self.page_pick == Some(checked)
                || self.visible_keys().all(|k| self.checked.contains(&k)));
        let select_state = match checked {
            0 => crate::widgets::Check::Off,
            _ if page_checked || self.checked_all => crate::widgets::Check::On,
            _ => crate::widgets::Check::Partial,
        };
        let select_radius =
            (self.layout.shape.card_radius() - SELECT_PILL_GAP).max(SELECT_PILL_GAP);
        let select = div()
            .id("select")
            .flex()
            .flex_row()
            .items_center()
            // As tall as the round buttons beside it, as far from the
            // card's edge as from its top, its corners the card's corner
            // less that gap, so the two curves nest.
            .h(px(40.0))
            .ml(px(SELECT_PILL_GAP - 8.0))
            .pl(px((40.0 - 28.0) / 2.0))
            .pr(px(4.0))
            .relative()
            .rounded(px(select_radius))
            .child(crate::widgets::hover_fade(
                "select-glow",
                Some(select_radius),
                th,
            ))
            .child(
                div()
                    .id("select-box")
                    .tooltip(tip(tr!("list-select"), th))
                    .size(px(28.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .keeps_press()
                    .on_click(cx.listener(|this, _, _, cx| {
                        let pick = if this.checked.is_empty() {
                            Pick::All
                        } else {
                            Pick::None
                        };
                        this.pick(pick, cx);
                    }))
                    .child(crate::widgets::checkbox_colored(
                        "select-all-box",
                        select_state,
                        th.text,
                        th,
                    )),
            )
            .child(
                div()
                    .id("select-menu")
                    .cursor_pointer()
                    .keeps_press()
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_menu(Menu::Select, cx)))
                    .child(icon("drop-down", th.text_dim, 20.0)),
            );
        let select = self.with_menu(select, Menu::Select, th, cx);
        let mut bar = toolbar(th).child(select);
        if checked == 0 {
            bar = bar.child(self.refresh_button("refresh", th, cx));
            bar = bar.children(self.quiet_button(th, cx)).child({
                let more = icon_button("list-more", "more", 20.0, th)
                    .when(self.menu != Some(Menu::ListMore), |d| {
                        d.tooltip(tip(tr!("list-more"), th))
                    })
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_menu(Menu::ListMore, cx)));
                self.with_menu(more, Menu::ListMore, th, cx)
            });
        } else {
            let any_unread = self.checked_rows().iter().any(|r| r.unread);
            let read_button = if any_unread {
                icon_button("mark-read", "mark-read", 20.0, th)
                    .tooltip(tip(tr!("list-mark-read"), th))
                    .on_click(
                        cx.listener(|this, _, _, cx| this.act_on_targets(Act::Read(true), cx)),
                    )
            } else {
                icon_button("mark-unread", "mark-unread", 20.0, th)
                    .tooltip(tip(tr!("list-mark-unread"), th))
                    .on_click(
                        cx.listener(|this, _, _, cx| this.act_on_targets(Act::Read(false), cx)),
                    )
            };
            bar = bar
                .child(self.action_buttons("list", Squeeze::NONE, th, cx))
                .child(separator(th))
                .child(read_button)
                .child({
                    let move_to = icon_button("list-move", "move-to", 20.0, th)
                        .when(self.menu != Some(Menu::MoveTo), |d| {
                            d.tooltip(tip(tr!("list-move-to"), th))
                        })
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_menu(Menu::MoveTo, cx)));
                    self.with_menu(move_to, Menu::MoveTo, th, cx)
                })
                .when(
                    self.account().is_some_and(|a| self.tree.is_gmail(a)),
                    |bar| {
                        let label_as = icon_button("list-label-as", "tag", 20.0, th)
                            .when(self.menu != Some(Menu::LabelAs), |d| {
                                d.tooltip(tip(tr!("menu-label-as"), th))
                            })
                            .on_click(
                                cx.listener(|this, _, _, cx| this.toggle_menu(Menu::LabelAs, cx)),
                            );
                        bar.child(self.with_menu(label_as, Menu::LabelAs, th, cx))
                    },
                )
                .child({
                    let more = icon_button("list-more", "more", 20.0, th)
                        .when(self.menu != Some(Menu::ListMore), |d| {
                            d.tooltip(tip(tr!("list-more"), th))
                        })
                        .on_click(
                            cx.listener(|this, _, _, cx| this.toggle_menu(Menu::ListMore, cx)),
                        );
                    self.with_menu(more, Menu::ListMore, th, cx)
                });
        }
        let label: Option<SharedString> = match (&self.search_error, &self.listing) {
            (Some(err), _) => Some(err.clone()),
            (
                None,
                Some(Listing::Search {
                    corrected: Some(corrected),
                    ..
                }),
            ) => Some(tr!("list-results-corrected", query = corrected.as_str()).into()),
            (None, Some(Listing::Search { query, .. })) => {
                Some(tr!("list-results", query = query.as_str()).into())
            }
            _ => None,
        };
        let range = if count == 0 {
            String::new()
        } else {
            let start = self.visible.start.min(count - 1) + 1;
            let end = self.visible.end.clamp(start, count);
            match &self.listing {
                Some(Listing::Search {
                    total: Some(total), ..
                }) if *total > count && !self.config.mail.conversations => tr!(
                    "list-range-about",
                    first = start as u64,
                    last = end as u64,
                    total = *total as u64
                ),
                _ => tr!(
                    "list-range",
                    first = start as u64,
                    last = end as u64,
                    total = count as u64
                ),
            }
        };
        let search_instead = match (&self.search_error, &self.listing) {
            (
                None,
                Some(Listing::Search {
                    query,
                    corrected: Some(_),
                    ..
                }),
            ) => Some(query.clone()),
            _ => None,
        };
        let at_top = self.visible.start == 0;
        let at_end = self.visible.end >= count;
        if phone {
            return bar.child(div().flex_1()).into_any_element();
        }
        // The tabs sit in the middle of the row, or as near it as the
        // buttons on either side let them.
        if self.shows_tabs() && self.tabs_fit() == TabsFit::TopRow {
            let width = self.list_width();
            let tabs = self.tabs_width(self.tab_fold.value(), false);
            let left = ((width - tabs) / 2.0)
                .min(width - TOP_ROW_RIGHT - tabs)
                .max(TOP_ROW_LEFT);
            bar = bar.relative().child(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(px(left))
                    .flex()
                    .items_center()
                    .child(self.render_tabs(None, th, cx)),
            );
        }
        bar.child(
            div()
                .pl(px(8.0))
                .flex_1()
                .min_w_0()
                .flex()
                .items_center()
                .gap(px(12.0))
                .text_size(px(14.0))
                .child(
                    div()
                        .flex_shrink(1.0)
                        .min_w_0()
                        .truncate()
                        .text_color(rgba(th.text_dim))
                        .children(label),
                )
                .children(search_instead.map(|query| {
                    div()
                        .id("search-instead")
                        .flex_shrink(1.0)
                        .min_w_0()
                        .truncate()
                        .cursor_pointer()
                        .keeps_press()
                        .text_color(rgba(th.accent))
                        .child(tr!("list-search-instead", query = query.as_str()))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.search_verbatim(query.clone(), cx);
                        }))
                })),
        )
        .child(
            div()
                .flex_none()
                .px(px(8.0))
                .text_size(px(12.0))
                .text_color(rgba(th.text_faint))
                .child(range),
        )
        .child(
            icon_button("page-up", "chevron-left", 20.0, th)
                .tooltip(tip(tr!("list-newer"), th))
                .when(at_top, |d| d.opacity(0.4))
                .on_click(cx.listener(|this, _, _, cx| {
                    let page = this.visible.len().max(1);
                    let ix = this.visible.start.saturating_sub(page);
                    this.scroll_list_to(ix);
                    cx.notify();
                })),
        )
        .child(
            icon_button("page-down", "chevron-right", 20.0, th)
                .tooltip(tip(tr!("list-older"), th))
                .when(at_end, |d| d.opacity(0.4))
                .on_click(cx.listener(|this, _, _, cx| {
                    let ix = this.visible.end.min(this.entries.len().saturating_sub(1));
                    this.scroll_list_to(ix);
                    cx.notify();
                })),
        )
        .into_any_element()
    }

    /// A phone's bar over the list: what the list shows, refresh and more.
    /// Ticking a line (on its picture) brings the actions.
    /// Whether a phone's list bar shows the inbox tabs.
    fn phone_bar_tabs(&self) -> bool {
        self.layout.shape.is_phone()
            && self.checked.is_empty()
            && self.shows_tabs()
            && self.search_error.is_none()
            && !matches!(self.listing, Some(Listing::Search { .. }))
    }

    fn render_phone_list_bar(&mut self, th: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let label: SharedString = match (&self.search_error, &self.listing) {
            (Some(err), _) => err.clone(),
            (None, Some(Listing::Search { query, .. })) => {
                tr!("list-results", query = query.as_str()).into()
            }
            _ if self.shows_tabs() => self
                .tabs
                .get(self.tab)
                .map_or_else(SharedString::default, |t| t.label().into()),
            _ => self.folder_name().unwrap_or_default().into(),
        };
        let more = icon_button("list-more", "more", 20.0, th)
            .when(self.menu != Some(Menu::ListMore), |d| {
                d.tooltip(tip(tr!("list-more"), th))
            })
            .on_click(cx.listener(|this, _, _, cx| this.toggle_menu(Menu::ListMore, cx)));
        // The tabs take the place of the title, which would only repeat
        // the open tab's name, and Refresh goes into the More menu, whose
        // button ends the tabs' pill. The pill spans the bar, its tabs
        // sharing the room, so they are easy to tap.
        if self.phone_bar_tabs() {
            let more = more.size(px(TAB_HEIGHT)).rounded_full().ml(px(TAB_SPACING));
            let fill = self.tabs_room(TabsFit::Row);
            return toolbar(th)
                .px(px(PHONE_TABS_SIDE))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .h(px(TABS_HEIGHT))
                        .pr(px(TABS_INSET))
                        .flex()
                        .flex_row()
                        .items_center()
                        .rounded_full()
                        .bg(rgba(th.search))
                        .child(
                            div()
                                .id("phone-tabs")
                                .min_w_0()
                                .overflow_x_scroll()
                                .child(self.render_tabs(Some(fill), th, cx)),
                        )
                        .child(self.with_menu(more, Menu::ListMore, th, cx)),
                )
                .into_any_element();
        }
        toolbar(th)
            .pl(px(16.0))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(14.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgba(th.text_dim))
                    .child(label),
            )
            .child(self.refresh_button("refresh", th, cx))
            .child(self.with_menu(more, Menu::ListMore, th, cx))
            .into_any_element()
    }

    /// Archive, spam and delete, for the ticked lines or the open
    /// conversation.
    /// Refresh, whose arrow turns while mail is being checked for.
    fn refresh_button(
        &self,
        id: &'static str,
        th: &Theme,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        if !self.checking_mail() {
            return icon_button(id, "refresh", 20.0, th)
                .tooltip(tip(tr!("list-refresh"), th))
                .on_click(cx.listener(|this, _, window, cx| this.reload(&Reload, window, cx)));
        }
        // Like `icon_button`, with the arrow turning.
        div()
            .id(id)
            .size(px(40.0))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .relative()
            .rounded_full()
            .child(crate::widgets::hover_fade("refresh-glow", None, th))
            .tooltip(tip(tr!("list-checking"), th))
            .child(super::nav_menu::turning_arrow(
                "refresh-turning",
                th.text_dim,
                20.0,
            ))
    }

    /// Archive, Report spam and Delete, less those `squeeze` leaves to
    /// the More menu.
    pub(super) fn action_buttons(
        &self,
        prefix: &'static str,
        squeeze: Squeeze,
        th: &Theme,
        cx: &mut Context<Self>,
    ) -> Div {
        div()
            .flex()
            .flex_row()
            .child(
                icon_button((prefix, 1_usize), "archive", 20.0, th)
                    .tooltip(tip(tr!("list-archive"), th))
                    .on_click(cx.listener(|this, _, _, cx| this.act_on_targets(Act::Archive, cx))),
            )
            .when(!squeeze.spam, |d| {
                d.child(
                    icon_button((prefix, 2_usize), "junk", 20.0, th)
                        .tooltip(tip(self.spam_label(false), th))
                        .on_click(cx.listener(|this, _, _, cx| this.act_on_targets(Act::Spam, cx))),
                )
            })
            .when(!squeeze.delete, |d| {
                d.child(
                    icon_button((prefix, 3_usize), "trash", 20.0, th)
                        .tooltip(tip(tr!("list-delete"), th))
                        .on_click(
                            cx.listener(|this, _, _, cx| this.act_on_targets(Act::Delete, cx)),
                        ),
                )
            })
    }

    /// Tells the list its lines changed; `keep_scroll` stays at the same
    /// line, otherwise it goes back to the top.
    pub(super) fn reset_list(&mut self, keep_scroll: bool) {
        let top = self.list_state.logical_scroll_top();
        let count = self.entries.len();
        self.list_state
            .reset(count, px(self.row_height()), keep_scroll.then_some(top));
        self.files_menu = None;
    }

    /// Scrolls the list so line `ix` is at the top.
    pub(super) fn scroll_list_to(&self, ix: usize) {
        self.list_state.scroll_to(ListOffset {
            item_ix: ix,
            offset_in_item: px(0.0),
        });
    }

    /// Works out which lines the list showed in its last frame.
    fn update_visible(&mut self) {
        let count = self.entries.len();
        let start = self.list_state.logical_scroll_top().item_ix.min(count);
        let viewport = self.list_state.viewport_bounds();
        let mut end = start;
        while end < count {
            match self.list_state.bounds_for_item(end) {
                Some(bounds) if bounds.top() < viewport.bottom() => end += 1,
                Some(_) => break,
                None => {
                    // Not drawn yet: as many as fit at the usual height.
                    let fit = (viewport.size.height / px(self.row_height())).ceil() as usize;
                    end = end.max(start + fit.max(1)).min(count);
                    break;
                }
            }
        }
        self.visible = start..end;
    }

    pub(super) fn toggle_menu(&mut self, menu: Menu, cx: &mut Context<Self>) {
        self.menu = if self.menu == Some(menu) {
            None
        } else {
            Some(menu)
        };
        self.sync_folder_pick(cx);
        cx.notify();
    }

    /// Notes a popup menu that closed since the last frame, however it
    /// closed, so `with_menu` fades it out (`motion` FAST), and forgets
    /// it once faded. Move to and Label as are left out: their folder
    /// search is gone once they close.
    pub(super) fn track_menu_fade(&mut self, cx: &mut Context<Self>) {
        let fade = katna_ui::motion::time(duration::FAST);
        if self.menu != self.menu_was {
            if let Some(was) = self.menu_was
                && !matches!(was, Menu::MoveTo | Menu::LabelAs)
                && !cx.reduce_motion()
            {
                self.menu_fade = Some((was, Instant::now()));
                // One more frame once it has faded, to take it away.
                cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(fade).await;
                    this.update(cx, |_, cx| cx.notify()).ok();
                })
                .detach();
            }
            self.menu_was = self.menu;
        }
        if let Some((which, since)) = self.menu_fade
            && (self.menu == Some(which) || since.elapsed() >= fade)
        {
            self.menu_fade = None;
        }
    }

    /// Puts `anchor` in a box that also holds `which` menu when it is open,
    /// drawn over everything, with a scrim that closes it on a click
    /// elsewhere.
    pub(super) fn with_menu(
        &self,
        anchor: impl IntoElement,
        which: Menu,
        th: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let open = self.menu == Some(which);
        let fading = !open && self.menu_fade.is_some_and(|(m, _)| m == which);
        div()
            .relative()
            .child(anchor)
            // Closed: it fades where it was, out of reach.
            .when(fading, |d| {
                let items = self
                    .menu_items(which, th, cx)
                    .child(div().absolute().top_0().left_0().size_full().occlude());
                d.child(
                    deferred(
                        anchored()
                            .offset(point(px(0.0), px(4.0)))
                            .snap_to_window_with_margin(px(8.0))
                            .child(
                                items.with_animation(
                                    ("menu-out", which as usize),
                                    Animation::new(katna_ui::motion::time(duration::FAST))
                                        .with_easing(ease_out_quint()),
                                    |el, t| el.opacity(1.0 - t),
                                ),
                            ),
                    )
                    .with_priority(2),
                )
            })
            .when(open, |d| {
                let items = self.menu_items(which, th, cx);
                d.child(
                    deferred(
                        div()
                            .id("menu-scrim")
                            .absolute()
                            .top(px(-2000.0))
                            .left(px(-4000.0))
                            .w(px(8000.0))
                            .h(px(6000.0))
                            .occlude()
                            .on_mouse_down(
                                gpui::MouseButton::Left,
                                cx.listener(|this, _, _, cx| {
                                    this.menu = None;
                                    cx.notify();
                                }),
                            ),
                    )
                    .with_priority(1),
                )
                .child(
                    // Just under the button (where the button's row puts
                    // it), moved back inside the window when it would run
                    // past an edge (a phone's narrow window).
                    deferred(
                        anchored()
                            .offset(point(px(0.0), px(4.0)))
                            .snap_to_window_with_margin(px(8.0))
                            .child(
                                div().occlude().child(
                                    items.with_animation(
                                        ("menu", which as usize),
                                        Animation::new(katna_ui::motion::time(
                                            Duration::from_millis(160),
                                        ))
                                        .with_easing(ease_out_quint()),
                                        |el, t| el.opacity(t).mt(px(-6.0 * (1.0 - t))),
                                    ),
                                ),
                            ),
                    )
                    .with_priority(2),
                )
            })
            .into_any_element()
    }

    fn menu_items(&self, which: Menu, th: &Theme, cx: &mut Context<Self>) -> Div {
        match which {
            Menu::CalendarOptions => self.calendar_options_menu(th, cx),
            Menu::CalendarZones => self.calendar_zones_menu(th, cx),
            Menu::CalendarViews => self.calendar_views_menu(th, cx),
            Menu::Quiet => self.quiet_toolbar_items(th, cx),
            Menu::Select => menu(th).children(
                [
                    (Pick::All, tr!("list-pick-all")),
                    (Pick::None, tr!("list-pick-none")),
                    (Pick::Read, tr!("list-pick-read")),
                    (Pick::Unread, tr!("list-pick-unread")),
                    (Pick::Starred, tr!("list-pick-starred")),
                    (Pick::Unstarred, tr!("list-pick-unstarred")),
                ]
                .into_iter()
                .map(|(pick, label)| {
                    menu_item(("pick", pick as usize), &label, th)
                        .on_click(cx.listener(move |this, _, _, cx| this.pick(pick, cx)))
                }),
            ),
            Menu::ListMore | Menu::ReaderMore => {
                let squeeze = (which == Menu::ReaderMore).then(|| self.reader_squeeze(th));
                let targets = if which == Menu::ListMore && self.checked.is_empty() {
                    None
                } else {
                    Some(())
                };
                match targets {
                    None => menu(th)
                        .when(which == Menu::ListMore && self.phone_bar_tabs(), |d| {
                            d.child(
                                menu_item_icon("more-refresh", "refresh", &tr!("list-refresh"), th)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.menu = None;
                                        this.reload(&Reload, window, cx);
                                    })),
                            )
                        })
                        .child(
                            menu_item_icon(
                                "mark-all-read",
                                "mark-read",
                                &tr!("menu-mark-all-read"),
                                th,
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                let keys = this.entries.iter().map(|e| e.key).collect();
                                this.act(Act::Read(true), keys, cx);
                            })),
                        ),
                    Some(()) => menu(th)
                        // What a narrow reading pane leaves off its toolbar.
                        .when(
                            squeeze.is_some_and(|s| s.summary)
                                && self.summaries_on()
                                && !self.chat_shown(),
                            |d| {
                                d.child(
                                    menu_item_icon(
                                        "more-summary",
                                        "sparkle",
                                        &tr!("summary-summarize"),
                                        th,
                                    )
                                    .on_click(cx.listener(
                                        |this, _, _, cx| {
                                            this.menu = None;
                                            this.toggle_summary(cx);
                                        },
                                    )),
                                )
                            },
                        )
                        .when(squeeze.is_some_and(|s| s.archive), |d| {
                            d.child(
                                menu_item_icon("more-archive", "archive", &tr!("list-archive"), th)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.act_on_targets(Act::Archive, cx)
                                    })),
                            )
                        })
                        .when(squeeze.is_some_and(|s| s.spam), |d| {
                            d.child(
                                menu_item_icon("more-spam", "junk", &self.spam_label(true), th)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.act_on_targets(Act::Spam, cx)
                                    })),
                            )
                        })
                        .when(squeeze.is_some_and(|s| s.delete), |d| {
                            d.child(
                                menu_item_icon("more-delete", "trash", &tr!("menu-delete"), th)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.act_on_targets(Act::Delete, cx)
                                    })),
                            )
                        })
                        .when(squeeze.is_some_and(|s| s.move_to), |d| {
                            d.child(
                                menu_item_icon("more-move-to", "move-to", &tr!("menu-move-to"), th)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.menu = Some(Menu::MoveTo);
                                        this.sync_folder_pick(cx);
                                        cx.notify();
                                    })),
                            )
                        })
                        .when(
                            squeeze.is_some_and(|s| s.move_to)
                                && self.account().is_some_and(|a| self.tree.is_gmail(a)),
                            |d| {
                                d.child(
                                    menu_item_icon(
                                        "more-label-as",
                                        "tag",
                                        &tr!("menu-label-as"),
                                        th,
                                    )
                                    .on_click(cx.listener(
                                        |this, _, _, cx| {
                                            this.menu = Some(Menu::LabelAs);
                                            this.sync_folder_pick(cx);
                                            cx.notify();
                                        },
                                    )),
                                )
                            },
                        )
                        .child(
                            menu_item_icon("more-read", "mark-read", &tr!("menu-mark-read"), th)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.act_on_targets(Act::Read(true), cx)
                                })),
                        )
                        .child(
                            menu_item_icon(
                                "more-unread",
                                "mark-unread",
                                &tr!("menu-mark-unread"),
                                th,
                            )
                            .on_click(cx.listener(
                                |this, _, window, cx| {
                                    this.mark_unread(&super::MarkUnread, window, cx)
                                },
                            )),
                        )
                        .child(
                            menu_item_icon("more-star", "star", &tr!("menu-star"), th).on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.act_on_targets(Act::Star(true), cx)
                                }),
                            ),
                        )
                        .child(
                            menu_item_icon("more-unstar", "star-filled", &tr!("menu-unstar"), th)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.act_on_targets(Act::Star(false), cx)
                                })),
                        )
                        .child(
                            menu_item_icon(
                                "more-important",
                                "important",
                                &tr!("menu-important"),
                                th,
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.act_on_targets(Act::Important(true), cx)
                            })),
                        )
                        .child(
                            menu_item_icon(
                                "more-not-important",
                                "important-filled",
                                &tr!("menu-not-important"),
                                th,
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.act_on_targets(Act::Important(false), cx)
                            })),
                        )
                        .child(
                            menu_item_icon(
                                "more-add-to-tasks",
                                "tasks",
                                &tr!("menu-add-to-tasks"),
                                th,
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.menu = None;
                                let keys = this.target_keys();
                                this.add_to_tasks_from(keys, cx);
                            })),
                        )
                        .child(
                            menu_item_icon(
                                "more-schedule-meeting",
                                "calendar",
                                &tr!("menu-schedule-meeting"),
                                th,
                            )
                            .on_click(cx.listener(
                                |this, _, window, cx| {
                                    this.menu = None;
                                    let key = this.target_keys().first().copied();
                                    this.schedule_meeting_from(key, window, cx);
                                },
                            )),
                        )
                        .child(
                            menu_item_icon("more-start-call", "video", &tr!("menu-start-call"), th)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.menu = None;
                                    let key = this.target_keys().first().copied();
                                    this.start_call_from(key, window, cx);
                                })),
                        )
                        .child(
                            menu_item_icon("more-add-note", "notes", &tr!("menu-add-note"), th)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.menu = None;
                                    let keys = this.target_keys();
                                    this.add_note_from(keys, window, cx);
                                })),
                        )
                        .child(
                            menu_item_icon("more-pin", "pin", &tr!("menu-pin"), th).on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.act_on_targets(Act::Pin(true), cx)
                                }),
                            ),
                        )
                        .child(
                            menu_item_icon("more-unpin", "pin-filled", &tr!("menu-unpin"), th)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.act_on_targets(Act::Pin(false), cx)
                                })),
                        )
                        .child(self.mute_menu_items(
                            which != Menu::ReaderMore || squeeze.is_some_and(|s| s.mute),
                            which == Menu::ReaderMore,
                            th,
                            cx,
                        ))
                        .when(which == Menu::ReaderMore, |d| {
                            d.child(div().my(px(6.0)).h(px(1.0)).bg(rgba(th.divider)))
                                .child(
                                    menu_item_icon(
                                        "more-print",
                                        "print",
                                        &tr!("menu-print-all"),
                                        th,
                                    )
                                    .on_click(cx.listener(
                                        |this, _, window, cx| {
                                            this.menu = None;
                                            this.print_conversation(window, cx);
                                        },
                                    )),
                                )
                                // A phone's chat has no Chat | Mail switch.
                                .when(squeeze.is_some_and(|s| s.archive), |d| {
                                    d.child(
                                        menu_item_icon(
                                            "more-show-mail",
                                            "mail",
                                            &tr!("chat-show-as-mail"),
                                            th,
                                        )
                                        .on_click(
                                            cx.listener(|this, _, window, cx| {
                                                this.menu = None;
                                                this.pick_chat(false, window, cx);
                                            }),
                                        ),
                                    )
                                })
                                .when(!self.detached, |d| {
                                    d.child(
                                        menu_item_icon(
                                            "more-new-window",
                                            "open-external",
                                            &tr!("menu-new-window"),
                                            th,
                                        )
                                        .on_click(
                                            cx.listener(|this, _, _, cx| {
                                                this.menu = None;
                                                this.open_reader_in_window(cx);
                                            }),
                                        ),
                                    )
                                })
                                .when(
                                    squeeze.is_some_and(|s| s.contact) && self.contact_offered(),
                                    |d| {
                                        let on = self.config.mail.contact_panel;
                                        d.child(
                                            menu_item_icon(
                                                "more-contact",
                                                "contacts",
                                                &if on {
                                                    tr!("contact-panel-hide")
                                                } else {
                                                    tr!("contact-panel-show")
                                                },
                                                th,
                                            )
                                            .on_click(
                                                cx.listener(|this, _, _, cx| {
                                                    this.menu = None;
                                                    this.toggle_contact_panel(cx);
                                                }),
                                            ),
                                        )
                                    },
                                )
                                .when(
                                    squeeze.is_some_and(|s| s.colors)
                                        && self.original_colors_offered(th),
                                    |d| {
                                        let on =
                                            self.reader.as_ref().is_some_and(|r| r.original_colors);
                                        d.child(
                                            menu_item_icon(
                                                "more-colors",
                                                "contrast",
                                                &if on {
                                                    tr!("reader-dark-colors")
                                                } else {
                                                    tr!("reader-original-colors")
                                                },
                                                th,
                                            )
                                            .on_click(
                                                cx.listener(|this, _, _, cx| {
                                                    this.menu = None;
                                                    if let Some(reader) = &mut this.reader {
                                                        reader.original_colors =
                                                            !reader.original_colors;
                                                    }
                                                    cx.notify();
                                                }),
                                            ),
                                        )
                                    },
                                )
                        }),
                }
            }
            Menu::MoveTo | Menu::LabelAs => {
                let mode = if which == Menu::LabelAs {
                    PickMode::Label
                } else {
                    PickMode::Move
                };
                let pick = self.folder_pick_in(PickFrom::Toolbar, mode);
                let rows = pick
                    .map(|pick| self.render_folder_pick(pick, 32.0, th, cx))
                    .unwrap_or_default();
                let always = pick
                    .and_then(|pick| self.render_always_move(pick, th, cx))
                    .map(|(el, _)| el);
                let mut rows = rows.into_iter().map(|(el, _)| el);
                menu(th)
                    .w(px(260.0))
                    .children(rows.next())
                    .child(
                        div()
                            .id("move-to-list")
                            .max_h(px(360.0))
                            .overflow_y_scroll()
                            .flex()
                            .flex_col()
                            .children(rows),
                    )
                    .children(always)
            }
        }
    }

    /// Ticks or unticks line `ix` and makes it where Shift+click ticks
    /// from. With `with_open`, the open line is ticked too when nothing is
    /// ticked yet, so Ctrl+click on a second line selects both.
    pub(super) fn click_check(&mut self, ix: usize, with_open: bool, cx: &mut Context<Self>) {
        let Some(key) = self.entries.get(ix).map(|e| e.key) else {
            return;
        };
        if with_open
            && self.checked.is_empty()
            && let Some(entry) = self
                .selected
                .filter(|&open| open != ix)
                .and_then(|open| self.entries.get(open))
        {
            self.checked.insert(entry.key);
        }
        if !self.checked.remove(&key) {
            self.checked.insert(key);
        }
        self.check_anchor = Some(ix);
        self.checked_all = false;
        self.page_pick = None;
        self.picked = None;
        cx.notify();
    }

    /// Ticks every line from the last one clicked (or the open line) to
    /// line `ix`.
    fn check_range(&mut self, ix: usize, cx: &mut Context<Self>) {
        let from = self.check_anchor.or(self.selected).unwrap_or(ix);
        let (lo, hi) = (
            from.min(ix),
            from.max(ix).min(self.entries.len().saturating_sub(1)),
        );
        if lo > hi {
            return;
        }
        self.checked
            .extend(self.entries[lo..=hi].iter().map(|e| e.key));
        self.check_anchor = Some(ix);
        self.checked_all = false;
        self.page_pick = None;
        self.picked = None;
        cx.notify();
    }

    /// Keys of the lines on screen.
    fn visible_keys(&self) -> impl Iterator<Item = EntryKey> + '_ {
        let range =
            self.visible.start.min(self.entries.len())..self.visible.end.min(self.entries.len());
        self.entries[range].iter().map(|e| e.key)
    }

    /// Rows of the ticked lines that are loaded.
    fn checked_rows(&mut self) -> Vec<Rc<Row>> {
        let entries: Vec<_> = self
            .entries
            .iter()
            .filter(|e| self.checked.contains(&e.key))
            .take(500)
            .copied()
            .collect();
        let folder = self.listed_folder();
        match &mut self.mail {
            Ok(mail) => mail
                .rows(&entries, folder, self.show_recipients)
                .into_iter()
                .flatten()
                .map(|r| self.with_pending(r))
                .collect(),
            Err(_) => Vec::new(),
        }
    }

    /// `row` with changes the daemon has not confirmed yet.
    pub(super) fn with_pending(&self, row: Rc<Row>) -> Rc<Row> {
        match self.pending.get(&row.key) {
            Some(p)
                if p.unread.is_some()
                    || p.flagged.is_some()
                    || p.important.is_some()
                    || p.pinned.is_some() =>
            {
                let mut row = (*row).clone();
                row.unread = p.unread.unwrap_or(row.unread);
                row.flagged = p.flagged.unwrap_or(row.flagged);
                row.important = p.important.unwrap_or(row.important);
                row.pinned = p.pinned.unwrap_or(row.pinned);
                Rc::new(row)
            }
            _ => row,
        }
    }

    fn pick(&mut self, pick: Pick, cx: &mut Context<Self>) {
        self.menu = None;
        self.checked_all = false;
        self.page_pick = None;
        self.picked = None;
        self.checked.clear();
        self.check_anchor = None;
        match pick {
            Pick::None => {}
            // The lines on screen; the banner offers the whole list.
            Pick::All => {
                let range = self.visible.start.min(self.entries.len())
                    ..self.visible.end.min(self.entries.len());
                self.checked
                    .extend(self.entries[range].iter().map(|e| e.key));
                if self.checked.len() < self.entries.len() {
                    self.page_pick = Some(self.checked.len());
                } else if !self.checked.is_empty() {
                    self.checked_all = true;
                }
            }
            // The matching lines on screen; the banner offers every
            // matching line of the list, loaded or not.
            Pick::Read | Pick::Unread | Pick::Starred | Pick::Unstarred => {
                let folder = self.listed_folder();
                let show_recipients = self.show_recipients;
                let marks = match &mut self.mail {
                    Ok(mail) => mail.marks(&self.entries, folder, show_recipients),
                    Err(_) => Vec::new(),
                };
                let visible = self.visible.start.min(self.entries.len())
                    ..self.visible.end.min(self.entries.len());
                let mut all = Vec::new();
                for (ix, (entry, marks)) in self.entries.iter().zip(marks).enumerate() {
                    let pending = self.pending.get(&entry.key);
                    let unread = pending.and_then(|p| p.unread).unwrap_or(marks.unread);
                    let flagged = pending.and_then(|p| p.flagged).unwrap_or(marks.flagged);
                    let take = match pick {
                        Pick::Read => !unread,
                        Pick::Unread => unread,
                        Pick::Starred => flagged,
                        _ => !flagged,
                    };
                    if take {
                        all.push(entry.key);
                        if visible.contains(&ix) {
                            self.checked.insert(entry.key);
                        }
                    }
                }
                if all.is_empty() {
                    self.show_snackbar(
                        pick_none_text(pick, self.config.mail.conversations),
                        None,
                        cx,
                    );
                } else {
                    // None on screen: nothing to offer beyond, so take
                    // them all at once.
                    let whole = self.checked.is_empty() || self.checked.len() == all.len();
                    if whole {
                        self.checked.extend(all.iter().copied());
                    }
                    self.picked = Some(Picked {
                        pick,
                        on_screen: self.checked.len(),
                        all,
                        whole,
                    });
                }
            }
        }
        cx.notify();
    }

    /// "All 20 on screen are selected. Select all 1,234" when every line on
    /// screen is ticked and there are more.
    fn render_select_banner(&self, th: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let on_screen = self.checked.len();
        let page_checked = on_screen > 0 && self.page_pick == Some(on_screen);
        let picked = self
            .picked
            .as_ref()
            .filter(|p| p.ticked() > 0 && p.ticked() == on_screen);
        if !(self.checked_all || page_checked || picked.is_some()) {
            return None;
        }
        let kind = if self.config.mail.conversations {
            "conversation"
        } else {
            "message"
        };
        let folder = self.folder_name();
        let total = self.entries.len() as u64;
        let (text, link) = match (self.checked_all, folder) {
            (false, folder) if let Some(picked) = picked => {
                let pick = pick_name(picked.pick);
                let all = picked.all.len() as u64;
                match (picked.whole, folder) {
                    (true, Some(folder)) => (
                        tr!(
                            "list-selected-picked-in",
                            pick = pick,
                            count = all,
                            kind = kind,
                            folder = folder
                        ),
                        tr!("list-clear-selection"),
                    ),
                    (true, None) => (
                        tr!(
                            "list-selected-picked",
                            pick = pick,
                            count = all,
                            kind = kind
                        ),
                        tr!("list-clear-selection"),
                    ),
                    (false, folder) => (
                        tr!(
                            "list-selected-picked-screen",
                            pick = pick,
                            count = on_screen as u64,
                            kind = kind
                        ),
                        match folder {
                            Some(folder) => tr!(
                                "list-select-picked-in",
                                pick = pick,
                                count = all,
                                kind = kind,
                                folder = folder
                            ),
                            None => {
                                tr!("list-select-picked", pick = pick, count = all, kind = kind)
                            }
                        },
                    ),
                }
            }
            (true, Some(folder)) => (
                tr!(
                    "list-selected-all-in",
                    count = total,
                    kind = kind,
                    folder = folder
                ),
                tr!("list-clear-selection"),
            ),
            (true, None) => (
                tr!("list-selected-all", count = total, kind = kind),
                tr!("list-clear-selection"),
            ),
            (false, folder) => (
                tr!(
                    "list-selected-screen",
                    count = on_screen as u64,
                    kind = kind
                ),
                match folder {
                    Some(folder) => tr!(
                        "list-select-all-in",
                        count = total,
                        kind = kind,
                        folder = folder
                    ),
                    None => tr!("list-select-all", count = total, kind = kind),
                },
            ),
        };
        Some(
            div()
                .flex_none()
                .h(px(40.0))
                .flex()
                .flex_row()
                .items_center()
                .justify_center()
                .gap(px(8.0))
                .bg(rgba(th.on_pane(th.read_row)))
                .border_b_1()
                .border_color(rgba(th.divider))
                .text_size(px(13.0))
                .child(text)
                .child(
                    div()
                        .id("select-all")
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgba(th.accent))
                        .cursor_pointer()
                        .keeps_press()
                        .on_click(cx.listener(|this, _, _, cx| {
                            let ticked = this.checked.len();
                            let picked = this
                                .picked
                                .as_ref()
                                .filter(|p| p.ticked() == ticked)
                                .map(|p| p.whole);
                            if picked == Some(false)
                                && let Some(picked) = this.picked.as_mut()
                            {
                                // "Select all 2,000 unread conversations".
                                this.checked.extend(picked.all.iter().copied());
                                picked.whole = true;
                            } else if this.checked_all || picked.is_some() {
                                this.checked.clear();
                                this.checked_all = false;
                                this.page_pick = None;
                                this.picked = None;
                            } else {
                                this.checked = this.entries.iter().map(|e| e.key).collect();
                                this.checked_all = true;
                            }
                            cx.notify();
                        }))
                        .child(link),
                )
                .into_any_element(),
        )
    }

    // Tabs

    /// Measures the inbox tabs' labels and unread counts in the window's
    /// font, for the pill bar to lay them out.
    pub(super) fn measure_tabs(&mut self, window: &gpui::Window) {
        let sizes = self
            .tabs
            .iter()
            .map(|tab| {
                let label = super::text_width(
                    &tab.label(),
                    TAB_TEXT,
                    FontWeight::MEDIUM,
                    self.font.as_ref(),
                    window,
                );
                let unread = self.tab_unread(tab);
                let badge = if unread == 0 {
                    0.0
                } else {
                    let number = format::thousands(unread);
                    let text = super::text_width(
                        &number,
                        BADGE_TEXT,
                        FontWeight::BOLD,
                        self.font.as_ref(),
                        window,
                    );
                    (text + 2.0 * BADGE_PAD).max(BADGE_HEIGHT)
                };
                (label, badge)
            })
            .collect();
        self.tab_sizes = sizes;
    }

    /// Moves each inbox tab's chip toward how it should look: shown while
    /// its tab has unread mail, quiet on the open tab.
    pub(super) fn tick_tab_chips(&mut self, window: &gpui::Window, reduce: bool) {
        let open = self.tab;
        let counts: Vec<u64> = self.tabs.iter().map(|t| self.tab_unread(t)).collect();
        self.tab_chips.truncate(counts.len());
        for (ix, &count) in counts.iter().enumerate() {
            let shown = if count > 0 { 1.0 } else { 0.0 };
            let quiet = if ix == open { 1.0 } else { 0.0 };
            if ix == self.tab_chips.len() {
                // A new tab starts as it should look, without a fade.
                self.tab_chips.push(TabChip {
                    shown: Spring::new(motion::SMOOTH, shown),
                    quiet: Spring::new(motion::GENTLE, quiet),
                    on: Spring::new(motion::SMOOTH, quiet),
                    width: 0.0,
                    count,
                });
            }
            let width = self.tab_sizes.get(ix).map_or(0.0, |s| s.1);
            let chip = &mut self.tab_chips[ix];
            if count > 0 {
                chip.width = width;
                chip.count = count;
            }
            chip.shown.set(shown);
            chip.quiet.set(quiet);
            chip.on.set(quiet);
            chip.shown.tick(window, reduce);
            chip.quiet.tick(window, reduce);
            chip.on.tick(window, reduce);
        }
    }

    /// Tab `ix`'s chip: how much shows (0 to 1), how quiet it is (0 to
    /// 1), its width and its count.
    fn tab_chip(&self, ix: usize) -> (f32, f32, f32, u64) {
        self.tab_chips.get(ix).map_or((0.0, 0.0, 0.0, 0), |c| {
            (
                c.shown.value().clamp(0.0, 1.0),
                c.quiet.value().clamp(0.0, 1.0),
                c.width,
                c.count,
            )
        })
    }

    /// The unread mail of a tab's categories.
    fn tab_unread(&self, tab: &crate::tabs::Tab) -> u64 {
        tab.categories
            .iter()
            .filter_map(|c| self.category_unread.get(c))
            .sum()
    }

    /// 1 for the open tab, 0 for the others; `settled` gives where a
    /// switch of tabs ends, else where it is now. The highlight moves at
    /// once, the tab's ripple showing the click, while a folded bar's
    /// labels swap smoothly.
    fn tab_on(&self, ix: usize, settled: bool) -> f32 {
        let at_end = if ix == self.tab { 1.0 } else { 0.0 };
        match self.tab_chips.get(ix) {
            Some(chip) if !settled => chip.on.value().clamp(0.0, 1.0),
            _ => at_end,
        }
    }

    /// How much tab `ix` shows its label and its badge, 0 to 1, with the
    /// tabs folded `fold` steps: the open tab keeps its label longest;
    /// a badge shows while its tab has unread mail, the open tab's too.
    fn tab_shares(&self, ix: usize, fold: f32, settled: bool) -> (f32, f32) {
        let on = self.tab_on(ix, settled);
        let step = |from: f32| (fold - from).clamp(0.0, 1.0);
        let label = lerp(1.0, on, step(FOLD_LABELS)) * (1.0 - step(FOLD_NO_COUNTS));
        // The first tab, Primary, has no count, as in Gmail.
        let badge = if ix == 0 {
            0.0
        } else {
            self.tab_chip(ix).0 * (1.0 - step(FOLD_OPEN_LABEL))
        };
        (label, badge)
    }

    /// Tab `ix`'s padding and icon size: a tab without its label has less
    /// padding and a larger icon, which grows as the label folds away.
    fn tab_pad_icon(&self, ix: usize, fold: f32, settled: bool) -> (f32, f32) {
        let (label, _) = self.tab_shares(ix, fold, settled);
        let pad = lerp(TAB_PAD, TAB_PAD_ICONS, fold.clamp(0.0, 1.0));
        (pad, lerp(TAB_ICON_ALONE, TAB_ICON, label))
    }

    /// Tab `ix`'s width in the pill bar.
    fn tab_width(&self, ix: usize, fold: f32, settled: bool) -> f32 {
        let label_w = self.tab_sizes.get(ix).map_or(0.0, |s| s.0);
        let badge_w = self.tab_chip(ix).2;
        let (label, badge) = self.tab_shares(ix, fold, settled);
        let (pad, icon) = self.tab_pad_icon(ix, fold, settled);
        let badge = if badge_w > 0.0 { badge } else { 0.0 };
        2.0 * pad + icon + (TAB_GAP + label_w) * label + (TAB_GAP + badge_w) * badge
    }

    /// The pill bar's width with every label and count showing.
    fn tabs_full_width(&self) -> f32 {
        self.tabs_width(FOLD_LABELS, true)
    }

    /// The pill bar's width folded `fold` steps, as it is now or, when
    /// `settled`, once a switch of tabs ends.
    fn tabs_width(&self, fold: f32, settled: bool) -> f32 {
        (0..self.tabs.len())
            .map(|ix| self.tab_width(ix, fold, settled))
            .sum::<f32>()
            + 2.0 * TABS_INSET
            + TAB_SPACING * self.tabs.len().saturating_sub(1) as f32
    }

    /// The width the inbox tabs have where they go.
    fn tabs_room(&self, fit: TabsFit) -> f32 {
        let width = self.list_width();
        if fit == TabsFit::TopRow {
            // Chosen only where the tabs fit with every label.
            f32::INFINITY
        } else if self.layout.shape.is_phone() && self.checked.is_empty() {
            width - 2.0 * PHONE_TABS_SIDE - PHONE_TABS_MORE
        } else {
            width - 2.0 * TABS_ROW_PAD
        }
    }

    /// How far the inbox tabs fold to fit their room: the first step at
    /// which they fit, or all the way.
    pub(super) fn tabs_fold_target(&self) -> f32 {
        if !self.shows_tabs() {
            return FOLD_LABELS;
        }
        let room = self.tabs_room(self.tabs_fit());
        [FOLD_LABELS, FOLD_OPEN_LABEL, FOLD_NO_COUNTS]
            .into_iter()
            .find(|&fold| self.tabs_width(fold, true) <= room)
            .unwrap_or(FOLD_ICONS)
    }

    /// Where the inbox tabs go for the list's width: in the list's top
    /// row while they fit there with their labels, else in a row of their
    /// own, folding as far as they must (see [`Self::tabs_fold_target`]).
    /// A phone has them in its list bar.
    pub(super) fn tabs_fit(&self) -> TabsFit {
        let width = self.list_width();
        let full = self.tabs_full_width();
        if self.layout.shape.is_phone() && self.checked.is_empty() {
            return TabsFit::Row;
        }
        let left = if self.checked.is_empty() {
            TOP_ROW_LEFT
        } else {
            TOP_ROW_LEFT_CHECKED
        };
        if !self.layout.shape.is_phone() && full <= width - left - TOP_ROW_RIGHT {
            TabsFit::TopRow
        } else {
            TabsFit::Row
        }
    }

    /// The inbox tabs as a pill bar, the open tab highlighted; a click
    /// moves the highlight at once, the ripple showing it. With `fill`, the bar takes that width, sharing what the
    /// tabs leave over evenly among them.
    pub(super) fn render_tabs(
        &self,
        fill: Option<f32>,
        th: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let fold = self.tab_fold.value();
        let extra = fill.map_or(0.0, |fill| {
            ((fill - self.tabs_width(fold, false)) / self.tabs.len().max(1) as f32).max(0.0)
        });
        let widths: Vec<f32> = (0..self.tabs.len())
            .map(|ix| self.tab_width(ix, fold, false) + extra)
            .collect();
        let lefts: Vec<f32> = widths
            .iter()
            .scan(TABS_INSET, |x, w| {
                let left = *x;
                *x += w + TAB_SPACING;
                Some(left)
            })
            .collect();
        let (hl_left, hl_width) = match (lefts.get(self.tab), widths.get(self.tab)) {
            (Some(&left), Some(&width)) => (left, width),
            _ => (TABS_INSET, 0.0),
        };
        let tabs = self.tabs.iter().enumerate().map(|(ix, tab)| {
            let label_w = self.tab_sizes.get(ix).map_or(0.0, |s| s.0);
            let (_, quiet, badge_w, unread) = self.tab_chip(ix);
            let (label, badge) = self.tab_shares(ix, fold, false);
            let (pad, icon_size) = self.tab_pad_icon(ix, fold, false);
            let on = self.tab_on(ix, false);
            let color = mix(th.text_dim, th.nav_selected_text, on);
            // The open tab's count goes quiet: a faint tint of its
            // highlight's text color, under the tab's own color, which
            // fades out over it so the number stays readable throughout.
            let quiet_bg = mix(th.nav_selected, th.nav_selected_text, CHIP_QUIET_BG);
            let quiet_text = mix(th.nav_selected, th.nav_selected_text, CHIP_QUIET_TEXT);
            let count = format::thousands(unread);
            div()
                .id(("tab", ix))
                .relative()
                .flex_none()
                .w(px(widths[ix]))
                .h(px(TAB_HEIGHT))
                .px(px(pad))
                .flex()
                .flex_row()
                .items_center()
                .justify_center()
                .overflow_hidden()
                .rounded_full()
                .text_size(px(TAB_TEXT))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgba(color))
                .cursor_pointer()
                .keeps_press()
                .when(ix != self.tab, |d| {
                    d.relative()
                        .child(crate::widgets::hover_fade(("tab-glow", ix), None, th))
                })
                .when(label < 0.5, |d| d.tooltip(tip(tab.label(), th)))
                .on_click(cx.listener(move |this, _, _, cx| {
                    // The open tab goes back to its top, as its folder does.
                    if this.tab == ix {
                        this.glide_list_to_top(cx);
                    } else {
                        this.open_tab(ix, cx);
                    }
                }))
                .child(Ripple::new(("tab-ripple", ix), rgba(th.ripple)).rounded(TAB_HEIGHT / 2.0))
                .child(icon(tab.icon, color, icon_size))
                .when(label > 0.001, |d| {
                    d.child(
                        div()
                            .flex_none()
                            .w(px((TAB_GAP + label_w) * label))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .opacity(label)
                            // The gap goes inside the clipped room, which
                            // shrinks to nothing with it: as padding it would
                            // hold the room open to the end, and the icon
                            // would shift when the label went.
                            .child(div().flex_none().pl(px(TAB_GAP)).child(tab.label())),
                    )
                })
                .when(badge > 0.001 && badge_w > 0.0, |d| {
                    d.child(
                        div()
                            .flex_none()
                            .w(px((TAB_GAP + badge_w) * badge))
                            .overflow_hidden()
                            .opacity(badge)
                            .child(
                                div()
                                    .flex_none()
                                    .ml(px(TAB_GAP))
                                    .w(px(badge_w))
                                    .h(px(BADGE_HEIGHT))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded_full()
                                    .relative()
                                    .bg(rgba(quiet_bg))
                                    .text_color(rgba(quiet_text))
                                    .text_size(px(BADGE_TEXT))
                                    .font_weight(FontWeight::BOLD)
                                    .child(count.clone())
                                    .when(quiet < 0.999, |d| {
                                        d.child(
                                            div()
                                                .absolute()
                                                .inset_0()
                                                .flex()
                                                .items_center()
                                                .justify_center()
                                                .rounded_full()
                                                .bg(rgba(th.tabs[tab.color]))
                                                .text_color(rgba(th.on_accent))
                                                .opacity(1.0 - quiet)
                                                .child(count),
                                        )
                                    }),
                            ),
                    )
                })
        });
        div()
            .relative()
            .flex_none()
            .h(px(TABS_HEIGHT))
            .p(px(TABS_INSET))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(TAB_SPACING))
            .rounded_full()
            .bg(rgba(tabs_track(th)))
            // The open tab's highlight, under the tabs.
            .child(
                div()
                    .absolute()
                    .top(px(TABS_INSET))
                    .left(px(hl_left))
                    .w(px(hl_width))
                    .h(px(TAB_HEIGHT))
                    .rounded_full()
                    .bg(rgba(th.nav_selected)),
            )
            .children(tabs)
            .child(self.tour_mark(super::tour::Spot::Tabs))
            .into_any_element()
    }

    // Lines

    fn render_list(&mut self, th: &Theme, cx: &mut Context<Self>) -> AnyElement {
        if self.entries.is_empty() {
            let text = match &self.listing {
                Some(Listing::Search { .. }) => tr!("list-empty-search"),
                Some(Listing::Folder(_)) if self.first_sync => {
                    return first_sync_placeholder(th);
                }
                Some(Listing::Folder(_) | Listing::Unified { .. }) if self.shows_tabs() => {
                    match self.tabs.get(self.tab) {
                        Some(tab) => tr!("list-empty-tab", tab = tab.label()),
                        None => tr!("list-empty-tab-unknown"),
                    }
                }
                Some(Listing::Folder(_) | Listing::Unified { .. }) => match self.folder_name() {
                    Some(folder) => tr!("list-empty-folder", folder = folder),
                    None => tr!("list-empty-folder-unknown"),
                },
                None => String::new(),
            };
            return self.placeholder(text, th);
        }
        self.update_visible();
        // Read the lines on show in one go; each line then finds its row.
        let folder = self.listed_folder();
        let ahead =
            self.visible.start.saturating_sub(10)..(self.visible.end + 30).min(self.entries.len());
        if let Ok(mail) = &mut self.mail {
            mail.rows(&self.entries[ahead], folder, self.show_recipients);
        }
        let shape = (self.stacked(), self.row_height().to_bits());
        if self.list_shape != shape {
            self.list_shape = shape;
            self.list_state.remeasure();
        }
        self.keep_opened_line(cx);
        self.list_state.set_head(px(self.list_head.get()));
        self.list_state.follow();
        list(
            self.list_state.state().clone(),
            cx.processor(|this, ix: usize, window, cx| {
                let ix = this.list_state.line(ix);
                let Some(entry) = this.entries.get(ix).copied() else {
                    return div().into_any_element();
                };
                let th = this.theme(window);
                let folder = this.listed_folder();
                let row = match &mut this.mail {
                    Ok(mail) => mail
                        .rows(&[entry], folder, this.show_recipients)
                        .pop()
                        .flatten(),
                    Err(_) => None,
                };
                let row = row.map(|r| this.with_pending(r));
                let row = this.render_row(ix, entry.key, row, &th, cx);
                this.fetch_pictures(cx);
                // The first line starts below the bar pinned over the top.
                if ix == 0 {
                    return div()
                        .flex()
                        .flex_col()
                        .child(div().h(px(this.list_head.get())))
                        .child(row)
                        .into_any_element();
                }
                row
            }),
        )
        .size_full()
        .into_any_element()
    }

    /// The account of a line of the whole unified inbox, which mixes
    /// accounts: its color, its name as the folder pane shows it and its
    /// address.
    fn line_account(&self, row: &Row, th: &Theme) -> Option<(u32, String, String)> {
        let Some(Listing::Unified { account: None, .. }) = &self.listing else {
            return None;
        };
        let account = self.accounts.iter().find(|a| a.id == row.account)?;
        let name = self
            .tree
            .accounts
            .iter()
            .find(|a| a.id == row.account)
            .map_or_else(|| account.address.clone(), |a| a.name.clone());
        let address = account.address.trim().to_owned();
        Some((self.account_color(&address, th), name, address))
    }

    fn render_row(
        &self,
        ix: usize,
        key: EntryKey,
        row: Option<Rc<Row>>,
        th: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let line_height = self.row_height();
        let stacked = self.stacked();
        let has_chips = row.as_ref().is_some_and(|r| !r.files.is_empty());
        let height = line_height + self.chips_extra(has_chips);
        let scrolling = self.list_scrolling.is_some();
        let hovered = !scrolling && self.hovered == Some(ix);
        let cursor = self.selected == Some(ix);
        let checked = self.checked.contains(&key);
        let open = self.split() && self.reader.as_ref().is_some_and(|r| r.key == key);
        // The cursor is grey while the conversation beside has the keys.
        let keys_here = !self.reader_keys;
        let unread = row.as_ref().is_some_and(|r| r.unread);
        let background = th.on_pane(if checked {
            th.checked_row
        } else if open {
            mix(th.surface, th.accent, if keys_here { 0.12 } else { 0.07 })
        } else if unread {
            th.surface
        } else {
            th.read_row
        });
        let base = div()
            .id(("row", ix))
            .relative()
            .w_full()
            .h(px(height))
            .flex()
            .flex_row()
            .bg(rgba(background))
            .border_b_1()
            .border_color(rgba(row_line(th)))
            .text_size(px(14.0))
            .cursor_pointer()
            .keeps_press()
            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                if *hovered {
                    this.hovered = Some(ix);
                } else if this.hovered == Some(ix) {
                    this.hovered = None;
                }
                cx.notify();
            }))
            .on_click(
                cx.listener(move |this, event: &gpui::ClickEvent, window, cx| {
                    // Ctrl+click ticks or unticks the line, Shift+click
                    // ticks every line from the last one clicked; a plain
                    // click opens it.
                    let modifiers = event.modifiers();
                    if modifiers.secondary() {
                        this.click_check(ix, true, cx);
                    } else if modifiers.shift {
                        this.check_range(ix, cx);
                    } else {
                        this.open(ix, window, cx);
                    }
                }),
            )
            .on_mouse_down(
                gpui::MouseButton::Right,
                cx.listener(move |this, event: &gpui::MouseDownEvent, _, cx| {
                    this.open_context_menu(ix, key, event.position, cx)
                }),
            )
            // Dragged onto a folder in the folder pane, it moves there.
            .when_some(
                row.as_ref()
                    .filter(|_| !self.layout.shape.is_phone())
                    .map(|r| self.mail_drag(key, r.account, &r.subject, th)),
                |d, drag| {
                    d.on_drag(drag, |drag, offset, _, cx| {
                        cx.new(|_| MailWindow::start_mail_drag(drag, offset))
                    })
                },
            )
            .on_drag_move(cx.listener(|this, event: &DragMoveEvent<MailDrag>, _, cx| {
                this.mail_drag_moved(event, cx)
            }))
            .when(self.mail_dragged(key, cx), |d| d.opacity(0.45))
            .child(Ripple::new(("row-ripple", ix), rgba(th.ripple)).rounded(0.0))
            // The keyboard cursor: a bar that grows from the middle.
            .child(
                div()
                    .absolute()
                    .left_0()
                    .w(px(3.0))
                    .rounded_r(px(2.0))
                    .bg(rgba(if keys_here { th.accent } else { th.text_faint }))
                    .with_spring(
                        ("row-cursor", ix),
                        SpringAnimation::new(katna_ui::motion::scaled(motion::SLIDE))
                            .to(if cursor { 1.0 } else { 0.0 }),
                        move |el, s: f32| {
                            let s = s.clamp(0.0, 1.0);
                            el.top(px(height / 2.0 * (1.0 - s))).h(px(height * s))
                        },
                    ),
            );
        let lifted = |base: Stateful<Div>| {
            // The line takes a flat, crisp tint of the accent color, with no
            // shadow; dark pages lighten it a touch first so it shows.
            let lit = if th.dark {
                mix(mix(background, 0xffffffff, 0.05), th.accent, 0.15)
            } else {
                mix(background, th.accent, 0.11)
            };
            base.with_spring(
                ("row-lift", ix),
                SpringAnimation::new(katna_ui::motion::scaled(ROW_LIFT)).to(if hovered {
                    1.0
                } else {
                    0.0
                }),
                move |el, s: f32| {
                    let s = s.clamp(0.0, 1.0);
                    if s > 0.001 {
                        el.bg(rgba(mix(background, lit, s)))
                    } else {
                        el
                    }
                },
            )
            .into_any_element()
        };
        let Some(row) = row else {
            return lifted(
                base.items_center()
                    .pl(px(96.0))
                    .text_color(rgba(th.text_faint))
                    .child(tr!("row-removed")),
            );
        };
        let now = jiff::Timestamp::now().as_second();
        // Snoozed mail shows when it comes back instead.
        let date = row
            .snoozed_until
            .or(row.date)
            .and_then(|d| format::local(d, &self.tz))
            .zip(format::local(now, &self.tz))
            .map(|(d, now)| format::list_date(d, now))
            .unwrap_or_default();
        let weight = if row.unread {
            FontWeight::BOLD
        } else {
            FontWeight::NORMAL
        };
        // As in Gmail, a tick box, star or marker that is off rests dim and
        // comes up to full contrast while the pointer is over its line. A
        // phone has no pointer, so there they stay as they are.
        let rest = !hovered && !self.layout.shape.is_phone();
        let off = |id: &'static str, name: &'static str, size: f32| {
            div()
                .with_spring(
                    (id, ix),
                    SpringAnimation::new(katna_ui::motion::scaled(ROW_LIFT)).to(if rest {
                        0.0
                    } else {
                        1.0
                    }),
                    move |el, s: f32| el.opacity(OFF_REST + (1.0 - OFF_REST) * s.clamp(0.0, 1.0)),
                )
                .child(icon(name, th.text_dim, size))
                .into_any_element()
        };
        let check = div()
            .id(("row-check", ix))
            .size(px(32.0))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded_full()
            .when(!scrolling, |d| d.hover(|s| s.bg(rgba(th.hover))))
            .on_click(cx.listener(move |this, event: &gpui::ClickEvent, _, cx| {
                cx.stop_propagation();
                if event.modifiers().shift {
                    this.check_range(ix, cx);
                } else {
                    this.click_check(ix, false, cx);
                }
            }))
            // One tree whether checked or not, so the tick can draw in.
            .child(
                div()
                    .with_spring(
                        ("row-check-rest", ix),
                        SpringAnimation::new(katna_ui::motion::scaled(ROW_LIFT))
                            .to(if rest && !checked { 0.0 } else { 1.0 }),
                        move |el, s: f32| {
                            el.opacity(OFF_REST + (1.0 - OFF_REST) * s.clamp(0.0, 1.0))
                        },
                    )
                    .child(crate::widgets::checkbox_colored(
                        ("row-box", ix),
                        crate::widgets::Check::from(checked),
                        th.text,
                        th,
                    )),
            );
        let flagged = row.flagged;
        let star = div()
            .id(("row-star", ix))
            .tooltip(tip(
                if row.flagged {
                    tr!("row-starred")
                } else {
                    tr!("row-not-starred")
                },
                th,
            ))
            .size(px(32.0))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded_full()
            .when(!scrolling, |d| d.hover(|s| s.bg(rgba(th.hover))))
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                this.act(Act::Star(!flagged), vec![key], cx);
            }))
            .child(if row.flagged {
                icon("star-filled", th.star, 20.0)
            } else {
                off("row-star-rest", "star", 20.0)
            });
        let important = row.important;
        // Settings > Appearance > Important markers.
        let marker = self.config.mail.important_markers.then(|| {
            div()
                .id(("row-important", ix))
                .tooltip(tip(
                    if important {
                        tr!("row-important")
                    } else {
                        tr!("row-mark-important")
                    },
                    th,
                ))
                .size(px(32.0))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .when(!scrolling, |d| d.hover(|s| s.bg(rgba(th.hover))))
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.act(Act::Important(!important), vec![key], cx);
                }))
                .child(if important {
                    icon("important-filled", th.important, 18.0)
                } else {
                    off("row-important-rest", "important", 18.0)
                })
        });
        let account = self.line_account(&row, th);
        let correspondent = div()
            .flex()
            .flex_row()
            .items_center()
            .min_w_0()
            .gap(px(4.0))
            .text_color(rgba(th.text))
            .map(|d| {
                // A muted sender gets a crossed bell after their name
                // (§15.1.1); the names are then laid out one by one.
                let marks: Vec<Option<AnyElement>> = row
                    .people
                    .iter()
                    .map(|(_, email)| {
                        email
                            .as_deref()
                            .and_then(|email| self.muted_mark(email, 16.0, th))
                    })
                    .collect();
                if marks.iter().all(Option::is_none) {
                    return d.child(
                        div()
                            .min_w_0()
                            .truncate()
                            .font_weight(weight)
                            .child(row.correspondent.clone()),
                    );
                }
                d.child(
                    div()
                        .min_w_0()
                        .flex()
                        .flex_row()
                        .items_center()
                        .overflow_hidden()
                        .font_weight(weight)
                        .children(row.people.iter().zip(marks).flat_map(|((text, _), mark)| {
                            let text = div()
                                .min_w_0()
                                .truncate()
                                .child(text.clone())
                                .into_any_element();
                            let mark = mark.map(|mark| {
                                // Lifted 1 px to sit on the name as the
                                // contact card's does.
                                div()
                                    .flex_none()
                                    .relative()
                                    .top(px(-1.0))
                                    .pl(px(4.0))
                                    .child(mark)
                                    .into_any_element()
                            });
                            std::iter::once(text).chain(mark)
                        })),
                )
            })
            .when(row.count > 1, |d| {
                // The conversation's mail count: a faint chat icon and the
                // number, centred on the names' line.
                d.child(
                    div()
                        .flex_none()
                        .pl(px(4.0))
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(3.0))
                        .text_size(px(13.0))
                        .text_color(rgba(th.text_faint))
                        .child(icon("forum", th.text_faint, 14.0))
                        .child(row.count.to_string()),
                )
            })
            .when(row.replied, |d| {
                // A faint reply arrow when the user answered, after the
                // count.
                d.child(
                    div()
                        .id(("row-replied", ix))
                        .flex_none()
                        .pl(px(4.0))
                        .flex()
                        .items_center()
                        .tooltip(tip(tr!("list-replied"), th))
                        .child(icon("reply", th.text_faint, 14.0)),
                )
            })
            // The account of a line of the whole unified inbox: its dot
            // after the names; hovering the dot names the account.
            .children(account.map(|(color, name, address)| {
                div()
                    .id(("row-account", ix))
                    .flex_none()
                    .ml(px(1.0))
                    .size(px(15.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .tooltip(tip(
                        if name.eq_ignore_ascii_case(&address) {
                            name
                        } else {
                            format!("{name}\n{address}")
                        },
                        th,
                    ))
                    .child(div().size(px(7.0)).rounded_full().bg(rgba(color)))
            }));
        // The quick actions fade in over the date.
        let actions = hovered.then(|| {
            div()
                .child(self.hover_actions(ix, key, row.unread, row.pinned, th, cx))
                .with_animation(
                    ("row-actions", ix),
                    Animation::new(katna_ui::motion::time(ACTIONS_IN))
                        .with_easing(ease_out_quint()),
                    |el, t| el.opacity(t),
                )
                .into_any_element()
        });
        let date = div()
            .flex_none()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.0))
            .text_size(px(12.0))
            .font_weight(weight)
            .whitespace_nowrap()
            .text_color(rgba(if row.unread { th.text } else { th.text_faint }))
            .when(self.line_muted(key), |d| {
                d.child(
                    div()
                        .id(("row-muted", ix))
                        .tooltip(tip(tr!("quiet-row-muted"), th))
                        .child(icon("bell-off", th.text_faint, 16.0)),
                )
            })
            .when(row.pinned, |d| {
                d.child(
                    div()
                        .id(("row-pinned", ix))
                        .tooltip(tip(tr!("row-pinned"), th))
                        .child(icon("pin-filled", th.accent, 16.0)),
                )
            })
            .map(|d| match row.snoozed_until {
                Some(until) => d.text_color(rgba(th.accent)).child(
                    div()
                        .id(("row-snoozed", ix))
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(4.0))
                        .tooltip(tip(
                            tr!(
                                "row-snoozed-until",
                                when = super::snooze::describe(until, &self.tz)
                            ),
                            th,
                        ))
                        .child(icon("schedule", th.accent, 16.0))
                        .child(date),
                ),
                None => d.child(date),
            });

        if stacked {
            let line_h = (line_height - 16.0) / 3.0;
            let line = |child: AnyElement| {
                div()
                    .h(px(line_h))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.0))
                    .child(child)
            };
            let subject = div()
                .flex_1()
                .min_w_0()
                .truncate()
                .font_weight(weight)
                .text_color(rgba(th.text))
                .child(row.subject.clone())
                .into_any_element();
            let snippet = div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_color(rgba(preview_color(th)))
                .child(row.snippet.clone())
                .into_any_element();
            // A phone shows the sender's picture, which ticks the line, and
            // keeps the star and Important marker on the right, as mobile
            // mail does; it has no hover toolbar to collide with them.
            // Wider, the tick, star and marker stand in a column on the left,
            // so the hover toolbar only ever covers the date.
            let phone = self.layout.shape.is_phone();
            let actions = if phone { None } else { actions };
            let lead_width = if phone { 68.0 } else { 44.0 };
            let (lead, side) = if phone {
                let lead = div()
                    .w(px(lead_width))
                    .flex_none()
                    .flex()
                    .flex_col()
                    .items_center()
                    .pt(px(2.0))
                    .child(self.line_picture(ix, &row.correspondent, &row.sender, checked, th, cx));
                (lead, Some((marker, star)))
            } else {
                // Each sits on one of the three lines, its hover circle
                // trimmed to the line so neighbors don't overlap.
                let size = (line_h + 4.0).min(32.0);
                let cell = |child: Stateful<Div>| {
                    div()
                        .h(px(line_h))
                        .flex()
                        .items_center()
                        .child(child.size(px(size)))
                };
                let lead = div()
                    .w(px(lead_width))
                    .flex_none()
                    .flex()
                    .flex_col()
                    .items_center()
                    .child(cell(check))
                    .child(cell(star))
                    .children(marker.map(cell));
                (lead, None)
            };
            return lifted(
                base.py(px(8.0)).child(lead).child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .pr(px(12.0))
                        .flex()
                        .flex_col()
                        .child(
                            line(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(correspondent)
                                    .into_any_element(),
                            )
                            .children(actions.or(Some(date.into_any_element()))),
                        )
                        .child(line(subject).children(side.map(|(marker, star)| {
                            div()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap(px(8.0))
                                .children(marker)
                                .child(star)
                        })))
                        .child(
                            line(snippet)
                                .children(self.render_row_task(ix, key, th, cx))
                                .when(row.attachments && !has_chips, |d| {
                                    d.child(icon("attachment", th.text_faint, 16.0))
                                })
                                .children(tracking_mark(ix, &row, 16.0, th)),
                        )
                        .when(has_chips, |d| {
                            let room = self.list_width() - lead_width - 12.0;
                            d.child(self.file_chips(ix, &row, 0.0, room, th, cx))
                        }),
                ),
            );
        }

        let mut text = row.subject.clone();
        let subject_end = text.len();
        if !row.snippet.is_empty() {
            text.push_str(" - ");
            text.push_str(&row.snippet);
        }
        let text_len = text.len();
        let text = StyledText::new(text).with_highlights(vec![
            (
                0..subject_end,
                HighlightStyle {
                    color: Some(rgba(th.text).into()),
                    font_weight: Some(weight),
                    ..Default::default()
                },
            ),
            (
                subject_end..text_len,
                HighlightStyle {
                    color: Some(rgba(preview_color(th)).into()),
                    ..Default::default()
                },
            ),
        ]);
        let wide = self.list_width() > 1000.0;
        let name_width = if wide { 200.0 } else { 150.0 };
        let first_line = div()
            .h(px(line_height))
            .flex_none()
            .w_full()
            .flex()
            .flex_row()
            .items_center()
            .pl(px(8.0))
            .child(check)
            .child(star)
            .children(marker)
            .child(
                div()
                    .w(px(name_width))
                    .flex_none()
                    .pl(px(8.0))
                    .pr(px(24.0))
                    .child(correspondent),
            )
            .child(div().flex_1().min_w_0().truncate().child(text))
            .children(
                self.render_row_task(ix, key, th, cx)
                    .map(|chip| div().pl(px(8.0)).child(chip)),
            )
            .when(row.attachments && !has_chips, |d| {
                d.child(
                    div()
                        .pl(px(8.0))
                        .child(icon("attachment", th.text_faint, 18.0)),
                )
            })
            .children(tracking_mark(ix, &row, 18.0, th).map(|mark| div().pl(px(8.0)).child(mark)))
            .child(
                div()
                    .flex_none()
                    .min_w(px(96.0))
                    .pl(px(16.0))
                    .pr(px(12.0))
                    .flex()
                    .justify_end()
                    .children(actions.or(Some(date.into_any_element()))),
            );
        // The chips line up under the subject.
        let chips_left = 8.0 + 3.0 * 32.0 + name_width;
        lifted(base.flex_col().child(first_line).when(has_chips, |d| {
            let room = self.list_width() - chips_left - 24.0;
            d.child(self.file_chips(ix, &row, chips_left, room, th, cx))
        }))
    }

    /// The attachment chips under a line: as many as fit (at most three),
    /// then a round "+N" button that lists the rest.
    fn file_chips(
        &self,
        ix: usize,
        row: &Row,
        left: f32,
        room: f32,
        th: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (key, files) = (row.key, row.files.as_slice());
        let fit = |room: f32| ((room + CHIP_GAP) / (CHIP_WIDTH + CHIP_GAP)).floor() as usize;
        let mut shown = fit(room);
        if shown < files.len() {
            shown = fit(room - MORE_SIZE - CHIP_GAP);
        }
        let shown = shown.clamp(1, 3).min(files.len());
        let rest: Vec<RowFile> = files[shown..].to_vec();
        let chip = |n: usize, file: &RowFile| {
            let kind = katna_preview::kind(&file.mime, &file.name);
            let open = file.clone();
            let downloading = self.chip_downloading(file);
            div()
                .id(("row-file", ix * 4 + n))
                .relative()
                .h(px(CHIP_HEIGHT))
                .min_w(px(64.0))
                .max_w(px(CHIP_WIDTH))
                .flex_shrink(1.0)
                .flex()
                .items_center()
                .gap(px(8.0))
                .pl(px(8.0))
                .pr(px(14.0))
                .rounded_full()
                .border_1()
                .border_color(rgba(th.outline))
                .bg(rgba(th.on_pane(th.surface)))
                .when(!downloading, |d| {
                    d.cursor_pointer()
                        .hover(|s| s.bg(rgba(th.hover)))
                        .tooltip(tip(file.name.clone(), th))
                })
                .on_click(cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    this.open_row_file(&open, window, cx);
                }))
                .children(self.chip_fill(file, true, th))
                .child(kind_badge(kind, 18.0))
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_size(px(13.0))
                        .text_color(rgba(th.text_dim))
                        .child(file.name.clone()),
                )
        };
        let chips: Vec<_> = files[..shown]
            .iter()
            .enumerate()
            .map(|(n, f)| chip(n, f))
            .collect();
        let more = (!rest.is_empty()).then(|| {
            let open = self.files_menu == Some(key);
            let names = rest
                .iter()
                .map(|f| f.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            let button = div()
                .id(("row-more-files", ix))
                .size(px(MORE_SIZE))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .border_1()
                .border_color(rgba(th.outline))
                .bg(rgba(if open {
                    th.hover
                } else {
                    th.on_pane(th.surface)
                }))
                .cursor_pointer()
                .keeps_press()
                .relative()
                .child(crate::widgets::hover_fade("hover-glow", None, th))
                .text_size(px(12.0))
                .text_color(rgba(th.text_dim))
                .child(tr!("list-files-more", count = rest.len() as u64))
                .when(!open, |d| d.tooltip(tip(names, th)))
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.menu = None;
                    this.files_menu = if this.files_menu == Some(key) {
                        None
                    } else {
                        Some(key)
                    };
                    cx.notify();
                }));
            div()
                .relative()
                .child(button)
                .when(open, |d| d.children(self.files_popover(ix, &rest, th, cx)))
        });
        div()
            .h(px(CHIP_HEIGHT))
            .flex_none()
            .pl(px(left))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(CHIP_GAP))
            .children(chips)
            .children(more)
            .into_any_element()
    }

    /// The list the "+N" button opens: the attachments without a chip.
    fn files_popover(
        &self,
        ix: usize,
        files: &[RowFile],
        th: &Theme,
        cx: &mut Context<Self>,
    ) -> [AnyElement; 2] {
        let scrim = deferred(
            div()
                .id("files-scrim")
                .absolute()
                .top(px(-2000.0))
                .left(px(-4000.0))
                .w(px(8000.0))
                .h(px(6000.0))
                .occlude()
                .on_mouse_down(
                    gpui::MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.files_menu = None;
                        cx.notify();
                    }),
                ),
        )
        .with_priority(1)
        .into_any_element();
        let items = files.iter().enumerate().map(|(n, file)| {
            let kind = katna_preview::kind(&file.mime, &file.name);
            let open = file.clone();
            let downloading = self.chip_downloading(file);
            div()
                .id(("files-item", n))
                .relative()
                .h(px(40.0))
                .px(px(16.0))
                .flex()
                .items_center()
                .gap(px(12.0))
                .when(!downloading, |d| {
                    d.cursor_pointer().hover(|s| s.bg(rgba(th.hover)))
                })
                .on_click(cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    this.open_row_file(&open, window, cx);
                }))
                .children(self.chip_fill(file, false, th))
                .child(kind_badge(kind, 20.0))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_color(rgba(th.text))
                        .child(file.name.clone()),
                )
                .child(
                    div()
                        .flex_none()
                        .text_size(px(12.0))
                        .text_color(rgba(th.text_faint))
                        .child(format::size(file.size)),
                )
        });
        let popover = deferred(
            anchored()
                .offset(point(px(0.0), px(6.0)))
                .snap_to_window_with_margin(px(8.0))
                .child(
                    menu(th)
                        .id(("files-menu", ix))
                        .w(px(320.0))
                        .max_h(px(320.0))
                        .overflow_y_scroll()
                        .occlude()
                        .text_size(px(14.0))
                        .children(items)
                        .with_animation(
                            ("files-menu", ix),
                            Animation::new(katna_ui::motion::time(Duration::from_millis(160)))
                                .with_easing(ease_out_quint()),
                            |el, t| el.opacity(t).mt(px(-6.0 * (1.0 - t))),
                        ),
                ),
        )
        .with_priority(2)
        .into_any_element();
        [scrim, popover]
    }

    /// Archive, delete, read/unread, pin and snooze buttons shown on the
    /// hovered line in place of its date.
    fn hover_actions(
        &self,
        ix: usize,
        key: EntryKey,
        unread: bool,
        pinned: bool,
        th: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let button = |id: usize, name: &str, label: String| {
            icon_button_colored(("row-action", ix * 5 + id), name, 18.0, th.text_dim, th)
                .size(px(32.0))
                .tooltip(tip(label, th))
        };
        div()
            .flex()
            .flex_row()
            .items_center()
            .child(
                button(0, "archive", tr!("list-archive")).on_click(cx.listener(
                    move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.act(Act::Archive, vec![key], cx);
                    },
                )),
            )
            .child(button(1, "trash", tr!("list-delete")).on_click(cx.listener(
                move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.act(Act::Delete, vec![key], cx);
                },
            )))
            .child(
                button(
                    2,
                    if unread { "mark-read" } else { "mark-unread" },
                    if unread {
                        tr!("list-mark-read")
                    } else {
                        tr!("list-mark-unread")
                    },
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.act(Act::Read(unread), vec![key], cx);
                })),
            )
            .child(
                button(
                    3,
                    if pinned { "pin-filled" } else { "pin" },
                    if pinned {
                        tr!("row-unpin")
                    } else {
                        tr!("row-pin")
                    },
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.act(Act::Pin(!pinned), vec![key], cx);
                })),
            )
            .map(|d| match self.folder_role() {
                Role::Snoozed => d.child(button(4, "inbox", tr!("list-unsnooze")).on_click(
                    cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.act(Act::Unsnooze, vec![key], cx);
                    }),
                )),
                Role::Drafts | Role::Sent | Role::Trash | Role::Junk => d,
                _ => d.child(
                    button(4, "schedule", tr!("list-snooze")).on_click(cx.listener(
                        move |this, event: &gpui::ClickEvent, _, cx| {
                            cx.stop_propagation();
                            this.open_snooze_menu(vec![key], event.position(), cx);
                        },
                    )),
                ),
            })
            .with_animation(
                ("row-actions", ix),
                Animation::new(katna_ui::motion::time(Duration::from_millis(140)))
                    .with_easing(ease_out_quint()),
                |el, t| el.opacity(t),
            )
            .into_any_element()
    }
}

/// Fades `body` in whenever `seq` changes: the card switched content.
fn fade_in(body: AnyElement, seq: usize) -> AnyElement {
    div()
        .flex_1()
        .min_h_0()
        .child(body)
        .with_animation(
            ("card", seq),
            Animation::new(katna_ui::motion::time(Duration::from_millis(220)))
                .with_easing(ease_out_quint()),
            // From half-drawn, so the card never shows a blank frame.
            |el, t| el.opacity(0.5 + 0.5 * t),
        )
        .into_any_element()
}

/// The preview text of a mail row: the faint text a third of the way
/// toward the list's background, quieter than the sender and subject. On a
/// see-through card it is the dim text, which stays readable over a bright
/// wallpaper ([`Theme::frosted_panes`]).
pub(super) fn preview_color(th: &Theme) -> u32 {
    if th.pane_tint < 100 {
        th.text_dim
    } else {
        mix(th.text_faint, th.surface, 0.35)
    }
}

/// The faint line between mail rows.
pub(super) fn row_line(th: &Theme) -> u32 {
    th.divider
}

/// The background behind the inbox tabs: the search box's colour in dark
/// mode, half way to the list's surface in light mode.
fn tabs_track(th: &Theme) -> u32 {
    if th.dark {
        th.search
    } else {
        mix(th.search, th.surface, 0.5)
    }
}

/// A thin vertical line between toolbar groups.
pub(super) fn separator(th: &Theme) -> Div {
    div()
        .mx(px(6.0))
        .w(px(1.0))
        .h(px(20.0))
        .bg(rgba(th.divider))
}

/// An empty folder while the first sync runs: the mail is on its way.
fn first_sync_placeholder(th: &Theme) -> AnyElement {
    div()
        .size_full()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(12.0))
        .p(px(24.0))
        .child(
            div()
                .w(px(160.0))
                .h(px(4.0))
                .rounded_full()
                .overflow_hidden()
                .relative()
                .bg(rgba(fade(th.accent, 0.24)))
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .h_full()
                        .w(px(64.0))
                        .rounded_full()
                        .bg(rgba(th.accent))
                        .with_animation(
                            "first-sync",
                            Animation::new(katna_ui::motion::time(Duration::from_millis(1300)))
                                .repeat(),
                            |bar, t| bar.left(px(-64.0 + 224.0 * t)),
                        ),
                ),
        )
        .child(
            div()
                .text_size(px(14.0))
                .text_color(rgba(th.text_dim))
                .child(tr!("list-first-sync")),
        )
        .child(
            div()
                .text_size(px(13.0))
                .text_color(rgba(th.text_faint))
                .child(tr!("list-first-sync-detail")),
        )
        .into_any_element()
}

/// The select menu's choice as the messages about it name it.
fn pick_name(pick: Pick) -> &'static str {
    match pick {
        Pick::Read => "read",
        Pick::Unread => "unread",
        Pick::Starred => "starred",
        Pick::Unstarred => "unstarred",
        Pick::All | Pick::None => "other",
    }
}

/// "No unread conversations here", when a pick matches nothing.
fn pick_none_text(pick: Pick, conversations: bool) -> String {
    let kind = if conversations {
        "conversation"
    } else {
        "message"
    };
    tr!("list-picked-none", pick = pick_name(pick), kind = kind)
}

/// The eye on a line of mail sent with open and click tracking: in the
/// accent color once a recipient opened it, with who did in its tooltip.
fn tracking_mark(ix: usize, row: &Row, size: f32, th: &Theme) -> Option<AnyElement> {
    let tracked = row.tracking?;
    let text = if tracked.clicked > 0 {
        tr!(
            "row-tracking-clicked",
            opened = tracked.opened,
            recipients = tracked.recipients,
            clicked = tracked.clicked
        )
    } else if tracked.opened > 0 {
        tr!(
            "row-tracking-opened",
            opened = tracked.opened,
            recipients = tracked.recipients
        )
    } else {
        tr!("row-tracking-none")
    };
    let color = if tracked.opened > 0 {
        th.accent
    } else {
        th.text_faint
    };
    Some(
        div()
            .id(("row-tracking", ix))
            .flex_none()
            .child(icon("eye", color, size))
            .tooltip(tip(text, th))
            .into_any_element(),
    )
}
