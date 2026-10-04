// SPDX-License-Identifier: GPL-3.0-or-later

//! The chat view (Settings > Experimental > Reading): a conversation
//! between people shown as a group chat. Each mail is a bubble holding only
//! what its sender wrote, the user's own on the right; the quoted mail and
//! the signature wait behind a ··· pill, a forwarded mail is a small card,
//! and attachments show as chat media: pictures in a grid, files as cards.
//! Mail in a row from one person within a few minutes forms a group with
//! one name and one picture. Newsletters keep the usual view, and the
//! header's Chat | Mail switch turns any conversation either way.
//!
//! The reply box at the bottom is the inline reply (`compose/chat_box.rs`);
//! a reply just sent shows its undo countdown beside its bubble.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::Instant;

use gpui::{
    AnimationExt, AnyElement, ClipboardItem, Context, Div, FontWeight, MouseButton, MouseDownEvent,
    SharedString, Window, div, prelude::*, relative, rgba,
};
use katna_i18n::tr;
use katna_preview::Kind as FileKind;
use katna_render::Attachment;
use katna_render::trim::{self, Forwarded};
use katna_store::{MessageId, Pinned};
use katna_ui::motion::{self, Spring, lerp};
use katna_ui::{px, unpx};

use super::super::attachments::{Thumb, kind_badge};
use super::super::compose::Kind;
use super::super::context_menu::Rows;
use super::super::select::Pieces;
use super::super::{MailWindow, Menu, rich};
use super::{Conversation, Part, first_name, key_number, read, tracking};
use crate::daemon::Command;
use crate::data::Mail;
use crate::format;
use crate::theme::{Theme, avatar_color, fade, mix};
use crate::widgets::{icon, icon_button, icon_button_colored, tip};

mod pins;

/// The header's list of people: how long each row takes to slide in, and
/// how long after the one above it starts.
const PEOPLE_ROW_MS: f32 = 260.0;
const PEOPLE_DELAY_MS: f32 = 40.0;
/// The header's pictures, and how far each overlaps the one before.
const STACK_PICTURE: f32 = 26.0;
const STACK_STEP: f32 = 22.0;
/// Frames the feed is kept at its end after mail comes or is sent.
const SETTLE_FRAMES: u8 = 4;
/// Mail from one person this close together joins their group.
const GROUP_SECONDS: i64 = 10 * 60;
/// The picture beside a group.
const PICTURE: f32 = 28.0;
/// The room the picture takes beside other people's bubbles.
const PICTURE_COLUMN: f32 = PICTURE + 8.0;
/// A bubble's corners: round, and tighter where it joins its group.
const ROUND: f32 = 16.0;
const JOINED: f32 = 6.0;
/// Pictures shown in a bubble's grid; more show as "+N" on the last.
const GRID: usize = 4;
/// Inline pictures smaller than this are logos and signature icons.
const SMALL_PICTURE: u64 = 12 * 1024;
/// How long a bubble is held on a phone before its menu opens, and how far
/// the finger may stray meanwhile.
const LONG_PRESS: std::time::Duration = std::time::Duration::from_millis(450);
const PRESS_SLOP: f32 = 10.0;

/// The go-down button's size, and how far from the end (in screens) the
/// feed must be before it shows.
const DOWN_SIZE: f32 = 40.0;
const DOWN_AFTER: f32 = 1.0;
/// How long the glide to the end takes, and the most it glides through.
const GLIDE: std::time::Duration = std::time::Duration::from_millis(360);
const GLIDE_SCREENS: f32 = 1.5;
/// Room kept between a long bubble's picture or hover buttons and the
/// edges of the feed in sight.
const STICK_GAP: f32 = 8.0;
/// The hover buttons' height.
const HOVER_BAR: f32 = 32.0;

/// How the open conversation shows as a chat.
#[derive(Default)]
pub(in crate::window) struct ChatState {
    /// The user's pick for this conversation: Chat (`true`) or Mail.
    pub(in crate::window) pick: Option<bool>,
    /// Every message's body is read, as the chat shows each one.
    pub(in crate::window) all_bodies: bool,
    /// Bubbles whose quoted text and signature show.
    open: HashSet<MessageId>,
    /// What each message says, trimmed, with the length of the body it
    /// was trimmed from.
    said: RefCell<HashMap<MessageId, (usize, Rc<Said>)>>,
    /// Messages shown when the chat last scrolled to its end.
    shown: usize,
    /// Frames left to keep the feed at its end: the newest bubble and the
    /// reply box settle over a few frames.
    settle: u8,
    /// The contact panel's width when the chat last drew, to keep the
    /// feed at its end while the panel slides in or out.
    room: f32,
    /// The list of everyone in the chat, open from the header: which
    /// opening it is, so each one plays its slide again.
    people: Option<usize>,
    people_runs: usize,
    people_arrow: crate::widgets::Fold,
    /// The reply box's height as last drawn, and as the feed last saw
    /// it: chips or the formatting bar growing it keep a feed at its end
    /// there.
    reply_drawn: Rc<Cell<f32>>,
    reply_seen: f32,
    /// The feed keeps to its end: it reached it and was not scrolled up
    /// since, so whatever grows or shrinks around it, it stays there.
    stuck: bool,
    /// [`Self::stuck`] as the feed's last drawn frame reads it: checked
    /// once the layout is final, so a change landing after this view
    /// drew still brings the feed back to its end.
    held: Rc<Cell<bool>>,
    /// Up to five things pinned to the top.
    pub(super) pins: pins::Pins,
    /// A bubble held down on a phone, and which press that is: held long
    /// enough, it opens the bubble's menu.
    press: Option<(MessageId, usize)>,
    presses: usize,
    /// Where each bubble's row was last drawn, top and bottom, in the
    /// window: a long bubble keeps its picture and hover buttons in the
    /// part of it in sight.
    spans: Rc<RefCell<HashMap<MessageId, (f32, f32)>>>,
    /// The part of the feed in sight this frame, top and bottom, from the
    /// feed's top.
    sight: Cell<(f32, f32)>,
    /// How much the go-down button shows, 0 to 1.
    down: Option<Spring>,
    /// Messages there were when the feed left its end: newer ones are
    /// counted on the go-down button.
    left_at: Option<usize>,
    /// Which glide to the end is running; scrolling stops it.
    glides: usize,
}

/// Someone in the chat, as the header's list shows them.
struct Member {
    name: String,
    email: String,
    /// Mails they sent in this conversation.
    mails: usize,
    me: bool,
}

/// One mail as a bubble shows it.
pub(in crate::window) struct Said {
    /// What the sender wrote.
    pub(in crate::window) text: String,
    quoted: Option<String>,
    pub(in crate::window) signature: Option<String>,
    forwarded: Option<Forwarded<String>>,
}

impl Said {
    fn of(body: &str) -> Self {
        let trimmed = trim::plain(body);
        let mut quoted = trimmed.quoted.filter(|q| !q.trim().is_empty());
        // A forward in a reply ends where the quoted mail starts: that
        // goes behind ··· like any quote.
        let forwarded = trimmed.forwarded.map(|mut forwarded| {
            let inner = trim::plain(&forwarded.body);
            if let Some(more) = inner.quoted.filter(|q| !q.trim().is_empty()) {
                forwarded.body = inner.said;
                quoted = Some(match quoted.take() {
                    Some(q) => format!("{more}\n\n{q}"),
                    None => more,
                });
            }
            forwarded
        });
        Self {
            text: trimmed.said.trim().to_owned(),
            quoted,
            signature: trimmed.signature.as_deref().and_then(signature_lines),
            forwarded,
        }
    }

    /// Moves the lines this mail ends with to its signature when another
    /// of the sender's mails (`others`) ends with them too.
    fn cut_shared_tail<'a>(&mut self, others: impl Iterator<Item = &'a str>) {
        for other in others {
            let other = trim::plain(other).said;
            if let Some(at) = trim::shared_tail(&self.text, &other) {
                self.signature = Some(self.text[at..].trim().to_owned());
                self.text.truncate(at);
                self.text.truncate(self.text.trim_end().len());
                return;
            }
        }
    }

    /// Something waits behind its ··· pill.
    fn hides(&self) -> bool {
        self.quoted.is_some() || self.signature.is_some()
    }
}

impl Conversation {
    /// Reads the body of every message not read yet, for the chat.
    fn read_all_bodies(&mut self, mail: &Mail) {
        for part in &mut self.parts {
            if part.body.is_none() {
                part.body = Some(read(mail, part.id));
            }
        }
        self.chat.all_bodies = true;
    }

    /// Mail between people: none of it is a newsletter or other bulk mail
    /// from someone else, and not all of it is drafts.
    fn between_people(&self, is_me: impl Fn(&str) -> bool) -> bool {
        let bulk = self.parts.iter().any(|p| {
            p.body
                .as_ref()
                .and_then(|b| b.view.as_ref())
                .is_some_and(|v| v.bulk && !v.from.first().is_some_and(|a| is_me(&a.email)))
        });
        let drafts = self.parts.iter().all(|p| self.drafts.contains(&p.id));
        !bulk && !drafts
    }

    /// What message `part` says, trimmed; `None` while it is not read.
    fn said(&self, part: &Part) -> Option<Rc<Said>> {
        let view = part.body.as_ref()?.view.as_ref()?;
        let body = &view.body;
        if let Some((len, said)) = self.chat.said.borrow().get(&part.id)
            && *len == body.len()
        {
            return Some(said.clone());
        }
        let mut said = Said::of(body);
        if said.signature.is_none()
            && let Some(from) = view.from.first()
        {
            said.cut_shared_tail(self.bodies_from(&from.email, part.id));
        }
        let said = Rc::new(said);
        self.chat
            .said
            .borrow_mut()
            .insert(part.id, (body.len(), said.clone()));
        Some(said)
    }

