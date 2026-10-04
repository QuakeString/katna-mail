// SPDX-License-Identifier: GPL-3.0-or-later

//! Pins in the chat: up to five mails, files or bits of text kept at the
//! top of a conversation, on this computer (`docs/ARCHITECTURE.md`, the
//! chat view). A thin bar under the header shows one; a click jumps to its
//! bubble and moves on to the next, and the list button shows them all,
//! to reorder or unpin. A sixth asks which one it replaces.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Instant;

use gpui::{
    AnimationExt, AnyElement, Context, FontWeight, MouseButton, MouseUpEvent, div, point,
    prelude::*, relative, rgba,
};
use katna_i18n::tr;
use katna_store::{ChatPin, MAX_CHAT_PINS, MessageId, Pinned};
use katna_ui::{px, unpx};

use super::super::super::MailWindow;
use super::super::super::attachments::kind_badge;
use super::super::first_name;
use super::day_label;
use crate::daemon::Command;
use crate::format;
use crate::theme::Theme;
use crate::widgets::{icon, icon_button_colored, raised, tip};

/// The longest label a pin keeps.
const LABEL: usize = 120;
/// How long a bubble jumped to stays lit.
pub(super) const FLASH_MS: f32 = 1200.0;

/// The open chat's pins.
#[derive(Default)]
pub(in crate::window) struct Pins {
    pub(super) list: Vec<ChatPin>,
    /// Read for the conversation as it is: a change in the store reads
    /// them again.
    read: bool,
    /// The pin the bar shows.
    at: usize,
    /// The list of all pins is open, and how many times it was opened
    /// (each opening plays its slide).
    open: Option<usize>,
    runs: usize,
    /// A sixth pin waits for the one it replaces.
    replace: Option<Replace>,
    /// A row of the list being dragged, and where it would go.
    drag: Option<(usize, usize)>,
    /// Where each bubble starts in the feed, and where the feed starts,
    /// as last drawn: what a jump scrolls to.
    pub(super) tops: Rc<RefCell<HashMap<MessageId, f32>>>,
    pub(super) feed_top: Rc<Cell<f32>>,
    /// The bubble just jumped to, lit for a moment.
    pub(super) flash: Option<(MessageId, Instant)>,
    pub(super) flashes: usize,
}

/// A pin waiting for room.
struct Replace {
    message: MessageId,
    what: Pinned,
    label: String,
    /// The pin it takes off: the oldest unless another is picked.
    chosen: i64,
}

impl Pins {
    /// The store changed: the pins are read again before the next frame.
    pub(in crate::window) fn forget(&mut self) {
        self.read = false;
    }

    /// The pins of message `id`.
    pub(super) fn of(&self, id: MessageId) -> impl Iterator<Item = &ChatPin> {
        self.list.iter().filter(move |p| p.message == id)
    }

    /// The pin of `what` in message `id`, if pinned.
    pub(super) fn find(&self, id: MessageId, what: &Pinned) -> Option<i64> {
        self.of(id).find(|p| p.what == *what).map(|p| p.id)
    }
}

impl MailWindow {
    /// Reads the open chat's pins when the store changed since.
    pub(super) fn read_chat_pins(&mut self) {
        let (Some(reader), Ok(mail)) = (&mut self.reader, &self.mail) else {
            return;
        };
        if reader.chat.pins.read {
            return;
        }
        let ids: Vec<MessageId> = reader
            .parts
            .iter()
            .filter(|p| p.pending.is_none())
            .map(|p| p.id)
            .collect();
        let pins = &mut reader.chat.pins;
        pins.list = mail.chat_pins(&ids);
        pins.read = true;
        pins.at = pins.at.min(pins.list.len().saturating_sub(1));
    }

