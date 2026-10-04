// SPDX-License-Identifier: GPL-3.0-or-later

//! The open conversation, laid out like webmail: an action bar (back,
//! archive, spam, delete, mark unread, move, more, "3 of 120"), the subject
//! with its folder chip, the messages (older ones folded to one line, a
//! "4 older messages" fold in long threads, the newest open) and, pinned
//! at the foot, Reply, Reply all and Forward, or the reply being written.

use std::cell::Cell;
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    Animation, AnimationExt, AnyElement, Context, FontWeight, MouseButton, SharedString, div,
    ease_out_quint, prelude::*, rgba,
};
use katna_core::MailCategory;
use katna_i18n::tr;
use katna_render::MessageView;
use katna_render::html::Document;
use katna_store::{FolderId, MessageFlags, MessageId};
use katna_ui::motion::lerp;
use katna_ui::px;
use katna_ui::tokens::{radius, space};
use katna_ui::unpx;

use super::compose::{Kind, SentCard};
use super::list::separator;
use super::rich::{self, Painter};
use super::{Act, MailWindow, Menu, READER_CONTEXT, SelectNext, SelectPrevious};
use crate::daemon::Command;
use crate::data::{self, EntryKey, Mail, Row};
use crate::format;
use crate::sidebar::Role;
use crate::theme::{Theme, fade};
use crate::widgets::{card_outline, icon, icon_button, icon_button_colored, tip, toolbar};

mod chat;
mod invite;
mod security;
mod summary;
mod ticks;
mod tracking;
use security::Secured;
pub(super) use summary::Summaries;

/// The reading view shows at most this many lines of a body.
const MAX_BODY_LINES: usize = 4000;
/// Fold the middle of a conversation when this many messages in a row are
/// folded.
const FOLD_AT: usize = 3;
/// The column the sender's picture sits in, centered, beside an open
/// message; the text starts at its right edge.
const PICTURE_COLUMN: f32 = 72.0;
/// From the picture's left edge to the text's.
const PICTURE_COLUMN_INSET: f32 = PICTURE_COLUMN - (PICTURE_COLUMN - 40.0) / 2.0;
/// An open message shows its date and star button, and the subject its
/// full size, in panes at least this wide.
const STAR_FROM: f32 = 260.0;
/// How wide a message's text gets with "Limit the width of messages":
/// about 90 characters a line.
const MESSAGE_WIDTH: f32 = 760.0;

/// An open conversation (or a single message).
pub(super) struct Conversation {
    pub key: EntryKey,
    subject: String,
    parts: Vec<Part>,
    /// The folded middle of a long conversation is shown.
    show_all: bool,
    /// In a dark theme, its HTML mail keeps the sender's own colors
    /// rather than dark ones.
    pub original_colors: bool,
    /// Whether it showed as a chat when its toolbar was last drawn, and
    /// how many times that changed since: the original colors button
    /// slides out for the chat and back for the mail.
    colors_chat: Option<bool>,
    colors_flips: usize,
    /// The popover of who opened a sent message or followed its links.
    seen: Option<tracking::Seen>,
    /// Its stored messages that are drafts, read with the messages rather
    /// than on every frame.
    drafts: HashSet<MessageId>,
    /// Its messages' `Message-ID`s, oldest first, for the notes about it.
    headers: Vec<String>,
    /// How it shows as a chat (Settings > Experimental).
    pub(super) chat: chat::ChatState,
    /// Its messages that were unread when it opened, for a summary's
    /// catch-up: opening it marks them read.
    came_unread: HashSet<MessageId>,
    /// Where each message was last drawn, from the top of the scrolled
    /// conversation, and the message a summary's point goes to once it
    /// is drawn (with the frames waited).
    tops: Rc<std::cell::RefCell<std::collections::HashMap<MessageId, f32>>>,
    jump: Option<(MessageId, u8)>,
    /// Every folder or label its messages are in, for the chips under
    /// the subject, and the inbox tab of its newest message.
    folders: Vec<FolderId>,
    category: Option<MailCategory>,
}

/// A chip under the subject: a folder, label, tab or mark.
struct HeaderChip {
    text: String,
    /// What its × does; none where the service can't take it off.
    remove: Option<ChipRemove>,
}

/// What a chip's × does, with an Undo like the toolbar's.
enum ChipRemove {
    Act(Act),
    /// Takes a Gmail label off.
    Label(FolderId, String),
}

/// One message of the conversation.
/// Where a message's attachments come from ([`Conversation::attachment_source`]).
pub(super) enum AttachmentSource {
    /// The stored message.
    Stored,
    /// The message GnuPG opened; `encrypted` when it was decrypted, so
    /// its attachments must not reach the disk unasked.
    Opened { raw: Arc<Vec<u8>>, encrypted: bool },
    /// Encrypted or signed, and not opened.
    Sealed,
}

struct Part {
    id: MessageId,
    row: Option<Rc<Row>>,
    expanded: bool,
    /// Loaded when first expanded.
    body: Option<Body>,
    /// The "to me, Bob ▾" details are open.
    details: bool,
    /// How tall it was when last drawn, to grow or shrink it smoothly.
    height: Rc<Cell<f32>>,
    /// Its height when it was last opened or folded, and how many times
    /// that happened: each time starts a new height animation.
    from: f32,
    turns: u32,
    /// Sent with open and click tracking: what its recipients did.
    activity: Option<katna_store::MessageActivity>,
    /// A read receipt for one of the user's messages; `None` inside
    /// until looked at.
    receipt: Option<Option<crate::receipts::Receipt>>,
    /// The user's own message's `Message-ID`.
    message_id: Option<String>,
    /// For the user's own message: its recipients' delivery and read
    /// ticks, by address (lower case).
    ticks: std::collections::HashMap<String, ticks::Tick>,
    /// Where its eye button was last drawn, and the window's size then,
    /// for the popover to point at.
    eye: tracking::Anchor,
    /// A reply just sent, shown before the store has it: `true` once it
    /// went out. Its ID is a stand-in.
    pending: Option<bool>,
}

impl Part {
    fn new(id: MessageId, row: Option<Rc<Row>>, expanded: bool) -> Self {
        Self {
            id,
            row,
            expanded,
            body: None,
            details: false,
            height: Rc::default(),
            from: 0.0,
            turns: 0,
            activity: None,
            receipt: None,
            message_id: None,
            ticks: std::collections::HashMap::new(),
            eye: Rc::default(),
            pending: None,
        }
    }

    /// Opens or folds it, animating from how it looks now.
    fn set_expanded(&mut self, expanded: bool, mail: &Mail) {
        if expanded == self.expanded {
            return;
        }
        self.expanded = expanded;
        self.from = self.height.get();
        self.turns += 1;
        if expanded && self.body.is_none() {
            self.body = Some(read(mail, self.id));
        }
    }
}

struct Body {
    /// `None` when the message body is not stored (not downloaded yet).
    view: Option<MessageView>,
    /// The body, in blocks of consecutive quoted or unquoted lines.
    blocks: Vec<(bool, SharedString)>,
    cut: bool,
    /// The HTML body laid out, when the message has one.
    doc: Option<Document>,
    /// The remote images of `doc`.
    remote: Vec<String>,
    /// The SVG pictures `doc` carries, drawn to bitmaps before they show.
    svgs: Vec<Arc<[u8]>>,
    /// The mail provider vouched for the `From` address (DMARC or aligned
    /// DKIM passed), so "Always show images from" this sender holds. Only
    /// looked at when `doc` has remote images.
    authenticated: bool,
    /// Encrypted or signed: what opening it found.
    security: Option<Secured>,
    /// The raw message, until it is handed to GnuPG.
    sealed: Option<Vec<u8>>,
    /// The message as GnuPG opened it (decrypted, or with the signature
    /// taken off), which the attachments are read from. In memory only.
    opened: Option<Arc<Vec<u8>>>,
    /// The invitation (or answer, or cancellation) the message carries.
    invite: Option<Arc<invite::Invite>>,
    /// The video call links in it, for Join buttons.
    calls: Vec<(katna_core::meeting::Service, String)>,
}

impl Body {
    /// Encrypted (being opened, or opened): remote content stays blocked.
    fn encrypted(&self) -> bool {
        match &self.security {
            Some(Secured::Opening(protection)) => {
                matches!(protection, katna_crypto::Protection::Encrypted(_))
            }
            Some(Secured::Opened(security)) => security.encrypted(),
            None => false,
        }
    }
}

impl Conversation {
    /// The ids of its messages.
    pub(super) fn message_ids(&self) -> HashSet<MessageId> {
        self.parts.iter().map(|p| p.id).collect()
    }

    /// Its subject, a message of it, and the sender of its newest
    /// message, for muting it or its sender.
    pub(super) fn mute_info(&self) -> Option<(String, MessageId, String)> {
        let last = self.parts.iter().rev().find(|p| p.pending.is_none())?;
        let sender = last
            .row
            .as_ref()
            .map(|r| r.sender.clone())
            .unwrap_or_default();
        Some((self.subject.clone(), last.id, sender))
    }

    /// The invitations of the loaded messages.
    fn invites(&self) -> impl Iterator<Item = &Arc<invite::Invite>> {
        self.parts
            .iter()
            .filter_map(|p| p.body.as_ref()?.invite.as_ref())
    }

    /// A loaded message of it is HTML that sets its own colors.
    fn has_own_colors(&self) -> bool {
        self.parts.iter().any(|p| {
            p.body
                .as_ref()
                .and_then(|b| b.doc.as_ref())
                .is_some_and(|d| d.styled || d.background.is_some())
        })
    }

    /// The loaded message `id`, or by default the newest loaded one: what a
    /// reply or forward starts from.
    pub(super) fn view(&self, id: Option<MessageId>) -> Option<&MessageView> {
        fn view(part: &Part) -> Option<&MessageView> {
            part.body.as_ref().and_then(|b| b.view.as_ref())
        }
        match id {
            Some(id) => self.parts.iter().find(|p| p.id == id).and_then(view),
            None => self
                .parts
                .iter()
                .rev()
                .filter(|p| p.pending.is_none())
                .find_map(view),
        }
    }

    /// The message [`Self::view`] picks.
    pub(super) fn view_id(&self, id: Option<MessageId>) -> Option<MessageId> {
        let has_view = |p: &&Part| p.body.as_ref().is_some_and(|b| b.view.is_some());
        match id {
            Some(id) => self.parts.iter().find(|p| p.id == id).filter(has_view),
            None => self
                .parts
                .iter()
                .rev()
                .filter(|p| p.pending.is_none())
                .find(has_view),
        }
        .map(|p| p.id)
    }