    /// The bodies of the other mail `email` sent in this conversation.
    fn bodies_from<'a>(&'a self, email: &'a str, not: MessageId) -> impl Iterator<Item = &'a str> {
        self.parts
            .iter()
            .filter(move |p| p.id != not)
            .filter_map(move |p| {
                let view = p.body.as_ref()?.view.as_ref()?;
                let from = view.from.first()?;
                from.email
                    .eq_ignore_ascii_case(email)
                    .then_some(view.body.as_str())
            })
    }

    /// The signature `email` last signed with in this conversation.
    pub(in crate::window) fn signature_of(&self, email: &str) -> Option<String> {
        self.parts.iter().rev().find_map(|part| {
            let from = part.body.as_ref()?.view.as_ref()?.from.first()?;
            if !from.email.eq_ignore_ascii_case(email) {
                return None;
            }
            self.said(part)?.signature.clone()
        })
    }
}

/// One bubble of the feed, worked out before it is drawn.
struct Bubble {
    ix: usize,
    id: MessageId,
    mine: bool,
    name: String,
    email: String,
    date: Option<i64>,
    said: Option<Rc<Said>>,
    /// The attachments it shows, with their place among all of them.
    files: Vec<(usize, Attachment)>,
    /// A reply just sent: `Some(false)` while it waits to go.
    pending: Option<bool>,
    /// The first and last of its group.
    first: bool,
    last: bool,
}

/// What the feed shows, in order.
enum Line {
    Day(String),
    /// "Arjun added Sara".
    Joined(String, Vec<String>),
    /// "Sara changed the subject to “Goa, final plan”".
    Renamed(String, String),
    Bubble(Bubble),
}

impl MailWindow {
    /// The open conversation shows as a chat: the chat view is on, and the
    /// conversation is mail between people or the user picked Chat.
    pub(in crate::window) fn chat_shown(&self) -> bool {
        self.config.experimental.chat_view
            && self.reading
            && self.reader.as_ref().is_some_and(|r| {
                !r.parts.is_empty()
                    && r.chat
                        .pick
                        .unwrap_or_else(|| r.between_people(|e| self.is_me(e)))
            })
    }

    /// The chat view was turned on or off: the open conversation follows
    /// it rather than an earlier pick.
    pub(in crate::window) fn open_chat_as_set(&mut self) {
        if let Some(reader) = &mut self.reader {
            reader.chat.pick = None;
            reader.chat.shown = 0;
        }
    }

    /// Reads what the chat needs before the open conversation is drawn.
    pub(super) fn prepare_chat(&mut self) {
        if !self.config.experimental.chat_view {
            return;
        }
        if let (Some(reader), Ok(mail)) = (&mut self.reader, &self.mail)
            && (!reader.chat.all_bodies || reader.parts.iter().any(|p| p.body.is_none()))
        {
            reader.read_all_bodies(mail);
        }
        self.read_chat_pins();
    }

    /// Shows the open conversation as a chat (`true`) or as mail.
    pub(in crate::window) fn pick_chat(
        &mut self,
        chat: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(reader) = &mut self.reader else {
            return;
        };
        reader.chat.pick = Some(chat);
        reader.chat.shown = 0;
        let key = reader.key;
        // A reply being written goes along, with its cursor where it was:
        // the mail view scrolls down to it, as when it opened. One with
        // nothing in it closes, keeping no draft.
        if self.chat_compose(key).is_some() && !self.drop_empty_reply(cx) {
            if !chat {
                self.reveal_inline_reply(cx);
            }
            // After the click that picked it, which focuses the window.
            if let Some(body) = self.chat_reply_focus(cx) {
                window.defer(cx, move |window, cx| window.focus(&body, cx));
            }
        }
        cx.notify();
    }