    /// What the bar shows for `what` of message `id`: the file's name, the
    /// text, or the first line the sender wrote.
    fn pin_label(&self, id: MessageId, what: &Pinned) -> String {
        let reader = self.reader.as_ref();
        let part = reader.and_then(|r| r.parts.iter().find(|p| p.id == id));
        let view = part.and_then(|p| p.body.as_ref()?.view.as_ref());
        let label = match what {
            Pinned::File(ix) => view
                .and_then(|v| v.attachments.get(*ix))
                .map(|a| a.name.clone())
                .unwrap_or_default(),
            Pinned::Text(text) => text.clone(),
            Pinned::Mail => part
                .and_then(|p| reader?.said(p))
                .and_then(|s| {
                    s.text
                        .lines()
                        .map(str::trim)
                        .find(|l| !l.is_empty())
                        .map(str::to_owned)
                })
                .or_else(|| reader.map(|r| r.subject.to_string()))
                .unwrap_or_default(),
        };
        let label = label.split_whitespace().collect::<Vec<_>>().join(" ");
        match label.char_indices().nth(LABEL) {
            Some((end, _)) => format!("{}…", &label[..end]),
            None => label,
        }
    }

    /// Pins `what` of message `id` first, or asks which pin it replaces
    /// when the chat holds five.
    pub(super) fn pin_in_chat(&mut self, id: MessageId, what: Pinned, cx: &mut Context<Self>) {
        let label = self.pin_label(id, &what);
        let Some(reader) = &mut self.reader else {
            return;
        };
        let pins = &mut reader.chat.pins;
        if pins.find(id, &what).is_some() {
            return;
        }
        if pins.list.len() >= MAX_CHAT_PINS {
            let Some(oldest) = pins.list.iter().min_by_key(|p| (p.created_at, p.id)) else {
                return;
            };
            pins.open = None;
            pins.replace = Some(Replace {
                message: id,
                what,
                label,
                chosen: oldest.id,
            });
            cx.notify();
            return;
        }
        self.place_pin(id, what, label, None, cx);
    }

    /// Shows the pin at once and has the daemon keep it; the store's
    /// change brings its real ID.
    fn place_pin(
        &mut self,
        id: MessageId,
        what: Pinned,
        label: String,
        replace: Option<i64>,
        cx: &mut Context<Self>,
    ) {
        let Some(reader) = &mut self.reader else {
            return;
        };
        let pins = &mut reader.chat.pins;
        if let Some(replace) = replace {
            pins.list.retain(|p| p.id != replace);
        }
        pins.list.insert(
            0,
            ChatPin {
                id: 0,
                message: id,
                what: what.clone(),
                label: label.clone(),
                created_at: jiff::Timestamp::now().as_second(),
            },
        );
        pins.at = 0;
        cx.notify();
        self.send(
            Command::PinInChat(id, what, label, replace),
            None,
            None,
            false,
            cx,
        );
    }

    /// Takes off pin `pin`.
    /// The mail and text selected in one bubble of the chat, to pin.
    pub(in crate::window) fn chat_text_to_pin(&self) -> Option<(MessageId, String)> {
        if !self.chat_shown() {
            return None;
        }
        let part = self.text.single_part()?;
        let id = self.reader.as_ref()?.parts.get(part)?.id;
        let text = self.text.text();
        let text = text.trim();
        (!text.is_empty()).then(|| (id, text.to_owned()))
    }

    /// Pins the text selected in a bubble.
    pub(in crate::window) fn pin_chat_text(&mut self, cx: &mut Context<Self>) {
        if let Some((id, text)) = self.chat_text_to_pin() {
            self.text.clear();
            self.pin_in_chat(id, Pinned::Text(text), cx);
        }
    }

    pub(super) fn unpin_in_chat(&mut self, pin: i64, cx: &mut Context<Self>) {
        let Some(reader) = &mut self.reader else {
            return;
        };
        let pins = &mut reader.chat.pins;
        pins.list.retain(|p| p.id != pin);
        pins.at = pins.at.min(pins.list.len().saturating_sub(1));
        if pins.list.is_empty() {
            pins.open = None;
        }
        cx.notify();
        if pin != 0 {
            self.send(Command::UnpinInChat(pin), None, None, false, cx);
        }
    }