    pub(super) fn subject(&self) -> &str {
        &self.subject
    }

    /// The line of the list it is, to open it elsewhere.
    pub(super) fn entry(&self) -> Option<data::Entry> {
        let latest = self.parts.iter().rev().find(|p| p.pending.is_none())?.id;
        Some(data::Entry {
            key: self.key,
            latest,
        })
    }

    /// Every message of the conversation, oldest first, as it is printed:
    /// the loaded ones as they are shown (protected ones as GnuPG opened
    /// them), folded ones read from the store.
    pub(super) fn printable(&self, mail: &Mail) -> Vec<Printable> {
        self.parts
            .iter()
            .map(|part| {
                let read_now;
                let body = match &part.body {
                    Some(body) => body,
                    None => {
                        read_now = read(mail, part.id);
                        &read_now
                    }
                };
                // Protected and not opened (or failed to open): only its
                // headers are known.
                let sealed = match &body.security {
                    Some(Secured::Opening(_)) => true,
                    Some(Secured::Opened(_)) => body.opened.is_none(),
                    None => false,
                };
                Printable {
                    id: part.id,
                    row: part.row.clone(),
                    view: body.view.clone(),
                    doc: body.doc.clone().filter(|_| !sealed),
                    encrypted: body.encrypted(),
                    authenticated: body.authenticated,
                    sealed,
                }
            })
            .collect()
    }

    /// Where the attachments of message `id` are read from.
    pub(super) fn attachment_source(&self, id: MessageId) -> AttachmentSource {
        let Some(body) = self
            .parts
            .iter()
            .find(|p| p.id == id)
            .and_then(|p| p.body.as_ref())
        else {
            return AttachmentSource::Stored;
        };
        match (&body.opened, &body.security) {
            (Some(raw), _) => AttachmentSource::Opened {
                raw: raw.clone(),
                encrypted: body.encrypted(),
            },
            // Protected and not opened (yet): nothing to read.
            (None, Some(_)) => AttachmentSource::Sealed,
            (None, None) => AttachmentSource::Stored,
        }
    }

    pub(super) fn load(mail: &mut Mail, key: EntryKey) -> Self {
        let ids = mail.entry_messages(key);
        let rows = mail.message_rows(&ids);
        let last = ids.len().saturating_sub(1);
        let parts: Vec<Part> = ids
            .iter()
            .zip(rows)
            .enumerate()
            .map(|(ix, (id, row))| {
                let unread = row.as_ref().is_some_and(|r| r.unread);
                let mut part = Part::new(*id, row, ix == last || unread);
                if part.expanded {
                    part.body = Some(read(mail, part.id));
                }
                part
            })
            .collect();
        let subject = parts
            .iter()
            .find_map(|p| p.row.as_ref().map(|r| r.subject.clone()))
            .unwrap_or_else(|| tr!("reader-no-subject"));
        let headers = ids
            .iter()
            .filter_map(|id| mail.message_id_header(*id))
            .collect();
        let came_unread = parts
            .iter()
            .filter(|p| p.row.as_ref().is_some_and(|r| r.unread))
            .map(|p| p.id)
            .collect();
        let mut conversation = Self {
            key,
            subject,
            parts,
            show_all: false,
            original_colors: false,
            colors_chat: None,
            colors_flips: 0,
            seen: None,
            drafts: HashSet::new(),
            headers,
            chat: chat::ChatState::default(),
            came_unread,
            tops: Rc::default(),
            jump: None,
            folders: Vec::new(),
            category: None,
        };
        conversation.read_labels(mail);
        conversation.read_tracking(mail);
        conversation.read_drafts(mail);
        conversation
    }

    /// Reads where its messages are and the inbox tab of the newest.
    fn read_labels(&mut self, mail: &Mail) {
        let mut folders = Vec::new();
        for part in self.parts.iter().filter(|p| p.pending.is_none()) {
            for folder in mail.message_folders(part.id) {
                if !folders.contains(&folder) {
                    folders.push(folder);
                }
            }
        }
        self.folders = folders;
        self.category = self
            .parts
            .iter()
            .rev()
            .find(|p| p.pending.is_none())
            .and_then(|p| mail.message_category(p.id));
    }

    /// Notes which of its messages are drafts.
    fn read_drafts(&mut self, mail: &Mail) {
        let ids: Vec<MessageId> = self.parts.iter().map(|p| p.id).collect();
        self.drafts = mail.drafts(&ids).into_iter().collect();
    }

    /// Reads what the recipients of the user's tracked messages did, and
    /// which messages are read receipts for the user's mail.
    fn read_tracking(&mut self, mail: &Mail) {
        let mine = |part: &Part| part.row.as_ref().is_some_and(|r| mail.is_me(&r.sender));
        let any_mine = self.parts.iter().any(mine);
        for part in &mut self.parts {
            part.activity = part
                .row
                .as_ref()
                .and_then(|r| r.tracking)
                .and_then(|_| mail.activity(part.id));
            if any_mine && part.receipt.is_none() {
                part.receipt = Some((!mine(part)).then(|| mail.receipt(part.id)).flatten());
            }
        }
        for part in &mut self.parts {
            if part.message_id.is_none() && mine(part) {
                part.message_id = mail.message_id_header(part.id);
            }
        }
        let ticks: Vec<_> = self
            .parts
            .iter()
            .map(|part| match &part.message_id {
                Some(id) if mine(part) => {
                    // Read receipts in the conversation, with when they came.
                    let read: Vec<_> = self
                        .parts
                        .iter()
                        .filter_map(|p| {
                            let receipt = p.receipt.as_ref()?.as_ref()?;
                            (receipt.original.as_ref() == Some(id))
                                .then(|| (receipt, p.row.as_ref().and_then(|r| r.date)))
                        })
                        .collect();
                    ticks::ticks(&mail.receipts(id), &read, part.activity.as_ref())
                }
                _ => std::collections::HashMap::new(),
            })
            .collect();
        for (part, ticks) in self.parts.iter_mut().zip(ticks) {
            part.ticks = ticks;
        }
    }

    /// The read receipts in the conversation for `part`.
    fn receipts_for(&self, part: &Part) -> Vec<&crate::receipts::Receipt> {
        let Some(id) = &part.message_id else {
            return Vec::new();
        };
        self.parts
            .iter()
            .filter_map(|p| p.receipt.as_ref()?.as_ref())
            .filter(|r| r.original.as_ref() == Some(id))
            .collect()
    }

    /// Reads the messages' flags again, keeping what is open.
    pub(super) fn refresh(&mut self, mail: &mut Mail) {
        let ids = mail.entry_messages(self.key);
        let rows = mail.message_rows(&ids);
        let mut old: Vec<Part> = std::mem::take(&mut self.parts);
        self.parts = ids
            .into_iter()
            .zip(rows)
            .map(|(id, row)| match old.iter().position(|p| p.id == id) {
                Some(ix) => {
                    let mut part = old.swap_remove(ix);
                    part.row = row;
                    // Downloaded since, by the sync or on request.
                    if part.body.as_ref().is_some_and(|b| b.view.is_none()) {
                        part.body = Some(read(mail, id));
                    }
                    part
                }
                None => Part {
                    body: Some(read(mail, id)),
                    ..Part::new(id, row, true)
                },
            })
            .collect();
        // Replies just sent stay until the store has them.
        self.parts
            .extend(old.into_iter().filter(|p| p.pending.is_some()));
        self.read_labels(mail);
        self.read_tracking(mail);
        self.read_drafts(mail);
        self.chat.pins.forget();
    }

    /// The `Message-ID`s of the user's stored messages in it.
    pub(super) fn own_message_ids(&self) -> Vec<String> {
        self.parts
            .iter()
            .filter(|p| p.pending.is_none())
            .filter_map(|p| p.message_id.clone())
            .collect()
    }

    /// Shows `cards`, replies just sent, after its stored messages, in
    /// place of those shown before.
    pub(super) fn place_sent<'a>(&mut self, cards: impl Iterator<Item = &'a SentCard>) {
        let (mut shown, stored): (Vec<Part>, Vec<Part>) = std::mem::take(&mut self.parts)
            .into_iter()
            .partition(|p| p.pending.is_some());
        self.parts = stored;
        for card in cards {
            let mut part = match shown.iter().position(|p| p.id == card.id) {
                Some(ix) => shown.swap_remove(ix),
                None => Part {
                    body: Some(shown_body(&card.raw)),
                    message_id: Some(card.message_id.clone()),
                    ..Part::new(card.id, Some(card.row.clone()), true)
                },
            };
            part.pending = Some(card.sent.is_some());
            self.parts.push(part);
        }
    }

    /// Open messages whose body is not stored yet.
    pub(super) fn missing_bodies(&self) -> Vec<MessageId> {
        self.parts
            .iter()
            .filter(|p| {
                (p.expanded || self.chat.all_bodies)
                    && p.body.as_ref().is_some_and(|b| b.view.is_none())
            })
            .map(|p| p.id)
            .collect()
    }

    /// Reads message `id` again, once its body is downloaded.
    pub(super) fn reload_body(&mut self, id: MessageId, mail: &Mail) {
        if let Some(part) = self
            .parts
            .iter_mut()
            .find(|p| p.id == id && p.body.is_some())
        {
            part.body = Some(read(mail, id));
        }
    }

    pub(super) fn unread_messages(&self) -> Vec<MessageId> {
        self.parts
            .iter()
            .filter(|p| p.pending.is_none() && p.row.as_ref().is_some_and(|r| r.unread))
            .map(|p| p.id)
            .collect()
    }

    fn toggle(&mut self, ix: usize, mail: &Mail) {
        if let Some(part) = self.parts.get_mut(ix) {
            part.set_expanded(!part.expanded, mail);
        }
    }

    fn set_all(&mut self, expanded: bool, mail: &Mail) {
        let last = self.parts.len().saturating_sub(1);
        for (ix, part) in self.parts.iter_mut().enumerate() {
            part.set_expanded(expanded || ix == last, mail);
        }
        self.show_all = expanded;
    }

    fn all_expanded(&self) -> bool {
        self.parts.iter().all(|p| p.expanded)
    }

    /// Each open message's remote images, with what decides whether they
    /// may load.
    pub(super) fn remote_content(&self) -> Vec<RemoteContent> {
        self.parts
            .iter()
            .filter(|p| p.expanded)
            .filter_map(|p| {
                // Encrypted mail never loads remote content: a fetch would
                // tell the sender (or whoever altered the message) that it
                // was opened, and could leak its text (EFAIL).
                let body = p.body.as_ref().filter(|b| !b.encrypted())?;
                let sender = body.view.as_ref()?.from.first()?.email.clone();
                Some(RemoteContent {
                    id: p.id,
                    sender,
                    authenticated: body.authenticated,
                    urls: body.remote.clone(),
                })
            })
            .collect()
    }

    /// The SVG pictures the open messages carry.
    pub(super) fn carried_svgs(&self) -> Vec<Arc<[u8]>> {
        self.parts
            .iter()
            .filter(|p| p.expanded)
            .filter_map(|p| p.body.as_ref())
            .flat_map(|b| b.svgs.iter().cloned())
            .collect()
    }

    /// Each open, downloaded message of the conversation.
    pub(super) fn open_views(&self) -> impl Iterator<Item = (MessageId, &MessageView)> {
        self.parts
            .iter()
            .filter(|p| p.expanded || self.chat.all_bodies)
            .filter_map(|p| Some((p.id, p.body.as_ref()?.view.as_ref()?)))
    }
}