    /// The Chat | Mail switch, in the chat's header and above the mail.
    pub(super) fn chat_switch(&self, chat: bool, th: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let side = |id: &'static str, name: &'static str, label: String, on: bool, to: bool| {
            div()
                .id(id)
                .h(px(28.0))
                .px(px(10.0))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(6.0))
                .rounded_full()
                .text_size(px(13.0))
                .font_weight(FontWeight::MEDIUM)
                .cursor_pointer()
                .when(on, |d| {
                    d.bg(rgba(th.surface))
                        .text_color(rgba(th.text))
                        .shadow(crate::widgets::elevation(th, 0.5))
                })
                .when(!on, |d| {
                    d.text_color(rgba(th.text_dim))
                        .hover(|s| s.text_color(rgba(th.text)))
                })
                .child(icon(name, if on { th.text } else { th.text_dim }, 16.0))
                .child(label)
                .on_click(cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    this.pick_chat(to, window, cx);
                }))
        };
        div()
            .flex_none()
            .p(px(2.0))
            .flex()
            .flex_row()
            .rounded_full()
            .bg(rgba(th.chip))
            .child(side(
                "chat-pick-chat",
                "chat",
                tr!("chat-switch-chat"),
                chat,
                true,
            ))
            .child(side(
                "chat-pick-mail",
                "mail",
                tr!("chat-switch-mail"),
                !chat,
                false,
            ))
            .into_any_element()
    }

    /// The feed's lines: day separators, people joining, and bubbles.
    fn chat_lines(&self) -> Vec<Line> {
        let Some(reader) = &self.reader else {
            return Vec::new();
        };
        let now = jiff::Timestamp::now().as_second();
        let today = format::local(now, &self.tz).map(|d| d.date());
        let mut lines = Vec::new();
        let mut day = None;
        let mut people: HashSet<String> = HashSet::new();
        let mut subject: Option<String> = None;
        for (ix, part) in reader.parts.iter().enumerate() {
            let view = part.body.as_ref().and_then(|b| b.view.as_ref());
            let row = part.row.as_ref();
            let (name, email) = match (view.and_then(|v| v.from.first()), row) {
                (Some(from), _) => (from.label().to_owned(), from.email.clone()),
                (None, Some(row)) => (row.correspondent.clone(), row.sender.clone()),
                (None, None) => (tr!("reader-unknown-sender"), String::new()),
            };
            let mine = part.pending.is_some() || self.is_me(&email);
            let date = row.and_then(|r| r.date).or(view.and_then(|v| v.date));
            let local = date.and_then(|d| format::local(d, &self.tz));
            if let Some(local) = local
                && day != Some(local.date())
            {
                day = Some(local.date());
                lines.push(Line::Day(day_label(local, today)));
            }
            if let Some(view) = view {
                let everyone = view.from.iter().chain(&view.to).chain(&view.cc);
                let new: Vec<String> = everyone
                    .filter(|a| !self.is_me(&a.email))
                    .filter(|a| people.insert(a.email.to_lowercase()))
                    .filter(|a| !a.email.eq_ignore_ascii_case(&email))
                    .map(|a| first_name(a.label()).to_owned())
                    .collect();
                if ix > 0 && !new.is_empty() {
                    let who = if mine {
                        tr!("chat-you")
                    } else {
                        first_name(&name).to_owned()
                    };
                    lines.push(Line::Joined(who, new));
                }
                people.insert(email.to_lowercase());
            }
            // A new subject, not just "Re:" added, shows as a line.
            let said = view
                .map(|v| v.subject.as_str())
                .or(row.map(|r| r.subject.as_str()))
                .unwrap_or_default();
            let key = katna_core::subject::normalize_subject(said).text;
            if !key.is_empty() {
                if subject.as_ref().is_some_and(|before| *before != key) {
                    let who = if mine {
                        tr!("chat-you")
                    } else {
                        first_name(&name).to_owned()
                    };
                    lines.push(Line::Renamed(who, bare_subject(said, &key)));
                }
                subject = Some(key);
            }
            let files = view
                .map(|v| {
                    v.attachments
                        .iter()
                        .enumerate()
                        .filter(|(_, a)| !(a.content_id.is_some() && a.size < SMALL_PICTURE))
                        .map(|(ix, a)| (ix, a.clone()))
                        .collect()
                })
                .unwrap_or_default();
            // Joins the bubble before when nothing came between them.
            let joins = match lines.last_mut() {
                Some(Line::Bubble(before)) => {
                    let near = match (before.date, date) {
                        (Some(a), Some(b)) => (b - a).abs() <= GROUP_SECONDS,
                        _ => true,
                    };
                    let same = before.mine == mine && before.email.eq_ignore_ascii_case(&email);
                    if same && near {
                        before.last = false;
                        true
                    } else {
                        false
                    }
                }
                _ => false,
            };
            lines.push(Line::Bubble(Bubble {
                ix,
                id: part.id,
                mine,
                name,
                email,
                date,
                said: reader.said(part),
                files,
                pending: part.pending,
                first: !joins,
                last: true,
            }));
        }
        lines
    }

    /// The open conversation as a chat: its header, the feed and the reply
    /// box.
    pub(super) fn render_chat(&mut self, th: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let lines = self.chat_lines();
        let room = self.contact_room();
        let phone = self.layout.shape.is_phone();
        // A wheel turn past either end moves the offset before layout puts
        // it back, so it's held within the feed here: the picture and the
        // hover buttons stay put when nothing can scroll.
        let max = unpx(self.reader_scroll.max_offset().y).max(0.0);
        let scrolled = (-unpx(self.reader_scroll.offset().y)).clamp(0.0, max);
        let at_end = scrolled >= max - 4.0;
        let screen = unpx(self.reader_scroll.bounds().size.height);
        let from_end = max - scrolled;
        let reduce = cx.reduce_motion();
        let Some(reader) = &mut self.reader else {
            return div().into_any_element();
        };
        let key = reader.key;
        reader.chat.sight.set((scrolled, scrolled + screen));
        // A screen up from the end, the go-down button fades in; within half
        // a screen of it, out.
        if at_end {
            reader.chat.left_at = None;
        } else if reader.chat.left_at.is_none() {
            reader.chat.left_at = Some(reader.parts.len());
        }
        let down = reader
            .chat
            .down
            .get_or_insert_with(|| Spring::new(motion::SMOOTH, 0.0));
        if screen > 0.0 && from_end > screen * DOWN_AFTER {
            down.set(1.0);
        } else if from_end < screen * DOWN_AFTER * 0.5 {
            down.set(0.0);
        }
        let down_t = down.step(reduce).clamp(0.0, 1.0);
        if !down.settled() {
            cx.notify();
        }
        let newer = reader
            .chat
            .left_at
            .map_or(0, |at| reader.parts.len().saturating_sub(at));
        // Opens at its newest mail, and goes there when mail comes or is
        // sent.
        if reader.chat.shown != reader.parts.len() {
            reader.chat.shown = reader.parts.len();
            reader.chat.settle = SETTLE_FRAMES;
        }
        // The panel sliding in or out reflows the bubbles: a feed at its
        // end stays there.
        if (room - reader.chat.room).abs() > 0.5 {
            reader.chat.room = room;
            if at_end {
                reader.chat.settle = reader.chat.settle.max(SETTLE_FRAMES);
            }
        }
        // A feed kept to its end stays there as the reply box or the
        // bubbles grow or shrink, whichever frame the change lands in;
        // scrolling up lets go of the end.
        let reply_height = reader.chat.reply_drawn.get();
        let change = reply_height - reader.chat.reply_seen;
        if change.abs() > 0.5 {
            reader.chat.reply_seen = reply_height;
            let near_end = scrolled >= max - 4.0 - change.abs();
            reader.chat.stuck |= near_end;
        }
        if at_end {
            reader.chat.stuck = true;
        } else if reader.chat.stuck && reader.chat.settle == 0 {
            reader.chat.settle = 1;
        }
        reader.chat.held.set(reader.chat.stuck);
        let held = reader.chat.held.clone();
        let feed_top = reader.chat.pins.feed_top.clone();
        let scroll = self.reader_scroll.clone();
        if reader.chat.settle > 0 {
            reader.chat.settle -= 1;
            self.reader_scroll.scroll_to_bottom();
            cx.notify();
        }
        let header = self.chat_header(th, cx);
        let feed: Vec<AnyElement> = lines
            .iter()
            .map(|line| match line {
                Line::Day(label) => crate::widgets::tag(label.clone(), th)
                    .self_center()
                    .my(px(8.0))
                    .into_any_element(),
                Line::Joined(who, names) => div()
                    .self_center()
                    .max_w(relative(0.8))
                    .my(px(4.0))
                    .text_size(px(12.0))
                    .text_color(rgba(th.text_faint))
                    .child(tr!(
                        "chat-added",
                        who = who.clone(),
                        names = names.join(", ")
                    ))
                    .into_any_element(),
                Line::Renamed(who, subject) => div()
                    .self_center()
                    .max_w(relative(0.8))
                    .my(px(4.0))
                    .text_size(px(12.0))
                    .text_color(rgba(th.text_faint))
                    .child(tr!(
                        "chat-renamed",
                        who = who.clone(),
                        subject = subject.clone()
                    ))
                    .into_any_element(),
                Line::Bubble(bubble) => self.render_bubble_row(bubble, th, cx),
            })
            .collect();
        self.adopt_chat_reply(key, cx);
        let people = self.chat_people();
        let names: Vec<&str> = people.iter().map(|(n, _)| first_name(n)).collect();
        let slide = self.chat_format_slide(key, cx);
        let reply =
            self.render_chat_reply(key, &names.join(", "), self.chat_aimed(key), slide, th, cx);
        let drawn = self
            .reader
            .as_ref()
            .map(|r| r.chat.reply_drawn.clone())
            .unwrap_or_default();
        let reply = div().relative().flex_none().child(reply).child(
            gpui::canvas(
                move |bounds, window, _| {
                    let height = katna_ui::unpx(bounds.size.height);
                    if (drawn.get() - height).abs() > 0.5 {
                        drawn.set(height);
                        // GPUI ignores `refresh` while it draws: the next
                        // frame redraws this view instead.
                        window.request_animation_frame();
                    }
                },
                |_, _, _, _| {},
            )
            .absolute()
            .size_full(),
        );
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .relative()
                    .child(
                        self.reader_bar.draw(
                            "chat-bar",
                            &self.reader_scroll,
                            div()
                                .id("reader")
                                .size_full()
                                .overflow_y_scroll()
                                .on_scroll_wheel(cx.listener(
                                    |this, e: &gpui::ScrollWheelEvent, window, cx| {
                                        let up = e.delta.pixel_delta(window.line_height()).y;
                                        if let Some(reader) = &mut this.reader {
                                            // The wheel takes over from a glide.
                                            reader.chat.glides += 1;
                                            if unpx(up) > 0.0 {
                                                reader.chat.stuck = false;
                                                reader.chat.held.set(false);
                                                reader.chat.settle = 0;
                                            }
                                        }
                                        // Scrolling the chat folds its summary.
                                        this.fold_chat_summary(cx);
                                    },
                                ))
                                .track_scroll(&self.reader_scroll)
                                .child(
                                    div()
                                        .min_h_full()
                                        .flex()
                                        .flex_col()
                                        .justify_end()
                                        .gap(px(2.0))
                                        .px(px(if phone { 10.0 } else { 16.0 }))
                                        .pt(px(self.reader_head.get() + 12.0))
                                        .pb(px(8.0))
                                        .children(feed)
                                        .map(|d| self.text_area(d, cx))
                                        .child(
                                            gpui::canvas(
                                                move |bounds, window, _| {
                                                    feed_top.set(unpx(bounds.origin.y));
                                                    let off = -unpx(scroll.offset().y);
                                                    let max = unpx(scroll.max_offset().y);
                                                    if held.get() && off < max - 4.0 {
                                                        scroll.scroll_to_bottom();
                                                        window.request_animation_frame();
                                                    }
                                                },
                                                |_, _, _, _| {},
                                            )
                                            .absolute()
                                            .top_0()
                                            .left_0()
                                            .size_0(),
                                        ),
                                ),
                            // The bar the open mail has.
                            th.text_dim & 0xffff_ff00 | 0x99,
                        ),
                    )
                    .children(self.render_chat_down(down_t, newer, th, cx))
                    // The header, its summary strip and pins stay at the
                    // top while the chat scrolls under them; what they
                    // drop down hangs from their foot.
                    .child(
                        self.pinned_head(
                            div()
                                .flex()
                                .flex_col()
                                .child(header)
                                .children(self.render_chat_summary_strip(th, cx))
                                .children(self.render_pin_bar(th, cx)),
                            th.chat_pane(),
                            false,
                            th,
                        ),
                    )
                    .child(
                        div()
                            .absolute()
                            .top(px(self.reader_head.get()))
                            .left_0()
                            .right_0()
                            .bottom_0()
                            .children(self.render_chat_summary_drop(th, cx))
                            .children(self.render_chat_people(th, cx))
                            .children(self.render_pin_list(th, cx))
                            .children(self.render_pin_replace(th, cx)),
                    )
                    // Where the link under the pointer really goes.
                    .children(
                        self.hovered_link
                            .as_ref()
                            .map(|link| rich::link_status(&link.url, th)),
                    ),
            )
            .child(reply)
            .children(self.render_text_menu(th, cx))
            .with_animation(
                ("open-chat", key_number(key)),
                gpui::Animation::new(katna_ui::motion::time(std::time::Duration::from_millis(
                    280,
                )))
                .with_easing(gpui::ease_out_quint()),
                |el, t| el.opacity(t),
            )
            .into_any_element()
    }

    /// The round button that takes the feed down to its newest mail, once
    /// it is scrolled a screen up: at the feed's bottom right, above the
    /// reply box, `t` shown, with the mail that came since it left the end.
    fn render_chat_down(
        &self,
        t: f32,
        newer: usize,
        th: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if t <= 0.001 {
            return None;
        }
        let badge = (newer > 0).then(|| {
            div()
                .absolute()
                .top(px(-6.0))
                .right(px(-4.0))
                .min_w(px(18.0))
                .h(px(18.0))
                .px(px(5.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .bg(rgba(th.accent))
                .text_size(px(11.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgba(th.on_accent))
                .child(katna_i18n::format::number(newer as u64))
        });
        Some(
            div()
                .absolute()
                // On the send button's centre line: the same size, as far
                // in as the reply box's right padding.
                .right(px(12.0))
                .bottom(px(12.0 + lerp(-12.0, 0.0, t)))
                .opacity(t)
                .child(
                    div()
                        .id("chat-down")
                        .occlude()
                        .relative()
                        .size(px(DOWN_SIZE))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .bg(rgba(th.raised))
                        .cursor_pointer()
                        .shadow(crate::widgets::elevation(th, 2.0))
                        .hover(|s| s.shadow(crate::widgets::elevation(th, 3.0)))
                        .tooltip(tip(tr!("chat-go-down"), th))
                        .on_mouse_move(|_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(|this, _, _, cx| this.glide_chat_to_end(cx)))
                        .child(icon("arrow-down", th.text, 22.0))
                        .children(badge),
                )
                .into_any_element(),
        )
    }

    /// Glides the feed down to its end, from at most a screen and a half
    /// above it, and keeps it there.
    fn glide_chat_to_end(&mut self, cx: &mut Context<Self>) {
        let Some(reader) = &mut self.reader else {
            return;
        };
        reader.chat.glides += 1;
        let run = reader.chat.glides;
        let scroll = self.reader_scroll.clone();
        let screen = unpx(scroll.bounds().size.height);
        let end = -unpx(scroll.max_offset().y);
        let at = unpx(scroll.offset().y);
        let from = at.max(end + screen * GLIDE_SCREENS);
        scroll.set_offset(gpui::point(scroll.offset().x, px(from)));
        let reduce = cx.reduce_motion();
        cx.spawn(async move |this, cx| {
            let start = Instant::now();
            loop {
                let t = if reduce {
                    1.0
                } else {
                    (start.elapsed().as_secs_f32() / GLIDE.as_secs_f32()).min(1.0)
                };
                let eased = 1.0 - (1.0 - t).powi(3);
                // The end can move while it glides.
                let end = -unpx(scroll.max_offset().y);
                let going = this.update(cx, |this, cx| {
                    let Some(reader) = &mut this.reader else {
                        return false;
                    };
                    if reader.chat.glides != run {
                        return false;
                    }
                    scroll.set_offset(gpui::point(
                        scroll.offset().x,
                        px(from + (end - from) * eased),
                    ));
                    if t >= 1.0 {
                        reader.chat.stuck = true;
                        reader.chat.held.set(true);
                    }
                    cx.notify();
                    t < 1.0
                });
                if !going.unwrap_or(false) {
                    return;
                }
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(16))
                    .await;
            }
        })
        .detach();
        cx.notify();
    }

    /// How far above its row's bottom a long bubble's picture stands, so
    /// it stays in sight while the bubble's end is scrolled out below,
    /// and where the hover buttons stand from the row's top: in the middle
    /// of the part of the bubble in sight. `None` before the row was drawn.
    fn stick(&self, id: MessageId) -> Option<(f32, f32)> {
        let reader = self.reader.as_ref()?;
        let (top, bottom) = *reader.chat.spans.borrow().get(&id)?;
        let feed_top = reader.chat.pins.feed_top.get();
        let (top, bottom) = (top - feed_top, bottom - feed_top);
        let (seen_top, seen_bottom) = reader.chat.sight.get();
        let height = bottom - top;
        let lift = (bottom - (seen_bottom - STICK_GAP)).clamp(0.0, (height - PICTURE).max(0.0));
        let from = top.max(seen_top + STICK_GAP);
        let to = bottom.min(seen_bottom - STICK_GAP);
        let middle = if to > from {
            (from + to) / 2.0
        } else {
            (top + bottom) / 2.0
        };
        let bar = (middle - HOVER_BAR / 2.0 - top).clamp(0.0, (height - HOVER_BAR).max(0.0));
        Some((lift, bar))
    }

    /// The conversation's people other than the user, names and addresses,
    /// in the order they came in.
    fn chat_people(&self) -> Vec<(String, String)> {
        let Some(reader) = &self.reader else {
            return Vec::new();
        };
        let mut seen = HashSet::new();
        reader
            .parts
            .iter()
            .flat_map(|p| match p.body.as_ref().and_then(|b| b.view.as_ref()) {
                Some(v) => v
                    .from
                    .iter()
                    .chain(&v.to)
                    .chain(&v.cc)
                    .map(|a| (a.label().to_owned(), a.email.clone()))
                    .collect(),
                // Until its mail is read, its sender from the list, so the
                // header never says only "and you".
                None => p
                    .row
                    .as_ref()
                    .map(|r| vec![(r.correspondent.clone(), r.sender.clone())])
                    .unwrap_or_default(),
            })
            .filter(|(_, email)| {
                !email.is_empty() && !self.is_me(email) && seen.insert(email.to_lowercase())
            })
            .collect()
    }

    /// Everyone in the chat: those who wrote, newest first, then those
    /// only written to, then the user.
    fn chat_members(&self) -> Vec<Member> {
        let Some(reader) = &self.reader else {
            return Vec::new();
        };
        let views: Vec<_> = reader
            .parts
            .iter()
            .filter_map(|p| p.body.as_ref()?.view.as_ref())
            .collect();
        let sent = |email: &str| {
            views
                .iter()
                .filter(|v| v.from.iter().any(|a| a.email.eq_ignore_ascii_case(email)))
                .count()
        };
        let mut seen = HashSet::new();
        let mut me: Option<Member> = None;
        let mut members = Vec::new();
        let writers = views.iter().rev().flat_map(|v| v.from.iter());
        let others = views.iter().flat_map(|v| v.to.iter().chain(&v.cc));
        for a in writers.chain(others) {
            if !a.email.contains('@') || !seen.insert(a.email.to_lowercase()) {
                continue;
            }
            let member = Member {
                name: a.label().to_owned(),
                email: a.email.clone(),
                mails: sent(&a.email),
                me: self.is_me(&a.email),
            };
            if member.me {
                me.get_or_insert(member);
            } else {
                members.push(member);
            }
        }
        members.extend(me);
        members
    }

    /// Opens or folds the list of everyone in the chat under the header.
    fn toggle_chat_people(&mut self, cx: &mut Context<Self>) {
        if let Some(reader) = &mut self.reader {
            let chat = &mut reader.chat;
            chat.people = match chat.people {
                Some(_) => None,
                None => {
                    chat.people_runs += 1;
                    Some(chat.people_runs)
                }
            };
            cx.notify();
        }
    }

    fn close_chat_people(&mut self, cx: &mut Context<Self>) {
        self.fold_chat_people(cx);
    }

    /// Folds the chat's list of people, if it is open: what Esc does
    /// first.
    pub(in crate::window) fn fold_chat_people(&mut self, cx: &mut Context<Self>) -> bool {
        let open = self
            .reader
            .as_mut()
            .is_some_and(|reader| reader.chat.people.take().is_some());
        if open {
            cx.notify();
        }
        open
    }

    /// The list of everyone in the chat, dropped over the feed from the
    /// header; the people slide in one after another.
    fn render_chat_people(&self, th: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let phone = self.layout.shape.is_phone();
        let reader = self.reader.as_ref()?;
        let run = reader.chat.people?;
        let key = reader.key;
        let members = self.chat_members();
        let reduce = cx.reduce_motion();
        let rows = members.into_iter().enumerate().map(|(ix, m)| {
            let email = m.email.to_lowercase();
            let row = div()
                .id(("chat-member", ix))
                .relative()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(12.0))
                .px(px(10.0))
                .py(px(7.0))
                .rounded(px(10.0))
                .cursor_pointer()
                .child(crate::widgets::hover_fade("hover-glow", Some(10.0), th))
                .tooltip(tip(tr!("chat-show-card"), th))
                .on_hover({
                    let email = email.clone();
                    cx.listener(move |this, hovered: &bool, _, cx| {
                        if *hovered {
                            this.read_person_ahead(&email, cx);
                        }
                    })
                })
                .on_click(cx.listener(move |this, e: &gpui::ClickEvent, _, cx| {
                    this.close_chat_people(cx);
                    let at = this.person_at(("chat-member", ix), e.position());
                    this.show_contact_of(key, &email, at, cx);
                }))
                .child(self.person_spot(("chat-member", ix)))
                .child(self.person_avatar(&m.name, &m.email, 30.0))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap(px(6.0))
                                .child(
                                    div()
                                        .min_w_0()
                                        .truncate()
                                        .text_size(px(14.0))
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(rgba(th.text))
                                        .child(m.name.clone()),
                                )
                                .when(m.me, |d| {
                                    d.child(
                                        div()
                                            .flex_none()
                                            .px(px(6.0))
                                            .rounded(px(6.0))
                                            .bg(rgba(mix(th.menu, th.text, 0.12)))
                                            .text_size(px(11.0))
                                            .text_color(rgba(th.text_dim))
                                            .child(tr!("chat-you")),
                                    )
                                }),
                        )
                        .child(
                            div()
                                .truncate()
                                .text_size(px(12.5))
                                .text_color(rgba(th.text_dim))
                                .child(m.email.clone()),
                        ),
                )
                .child(
                    div()
                        .flex_none()
                        .text_size(px(12.5))
                        .text_color(rgba(th.text_faint))
                        .child(tr!("chat-member-mails", count = m.mails)),
                );
            // Each row starts a little after the one above.
            let delay = PEOPLE_DELAY_MS * ix as f32;
            let total = delay + PEOPLE_ROW_MS;
            row.with_animation(
                ("chat-member-in", run * 1000 + ix),
                gpui::Animation::new(katna_ui::motion::time(std::time::Duration::from_millis(
                    if reduce { 1 } else { total as u64 },
                ))),
                move |el, t| {
                    let local = ((t * total - delay) / PEOPLE_ROW_MS).clamp(0.0, 1.0);
                    let eased = 1.0 - (1.0 - local).powi(3);
                    el.opacity(eased).top(px(-6.0 * (1.0 - eased)))
                },
            )
        });
        let count = self.chat_members().len();
        let panel = div()
            .id("chat-people")
            .absolute()
            .top(px(6.0))
            .left(px(if phone { 8.0 } else { 12.0 }))
            // A phone's spans the chat.
            .when(phone, |d| d.right(px(8.0)))
            .when(!phone, |d| d.w(px(360.0)).max_w(relative(0.9)))
            .max_h(relative(0.8))
            .overflow_y_scroll()
            .map(|d| crate::widgets::raised(d, th, 14.0, 3.0))
            // Pointer and clicks stay on the list, not the bubbles under it.
            .occlude()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .px(px(18.0))
                    .pt(px(12.0))
                    .pb(px(4.0))
                    .text_size(px(12.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgba(th.text_dim))
                    .child(tr!("chat-people-heading", count = count)),
            )
            .child(
                div()
                    .px(px(8.0))
                    .pb(px(10.0))
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .children(rows),
            )
            .with_animation(
                ("chat-people-in", run),
                gpui::Animation::new(katna_ui::motion::time(std::time::Duration::from_millis(
                    if reduce { 1 } else { 220 },
                )))
                .with_easing(gpui::ease_out_quint()),
                |el, t| el.opacity(t).mt(px(-8.0 * (1.0 - t))),
            );
        Some(
            div()
                .absolute()
                .inset_0()
                // A click beside the list folds it.
                .child(
                    div()
                        .id("chat-people-scrim")
                        .absolute()
                        .inset_0()
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _, _, cx| this.close_chat_people(cx)),
                        ),
                )
                .child(panel)
                .into_any_element(),
        )
    }

    /// When the reply being written answers an older mail, or its sender
    /// only: who sent that mail, and the start of what they said.
    fn chat_aimed(&self, key: crate::data::EntryKey) -> Option<(String, String)> {
        let reader = self.reader.as_ref()?;
        let (source, all) = self.chat_reply_source(key)?;
        if all && reader.view_id(None) == Some(source) {
            return None;
        }
        let part = reader.parts.iter().find(|p| p.id == source)?;
        let from = part.body.as_ref()?.view.as_ref()?.from.first()?;
        let name = first_name(from.label()).to_owned();
        let said = reader
            .said(part)
            .and_then(|s| {
                s.text
                    .lines()
                    .find(|l| !l.trim().is_empty())
                    .map(str::to_owned)
            })
            .unwrap_or_default();
        Some((name, said))
    }

    /// The people's pictures, the subject with who is in it, and the
    /// Chat | Mail switch.
    fn chat_header(&self, th: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let Some(reader) = &self.reader else {
            return div().into_any_element();
        };
        let people = self.chat_people();
        let names: Vec<&str> = people.iter().map(|(n, _)| first_name(n)).collect();
        let mails = reader.parts.iter().filter(|p| p.pending.is_none()).count();
        let people_open = reader.chat.people.is_some();
        // Overlapping pictures, each ringed in the card's colour.
        let shown = people.len().min(3);
        let stack = people.iter().take(3).enumerate().map(|(i, (name, email))| {
            div()
                .absolute()
                .top_0()
                .left(px(STACK_STEP * i as f32))
                .size(px(STACK_PICTURE + 4.0))
                .rounded_full()
                .border_2()
                .border_color(rgba(th.surface))
                .bg(rgba(th.surface))
                .child(self.person_avatar(name, email, STACK_PICTURE))
        });
        // A phone's header takes the toolbar's place: Back before it, More
        // after it, and the Chat | Mail switch in More.
        let phone = self.layout.shape.is_phone();
        let back = phone.then(|| {
            icon_button("chat-back", "back", 20.0, th)
                .tooltip(tip(tr!("reader-back"), th))
                .on_click(cx.listener(|this, _, window, cx| {
                    this.close_message(&super::super::CloseMessage, window, cx)
                }))
        });
        let end = if phone {
            let more = icon_button("chat-more", "more", 20.0, th)
                .when(self.menu != Some(Menu::ReaderMore), |d| {
                    d.tooltip(tip(tr!("reader-more"), th))
                })
                .on_click(cx.listener(|this, _, _, cx| this.toggle_menu(Menu::ReaderMore, cx)));
            self.with_menu(more, Menu::ReaderMore, th, cx)
                .into_any_element()
        } else {
            self.chat_switch(true, th, cx)
        };
        div()
            .flex_none()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(if phone { 4.0 } else { 10.0 }))
            .pl(px(if phone { 8.0 } else { 16.0 }))
            .pr(px(if phone { 8.0 } else { 12.0 }))
            .py(px(if phone { 6.0 } else { 10.0 }))
            .border_b_1()
            .border_color(rgba(th.divider))
            .children(back)
            .child(
                div()
                    .id("chat-header-people")
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(10.0))
                    .ml(px(-6.0))
                    .pl(px(6.0))
                    .py(px(4.0))
                    .rounded(px(10.0))
                    .cursor_pointer()
                    .relative()
                    .child(crate::widgets::hover_fade("hover-glow", Some(10.0), th))
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_chat_people(cx)))
                    .child(
                        div()
                            .flex_none()
                            .relative()
                            .h(px(STACK_PICTURE + 4.0))
                            .w(px(STACK_PICTURE
                                + 4.0
                                + STACK_STEP * shown.saturating_sub(1) as f32))
                            .children(stack),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(15.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgba(th.text))
                                    .child(reader.subject.clone()),
                            )
                            .child(
                                div()
                                    .min_w_0()
                                    .text_size(px(12.0))
                                    .text_color(rgba(th.text_faint))
                                    .child(
                                        div()
                                            .flex()
                                            .flex_row()
                                            .items_center()
                                            .gap(px(2.0))
                                            .child(div().min_w_0().truncate().child(tr!(
                                                "chat-people",
                                                names = names.join(", "),
                                                count = mails
                                            )))
                                            .child(div().flex_none().child(
                                                crate::widgets::fold_arrow(
                                                    "chat-people-arrow",
                                                    &reader.chat.people_arrow,
                                                    people_open,
                                                    th.text_faint,
                                                    16.0,
                                                ),
                                            )),
                                    ),
                            ),
                    ),
            )
            .children(self.summary_pill(th, cx))
            .child(end)
            .into_any_element()
    }

    /// A bubble with the sender's picture (others' last in a group), its
    /// undo countdown (the user's, while it waits) and the hover buttons.
    fn render_bubble_row(&self, bubble: &Bubble, th: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let group = SharedString::from(format!("bubble-{}", bubble.id.0));
        let id = bubble.id;
        let key = self.reader.as_ref().map(|r| r.key);
        let email = bubble.email.clone();
        let (lift, bar_top) = self.stick(bubble.id).unwrap_or_default();
        let picture = (!bubble.mine).then(|| {
            let slot = div()
                .w(px(PICTURE_COLUMN))
                .flex_none()
                .flex()
                .items_end()
                .pb(px(lift));
            if bubble.last {
                let pick = email.clone();
                let ix = bubble.ix;
                slot.child(
                    div()
                        .id(("chat-picture", bubble.ix))
                        .relative()
                        .cursor_pointer()
                        .tooltip(tip(tr!("chat-show-card"), th))
                        .on_hover({
                            let pick = pick.clone();
                            cx.listener(move |this, hovered: &bool, _, cx| {
                                if *hovered {
                                    this.read_person_ahead(&pick, cx);
                                }
                            })
                        })
                        .on_click(cx.listener(move |this, e: &gpui::ClickEvent, _, cx| {
                            if let Some(key) = key {
                                let at = this.person_at(("chat-picture", ix), e.position());
                                this.show_contact_of(key, &pick, at, cx);
                            }
                        }))
                        .child(self.person_spot(("chat-picture", bubble.ix)))
                        .child(self.person_avatar(&bubble.name, &bubble.email, PICTURE)),
                )
            } else {
                slot
            }
        });
        let pending = bubble.pending.is_some();
        // The hover buttons sit on the bubble's inner side. A phone has no
        // hover: a long press opens the menu instead.
        let phone = self.layout.shape.is_phone();
        let hover =
            (!pending && !phone).then(|| self.bubble_hover(bubble, group.clone(), bar_top, th, cx));
        let (before, after) = if bubble.mine {
            (hover, None)
        } else {
            (None, hover)
        };
        let undo = self.undo_beside(bubble, th, cx);
        let pins = self.reader.as_ref().map(|r| &r.chat.pins);
        let tops = pins.map(|p| p.tops.clone());
        let spans = self.reader.as_ref().map(|r| r.chat.spans.clone());
        let flash = pins
            .filter(|p| {
                p.flash.is_some_and(|(m, at)| {
                    m == id && at.elapsed().as_secs_f32() * 1000.0 < pins::FLASH_MS
                })
            })
            .map(|p| p.flashes);
        let row = div()
            .id(("chat-row", bubble.ix))
            .group(group)
            .relative()
            .rounded(px(12.0))
            .w_full()
            .flex()
            .flex_row()
            .items_end()
            .gap(px(6.0))
            .when(bubble.first, |d| d.mt(px(6.0)))
            .when(bubble.mine, |d| d.justify_end())
            .when(!pending, |d| {
                d.on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                        this.open_bubble_menu(id, None, e.position, cx);
                    }),
                )
            })
            // Held anywhere on it, text included, before the text takes
            // the press.
            .when(!pending && phone, |d| {
                d.capture_any_mouse_down(cx.listener(
                    move |this, e: &MouseDownEvent, window, cx| {
                        if e.button == MouseButton::Left {
                            this.press_bubble(id, e.position, window, cx);
                        }
                    },
                ))
                .capture_any_mouse_up(cx.listener(|this, _, _, _| {
                    if let Some(reader) = &mut this.reader {
                        reader.chat.press = None;
                    }
                }))
            })
            .children(picture)
            .children(before)
            .children(undo)
            .child(self.render_bubble(bubble, th, cx))
            .children(after)
            // Where it stands, for a pin's jump.
            .children(tops.zip(spans).map(|(tops, spans)| {
                gpui::canvas(
                    move |bounds, window, _| {
                        tops.borrow_mut().insert(id, unpx(bounds.origin.y));
                        let span = (unpx(bounds.top()), unpx(bounds.bottom()));
                        let before = spans.borrow_mut().insert(id, span);
                        // Drawn somewhere new in the feed: its picture and
                        // buttons follow next frame.
                        if before.is_some_and(|b| (b.1 - b.0 - (span.1 - span.0)).abs() > 0.5) {
                            window.request_animation_frame();
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full()
            }));
        // A pin's bubble, just jumped to, glows for a moment.
        match flash {
            Some(run) if !cx.reduce_motion() => {
                let tint = th.hover;
                row.with_animation(
                    ("chat-flash", run),
                    gpui::Animation::new(katna_ui::motion::time(std::time::Duration::from_millis(
                        pins::FLASH_MS as u64,
                    ))),
                    move |el, t| el.bg(rgba(fade(tint, (1.0 - t).powi(2)))),
                )
                .into_any_element()
            }
            _ => row.into_any_element(),
        }
    }

    /// One run of a bubble's text (`which`: what was said, its signature
    /// or its quotes) in a holder `style` shapes, its links opening.
    #[allow(clippy::too_many_arguments)]
    fn linked_piece(
        &self,
        pieces: &mut Pieces,
        ix: usize,
        which: usize,
        text: String,
        anchors: &[(String, String)],
        style: impl FnOnce(Div) -> Div,
        th: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let hover = self.reader.as_ref().map(|r| rich::Links {
            window: cx.weak_entity(),
            conversation: key_number(r.key),
            part: ix,
        });
        rich::linked_piece(
            pieces,
            text.into(),
            anchors,
            style,
            ix * 3 + which,
            hover,
            th,
        )
    }

    /// The bubble itself.
    fn render_bubble(&self, bubble: &Bubble, th: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let id = bubble.id;
        let open = self
            .reader
            .as_ref()
            .is_some_and(|r| r.chat.open.contains(&id));
        let (top, bottom) = (
            if bubble.first { ROUND } else { JOINED },
            if bubble.last { ROUND } else { JOINED },
        );
        let name = (!bubble.mine && bubble.first).then(|| {
            let pick = bubble.email.clone();
            let key = self.reader.as_ref().map(|r| r.key);
            let ix = bubble.ix;
            div()
                .id(("chat-name", ix))
                .relative()
                .self_start()
                .cursor_pointer()
                .text_size(px(13.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgba(name_color(&bubble.email, th)))
                .hover(|s| s.underline())
                .on_hover({
                    let pick = pick.clone();
                    cx.listener(move |this, hovered: &bool, _, cx| {
                        if *hovered {
                            this.read_person_ahead(&pick, cx);
                        }
                    })
                })
                .on_click(cx.listener(move |this, e: &gpui::ClickEvent, _, cx| {
                    if let Some(key) = key {
                        let at = this.person_at(("chat-name", ix), e.position());
                        this.show_contact_of(key, &pick, at, cx);
                    }
                }))
                .child(self.person_spot(("chat-name", ix)))
                .child(bubble.name.clone())
        });
        let said = bubble.said.as_ref();
        // A mail in another language offers its translation, as in Mail,
        // and shows it in place of what was said.
        let part = self.reader.as_ref().and_then(|r| r.parts.get(bubble.ix));
        let translation = part
            .filter(|_| bubble.pending.is_none() && said.is_some())
            .and_then(|p| {
                let body = p.body.as_ref()?;
                let text = &body.view.as_ref()?.body;
                self.translation_bar(bubble.ix, id, text, body.encrypted(), true, th, cx)
            });
        let translated = self
            .translated_text(id)
            .map(|text| Said::of(&text).text)
            .filter(|t| !t.is_empty());
        // Its text, signature and quotes are selectable, to copy or pin,
        // and their links open as in Mail: the mail's own links, and
        // addresses written out.
        let anchors = part
            .filter(|_| translated.is_none())
            .and_then(|p| p.body.as_ref()?.doc.as_ref())
            .map(rich::anchors)
            .unwrap_or_default();
        let mut pieces = self.text.pieces(bubble.ix, th);
        let text = translated
            .or_else(|| said.map(|s| s.text.clone()))
            .filter(|t| !t.is_empty())
            .map(|t| {
                let text = self.linked_piece(&mut pieces, bubble.ix, 0, t, &anchors, |d| d, th, cx);
                self.selectable_body(bubble.ix, div().child(text), cx)
            });
        let not_read = said.is_none().then(|| {
            div()
                .text_color(rgba(th.text_faint))
                .child(tr!("chat-not-downloaded"))
        });
        let forwarded = said
            .and_then(|s| s.forwarded.as_ref())
            .map(|f| forwarded_card(f, open, th));
        let pill = said.filter(|s| s.hides()).map(|_| {
            div()
                .id(("chat-more", bubble.ix))
                .self_start()
                .mt(px(4.0))
                .px(px(8.0))
                .h(px(20.0))
                .flex()
                .items_center()
                .rounded_full()
                .bg(rgba(th.hover))
                .cursor_pointer()
                .text_size(px(12.0))
                .text_color(rgba(th.text_dim))
                .hover(|s| s.text_color(rgba(th.text)))
                .tooltip(tip(
                    if open {
                        tr!("chat-hide-quoted")
                    } else {
                        tr!("chat-show-quoted")
                    },
                    th,
                ))
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    if let Some(reader) = &mut this.reader
                        && !reader.chat.open.remove(&id)
                    {
                        reader.chat.open.insert(id);
                    }
                    cx.notify();
                }))
                .child(if open {
                    tr!("chat-hide-dots")
                } else {
                    "···".to_owned()
                })
        });
        let hidden = said.filter(|_| open).map(|s| {
            let signature = s.signature.clone().map(|sig| {
                self.linked_piece(
                    &mut pieces,
                    bubble.ix,
                    1,
                    sig,
                    &anchors,
                    |d| d.text_color(rgba(th.text_dim)),
                    th,
                    cx,
                )
            });
            let quoted = s.quoted.clone().map(|quoted| {
                self.linked_piece(
                    &mut pieces,
                    bubble.ix,
                    2,
                    quoted,
                    &anchors,
                    |d| {
                        d.pl(px(10.0))
                            .border_l_2()
                            .border_color(rgba(th.outline))
                            .text_color(rgba(th.text_faint))
                    },
                    th,
                    cx,
                )
            });
            let hidden = div()
                .mt(px(6.0))
                .flex()
                .flex_col()
                .gap(px(6.0))
                .text_size(px(13.0))
                .children(signature)
                .children(quoted);
            self.selectable_body(bubble.ix, hidden, cx)
        });
        let media = self.bubble_media(bubble, th, cx);
        let meta = self.bubble_meta(bubble, th, cx);
        div()
            .id(("chat-bubble", bubble.ix))
            .max_w(relative(if self.layout.shape.is_phone() {
                0.82
            } else {
                0.72
            }))
            .min_w_0()
            .flex()
            .flex_col()
            .px(px(11.0))
            .pt(px(7.0))
            .pb(px(5.0))
            .bg(rgba(if bubble.mine {
                th.bubble_own()
            } else {
                th.bubble_other()
            }))
            .map(|d| {
                // Round, with the corners along the group's side joined.
                if bubble.mine {
                    d.rounded_tl(px(ROUND))
                        .rounded_bl(px(ROUND))
                        .rounded_tr(px(top))
                        .rounded_br(px(bottom))
                } else {
                    d.rounded_tr(px(ROUND))
                        .rounded_br(px(ROUND))
                        .rounded_tl(px(top))
                        .rounded_bl(px(bottom))
                }
            })
            .when(bubble.pending == Some(false), |d| d.opacity(0.75))
            .text_size(px(14.0))
            .line_height(px(20.0))
            .text_color(rgba(th.text))
            .children(name)
            .children(translation)
            .children(text)
            .children(not_read)
            .children(forwarded)
            .children(media)
            .children(pill)
            .children(hidden)
            .child(meta)
            .into_any_element()
    }

    /// The time, and for the user's own mail whether it went and, when it
    /// was sent with tracking, whether it was seen: the ticks turn the
    /// eye's color, and hovering the time or ticks opens who saw it, as
    /// the eye does in the mail view.
    fn bubble_meta(&self, bubble: &Bubble, th: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let time = bubble
            .date
            .and_then(|d| format::local(d, &self.tz))
            .map(katna_i18n::format::time)
            .unwrap_or_default();
        let pinned = self
            .reader
            .as_ref()
            .is_some_and(|r| r.chat.pins.of(bubble.id).next().is_some())
            .then(|| icon("pin", th.text_faint, 12.0));
        let part = self.reader.as_ref().and_then(|r| r.parts.get(bubble.ix));
        let seen = part
            .filter(|_| bubble.mine && bubble.pending.is_none())
            .and_then(|part| Some((part, self.seen_state(part)?)));
        let state = bubble.mine.then(|| match bubble.pending {
            Some(false) => icon("schedule", th.text_faint, 13.0),
            _ => {
                let color = tracking::seen_color(seen.is_some_and(|(_, s)| s), th);
                // The popover of who saw it points at the ticks.
                div()
                    .relative()
                    .flex()
                    .child(icon("done-all", color, 15.0))
                    .children(seen.map(|(part, _)| Self::seen_spot(part)))
                    .into_any_element()
            }
        });
        // Seen more than once, or a link followed: how many times of
        // each, in a faint pill.
        let count = seen
            .map(|(part, _)| self.seen_counts(part))
            .filter(|(opens, clicks)| *opens > 1 || *clicks > 0)
            .map(|(opens, clicks)| {
                let number = |name: &'static str, n: u32| {
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(3.0))
                        .child(icon(name, th.text_faint, 11.0))
                        .child(katna_i18n::format::number(n as u64))
                };
                div()
                    .mr(px(3.0))
                    .h(px(16.0))
                    .px(px(5.0))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(5.0))
                    .rounded_full()
                    .border_1()
                    .border_color(rgba(th.divider))
                    .text_color(rgba(th.text_dim))
                    .when(opens > 0, |d| d.child(number("eye", opens)))
                    .when(opens > 0 && clicks > 0, |d| {
                        d.child(div().w(px(1.0)).h(px(9.0)).bg(rgba(th.divider)))
                    })
                    .when(clicks > 0, |d| d.child(number("link", clicks)))
            });
        let meta = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(3.0))
            .text_size(px(11.0))
            .line_height(px(14.0))
            .text_color(rgba(th.text_faint))
            .children(count)
            .children(pinned)
            .child(time)
            .children(state);
        let Some((part, _)) = seen else {
            return meta.self_end().mt(px(2.0)).into_any_element();
        };
        let ix = bubble.ix;
        let target = meta.id(("chat-seen", ix)).cursor_pointer();
        div()
            .self_end()
            .mt(px(2.0))
            .child(self.seen_anchor(ix, part, target, false, cx))
            .children(self.seen_popover(ix, part, th, cx))
            .into_any_element()
    }

    /// Pictures in a grid, other files as cards.
    fn bubble_media(
        &self,
        bubble: &Bubble,
        th: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if bubble.files.is_empty() || bubble.pending.is_some() {
            return None;
        }
        let id = bubble.id;
        let kind = |a: &Attachment| katna_preview::kind(&a.mime, &a.name);
        let (pictures, others): (Vec<_>, Vec<_>) = bubble
            .files
            .iter()
            .partition(|(_, a)| matches!(kind(a), FileKind::Picture(_)));
        let more = pictures.len().saturating_sub(GRID);
        let shown = pictures.len().min(GRID);
        let tiles = pictures.iter().take(GRID).enumerate().map(|(n, (ix, a))| {
            let ix = *ix;
            let thumb = self.files.thumb(id, ix);
            let wide = shown == 1 || (shown == 3 && n == 0);
            div()
                .id(("chat-photo", id.0 as usize * 64 + ix))
                .relative()
                .when(wide, |d| d.w_full())
                .when(!wide, |d| d.w(relative(0.49)))
                .h(px(if shown == 1 { 180.0 } else { 110.0 }))
                .rounded(px(10.0))
                .overflow_hidden()
                .bg(rgba(th.chip))
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    this.open_attachment(id, ix, window, cx);
                }))
                .child(match thumb {
                    Some(Thumb::Picture { sharp, .. }) => {
                        gpui::img(gpui::ImageSource::Render(sharp))
                            .size_full()
                            .object_fit(gpui::ObjectFit::Cover)
                            .into_any_element()
                    }
                    _ => div()
                        .size_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(kind_badge(kind(a), 28.0))
                        .into_any_element(),
                })
                .when(n + 1 == GRID && more > 0, |d| {
                    d.child(
                        div()
                            .absolute()
                            .top_0()
                            .left_0()
                            .size_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(rgba(0x0000_0080))
                            .text_size(px(22.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgba(0xffff_ffff))
                            .child(format!("+{}", katna_i18n::format::number(more as u64))),
                    )
                })
        });
        let grid = (shown > 0).then(|| {
            div()
                .mt(px(4.0))
                .w(px(280.0))
                .max_w_full()
                .flex()
                .flex_row()
                .flex_wrap()
                .justify_between()
                .gap_y(px(4.0))
                .children(tiles)
        });
        let cards = others.into_iter().map(|(ix, a)| {
            let ix = *ix;
            let video = a.mime.starts_with("video/");
            div()
                .id(("chat-file", id.0 as usize * 64 + ix))
                .mt(px(4.0))
                .w(px(250.0))
                .max_w_full()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(10.0))
                .px(px(10.0))
                .py(px(8.0))
                .rounded(px(11.0))
                .bg(rgba(th.surface))
                .cursor_pointer()
                // Under the pointer it lifts a little, with half the app's
                // hover tint laid over it: solid, as the bubble shows
                // through it otherwise.
                .hover(|s| {
                    let tint = (th.hover & 0xff) as f32 / 255.0 * 0.5;
                    s.bg(rgba(mix(th.surface, th.hover | 0xff, tint)))
                        .shadow(crate::widgets::elevation(th, 1.0))
                })
                .on_click(cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    this.open_attachment(id, ix, window, cx);
                }))
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                        this.open_bubble_menu(id, Some(ix), e.position, cx);
                    }),
                )
                .child(if video {
                    div()
                        .size(px(32.0))
                        .flex_none()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(6.4))
                        .bg(rgba(0x5f63_68ff))
                        .child(icon("play", 0xffff_ffff, 20.0))
                        .into_any_element()
                } else {
                    kind_badge(kind(a), 32.0)
                })
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
                                .child(a.name.clone()),
                        )
                        .child(
                            div()
                                .text_size(px(12.0))
                                .line_height(px(16.0))
                                .text_color(rgba(th.text_faint))
                                .child(format::size(a.size)),
                        ),
                )
        });
        Some(
            div()
                .flex()
                .flex_col()
                .children(grid)
                .children(cards)
                .into_any_element(),
        )
    }

    /// The buttons that show while the pointer is over a bubble: Reply all
    /// and more.
    fn bubble_hover(
        &self,
        bubble: &Bubble,
        group: SharedString,
        top: f32,
        th: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = bubble.id;
        let pinned = self
            .reader
            .as_ref()
            .is_some_and(|r| r.chat.pins.find(id, &Pinned::Mail).is_some());
        // In the middle of the part of the bubble in sight.
        let bar = div()
            .flex_none()
            .self_start()
            .mt(px(top))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(2.0))
            .p(px(2.0))
            .rounded_full()
            .bg(rgba(th.raised))
            .shadow(crate::widgets::elevation(th, 1.0))
            // Shown on hover; not `hidden()`, which GPUI cannot switch
            // between layout and paint.
            .opacity(0.0)
            .group_hover(group, |s| s.opacity(1.0))
            .child(
                icon_button_colored(
                    ("chat-reply-all", bubble.ix),
                    "reply-all",
                    18.0,
                    th.text_dim,
                    th,
                )
                .size(px(28.0))
                .tooltip(tip(tr!("chat-reply-all"), th))
                .on_click(cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    this.chat_reply(Some(id), Kind::ReplyAll, window, cx);
                })),
            )
            .child(
                icon_button_colored(
                    ("chat-bubble-pin", bubble.ix),
                    if pinned { "pin-filled" } else { "pin" },
                    18.0,
                    th.text_dim,
                    th,
                )
                .size(px(28.0))
                .tooltip(tip(
                    if pinned {
                        tr!("chat-unpin")
                    } else {
                        tr!("chat-pin")
                    },
                    th,
                ))
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.toggle_chat_pin(id, Pinned::Mail, cx);
                })),
            )
            .child(
                icon_button_colored(
                    ("chat-bubble-more", bubble.ix),
                    "more",
                    18.0,
                    th.text_dim,
                    th,
                )
                .size(px(28.0))
                .tooltip(tip(tr!("chat-more"), th))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                        this.open_bubble_menu(id, None, e.position, cx);
                    }),
                ),
            );
        bar.into_any_element()
    }

    /// For the user's reply waiting to go: the seconds left to take it
    /// back, and Undo.
    fn undo_beside(
        &self,
        bubble: &Bubble,
        th: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if bubble.pending != Some(false) {
            return None;
        }
        let key = self.reader.as_ref()?.key;
        let (outbox, until, total) = self
            .sending
            .cards(key)
            .find(|c| c.id == bubble.id)
            .and_then(|c| c.countdown())?;
        let left = until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return None;
        }
        let share = left.as_secs_f32() / total.as_secs_f32().max(0.001);
        Some(
            div()
                .flex_none()
                .self_center()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(6.0))
                .child(super::super::countdown_ring(
                    share,
                    left.as_secs_f32().ceil() as u64,
                    th.text_dim,
                    th.divider,
                    24.0,
                ))
                .children(outbox.map(|outbox| {
                    div()
                        .id(("chat-undo", bubble.ix))
                        .h(px(24.0))
                        .px(px(11.0))
                        .flex()
                        .items_center()
                        .rounded_full()
                        .bg(rgba(th.chip))
                        .cursor_pointer()
                        .text_size(px(12.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgba(th.text))
                        .relative()
                        .child(crate::widgets::hover_fade("hover-glow", None, th))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            cx.stop_propagation();
                            this.run_undo(Command::UndoSend(outbox), window, cx);
                        }))
                        .child(tr!("chat-undo"))
                }))
                .into_any_element(),
        )
    }

    /// Bubble `id` was pressed on a phone at `at`: held there long enough,
    /// its menu rises.
    fn press_bubble(
        &mut self,
        id: MessageId,
        at: gpui::Point<gpui::Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(reader) = &mut self.reader else {
            return;
        };
        reader.chat.presses += 1;
        let run = reader.chat.presses;
        reader.chat.press = Some((id, run));
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(LONG_PRESS).await;
            let _ = this.update_in(cx, |this, window, cx| {
                let Some(reader) = &mut this.reader else {
                    return;
                };
                if reader.chat.press != Some((id, run)) {
                    return;
                }
                reader.chat.press = None;
                let now = window.mouse_position();
                let strayed = (unpx(now.x) - unpx(at.x)).hypot(unpx(now.y) - unpx(at.y));
                if strayed <= PRESS_SLOP {
                    this.open_bubble_menu(id, None, at, cx);
                }
            });
        })
        .detach();
    }

    /// Opens the right-click menu of bubble `id`, or of its file `file`.
    fn open_bubble_menu(
        &mut self,
        id: MessageId,
        file: Option<usize>,
        at: gpui::Point<gpui::Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.open_chat_context_menu(id, file, at, cx);
    }

    /// The right-click menu of a bubble: reply to all, reply to its sender
    /// only, forward, pin it (or its `file`), copy its text, translate it,
    /// show the conversation as mail.
    pub(in crate::window) fn bubble_menu_rows(
        &self,
        id: MessageId,
        file: Option<usize>,
        rh: f32,
        th: &Theme,
        cx: &mut Context<Self>,
    ) -> Rows {
        let mut rows = Rows::new(rh);
        let Some(reader) = &self.reader else {
            return rows;
        };
        let part = reader.parts.iter().find(|p| p.id == id);
        let from = part.and_then(|p| p.body.as_ref()?.view.as_ref()?.from.first().cloned());
        let mine = from.as_ref().is_none_or(|a| self.is_me(&a.email));
        let text = part.and_then(|p| reader.said(p)).map(|s| s.text.clone());
        let item = |key: &'static str, icon: &str, label: String| {
            self.context_item(key, icon, label, rh, th, cx)
        };
        rows.item(
            item("chat-menu-reply-all", "reply-all", tr!("chat-reply-all")).on_click(cx.listener(
                move |this, _, window, cx| {
                    this.close_context_menu(cx);
                    this.chat_reply(Some(id), Kind::ReplyAll, window, cx);
                },
            )),
        );
        if let Some(from) = from.filter(|_| !mine) {
            rows.item(
                item(
                    "chat-menu-reply",
                    "reply",
                    tr!(
                        "chat-reply-only",
                        name = first_name(from.label()).to_owned()
                    ),
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.close_context_menu(cx);
                    this.chat_reply(Some(id), Kind::Reply, window, cx);
                })),
            );
        }
        rows.item(
            item("chat-menu-forward", "forward", tr!("chat-forward")).on_click(cx.listener(
                move |this, _, window, cx| {
                    this.close_context_menu(cx);
                    this.open_compose(Kind::Forward, Some(id), window, cx);
                    this.pop_out_compose(window, cx);
                },
            )),
        );
        rows.rule(th);
        let what = file.map_or(Pinned::Mail, Pinned::File);
        let pinned = reader.chat.pins.find(id, &what).is_some();
        let pin_label = match (file.is_some(), pinned) {
            (false, false) => tr!("chat-pin"),
            (false, true) => tr!("chat-unpin"),
            (true, false) => tr!("chat-pin-file"),
            (true, true) => tr!("chat-unpin-file"),
        };
        rows.item(
            item(
                "chat-menu-pin",
                if pinned { "pin-filled" } else { "pin" },
                pin_label,
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.close_context_menu(cx);
                this.toggle_chat_pin(id, what.clone(), cx);
            })),
        );
        if let Some(text) = text {
            rows.item(
                item("chat-menu-copy", "copy", tr!("chat-copy-text")).on_click(cx.listener(
                    move |this, _, _, cx| {
                        this.close_context_menu(cx);
                        cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                    },
                )),
            );
        }
        if let Some(label) = self.translate_menu_label(id) {
            rows.item(
                item("chat-menu-translate", "translate", label).on_click(cx.listener(
                    move |this, _, _, cx| {
                        this.close_context_menu(cx);
                        this.translate_from_menu(id, cx);
                    },
                )),
            );
        }
        rows.item(
            item("chat-menu-mail", "mail", tr!("chat-show-as-mail")).on_click(cx.listener(
                move |this, _, window, cx| {
                    this.close_context_menu(cx);
                    this.pick_chat(false, window, cx);
                    // The bubble's mail opens in the mail view.
                    if let (Some(reader), Ok(mail)) = (&mut this.reader, &this.mail)
                        && let Some(ix) = reader.parts.iter().position(|p| p.id == id)
                    {
                        reader.parts[ix].set_expanded(true, mail);
                    }
                },
            )),
        );
        rows
    }
}

