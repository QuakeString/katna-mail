// SPDX-License-Identifier: GPL-3.0-or-later

//! A conversation summed up by AI (`docs/ARCHITECTURE.md` §16.5): what it
//! is about, what was settled and what waits on the user, each point
//! naming the mail it came from. Asked from the list's right-click menu
//! (a card beside the line, which opens and marks nothing), the reading
//! pane's sparkle (a card under the subject) or the chat header's (a
//! strip under the header that drops the card down). With unread mail
//! after mail already read, it sums up only the new mail: a catch-up.
//! The daemon keeps each summary, so it shows again at once; new mail
//! after it is added only when asked. None of it shows while writing
//! help is off.

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::{
    AnimationExt, AnyElement, ClipboardItem, Context, FontWeight, MouseButton, Pixels, Point,
    SharedString, Task, Window, anchored, canvas, deferred, div, point, prelude::*, rgba,
};
use katna_ai::summary::{self, PointKind, SummarizeRequest, Summary};
use katna_ai::wire::{plan, problem};
use katna_core::config::AiSource;
use katna_i18n::tr;
use katna_render::trim;
use katna_store::{MessageId, SummaryKind};
use katna_ui::tokens::space;
use katna_ui::{px, unpx};

use super::super::MailWindow;
use super::super::compose::rephrase::{Fix, idea_placeholder, placeholder, problem_text};
use super::first_name;
use crate::daemon::{self, Rephrased};
use crate::data::EntryKey;
use crate::theme::{Theme, fade, mix};
use crate::widgets::{
    Fold, filled_button, fold_arrow, fold_box, icon, icon_button_colored, icon_button_with,
    outlined_button, raised, tip,
};

mod peek_reply;

/// The card beside a line of the list.
const PEEK_WIDTH: f32 = 420.0;
/// The room kept between a card and the window's edge.
const MARGIN: f32 = 8.0;
/// How long the chat's card takes to drop down and to fold back up.
const DROP_IN: Duration = Duration::from_millis(180);
const DROP_OUT: Duration = Duration::from_millis(140);

/// How soon after a press outside folded the dropped card a click on
/// its strip counts as that same press.
const JUST_FOLDED: Duration = Duration::from_millis(400);
/// How tall the card is taken to be before it is first drawn.
const PEEK_GUESS: f32 = 360.0;
/// A point's label column.
const LABEL_WIDTH: f32 = 72.0;
/// The problem of encrypted mail while Settings keeps writing help out
/// of it.
const PROBLEM_ENCRYPTED: &str = "encrypted-off";

/// The summaries of conversations this window has shown.
#[derive(Default)]
pub(in crate::window) struct Summaries {
    by_key: HashMap<EntryKey, Sum>,
    /// Conversations whose kept summary was looked for.
    looked: HashSet<EntryKey>,
    /// The card beside a line of the list.
    peek: Option<Peek>,
    /// Replies written in that card and not sent, by conversation: the
    /// card and the conversation's reply box take them up again.
    kept_replies: HashMap<EntryKey, String>,
    /// When a press outside last folded the chat's dropped card, so the
    /// same press on its strip does not drop it again.
    drop_folded: Option<Instant>,
}

/// The card beside a line of the list.
struct Peek {
    key: EntryKey,
    /// Where it opens, in window coordinates: its top left corner at the
    /// right-click, or under the line.
    at: Point<Pixels>,
    /// How tall the line above `at` is, to open above it when there is no
    /// room below; 0 at a right-click.
    line: f32,
    /// How tall the card was last drawn, to know whether it fits below.
    height: Rc<Cell<f32>>,
    subject: String,
    /// How many mails the conversation has, shown until the summary
    /// says how many it read.
    mails: usize,
    people: usize,
    files: Vec<String>,
    /// Writing a reply in the card.
    reply: Option<peek_reply::PeekReply>,
}

/// A conversation's summary, or the asking for it.
struct Sum {
    state: State,
    /// Shown in the open conversation; × hides it until asked again.
    shown: bool,
    /// Folded to one line under the subject.
    folded: bool,
    /// Dropped down from the chat's strip.
    dropped: bool,
    /// The glide between the card and its folded line.
    fold: Fold,
    /// The glide as it shows or hides (the sparkle, ×).
    show: Fold,
    /// The chat's card dropping from its strip, and the strip's arrow.
    drop: Fold,
    _task: Option<Task<()>>,
}

enum State {
    /// Encrypted mail: its text would leave encrypted, so it asks first.
    Ask,
    Loading,
    Done(Box<Done>),
    /// A [`problem`] name.
    Failed(String),
}

struct Done {
    summary: Summary,
    /// The mails it was made from, in the order sent: what each point's
    /// mail number means.
    mails: Vec<Sent>,
    catch_up: bool,
    /// `katna` or `own`.
    service: String,
    plan: String,
    days_left: u32,
    /// Unix seconds.
    made: i64,
}

/// A mail as it was sent to be summed up.
#[derive(Clone)]
struct Sent {
    id: MessageId,
    name: String,
    email: String,
    date: Option<i64>,
}

/// A conversation's mails, gathered to be summed up.
pub(in crate::window) struct Gathered {
    pub request: SummarizeRequest,
    sent: Vec<Sent>,
    /// Some of its mails are encrypted.
    pub encrypted: bool,
}

impl MailWindow {
    /// Whether summaries are offered: writing help is on.
    pub(in crate::window) fn summaries_on(&self) -> bool {
        self.config.ai.source != AiSource::Off
    }

    /// Whether the open conversation's summary shows.
    pub(in crate::window) fn summary_shown(&self) -> bool {
        self.summaries_on()
            && self
                .reader
                .as_ref()
                .and_then(|r| self.summaries.by_key.get(&r.key))
                .is_some_and(|s| s.shown)
    }

    /// Reads the open conversation's kept summary, once.
    pub(super) fn prepare_summary(&mut self) {
        if let Some(key) = self.reader.as_ref().map(|r| r.key) {
            self.look_for_summary(key);
        }
    }