/// The remote images of an open message.
pub(super) struct RemoteContent {
    pub id: MessageId,
    /// Its `From` address.
    pub sender: String,
    /// The provider vouched for `sender` ([`Body::authenticated`]).
    pub authenticated: bool,
    pub urls: Vec<String>,
}

/// A message of the conversation, for printing.
pub(super) struct Printable {
    pub id: MessageId,
    pub row: Option<Rc<Row>>,
    /// `None` when it is not downloaded yet.
    pub view: Option<MessageView>,
    /// Its HTML body laid out, as the reader draws it.
    pub doc: Option<Document>,
    /// Encrypted: its remote pictures are never loaded.
    pub encrypted: bool,
    /// The provider vouched for its sender ([`Body::authenticated`]).
    pub authenticated: bool,
    /// Encrypted or signed, and its text not opened.
    pub sealed: bool,
}

/// What the list of messages shows: a message, or a fold of several.
/// What the reading pane's toolbar leaves to the More menu where it is
/// too narrow for every button. Back, Archive, More and the Newer and
/// Older arrows always stay; the rest go one by one, least used first
/// (`Squeeze::DROP_ORDER`), so the arrows are never pushed off.
#[derive(Debug, Clone, Copy)]
pub(super) struct Squeeze {
    pub new_window: bool,
    pub print: bool,
    pub colors: bool,
    /// The sparkle that sums up the conversation.
    pub summary: bool,
    pub contact: bool,
    pub move_to: bool,
    /// The bell that mutes the conversation.
    pub mute: bool,
    pub unread: bool,
    pub spam: bool,
    /// The lines between the groups of buttons.
    pub separators: bool,
    pub delete: bool,
    /// "3 of 120" beside the arrows.
    pub position: bool,
    /// Archive, which only a phone's chat leaves to the More menu: its
    /// header takes the toolbar's place.
    pub archive: bool,
}

/// A toolbar button's width and the toolbar's gap between buttons.
const BUTTON: f32 = 40.0;
const TOOL_GAP: f32 = 2.0;
/// A separator with its margins.
const SEPARATOR: f32 = 13.0;
/// The toolbar's padding, the card's edge and a little room to spare.
const TOOLBAR_FIXED: f32 = 16.0 + 6.0;

impl Squeeze {
    pub const NONE: Self = Self {
        new_window: false,
        print: false,
        colors: false,
        summary: false,
        contact: false,
        move_to: false,
        mute: false,
        unread: false,
        spam: false,
        separators: false,
        delete: false,
        position: false,
        archive: false,
    };

    /// Everything in the More menu.
    pub const ALL: Self = Self {
        new_window: true,
        print: true,
        colors: true,
        summary: true,
        contact: true,
        move_to: true,
        mute: true,
        unread: true,
        spam: true,
        separators: true,
        delete: true,
        position: true,
        archive: true,
    };

    /// Fits the items `shown` into a toolbar `width` wide, leaving off
    /// those `start` already does and then the least used.
    fn fit(width: f32, shown: &Toolbar, start: Self) -> Self {
        crate::widgets::fold(start, &Self::DROP_ORDER, |squeeze| {
            shown.width(squeeze) <= width
        })
    }

    const DROP_ORDER: [fn(&mut Self); 12] = [
        |s| s.new_window = true,
        |s| s.print = true,
        |s| s.mute = true,
        |s| s.colors = true,
        |s| s.contact = true,
        |s| s.move_to = true,
        |s| s.summary = true,
        |s| s.unread = true,
        |s| s.spam = true,
        |s| s.separators = true,
        |s| s.delete = true,
        |s| s.position = true,
    ];
}

/// Which of the reading pane's toolbar items this window and message have
/// at all, before any squeezing.
struct Toolbar {
    back: bool,
    separators: bool,
    contact: bool,
    colors: bool,
    summary: bool,
    new_window: bool,
    /// The width of "3 of 120", or `None` without it.
    position: Option<f32>,
    arrows: bool,
}

impl Toolbar {
    fn width(&self, squeeze: &Squeeze) -> f32 {
        let mut buttons = 2.0; // Archive and More
        let mut rest = 0.0;
        let mut items = 1.0; // the spacer
        let mut add = |on: bool, n: f32| {
            if on {
                buttons += n;
            }
        };
        add(self.back, 1.0);
        add(!squeeze.spam, 1.0);
        add(!squeeze.delete, 1.0);
        add(!squeeze.unread, 1.0);
        add(!squeeze.move_to, 1.0);
        add(!squeeze.mute, 1.0);
        add(self.contact && !squeeze.contact, 1.0);
        add(self.colors && !squeeze.colors, 1.0);
        add(self.summary && !squeeze.summary, 1.0);
        add(!squeeze.print, 1.0);
        add(self.new_window && !squeeze.new_window, 1.0);
        add(self.arrows, 2.0);
        if self.separators && !squeeze.separators {
            let n = if self.back { 2.0 } else { 1.0 };
            rest += n * SEPARATOR;
            items += n;
        }
        if let Some(position) = self.position.filter(|_| !squeeze.position) {
            rest += position;
            items += 1.0;
        }
        // Archive, Report spam and Delete sit together without gaps.
        items += buttons - f32::from(!squeeze.spam) - f32::from(!squeeze.delete);
        TOOLBAR_FIXED + buttons * BUTTON + rest + items * TOOL_GAP
    }
}

enum Shown {
    Part(usize),
    Fold(usize),
}

impl MailWindow {
    /// The id of the open conversation's `ix`th message.
    pub(super) fn part_id(&self, ix: usize) -> Option<MessageId> {
        self.reader.as_ref()?.parts.get(ix).map(|p| p.id)
    }

    /// The plain text of message `id` in the open conversation, unless it
    /// is encrypted or empty.
    pub(super) fn plain_text_of(&self, id: MessageId) -> Option<String> {
        let part = self.reader.as_ref()?.parts.iter().find(|p| p.id == id)?;
        let body = part.body.as_ref().filter(|b| !b.encrypted())?;
        Some(body.view.as_ref()?.body.clone()).filter(|text| !text.trim().is_empty())
    }

    /// Whether the open conversation can switch between its own colors
    /// and dark ones.
    /// Not in a chat, whose bubbles show text, not the mail's own look.
    pub(super) fn original_colors_offered(&self, th: &Theme) -> bool {
        self.own_colors(th) && !self.chat_shown()
    }

    /// Whether the open conversation's mail sets colors of its own that a
    /// dark theme turns dark.
    fn own_colors(&self, th: &Theme) -> bool {
        th.dark
            && self.config.mail.dark_mail
            && self.reader.as_ref().is_some_and(|r| r.has_own_colors())
    }

    /// The original colors button in the toolbar, sliding out as the
    /// conversation turns into a chat and back as it turns into mail, so
    /// the buttons beside it move rather than jump.
    fn original_colors_slot(&mut self, th: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.own_colors(th) {
            return None;
        }
        let chat = self.chat_shown();
        let reader = self.reader.as_mut()?;
        if reader.colors_chat.is_some_and(|was| was != chat) {
            reader.colors_flips += 1;
        }
        reader.colors_chat = Some(chat);
        let flips = reader.colors_flips;
        if flips == 0 {
            return (!chat)
                .then(|| self.original_colors_toggle(th, cx))
                .flatten();
        }
        let toggle = self.original_colors_toggle(th, cx)?;
        // An icon button's width.
        let width = 40.0;
        Some(
            div()
                .flex_none()
                .overflow_hidden()
                .child(toggle)
                .with_animation(
                    ("reader-colors-slot", flips),
                    Animation::new(katna_ui::motion::time(Duration::from_millis(220)))
                        .with_easing(ease_out_quint()),
                    move |d, t| {
                        let shown = if chat { 1.0 - t } else { t };
                        // Out of sight it takes no room, its gap neither.
                        d.w(px(width * shown))
                            .ml(px(-2.0 * (1.0 - shown)))
                            .opacity(shown)
                    },
                )
                .into_any_element(),
        )
    }

    /// In a dark theme, the button that shows the open conversation's HTML
    /// mail in its sender's own colors, or back in dark ones. Only where
    /// a message sets its own colors, so there is something to switch.
    fn original_colors_toggle(&self, th: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.own_colors(th) {
            return None;
        }
        let on = self.reader.as_ref()?.original_colors;
        Some(
            icon_button_colored(
                "reader-original-colors",
                "contrast",
                20.0,
                if on { th.accent } else { th.text_dim },
                th,
            )
            .when(on, |d| d.bg(rgba(th.hover)))
            .tooltip(tip(
                if on {
                    tr!("reader-dark-colors")
                } else {
                    tr!("reader-original-colors")
                },
                th,
            ))
            .on_click(cx.listener(|this, _, _, cx| {
                if let Some(reader) = &mut this.reader {
                    reader.original_colors = !reader.original_colors;
                }
                cx.notify();
            }))
            .into_any_element(),
        )
    }
}