/// A signature without its `-- ` line; `None` when nothing is left.
/// `subject` without its "Re:" and "Fwd:" prefixes and list tags: the
/// shortest end of it that normalizes to `key`.
fn bare_subject(subject: &str, key: &str) -> String {
    subject
        .char_indices()
        .map(|(at, _)| subject[at..].trim())
        .rev()
        .find(|rest| katna_core::subject::normalize_subject(rest).text == key)
        .unwrap_or(subject.trim())
        .to_owned()
}

fn signature_lines(signature: &str) -> Option<String> {
    let lines: Vec<&str> = signature
        .trim()
        .lines()
        .skip_while(|l| l.trim_end() == "--" || l.trim().is_empty())
        .collect();
    let text = lines.join("\n").trim().to_owned();
    (!text.is_empty()).then_some(text)
}

/// A forwarded mail inside a bubble: who it came from and what it says,
/// cut short until the bubble's ··· opens it.
fn forwarded_card(forwarded: &Forwarded<String>, open: bool, th: &Theme) -> AnyElement {
    let from = [forwarded.from.as_deref(), forwarded.subject.as_deref()]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");
    let body: String = if open {
        forwarded.body.trim().to_owned()
    } else {
        let mut lines: Vec<&str> = forwarded.body.trim().lines().take(4).collect();
        if forwarded.body.trim().lines().count() > 4 {
            lines.push("…");
        }
        lines.join("\n")
    };
    div()
        .mt(px(4.0))
        .pl(px(10.0))
        .py(px(4.0))
        .border_l_2()
        .border_color(rgba(th.accent))
        .flex()
        .flex_col()
        .gap(px(2.0))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(4.0))
                .text_size(px(12.0))
                .text_color(rgba(th.text_dim))
                .child(icon("forward", th.text_dim, 14.0))
                .child(tr!("chat-forwarded")),
        )
        .when(!from.is_empty(), |d| {
            d.child(
                div()
                    .text_size(px(13.0))
                    .font_weight(FontWeight::MEDIUM)
                    .child(from),
            )
        })
        .child(div().text_size(px(13.0)).child(body))
        .into_any_element()
}