    /// Reads conversation `key`'s kept summary, once: the newest of a
    /// catch-up and a whole one.
    fn look_for_summary(&mut self, key: EntryKey) {
        if !self.summaries_on() || !self.summaries.looked.insert(key) {
            return;
        }
        if self.summaries.by_key.contains_key(&key) {
            return;
        }
        let Ok(mail) = &mut self.mail else {
            return;
        };
        let ids = mail.entry_messages(key);
        let Some(kept) = mail.summaries(&ids).into_iter().next() else {
            return;
        };
        let Ok(summary) = serde_json::from_str::<Summary>(&kept.body) else {
            return;
        };
        // The mails it was made from: the newest ones up to the newest it
        // covers, as they were sent.
        let drafts: HashSet<MessageId> = mail.drafts(&ids).into_iter().collect();
        let ids: Vec<MessageId> = ids.into_iter().filter(|id| !drafts.contains(id)).collect();
        let end = ids
            .iter()
            .position(|id| *id == kept.message)
            .map_or(ids.len(), |at| at + 1);
        let start = end.saturating_sub(kept.mails as usize);
        let sent = sent_of(mail, &ids[start..end]);
        self.summaries.by_key.insert(
            key,
            Sum {
                state: State::Done(Box::new(Done {
                    summary,
                    mails: sent,
                    catch_up: kept.kind == SummaryKind::New,
                    plan: if kept.service == "own" {
                        daemon_own()
                    } else {
                        String::new()
                    },
                    service: kept.service,
                    days_left: 0,
                    made: kept.created_at,
                })),
                shown: true,
                folded: false,
                dropped: false,
                fold: Fold::default(),
                show: Fold::closed(),
                drop: Fold::closed(),
                _task: None,
            },
        );
    }