    /// Pins `what` of message `id`, or unpins it when pinned.
    pub(super) fn toggle_chat_pin(&mut self, id: MessageId, what: Pinned, cx: &mut Context<Self>) {
        let pinned = self
            .reader
            .as_ref()
            .and_then(|r| r.chat.pins.find(id, &what));
        match pinned {
            Some(pin) => self.unpin_in_chat(pin, cx),
            None => self.pin_in_chat(id, what, cx),
        }
    }

    /// The bar's click: goes to the bubble of the pin it shows, then shows
    /// the next.
    fn jump_to_chat_pin(&mut self, cx: &mut Context<Self>) {
        let Some(reader) = &mut self.reader else {
            return;
        };
        let pins = &mut reader.chat.pins;
        let Some(message) = pins.list.get(pins.at).map(|p| p.message) else {
            return;
        };
        pins.at = (pins.at + 1) % pins.list.len();
        self.jump_to_bubble(message, cx);
    }

    /// Scrolls the chat to the bubble of `message`, which glows for a
    /// moment.
    pub(in crate::window) fn jump_to_bubble(&mut self, message: MessageId, cx: &mut Context<Self>) {
        let Some(reader) = &mut self.reader else {
            return;
        };
        let pins = &mut reader.chat.pins;
        let top = pins.tops.borrow().get(&message).copied();
        let feed_top = pins.feed_top.get();
        pins.flash = Some((message, Instant::now()));
        pins.flashes += 1;
        // Off the end: the feed no longer keeps to it.
        reader.chat.stuck = false;
        reader.chat.held.set(false);
        reader.chat.settle = 0;
        if let Some(top) = top {
            let max = unpx(self.reader_scroll.max_offset().y);
            // Below the header pinned over the top.
            let y = (top - feed_top - self.reader_head.get() - 12.0).clamp(0.0, max.max(0.0));
            self.reader_scroll.set_offset(point(px(0.0), px(-y)));
        }
        cx.notify();
    }

    /// Folds the list of pins or the replace question, if open: what Esc
    /// does first.
    pub(in crate::window) fn fold_chat_pins(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(reader) = &mut self.reader else {
            return false;
        };
        let pins = &mut reader.chat.pins;
        let open = pins.open.take().is_some() | pins.replace.take().is_some();
        pins.drag = None;
        if open {
            cx.notify();
        }
        open
    }

    fn toggle_pin_list(&mut self, cx: &mut Context<Self>) {
        if let Some(reader) = &mut self.reader {
            let pins = &mut reader.chat.pins;
            pins.replace = None;
            pins.open = match pins.open {
                Some(_) => None,
                None => {
                    pins.runs += 1;
                    Some(pins.runs)
                }
            };
            cx.notify();
        }
    }

    /// A drag in the list ended: the pins take their new order.
    fn drop_pin(&mut self, cx: &mut Context<Self>) {
        let Some(reader) = &mut self.reader else {
            return;
        };
        let pins = &mut reader.chat.pins;
        let Some((from, to)) = pins.drag.take() else {
            return;
        };
        cx.notify();
        if from == to || from >= pins.list.len() {
            return;
        }
        let pin = pins.list.remove(from);
        pins.list.insert(to.min(pins.list.len()), pin);
        pins.at = 0;
        let ids: Vec<i64> = pins.list.iter().map(|p| p.id).collect();
        if ids.iter().all(|id| *id != 0) {
            self.send(Command::OrderChatPins(ids), None, None, false, cx);
        }
    }

    /// Who sent message `id`, by first name, and the day it came.
    fn pin_source(&self, id: MessageId) -> (String, String) {
        let Some(part) = self
            .reader
            .as_ref()
            .and_then(|r| r.parts.iter().find(|p| p.id == id))
        else {
            return Default::default();
        };
        let view = part.body.as_ref().and_then(|b| b.view.as_ref());
        let name = match view.and_then(|v| v.from.first()) {
            Some(from) if self.is_me(&from.email) => tr!("chat-you"),
            Some(from) => first_name(from.label()).to_owned(),
            None => part
                .row
                .as_ref()
                .map(|r| first_name(&r.correspondent).to_owned())
                .unwrap_or_default(),
        };
        let now = jiff::Timestamp::now().as_second();
        let today = format::local(now, &self.tz).map(|d| d.date());
        let when = part
            .row
            .as_ref()
            .and_then(|r| r.date)
            .or(view.and_then(|v| v.date))
            .and_then(|d| format::local(d, &self.tz))
            .map(|d| day_label(d, today))
            .unwrap_or_default();
        (name, when)
    }