impl MailWindow {
    /// The reading pane beside the list: its own card.
    pub(super) fn render_reader_card(&mut self, th: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let (radius, outline) = (
            self.layout.shape.card_radius(),
            self.layout.shape.card_outline(),
        );
        let (shadow, edge) = self.card_edges(self.card_keys(true), outline);
        let keys = self.reader_keys;
        div()
            .id("reader-card")
            .key_context(READER_CONTEXT)
            .track_focus(&self.reader_focus)
            .size_full()
            .flex()
            .flex_col()
            .relative()
            .overflow_hidden()
            .map(|d| {
                let fill = if self.chat_shown() {
                    th.chat_pane()
                } else {
                    th.pane()
                };
                crate::widgets::card(d, th, fill, radius, shadow)
            })
            .p(px(outline))
            .on_action(cx.listener(Self::reader_back))
            .on_action(cx.listener(Self::select_next))
            .on_action(cx.listener(Self::select_previous))
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
            .on_action(cx.listener(Self::summarize_key))
            .child(self.render_reader_toolbar(th, cx))
            .child(div().flex_1().min_h_0().child(self.render_reader(th, cx)))
            .children(card_outline(th, radius, edge))
            // Which pane has the keys: a faint accent edge on this one.
            .when(keys, |d| {
                d.child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .right_0()
                        .bottom_0()
                        .rounded(px(radius))
                        .border_1()
                        .border_color(rgba(fade(th.accent, 0.5))),
                )
            })
            .into_any_element()
    }

    /// "3 of 120" for the open conversation, or `None` where the toolbar
    /// has no Newer and Older (a phone, a conversation window).
    fn reader_position(&self) -> Option<(usize, String)> {
        let count = self.entries.len();
        if self.layout.shape.is_phone() || self.detached || count == 0 {
            return None;
        }
        let ix = self
            .reader
            .as_ref()
            .and_then(|r| self.entries.iter().position(|e| e.key == r.key))
            .or(self.selected)
            .unwrap_or(0);
        let text = tr!(
            "reader-position",
            position = ix as u64 + 1,
            total = count as u64
        );
        Some((ix, text))
    }

    /// What the reading pane's toolbar leaves to the More menu.
    pub(super) fn reader_squeeze(&self, th: &Theme) -> Squeeze {
        let phone = self.layout.shape.is_phone();
        // A phone's chat has no toolbar: its header keeps Back and More.
        if self.phone_chat() {
            return Squeeze::ALL;
        }
        let shown = Toolbar {
            back: !self.detached,
            separators: !phone,
            contact: self.contact_offered(),
            colors: self.original_colors_offered(th),
            summary: self.summaries_on() && !self.chat_shown(),
            new_window: !self.detached,
            // About 6.5 px a character at 12 px, and its 8 px padding.
            position: self
                .reader_position()
                .map(|(_, text)| text.chars().count() as f32 * 6.5 + 16.0),
            arrows: !phone && !self.detached,
        };
        // A phone keeps these in the More menu.
        let start = Squeeze {
            print: phone,
            new_window: phone,
            move_to: phone,
            mute: phone,
            ..Squeeze::NONE
        };
        Squeeze::fit(self.reader_width(), &shown, start)
    }

    /// The open conversation shows as a chat on a phone, whose header
    /// takes the toolbar's place.
    pub(super) fn phone_chat(&self) -> bool {
        self.layout.shape.is_phone() && self.chat_shown()
    }

    pub(super) fn render_reader_toolbar(
        &mut self,
        th: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if self.phone_chat() {
            return div().into_any_element();
        }
        let count = self.entries.len();
        let back = if self.split() {
            icon_button("reader-close", "close", 20.0, th).tooltip(tip(tr!("reader-close"), th))
        } else {
            icon_button("reader-back", "back", 20.0, th).tooltip(tip(tr!("reader-back"), th))
        }
        .on_click(
            cx.listener(|this, _, window, cx| this.close_message(&super::CloseMessage, window, cx)),
        );
        let phone = self.layout.shape.is_phone();
        let squeeze = self.reader_squeeze(th);
        let gmail = self.account().is_some_and(|a| self.tree.is_gmail(a));
        let separators = !phone && !squeeze.separators;
        // A phone moves between conversations from the list; a
        // conversation window shows only its own.
        let position = self.reader_position();
        let ix = position.as_ref().map_or(0, |(ix, _)| *ix);
        let more = {
            let more = icon_button("reader-more", "more", 20.0, th)
                .when(
                    !matches!(
                        self.menu,
                        Some(Menu::ReaderMore | Menu::MoveTo | Menu::LabelAs)
                    ),
                    |d| d.tooltip(tip(tr!("reader-more"), th)),
                )
                .on_click(cx.listener(|this, _, _, cx| this.toggle_menu(Menu::ReaderMore, cx)));
            let more = self.with_menu(more, Menu::ReaderMore, th, cx);
            // Move to, from the More menu, opens under it.
            if squeeze.move_to {
                let more = self.with_menu(more, Menu::MoveTo, th, cx);
                self.with_menu(more, Menu::LabelAs, th, cx)
            } else {
                more
            }
        };
        // What gives way when the pane is narrow; the arrows never do.
        let actions = div()
            .flex_1()
            .min_w_0()
            .overflow_hidden()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(2.0))
            // A conversation window closes from its own frame.
            .when(!self.detached, |d| {
                d.child(back).when(separators, |d| d.child(separator(th)))
            })
            .child(self.action_buttons("reader", squeeze, th, cx))
            .when(separators, |d| d.child(separator(th)))
            .when(!squeeze.unread, |d| {
                d.child(
                    icon_button("reader-unread", "mark-unread", 20.0, th)
                        .tooltip(tip(tr!("reader-mark-unread"), th))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.mark_unread(&super::MarkUnread, window, cx)
                        })),
                )
            })
            .when(!squeeze.move_to, |d| {
                d.child({
                    let move_to = icon_button("reader-move", "move-to", 20.0, th)
                        .when(self.menu != Some(Menu::MoveTo), |d| {
                            d.tooltip(tip(tr!("reader-move-to"), th))
                        })
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_menu(Menu::MoveTo, cx)));
                    self.with_menu(move_to, Menu::MoveTo, th, cx)
                })
                .when(gmail, |d| {
                    let label_as = icon_button("reader-label-as", "tag", 20.0, th)
                        .when(self.menu != Some(Menu::LabelAs), |d| {
                            d.tooltip(tip(tr!("menu-label-as"), th))
                        })
                        .on_click(
                            cx.listener(|this, _, _, cx| this.toggle_menu(Menu::LabelAs, cx)),
                        );
                    d.child(self.with_menu(label_as, Menu::LabelAs, th, cx))
                })
            })
            .when(!squeeze.mute, |d| d.child(self.reader_mute_button(th, cx)))
            .when(!squeeze.summary && !self.chat_shown(), |d| {
                d.children(self.summary_button("reader-summary", th, cx))
            })
            .child(more)
            .child(div().flex_1())
            .when(!squeeze.contact, |d| {
                d.children(self.contact_toggle(th, cx))
            })
            .when(!squeeze.colors, |d| {
                d.children(self.original_colors_slot(th, cx))
            })
            .when(!squeeze.print, |d| {
                d.child(
                    icon_button("reader-print", "print", 20.0, th)
                        .tooltip(tip(tr!("reader-print-all"), th))
                        .on_click(
                            cx.listener(|this, _, window, cx| this.print_conversation(window, cx)),
                        ),
                )
            })
            .when(!self.detached && !squeeze.new_window, |d| {
                d.child(
                    icon_button("reader-new-window", "open-external", 20.0, th)
                        .tooltip(tip(tr!("reader-new-window"), th))
                        .on_click(cx.listener(|this, _, _, cx| this.open_reader_in_window(cx))),
                )
            });
        let steps = position.map(|(_, text)| {
            div()
                .flex_none()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(2.0))
                .when(!squeeze.position, |d| {
                    d.child(
                        div()
                            .px(px(8.0))
                            .text_size(px(12.0))
                            .text_color(rgba(th.text_faint))
                            .whitespace_nowrap()
                            .child(text),
                    )
                })
                .child(
                    icon_button("newer", "chevron-left", 20.0, th)
                        .tooltip(tip(tr!("reader-newer"), th))
                        .when(ix == 0, |d| d.opacity(0.4))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.select_previous(&SelectPrevious, window, cx)
                        })),
                )
                .child(
                    icon_button("older", "chevron-right", 20.0, th)
                        .tooltip(tip(tr!("reader-older"), th))
                        .when(ix + 1 >= count, |d| d.opacity(0.4))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.select_next(&SelectNext, window, cx)
                        })),
                )
        });
        toolbar(th)
            .child(actions)
            .children(steps)
            .into_any_element()
    }

    pub(super) fn render_reader(&mut self, th: &Theme, cx: &mut Context<Self>) -> AnyElement {
        self.open_sealed(cx);
        self.fetch_remote(cx);
        self.download_bodies(cx);
        self.prepare_chat();
        self.prepare_summary();
        self.request_thumbnails(cx);
        if let Some(key) = self.reader.as_ref().map(|r| r.key) {
            self.text.begin(key);
        }
        if self.chat_shown() {
            return self.render_chat(th, cx);
        }
        // The link under the pointer was in another conversation.
        let conversation = self.reader.as_ref().map(|r| key_number(r.key));
        if self
            .hovered_link
            .as_ref()
            .is_some_and(|h| Some(h.conversation()) != conversation)
        {
            self.hovered_link = None;
        }
        // Notes about the conversation show under its subject.
        let notes = self
            .reader
            .as_ref()
            .map(|r| r.headers.clone())
            .and_then(|headers| {
                self.notes_page(cx);
                self.render_mail_notes(&headers, self.reader_indent(), th, cx)
            });
        let muted = self.render_muted_strip(th, cx);
        self.settle_summary_jump(cx);
        let summary = self.render_summary_card(th, cx);
        let Some(reader) = &self.reader else {
            return self.placeholder("", th);
        };
        if reader.parts.is_empty() {
            return self.placeholder(tr!("reader-removed"), th);
        }
        let all_expanded = reader.all_expanded();
        let key = reader.key;
        let chips: Vec<AnyElement> = self
            .header_chips(reader)
            .into_iter()
            .enumerate()
            .map(|(ix, chip)| self.render_header_chip(ix, chip, key, th, cx))
            .collect();
        let title = div()
            .flex()
            .flex_row()
            .items_start()
            .gap(px(12.0))
            .pl(px(self.reader_indent()))
            .pr(px(16.0))
            .pt(px(20.0))
            .pb(px(12.0))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap(px(8.0))
                    .child(
                        div()
                            .w_full()
                            .map(|d| {
                                // A very narrow pane takes a smaller title so
                                // its words don't break.
                                if self.reader_width() < STAR_FROM {
                                    d.text_size(px(18.0)).line_height(px(24.0))
                                } else {
                                    d.text_size(px(22.0)).line_height(px(28.0))
                                }
                            })
                            .text_color(rgba(th.text))
                            .child(reader.subject.clone()),
                    )
                    .when(!chips.is_empty(), |d| {
                        d.child(
                            div()
                                .flex()
                                .flex_row()
                                .flex_wrap()
                                .gap(px(space::S2))
                                .children(chips),
                        )
                    }),
            )
            // With the chat view on, the conversation can show as a chat.
            .when(self.config.experimental.chat_view, |d| {
                d.child(self.chat_switch(false, th, cx))
            })
            .when(reader.parts.len() > 1, |d| {
                d.child(
                    icon_button_colored(
                        "expand-all",
                        "expand",
                        20.0,
                        if all_expanded { th.accent } else { th.text_dim },
                        th,
                    )
                    .tooltip(tip(
                        if all_expanded {
                            tr!("reader-collapse-all")
                        } else {
                            tr!("reader-expand-all")
                        },
                        th,
                    ))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let (Some(reader), Ok(mail)) = (&mut this.reader, &this.mail) {
                            reader.set_all(!all_expanded, mail);
                        }
                        cx.notify();
                    })),
                )
            });

        // Fold runs of collapsed messages in the middle. `order` is the
        // messages as shown: oldest first, or newest first by the setting.
        let newest_first = self.config.mail.newest_first;
        let n = reader.parts.len();
        let order: Vec<usize> = if newest_first {
            (0..n).rev().collect()
        } else {
            (0..n).collect()
        };
        let mut shown = Vec::new();
        let mut at = 0;
        while at < n {
            let run_end = (at..n)
                .find(|&j| reader.parts[order[j]].expanded)
                .unwrap_or(n);
            let run = run_end - at;
            if !reader.show_all && at > 0 && run >= FOLD_AT && run_end < n {
                shown.push(Shown::Fold(run));
                at = run_end;
            } else {
                shown.push(Shown::Part(order[at]));
                at += 1;
            }
        }
        let parts: Vec<AnyElement> = shown
            .into_iter()
            .map(|s| match s {
                Shown::Part(ix) => self.render_part(ix, th, cx),
                Shown::Fold(count) => fold(count, th, cx),
            })
            .collect();

        // Reply, Reply all and Forward stay at the foot of the pane while
        // the conversation scrolls. A reply is written at the end of the
        // conversation itself, as in Gmail: it grows with its text and
        // scrolls with the messages. It stays at the end even with the
        // newest message first, where the view scrolls down to it.
        let key = reader.key;
        let reply = self.render_inline_reply(key, th, cx);
        // Drafts are edited, not answered.
        let drafts_only =
            self.mail.is_ok() && reader.parts.iter().all(|p| reader.drafts.contains(&p.id));
        let footer = (reply.is_none() && !drafts_only).then(|| self.render_reply_row(th, cx));
        // Where the link under the pointer really goes.
        let link_status = self
            .hovered_link
            .as_ref()
            .map(|link| rich::link_status(&link.url, th));
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
                            "reader-bar",
                            &self.reader_scroll,
                            div()
                                .id("reader")
                                .size_full()
                                .overflow_y_scroll()
                                .track_scroll(&self.reader_scroll)
                                .child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .pt(px(self.reader_head.get() + space::S4))
                                        .pb(px(24.0))
                                        .children(summary)
                                        .children(muted)
                                        .children(notes)
                                        .children(parts)
                                        .children(reply)
                                        .map(|d| self.text_area(d, cx))
                                        .with_animation(
                                            ("open-conversation", key_number(key)),
                                            Animation::new(katna_ui::motion::time(
                                                Duration::from_millis(280),
                                            ))
                                            .with_easing(ease_out_quint()),
                                            |el, t| el.opacity(t).mt(px(14.0 * (1.0 - t))),
                                        ),
                                ),
                            // The Files page's bar, over the open mail's
                            // right edge.
                            th.text_dim & 0xffff_ff00 | 0x99,
                        ),
                    )
                    // The subject stays at the top while the mails scroll
                    // under it, as the chat's header does.
                    .child(
                        self.pinned_head(
                            title.with_animation(
                                ("open-subject", key_number(key)),
                                Animation::new(katna_ui::motion::time(Duration::from_millis(280)))
                                    .with_easing(ease_out_quint()),
                                |el, t| el.opacity(t),
                            ),
                            th.pane(),
                            true,
                            th,
                        ),
                    )
                    .children(link_status),
            )
            .children(footer.map(|footer| {
                div()
                    .flex_none()
                    .border_t_1()
                    .border_color(rgba(th.divider))
                    .child(footer)
            }))
            .children(self.render_text_menu(th, cx))
            .into_any_element()
    }

    /// The header pinned over the top of the open mail or chat
    /// ([`crate::widgets::pinned_head`]), with a line under it once
    /// something is beneath when `line` (the chat's header draws its own).
    pub(super) fn pinned_head(
        &self,
        content: impl IntoElement,
        fill: u32,
        line: bool,
        th: &Theme,
    ) -> AnyElement {
        let under = line && unpx(self.reader_scroll.offset().y) < -0.5;
        let frost = self.config.experimental.frosted_headers;
        crate::widgets::pinned_head(content, fill, under, frost, self.reader_head.clone(), th)
    }

    fn render_part_content(&self, ix: usize, th: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let Some(reader) = &self.reader else {
            return div().into_any_element();
        };
        let part = &reader.parts[ix];
        let turn = part_turn(reader.key, ix, part.turns);
        let last = ix + 1 == reader.parts.len();
        let row = part.row.clone();
        let view = part.body.as_ref().and_then(|b| b.view.as_ref());
        let (name, email) = match (view.and_then(|v| v.from.first()), &row) {
            (Some(from), _) => (from.label().to_owned(), from.email.clone()),
            // Not read yet: the list line knows the sender, so the picture
            // (when one was already fetched) stays the same once it opens.
            (None, Some(row)) => (row.correspondent.clone(), row.sender.clone()),
            (None, None) => (tr!("reader-unknown-sender"), String::new()),
        };
        let now = jiff::Timestamp::now().as_second();
        let date = row
            .as_ref()
            .and_then(|r| r.date)
            .or(view.and_then(|v| v.date));
        let unread = row.as_ref().is_some_and(|r| r.unread);
        let flagged = row.as_ref().is_some_and(|r| r.flagged);
        let id = part.id;
        // A reply just sent: not stored yet, so nothing acts on it.
        let pending = part.pending.is_some();
        let waiting = part.pending == Some(false);
        let toggle = cx.listener(move |this, _, _, cx| {
            if let (Some(reader), Ok(mail)) = (&mut this.reader, &this.mail) {
                reader.toggle(ix, mail);
            }
            cx.notify();
        });

        if !part.expanded {
            let snippet = row.as_ref().map(|r| r.snippet.clone()).unwrap_or_default();
            let short_date = if waiting {
                tr!("reader-sending")
            } else {
                date.and_then(|d| format::local(d, &self.tz))
                    .zip(format::local(now, &self.tz))
                    .map(|(d, now)| format::list_date(d, now))
                    .unwrap_or_default()
            };
            return div()
                .id(("part", ix))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(16.0))
                .pl(px(16.0))
                .pr(px(24.0))
                .py(px(12.0))
                .border_t_1()
                .border_color(rgba(th.divider))
                .cursor_pointer()
                .hover(|s| s.bg(rgba(th.hover)))
                .on_click(toggle)
                .child(self.person_avatar(&name, &email, 40.0))
                .child(turn_fade(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(16.0))
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
                                                .font_weight(if unread {
                                                    FontWeight::BOLD
                                                } else {
                                                    FontWeight::MEDIUM
                                                })
                                                .text_color(rgba(th.text))
                                                .child(name),
                                        )
                                        .children(self.muted_mark(&email, 16.0, th)),
                                )
                                .child(
                                    div()
                                        .truncate()
                                        .text_size(px(13.0))
                                        .text_color(rgba(th.text_faint))
                                        .child(snippet),
                                ),
                        )
                        .child(
                            div()
                                .flex_none()
                                .text_size(px(12.0))
                                .text_color(rgba(th.text_faint))
                                .child(short_date),
                        ),
                    turn,
                ))
                .into_any_element();
        }

        let long_date = if waiting {
            Some(tr!("reader-sending"))
        } else {
            date.and_then(|d| {
                let long = format::long_date(format::local(d, &self.tz)?);
                Some(match format::ago(d, now) {
                    Some(ago) => tr!("reader-date-ago", date = long, ago = ago),
                    None => long,
                })
            })
        }
        .unwrap_or_default();
        let full_names = self.config.mail.full_names;
        let names = |list: &[katna_render::Address]| {
            let mut seen = std::collections::HashSet::new();
            let people: Vec<(bool, &katna_render::Address)> = list
                .iter()
                .filter(|a| seen.insert(a.email.to_lowercase()))
                .map(|a| (self.is_me(&a.email), a))
                .collect();
            recipient_names(&people, full_names).join(", ")
        };
        let recipients = view.map(|v| {
            let mut all = v.to.clone();
            all.extend(v.cc.iter().cloned());
            if part.ticks.is_empty() {
                return div()
                    .min_w_0()
                    .truncate()
                    .child(tr!("reader-to", names = names(&all)))
                    .into_any_element();
            }
            // Each name with its delivered or read tick.
            let mut seen = std::collections::HashSet::new();
            let people: Vec<(bool, &katna_render::Address)> = all
                .iter()
                .filter(|a| seen.insert(a.email.to_lowercase()))
                .map(|a| (self.is_me(&a.email), a))
                .collect();
            let labels = recipient_names(&people, full_names);
            let count = labels.len();
            div()
                .min_w_0()
                .flex()
                .flex_row()
                .items_center()
                .overflow_hidden()
                .whitespace_nowrap()
                .child(div().flex_none().mr(px(4.0)).child(tr!("reader-to-label")))
                .children(people.iter().zip(labels).enumerate().map(
                    |(i, ((_, address), label))| {
                        let tick = part.ticks.get(&address.email.to_lowercase());
                        div()
                            .flex_none()
                            .flex()
                            .flex_row()
                            .items_center()
                            .child(label)
                            .children(self.render_tick(tick, ("part-tick", ix * 1000 + i), th))
                            .when(i + 1 < count, |d| d.child(div().mr(px(4.0)).child(",")))
                    },
                ))
                .into_any_element()
        });
        // Clicking \u{201c}to\u{201d} turns the details the other way from the
        // Full headers setting.
        let details = part.details != self.config.mail.full_headers;
        // A very narrow pane leaves starring to the toolbar's More menu and
        // the date to the details under "to", and lets the name shrink
        // further.
        let roomy = self.reader_width() >= STAR_FROM;
        let mark = self
            .muted_mark(&email, 16.0, th)
            .map(|mark| div().flex_none().self_center().child(mark));
        // A muted sender's crossed bell takes its room from the date.
        let name_room =
            lerp(120.0, 48.0, self.reader_compact()) + if mark.is_some() { 22.0 } else { 0.0 };
        let header = div()
            .id(("part-header", ix))
            .flex()
            .flex_row()
            .items_start()
            .gap(px(8.0))
            .when(!last, |d| d.cursor_pointer().on_click(toggle))
            .child(
                div()
                    .flex_1()
                    .min_w(px(name_room))
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_baseline()
                            .gap(px(6.0))
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(14.0))
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(rgba(th.text))
                                    .child(name.clone()),
                            )
                            .children(mark)
                            .when(!email.is_empty() && email != name, |d| {
                                // Takes only the room the name leaves.
                                d.child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .text_size(px(12.0))
                                        .text_color(rgba(th.text_faint))
                                        .child(format!("<{email}>")),
                                )
                            }),
                    )
                    .when_some(recipients, |d, recipients| {
                        d.child(
                            div()
                                .id(("part-to", ix))
                                .flex()
                                .flex_row()
                                .items_center()
                                .cursor_pointer()
                                .text_size(px(12.0))
                                .text_color(rgba(th.text_faint))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    if let Some(part) =
                                        this.reader.as_mut().and_then(|r| r.parts.get_mut(ix))
                                    {
                                        part.details = !part.details;
                                    }
                                    cx.notify();
                                }))
                                .child(recipients)
                                .child(icon("drop-down", th.text_faint, 18.0)),
                        )
                    }),
            )
            .when(roomy, |d| {
                // Gives way to the sender's name in a narrow pane.
                d.child(
                    div()
                        .min_w_0()
                        .truncate()
                        .pt(px(2.0))
                        .text_size(px(12.0))
                        .text_color(rgba(th.text_faint))
                        .child(long_date.clone()),
                )
            })
            .when(!pending, |d| {
                d.children(self.seen_eye(ix, part, th, cx))
                    .children(self.seen_popover(ix, part, th, cx))
            })
            .when(roomy && !pending, |d| {
                d.child(
                    icon_button_colored(
                        ("part-star", ix),
                        if flagged { "star-filled" } else { "star" },
                        20.0,
                        if flagged { th.star } else { th.text_faint },
                        th,
                    )
                    .size(px(32.0))
                    .tooltip(tip(
                        if flagged {
                            tr!("reader-starred")
                        } else {
                            tr!("reader-not-starred")
                        },
                        th,
                    ))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.star_message(ix, id, !flagged, cx);
                    })),
                )
            })
            .when(!pending, |d| {
                d.child({
                    // Settings > General > Reply button.
                    let (kind, name, label) = if self.config.mail.reply_all {
                        (Kind::ReplyAll, "reply-all", tr!("reply-reply-all"))
                    } else {
                        (Kind::Reply, "reply", tr!("reply-reply"))
                    };
                    icon_button(("part-reply", ix), name, 20.0, th)
                        .tooltip(tip(label, th))
                        .size(px(32.0))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            cx.stop_propagation();
                            this.open_compose(kind, Some(id), window, cx);
                        }))
                })
            });

        let details_box = (details && view.is_some()).then(|| {
            let view = view.expect("checked");
            // Its text can be selected and copied as the mail's can; each
            // address opens its person in the contact panel, and its
            // right-click menu can copy it.
            let slot = match self.reader.as_ref() {
                Some(r) if self.config.mail.newest_first => r.parts.len() - 1 - ix,
                _ => ix,
            };
            let mut pieces = self.text.pieces(super::select::DETAILS_PART + slot, th);
            // Where the contact panel has no room, the card pops over.
            let panel = !self.layout.shape.is_phone();
            let line = |label: String, value: AnyElement| {
                div()
                    .flex()
                    .flex_row()
                    .gap(px(12.0))
                    .child(
                        div()
                            .w(px(64.0))
                            .flex_none()
                            .flex()
                            .justify_end()
                            .text_color(rgba(th.text_faint))
                            .child(label),
                    )
                    .child(div().flex_1().min_w_0().child(value))
            };
            let mut addresses = |field: &str, list: &[katna_render::Address]| -> AnyElement {
                let count = list.len();
                div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .items_center()
                    .children(list.iter().enumerate().map(|(i, a)| {
                        let shown = match &a.name {
                            Some(name) => format!("{name} <{}>", a.email),
                            None => a.email.clone(),
                        };
                        let (styled, holder) = pieces.piece(shown.into(), Vec::new());
                        let email = a.email.to_lowercase();
                        let copied = a.email.clone();
                        let spot = SharedString::from(format!("details-{ix}-{field}-{i}"));
                        let spot_id = spot.clone();
                        // A long address wraps in a narrow pane.
                        div()
                            .min_w_0()
                            .flex()
                            .flex_row()
                            .items_center()
                            .child(
                                holder
                                    .min_w_0()
                                    .relative()
                                    .id(spot.clone())
                                    .when(panel, |d| {
                                        d.cursor_pointer().hover(|s| s.text_color(rgba(th.text)))
                                    })
                                    .on_hover({
                                        let email = email.clone();
                                        cx.listener(move |this, hovered: &bool, _, cx| {
                                            if *hovered {
                                                this.read_person_ahead(&email, cx);
                                            }
                                        })
                                    })
                                    .on_click(cx.listener(
                                        move |this, e: &gpui::ClickEvent, _, cx| {
                                            // A drag that selected text is not a click.
                                            if this.text.is_empty() {
                                                let at = this.person_at(spot.clone(), e.position());
                                                this.show_person(&email, at, cx);
                                            }
                                        },
                                    ))
                                    .child(self.person_spot(spot_id))
                                    .on_mouse_down(
                                        MouseButton::Right,
                                        cx.listener(move |this, _, _, _| {
                                            this.text.menu_address = Some(copied.clone().into());
                                        }),
                                    )
                                    .child(styled),
                            )
                            .children(
                                self.muted_mark(&a.email, 14.0, th)
                                    .map(|bell| div().ml(px(4.0)).child(bell)),
                            )
                            .when(i + 1 < count, |d| d.child(div().mr(px(4.0)).child(",")))
                    }))
                    .into_any_element()
            };
            let from = addresses("from", &view.from);
            let to = (!view.to.is_empty()).then(|| addresses("to", &view.to));
            let copy = (!view.cc.is_empty()).then(|| addresses("cc", &view.cc));
            let mut plain = |text: String| -> AnyElement {
                let (styled, holder) = pieces.piece(text.into(), Vec::new());
                holder.child(styled).into_any_element()
            };
            let date = plain(long_date.clone());
            let subject = plain(view.subject.clone());
            let details = div()
                .mt(px(8.0))
                .p(px(12.0))
                .flex()
                .flex_col()
                .gap(px(4.0))
                .rounded(px(8.0))
                .border_1()
                .border_color(rgba(th.outline))
                .text_size(px(12.0))
                .text_color(rgba(th.text_dim))
                .child(line(tr!("reader-details-from"), from))
                .children(to.map(|to| line(tr!("reader-details-to"), to)))
                .children(copy.map(|cc| line(tr!("reader-details-cc"), cc)))
                .child(line(tr!("reader-details-date"), date))
                .child(line(tr!("reader-details-subject"), subject));
            self.selectable_body(super::select::DETAILS_PART + slot, details, cx)
                .with_animation(
                    ("details", ix),
                    Animation::new(katna_ui::motion::time(Duration::from_millis(180)))
                        .with_easing(ease_out_quint()),
                    |el, t| el.opacity(t),
                )
        });

        let body = match part.body.as_ref() {
            Some(Body {
                view: Some(view),
                blocks,
                cut,
                doc,
                authenticated,
                ..
            }) => {
                let too_long = match doc {
                    Some(doc) => doc.truncated,
                    None => *cut || view.truncated,
                };
                let encrypted = part.body.as_ref().is_some_and(Body::encrypted);
                let blocked = encrypted && doc.as_ref().is_some_and(|d| d.remote_images > 0);
                let notes = [
                    too_long.then(|| tr!("reader-too-long")),
                    blocked.then(|| tr!("reader-encrypted-images")),
                ];
                let allowed = !encrypted && self.remote.allowed(id, &email, *authenticated);
                let banner = doc
                    .as_ref()
                    .filter(|doc| doc.remote_images > 0 && !allowed && !encrypted)
                    .map(|_| self.images_banner(ix, id, &email, th, cx));
                // Images the body shows are not listed again.
                let listed: Vec<_> = view
                    .attachments
                    .iter()
                    .enumerate()
                    .filter(|(_, a)| {
                        !doc.as_ref().is_some_and(|doc| {
                            a.content_id.as_ref().is_some_and(|id| {
                                doc.inline_ids.iter().any(|i| i.eq_ignore_ascii_case(id))
                            })
                        })
                    })
                    .collect();
                let attachments = (!pending).then(|| self.attachment_cards(id, &listed, th, cx));
                let translation = if pending {
                    None
                } else {
                    self.translation_bar(ix, id, &view.body, encrypted, false, th, cx)
                };
                let translated = self.translated_blocks(id);
                let invite = part
                    .body
                    .as_ref()
                    .and_then(|b| b.invite.as_ref())
                    .filter(|_| !pending)
                    .map(|invite| self.invite_card(ix, id, invite, th, cx));
                let calls = part
                    .body
                    .as_ref()
                    .filter(|_| !pending)
                    .and_then(|b| self.call_links(ix, &b.calls, th));
                div()
                    .flex()
                    .flex_col()
                    .when(self.config.mail.limit_width, |d| d.max_w(px(MESSAGE_WIDTH)))
                    .pt(px(16.0))
                    .text_size(px(14.0))
                    .line_height(px(21.0))
                    .text_color(rgba(th.text))
                    .children(notes.into_iter().flatten().map(|note| {
                        div()
                            .mb(px(12.0))
                            .px(px(12.0))
                            .py(px(8.0))
                            .rounded(px(8.0))
                            .bg(rgba(th.on_pane(th.read_row)))
                            .text_size(px(12.0))
                            .text_color(rgba(th.text_dim))
                            .child(note)
                    }))
                    .children(banner)
                    .children(invite)
                    .children(calls)
                    .children(translation)
                    .child({
                        // Selection follows the order messages are shown in.
                        let slot = match self.reader.as_ref() {
                            Some(r) if self.config.mail.newest_first => r.parts.len() - 1 - ix,
                            _ => ix,
                        };
                        let mut pieces = self.text.pieces(slot, th);
                        let blocks = translated.as_ref().unwrap_or(blocks);
                        let links = self.reader.as_ref().map(|r| rich::Links {
                            window: cx.weak_entity(),
                            conversation: key_number(r.key),
                            part: slot,
                        });
                        let text = match doc.as_ref().filter(|_| translated.is_none()) {
                            Some(doc) => div().child(
                                Painter::new(
                                    th,
                                    &self.remote.images,
                                    &self.remote.drawn,
                                    links,
                                    allowed,
                                    self.remote.mono(),
                                    self.config.mail.dark_mail
                                        && !self.reader.as_ref().is_some_and(|r| r.original_colors),
                                    pieces,
                                )
                                .document(doc),
                            ),
                            // Addresses written out in plain text open as
                            // links do.
                            None => div().children(blocks.iter().enumerate().map(
                                |(n, (quoted, text))| {
                                    rich::linked_piece(
                                        &mut pieces,
                                        text.clone(),
                                        &[],
                                        |d| {
                                            d.when(*quoted, |d| {
                                                d.pl(px(12.0))
                                                    .border_l_2()
                                                    .border_color(rgba(th.outline))
                                                    .text_color(rgba(th.text_faint))
                                            })
                                        },
                                        n,
                                        links.clone(),
                                        th,
                                    )
                                },
                            )),
                        };
                        self.selectable_body(slot, text, cx)
                    })
                    .children(attachments.flatten())
                    .into_any_element()
            }
            _ => self.download_note(id, ix, th, cx),
        };

        // The picture sits where it does on the folded line, so only the
        // text changes when a message opens.
        div()
            .id(("part", ix))
            .flex()
            .flex_row()
            .pr(px(16.0))
            .pt(px(12.0))
            .pb(px(if last { 0.0 } else { 16.0 }))
            .when(ix > 0, |d| d.border_t_1().border_color(rgba(th.divider)))
            .child(
                div()
                    .w(px(PICTURE_COLUMN))
                    .flex_none()
                    .flex()
                    .justify_center()
                    // Their card: in the panel, else popping over here.
                    .child({
                        let pick = email.clone();
                        div()
                            .id(("part-picture", ix))
                            .relative()
                            .cursor_pointer()
                            .tooltip(crate::widgets::tip(tr!("chat-show-card"), th))
                            .on_hover({
                                let pick = pick.clone();
                                cx.listener(move |this, hovered: &bool, _, cx| {
                                    if *hovered {
                                        this.read_person_ahead(&pick, cx);
                                    }
                                })
                            })
                            .on_click(cx.listener(move |this, e: &gpui::ClickEvent, _, cx| {
                                let at = this.person_at(("part-picture", ix), e.position());
                                this.show_person(&pick, at, cx);
                            }))
                            .child(self.person_spot(("part-picture", ix)))
                            .child(self.person_avatar(&name, &email, 40.0))
                    }),
            )
            .child(turn_fade(
                div()
                    .flex_1()
                    .min_w_0()
                    .max_w(px(960.0))
                    .child(header)
                    .child(
                        // On a phone, and in a narrow pane, the message takes
                        // the room under the picture too, from its left edge.
                        div()
                            .ml(px(-PICTURE_COLUMN_INSET * self.reader_compact()))
                            .children(details_box)
                            .children(self.security_banner(part, th, cx))
                            .children(self.tracking_banner(part, th))
                            .child(body),
                    ),
                turn,
            ))
            .into_any_element()
    }

    /// A message of the open conversation. Opening or folding it grows or
    /// shrinks it from the height it had, so the messages below slide.
    fn render_part(&self, ix: usize, th: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let Some(reader) = &self.reader else {
            return div().into_any_element();
        };
        let part = &reader.parts[ix];
        let measured = part.height.clone();
        let target = part.height.clone();
        let from = part.from;
        let (tops, id, scroll) = (reader.tops.clone(), part.id, self.reader_scroll.clone());
        let content = div()
            .flex()
            .flex_col()
            .overflow_hidden()
            .on_children_prepainted(move |bounds, _, _| {
                if let Some(bounds) = bounds.first() {
                    measured.set(unpx(bounds.size.height));
                    let top = unpx(bounds.origin.y) - unpx(scroll.offset().y);
                    tops.borrow_mut().insert(id, top);
                }
            })
            .child(
                div()
                    .flex_none()
                    .child(self.render_part_content(ix, th, cx)),
            );
        match part_turn(reader.key, ix, part.turns) {
            None => content.into_any_element(),
            Some(id) => content
                .with_animation(
                    id,
                    Animation::new(katna_ui::motion::time(TURN)).with_easing(ease_out_cubic),
                    move |el, t| {
                        if t >= 1.0 {
                            el
                        } else {
                            el.h(px(from + (target.get() - from) * t))
                        }
                    },
                )
                .into_any_element(),
        }
    }

    /// Stars or unstars one message of the open conversation.
    /// The chips under the subject, in Gmail's order: Inbox, Important,
    /// the inbox tab, Starred, the other special folders, then the
    /// user's own folders and labels. Each says what its × takes off,
    /// where the service can.
    fn header_chips(&self, reader: &Conversation) -> Vec<HeaderChip> {
        let rows: Vec<&Row> = reader
            .parts
            .iter()
            .filter_map(|p| p.row.as_deref())
            .collect();
        let gmail = rows.first().is_some_and(|r| self.tree.is_gmail(r.account));
        let nodes: Vec<(FolderId, Role, String)> = reader
            .folders
            .iter()
            .filter_map(|f| {
                let node = self.tree.node(*f)?;
                let name = if node.role == Role::Other {
                    node.path.clone()
                } else {
                    node.label()
                };
                Some((*f, node.role, name))
            })
            .collect();
        let has = |role: Role| nodes.iter().any(|(_, r, _)| *r == role);
        let mut chips = Vec::new();
        if let Some((_, _, name)) = nodes.iter().find(|(_, r, _)| *r == Role::Inbox) {
            chips.push(HeaderChip {
                text: name.clone(),
                // Taking Gmail's Inbox label off archives.
                remove: gmail.then_some(ChipRemove::Act(Act::Archive)),
            });
        }
        if rows.iter().any(|r| r.important) {
            chips.push(HeaderChip {
                text: tr!("folder-important"),
                remove: Some(ChipRemove::Act(Act::Important(false))),
            });
        }
        if has(Role::Inbox)
            && let Some(category) = reader.category.filter(|c| *c != MailCategory::Primary)
        {
            chips.push(HeaderChip {
                text: super::rule_editor::tab_label(category),
                remove: None,
            });
        }
        if rows.iter().any(|r| r.flagged) {
            chips.push(HeaderChip {
                text: tr!("folder-starred"),
                remove: Some(ChipRemove::Act(Act::Star(false))),
            });
        }
        for role in [Role::Sent, Role::Drafts, Role::Junk, Role::Trash] {
            if let Some((_, _, name)) = nodes.iter().find(|(_, r, _)| *r == role) {
                chips.push(HeaderChip {
                    text: name.clone(),
                    remove: None,
                });
            }
        }
        let mut own: Vec<&(FolderId, Role, String)> =
            nodes.iter().filter(|(_, r, _)| *r == Role::Other).collect();
        own.sort_by_key(|(_, _, name)| name.to_lowercase());
        for (id, _, name) in own {
            chips.push(HeaderChip {
                text: name.clone(),
                // Only a Gmail label comes off; a folder holds the mail.
                remove: gmail.then(|| ChipRemove::Label(*id, name.clone())),
            });
        }
        // Mail the store doesn't place yet shows the list's folder.
        if chips.is_empty()
            && let Some(folder) = self.folder_name()
        {
            chips.push(HeaderChip {
                text: folder,
                remove: None,
            });
        }
        chips
    }

    /// One chip under the subject; its × shows on hover.
    fn render_header_chip(
        &self,
        ix: usize,
        chip: HeaderChip,
        key: EntryKey,
        th: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let group = SharedString::from(format!("header-chip-{ix}"));
        div()
            .id(("header-chip", ix))
            .group(group.clone())
            .relative()
            .px(px(6.0))
            .py(px(1.0))
            .rounded(px(radius::XS))
            .bg(rgba(th.chip))
            .text_size(px(12.0))
            .text_color(rgba(th.text_dim))
            .child(chip.text.clone())
            // The × keeps no room at rest, so every chip's text sits
            // evenly; on hover it covers the chip's end on a solid patch.
            .when_some(chip.remove, |d, remove| {
                d.child(
                    div()
                        .id(("header-chip-remove", ix))
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .right_0()
                        .rounded_r(px(radius::XS))
                        .bg(rgba(th.surface))
                        .opacity(0.0)
                        .group_hover(group, |s| s.opacity(1.0))
                        .child(
                            // The chip's own fill over the solid patch, so
                            // the end keeps its colour and corners.
                            div()
                                .size_full()
                                .flex()
                                .items_center()
                                .pr(px(space::S1))
                                .rounded_r(px(radius::XS))
                                .bg(rgba(th.chip))
                                .child(
                                    div()
                                        .size(px(16.0))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .rounded(px(radius::inner(4.0, space::S1)))
                                        .cursor_pointer()
                                        .hover(|s| s.bg(rgba(th.hover)))
                                        .child(icon("close", th.text_dim, 12.0)),
                                ),
                        )
                        .tooltip(tip(
                            tr!("reader-chip-remove", label = chip.text.as_str()),
                            th,
                        ))
                        .on_click(cx.listener(move |this, _, _, cx| match &remove {
                            ChipRemove::Act(act) => this.act(*act, vec![key], cx),
                            ChipRemove::Label(label, name) => {
                                this.toggle_label(vec![key], *label, name, false, cx)
                            }
                        })),
                )
            })
            .into_any_element()
    }

    fn star_message(&mut self, ix: usize, id: MessageId, on: bool, cx: &mut Context<Self>) {
        if let Some(part) = self.reader.as_mut().and_then(|r| r.parts.get_mut(ix))
            && let Some(row) = &part.row
        {
            let mut row = (**row).clone();
            row.flagged = on;
            part.row = Some(Rc::new(row));
        }
        if let Some(key) = self.reader.as_ref().map(|r| r.key) {
            let any = on
                || self
                    .reader
                    .iter()
                    .flat_map(|r| &r.parts)
                    .any(|p| p.row.as_ref().is_some_and(|r| r.flagged));
            self.pending.entry(key).or_default().flagged = Some(any);
        }
        let ids = match &self.mail {
            Ok(mail) => data::flag_changes(&mail.with_copies(&[id]), MessageFlags::FLAGGED, on),
            Err(_) => vec![id],
        };
        let command = Command::Star(ids.clone(), on);
        let done = command.done_text(1, false);
        let undo = Command::Star(ids, !on);
        self.send(command, done, Some(undo), false, cx);
        cx.notify();
    }

    /// Whether `email` is one of the user's own addresses.
    fn is_me(&self, email: &str) -> bool {
        self.accounts
            .iter()
            .any(|a| a.address.eq_ignore_ascii_case(email))
    }
}