    /// The conversation's mails as they are sent to be summed up: the
    /// newest [`summary::MAX_MAILS`], drafts left out, each trimmed to
    /// what its sender wrote.
    pub(in crate::window) fn gather_summary(
        &mut self,
        key: EntryKey,
        catch_up: bool,
    ) -> Option<Gathered> {
        // Opening a conversation marks it read: what was unread then is
        // what the open conversation remembers.
        let came_unread = self
            .reader
            .as_ref()
            .filter(|r| r.key == key)
            .map(|r| r.came_unread.clone());
        let opened: HashMap<MessageId, String> = self
            .reader
            .as_ref()
            .filter(|r| r.key == key)
            .map(|r| {
                r.parts
                    .iter()
                    .filter_map(|p| {
                        let body = p.body.as_ref()?;
                        body.opened.as_ref()?;
                        Some((p.id, body.view.as_ref()?.body.clone()))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let Ok(mail) = &mut self.mail else {
            return None;
        };
        let ids = mail.entry_messages(key);
        let drafts: HashSet<MessageId> = mail.drafts(&ids).into_iter().collect();
        let ids: Vec<MessageId> = ids.into_iter().filter(|id| !drafts.contains(id)).collect();
        let ids = &ids[ids.len().saturating_sub(summary::MAX_MAILS)..];
        if ids.is_empty() {
            return None;
        }
        let rows = mail.message_rows(ids);
        let tz = jiff::tz::TimeZone::system();
        let mut encrypted = false;
        let mut mails = Vec::new();
        let mut sent = Vec::new();
        let mut subject = String::new();
        for (id, row) in ids.iter().zip(&rows) {
            let Some(row) = row else {
                continue;
            };
            if subject.is_empty() {
                subject = row.subject.clone();
            }
            let text = match mail.raw(*id) {
                Some(raw) if katna_crypto::protection(&raw).is_some() => {
                    encrypted = true;
                    opened.get(id).cloned().unwrap_or_default()
                }
                Some(raw) => katna_render::message_view(&raw).body,
                None => row.snippet.clone(),
            };
            let me = mail.is_me(&row.sender);
            let name = if row.correspondent.is_empty() {
                row.sender.clone()
            } else {
                row.correspondent.clone()
            };
            let unread = match &came_unread {
                Some(unread) => unread.contains(id),
                None => row.unread,
            };
            mails.push(summary::Mail {
                from: if me { "me".to_owned() } else { name.clone() },
                when: row
                    .date
                    .and_then(|d| crate::format::local(d, &tz))
                    .map(crate::format::long_date)
                    .unwrap_or_default(),
                text: trim::plain(&text).said.trim().to_owned(),
                new: unread,
            });
            sent.push(Sent {
                id: *id,
                name: if me { tr!("summary-you") } else { name },
                email: row.sender.clone(),
                date: row.date,
            });
        }
        // A catch-up needs mail read before the new.
        let catch_up =
            catch_up && mails.iter().any(|m| m.new) && mails.first().is_some_and(|m| !m.new);
        if !catch_up {
            for mail in &mut mails {
                mail.new = false;
            }
        }
        Some(Gathered {
            request: SummarizeRequest {
                subject,
                mails,
                catch_up,
            },
            sent,
            encrypted,
        })
    }

    /// Sums up conversation `key`: a catch-up when `catch_up` and it has
    /// new mail after mail already read. Encrypted mail asks first unless
    /// `asked`.
    fn start_summary(
        &mut self,
        key: EntryKey,
        catch_up: bool,
        asked: bool,
        cx: &mut Context<Self>,
    ) {
        if !self.summaries_on() {
            return;
        }
        self.summaries.looked.insert(key);
        let Some(gathered) = self.gather_summary(key, catch_up) else {
            return;
        };
        let sum = self.summaries.by_key.entry(key).or_insert(Sum {
            state: State::Loading,
            shown: true,
            folded: false,
            dropped: false,
            fold: Fold::default(),
            show: Fold::closed(),
            drop: Fold::closed(),
            _task: None,
        });
        sum.shown = true;
        sum.folded = false;
        if gathered.encrypted && !asked {
            sum.state = if self.config.ai.encrypted {
                State::Ask
            } else {
                State::Failed(PROBLEM_ENCRYPTED.to_owned())
            };
            sum._task = None;
            cx.notify();
            return;
        }
        let catch_up = gathered.request.catch_up;
        sum.state = State::Loading;
        let connection = self.daemon.clone();
        let newest = gathered.sent.last().map_or(MessageId(0), |s| s.id);
        let request = gathered.request;
        let sent = gathered.sent;
        sum._task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let connection = match connection {
                        Some(connection) => connection,
                        None => daemon::connect()
                            .await
                            .map_err(|_| problem::FAILED.to_owned())?,
                    };
                    daemon::ai_summarize(&connection, newest, &request).await
                })
                .await;
            this.update(cx, |this, cx| {
                let Some(sum) = this.summaries.by_key.get_mut(&key) else {
                    return;
                };
                sum.state = match result.and_then(|done| {
                    let summary = serde_json::from_str::<Summary>(&done.text)
                        .map_err(|_| problem::FAILED.to_owned())?;
                    Ok((summary, done))
                }) {
                    Ok((
                        summary,
                        Rephrased {
                            plan, days_left, ..
                        },
                    )) => State::Done(Box::new(Done {
                        summary,
                        mails: sent,
                        catch_up,
                        service: if plan == daemon_own() {
                            "own".to_owned()
                        } else {
                            "katna".to_owned()
                        },
                        plan,
                        days_left,
                        made: jiff::Timestamp::now().as_second(),
                    })),
                    Err(problem) => State::Failed(problem),
                };
                sum._task = None;
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    /// Stops asking for conversation `key`'s summary.
    fn stop_summary(&mut self, key: EntryKey, cx: &mut Context<Self>) {
        if let Some(sum) = self.summaries.by_key.get_mut(&key)
            && matches!(sum.state, State::Loading | State::Ask)
        {
            self.summaries.by_key.remove(&key);
            cx.notify();
        }
    }

    /// The sparkle of the reading pane and the chat header: shows the
    /// open conversation's summary, asking for one if it has none, or
    /// hides it while it shows (in the chat, whether dropped or only its
    /// strip: the press on the sparkle has already folded the card).
    pub(in crate::window) fn toggle_summary(&mut self, cx: &mut Context<Self>) {
        let Some(key) = self.reader.as_ref().map(|r| r.key) else {
            return;
        };
        let chat = self.chat_shown();
        match self.summaries.by_key.get_mut(&key) {
            Some(sum) if sum.shown => {
                sum.shown = false;
                sum.dropped = false;
                cx.notify();
            }
            Some(sum) if !matches!(sum.state, State::Failed(_)) => {
                sum.shown = true;
                sum.folded = false;
                sum.dropped = chat;
                cx.notify();
            }
            _ => {
                self.start_summary(key, true, false, cx);
                if let Some(sum) = self.summaries.by_key.get_mut(&key) {
                    sum.dropped = chat;
                }
            }
        }
    }

    /// Shift+S: the card beside the selected line in the list, or the open
    /// conversation's summary in the reading pane.
    pub(in crate::window) fn summarize_key(
        &mut self,
        _: &super::super::Summarize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.summaries_on() {
            return;
        }
        if self.reader_focus.contains_focused(window, cx) {
            self.toggle_summary(cx);
            return;
        }
        let Some(ix) = self.selected.filter(|&ix| ix < self.entries.len()) else {
            return;
        };
        let Some(bounds) = self.list_state.bounds_for_item(ix) else {
            return;
        };
        let key = self.entries[ix].key;
        self.open_summary_peek(key, under_line(bounds), unpx(bounds.size.height), cx);
    }

    /// The list menu's Summarize: the card where the right-click was
    /// (`at`), else under line `ix`.
    pub(in crate::window) fn summarize_line(
        &mut self,
        ix: usize,
        key: EntryKey,
        at: Option<Point<Pixels>>,
        cx: &mut Context<Self>,
    ) {
        let (at, line) = match (at, self.list_state.bounds_for_item(ix)) {
            (Some(at), _) => (at, 0.0),
            (None, Some(bounds)) => (under_line(bounds), unpx(bounds.size.height)),
            (None, None) => (point(px(0.0), px(0.0)), 0.0),
        };
        self.open_summary_peek(key, at, line, cx);
    }

    fn open_summary_peek(
        &mut self,
        key: EntryKey,
        at: Point<Pixels>,
        line: f32,
        cx: &mut Context<Self>,
    ) {
        self.look_for_summary(key);
        let folder = self.listed_folder();
        let entry = self.entries.iter().find(|e| e.key == key).copied();
        let Ok(mail) = &mut self.mail else {
            return;
        };
        let row = entry.and_then(|e| mail.rows(&[e], folder, false).pop().flatten());
        let (subject, mails, people, files) = match &row {
            Some(row) => {
                let people = row
                    .people
                    .iter()
                    .filter(|(_, email)| email.is_some())
                    .count()
                    .max(1);
                let mut files: Vec<String> = Vec::new();
                for file in &row.files {
                    if !files.contains(&file.name) {
                        files.push(file.name.clone());
                    }
                }
                (
                    row.subject.clone(),
                    (row.count as usize).max(1),
                    people,
                    files,
                )
            }
            None => (String::new(), 1, 0, Vec::new()),
        };
        self.summaries.peek = Some(Peek {
            key,
            at,
            line,
            height: Rc::new(Cell::new(PEEK_GUESS)),
            reply: None,
            subject,
            mails,
            people,
            files,
        });
        let fresh = self
            .summaries
            .by_key
            .get(&key)
            .is_none_or(|s| matches!(s.state, State::Failed(_)));
        if fresh {
            self.start_summary(key, true, false, cx);
        }
        cx.notify();
    }

    /// Closes the card beside a line of the list. Returns whether it was
    /// open.
    pub(in crate::window) fn close_summary_peek(&mut self, cx: &mut Context<Self>) -> bool {
        // A reply written in it and not sent is kept.
        if let Some((key, text)) = self.peek_reply_text(cx) {
            self.summaries.kept_replies.insert(key, text);
        }
        let open = self.summaries.peek.take().is_some();
        if open {
            cx.notify();
        }
        open
    }

    /// Folds the chat's dropped summary: what Esc does first.
    pub(in crate::window) fn fold_chat_summary(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(key) = self.reader.as_ref().map(|r| r.key) else {
            return false;
        };
        match self.summaries.by_key.get_mut(&key) {
            Some(sum) if sum.dropped => {
                sum.dropped = false;
                cx.notify();
                true
            }
            _ => false,
        }
    }

    /// Goes to message `id` of the open conversation: its bubble in the
    /// chat, or the message opened in the usual view.
    fn go_to_summed(&mut self, id: MessageId, cx: &mut Context<Self>) {
        if self.chat_shown() {
            if let Some(sum) = self
                .reader
                .as_ref()
                .and_then(|r| self.summaries.by_key.get_mut(&r.key))
            {
                sum.dropped = false;
            }
            self.jump_to_bubble(id, cx);
            return;
        }
        let Some(reader) = &mut self.reader else {
            return;
        };
        if let (Some(part), Ok(mail)) = (reader.parts.iter_mut().find(|p| p.id == id), &self.mail) {
            part.set_expanded(true, mail);
        }
        reader.show_all = true;
        reader.jump = Some((id, 0));
        cx.notify();
    }

    /// Scrolls to the message a summary's point asked for, once it is
    /// laid out.
    pub(super) fn settle_summary_jump(&mut self, cx: &mut Context<Self>) {
        let Some(reader) = &mut self.reader else {
            return;
        };
        let Some((id, tries)) = reader.jump else {
            return;
        };
        let top = reader.tops.borrow().get(&id).copied();
        match top {
            Some(top) if tries > 0 => {
                reader.jump = None;
                let view = unpx(self.reader_scroll.bounds().origin.y);
                let max = unpx(self.reader_scroll.max_offset().y);
                // Below the subject pinned over the top.
                let y = (top - view - self.reader_head.get() - 8.0).clamp(0.0, max.max(0.0));
                self.reader_scroll.set_offset(point(px(0.0), px(-y)));
            }
            _ if tries < 4 => reader.jump = Some((id, tries + 1)),
            _ => reader.jump = None,
        }
        cx.notify();
    }

    /// Copies a summary as plain text.
    fn copy_summary(&mut self, key: EntryKey, cx: &mut Context<Self>) {
        let Some(State::Done(done)) = self.summaries.by_key.get(&key).map(|s| &s.state) else {
            return;
        };
        let mut text = done.summary.gist.clone();
        for point in &done.summary.points {
            text.push_str(&format!("\n• {}: {}", kind_label(point.kind), point.text));
        }
        if let Some(asked) = &done.summary.for_you {
            text.push_str(&format!("\n{}: {}", tr!("summary-for-you"), asked.text));
        }
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        self.show_snackbar(tr!("summary-copied"), None, cx);
    }

    /// The open conversation's messages after the newest its summary
    /// covers, as the reading pane has them.
    fn summary_newer(&self, done: &Done) -> usize {
        let Some(reader) = &self.reader else {
            return 0;
        };
        let Some(last) = done.mails.last() else {
            return 0;
        };
        let stored: Vec<&super::Part> = reader
            .parts
            .iter()
            .filter(|p| p.pending.is_none() && !reader.drafts.contains(&p.id))
            .collect();
        match stored.iter().position(|p| p.id == last.id) {
            Some(at) => stored.len() - at - 1,
            None => 0,
        }
    }

    /// The reading pane's sparkle, pressed while the summary shows.
    pub(super) fn summary_button(
        &self,
        id: &'static str,
        th: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.summaries_on() {
            return None;
        }
        let on = self.summary_shown();
        Some(
            icon_button_colored(
                id,
                "sparkle",
                20.0,
                if on { th.accent } else { th.text_dim },
                th,
            )
            .when(on, |d| d.bg(rgba(fade(th.accent, 0.14))))
            .tooltip(tip(
                if on {
                    tr!("summary-hide")
                } else {
                    tr!("summary-summarize")
                },
                th,
            ))
            .on_click(cx.listener(|this, _, _, cx| {
                cx.stop_propagation();
                this.toggle_summary(cx);
            }))
            .into_any_element(),
        )
    }

    /// The chat header's sparkle: a pill of its own beside the Chat | Mail
    /// switch, as tall as it, whose inside lifts like the switch's picked
    /// side while the summary shows.
    pub(super) fn summary_pill(&self, th: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.summaries_on() {
            return None;
        }
        let on = self.summary_shown();
        Some(
            div()
                .flex_none()
                .p(px(space::S1))
                .rounded_full()
                .bg(rgba(th.chip))
                .child(
                    div()
                        .id("chat-summary")
                        .h(px(28.0))
                        .w(px(36.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .cursor_pointer()
                        .when(on, |d| {
                            d.bg(rgba(th.surface))
                                .shadow(crate::widgets::elevation(th, 0.5))
                        })
                        .when(!on, |d| d.hover(|s| s.bg(rgba(th.hover))))
                        .child(icon(
                            "sparkle",
                            if on { th.accent } else { th.text_dim },
                            16.0,
                        ))
                        .tooltip(tip(
                            if on {
                                tr!("summary-hide")
                            } else {
                                tr!("summary-summarize")
                            },
                            th,
                        ))
                        .on_click(cx.listener(|this, _, _, cx| {
                            cx.stop_propagation();
                            this.toggle_summary(cx);
                        })),
                )
                .into_any_element(),
        )
    }

    /// The summary under the subject of the open conversation, in the
    /// usual view.
    pub(super) fn render_summary_card(
        &self,
        th: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let reader = self.reader.as_ref()?;
        let key = reader.key;
        let sum = self.summaries.by_key.get(&key)?;
        if !self.summaries_on() {
            return None;
        }
        // It glides open from the sparkle and closed with ×, as folds do.
        sum.show.sync(sum.shown);
        if !sum.shown && !sum.show.moving() {
            return None;
        }
        let card = if sum.shown {
            let inner = if sum.folded {
                self.summary_strip(key, true, th, cx)
            } else {
                self.summary_content(key, Place::Card, th, cx)?
            };
            div()
                .pl(px(self.reader_indent()))
                .pr(px(16.0))
                .pb(px(12.0))
                .child(fold_box("summary-fold", &sum.fold, inner))
        } else {
            div()
        };
        Some(fold_box("summary-show", &sum.show, card))
    }

    /// The chat's strip under its header, and the card it drops down.
    pub(super) fn render_chat_summary_strip(
        &self,
        th: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let key = self.reader.as_ref()?.key;
        let sum = self.summaries.by_key.get(&key)?;
        if !self.summaries_on() {
            return None;
        }
        sum.show.sync(sum.shown);
        if !sum.shown && !sum.show.moving() {
            return None;
        }
        let strip = if sum.shown {
            self.summary_strip(key, false, th, cx)
        } else {
            div().into_any_element()
        };
        Some(fold_box("chat-summary-show", &sum.show, strip))
    }

    /// The card the chat's strip drops over the chat.
    pub(super) fn render_chat_summary_drop(
        &self,
        th: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let key = self.reader.as_ref()?.key;
        let sum = self.summaries.by_key.get(&key)?;
        if !self.summaries_on() {
            return None;
        }
        let open = sum.shown && sum.dropped;
        sum.drop.sync(open);
        // Folding up, it fades back the way it came and lets clicks
        // through to the chat at once.
        if !open && !sum.drop.moving() {
            return None;
        }
        let content = self.summary_content(key, Place::Drop, th, cx)?;
        let card = div()
            .absolute()
            .top(px(8.0))
            .left(px(12.0))
            .right(px(12.0))
            .child(content);
        let card = if open {
            card.occlude()
                .on_mouse_down_out(cx.listener(move |this, _, _, cx| {
                    if let Some(sum) = this.summaries.by_key.get_mut(&key)
                        && sum.dropped
                    {
                        sum.dropped = false;
                        this.summaries.drop_folded = Some(Instant::now());
                        cx.notify();
                    }
                }))
        } else {
            card
        };
        let (time, from, to) = if open {
            (DROP_IN, 0.0, 1.0)
        } else {
            (DROP_OUT, 1.0, 0.0)
        };
        Some(
            card.with_animation(
                sum.drop.id("chat-summary-drop"),
                gpui::Animation::new(katna_ui::motion::time(time))
                    .with_easing(gpui::ease_out_quint()),
                move |el, t| {
                    let t = from + (to - from) * t;
                    el.opacity(t).mt(px(-6.0 * (1.0 - t)))
                },
            )
            .into_any_element(),
        )
    }

    /// One line: the chat's strip, or the folded card. Clicking it drops
    /// the chat's card down, or unfolds the card.
    fn summary_strip(
        &self,
        key: EntryKey,
        card: bool,
        th: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(sum) = self.summaries.by_key.get(&key) else {
            return div().into_any_element();
        };
        let open = !card && sum.dropped;
        let (title, rest): (String, String) = match &sum.state {
            State::Done(done) => {
                let newer = self.summary_newer(done);
                let title = if done.catch_up {
                    tr!("summary-catch-up", count = catch_up_count(done) as u64)
                } else {
                    tr!("summary-title")
                };
                let rest = if newer > 0 {
                    tr!(
                        "summary-strip-newer",
                        count = newer as u64,
                        gist = done.summary.gist.clone()
                    )
                } else {
                    done.summary.gist.clone()
                };
                (title, rest)
            }
            State::Loading => (
                tr!("summary-title"),
                tr!("summary-asking", service = self.ai_service_name()),
            ),
            State::Ask => (tr!("summary-title"), tr!("summary-ask-short")),
            State::Failed(problem) => (tr!("summary-title"), self.summary_problem(problem).0),
        };
        // The line is a soft rounded hover inside the strip or card, as
        // everywhere else.
        let row = div()
            .id(if card {
                "summary-folded"
            } else {
                "chat-summary-strip"
            })
            .flex()
            .flex_row()
            .items_center()
            .gap(px(10.0))
            .cursor_pointer()
            .text_size(px(13.5))
            .text_color(rgba(th.text))
            .hover(|s| s.bg(rgba(th.hover)))
            .map(|d| {
                if card {
                    d.rounded(px(11.0)).px(px(16.0)).py(px(8.0))
                } else {
                    d.rounded(px(8.0)).px(px(10.0)).py(px(4.0))
                }
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                if let Some(sum) = this.summaries.by_key.get_mut(&key) {
                    if card {
                        sum.folded = false;
                        sum.fold.turn();
                    } else if !this
                        .summaries
                        .drop_folded
                        .take()
                        .is_some_and(|at| at.elapsed() < JUST_FOLDED)
                    {
                        sum.dropped = !sum.dropped;
                    }
                    cx.notify();
                }
            }))
            .child(icon("sparkle", th.accent, 16.0))
            .child(
                div().flex_1().min_w_0().truncate().child(
                    div()
                        .flex()
                        .flex_row()
                        .gap(px(6.0))
                        .child(
                            div()
                                .flex_none()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(title),
                        )
                        .when(!open, |d| {
                            d.child(div().flex_none().text_color(rgba(th.text_faint)).child("·"))
                                .child(
                                    div()
                                        .min_w_0()
                                        .truncate()
                                        .text_color(rgba(th.text_dim))
                                        .child(rest),
                                )
                        }),
                ),
            )
            .child(if card {
                fold_arrow("summary-arrow", &sum.fold, false, th.text_dim, 16.0)
            } else {
                fold_arrow("summary-drop-arrow", &sum.drop, open, th.text_dim, 16.0)
            });
        div()
            .flex_none()
            .map(|d| {
                if card {
                    crate::widgets::tile(d, th)
                } else {
                    d.border_b_1()
                        .border_color(rgba(th.divider))
                        .px(px(6.0))
                        .py(px(4.0))
                }
            })
            .bg(rgba(summary_surface(th)))
            .child(row)
            .into_any_element()
    }

    /// What to say about `problem`, and what its button does.
    fn summary_problem(&self, problem: &str) -> (String, Fix) {
        if problem == PROBLEM_ENCRYPTED {
            return (
                tr!("summary-encrypted-off"),
                Fix::Settings(super::super::settings_page::Section::Ai),
            );
        }
        problem_text(problem, &self.ai_service_name())
    }

    /// The summary's card: under the subject, dropped over the chat or
    /// beside a line of the list.
    fn summary_content(
        &self,
        key: EntryKey,
        place: Place,
        th: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let sum = self.summaries.by_key.get(&key)?;
        let peek = self
            .summaries
            .peek
            .as_ref()
            .filter(|_| place == Place::Peek);
        let id = |name: &str| SharedString::from(format!("summary-{}-{name}", place.id()));
        let small_button = |name: &str, icon_name: &'static str, label: String| {
            icon_button_colored(id(name), icon_name, 17.0, th.text_dim, th)
                .size(px(30.0))
                .tooltip(tip(label, th))
        };

        // The header: what it sums up, and its buttons.
        let mut title = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.0))
            .text_size(px(13.0))
            .text_color(rgba(th.text_dim))
            .child(icon("sparkle", th.accent, 16.0));
        let done = match &sum.state {
            State::Done(done) => Some(done.as_ref()),
            _ => None,
        };
        let newer = done.map_or(0, |d| self.summary_newer(d));
        match peek {
            Some(peek) => {
                title = title
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(14.5))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgba(th.text))
                            .child(peek.subject.clone()),
                    )
                    .child(div().flex_none().child(tr!(
                        "summary-peek-count",
                        mails = done.map_or(peek.mails, |d| d.mails.len()) as u64,
                        people = done.map_or(peek.people, |d| {
                            let mut seen: Vec<String> = Vec::new();
                            for sent in &d.mails {
                                let who = sent.email.to_lowercase();
                                if !seen.contains(&who) {
                                    seen.push(who);
                                }
                            }
                            seen.len().max(peek.people)
                        }) as u64
                    )));
            }
            None => {
                let heading = match done {
                    Some(done) if done.catch_up => {
                        tr!("summary-catch-up", count = catch_up_count(done) as u64)
                    }
                    _ => tr!("summary-title"),
                };
                title = title
                    .child(
                        div()
                            .flex_none()
                            .text_size(px(14.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgba(th.text))
                            .child(heading),
                    )
                    .children(done.filter(|d| !d.catch_up).map(|done| {
                        let count = done.mails.len() as u64;
                        div().flex_none().child(if newer > 0 {
                            tr!(
                                "summary-of-mails",
                                count = count,
                                total = count + newer as u64
                            )
                        } else {
                            tr!("summary-mails", count = count)
                        })
                    }))
                    .child(div().flex_1());
            }
        }
        if newer > 0 && peek.is_none() {
            title = title.child(
                div()
                    .id(id("add-new"))
                    .flex_none()
                    .h(px(28.0))
                    .px(px(12.0))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.0))
                    .rounded_full()
                    .bg(rgba(fade(th.accent, 0.16)))
                    .text_color(rgba(th.text))
                    .text_size(px(13.0))
                    .font_weight(FontWeight::MEDIUM)
                    .cursor_pointer()
                    .hover(|s| s.bg(rgba(fade(th.accent, 0.24))))
                    .child(icon("refresh", th.text, 15.0))
                    .child(tr!("summary-add-new", count = newer as u64))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.start_summary(key, false, true, cx);
                    })),
            );
        }
        match place {
            Place::Card => {
                title = title
                    .child(
                        icon_button_with(
                            id("fold"),
                            fold_arrow("summary-arrow", &sum.fold, true, th.text_dim, 17.0),
                            th,
                        )
                        .size(px(30.0))
                        .tooltip(tip(tr!("summary-fold"), th))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(sum) = this.summaries.by_key.get_mut(&key) {
                                sum.folded = true;
                                sum.fold.turn();
                                cx.notify();
                            }
                        })),
                    )
                    .child(
                        small_button("close", "close", tr!("summary-hide")).on_click(cx.listener(
                            move |this, _, _, cx| {
                                this.stop_summary(key, cx);
                                if let Some(sum) = this.summaries.by_key.get_mut(&key) {
                                    sum.shown = false;
                                    cx.notify();
                                }
                            },
                        )),
                    );
            }
            Place::Peek => {
                title = title.child(
                    small_button("close", "close", tr!("summary-close")).on_click(cx.listener(
                        |this, _, _, cx| {
                            this.close_summary_peek(cx);
                        },
                    )),
                );
            }
            Place::Drop => {}
        }

        // Writing a reply: the card itself turns into it.
        let writing = peek.and_then(|p| p.reply.as_ref()).map(|reply| {
            let gist = match &sum.state {
                State::Done(done) => Some(done.summary.gist.clone()),
                _ => None,
            };
            self.render_peek_reply(reply, gist, th, cx)
        });
        let body: AnyElement = match &sum.state {
            State::Ask => div()
                .flex()
                .flex_col()
                .gap(px(10.0))
                .child(
                    div()
                        .text_size(px(13.5))
                        .line_height(px(20.0))
                        .text_color(rgba(th.text))
                        .child(tr!("summary-encrypted", service = self.ai_service_name())),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .justify_end()
                        .gap(px(8.0))
                        .child(
                            outlined_button(id("cancel"), tr!("summary-cancel"), th)
                                .h(px(32.0))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.stop_summary(key, cx);
                                    this.close_summary_peek(cx);
                                })),
                        )
                        .child(
                            filled_button(id("send"), tr!("summary-send"), th)
                                .h(px(32.0))
                                .px(px(18.0))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.start_summary(key, true, true, cx);
                                })),
                        ),
                )
                .into_any_element(),
            State::Loading => div()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(div().py(px(4.0)).child(placeholder(th, cx.reduce_motion())))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .pt(px(6.0))
                        .border_t_1()
                        .border_color(rgba(th.divider))
                        .text_size(px(12.0))
                        .text_color(rgba(th.text_faint))
                        .child(
                            div()
                                .flex_1()
                                .child(tr!("summary-asking", service = self.ai_service_name())),
                        )
                        .child(
                            div()
                                .id(id("stop"))
                                .px(px(10.0))
                                .py(px(4.0))
                                .rounded_full()
                                .text_size(px(13.0))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(rgba(th.accent))
                                .cursor_pointer()
                                .relative()
                                .child(crate::widgets::hover_fade("hover-glow", None, th))
                                .child(tr!("summary-stop"))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.stop_summary(key, cx);
                                    this.close_summary_peek(cx);
                                })),
                        ),
                )
                .into_any_element(),
            State::Failed(problem) => {
                let (text, fix) = self.summary_problem(problem);
                let label = match fix {
                    Fix::Retry => tr!("summary-try-again"),
                    Fix::Settings(_) => tr!("summary-open-settings"),
                };
                div()
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap(px(8.0))
                    .child(
                        div()
                            .text_size(px(13.5))
                            .text_color(rgba(th.text_dim))
                            .child(text),
                    )
                    .child(
                        outlined_button(id("fix"), label, th)
                            .h(px(30.0))
                            .px(px(14.0))
                            .on_click(cx.listener(move |this, _, window, cx| match fix {
                                Fix::Retry => this.start_summary(key, true, false, cx),
                                Fix::Settings(section) => {
                                    this.close_summary_peek(cx);
                                    this.open_settings_page(section, window, cx);
                                }
                            })),
                    )
                    .into_any_element()
            }
            State::Done(done) => self.summary_done(key, done, place, th, cx),
        };

        Some(
            div()
                .id(id("card"))
                .flex()
                .flex_col()
                .gap(px(10.0))
                .px(px(16.0))
                .pt(px(12.0))
                .pb(px(10.0))
                .map(|d| match place {
                    Place::Card => crate::widgets::tile(d, th).bg(rgba(summary_surface(th))),
                    Place::Drop | Place::Peek => raised(d, th, 16.0, 3.0),
                })
                .text_color(rgba(th.text))
                .map(|d| match writing {
                    Some(reply) => d.child(reply),
                    None => d.child(title).child(body),
                })
                .into_any_element(),
        )
    }

    /// A summary made: its gist, points, what waits on the user and,
    /// beside a line of the list, its files; then who made it and its
    /// buttons.
    fn summary_done(
        &self,
        key: EntryKey,
        done: &Done,
        place: Place,
        th: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = |name: String| SharedString::from(format!("summary-{}-{name}", place.id()));
        let peek = place == Place::Peek;
        let source = |n: usize, mail: Option<usize>| -> Option<AnyElement> {
            let sent = done.mails.get(mail?)?.clone();
            let message = sent.id;
            Some(
                div()
                    .id(id(format!("source-{n}")))
                    .flex_none()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(4.0))
                    .ml(px(6.0))
                    .px(px(4.0))
                    .rounded(px(8.0))
                    .text_size(px(12.0))
                    .text_color(rgba(th.accent))
                    .when(!peek, |d| {
                        d.cursor_pointer()
                            .hover(|s| s.bg(rgba(th.hover)))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.go_to_summed(message, cx);
                            }))
                    })
                    .tooltip(tip(
                        match sent
                            .date
                            .and_then(|d| crate::format::local(d, &jiff::tz::TimeZone::system()))
                        {
                            Some(date) => tr!(
                                "summary-from-mail",
                                name = sent.name.clone(),
                                date = crate::format::long_date(date)
                            ),
                            None => sent.name.clone(),
                        },
                        th,
                    ))
                    .child(self.person_avatar(&sent.name, &sent.email, 16.0))
                    .child(first_name(&sent.name).to_owned())
                    .into_any_element(),
            )
        };
        let label = |text: String| {
            div()
                .flex_none()
                .w(px(LABEL_WIDTH))
                .pt(px(2.0))
                .text_size(px(11.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgba(th.text_faint))
                .child(text.to_uppercase())
        };
        let points = done.summary.points.iter().enumerate().map(|(n, p)| {
            div()
                .flex()
                .flex_row()
                .items_start()
                .gap(px(10.0))
                .text_size(px(13.5))
                .line_height(px(20.0))
                .child(label(kind_label(p.kind)))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .items_start()
                        .child(div().w_full().child(p.text.clone()))
                        .children(source(n, p.mail)),
                )
        });
        let files = self
            .summaries
            .peek
            .as_ref()
            .filter(|_| peek)
            .map(|p| p.files.clone())
            .unwrap_or_default();
        let files = (!files.is_empty()).then(|| {
            div()
                .flex()
                .flex_row()
                .items_start()
                .gap(px(10.0))
                .child(label(tr!("summary-files")))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_row()
                        .flex_wrap()
                        .gap(px(6.0))
                        .children(files.into_iter().take(4).map(|name| {
                            div()
                                .max_w(px(260.0))
                                .truncate()
                                .px(px(10.0))
                                .py(px(2.0))
                                .rounded_full()
                                .border_1()
                                .border_color(rgba(th.divider))
                                .bg(rgba(th.surface))
                                .text_size(px(12.5))
                                .child(name)
                        })),
                )
        });
        let for_you = done.summary.for_you.as_ref().map(|asked| {
            div()
                .flex()
                .flex_row()
                .items_start()
                .gap(px(8.0))
                .px(px(10.0))
                .py(px(6.0))
                .rounded(px(8.0))
                .bg(rgba(fade(th.important, if th.dark { 0.2 } else { 0.16 })))
                .text_size(px(13.5))
                .line_height(px(20.0))
                .child(
                    div()
                        .flex_none()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(tr!("summary-for-you")),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .items_start()
                        .child(div().w_full().child(asked.text.clone()))
                        .children(source(99, asked.mail)),
                )
        });
        let tz = jiff::tz::TimeZone::system();
        let made = crate::format::local(done.made, &tz).map(katna_i18n::format::time);
        let who = if done.service == "own" {
            self.ai_service_name()
        } else {
            tr!("compose-ai-katna")
        };
        let footer_text = if peek {
            tr!("summary-not-read", service = who)
        } else if done.plan == plan::TRIAL && done.days_left > 0 {
            tr!(
                "compose-ai-trial-left",
                service = who,
                days = done.days_left
            )
        } else {
            match made {
                Some(time) => tr!("summary-made", service = who, time = time),
                None => who,
            }
        };
        let open_key = key;
        let footer = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(4.0))
            .pt(px(6.0))
            .border_t_1()
            .border_color(rgba(th.divider))
            .text_size(px(12.0))
            .text_color(rgba(th.text_faint))
            .child(div().flex_1().min_w_0().child(footer_text))
            .child(
                icon_button_colored(id("copy".into()), "copy", 17.0, th.text_dim, th)
                    .size(px(30.0))
                    .tooltip(tip(tr!("summary-copy"), th))
                    .on_click(cx.listener(move |this, _, _, cx| this.copy_summary(key, cx))),
            )
            .when(!peek, |d| {
                d.child(
                    icon_button_colored(id("again".into()), "refresh", 17.0, th.text_dim, th)
                        .size(px(30.0))
                        .tooltip(tip(tr!("summary-again"), th))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let catch_up = matches!(
                                this.summaries.by_key.get(&key).map(|s| &s.state),
                                Some(State::Done(d)) if d.catch_up
                            );
                            this.start_summary(key, catch_up, true, cx);
                        })),
                )
            })
            .when(peek, |d| {
                // Open and Reply: two compact pills of one size.
                let pill = |name: &str, icon_name: &'static str, label: String, tonal: bool| {
                    div()
                        .id(id(name.into()))
                        .flex_none()
                        .h(px(30.0))
                        .px(px(12.0))
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(5.0))
                        .rounded_full()
                        .when(tonal, |d| {
                            d.bg(rgba(fade(th.accent, 0.16)))
                                .hover(|s| s.bg(rgba(fade(th.accent, 0.24))))
                        })
                        .when(!tonal, |d| {
                            d.border_1()
                                .border_color(rgba(th.outline))
                                .hover(|s| s.bg(rgba(th.hover)))
                        })
                        .text_color(rgba(th.text))
                        .text_size(px(13.0))
                        .font_weight(FontWeight::MEDIUM)
                        .cursor_pointer()
                        .child(icon(icon_name, th.text, 15.0))
                        .child(label)
                };
                d.child(
                    pill("open", "open-external", tr!("summary-open"), false)
                        .ml(px(4.0))
                        .tooltip(tip(tr!("summary-open-tip"), th))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.close_summary_peek(cx);
                            if let Some(ix) = this.entries.iter().position(|e| e.key == open_key) {
                                this.open(ix, window, cx);
                            }
                        })),
                )
                .child(
                    pill("reply", "pen-sparkle", tr!("summary-reply"), true)
                        .tooltip(tip(tr!("summary-reply-tip"), th))
                        .on_click(
                            cx.listener(|this, _, window, cx| this.start_peek_reply(window, cx)),
                        ),
                )
            });
        div()
            .flex()
            .flex_col()
            .gap(px(10.0))
            .child(
                div()
                    .text_size(px(14.5))
                    .line_height(px(22.0))
                    .child(done.summary.gist.clone()),
            )
            .when(!done.summary.points.is_empty() || files.is_some(), |d| {
                d.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(6.0))
                        .children(points)
                        .children(files),
                )
            })
            .children(for_you)
            .child(footer)
            .into_any_element()
    }

    /// The card beside a line of the list, over a scrim that closes it.
    pub(in crate::window) fn render_summary_peek(
        &self,
        th: &Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let peek = self.summaries.peek.as_ref()?;
        if !self.summaries_on() {
            return None;
        }
        let content = self.summary_content(peek.key, Place::Peek, th, cx)?;
        let viewport = window.viewport_size();
        let (vw, vh) = (unpx(viewport.width), unpx(viewport.height));
        let (x, y) = (unpx(peek.at.x), unpx(peek.at.y));
        let height = peek.height.get().min(vh - 2.0 * MARGIN);
        // Opens where the right-click was (or under the line), flipping
        // left or up where there is no room, like the menu it came from.
        let x = if x + PEEK_WIDTH + MARGIN <= vw {
            x
        } else if x - PEEK_WIDTH >= MARGIN {
            x - PEEK_WIDTH
        } else {
            (vw - PEEK_WIDTH - MARGIN).max(MARGIN)
        };
        let above = y - peek.line - height;
        let y = if y + height + MARGIN <= vh {
            y
        } else if above >= MARGIN {
            above
        } else {
            (vh - MARGIN - height).max(MARGIN)
        };
        let measured = peek.height.clone();
        let measure = canvas(
            move |bounds, window, _| {
                let h = unpx(bounds.size.height);
                if (measured.get() - h).abs() > 0.5 {
                    measured.set(h);
                    window.refresh();
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full();
        let close = || {
            cx.listener(|this: &mut Self, _: &gpui::MouseDownEvent, _, cx| {
                this.close_summary_peek(cx);
            })
        };
        Some(
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .child(
                    deferred(
                        div()
                            .id("summary-peek-scrim")
                            .absolute()
                            .top(px(-2000.0))
                            .left(px(-4000.0))
                            .w(px(8000.0))
                            .h(px(6000.0))
                            .occlude()
                            .on_mouse_down(MouseButton::Left, close())
                            .on_mouse_down(MouseButton::Right, close()),
                    )
                    .with_priority(3),
                )
                .child(
                    deferred(
                        anchored()
                            .position(point(px(x), px(y)))
                            .snap_to_window_with_margin(px(MARGIN))
                            .child(
                                div()
                                    .relative()
                                    .occlude()
                                    .w(px(PEEK_WIDTH))
                                    .child(content)
                                    .child(measure)
                                    .with_animation(
                                        "summary-peek",
                                        gpui::Animation::new(katna_ui::motion::time(
                                            Duration::from_millis(160),
                                        ))
                                        .with_easing(gpui::ease_out_quint()),
                                        |el, t| el.opacity(t).ml(px(-6.0 * (1.0 - t))),
                                    ),
                            ),
                    )
                    .with_priority(4),
                )
                .into_any_element(),
        )
    }
}

/// Where the card opens under a line found by keyboard: past the
/// checkbox and star, just below the line.
fn under_line(bounds: gpui::Bounds<Pixels>) -> Point<Pixels> {
    point(bounds.origin.x + px(96.0), bounds.bottom())
}

/// Where a summary's card is.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Place {
    /// Under the subject.
    Card,
    /// Dropped down over the chat.
    Drop,
    /// Beside a line of the list.
    Peek,
}

impl Place {
    fn id(self) -> &'static str {
        match self {
            Place::Card => "card",
            Place::Drop => "drop",
            Place::Peek => "peek",
        }
    }
}