    /// The pin's picture in the list: the file's badge, or a chat or mail
    /// mark.
    fn pin_mark(&self, pin: &ChatPin, th: &Theme) -> AnyElement {
        if let Pinned::File(ix) = pin.what
            && let Some(a) = self
                .reader
                .as_ref()
                .and_then(|r| r.parts.iter().find(|p| p.id == pin.message))
                .and_then(|p| p.body.as_ref()?.view.as_ref()?.attachments.get(ix).cloned())
        {
            return div()
                .flex_none()
                .size(px(28.0))
                .flex()
                .items_center()
                .justify_center()
                .child(kind_badge(katna_preview::kind(&a.mime, &a.name), 26.0))
                .into_any_element();
        }
        let name = if matches!(pin.what, Pinned::Text(_)) {
            "chat"
        } else {
            "mail"
        };
        div()
            .flex_none()
            .size(px(28.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(7.0))
            .bg(rgba(th.chip))
            .child(icon(name, th.text_dim, 15.0))
            .into_any_element()
    }

    /// The thin bar under the header: the pin it is on, with marks for
    /// which of them, and the list button.
    pub(super) fn render_pin_bar(&self, th: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let pins = &self.reader.as_ref()?.chat.pins;
        let pin = pins.list.get(pins.at)?;
        let count = pins.list.len();
        let marks = (0..count).map(|n| {
            div()
                .w(px(2.5))
                .h(px(if count > 3 { 6.0 } else { 8.0 }))
                .rounded(px(2.0))
                .bg(rgba(if n == pins.at {
                    th.text_dim
                } else {
                    th.divider
                }))
        });
        let label = match &pin.what {
            Pinned::Text(_) => format!("“{}”", pin.label),
            _ => pin.label.clone(),
        };
        Some(
            div()
                .id("chat-pin-bar")
                .flex_none()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(10.0))
                .pl(px(16.0))
                .pr(px(10.0))
                .py(px(6.0))
                .border_b_1()
                .border_color(rgba(th.divider))
                .cursor_pointer()
                .hover(|s| s.bg(rgba(th.hover)))
                .on_click(cx.listener(|this, _, _, cx| this.jump_to_chat_pin(cx)))
                .child(
                    div()
                        .flex_none()
                        .flex()
                        .flex_col()
                        .gap(px(2.0))
                        .children(marks),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .text_size(px(11.5))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgba(th.text_dim))
                                .child(tr!("chat-pinned-of", at = pins.at + 1, count = count)),
                        )
                        .child(
                            div()
                                .truncate()
                                .text_size(px(12.5))
                                .text_color(rgba(th.text_faint))
                                .child(label),
                        ),
                )
                .child(
                    icon_button_colored("chat-pin-list", "list-bulleted", 18.0, th.text_dim, th)
                        .size(px(28.0))
                        .tooltip(tip(tr!("chat-pins-all"), th))
                        .on_click(cx.listener(|this, _, _, cx| {
                            cx.stop_propagation();
                            this.toggle_pin_list(cx);
                        })),
                )
                .into_any_element(),
        )
    }

    /// One pin as a row of the list or of the replace question.
    fn pin_row(&self, pin: &ChatPin, th: &Theme) -> gpui::Div {
        let (name, when) = self.pin_source(pin.message);
        let from = match pin.what {
            Pinned::Mail => tr!("chat-pin-from-mail", name = name, when = when),
            Pinned::File(_) => tr!("chat-pin-from-file", name = name, when = when),
            Pinned::Text(_) => tr!("chat-pin-from-text", name = name, when = when),
        };
        let label = match &pin.what {
            Pinned::Text(_) => format!("“{}”", pin.label),
            _ => pin.label.clone(),
        };
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(10.0))
            .px(px(8.0))
            .py(px(6.0))
            .rounded(px(8.0))
            .child(self.pin_mark(pin, th))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .truncate()
                            .text_size(px(13.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgba(th.text))
                            .child(label),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(px(11.5))
                            .text_color(rgba(th.text_faint))
                            .child(from),
                    ),
            )
    }

    /// A panel dropped over the feed from the pin bar, with a scrim that
    /// folds it.
    fn pin_panel(
        &self,
        id: &'static str,
        run: usize,
        panel: gpui::Stateful<gpui::Div>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let reduce = cx.reduce_motion();
        div()
            .absolute()
            .inset_0()
            .child(
                div()
                    .id("chat-pins-scrim")
                    .absolute()
                    .inset_0()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.fold_chat_pins(cx);
                        }),
                    ),
            )
            .child(
                panel
                    .absolute()
                    .top(px(6.0))
                    .left(px(10.0))
                    .right(px(10.0))
                    .max_h(relative(0.85))
                    .overflow_y_scroll()
                    .occlude()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .with_animation(
                        (id, run),
                        gpui::Animation::new(katna_ui::motion::time(
                            std::time::Duration::from_millis(if reduce { 1 } else { 220 }),
                        ))
                        .with_easing(gpui::ease_out_quint()),
                        |el, t| el.opacity(t).mt(px(-8.0 * (1.0 - t))),
                    ),
            )
            .into_any_element()
    }

    /// All pins, from the bar's list button: drag to reorder, × to unpin.
    pub(super) fn render_pin_list(&self, th: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let pins = &self.reader.as_ref()?.chat.pins;
        let run = pins.open?;
        if pins.list.is_empty() {
            return None;
        }
        // While a row is dragged, the rows stand in the order it would
        // leave.
        let mut order: Vec<usize> = (0..pins.list.len()).collect();
        if let Some((from, to)) = pins.drag {
            let moved = order.remove(from);
            order.insert(to.min(order.len()), moved);
        }
        let dragged = pins.drag.map(|(from, _)| from);
        let rows = order.into_iter().enumerate().map(|(slot, ix)| {
            let pin = &pins.list[ix];
            let pin_id = pin.id;
            self.pin_row(pin, th)
                .id(("chat-pin-row", slot))
                .cursor_grab()
                .when(dragged == Some(ix), |d| d.bg(rgba(th.hover)).opacity(0.85))
                .when(dragged.is_none(), |d| d.hover(|s| s.bg(rgba(th.hover))))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        if let Some(reader) = &mut this.reader {
                            reader.chat.pins.drag = Some((ix, slot));
                            cx.notify();
                        }
                    }),
                )
                .on_mouse_move(cx.listener(move |this, e: &gpui::MouseMoveEvent, _, cx| {
                    if e.pressed_button != Some(MouseButton::Left) {
                        return;
                    }
                    if let Some(reader) = &mut this.reader
                        && let Some((from, to)) = reader.chat.pins.drag
                        && to != slot
                    {
                        reader.chat.pins.drag = Some((from, slot));
                        cx.notify();
                    }
                }))
                .on_click(cx.listener(move |this, _, _, cx| {
                    // A click without a drag goes to the pin's bubble.
                    let Some(reader) = &mut this.reader else {
                        return;
                    };
                    if let Some(at) = reader.chat.pins.list.iter().position(|p| p.id == pin_id) {
                        reader.chat.pins.at = at;
                        reader.chat.pins.open = None;
                        this.jump_to_chat_pin(cx);
                    }
                }))
                .child(
                    icon_button_colored(("chat-unpin", slot), "close", 16.0, th.text_faint, th)
                        .size(px(26.0))
                        .tooltip(tip(tr!("chat-unpin"), th))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.unpin_in_chat(pin_id, cx);
                        })),
                )
        });
        let panel = div()
            .id("chat-pin-list")
            .p(px(6.0))
            .flex()
            .flex_col()
            .map(|d| raised(d, th, 12.0, 3.0))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, cx| this.drop_pin(cx)),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, cx| this.drop_pin(cx)),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .justify_between()
                    .px(px(8.0))
                    .pt(px(4.0))
                    .pb(px(6.0))
                    .text_size(px(11.5))
                    .text_color(rgba(th.text_faint))
                    .child(tr!(
                        "chat-pins-heading",
                        count = pins.list.len(),
                        most = MAX_CHAT_PINS
                    ))
                    .when(pins.list.len() > 1, |d| d.child(tr!("chat-pins-drag"))),
            )
            .children(rows);
        Some(self.pin_panel("chat-pin-list-in", run, panel, cx))
    }

    /// A sixth pin: which of the five it replaces, the oldest picked.
    pub(super) fn render_pin_replace(
        &self,
        th: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let pins = &self.reader.as_ref()?.chat.pins;
        let replace = pins.replace.as_ref()?;
        let rows = pins.list.iter().enumerate().map(|(ix, pin)| {
            let pin_id = pin.id;
            let on = pin.id == replace.chosen;
            self.pin_row(pin, th)
                .id(("chat-pin-replace", ix))
                .cursor_pointer()
                .when(on, |d| d.bg(rgba(th.hover)))
                .when(!on, |d| d.hover(|s| s.bg(rgba(th.hover))))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(replace) = this
                        .reader
                        .as_mut()
                        .and_then(|r| r.chat.pins.replace.as_mut())
                    {
                        replace.chosen = pin_id;
                        cx.notify();
                    }
                }))
                .child(radio(on, th))
        });
        let button = |id: &'static str, label: String, primary: bool| {
            div()
                .id(id)
                .h(px(32.0))
                .px(px(16.0))
                .flex()
                .items_center()
                .rounded_full()
                .cursor_pointer()
                .text_size(px(13.0))
                .font_weight(FontWeight::SEMIBOLD)
                .when(primary, |d| {
                    d.bg(rgba(th.accent))
                        .text_color(rgba(th.on_accent))
                        .hover(|s| s.opacity(0.9))
                })
                .when(!primary, |d| {
                    d.text_color(rgba(th.text)).hover(|s| s.bg(rgba(th.hover)))
                })
                .child(label)
        };
        let panel = div()
            .id("chat-pin-replace")
            .p(px(6.0))
            .flex()
            .flex_col()
            .map(|d| raised(d, th, 12.0, 3.0))
            .child(
                div()
                    .px(px(8.0))
                    .pt(px(6.0))
                    .text_size(px(14.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgba(th.text))
                    .child(tr!("chat-pins-replace-title")),
            )
            .child(
                div()
                    .px(px(8.0))
                    .pt(px(2.0))
                    .pb(px(6.0))
                    .text_size(px(12.5))
                    .text_color(rgba(th.text_dim))
                    .child(tr!("chat-pins-replace-hint")),
            )
            .children(rows)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .justify_end()
                    .gap(px(6.0))
                    .px(px(4.0))
                    .pt(px(8.0))
                    .pb(px(4.0))
                    .child(
                        button("chat-pins-cancel", tr!("chat-pins-cancel"), false).on_click(
                            cx.listener(|this, _, _, cx| {
                                this.fold_chat_pins(cx);
                            }),
                        ),
                    )
                    .child(
                        button("chat-pins-replace", tr!("chat-pins-replace"), true).on_click(
                            cx.listener(|this, _, _, cx| {
                                let Some(replace) = this
                                    .reader
                                    .as_mut()
                                    .and_then(|r| r.chat.pins.replace.take())
                                else {
                                    return;
                                };
                                let chosen = (replace.chosen != 0).then_some(replace.chosen);
                                this.place_pin(
                                    replace.message,
                                    replace.what,
                                    replace.label,
                                    chosen,
                                    cx,
                                );
                            }),
                        ),
                    ),
            );
        Some(self.pin_panel("chat-pin-replace-in", 0, panel, cx))
    }
}

/// A round choice mark, filled when `on`.
fn radio(on: bool, th: &Theme) -> AnyElement {
    div()
        .flex_none()
        .size(px(18.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .border_2()
        .border_color(rgba(if on { th.accent } else { th.text_faint }))
        .when(on, |d| {
            d.child(div().size(px(8.0)).rounded_full().bg(rgba(th.accent)))
        })
        .into_any_element()
}