/// The names of a message's recipients for its \u{201c}to\u{201d} line, each
/// with whether it is one of the user's addresses: \u{201c}me\u{201d}, and
/// others by first name unless `full` (or two share a first name).
/// Recipients without a name show their address.
fn recipient_names(people: &[(bool, &katna_render::Address)], full: bool) -> Vec<String> {
    fn first(a: &katna_render::Address) -> Option<&str> {
        a.name.as_deref().map(first_name)
    }
    people
        .iter()
        .map(|(me, a)| {
            if *me {
                return tr!("reader-me");
            }
            match first(a) {
                Some(short)
                    if !full
                        && people
                            .iter()
                            .filter(|(me, b)| !me && first(b) == Some(short))
                            .count()
                            == 1 =>
                {
                    short.to_owned()
                }
                _ => a.label().to_owned(),
            }
        })
        .collect()
}

/// The first name in a display name: \u{201c}Ada\u{201d} from \u{201c}Ada Lovelace\u{201d} or
/// \u{201c}Lovelace, Ada\u{201d}. A name it cannot split (one word, or a title
/// such as \u{201c}Dr.\u{201d} first) stays whole.
fn first_name(name: &str) -> &str {
    let name = name.trim().trim_matches(['"', '\'']).trim();
    let given = match name.split_once(',') {
        Some((_, given)) => given.trim(),
        None => name,
    };
    match given.split_whitespace().next() {
        Some(word) if !word.ends_with('.') && word.chars().count() > 1 => word,
        _ => name,
    }
}