/// How many new mails a catch-up sums up.
fn catch_up_count(done: &Done) -> usize {
    (done.summary.new as usize).max(1)
}

/// The plan name of the user's own service.
fn daemon_own() -> String {
    "own".to_owned()
}

/// A point's label.
fn kind_label(kind: PointKind) -> String {
    match kind {
        PointKind::Settled => tr!("summary-point-settled"),
        PointKind::Money => tr!("summary-point-money"),
        PointKind::Dates => tr!("summary-point-dates"),
        PointKind::Next => tr!("summary-point-next"),
        PointKind::Open => tr!("summary-point-open"),
    }
}

/// The summary's own surface: a soft grey on the card; its lines are
/// the theme's divider.
fn summary_surface(th: &Theme) -> u32 {
    mix(th.surface, th.text, if th.dark { 0.04 } else { 0.025 })
}

/// The mails `ids` as a kept summary's points name them.
fn sent_of(mail: &mut crate::data::Mail, ids: &[MessageId]) -> Vec<Sent> {
    let rows = mail.message_rows(ids);
    ids.iter()
        .zip(rows)
        .map(|(id, row)| match row {
            Some(row) => {
                let me = mail.is_me(&row.sender);
                Sent {
                    id: *id,
                    name: if me {
                        tr!("summary-you")
                    } else if row.correspondent.is_empty() {
                        row.sender.clone()
                    } else {
                        row.correspondent.clone()
                    },
                    email: row.sender.clone(),
                    date: row.date,
                }
            }
            None => Sent {
                id: *id,
                name: String::new(),
                email: String::new(),
                date: None,
            },
        })
        .collect()
}