/// The color of a sender's name over their bubble: their picture's color,
/// lightened on a dark theme so it reads.
fn name_color(email: &str, th: &Theme) -> u32 {
    let color = avatar_color(email);
    if th.dark {
        mix(color, 0xffff_ffff, 0.35)
    } else {
        color
    }
}

/// The day separator's words: Today, Yesterday, the weekday within a
/// week, else the date.
fn day_label(date: jiff::civil::DateTime, today: Option<jiff::civil::Date>) -> String {
    let Some(today) = today else {
        return katna_i18n::format::date(date);
    };
    let days = (today - date.date()).get_days();
    match days {
        0 => tr!("chat-today"),
        1 => tr!("chat-yesterday"),
        2..7 => katna_i18n::format::weekday(date),
        _ if date.year() == today.year() => katna_i18n::format::day_month(date),
        _ => katna_i18n::format::date(date),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn says_only_what_was_written() {
        let said = Said::of("Sounds good!\n\nOn Mon, Ana wrote:\n> Lunch?\n");
        assert_eq!(said.text, "Sounds good!");
        assert!(said.hides());
    }

    #[test]
    fn a_signature_said_twice_is_a_signature() {
        let mut said = Said::of("Done, invite sent.\n\nArjun Mehta\nDemo Travel Co\n");
        assert_eq!(said.signature, None);
        said.cut_shared_tail(
            ["Can we move the call to 4?\n\nArjun Mehta\nDemo Travel Co"].into_iter(),
        );
        assert_eq!(said.text, "Done, invite sent.");
        assert_eq!(
            said.signature.as_deref(),
            Some("Arjun Mehta\nDemo Travel Co")
        );
        assert!(said.hides());
    }

    #[test]
    fn signatures_lose_their_dashes() {
        assert_eq!(
            signature_lines("-- \nArjun Mehta\nDemo Travel Co").as_deref(),
            Some("Arjun Mehta\nDemo Travel Co")
        );
        assert_eq!(signature_lines("--"), None);
    }

    #[test]
    fn a_new_subject_drops_its_prefixes() {
        let key = |s: &str| katna_core::subject::normalize_subject(s).text;
        let said = "Re: Fwd: [trip] Goa, final plan";
        assert_eq!(bare_subject(said, &key(said)), "Goa, final plan");
        assert_eq!(bare_subject("Goa", &key("Goa")), "Goa");
    }

    #[test]
    fn days_read_as_people_say_them() {
        let today = jiff::civil::date(2026, 10, 1);
        let at = |d: jiff::civil::Date| d.at(9, 30, 0, 0);
        assert_eq!(day_label(at(today), Some(today)), tr!("chat-today"));
        assert_eq!(
            day_label(at(today.yesterday().unwrap()), Some(today)),
            tr!("chat-yesterday")
        );
    }
}