/// "4 older messages": a line with a round count that unfolds them.
fn fold(count: usize, th: &Theme, cx: &mut Context<MailWindow>) -> AnyElement {
    div()
        .id("fold")
        .relative()
        .h(px(28.0))
        .flex()
        .items_center()
        .cursor_pointer()
        .on_click(cx.listener(|this, _, _, cx| {
            if let Some(reader) = &mut this.reader {
                reader.show_all = true;
            }
            cx.notify();
        }))
        .child(
            div()
                .absolute()
                .left_0()
                .right_0()
                .top(px(10.0))
                .h(px(1.0))
                .bg(rgba(th.divider)),
        )
        .child(
            div()
                .absolute()
                .left_0()
                .right_0()
                .top(px(14.0))
                .h(px(1.0))
                .bg(rgba(th.divider)),
        )
        .child(
            div()
                .ml(px(28.0))
                .size(px(28.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .border_1()
                .border_color(rgba(fade(th.text_faint, 0.6)))
                .bg(rgba(th.surface))
                .text_size(px(12.0))
                .text_color(rgba(th.text_dim))
                .hover(|s| s.bg(rgba(th.hover)))
                .child(format::thousands(count as u64)),
        )
        .into_any_element()
}

/// How long a message takes to open or fold.
const TURN: Duration = Duration::from_millis(260);

/// The animation ID of a message's latest opening or folding; `None` until
/// it has been opened or folded.
fn part_turn(key: EntryKey, ix: usize, turns: u32) -> Option<SharedString> {
    (turns > 0).then(|| format!("part-turn-{}-{ix}-{turns}", key_number(key)).into())
}

/// Fades a message's text in after it opens or folds.
fn turn_fade(el: gpui::Div, turn: Option<SharedString>) -> AnyElement {
    match turn {
        None => el.into_any_element(),
        Some(id) => el
            .with_animation(
                SharedString::from(format!("{id}-text")),
                Animation::new(katna_ui::motion::time(TURN)).with_easing(ease_out_cubic),
                |el, t| el.opacity(0.3 + 0.7 * t),
            )
            .into_any_element(),
    }
}

/// Gentler than quint: the height change stays visible for most of
/// [`TURN`].
fn ease_out_cubic(t: f32) -> f32 {
    1.0 - (1.0 - t).powi(3)
}

fn key_number(key: EntryKey) -> usize {
    match key {
        EntryKey::Message(id) => id.0 as usize,
        // Apart from message numbers, so the two never share an element ID.
        EntryKey::Thread(thread) => (thread.0 as usize) | (1 << (usize::BITS - 1)),
    }
}

/// Loads message `id` for reading.
fn read(mail: &Mail, id: MessageId) -> Body {
    let Some(raw) = mail.raw(id) else {
        return Body {
            view: None,
            blocks: Vec::new(),
            cut: false,
            doc: None,
            remote: Vec::new(),
            svgs: Vec::new(),
            authenticated: false,
            security: None,
            sealed: None,
            opened: None,
            invite: None,
            calls: Vec::new(),
        };
    };
    match katna_crypto::protection(&raw) {
        Some(protection) => security::sealed(raw, protection),
        None => shown(&raw, None),
    }
}

/// The body of a message just sent, as written.
fn shown_body(raw: &[u8]) -> Body {
    shown(raw, None)
}

/// The body of `raw` as the reading view shows it.
fn shown(raw: &[u8], security: Option<Secured>) -> Body {
    let view = katna_render::message_view(raw);
    // Mail only partly encrypted shows as plain text: text around the
    // opened part could be anyone's, and as HTML it could wrap the opened
    // text into a link to their site (EFAIL).
    let partly_encrypted = matches!(
        &security,
        Some(Secured::Opened(security)) if security.encrypted() && !security.whole
    );
    let doc = katna_render::message_document(raw).filter(|_| !partly_encrypted);
    let (blocks, cut) = match doc {
        Some(_) => (Vec::new(), false),
        None => body_blocks(&view.body, MAX_BODY_LINES),
    };
    let remote = doc.as_ref().map(rich::remote_urls).unwrap_or_default();
    let svgs = doc.as_ref().map(rich::carried_svgs).unwrap_or_default();
    let authenticated = !remote.is_empty() && katna_render::sender_authenticated(raw);
    let invite = invite::invite(raw);
    // An invitation's card has its own Join.
    let calls = if invite.is_some() {
        Vec::new()
    } else {
        let links = doc.as_ref().map(rich::links).unwrap_or_default();
        katna_core::meeting::find(links, [view.body.as_str()], None)
    };
    Body {
        view: Some(view),
        blocks,
        cut,
        doc,
        remote,
        svgs,
        authenticated,
        security,
        sealed: None,
        opened: None,
        invite,
        calls,
    }
}

/// Splits a body into runs of quoted (`>`) and unquoted lines, at most
/// `max_lines` lines in all. Returns whether lines were left out.
pub(super) fn body_blocks(body: &str, max_lines: usize) -> (Vec<(bool, SharedString)>, bool) {
    let mut blocks: Vec<(bool, String)> = Vec::new();
    let mut lines = body.lines();
    for line in lines.by_ref().take(max_lines) {
        let line = line.trim_end();
        let quoted = line.starts_with('>');
        match blocks.last_mut() {
            Some((q, text)) if *q == quoted => {
                text.push('\n');
                text.push_str(line);
            }
            _ => blocks.push((quoted, line.to_owned())),
        }
    }
    let cut = lines.next().is_some();
    (
        blocks
            .into_iter()
            .map(|(quoted, text)| (quoted, text.into()))
            .collect(),
        cut,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_names() {
        assert_eq!(first_name("Ada Lovelace"), "Ada");
        assert_eq!(first_name("Lovelace, Ada"), "Ada");
        assert_eq!(first_name("\"Ada Lovelace\""), "Ada");
        assert_eq!(first_name("Dr. Ada Lovelace"), "Dr. Ada Lovelace");
        assert_eq!(first_name("Ada"), "Ada");
        assert_eq!(first_name("J Smith"), "J Smith");
    }

    #[test]
    fn recipients_by_first_name_unless_full_or_shared() {
        let address = |name: Option<&str>, email: &str| katna_render::Address {
            name: name.map(str::to_owned),
            email: email.to_owned(),
        };
        let me = address(Some("Sam Roy"), "sam@example.org");
        let ada = address(Some("Ada Lovelace"), "ada@example.org");
        let ada_b = address(Some("Ada Byron"), "byron@example.org");
        let bare = address(None, "ops@example.org");
        let people = [(true, &me), (false, &ada), (false, &bare)];
        assert_eq!(
            recipient_names(&people, false),
            ["me", "Ada", "ops@example.org"]
        );
        assert_eq!(
            recipient_names(&people, true),
            ["me", "Ada Lovelace", "ops@example.org"]
        );
        let twins = [(false, &ada), (false, &ada_b)];
        assert_eq!(
            recipient_names(&twins, false),
            ["Ada Lovelace", "Ada Byron"]
        );
    }

    #[test]
    fn quoted_blocks() {
        let body = "Hi,\n\nsee below.\n> old line 1\n>> older\nthanks\r\n";
        let (blocks, cut) = body_blocks(body, 100);
        let blocks: Vec<(bool, &str)> = blocks.iter().map(|(q, t)| (*q, t.as_ref())).collect();
        assert_eq!(
            blocks,
            [
                (false, "Hi,\n\nsee below."),
                (true, "> old line 1\n>> older"),
                (false, "thanks"),
            ]
        );
        assert!(!cut);
        let (blocks, cut) = body_blocks(body, 2);
        assert_eq!(blocks.len(), 1);
        assert!(cut);
    }

    #[test]
    fn toolbar_gives_way_least_used_first_and_keeps_the_arrows() {
        let shown = Toolbar {
            back: true,
            separators: true,
            contact: true,
            colors: true,
            summary: true,
            new_window: true,
            position: Some(80.0),
            arrows: true,
        };
        let wide = Squeeze::fit(2000.0, &shown, Squeeze::NONE);
        assert!(!wide.new_window && !wide.position && !wide.delete);
        let mut last = 0;
        for width in (150..900).rev().step_by(10) {
            let squeeze = Squeeze::fit(width as f32, &shown, Squeeze::NONE);
            let dropped = [
                squeeze.new_window,
                squeeze.print,
                squeeze.colors,
                squeeze.contact,
                squeeze.move_to,
                squeeze.unread,
                squeeze.spam,
                squeeze.separators,
                squeeze.delete,
                squeeze.position,
            ];
            let n = dropped.iter().filter(|d| **d).count();
            // One by one, in order, and never back as it narrows.
            assert!(dropped.iter().take(n).all(|d| *d));
            assert!(n >= last);
            last = n;
            // What stays fits, down to Back, Archive, More and the arrows.
            if n < dropped.len() {
                assert!(shown.width(&squeeze) <= width as f32);
            }
        }
        assert_eq!(last, 10);
    }
}
