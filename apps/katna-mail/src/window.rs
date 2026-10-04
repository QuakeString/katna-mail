// SPDX-License-Identifier: GPL-3.0-or-later

//! The main window (`docs/ARCHITECTURE.md` §13.6): a top bar with the
//! search box in the middle, the navigation with Compose and the folders,
//! the list card and the open conversation, beside the list (three panes,
//! the default) or in its place (two panes). Motion comes from springs
//! (`katna_ui::motion`) and click ripples; all of it honors the desktop's
//! reduce-motion setting.
//!
//! The parts live in submodules: `nav` (top bar and navigation), `list`
//! (toolbar, tabs and rows), `reader` (the open conversation), `settings`
//! (quick settings), `search_panel` (search options), `compose`, `apps`
//! (the app rail), `add_account` (adding an account), `context_menu`
//! (the list's right-click menu), `onboarding` (the first start), `tour`
//! (a walk through the window), `whats_new` (after an update) and `layout` (phone, tablet and desktop
//! layouts, by the window's width).

mod about;
mod account_color;
mod account_roll;
mod account_status;
mod account_view;
mod accounts;
mod activity;
mod add_account;
mod agenda;
mod app_menu;
mod apps;
mod attachments;
mod calendar;
pub mod capture;
mod colors;
mod compose;
mod contact;
mod contacts_csv;
mod contacts_edit;
mod contacts_io;
mod contacts_labels;
mod contacts_merge;
mod contacts_other;
mod contacts_page;
mod contacts_share;
mod context_menu;
mod crash_notice;
mod dark;
mod delete_ask;
mod desktop;
mod detached;
mod download;
mod event_edit;
mod feedback_page;
mod files_page;
mod folder_pick;
mod frost_sliders;
mod gallery;
mod katna_account;
mod keymap;
mod labels;
mod language;
mod layout;
mod lines;
mod list;
mod look;
mod mail_drag;
mod mail_providers;
mod meeting;
mod nav;
mod nav_menu;
mod notched;
mod notes;
mod offline;
mod onboarding;
mod popovers;
mod print;
mod print_preview;
mod quiet;
mod reader;
mod remote;
mod reply_row;
mod rich;
mod row_reorder;
mod rule_editor;
mod scale_slider;
mod scheme_color;
mod scheme_editor;
mod scheme_picker;
mod search_panel;
mod select;
mod settings;
mod settings_page;
mod settings_search;
mod share_ask;
mod sheet;
mod shortcuts_dialog;
mod sign_in_again;
mod skeleton;
mod snooze;
mod sounds;
mod storage;
mod tasks_page;
mod tour;
mod translate;
mod unified;
mod updates;
mod view_state;
mod viewer;
mod whats_new;

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use futures_lite::StreamExt;
use gpui::{
    AnyElement, App, Context, Entity, FocusHandle, Focusable, FontWeight, Hsla, ListOffset,
    MouseButton, MouseMoveEvent, Render, ScrollHandle, SharedString, Subscription, Task, TextRun,
    WeakEntity, Window, actions, div, prelude::*, rgba,
};
use jiff::tz::TimeZone;
use katna_chrome::{Bar, ChromeColors, Environment, WindowChrome};
use katna_core::config::{ReadingPane, Theme as ThemeChoice};
use katna_core::{Account, AccountId, Config, Paths};
use katna_dbus::zbus::Connection;
use katna_search::SearchResults;
use katna_store::{FolderId, MessageFlags, MessageId};
use katna_ui::motion::{self, Spring, lerp};
use katna_ui::px;
use katna_ui::unpx;
use katna_ui::{InputEvent, TextInput};

use crate::daemon::{self, Command};
use crate::data::{self, Entry, EntryKey, Mail, OpenError};
use crate::sidebar::{self, Role, Tree};
use crate::tabs::{self, Provider, Tab};
use crate::theme::{Accent, Theme};
use crate::widgets::{elevation, icon, tip};

use apps::{App as RailApp, People};
use reader::Conversation;
use search_panel::SearchPanel;

pub use desktop::{MenuBar, menu_bar, refresh_menu_bar};
pub use look::look;
pub(crate) use popovers::MenuKey;

actions!(
    katna_mail,
    [
        SelectNext,
        SelectPrevious,
        FocusNext,
        FocusPrevious,
        NextPane,
        PreviousPane,
        SendMail,
        RephraseSelection,
        Summarize,
        OpenContextMenu,
        SelectFirst,
        SelectLast,
        ListTop,
        PageDown,
        PageUp,
        OpenMessage,
        CloseMessage,
        ScrollDown,
        ScrollUp,
        ScrollPageDown,
        ScrollPageUp,
        FocusSearch,
        FocusList,
        ToggleNavigation,
        Compose,
        Reload,
        Quit,
        Archive,
        Delete,
        ReportSpam,
        MarkRead,
        MarkUnread,
        ToggleStar,
        AddToTasks,
        MarkImportant,
        ToggleMute,
        MarkNotImportant,
        ToggleCheck,
        ToggleSettings,
        Reply,
        ReplyAll,
        Forward,
        MoveTo,
        SelectAll,
        SelectNone,
        Undo,
        GoToInbox,
        GoToStarred,
        GoToSent,
        GoToDrafts,
        GoToAllMail,
        ShowMail,
        ShowCalendar,
        ShowContacts,
        ShowTasks,
        ShowNotes,
        ShowFiles,
        OpenSettings,
        ShowShortcuts,
        ShowWhatsNew,
        CheckForUpdates,
        ShowAbout,
    ]
);

const WINDOW_CONTEXT: &str = "MailWindow";
const LIST_CONTEXT: &str = "MessageList";
const READER_CONTEXT: &str = "MessageReader";
/// The folder pane while it has the keys.
const NAV_CONTEXT: &str = "Navigation";
const SEARCH_CONTEXT: &str = "SearchBox";

pub(super) const TOP_BAR_HEIGHT: f32 = 64.0;
const NAV_WIDTH: f32 = 256.0;
/// How far the folder highlight pill (and the drawer's) stays off the
/// pane's left edge.
const NAV_ROW_INSET: f32 = 8.0;
/// The share of its shadow and of its edge a card keeps while another
/// pane has the keys.
const SHADOW_REST: f32 = crate::widgets::CARD_REST;
const EDGE_REST: f32 = 0.55;
/// The one gap between the top bar's elements: the menu button and the
/// app's name, the name and the search box (when the window is too narrow
/// for the box's usual place), the search box and Settings, Settings and
/// the account picture. The header bar itself spaces its items 6 px apart.
const TOP_BAR_GAP: f32 = 16.0;
const BAR_ITEM_GAP: f32 = 6.0;
/// The room Settings and the account picture take at the top bar's end, up
/// to the window buttons: 40 px wide each, with the gap between them, and
/// 8 px after the picture plus the bar's own spacing. The language button
/// is in the account menu.
const TOP_END_WIDTH: f32 = 40.0 + TOP_BAR_GAP + 40.0 + 8.0 + BAR_ITEM_GAP;
/// The size of the word on the Compose button.
const COMPOSE_TEXT_SIZE: f32 = 14.0;
/// Compose is as tall as a phone's: a 56 px square in the rail, a pill in
/// the folder pane.
const COMPOSE_HEIGHT: f32 = 56.0;
const COMPOSE_RADIUS: f32 = 16.0;
/// Where Compose sits in the rail: centred, 8 px from the top.
const COMPOSE_RAIL_LEFT: f32 = (apps::APP_RAIL_WIDTH - COMPOSE_HEIGHT) / 2.0;
const COMPOSE_TOP: f32 = 8.0;
/// The room Compose takes above the folders, with 16 px under it.
const COMPOSE_NAV_ROOM: f32 = COMPOSE_TOP + COMPOSE_HEIGHT + 16.0;
/// Where the app's name starts on the top bar: the bar's 6 px padding,
/// the menu button (48 px with a 6 px margin) and the gap after it.
const TITLE_LEFT: f32 = 6.0 + 6.0 + 48.0 + TOP_BAR_GAP;
/// The Katna mark beside the app's name, the gap after it and the size of
/// the name.
const TITLE_MARK: f32 = 32.0;
const TITLE_MARK_GAP: f32 = 10.0;
const TITLE_TEXT_SIZE: f32 = 22.0;

/// Width of Compose: a square in the rail (`label` 0), the pencil and the
/// word (`text` px wide) in the folder pane (`label` 1).
fn compose_width(label: f32, text: f32) -> f32 {
    lerp(COMPOSE_HEIGHT, 16.0 + 24.0 + 12.0 + text + 24.0, label)
}

/// How wide `text` is drawn in the window's font, so what holds it fits
/// whatever font and size the desktop uses.
fn text_width(
    text: &str,
    size: f32,
    weight: FontWeight,
    font: Option<&SharedString>,
    window: &Window,
) -> f32 {
    let mut style = window.text_style().font();
    if let Some(family) = font {
        style.family = family.clone();
    }
    style.weight = weight;
    let run = TextRun {
        len: text.len(),
        font: style,
        color: Hsla::default(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let line = window
        .text_system()
        .shape_line(text.to_owned().into(), px(size), &[run], None);
    unpx(line.width).ceil() + 1.0
}

/// How wide `label` is drawn on the big button at the top of the left bar.
fn compose_text_width(label: &str, font: Option<&SharedString>, window: &Window) -> f32 {
    text_width(label, COMPOSE_TEXT_SIZE, FontWeight::MEDIUM, font, window)
}

/// The widths of "Katna" and of the longest app name after it, so the
/// search box keeps its place whichever app is on show.
fn title_widths(font: Option<&SharedString>, window: &Window) -> (f32, f32) {
    let width = |text: &str| text_width(text, TITLE_TEXT_SIZE, FontWeight::NORMAL, font, window);
    let brand = width(&katna_i18n::tr!("top-brand"));
    let name = RailApp::ALL
        .into_iter()
        .map(|app| width(&app.label()))
        .fold(0.0, f32::max);
    (brand, name)
}

/// Width of the mark and the name on the top bar; the words fold away
/// as `label` goes to 0.
fn title_width(label: f32, (brand, name): (f32, f32)) -> f32 {
    TITLE_MARK + (TITLE_MARK_GAP + brand + TITLE_WORD_GAP + name) * label
}

/// The space between "Katna" and the app's name.
const TITLE_WORD_GAP: f32 = 6.0;
/// Corners of cards that float: menus aside, dialogs and panels.
const PANEL_RADIUS: f32 = katna_ui::tokens::radius::LG;
const SEARCH_WIDTH: f32 = 720.0;
/// The narrowest the search box gets beside the top bar's buttons.
const SEARCH_MIN_WIDTH: f32 = 120.0;
/// The Activity button beside the search box.
const ACTIVITY_BUTTON_WIDTH: f32 = 40.0;
/// Quick settings panel, with its right margin.
const SETTINGS_WIDTH: f32 = 336.0;
/// The space between cards side by side (the list, the reading pane,
/// Quick settings) and between the cards and the window's edges: one
/// value, so every gap is the same.
const CARD_GAP: f32 = 16.0;
/// Space between the list and the reading pane; also the handle to drag.
const SPLIT_GAP: f32 = CARD_GAP;
/// Narrower lists show each line as three (sender, subject, snippet).
const STACKED_BELOW: f32 = 680.0;
const PAGE: usize = 10;
/// Wait this long after a keystroke before searching, so fast typing
/// searches once.
const SEARCH_DELAY: Duration = Duration::from_millis(60);
/// Rest on Mail in the app rail this long before the folded navigation
/// opens over the list.
const PEEK_DELAY: Duration = Duration::from_millis(300);
/// The opened navigation waits this long after the pointer leaves, so it can
/// cross from the rail to the panel.
const PEEK_LINGER: Duration = Duration::from_millis(250);
const SNACKBAR_TIME: Duration = Duration::from_secs(5);
/// Changes signalled by the daemon within this time are read together.
const CHANGE_DELAY: Duration = Duration::from_millis(120);
/// How often the store is tried again while the daemon updates it.
const MIGRATION_RETRY: Duration = Duration::from_millis(250);
/// How long the window looks as if loading while the daemon updates the
/// store, before it says the store could not be opened.
const MIGRATION_PATIENCE: Duration = Duration::from_secs(60);
/// At least this long between reloads for the daemon's changes. While it
/// downloads mail it signals every few hundred milliseconds, and each
/// reload of a big folder holds the window up for a moment; one reload
/// per burst keeps scrolling and typing smooth meanwhile.
const CHANGE_GAP: Duration = Duration::from_secs(1);
const LINE_SCROLL: f32 = 48.0;
/// How long a send failure stays on screen.
const FAILURE_TIME: Duration = Duration::from_secs(12);

/// What the message list shows.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Listing {
    Folder(FolderId),
    /// A list of the unified inbox, of every account or of `account`.
    Unified {
        view: sidebar::Unified,
        account: Option<AccountId>,
    },
    Search {
        query: String,
        total: Option<usize>,
        /// What was searched instead, when the query had a word that is not
        /// in the mail ("Showing results for …").
        corrected: Option<String>,
    },
}

/// What the list showed when a search started: back when the search is
/// cancelled without a result opened.
#[derive(Debug, Clone)]
struct BeforeSearch {
    listing: Listing,
    /// The open conversation.
    open: Option<EntryKey>,
    /// The line the cursor was on.
    selected: Option<EntryKey>,
    top: ListOffset,
}

/// An open popup menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Menu {
    /// Next to the list's checkbox: select all, none, read, ...
    Select,
    /// The list's "more" button.
    ListMore,
    /// "Move to" with the folders of the account.
    MoveTo,
    /// Gmail's "Label as" with the account's labels.
    LabelAs,
    /// The open conversation's "more" button.
    ReaderMore,
    /// The calendar bar's options: density, second time zone.
    CalendarOptions,
    /// The second time zones to choose from.
    CalendarZones,
    /// The views, when the bar is too narrow for their buttons.
    CalendarViews,
    /// The list toolbar's bell: how long to mute the open folder or tab.
    Quiet,
}

/// A change the user asks for on some lines of the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Act {
    Archive,
    Delete,
    Spam,
    MoveTo(FolderId),
    Read(bool),
    Star(bool),
    Important(bool),
    Pin(bool),
    /// Until then (Unix seconds).
    Snooze(i64),
    Unsnooze,
}

/// What the pointer rests on that opens the folded navigation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Hover {
    /// Compose in the app rail.
    Compose,
    /// Mail in the app rail.
    Mail,
    Panel,
}

/// One step Ctrl+Z takes back.
#[derive(Debug, Clone, PartialEq, Eq)]
enum UndoStep {
    /// Undone by sending this.
    Command(Command),
    /// Deleted forever: Ctrl+Z says it cannot be undone.
    DeletedForever,
}

/// How many steps Ctrl+Z remembers.
const UNDO_STEPS: usize = 50;

/// A short note at the bottom of the window, maybe with an Undo button.
struct Snackbar {
    text: SharedString,
    undo: Option<Command>,
    /// Counting down to this moment from this long before, in a ring
    /// with the seconds inside; Undo goes when it is reached.
    countdown: Option<(Instant, Duration)>,
    shown: Spring,
    _hide: Task<()>,
}

/// Changes sent to the daemon but not read back from the store yet, so
/// the list shows them at once.
#[derive(Debug, Default, Clone, Copy)]
struct Pending {
    unread: Option<bool>,
    flagged: Option<bool>,
    important: Option<bool>,
    pinned: Option<bool>,
}

pub struct MailWindow {
    chrome: WindowChrome,
    /// The app of the rail on show.
    app: RailApp,
    /// The Calendar page.
    calendar: calendar::CalendarPage,
    /// The day's agenda beside the mail.
    agenda: agenda::AgendaPanel,
    /// Undo steps that bring back the conversation that was open, with
    /// its key, so undoing opens it again.
    undo_reopens: Vec<(Command, EntryKey)>,
    /// The conversation to open once an undo brings it back to the list.
    reopen_after_undo: Option<EntryKey>,
    before_search: Option<BeforeSearch>,
    /// The search box's X was clicked: the conversation opened from the
    /// results stays open as the folder comes back.
    clear_keeps_open: bool,
    /// A press landed on the search box or its options panel, so the
    /// window's own press handler leaves the box focused.
    search_pressed: bool,
    /// The app whose name rolls away at the top left, and how far the
    /// new name has rolled in (0 to 1).
    title_from: RailApp,
    title_roll: Spring,
    /// The big button's icon and word now and the ones it turns away from
    /// as the page (or a drive's upload) changes them, and how far it has
    /// turned (0 to 1).
    primary_icon: &'static str,
    primary_icon_from: &'static str,
    primary_label: String,
    primary_label_from: String,
    primary_icon_turn: Spring,
    /// How wide the big button's word is drawn this frame: eased from the
    /// old word's width to the new one's while it turns.
    primary_label_width: f32,
    /// The account picture at the top right rolling from the last
    /// account's, and how far the new one has rolled in (0 to 1).
    avatar_roll: account_roll::AvatarRoll,
    avatar_turn: Spring,
    /// 0 = hidden, 1 = shown: Compose, in the folder pane or the rail.
    compose_shown: Spring,
    /// 0 = Compose is in the rail, 1 = over the folders beside the list.
    compose_dock: Spring,
    people: Option<People>,
    /// The Notes page, once opened.
    notes: Option<notes::NotesPage>,
    people_task: Option<Task<()>>,
    /// The Contacts page: saved contacts and their pictures.
    contacts: contacts_page::ContactsPage,
    /// The Tasks page.
    tasks: tasks_page::TasksPage,
    /// The Files page: every attachment in one place.
    library: files_page::Library,
    /// The attach picker over the chat (paperclip > From Files).
    picker: Option<files_page::picker::Picker>,
    /// The desktop's UI font, or `None` to leave GPUI's default.
    font: Option<SharedString>,
    /// How far text in a pill goes up to look centred in it, per pixel
    /// of font size: see [`MailWindow::measure_pill_text`].
    pill_text_lift: f32,
    paths: Paths,
    config: Config,
    config_path: PathBuf,
    mail: Result<Mail, OpenError>,
    accounts: Vec<Account>,
    /// How full each account's mail storage is, when its server says.
    quotas: HashMap<AccountId, katna_store::StorageQuota>,
    /// Changes and mail waiting to go to each account's servers.
    waiting: HashMap<AccountId, u64>,
    /// Redraws when an account taken offline for a while comes back.
    offline_end: Option<Task<()>>,
    /// The account the storage line last showed, kept while the list
    /// shows no one account's folder.
    storage_account: std::cell::Cell<Option<AccountId>>,
    tree: Tree,
    /// Unread mail per folder, counted in the background.
    unread: HashMap<FolderId, u64>,
    /// What notifies and counts: folder bells and mutes.
    alerts: data::Alerts,
    unread_task: Option<Task<()>>,
    expanded: HashSet<String>,
    /// Accounts folded or opened in the folder pane by their arrow; the
    /// others are open, or folded under the unified inbox.
    open_accounts: HashMap<AccountId, bool>,
    /// Whether the unified inbox's lists show under "All Accounts".
    all_accounts_open: bool,
    nav_rows: Vec<sidebar::Row>,
    folder: Option<FolderId>,
    /// The unified inbox's list last opened, as `folder` is the folder:
    /// what shows again when a search is cleared.
    unified: Option<(sidebar::Unified, Option<AccountId>)>,
    show_recipients: bool,
    listing: Option<Listing>,
    /// The inbox tabs of the listed folder's account; none outside inboxes.
    tabs: Vec<Tab>,
    /// The open inbox tab, an index into `tabs`.
    tab: usize,
    category_unread: HashMap<katna_core::MailCategory, u64>,
    entries: Vec<Entry>,
    /// The list cursor.
    selected: Option<usize>,
    /// Ticked lines.
    checked: HashSet<EntryKey>,
    /// The line Shift+click ticks from: the last one ticked or unticked by
    /// a click.
    check_anchor: Option<usize>,
    /// Every line of the list is ticked, not only the ones on screen.
    checked_all: bool,
    /// How many lines "Select all" ticked on screen, while those are still
    /// the ticked ones. The banner offering the whole list follows this
    /// rather than the lines on screen, which the banner itself changes.
    page_pick: Option<usize>,
    /// What Read, Unread, Starred or Unstarred in the select menu ticked,
    /// while those are still the ticked ones: the banner says so and offers
    /// the rest.
    picked: Option<list::Picked>,
    pending: HashMap<EntryKey, Pending>,
    /// Rows of the list on screen at the last layout.
    visible: Range<usize>,
    /// The line under the pointer. It shows as hovered only while the list
    /// is not scrolling, as in Gmail.
    hovered: Option<usize>,
    /// The list is scrolling: lines passing under the pointer don't light
    /// up. Ends a moment after the last scroll.
    list_scrolling: Option<Task<()>>,
    reader: Option<Conversation>,
    /// A window of its own showing one conversation (double-click on a
    /// line), not the main mail window.
    detached: bool,
    /// For a conversation window, the mail window it came from: it shows
    /// the snackbar (and its Undo) when the conversation moves away and
    /// this window closes.
    main: Option<WeakEntity<Self>>,
    /// Remote images and sender pictures of the open conversation.
    remote: remote::Remote,
    /// The link under the pointer in the reading pane, whose address shows
    /// at its foot.
    hovered_link: Option<rich::HoveredLink>,
    /// Translations of opened messages (the Translate bar).
    translations: translate::Translations,
    /// The selected text of the open conversation.
    text: select::TextSelection,
    /// The selected text outside the conversation: dialogs, Settings,
    /// pages, errors.
    ui_text: select::TextSelection,
    /// The last text pressed was outside the conversation: `ui_text` has
    /// the selection.
    ui_active: bool,
    /// This window's own handle, for text made selectable where no
    /// `Context` is at hand (`select::selectable_in`).
    me: WeakEntity<Self>,
    /// Whether a conversation is open: in place of the list with two
    /// panes, beside it with three.
    reading: bool,
    /// Changes whenever the card switches content, to replay its fade-in.
    card_seq: usize,
    search: Entity<TextInput>,
    /// The mail search put aside while the box searches settings.
    mail_query: Option<String>,
    /// Counts the rows a settings search has lit up.
    flash_seq: usize,
    search_error: Option<SharedString>,
    /// Search this text as typed, not corrected ("Search instead for …").
    search_verbatim: Option<String>,
    search_task: Option<Task<()>>,
    search_panel: Option<SearchPanel>,
    search_panel_spring: Spring,
    menu: Option<Menu>,
    /// The popup menu open at the last frame, and the one fading out
    /// since it closed (`list::track_menu_fade`).
    menu_was: Option<Menu>,
    menu_fade: Option<(Menu, Instant)>,
    /// The right-click menu of the list.
    context_menu: Option<context_menu::ContextMenu>,
    /// Conversations summed up by AI, and the card beside a line.
    summaries: reader::Summaries,
    /// The right-click menu of the folder pane.
    nav_menu: Option<nav_menu::NavMenu>,
    /// The mute choices opened from a folder's or account's menu.
    quiet_menu: Option<quiet::QuietMenu>,
    /// Checks for new mail under way; the refresh arrow turns meanwhile.
    checking: Vec<nav_menu::Check>,
    check_seq: u64,
    /// The snooze menu, or its date and time picker.
    snooze_menu: Option<snooze::SnoozeMenu>,
    /// The navigation is open (not folded to the rail).
    nav_open: bool,
    /// The folded navigation is opened over the list while the pointer is
    /// on it or on Mail in the app rail.
    nav_peek: bool,
    /// The pointer is on Mail in the app rail, and on the panel.
    peek_hover: (bool, bool),
    /// What in the rail opened the navigation last: its notch points there.
    peek_from: Hover,
    peek_task: Option<Task<()>>,
    /// 0 = folded, 1 = open: the drawn navigation.
    nav_spring: Spring,
    /// The pages other than Mail whose side column (calendars, lists,
    /// labels) is folded away on a desktop, and how far the one on show
    /// is open.
    page_side_spring: Spring,
    page_side_t: f32,
    /// 0 = folded, 1 = open: the space the navigation takes from the card.
    reserve_spring: Spring,
    search_spring: Spring,
    /// 0 = closed, 1 = open: the reading pane beside the list.
    pane_spring: Spring,
    /// How far the keys have moved to the conversation beside the list,
    /// from 0 (the list has them) to 1; the pane without them sinks back.
    keys_spring: Spring,
    keys_t: f32,
    /// Where the divider drag started: pointer x and the pane's share.
    split_drag: Option<(f32, f32)>,
    /// Width available to the list and the reading pane, at the last frame.
    cards_width: f32,
    /// Reply, Reply all and Forward at the foot of a conversation.
    reply_row: reply_row::ReplyRow,
    /// The contact panel beside the open conversation.
    contact: contact::ContactPanel,
    /// The same once the layout's motion settles, so the lines change
    /// shape once rather than midway through it.
    cards_target: f32,
    settings_open: bool,
    settings_spring: Spring,
    /// The reading-pane choice of the quick settings under the pointer.
    pane_hover: Option<ReadingPane>,
    /// How far the inbox tabs have folded to fit their room (see
    /// `tabs_fold_target`), gliding between steps.
    tab_fold: Spring,
    /// Each inbox tab's label and count badge widths, measured each frame.
    tab_sizes: Vec<(f32, f32)>,
    /// Each inbox tab's unread chip as it shows, fades and folds away.
    tab_chips: Vec<list::TabChip>,
    snackbar: Option<Snackbar>,
    /// What Ctrl+Z takes back, newest last: this window's actions since
    /// it opened.
    undo_history: Vec<UndoStep>,
    /// After a crash: the report to view or copy.
    crash_notice: Option<crash_notice::CrashNotice>,
    /// "Sign in again" for accounts whose OAuth2 sign-in stopped working.
    sign_in_again: sign_in_again::SignInAgain,
    /// Settings > User feedback's list of crash reports, as last read.
    saved_reports: Option<feedback_page::SavedReports>,
    /// Settings > Subscription (the Katna account), once shown.
    katna: Option<katna_account::KatnaPage>,
    compose: Option<compose::Compose>,
    /// Attachment thumbnails and the attachment viewer.
    files: attachments::Files,
    add_account: Option<add_account::AddAccount>,
    /// The first-start pages, until the first account is in and set up.
    onboarding: Option<onboarding::Onboarding>,
    /// The What's new dialog, after an update or from quick settings.
    whats_new: Option<whats_new::WhatsNew>,
    /// Help > Keyboard shortcuts, while open.
    shortcuts_dialog: Option<shortcuts_dialog::ShortcutsDialog>,
    /// "Help improve Katna", asked once after an update.
    share_ask: Option<share_ask::ShareAsk>,
    /// The print preview, before the desktop's print dialog.
    print_preview: Option<print_preview::PrintPreview>,
    /// Ask it once What's new is closed.
    share_ask_later: bool,
    /// The About Katna dialog.
    about: Option<about::About>,
    /// When the version's copy button was last clicked: it shows a check
    /// for a moment.
    version_copied: Option<std::time::Instant>,
    /// Every shared control, in development builds.
    gallery: Option<gallery::Gallery>,
    /// Updates of Katna, shown in About.
    updates: updates::Updates,
    /// Follows uploads to Google Drive, once one started.
    drive_watch: Option<Task<()>>,
    tour: Option<tour::Tour>,
    tour_marks: tour::Marks,
    /// Where the parts the tour shows were in the last frame.
    tour_seen: HashMap<tour::Spot, gpui::Bounds<gpui::Pixels>>,
    /// Marks the open conversation read once it has been open long
    /// enough (`mail.mark_read`); replaced when another one opens.
    read_timer: Option<Task<()>>,
    /// An account still waits for its first sync, so an empty folder may
    /// only be not fetched yet.
    first_sync: bool,
    /// POP3 choices sent to the daemon, shown until the store has them.
    pop3_keep: HashMap<AccountId, katna_core::Pop3Keep>,
    _first_sync_check: Option<Task<()>>,
    /// Since when the store has waited for the daemon to move it to this
    /// version's schema, and the next try.
    migrating: Option<(Instant, Task<()>)>,
    /// The account card above the rail's account picture.
    account_menu: bool,
    /// The application menu, open from the account card's ☰ button.
    app_menu: Option<app_menu::AppMenu>,
    /// The language picker, open from the top bar, the drawer or Settings.
    language_picker: Option<language::LanguagePicker>,
    /// The message last handed to the outbox, for Undo.
    unsent: Option<compose::Unsent>,
    /// Messages on their way out, and replies shown before the store has
    /// them.
    sending: compose::Sending,
    /// The spelling dictionary and scheduled mail of compose.
    writing: compose::Writing,
    /// The list of opens and clicks under the Activity button, when open.
    activity: Option<activity::Menu>,
    /// The Activity report, when open.
    activity_report: Option<activity::Report>,
    /// Opens and clicks not seen in Activity yet.
    activity_unseen: usize,
    /// Where the Activity button is.
    activity_button: std::rc::Rc<std::cell::Cell<Option<gpui::Bounds<gpui::Pixels>>>>,
    /// The Settings page, when open in place of the list.
    settings_page: Option<settings_page::SettingsPage>,
    /// The question before removing an account or deleting all data.
    danger: Option<accounts::Danger>,
    /// The question before deleting several lines, or deleting for good.
    delete_ask: Option<delete_ask::DeleteAsk>,
    /// Set while the answered question's delete runs, so it isn't asked
    /// again.
    delete_confirmed: bool,
    new_label: Option<labels::NewLabel>,
    /// The rule editor (Settings > Folders & rules, Make a rule…).
    rule_editor: Option<rule_editor::RuleEditor>,
    /// The search over the folders in Move to or Label as.
    folder_pick: Option<folder_pick::FolderPick>,
    /// The lines a drag onto a folder carries, while it is under way.
    mail_dragging: Vec<EntryKey>,
    /// Bodies being downloaded because their message or an attachment
    /// chip of it was opened.
    downloads: HashMap<MessageId, download::Download>,
    /// The attachment chip waiting for its message to download.
    chip_download: Option<download::ChipDownload>,
    /// Navigation openness at this frame, for the folder rows.
    nav_t: f32,
    daemon: Option<Connection>,
    _listen: Option<Task<()>>,
    _watch_sending: Option<Task<()>>,
    desktop_colors: colors::DesktopColors,
    /// Phone, tablet or desktop, by the window's width.
    layout: layout::Layout,
    list_focus: FocusHandle,
    /// The conversation beside the list (three panes): Up and Down scroll
    /// it while it has the keys, and move in the list while the list has.
    reader_focus: FocusHandle,
    /// Whether the conversation beside the list has the keys, as of this
    /// frame: the list's cursor dims and the pane's outline lights.
    reader_keys: bool,
    /// A dialog without fields of its own to focus (the delete question),
    /// and any dialog's frame that keeps Tab inside it.
    dialog_focus: FocusHandle,
    /// Settings > Appearance > Colors' editor, while open.
    scheme_editor: Option<scheme_editor::SchemeEditor>,
    /// The open color picker, for the scheme editor or the accent.
    color_picker: Option<scheme_color::ColorPicker>,
    /// Where the swatches that open the picker were drawn, for its place.
    color_swatches: scheme_color::Swatches,
    /// Colors picked lately, newest first.
    recent_colors: Vec<u32>,
    /// The folder pane while it has the keys, the line they are on, and
    /// whether it had them when this frame was drawn.
    nav_focus: FocusHandle,
    nav_cursor: Option<usize>,
    nav_keys_shown: bool,
    /// The keys, not the pointer, last moved in the folder pane: its line
    /// shows a ring.
    nav_by_keys: bool,
    /// The whole window: where the menu bar's actions start when the
    /// keyboard focus is on something no longer drawn.
    window_focus: FocusHandle,
    /// The message list: lines differ in height (attachment chips).
    list_state: lines::Lines,
    /// The line whose "+N" attachments button has its list open.
    files_menu: Option<EntryKey>,
    /// Layout and line height the list's lines were measured for.
    list_shape: (bool, u32),
    /// The line just opened, kept where it was in the list while the
    /// reading pane opens beside it and the lines change shape.
    keep_line: Option<list::KeepLine>,
    /// How tall the header pinned over the open mail or chat was last
    /// drawn, so what scrolls under it starts below it.
    reader_head: std::rc::Rc<std::cell::Cell<f32>>,
    /// How tall the bar pinned over the list was last drawn.
    list_head: std::rc::Rc<std::cell::Cell<f32>>,
    nav_list: gpui::ListState,
    nav_items: Vec<nav::NavItem>,
    /// Bumped when the folder pane's lines change; `nav_synced` is what
    /// its list last showed.
    nav_rev: u64,
    nav_synced: u64,
    /// Lines of the folder pane sliding open or shut.
    nav_fold: Option<nav::Fold>,
    reader_scroll: ScrollHandle,
    /// The open mail's scroll bar, shown while it scrolls or is pointed at.
    reader_bar: katna_ui::ScrollBar,
    tz: TimeZone,
    _subscriptions: Vec<Subscription>,
}

impl MailWindow {
    /// The main window, showing what `shown` showed when the window last
    /// closed, if anything.
    pub fn new(
        env: Environment,
        paths: Paths,
        font: Option<SharedString>,
        shown: Option<katna_core::window::ViewState>,
        preloading: data::Preloading,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let started = std::time::Instant::now();
        let config_existed = paths.config_file().exists();
        let mut this = Self::build(env, paths, font, window, cx);
        keymap::bind(&this.config.shortcuts, cx);
        if let Ok(mail) = &mut this.mail {
            mail.use_preload(preloading.wait());
        }
        this.load_tree();
        if let Some(shown) = &shown {
            this.restore_view(shown, cx);
        }
        this.open_default_folder(cx);
        this.count_unread(cx);
        this.count_activity();
        this.wait_for_migration(cx);
        // Whether this computer is signed in to Katna, read once the first
        // frame is up (the daemon call runs in the background) so server
        // features know it by the time they are opened.
        cx.on_next_frame(window, |this, window, cx| this.katna_load(window, cx));
        this.listen(cx);
        this.watch_offline_ends(cx);
        colors::apply_motion(&this.config.mail, this.desktop_colors.motion, cx);
        this.watch_colors(cx);
        if let Some(err) = this.mail.as_ref().ok().and_then(Mail::index_error) {
            tracing::info!("{err}");
        }
        window.focus(&this.list_focus, cx);
        if this.needs_account() {
            this.onboarding = Some(onboarding::Onboarding::new());
        }
        this.welcome_or_whats_new(config_existed, window, cx);
        this.check_crashes(cx);
        if let Ok(mail) = &mut this.mail
            && let Some(list) = mail.started()
        {
            let paths = this.paths.clone();
            cx.background_executor()
                .spawn(async move { data::remember_first_list(&paths, &list) })
                .detach();
        }
        tracing::info!(elapsed = ?started.elapsed(), lines = this.entries.len(), "mail loaded");
        this
    }

    /// The window's state before any mail is listed.
    fn build(
        env: Environment,
        paths: Paths,
        font: Option<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search = cx.new(|cx| TextInput::new(katna_i18n::tr!("search-mail"), cx));
        let subscriptions = vec![cx.subscribe_in(&search, window, Self::on_search_event)];
        let config_path = paths.config_file();
        let config = Config::load(&config_path).unwrap_or_else(|err| {
            tracing::warn!("{err}; using the default settings");
            Config::default()
        });
        let desktop_colors = colors::DesktopColors::new(&env.desktop, paths.config_dir());
        let mut this = Self {
            chrome: WindowChrome::new(env, "Katna Mail", window, cx),
            app: RailApp::Mail,
            calendar: calendar::CalendarPage::new(config.calendar.custom_days(), cx),
            agenda: agenda::AgendaPanel::new(),
            undo_reopens: Vec::new(),
            reopen_after_undo: None,
            before_search: None,
            clear_keeps_open: false,
            search_pressed: false,
            title_from: RailApp::Mail,
            title_roll: Spring::new(motion::SLIDE, 1.0),
            primary_icon: "compose",
            primary_icon_from: "compose",
            primary_label: String::new(),
            primary_label_from: String::new(),
            primary_label_width: 0.0,
            primary_icon_turn: Spring::new(motion::SMOOTH, 1.0),
            avatar_roll: account_roll::AvatarRoll::new(),
            avatar_turn: Spring::new(motion::SLIDE, 1.0),
            compose_shown: Spring::new(motion::SMOOTH, 1.0),
            compose_dock: Spring::new(motion::SLIDE, 1.0),
            people: None,
            notes: None,
            people_task: None,
            contacts: Default::default(),
            tasks: Default::default(),
            library: Default::default(),
            picker: None,
            font,
            pill_text_lift: 0.0,
            mail: Mail::open(&paths),
            remote: remote::Remote::load(&paths),
            hovered_link: None,
            translations: translate::Translations::default(),
            text: select::TextSelection::new(cx),
            ui_text: select::TextSelection::new_windowed(cx),
            ui_active: false,
            me: cx.entity().downgrade(),
            accounts: Vec::new(),
            quotas: HashMap::new(),
            waiting: HashMap::new(),
            offline_end: None,
            storage_account: std::cell::Cell::new(None),
            paths,
            config,
            config_path,
            tree: Tree::default(),
            unread: HashMap::new(),
            alerts: data::Alerts::default(),
            unread_task: None,
            expanded: HashSet::new(),
            open_accounts: HashMap::new(),
            all_accounts_open: true,
            unified: None,
            nav_rows: Vec::new(),
            folder: None,
            show_recipients: false,
            listing: None,
            tabs: Vec::new(),
            tab: 0,
            category_unread: HashMap::new(),
            entries: Vec::new(),
            selected: None,
            checked: HashSet::new(),
            check_anchor: None,
            checked_all: false,
            page_pick: None,
            picked: None,
            pending: HashMap::new(),
            visible: 0..0,
            hovered: None,
            list_scrolling: None,
            reader: None,
            detached: false,
            main: None,
            reading: false,
            card_seq: 0,
            search,
            mail_query: None,
            flash_seq: 0,
            search_error: None,
            search_verbatim: None,
            search_task: None,
            search_panel: None,
            search_panel_spring: Spring::new(motion::SMOOTH, 0.0),
            menu: None,
            menu_was: None,
            menu_fade: None,
            context_menu: None,
            summaries: reader::Summaries::default(),
            nav_menu: None,
            quiet_menu: None,
            checking: Vec::new(),
            check_seq: 0,
            snooze_menu: None,
            nav_open: true,
            nav_peek: false,
            peek_hover: (false, false),
            peek_from: Hover::Mail,
            peek_task: None,
            nav_spring: Spring::new(motion::SLIDE, 1.0),
            page_side_spring: Spring::new(motion::SLIDE, 1.0),
            page_side_t: 1.0,
            reserve_spring: Spring::new(motion::SLIDE, 1.0),
            search_spring: Spring::new(motion::SMOOTH, 0.0),
            pane_spring: Spring::new(motion::SLIDE, 0.0),
            keys_spring: Spring::new(motion::SMOOTH, 0.0),
            keys_t: 0.0,
            split_drag: None,
            cards_width: 0.0,
            reply_row: reply_row::ReplyRow::new(),
            contact: contact::ContactPanel::new(),
            cards_target: 0.0,
            settings_open: false,
            pane_hover: None,
            settings_spring: Spring::new(motion::SLIDE, 0.0),
            tab_fold: Spring::new(motion::SMOOTH, 0.0),
            tab_sizes: Vec::new(),
            tab_chips: Vec::new(),
            snackbar: None,
            undo_history: Vec::new(),
            crash_notice: None,
            sign_in_again: sign_in_again::SignInAgain::default(),
            saved_reports: None,
            katna: None,
            compose: None,
            files: attachments::Files::default(),
            add_account: None,
            onboarding: None,
            whats_new: None,
            shortcuts_dialog: None,
            share_ask: None,
            print_preview: None,
            share_ask_later: false,
            about: None,
            version_copied: None,
            gallery: None,
            updates: updates::Updates::default(),
            drive_watch: None,
            tour: None,
            tour_marks: Default::default(),
            tour_seen: HashMap::new(),
            read_timer: None,
            first_sync: false,
            pop3_keep: HashMap::new(),
            _first_sync_check: None,
            migrating: None,
            account_menu: false,
            app_menu: None,
            language_picker: None,
            unsent: None,
            writing: compose::Writing::default(),
            activity: None,
            activity_report: None,
            activity_unseen: 0,
            activity_button: std::rc::Rc::default(),
            settings_page: None,
            danger: None,
            delete_ask: None,
            delete_confirmed: false,
            new_label: None,
            rule_editor: None,
            folder_pick: None,
            mail_dragging: Vec::new(),
            downloads: HashMap::new(),
            chip_download: None,
            nav_t: 1.0,
            daemon: None,
            _listen: None,
            _watch_sending: None,
            sending: compose::Sending::default(),
            desktop_colors,
            layout: layout::Layout::new(),
            list_focus: cx.focus_handle(),
            reader_focus: cx.focus_handle(),
            reader_keys: false,
            dialog_focus: cx.focus_handle(),
            scheme_editor: None,
            color_picker: None,
            color_swatches: Default::default(),
            recent_colors: Vec::new(),
            nav_focus: cx.focus_handle(),
            nav_cursor: None,
            nav_keys_shown: false,
            nav_by_keys: false,
            window_focus: cx.focus_handle(),
            list_state: lines::Lines::new(),
            files_menu: None,
            list_shape: (false, 0),
            keep_line: None,
            reader_head: Default::default(),
            list_head: Default::default(),
            nav_list: nav::nav_list(),
            nav_items: Vec::new(),
            nav_rev: 1,
            nav_synced: 0,
            nav_fold: None,
            reader_scroll: ScrollHandle::new(),
            reader_bar: katna_ui::ScrollBar::default(),
            tz: TimeZone::try_system().unwrap_or(TimeZone::UTC),
            _subscriptions: subscriptions,
        };
        this.remote.always = this.config.mail.remote_images;
        this.watch_escape(window, cx);
        let weak = cx.entity().downgrade();
        // The toolbar's "1–50 of N" follows the scrolling, and the wheel
        // stops the list gliding back to its top.
        this.list_state.state().set_scroll_handler(move |_, _, cx| {
            weak.update(cx, |this, cx| {
                this.layout.stop_glide();
                this.hold_hover_while_scrolling(cx);
                cx.notify();
            })
            .ok();
        });
        this
    }

    /// Searches `query` as typed, after a correction the user didn't want.
    fn search_verbatim(&mut self, query: String, cx: &mut Context<Self>) {
        self.search_verbatim = Some(query.clone());
        self.start_search(query, cx);
    }

    /// Puts `query` in the search box and searches.
    pub fn search_for(&mut self, query: String, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.search.focus_handle(cx), cx);
        self.search
            .update(cx, |search, cx| search.set_text(query, cx));
    }

    /// Opens the first line of the list, for screenshots and tests.
    pub fn open_first(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open(0, window, cx);
    }

    /// The colors: the desktop's light or dark, unless the settings pick
    /// one, in the color scheme and accent color the settings pick
    /// ([`Theme::pick`]).
    fn theme(&self, window: &Window) -> Theme {
        self.theme_for(&self.chrome, window)
    }

    /// [`Self::theme`] for a window framed by `chrome`.
    fn theme_for(&self, chrome: &WindowChrome, window: &Window) -> Theme {
        let choice = match self.config.mail.theme {
            ThemeChoice::System => None,
            ThemeChoice::Light => Some(false),
            ThemeChoice::Dark => Some(true),
        };
        let system = &self.desktop_colors.colors;
        let view = &self.config.mail;
        // A scheme with one side decides light or dark itself.
        let choice = Theme::forced_dark(view.colors(), system).or(choice);
        let dark = choice.unwrap_or_else(|| WindowChrome::desktop_dark(window));
        let scheme = Theme::picks_scheme(dark, view.colors(), system);
        let th = Theme::pick(dark, view.colors(), Accent::parse(&view.accent), system);
        // The window frame follows the same choices.
        chrome.set_dark(choice);
        chrome.set_colors(scheme.then_some(ChromeColors {
            window_bg: th.page,
            view_bg: th.surface,
            fg: th.text,
            accent: th.accent,
        }));
        chrome.set_backdrop(Some(th.page));
        // Menus and popovers are frosted on their own switch, whether the
        // window is blurred or not: Katna draws their blur itself.
        let th = if self.config.experimental.frosted_popups && katna_ui::frost::supported() {
            let (blur, tint) = self.frost_amount();
            th.frosted(blur * window.scale_factor(), tint)
        } else {
            th
        };
        if chrome.blurred() {
            let (pane, chat, search) = self.pane_frost();
            th.translucent().frosted_panes(pane, chat, search)
        } else {
            th
        }
    }

    /// No account yet: the store is not made, or has no account.
    fn needs_account(&self) -> bool {
        match &self.mail {
            Err(OpenError::NoStore { .. }) => true,
            Err(OpenError::Migrating(_) | OpenError::Other(_)) => false,
            Ok(_) => self.accounts.is_empty(),
        }
    }

    /// Asks the daemon whether an account waits for its first sync.
    fn check_first_sync(&mut self, cx: &mut Context<Self>) {
        let Some(connection) = self.daemon.clone() else {
            return;
        };
        self._first_sync_check = Some(cx.spawn(async move |this, cx| {
            let pending = daemon::first_sync_pending(&connection).await;
            this.update(cx, |this, cx| {
                let pending = pending.unwrap_or(false);
                if this.first_sync != pending {
                    this.first_sync = pending;
                    cx.notify();
                }
            })
            .ok();
        }));
    }

    /// Whether an open conversation sits beside the list: the three-pane
    /// setting, in a window wide enough for it.
    fn split(&self) -> bool {
        let shape = &self.layout.shape;
        self.config.mail.reading_pane == ReadingPane::Right && shape.size.splits(shape.width)
    }

    /// Whether a conversation is open beside the list.
    fn pane_open(&self) -> bool {
        self.split() && self.reading && self.reader.is_some()
    }

    fn save_config(&mut self) {
        if let Err(err) = self.config.save(&self.config_path) {
            tracing::warn!("saving settings: {err}");
        }
    }

    fn load_tree(&mut self) {
        let Ok(mail) = &self.mail else {
            return;
        };
        self.accounts = mail.accounts();
        self.quotas = mail.quotas();
        self.waiting = mail.waiting();
        self.config.mail.order_accounts(&mut self.accounts);
        self.tree = Tree::build(&self.accounts, &mail.folders(), &self.unread);
        self.tree.unified_out = self.unified_out();
        self.expanded = self.tree.initially_expanded();
        self.settle_account_colors();
        self.rebuild_nav();
    }

    /// Counts unread mail in the background, then shows the counts.
    fn count_unread(&mut self, cx: &mut Context<Self>) {
        if self.mail.is_err() {
            return;
        }
        let paths = self.paths.clone();
        self.unread_task = Some(cx.spawn(async move |this, cx| {
            // The folders are read there too, so the window never waits
            // for their counts.
            let (folders, unread, alerts) = cx
                .background_executor()
                .spawn(async move { data::folders_and_unread(&paths) })
                .await;
            this.update(cx, |this, cx| {
                this.unread = unread;
                this.alerts = alerts;
                if let Ok(mail) = &this.mail {
                    let folders = folders.unwrap_or_else(|| mail.folders());
                    this.tree = Tree::build(&this.accounts, &folders, &this.unread);
                    this.tree.unified_out = this.unified_out();
                    this.rebuild_nav();
                }
                cx.notify();
            })
            .ok();
        }));
    }

    /// Connects to the daemon and reloads whenever it says mail changed.
    fn listen(&mut self, cx: &mut Context<Self>) {
        self._listen = Some(cx.spawn(async move |this, cx| {
            let connection = match cx.background_executor().spawn(daemon::connect()).await {
                Ok(connection) => connection,
                Err(err) => {
                    tracing::info!("{err}");
                    return;
                }
            };
            this.update(cx, |this, cx| {
                this.daemon = Some(connection.clone());
                // The main window reports sending and the first sync.
                if !this.detached {
                    this.watch_sending(connection.clone(), cx);
                    this.watch_scheduled(connection.clone(), cx);
                    this.watch_updates(connection.clone(), cx);
                    this.watch_contacts(connection.clone(), cx);
                    this.watch_tasks(cx);
                    this.check_first_sync(cx);
                    this.check_signed_out(cx);
                }
            })
            .ok();
            let mut changes = match daemon::mail_changes(&connection).await {
                Ok(changes) => changes,
                Err(err) => {
                    tracing::info!("not following mail changes: {err}");
                    return;
                }
            };
            let mut reloaded: Option<Instant> = None;
            while changes.next().await.is_some() {
                let gap =
                    reloaded.map_or(Duration::ZERO, |at| CHANGE_GAP.saturating_sub(at.elapsed()));
                cx.background_executor().timer(CHANGE_DELAY.max(gap)).await;
                // Everything signalled meanwhile is read by this one reload.
                while let Some(Some(())) = futures_lite::future::poll_once(changes.next()).await {}
                let refreshed = this.update(cx, |this, cx| {
                    this.refresh(false, cx);
                    // Notes written on another device came in.
                    if this.notes.is_some() {
                        this.load_notes(cx);
                    }
                    if !this.detached {
                        this.check_first_sync(cx);
                        this.check_signed_out(cx);
                    }
                });
                if refreshed.is_err() {
                    break;
                }
                reloaded = Some(Instant::now());
            }
        }));
    }

    /// Says when a message went out, or when the server refused it for
    /// good.
    fn watch_sending(&mut self, connection: Connection, cx: &mut Context<Self>) {
        self._watch_sending = Some(cx.spawn(async move |this, cx| {
            let mut outcomes = match daemon::send_outcomes(&connection).await {
                Ok(outcomes) => Box::pin(outcomes),
                Err(err) => {
                    tracing::info!("not following the outbox: {err}");
                    return;
                }
            };
            while let Some(outcome) = outcomes.next().await {
                let shown = this.update(cx, |this, cx| match outcome {
                    daemon::SendOutcome::Sent(id) => this.went_out(id, cx),
                    daemon::SendOutcome::Failed {
                        id,
                        subject,
                        detail,
                    } => {
                        this.send_failed(id, cx);
                        this.play_event_sound(katna_core::config::SoundEvent::NotSent);
                        let subject = if subject.trim().is_empty() {
                            "(no subject)".to_owned()
                        } else {
                            subject
                        };
                        let text = format!("\u{201c}{subject}\u{201d} could not be sent: {detail}");
                        this.show_snackbar_for(text, None, FAILURE_TIME, cx);
                    }
                });
                if shown.is_err() {
                    break;
                }
            }
        }));
    }

    /// How strongly a card beside the list shows its shadow and its edge,
    /// as `outline` times (shadow, edge): the pane with the keys stands out
    /// and the others sink back. `active` is how far this card has the
    /// keys, from [`MailWindow::card_keys`].
    fn card_edges(&self, active: f32, outline: f32) -> (f32, f32) {
        let active = active.clamp(0.0, 1.0);
        (
            outline * lerp(SHADOW_REST, 1.0, active),
            outline * lerp(EDGE_REST, 1.0, active),
        )
    }

    /// How far the list (`reader` false) or the conversation beside it has
    /// the keys, following them as they move.
    fn card_keys(&self, reader: bool) -> f32 {
        if reader {
            self.keys_t
        } else {
            1.0 - self.keys_t
        }
    }

    /// With one account the folders stand alone, without an account heading.
    fn rebuild_nav(&mut self) {
        self.nav_fold = None;
        self.nav_rev += 1;
        self.nav_rows = self.nav_rows_now();
    }

    /// The lines the folder pane shows now.
    fn nav_rows_now(&self) -> Vec<sidebar::Row> {
        let only = self.shown_account();
        // With one account on show its folders stand alone, never folded.
        let single = only.is_some() || self.tree.accounts.len() == 1;
        let mut rows = self
            .tree
            .rows(&self.expanded, only, |id| single || self.account_open(id));
        let accounts = rows
            .iter()
            .filter(|r| matches!(r, sidebar::Row::Account { .. }))
            .count();
        if accounts == 1 {
            rows.retain(|r| !matches!(r, sidebar::Row::Account { .. }));
        }
        if self.config.mail.unified_inbox {
            let mut all = self
                .tree
                .unified_rows(&self.expanded, self.all_accounts_open);
            all.append(&mut rows);
            rows = all;
        }
        // Scheduled mail shows under the first Sent folder while there is
        // some.
        let scheduled = self.writing.scheduled_count();
        if scheduled > 0 {
            let after_sent = |ix: usize| {
                ix + 1
                    + rows[ix + 1..]
                        .iter()
                        .take_while(|r| matches!(r, sidebar::Row::UnifiedAccount { .. }))
                        .count()
            };
            let at = rows
                .iter()
                .position(|r| {
                    matches!(
                        r,
                        sidebar::Row::Folder {
                            role: Role::Sent,
                            ..
                        } | sidebar::Row::Unified {
                            view: sidebar::Unified::Sent,
                            ..
                        }
                    )
                })
                .map_or(rows.len(), after_sent);
            rows.insert(
                at,
                sidebar::Row::Folder {
                    key: compose::SCHEDULED_NAV_KEY.to_owned(),
                    depth: 0,
                    label: "Scheduled".to_owned(),
                    role: Role::Other,
                    folder: None,
                    unread: scheduled as u64,
                    has_children: false,
                    expanded: false,
                },
            );
        }
        rows
    }

    /// The folder the list shows; `None` for search results.
    fn listed_folder(&self) -> Option<FolderId> {
        match &self.listing {
            Some(Listing::Folder(folder)) => Some(*folder),
            _ => None,
        }
    }

    /// "Report spam", or "Not spam" in the Spam folder, where the same
    /// button takes mail back to the inbox.
    fn spam_label(&self, menu: bool) -> String {
        match (self.folder_role() == Role::Junk, menu) {
            (true, _) => katna_i18n::tr!("menu-not-spam"),
            (false, true) => katna_i18n::tr!("menu-spam"),
            (false, false) => katna_i18n::tr!("list-spam"),
        }
    }

    fn folder_role(&self) -> Role {
        match &self.listing {
            Some(Listing::Folder(folder)) => {
                self.tree.node(*folder).map_or(Role::Other, |n| n.role)
            }
            Some(Listing::Unified { view, .. }) => view.role().unwrap_or(Role::Other),
            _ => Role::Other,
        }
    }

    /// Whether the list shows inbox tabs now.
    fn shows_tabs(&self) -> bool {
        !self.tabs.is_empty() && self.folder_role() == Role::Inbox
    }

    /// The inbox tabs of `account`: as its settings say, else its
    /// provider's.
    fn account_tabs(&self, account: AccountId) -> Vec<Tab> {
        if !self.config.mail.inbox_tabs {
            return Vec::new();
        }
        let Some(account) = self.accounts.iter().find(|a| a.id == account) else {
            return Vec::new();
        };
        let setting = self.config.mail.tabs_of(&account.address);
        tabs::tabs(&setting, self.provider(account))
    }

    fn provider(&self, account: &Account) -> Provider {
        let host = self
            .mail
            .as_ref()
            .ok()
            .and_then(|mail| mail.incoming_host(account.id));
        Provider::detect(&account.address, host.as_deref())
    }

    /// `folder`'s lines and, for an inbox, its unread conversations per
    /// tab.
    fn list_entries(
        &self,
        folder: FolderId,
    ) -> (Vec<Entry>, Option<HashMap<katna_core::MailCategory, u64>>) {
        let Ok(mail) = &self.mail else {
            return (Vec::new(), None);
        };
        let role = self.tree.node(folder).map_or(Role::Other, |n| n.role);
        let categories = self
            .tabs
            .get(self.tab)
            .filter(|_| role == Role::Inbox)
            .map(|tab| tab.categories.as_slice());
        let conversations = self.config.mail.conversations;
        if role == Role::Inbox {
            let (entries, unread) = mail.inbox_entries(folder, categories, conversations);
            (entries, Some(unread))
        } else {
            (mail.entries(folder, categories, conversations), None)
        }
    }

    fn open_folder(&mut self, folder: FolderId, cx: &mut Context<Self>) {
        self.follow_folder_account(folder);
        let Ok(mail) = &mut self.mail else {
            return;
        };
        let role = self.tree.node(folder).map_or(Role::Other, |n| n.role);
        if role.shows_recipients() != self.show_recipients {
            self.show_recipients = role.shows_recipients();
            mail.clear_rows();
        }
        if self.folder != Some(folder) {
            self.tab = 0;
        }
        self.tabs = match self.tree.account_of(folder) {
            Some(account) if role == Role::Inbox => self.account_tabs(account),
            _ => Vec::new(),
        };
        self.tab = self.tab.min(self.tabs.len().saturating_sub(1));
        self.folder = Some(folder);
        self.unified = None;
        self.listing = Some(Listing::Folder(folder));
        let (entries, unread) = self.list_entries(folder);
        self.entries = entries;
        self.category_unread = unread.unwrap_or_default();
        self.reset_list(false);
        self.selected = (!self.entries.is_empty()).then_some(0);
        self.checked.clear();
        self.check_anchor = None;
        self.checked_all = false;
        self.page_pick = None;
        self.picked = None;
        self.menu = None;
        self.show_list();
        cx.notify();
    }

    fn open_tab(&mut self, tab: usize, cx: &mut Context<Self>) {
        if self.tab == tab {
            return;
        }
        // Another tab opens at its top, without the last one's glide.
        self.layout.stop_glide();
        self.tab = tab;
        // No fade: the tab's lines replace the last ones in the same frame,
        // as the indicator slides over. A conversation open beside the
        // list stays open, as the list is still in sight; over the list,
        // it closes to show the tab.
        let keep_open = self.pane_open();
        if let Some(folder) = self.folder {
            self.open_folder(folder, cx);
        } else if let Some((view, account)) = self.unified {
            self.open_unified(view, account, cx);
        }
        if keep_open {
            self.reading = true;
            let key = self.reader.as_ref().map(|r| r.key);
            if let Some(ix) = self.entries.iter().position(|e| Some(e.key) == key) {
                self.selected = Some(ix);
            }
        }
    }

    fn show_list(&mut self) {
        if self.reading && !self.split() {
            self.card_seq += 1;
        }
        self.reading = false;
        // A conversation beside the list or over it slides away first.
        if !self.split() && !self.slides() {
            self.reader = None;
        }
        self.hovered = None;
    }

    /// Moves the list cursor; while reading, opens that line instead.
    fn select(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix >= self.entries.len() {
            return;
        }
        self.selected = Some(ix);
        self.list_state.scroll_to_reveal_item(ix);
        if self.reading {
            self.load_reader(ix, cx);
        }
        cx.notify();
    }

    fn open(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if ix >= self.entries.len() {
            return;
        }
        // A draft is written on, as in Gmail: in Drafts, or wherever a
        // line holds nothing but drafts.
        let entry = self.entries[ix];
        let only_drafts = self.mail.as_ref().ok().is_some_and(|mail| {
            let ids = mail.entry_messages(entry.key);
            !ids.is_empty() && mail.drafts(&ids).len() == ids.len()
        });
        if self.folder_role() == Role::Drafts || only_drafts {
            self.selected = Some(ix);
            self.open_draft(entry.latest, window, cx);
            cx.notify();
            return;
        }
        if !self.reading && !self.split() {
            self.card_seq += 1;
        }
        // A result opened: cancelling the search no longer goes back.
        if matches!(self.listing, Some(Listing::Search { .. })) {
            self.before_search = None;
        }
        // With a conversation already beside the list the lines keep their
        // height, so the list stays where it was scrolled; only a line cut
        // off at an edge comes fully into view.
        if self.pane_open() {
            self.keep_line = None;
            self.list_state.scroll_to_reveal_item(ix);
        } else {
            self.keep_line = self
                .list_state
                .bounds_for_item(ix)
                .map(|bounds| list::KeepLine {
                    ix,
                    top: bounds.top() - self.list_state.viewport_bounds().top(),
                    placed: false,
                });
        }
        self.selected = Some(ix);
        self.reading = true;
        self.menu = None;
        self.load_reader(ix, cx);
        window.focus(&self.list_focus, cx);
        cx.notify();
    }

    fn load_reader(&mut self, ix: usize, cx: &mut Context<Self>) {
        let entry = self.entries[ix];
        if self.reader.as_ref().is_some_and(|r| r.key == entry.key) {
            return;
        }
        let Ok(mail) = &mut self.mail else {
            return;
        };
        let conversation = Conversation::load(mail, entry.key);
        self.reader_scroll.set_offset(gpui::point(px(0.0), px(0.0)));
        let has_cards = self.sending.cards(entry.key).next().is_some();
        // Opening marks the conversation read, as webmail does: at once,
        // after it has been open a moment, or never (`mail.mark_read`).
        let unread = conversation.unread_messages();
        self.reader = Some(conversation);
        if has_cards {
            self.show_sent_cards(cx);
        }
        self.read_timer = None;
        if unread.is_empty() {
            return;
        }
        match self.config.mail.mark_read.delay() {
            Some(delay) if delay.is_zero() => self.mark_opened_read(entry.key, unread, cx),
            Some(delay) => {
                self.read_timer = Some(cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(delay).await;
                    this.update(cx, |this, cx| {
                        // Only if it is still the one open.
                        if this.reading && this.reader.as_ref().is_some_and(|r| r.key == entry.key)
                        {
                            this.mark_opened_read(entry.key, unread, cx);
                            cx.notify();
                        }
                    })
                    .ok();
                }));
            }
            None => {}
        }
    }

    fn mark_opened_read(&mut self, key: EntryKey, unread: Vec<MessageId>, cx: &mut Context<Self>) {
        self.pending.entry(key).or_default().unread = Some(false);
        self.send(Command::MarkRead(unread, true), None, None, true, cx);
    }

    fn move_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.entries.is_empty() {
            return;
        }
        let last = self.entries.len() - 1;
        let ix = match self.selected {
            None if delta > 0 => 0,
            None => last,
            Some(ix) => ix.saturating_add_signed(delta).min(last),
        };
        self.select(ix, cx);
    }

    fn select_next(&mut self, _: &SelectNext, _: &mut Window, cx: &mut Context<Self>) {
        self.move_selection(1, cx);
    }

    fn select_previous(&mut self, _: &SelectPrevious, _: &mut Window, cx: &mut Context<Self>) {
        self.move_selection(-1, cx);
    }

    fn select_first(&mut self, _: &SelectFirst, _: &mut Window, cx: &mut Context<Self>) {
        self.select(0, cx);
    }

    /// Home with the keys in none of the panes, as after a click on the
    /// top bar: the list on screen glides back to its top. The folder pane
    /// and a conversation keep Home for themselves, and the list's own
    /// Home selects its first line.
    fn list_top(&mut self, _: &ListTop, window: &mut Window, cx: &mut Context<Self>) {
        let in_pane = [&self.nav_focus, &self.reader_focus, &self.list_focus]
            .iter()
            .any(|f| f.contains_focused(window, cx));
        let list_shown = self.app == RailApp::Mail
            && self.settings_page.is_none()
            && self.mail.is_ok()
            && (!self.reading || self.split());
        if in_pane || !list_shown {
            cx.propagate();
            return;
        }
        self.glide_list_to_top(cx);
    }

    fn select_last(&mut self, _: &SelectLast, _: &mut Window, cx: &mut Context<Self>) {
        self.select(self.entries.len().saturating_sub(1), cx);
    }

    fn page_down(&mut self, _: &PageDown, _: &mut Window, cx: &mut Context<Self>) {
        self.move_selection(PAGE as isize, cx);
    }

    fn page_up(&mut self, _: &PageUp, _: &mut Window, cx: &mut Context<Self>) {
        self.move_selection(-(PAGE as isize), cx);
    }

    fn open_message(&mut self, _: &OpenMessage, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ix) = self.selected {
            self.open(ix, window, cx);
            // Beside the list, Up and Down already show each conversation;
            // Enter goes into the one shown, to scroll and act on it.
            if self.pane_open() {
                window.focus(&self.reader_focus, cx);
            }
        }
    }

    /// Shift+F10 or the Menu key: the selected line's right-click menu,
    /// at its left edge, as on any desktop.
    fn open_context_menu_key(
        &mut self,
        _: &OpenContextMenu,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(ix) = self.selected.filter(|&ix| ix < self.entries.len()) else {
            return;
        };
        let Some(bounds) = self.list_state.bounds_for_item(ix) else {
            return;
        };
        let at = bounds.origin + gpui::point(px(48.0), bounds.size.height);
        self.open_context_menu(ix, self.entries[ix].key, at, cx);
    }

    /// Esc (or U, Backspace) in the conversation beside the list: the keys
    /// go back to the list, and the conversation stays shown.
    fn reader_back(&mut self, _: &CloseMessage, window: &mut Window, cx: &mut Context<Self>) {
        // Esc first folds the chat's list of people or its attach picker.
        if self.fold_chat_summary(cx)
            || self.fold_chat_people(cx)
            || self.fold_chat_pins(cx)
            || self.fold_files_picker(cx)
        {
            return;
        }
        window.focus(&self.list_focus, cx);
        cx.notify();
    }

    /// F6: the next pane from anywhere, a field included, as in Outlook,
    /// Thunderbird and KDE's apps.
    fn next_pane(&mut self, _: &NextPane, window: &mut Window, cx: &mut Context<Self>) {
        if !self.cycle_panes(true, window, cx) && self.settings_page.is_none() {
            self.focus_list(&FocusList, window, cx);
        }
    }

    /// Shift+F6: the pane before.
    fn previous_pane(&mut self, _: &PreviousPane, window: &mut Window, cx: &mut Context<Self>) {
        if !self.cycle_panes(false, window, cx) && self.settings_page.is_none() {
            self.focus_list(&FocusList, window, cx);
        }
    }

    /// Tab and Shift+Tab between the search box, the list and the
    /// conversation beside it, as a desktop mail app moves between its
    /// panes. `false` when the keys are elsewhere (a field, a dialog,
    /// Settings), where Tab goes to the next field or button.
    fn cycle_panes(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.settings_page.is_some() || self.mail.is_err() {
            return false;
        }
        #[derive(Clone, Copy, PartialEq)]
        enum Pane {
            Folders,
            List,
            Reader,
            Search,
        }
        // In the order they sit, left to right, then the search box.
        let mut panes = Vec::with_capacity(4);
        if self.nav_reachable() {
            panes.push(Pane::Folders);
        }
        panes.push(Pane::List);
        if self.pane_open() {
            panes.push(Pane::Reader);
        }
        panes.push(Pane::Search);
        let here = if self.search.focus_handle(cx).is_focused(window) {
            Pane::Search
        } else if self.list_focus.is_focused(window) {
            Pane::List
        } else if self.nav_focus.is_focused(window) {
            Pane::Folders
        } else if self.reader_focus.is_focused(window) || self.text.focus.is_focused(window) {
            Pane::Reader
        } else {
            return false;
        };
        let Some(at) = panes.iter().position(|p| *p == here) else {
            return false;
        };
        let count = panes.len();
        let next = if forward {
            (at + 1) % count
        } else {
            (at + count - 1) % count
        };
        match panes[next] {
            Pane::Folders => self.focus_nav(window, cx),
            Pane::List => self.focus_list(&FocusList, window, cx),
            Pane::Reader => window.focus(&self.reader_focus, cx),
            Pane::Search => self.focus_search(&FocusSearch, window, cx),
        }
        cx.notify();
        true
    }

    fn close_message(&mut self, _: &CloseMessage, window: &mut Window, cx: &mut Context<Self>) {
        // Esc first folds the chat's list of people or its attach picker.
        if self.fold_chat_people(cx) || self.fold_chat_pins(cx) || self.fold_files_picker(cx) {
            return;
        }
        if self.detached {
            window.remove_window();
            return;
        }
        self.show_list();
        window.focus(&self.list_focus, cx);
        cx.notify();
    }

    fn scroll_reader(&mut self, dy: f32, cx: &mut Context<Self>) {
        let offset = self.reader_scroll.offset();
        let max = self.reader_scroll.max_offset();
        let y = (offset.y - px(dy)).clamp(-max.y, px(0.0));
        self.reader_scroll.set_offset(gpui::point(offset.x, y));
        cx.notify();
    }

    fn reader_page(&self) -> f32 {
        (unpx(self.reader_scroll.bounds().size.height) - LINE_SCROLL).max(LINE_SCROLL)
    }

    fn scroll_down(&mut self, _: &ScrollDown, _: &mut Window, cx: &mut Context<Self>) {
        self.scroll_reader(LINE_SCROLL, cx);
    }

    fn scroll_up(&mut self, _: &ScrollUp, _: &mut Window, cx: &mut Context<Self>) {
        self.scroll_reader(-LINE_SCROLL, cx);
    }

    fn scroll_page_down(&mut self, _: &ScrollPageDown, _: &mut Window, cx: &mut Context<Self>) {
        self.scroll_reader(self.reader_page(), cx);
    }

    fn scroll_page_up(&mut self, _: &ScrollPageUp, _: &mut Window, cx: &mut Context<Self>) {
        self.scroll_reader(-self.reader_page(), cx);
    }

    fn focus_search(&mut self, _: &FocusSearch, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.search.focus_handle(cx), cx);
        self.search.update(cx, |search, cx| {
            let text = search.text().to_owned();
            search.set_text(text, cx);
        });
    }

    fn focus_list(&mut self, _: &FocusList, window: &mut Window, cx: &mut Context<Self>) {
        if self.app == RailApp::Files {
            self.focus_files(window, cx);
            return;
        }
        window.focus(&self.list_focus, cx);
        if self.selected.is_none() && !self.entries.is_empty() {
            self.select(0, cx);
        }
    }

    fn toggle_navigation(&mut self, _: &ToggleNavigation, _: &mut Window, cx: &mut Context<Self>) {
        // Phones and tablets open the folders as a drawer over the list.
        if !self.layout.shape.is_desktop() {
            self.layout.drawer = !self.layout.drawer;
            self.nav_peek = false;
            self.peek_task = None;
            cx.notify();
            return;
        }
        // One fold for Mail's folders and every page's side column.
        self.nav_open = !self.nav_open;
        self.nav_toggled_by_hand();
        self.nav_peek = false;
        self.peek_hover = (false, false);
        self.peek_task = None;
        cx.notify();
    }

    fn toggle_settings(&mut self, _: &ToggleSettings, window: &mut Window, cx: &mut Context<Self>) {
        self.settings_open = !self.settings_open;
        self.search_panel = None;
        let _ = window;
        cx.notify();
    }

    fn compose(&mut self, _: &Compose, window: &mut Window, cx: &mut Context<Self>) {
        self.open_compose(compose::Kind::New, None, window, cx);
    }

    fn reply(&mut self, _: &Reply, window: &mut Window, cx: &mut Context<Self>) {
        self.reply_with(compose::Kind::Reply, window, cx);
    }

    fn reply_all(&mut self, _: &ReplyAll, window: &mut Window, cx: &mut Context<Self>) {
        self.reply_with(compose::Kind::ReplyAll, window, cx);
    }

    fn forward(&mut self, _: &Forward, window: &mut Window, cx: &mut Context<Self>) {
        self.reply_with(compose::Kind::Forward, window, cx);
    }

    /// Reply, reply all or forward from the open conversation.
    fn reply_with(&mut self, kind: compose::Kind, window: &mut Window, cx: &mut Context<Self>) {
        if self.reading && self.reader.is_some() {
            self.open_compose(kind, None, window, cx);
        }
    }

    fn move_to(&mut self, _: &MoveTo, _: &mut Window, cx: &mut Context<Self>) {
        if self.checked.is_empty()
            && !self.reading
            && let Some(entry) = self.selected.and_then(|ix| self.entries.get(ix))
        {
            // The list shows "Move to" for ticked lines.
            self.checked.insert(entry.key);
        }
        if !self.checked.is_empty() || self.reading {
            self.menu = Some(Menu::MoveTo);
            self.sync_folder_pick(cx);
            cx.notify();
        }
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.checked = self.entries.iter().map(|e| e.key).collect();
        self.checked_all = true;
        self.picked = None;
        cx.notify();
    }

    fn select_none(&mut self, _: &SelectNone, _: &mut Window, cx: &mut Context<Self>) {
        self.checked.clear();
        self.check_anchor = None;
        self.checked_all = false;
        self.picked = None;
        cx.notify();
    }

    fn undo_action(&mut self, _: &Undo, window: &mut Window, cx: &mut Context<Self>) {
        self.undo_last(window, cx);
    }

    fn go_to(&mut self, role: Role, window: &mut Window, cx: &mut Context<Self>) {
        let Some(folder) = self
            .account()
            .and_then(|account| self.tree.role_folder(account, role))
        else {
            self.show_snackbar("This account has no such folder.", None, cx);
            return;
        };
        self.settings_page = None;
        self.open_app(RailApp::Mail, cx);
        self.clear_search(cx);
        self.card_seq += 1;
        self.open_folder(folder, cx);
        window.focus(&self.list_focus, cx);
    }

    fn go_to_inbox(&mut self, _: &GoToInbox, window: &mut Window, cx: &mut Context<Self>) {
        self.go_to(Role::Inbox, window, cx);
    }

    fn go_to_starred(&mut self, _: &GoToStarred, window: &mut Window, cx: &mut Context<Self>) {
        self.go_to(Role::Flagged, window, cx);
    }

    fn go_to_sent(&mut self, _: &GoToSent, window: &mut Window, cx: &mut Context<Self>) {
        self.go_to(Role::Sent, window, cx);
    }

    fn go_to_drafts(&mut self, _: &GoToDrafts, window: &mut Window, cx: &mut Context<Self>) {
        self.go_to(Role::Drafts, window, cx);
    }

    fn go_to_all_mail(&mut self, _: &GoToAllMail, window: &mut Window, cx: &mut Context<Self>) {
        self.go_to(Role::All, window, cx);
    }

    fn show_snackbar(
        &mut self,
        text: impl Into<SharedString>,
        undo: Option<Command>,
        cx: &mut Context<Self>,
    ) {
        self.show_snackbar_for(text, undo, SNACKBAR_TIME, cx);
    }

    fn show_snackbar_for(
        &mut self,
        text: impl Into<SharedString>,
        undo: Option<Command>,
        time: Duration,
        cx: &mut Context<Self>,
    ) {
        let mut shown = Spring::new(motion::SLIDE, 0.0);
        shown.set(1.0);
        let hide = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(time).await;
            this.update(cx, |this, cx| {
                if let Some(snackbar) = &mut this.snackbar {
                    snackbar.shown.set(0.0);
                    cx.notify();
                }
            })
            .ok();
        });
        if let Some(undo) = &undo {
            self.remember(UndoStep::Command(undo.clone()));
        }
        self.snackbar = Some(Snackbar {
            text: text.into(),
            undo,
            countdown: None,
            shown,
            _hide: hide,
        });
        cx.notify();
    }

    /// A snackbar counting the seconds down to `until`, with Undo until
    /// then; it stays for `time` in all.
    fn show_countdown(
        &mut self,
        text: impl Into<SharedString>,
        undo: Command,
        until: Instant,
        time: Duration,
        cx: &mut Context<Self>,
    ) {
        self.show_snackbar_for(text, Some(undo), time, cx);
        if let Some(snackbar) = &mut self.snackbar {
            let total = until.saturating_duration_since(Instant::now());
            snackbar.countdown = Some((until, total));
        }
    }

    /// Keeps `step` for Ctrl+Z.
    fn remember(&mut self, step: UndoStep) {
        self.undo_history.push(step);
        if self.undo_history.len() > UNDO_STEPS {
            self.undo_history.remove(0);
        }
    }

    fn hide_snackbar(&mut self, cx: &mut Context<Self>) {
        if let Some(snackbar) = &mut self.snackbar {
            snackbar.shown.set(0.0);
            cx.notify();
        }
    }

    /// The pointer came to or left Mail in the rail or the navigation
    /// panel: while the navigation is folded, it opens after a moment of
    /// rest and closes a moment after the pointer is gone from both.
    fn hover_navigation(&mut self, what: Hover, hovered: bool, cx: &mut Context<Self>) {
        match what {
            Hover::Compose | Hover::Mail => {
                self.peek_hover.0 = hovered;
                if hovered && !self.nav_peek {
                    self.peek_from = what;
                }
            }
            Hover::Panel => self.peek_hover.1 = hovered,
        }
        if self.nav_docked()
            || self.layout.drawer
            || self.app != RailApp::Mail
            || self.settings_page.is_some()
        {
            return;
        }
        let on = self.peek_hover.0 || self.peek_hover.1;
        if on == self.nav_peek {
            self.peek_task = None;
            return;
        }
        // Only the rail opens the panel; the panel only keeps it open.
        if on && !self.peek_hover.0 {
            return;
        }
        let delay = if on { PEEK_DELAY } else { PEEK_LINGER };
        self.peek_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            this.update(cx, |this, cx| {
                this.peek_task = None;
                if !this.nav_docked() {
                    this.nav_peek = on;
                    cx.notify();
                }
            })
            .ok();
        }));
    }

    fn reload(&mut self, _: &Reload, _: &mut Window, cx: &mut Context<Self>) {
        if self.mail.is_err() {
            self.reopen(cx);
            return;
        }
        self.check_mail(None, cx);
        self.refresh(true, cx);
    }

    /// Tries the store again after it could not be opened.
    fn reopen(&mut self, cx: &mut Context<Self>) {
        self.mail = Mail::open(&self.paths);
        self.load_tree();
        self.open_default_folder(cx);
        self.count_unread(cx);
        if self.mail.is_ok() && self.migrating.take().is_some() {
            // The calendar and agenda read the same store.
            self.refresh(false, cx);
        }
        self.wait_for_migration(cx);
        cx.notify();
    }

    /// While the daemon moves the store up to this version, as it does
    /// as it starts after an update, tries it again shortly.
    fn wait_for_migration(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.mail, Err(OpenError::Migrating(_))) {
            self.migrating = None;
            return;
        }
        let since = self
            .migrating
            .as_ref()
            .map_or_else(Instant::now, |(since, _)| *since);
        let task = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(MIGRATION_RETRY).await;
            this.update(cx, |this, cx| this.reopen(cx)).ok();
        });
        self.migrating = Some((since, task));
    }

    /// Opens the first inbox when nothing is listed, as when the first
    /// account's folders arrive.
    fn open_default_folder(&mut self, cx: &mut Context<Self>) {
        if self.listing.is_some() {
            return;
        }
        if self.shows_unified() {
            self.open_unified(sidebar::Unified::Inbox, None, cx);
            return;
        }
        if let Some((folder, ancestors)) = self.default_folder() {
            self.expanded.extend(ancestors);
            self.rebuild_nav();
            self.open_folder(folder, cx);
        }
    }

    /// After an account was added: its folders follow with the first sync.
    fn account_added(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        if self.mail.is_err() {
            self.reopen(cx);
        } else {
            self.refresh(true, cx);
        }
        self.first_sync = true;
        self.check_first_sync(cx);
        self.onboarding_account_added(cx);
    }

    /// Reads the store again, keeping the cursor and the open conversation.
    fn refresh(&mut self, animate: bool, cx: &mut Context<Self>) {
        if self.app == RailApp::Calendar && !self.detached {
            self.load_calendar(cx);
        }
        self.load_agenda(cx);
        self.forget_invite_looks();
        if self.mail.is_err() {
            // The daemon may have made the store since.
            self.reopen(cx);
            return;
        }
        let expanded = std::mem::take(&mut self.expanded);
        if let Ok(mail) = &mut self.mail {
            mail.refresh();
        }
        // A task's mail may have just come in.
        self.map_task_mails();
        self.pending.clear();
        if self.detached {
            self.expanded = expanded;
            if let (Some(reader), Ok(mail)) = (&mut self.reader, &mut self.mail) {
                reader.refresh(mail);
            }
            self.show_sent_cards(cx);
            return;
        }
        self.load_tree();
        self.expanded = expanded;
        self.rebuild_nav();
        self.refresh_activity();
        let selected_key = self
            .selected
            .and_then(|ix| self.entries.get(ix))
            .map(|e| e.key);
        match self.listing.clone() {
            Some(listing @ (Listing::Folder(_) | Listing::Unified { .. })) => {
                let (entries, unread) = match listing {
                    Listing::Folder(folder) => self.list_entries(folder),
                    Listing::Unified { view, account } => self.unified_entries(view, account),
                    Listing::Search { .. } => (Vec::new(), None),
                };
                let open = self.kept_open_line(&listing, &entries);
                self.entries = entries;
                // The open conversation stays where it was until it closes,
                // as in webmail, though reading it took it out of the list.
                if let Some((at, entry)) = open {
                    self.entries.insert(at.min(self.entries.len()), entry);
                }
                self.reset_list(true);
                if let Some(unread) = unread {
                    self.category_unread = unread;
                }
                self.selected =
                    selected_key.and_then(|key| self.entries.iter().position(|e| e.key == key));
                let keys: HashSet<EntryKey> = self.entries.iter().map(|e| e.key).collect();
                self.checked.retain(|key| keys.contains(key));
                // An undone delete or move: the conversation is back, and
                // opens again where it was.
                if let Some(ix) = self
                    .reopen_after_undo
                    .and_then(|key| self.entries.iter().position(|e| e.key == key))
                {
                    self.reopen_after_undo = None;
                    if !self.reading && !self.split() {
                        self.card_seq += 1;
                    }
                    self.selected = Some(ix);
                    self.reading = true;
                    self.list_state.scroll_to_reveal_item(ix);
                    self.load_reader(ix, cx);
                }
                if self.selected.is_none() && self.reading {
                    self.show_list();
                    self.reader = None;
                }
            }
            Some(Listing::Search { query, .. }) => self.start_search(query, cx),
            None => self.open_default_folder(cx),
        }
        if let (Some(reader), Ok(mail)) = (&mut self.reader, &mut self.mail) {
            reader.refresh(mail);
        }
        self.show_sent_cards(cx);
        if animate {
            self.card_seq += 1;
        }
        self.count_unread(cx);
        cx.notify();
    }

    fn quit(&mut self, _: &Quit, _: &mut Window, cx: &mut Context<Self>) {
        cx.quit();
    }

    fn clear_search(&mut self, cx: &mut Context<Self>) {
        self.search_task = None;
        self.search_error = None;
        self.search.update(cx, |search, cx| {
            if !search.text().is_empty() {
                search.set_text("", cx);
            }
        });
    }

    fn on_search_event(
        &mut self,
        search: &Entity<TextInput>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.settings_page.is_some() {
            self.on_settings_search(event, window, cx);
            return;
        }
        if self.app == RailApp::Notes {
            self.on_notes_search(event, window, cx);
            return;
        }
        if self.app == RailApp::Contacts {
            self.on_contacts_search(search, event, cx);
            return;
        }
        if self.app == RailApp::Calendar {
            self.on_calendar_search(search, event, window, cx);
            return;
        }
        if self.app == RailApp::Tasks {
            self.on_tasks_search(search, event, window, cx);
            return;
        }
        if self.app == RailApp::Files {
            self.on_files_search(search, event, window, cx);
            return;
        }
        match event {
            InputEvent::Changed => {
                let text = search.read(cx).text().trim().to_owned();
                if !text.is_empty() {
                    self.open_app(RailApp::Mail, cx);
                }
                self.start_search(text, cx);
            }
            InputEvent::Submit => self.focus_list(&FocusList, window, cx),
            InputEvent::Cancel => {
                if search.read(cx).text().is_empty() {
                    window.focus(&self.list_focus, cx);
                } else {
                    self.clear_search(cx);
                }
            }
        }
    }

    fn start_search(&mut self, text: String, cx: &mut Context<Self>) {
        self.search_error = None;
        let keep_open = std::mem::take(&mut self.clear_keeps_open);
        if text.is_empty() {
            self.search_task = None;
            if matches!(self.listing, Some(Listing::Search { .. })) {
                // Only the X keeps a result open; text deleted away goes back.
                let opened = (keep_open && self.reading)
                    .then(|| self.reader.take())
                    .flatten();
                let card_seq = self.card_seq;
                self.open_listed(cx);
                match opened {
                    Some(reader) => self.keep_open(reader, card_seq),
                    None => self.restore_before_search(cx),
                }
            }
            self.before_search = None;
            return;
        }
        let Some(index) = self.mail.as_mut().ok().and_then(Mail::index) else {
            self.search_error = self
                .mail
                .as_ref()
                .ok()
                .and_then(|m| m.index_error().map(SharedString::from));
            cx.notify();
            return;
        };
        let correct = self.search_verbatim.as_deref() != Some(text.as_str());
        self.search_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SEARCH_DELAY).await;
            let now = jiff::Timestamp::now().as_second();
            let query = text.clone();
            let results = cx
                .background_executor()
                .spawn(async move { data::search(&index, &query, now, correct) })
                .await;
            this.update(cx, |this, cx| this.show_results(text, results, cx))
                .ok();
        }));
    }

    /// Keeps what the list shows as a search replaces it, before the
    /// results take the lines and the open conversation.
    fn save_before_search(&mut self) {
        let Some(listing) = self.listing.clone() else {
            return;
        };
        let key = |ix: usize| self.entries.get(ix).map(|e| e.key);
        self.before_search = Some(BeforeSearch {
            listing,
            open: self
                .reading
                .then(|| self.reader.as_ref().map(|r| r.key))
                .flatten(),
            selected: self.selected.and_then(key),
            top: self.list_state.logical_scroll_top(),
        });
    }

    /// A search cancelled with no result opened: the list goes back to
    /// where it was, with the conversation that was open open again. The
    /// list is listed again first (`open_listed`).
    fn restore_before_search(&mut self, cx: &mut Context<Self>) {
        let Some(before) = self.before_search.take() else {
            return;
        };
        // Another folder picked meanwhile keeps its own place.
        if self.listing.as_ref() != Some(&before.listing) {
            return;
        }
        let at = |key: Option<EntryKey>| {
            key.and_then(|key| self.entries.iter().position(|e| e.key == key))
        };
        if let Some(ix) = at(before.selected) {
            self.selected = Some(ix);
        }
        self.list_state.scroll_to(before.top);
        if let Some(ix) = at(before.open) {
            if !self.reading && !self.split() {
                self.card_seq += 1;
            }
            self.selected = Some(ix);
            self.reading = true;
            self.load_reader(ix, cx);
        }
        cx.notify();
    }

    /// Cleared with the X: the conversation opened from the results stays
    /// where it is, picked in the folder when it lies there.
    fn keep_open(&mut self, reader: Conversation, card_seq: usize) {
        self.card_seq = card_seq;
        let ix = self.entries.iter().position(|e| e.key == reader.key);
        self.reader = Some(reader);
        self.reading = true;
        self.selected = ix;
        if let Some(ix) = ix {
            self.list_state.scroll_to_reveal_item(ix);
        }
    }

    fn show_results(
        &mut self,
        query: String,
        results: Result<(SearchResults, Option<String>), String>,
        cx: &mut Context<Self>,
    ) {
        match results {
            Ok((results, corrected)) => {
                // Search results mix folders; show senders.
                if self.show_recipients {
                    self.show_recipients = false;
                    if let Ok(mail) = &mut self.mail {
                        mail.clear_rows();
                    }
                }
                let first = !matches!(self.listing, Some(Listing::Search { .. }));
                if first {
                    self.save_before_search();
                }
                // The same search again, after the mail changed: keep the
                // cursor, the ticks and the open conversation.
                let again = matches!(
                    &self.listing,
                    Some(Listing::Search { query: old, .. }) if *old == query
                );
                let selected_key = self
                    .selected
                    .and_then(|ix| self.entries.get(ix))
                    .map(|e| e.key);
                let hits: Vec<MessageId> = results.hits.iter().map(|hit| hit.message).collect();
                let only = self.shown_account();
                self.entries = match &self.mail {
                    Ok(mail) => mail.hit_entries(&hits, self.config.mail.conversations, only),
                    Err(_) => Vec::new(),
                };
                self.listing = Some(Listing::Search {
                    query,
                    total: results.total,
                    corrected,
                });
                if again {
                    self.selected =
                        selected_key.and_then(|key| self.entries.iter().position(|e| e.key == key));
                    let keys: HashSet<EntryKey> = self.entries.iter().map(|e| e.key).collect();
                    self.checked.retain(|key| keys.contains(key));
                    self.reset_list(true);
                    if self.selected.is_none() && self.reading {
                        self.show_list();
                        self.reader = None;
                    }
                    cx.notify();
                    return;
                }
                self.selected = (!self.entries.is_empty()).then_some(0);
                self.checked.clear();
                self.check_anchor = None;
                self.checked_all = false;
                self.page_pick = None;
                self.picked = None;
                self.reset_list(false);
                if first {
                    self.card_seq += 1;
                }
                self.show_list();
                self.reader = None;
            }
            Err(err) => self.search_error = Some(err.into()),
        }
        cx.notify();
    }

    fn folder_name(&self) -> Option<String> {
        match &self.listing {
            Some(Listing::Folder(folder)) => self.tree.node(*folder).map(|n| n.label()),
            Some(Listing::Unified { view, account }) => Some(self.unified_name(*view, *account)),
            _ => None,
        }
    }

    /// The account of the list: the folder's, or the first one.
    fn account(&self) -> Option<AccountId> {
        self.folder
            .and_then(|f| self.tree.account_of(f))
            .or_else(|| self.shown_account())
            .or_else(|| self.accounts.first().map(|a| a.id))
    }

    // Changes

    /// The lines an action applies to: the ticked ones, else the open
    /// conversation, else the cursor's line.
    fn target_keys(&self) -> Vec<EntryKey> {
        if !self.checked.is_empty() {
            return self
                .entries
                .iter()
                .filter(|e| self.checked.contains(&e.key))
                .map(|e| e.key)
                .collect();
        }
        if self.reading
            && let Some(reader) = &self.reader
        {
            return vec![reader.key];
        }
        self.selected
            .and_then(|ix| self.entries.get(ix))
            .map(|e| vec![e.key])
            .unwrap_or_default()
    }

    fn act_on_targets(&mut self, act: Act, cx: &mut Context<Self>) {
        let keys = self.target_keys();
        self.act(act, keys, cx);
    }

    /// Applies `act` to the lines `keys`: at once in the window, then in
    /// the store and on the server through the daemon.
    fn act(&mut self, act: Act, keys: Vec<EntryKey>, cx: &mut Context<Self>) {
        self.act_with(act, keys, true, cx);
    }

    /// Does `act` to the lines `keys`, saying so with an Undo when
    /// `announce`. Returns what undoes it.
    fn act_with(
        &mut self,
        act: Act,
        keys: Vec<EntryKey>,
        announce: bool,
        cx: &mut Context<Self>,
    ) -> Option<Command> {
        self.menu = None;
        if keys.is_empty() {
            return None;
        }
        let Ok(mail) = &self.mail else {
            return None;
        };
        // What the snackbar counts: conversations or messages.
        let (count, conversations) = (keys.len(), self.config.mail.conversations);
        let kind = if conversations {
            "conversation"
        } else {
            "message"
        };
        let folder = self
            .folder
            .filter(|_| matches!(self.listing, Some(Listing::Folder(_))));
        let messages_in = |key: EntryKey| match folder {
            Some(folder) => mail.entry_messages_in(key, folder),
            None => mail.entry_messages(key),
        };
        // Flag changes touch every stored copy of a message (the line is
        // starred or unread when any copy is), and undo restores exactly
        // the copies that changed.
        let copies_of = |keys: &[EntryKey]| {
            let ids: Vec<MessageId> = keys.iter().flat_map(|k| mail.entry_messages(*k)).collect();
            mail.with_copies(&ids)
        };
        let account = self.account();
        let in_spam = self.folder_role() == Role::Junk;
        let trash = account.and_then(|a| mail.trash_folder(a));
        let for_good = matches!(act, Act::Delete)
            && match (trash, folder) {
                (None, _) => true,
                (Some(trash), Some(folder)) => folder == trash,
                // Search results: all of them are in Trash already.
                (Some(trash), None) => keys
                    .iter()
                    .flat_map(|k| messages_in(*k))
                    .all(|id| mail.message_folders(id).contains(&trash)),
            };
        if matches!(act, Act::Delete) && announce && self.delete_needs_asking(count, for_good) {
            self.ask_delete(keys, for_good, cx);
            return None;
        }
        let (command, undo) = match act {
            Act::Read(read) => {
                let ids = data::flag_changes(&copies_of(&keys), MessageFlags::SEEN, read);
                for key in &keys {
                    self.pending.entry(*key).or_default().unread = Some(!read);
                }
                let undo = Command::MarkRead(ids.clone(), !read);
                (Command::MarkRead(ids, read), Some(undo))
            }
            Act::Star(on) => {
                // Starring marks the newest message; unstarring clears all.
                let copies = if on {
                    let wanted: HashSet<EntryKey> = keys.iter().copied().collect();
                    let latest: Vec<MessageId> = self
                        .entries
                        .iter()
                        .filter(|e| wanted.contains(&e.key))
                        .map(|e| e.latest)
                        .collect();
                    mail.with_copies(&latest)
                } else {
                    copies_of(&keys)
                };
                let ids = data::flag_changes(&copies, MessageFlags::FLAGGED, on);
                for key in &keys {
                    self.pending.entry(*key).or_default().flagged = Some(on);
                }
                let undo = Command::Star(ids.clone(), !on);
                (Command::Star(ids, on), Some(undo))
            }
            Act::Important(on) => {
                let ids = data::flag_changes(&copies_of(&keys), MessageFlags::IMPORTANT, on);
                for key in &keys {
                    self.pending.entry(*key).or_default().important = Some(on);
                }
                let undo = Command::Important(ids.clone(), !on);
                (Command::Important(ids, on), Some(undo))
            }
            Act::Pin(on) => {
                // The whole conversation, wherever its messages are.
                let ids: Vec<MessageId> = copies_of(&keys).into_iter().map(|(id, _)| id).collect();
                for key in &keys {
                    self.pending.entry(*key).or_default().pinned = Some(on);
                }
                let undo = Command::Pin(ids.clone(), !on);
                (Command::Pin(ids, on), Some(undo))
            }
            Act::Snooze(until) => {
                let ids: Vec<MessageId> = keys.iter().flat_map(|k| messages_in(*k)).collect();
                let undo = Command::Unsnooze(ids.clone());
                let advanced = self.remove_lines(&keys, cx);
                if let Some(key) = advanced {
                    self.undo_reopens.push((undo.clone(), key));
                    if self.undo_reopens.len() > UNDO_STEPS {
                        self.undo_reopens.remove(0);
                    }
                }
                (Command::Snooze(ids, until), Some(undo))
            }
            Act::Unsnooze => {
                let ids: Vec<MessageId> = keys.iter().flat_map(|k| messages_in(*k)).collect();
                // Undo snoozes again until the same time, if it is still ahead.
                let now = jiff::Timestamp::now().as_second();
                let undo = mail
                    .snoozed_until(&ids)
                    .filter(|until| *until > now + 60)
                    .map(|until| Command::Snooze(ids.clone(), until));
                self.remove_lines(&keys, cx);
                (Command::Unsnooze(ids), undo)
            }
            Act::Archive | Act::Delete | Act::Spam | Act::MoveTo(_) => {
                let ids: Vec<MessageId> = keys.iter().flat_map(|k| messages_in(*k)).collect();
                let target = match act {
                    // In Spam, "Not spam" takes it back to the inbox.
                    Act::Spam if in_spam => {
                        Some(account.and_then(|a| self.tree.role_folder(a, Role::Inbox))?)
                    }
                    Act::Spam => {
                        let junk = self
                            .account()
                            .and_then(|a| self.tree.role_folder(a, Role::Junk));
                        match junk {
                            Some(junk) => Some(junk),
                            None => {
                                self.show_snackbar(
                                    katna_i18n::tr!("toast-no-spam-folder"),
                                    None,
                                    cx,
                                );
                                return None;
                            }
                        }
                    }
                    Act::MoveTo(target) => Some(target),
                    _ => None,
                };
                let command = match (act, target) {
                    (Act::Archive, _) => Command::Archive(ids.clone()),
                    (Act::Delete, _) => Command::Delete(ids.clone()),
                    (_, Some(target)) => Command::Move(ids.clone(), target),
                    _ => return None,
                };
                // Deleting on an account without a Trash folder, or in
                // Trash itself, is for good (as the daemon does it), so
                // there is nothing to undo. Out of search results or a
                // conversation window, each message goes back where it was.
                let undo = match folder {
                    _ if for_good => None,
                    Some(folder) => Some(Command::Move(ids, folder)),
                    None => {
                        let all_mail = account.and_then(|a| self.tree.role_folder(a, Role::All));
                        let to = match act {
                            Act::Archive => account.and_then(|a| {
                                self.tree
                                    .role_folder(a, Role::Archive)
                                    .or_else(|| self.tree.role_folder(a, Role::All))
                            }),
                            Act::Delete => trash,
                            _ => target,
                        };
                        move_back(mail, all_mail, &ids, to)
                    }
                };
                let advanced = self.remove_lines(&keys, cx);
                // Undo brings the conversation back and opens it again.
                if let (Some(key), Some(undo)) = (advanced, &undo) {
                    self.undo_reopens.push((undo.clone(), key));
                    if self.undo_reopens.len() > UNDO_STEPS {
                        self.undo_reopens.remove(0);
                    }
                }
                (command, undo)
            }
        };
        let done = match act {
            _ if !announce => None,
            Act::Read(read) => Some(if read {
                katna_i18n::tr!("toast-marked-read", count = count as u64, kind = kind)
            } else {
                katna_i18n::tr!("toast-marked-unread", count = count as u64, kind = kind)
            }),
            Act::Snooze(until) => Some(katna_i18n::tr!(
                "toast-snoozed",
                count = count as u64,
                kind = kind,
                when = snooze::describe(until, &self.tz)
            )),
            Act::Spam if in_spam => Some(katna_i18n::tr!(
                "toast-not-spam",
                count = count as u64,
                kind = kind
            )),
            Act::Spam => Some(katna_i18n::tr!(
                "toast-spam",
                count = count as u64,
                kind = kind
            )),
            Act::Delete if for_good => Some(katna_i18n::tr!(
                "toast-deleted-forever",
                count = count as u64,
                kind = kind
            )),
            _ => command.done_text(count, conversations),
        };
        let deleted_forever = for_good && announce;
        // Moved out of a conversation window, which now closes: the mail
        // window says so and offers Undo.
        if self.detached
            && self.reader.is_none()
            && let Some(main) = self.main.as_ref().and_then(WeakEntity::upgrade)
        {
            main.update(cx, |main, cx| {
                if deleted_forever {
                    main.remember(UndoStep::DeletedForever);
                }
                main.send(command, done, undo.clone(), false, cx)
            });
            cx.notify();
            return undo;
        }
        if deleted_forever {
            self.remember(UndoStep::DeletedForever);
        }
        self.send(command, done, undo.clone(), false, cx);
        cx.notify();
        undo
    }

    /// The open conversation's line and where it is, when the list
    /// `listing` is about to get `entries` without it: a list of unread
    /// mail loses the conversation that opening it marked read.
    fn kept_open_line(&self, listing: &Listing, entries: &[Entry]) -> Option<(usize, Entry)> {
        let Listing::Unified { view, .. } = listing else {
            return None;
        };
        if view.filter() == Default::default() || !self.reading || self.detached {
            return None;
        }
        let key = self.reader.as_ref()?.key;
        if entries.iter().any(|e| e.key == key) {
            return None;
        }
        let at = self.entries.iter().position(|e| e.key == key)?;
        Some((at, self.entries[at]))
    }

    /// Takes lines out of the list, keeping the cursor on the next one.
    /// When the open conversation goes, the next one opens in its place
    /// (Settings > General > Auto-advance), with no empty pane between;
    /// returns the key of the one that went.
    fn remove_lines(&mut self, keys: &[EntryKey], cx: &mut Context<Self>) -> Option<EntryKey> {
        // A set: the list and the selection can both be thousands long.
        let keys: HashSet<EntryKey> = keys.iter().copied().collect();
        let cursor = self.selected.unwrap_or(0);
        let removed_before = |at: usize, entries: &[Entry]| {
            entries[..at.min(entries.len())]
                .iter()
                .filter(|e| keys.contains(&e.key))
                .count()
        };
        let before_cursor = removed_before(cursor, &self.entries);
        // Where the open conversation was, when it goes: the line that
        // took its place is at this index afterwards.
        let open_gone = self
            .reader
            .as_ref()
            .filter(|r| keys.contains(&r.key))
            .map(|r| r.key);
        let open_at = open_gone
            .and_then(|key| self.entries.iter().position(|e| e.key == key))
            .map(|at| at - removed_before(at, &self.entries));
        for ix in (0..self.entries.len()).rev() {
            if keys.contains(&self.entries[ix].key) {
                self.list_state.remove(ix);
            }
        }
        self.entries.retain(|e| !keys.contains(&e.key));
        self.selected = if self.entries.is_empty() {
            None
        } else {
            Some((cursor - before_cursor).min(self.entries.len() - 1))
        };
        for key in &keys {
            self.checked.remove(key);
        }
        self.checked_all = false;
        self.page_pick = None;
        self.picked = None;
        self.hovered = None;
        let open_gone = open_gone?;
        let next = open_at
            .filter(|_| self.reading && !self.detached)
            .and_then(|at| self.config.mail.auto_advance.pick(at, self.entries.len()));
        match next {
            Some(ix) => {
                self.selected = Some(ix);
                self.list_state.scroll_to_reveal_item(ix);
                self.load_reader(ix, cx);
            }
            None => {
                self.show_list();
                self.reader = None;
            }
        }
        Some(open_gone)
    }

    /// Sends `command` to the daemon. Shows `done` when it is applied, and
    /// the error if it fails, unless `quiet`.
    fn send(
        &mut self,
        command: Command,
        done: Option<String>,
        undo: Option<Command>,
        quiet: bool,
        cx: &mut Context<Self>,
    ) {
        let connection = self.daemon.clone();
        let notes = command.touches_notes();
        let drive = command.drive();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let connection = match connection {
                        Some(connection) => connection,
                        None => daemon::connect().await?,
                    };
                    daemon::send(&connection, &command).await
                })
                .await;
            this.update(cx, |this, cx| {
                if let Some(account) = drive {
                    this.drive_listing_changed(account, cx);
                }
                match result {
                    Ok(()) => {
                        if notes {
                            this.load_notes(cx);
                        }
                        if let Some(done) = done {
                            this.show_snackbar(done, undo, cx);
                        }
                    }
                    Err(err) => {
                        tracing::info!("{err}");
                        if !quiet {
                            this.show_snackbar(err, None, cx);
                            // Show the store as it is again.
                            this.refresh(false, cx);
                        }
                    }
                }
            })
            .ok();
        })
        .detach();
    }

    /// Undo on the snackbar: takes back what it tells of.
    fn undo(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(undo) = self.snackbar.as_mut().and_then(|s| s.undo.take()) else {
            return;
        };
        let step = UndoStep::Command(undo.clone());
        if let Some(ix) = self.undo_history.iter().rposition(|s| *s == step) {
            self.undo_history.remove(ix);
        }
        self.hide_snackbar(cx);
        self.run_undo(undo, window, cx);
    }

    /// Ctrl+Z: takes back the newest action not undone yet, whether or not
    /// its snackbar is still on screen.
    fn undo_last(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.undo_history.pop() {
            None => self.show_snackbar(katna_i18n::tr!("toast-nothing-to-undo"), None, cx),
            Some(UndoStep::DeletedForever) => {
                self.show_snackbar(
                    katna_i18n::tr!("toast-cannot-undo-delete-forever"),
                    None,
                    cx,
                );
            }
            Some(UndoStep::Command(undo)) => {
                if let Some(snackbar) = &mut self.snackbar
                    && snackbar.undo.as_ref() == Some(&undo)
                {
                    snackbar.undo = None;
                    self.hide_snackbar(cx);
                }
                self.run_undo(undo, window, cx);
            }
        }
    }

    fn run_undo(&mut self, undo: Command, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ix) = self.undo_reopens.iter().rposition(|(u, _)| *u == undo) {
            self.reopen_after_undo = Some(self.undo_reopens.remove(ix).1);
        }
        if undo == Command::ReopenDraft {
            self.reopen_closed_draft(window, cx);
            return;
        }
        if undo == Command::RestoreQuote {
            self.restore_quote(window, cx);
            return;
        }
        if undo == Command::UndoRephrase {
            self.undo_rephrase(window, cx);
            return;
        }
        if let Command::RestoreSubject(subject) = undo {
            self.restore_subject(subject, window, cx);
            return;
        }
        if let Command::Event(change) = undo {
            self.undo_event_change(*change, cx);
            return;
        }
        if let Command::RestoreContacts(keys) = &undo {
            self.restore_contacts(keys, cx);
            return;
        }
        if let Command::RestoreScheme(id, contents, was_used) = &undo {
            self.restore_scheme(id, contents, *was_used, cx);
            return;
        }
        if let Command::UndoSend(id) = undo {
            self.send_undone(id, cx);
            // Taken back from the outbox: the message opens again.
            let connection = self.daemon.clone();
            cx.spawn_in(window, async move |this, cx| {
                let result = cx
                    .background_executor()
                    .spawn(async move {
                        let connection = match connection {
                            Some(connection) => connection,
                            None => daemon::connect().await?,
                        };
                        daemon::send(&connection, &undo).await
                    })
                    .await;
                this.update_in(cx, |this, window, cx| match result {
                    Ok(()) => {
                        // Send and archive: the conversation comes back too.
                        let unarchive = this.unsent.as_mut().and_then(|u| u.unarchive.take());
                        if let Some(command) = unarchive {
                            this.send(command, None, None, false, cx);
                        }
                        this.reopen_unsent(window, cx);
                        this.show_snackbar(katna_i18n::tr!("toast-send-undone"), None, cx);
                    }
                    Err(err) => this.show_snackbar(err, None, cx),
                })
                .ok();
            })
            .detach();
            return;
        }
        self.send(undo, Some(katna_i18n::tr!("toast-undone")), None, false, cx);
    }

    fn archive(&mut self, _: &Archive, _: &mut Window, cx: &mut Context<Self>) {
        self.act_on_targets(Act::Archive, cx);
    }

    fn delete(&mut self, _: &Delete, _: &mut Window, cx: &mut Context<Self>) {
        self.act_on_targets(Act::Delete, cx);
    }

    fn report_spam(&mut self, _: &ReportSpam, _: &mut Window, cx: &mut Context<Self>) {
        self.act_on_targets(Act::Spam, cx);
    }

    fn mark_read(&mut self, _: &MarkRead, _: &mut Window, cx: &mut Context<Self>) {
        self.act_on_targets(Act::Read(true), cx);
    }

    fn mark_unread(&mut self, _: &MarkUnread, window: &mut Window, cx: &mut Context<Self>) {
        let reading = self.reading && self.checked.is_empty();
        self.act_on_targets(Act::Read(false), cx);
        // As in webmail, marking the open conversation unread closes it.
        if reading {
            self.close_message(&CloseMessage, window, cx);
            self.reader = None;
        }
    }

    fn toggle_star(&mut self, _: &ToggleStar, _: &mut Window, cx: &mut Context<Self>) {
        let keys = self.target_keys();
        let on = !keys.iter().all(|k| self.is_flagged(*k));
        self.act(Act::Star(on), keys, cx);
    }

    fn mark_important(&mut self, _: &MarkImportant, _: &mut Window, cx: &mut Context<Self>) {
        self.act_on_targets(Act::Important(true), cx);
    }

    fn mark_not_important(&mut self, _: &MarkNotImportant, _: &mut Window, cx: &mut Context<Self>) {
        self.act_on_targets(Act::Important(false), cx);
    }

    fn toggle_mute(&mut self, _: &ToggleMute, _: &mut Window, cx: &mut Context<Self>) {
        self.toggle_mute_targets(cx);
    }

    fn toggle_check(&mut self, _: &ToggleCheck, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(ix) = self.selected {
            self.click_check(ix, false, cx);
        }
    }

    /// Whether line `key` is starred, counting changes not read back yet.
    fn is_flagged(&mut self, key: EntryKey) -> bool {
        if let Some(flagged) = self.pending.get(&key).and_then(|p| p.flagged) {
            return flagged;
        }
        let Some(entry) = self.entries.iter().find(|e| e.key == key).copied() else {
            return false;
        };
        let folder = self.listed_folder();
        match &mut self.mail {
            Ok(mail) => mail
                .rows(&[entry], folder, self.show_recipients)
                .into_iter()
                .flatten()
                .any(|r| r.flagged),
            Err(_) => false,
        }
    }

    fn on_split_drag(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        let Some((start_x, start_share)) = self.split_drag else {
            return;
        };
        if event.pressed_button != Some(MouseButton::Left) {
            self.split_drag = None;
            self.save_config();
            return;
        }
        let width = (self.cards_width - SPLIT_GAP).max(1.0);
        let dx = unpx(event.position.x) - start_x;
        let share = (start_share - dx / width).clamp(0.25, 0.75);
        self.config.mail.reading_pane_share = share;
        cx.notify();
    }

    fn render_snackbar(
        &mut self,
        th: &Theme,
        window: &Window,
        reduce: bool,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let snackbar = self.snackbar.as_mut()?;
        let s = snackbar.shown.tick(window, reduce);
        if snackbar.shown.target() == 0.0 && snackbar.shown.settled() {
            self.snackbar = None;
            return None;
        }
        let s = s.max(0.0);
        let leaving = snackbar.shown.target() == 0.0;
        // Counted down: too late to take back.
        let left = snackbar
            .countdown
            .map(|(until, total)| (until.saturating_duration_since(Instant::now()), total))
            .filter(|(left, _)| !left.is_zero());
        if left.is_none() && snackbar.countdown.is_some() {
            snackbar.undo = None;
            snackbar.countdown = None;
        }
        // The ring shrinks smoothly, a frame at a time.
        let ring = left.map(|(left, total)| {
            window.request_animation_frame();
            let share = left.as_secs_f32() / total.as_secs_f32().max(0.001);
            let ink = th.snackbar_text;
            countdown_ring(
                share,
                left.as_secs_f32().ceil() as u64,
                ink,
                crate::theme::fade(ink, 0.25),
                30.0,
            )
        });
        let text = snackbar.text.clone();
        let has_undo = snackbar.undo.is_some();
        // On a phone the note spans the window above the bottom bar.
        let shape = self.layout.shape;
        let edge = lerp(24.0, 8.0, shape.phone);
        Some(
            div()
                .id("snackbar")
                // Clicks on the note stop here, not on what is under it,
                // until it is fading away.
                .when(!leaving, |d| d.occlude())
                .absolute()
                .left(px(edge))
                .when(shape.is_phone(), |d| d.right(px(edge)))
                .bottom(px(shape.bottom_bar() + lerp(-12.0, edge, s)))
                .opacity(s.min(1.0))
                .min_w(px(288.0))
                .max_w(px(560.0))
                .pl(px(16.0))
                .pr(px(8.0))
                .py(px(6.0))
                .min_h(px(48.0))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .rounded(px(6.0))
                .bg(rgba(th.snackbar))
                .text_color(rgba(th.snackbar_text))
                .text_size(px(14.0))
                .shadow(elevation(th, 3.0))
                .when(ring.is_some(), |d| d.pl(px(10.0)))
                .children(ring)
                .child(div().flex_1().min_w_0().mr(px(16.0)).child(text))
                .when(has_undo, |d| {
                    d.child(
                        div()
                            .id("undo")
                            .px(px(12.0))
                            .py(px(8.0))
                            .rounded(px(4.0))
                            .text_color(rgba(if th.dark { th.nav_selected } else { 0xa8c7faff }))
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .cursor_pointer()
                            .hover(|s| s.bg(rgba(0xffffff1f)))
                            .on_click(cx.listener(|this, _, window, cx| this.undo(window, cx)))
                            .child(katna_i18n::tr!("toast-undo")),
                    )
                })
                .child(
                    div()
                        .id("snackbar-close")
                        .size(px(32.0))
                        .flex_none()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .cursor_pointer()
                        .hover(|s| s.bg(rgba(0xffffff1f)))
                        .tooltip(tip(katna_i18n::tr!("toast-close"), th))
                        .on_click(cx.listener(|this, _, _, cx| this.hide_snackbar(cx)))
                        .child(icon("close", th.snackbar_text, 18.0)),
                )
                .into_any_element(),
        )
    }

    /// Whether the daemon has had long enough to update the store.
    fn migration_overdue(&self) -> bool {
        self.migrating
            .as_ref()
            .is_some_and(|(since, _)| since.elapsed() >= MIGRATION_PATIENCE)
    }

    fn render_error(&self, error: &OpenError, th: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let err = match error {
            // The daemon makes the store when the first account is added;
            // until then the first-start pages show.
            OpenError::NoStore { .. } => return div().into_any_element(),
            // Waiting on the daemon looks like the window still loading,
            // unless it has taken far longer than an update takes.
            OpenError::Migrating(_) if !self.migration_overdue() => {
                return self.render_skeleton(th, cx);
            }
            OpenError::Migrating(err) | OpenError::Other(err) => err.clone(),
        };
        let card = page_card(th)
            .child(icon("mail", th.text_faint, 64.0))
            .child(
                div()
                    .text_size(px(22.0))
                    .text_color(rgba(th.text))
                    .child("The mail store could not be opened"),
            )
            .child(
                div()
                    .max_w(px(460.0))
                    .text_size(px(14.0))
                    .line_height(px(21.0))
                    .text_color(rgba(th.text_faint))
                    .text_center()
                    .child(err),
            );
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(card)
            .into_any_element()
    }

    /// The list and, with three panes, the reading pane beside it.
    fn render_cards(&mut self, th: &Theme, available: f32, cx: &mut Context<Self>) -> AnyElement {
        let split = self.split();
        let pane_t = self.pane_spring.value().max(0.0);
        let share = self.config.mail.reading_pane_share;
        let pane_width = ((available - SPLIT_GAP) * share).max(0.0);
        let list = self.render_list_card(th, cx);
        let row = div()
            .id("cards")
            .size_full()
            .flex()
            .flex_row()
            .on_mouse_move(
                cx.listener(|this, event: &MouseMoveEvent, _, cx| this.on_split_drag(event, cx)),
            )
            .child(div().flex_1().min_w_0().h_full().child(list));
        let row = if split && pane_t > 0.001 {
            let handle = div()
                .id("split-handle")
                .flex_none()
                .w(px(SPLIT_GAP * pane_t.min(1.0)))
                .h_full()
                .flex()
                .items_center()
                .justify_center()
                .cursor_col_resize()
                .group("split")
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, event: &gpui::MouseDownEvent, _, cx| {
                        this.split_drag =
                            Some((unpx(event.position.x), this.config.mail.reading_pane_share));
                        cx.stop_propagation();
                    }),
                )
                .child(
                    div()
                        .w(px(4.0))
                        .h(px(40.0))
                        .rounded_full()
                        .bg(rgba(th.divider))
                        .group_hover("split", |s| s.bg(rgba(th.text_faint))),
                );
            // Clipped a little outside the card on every side, so the
            // card's shadow is never cut; the row stretches it to its height
            // plus that room.
            let room = crate::widgets::CARD_SHADOW_ROOM;
            let pane = div()
                .flex_none()
                .w(px(pane_width * pane_t + 2.0 * room))
                .m(px(-room))
                .p(px(room))
                .overflow_hidden()
                .child(
                    div()
                        .w(px(pane_width))
                        .h_full()
                        .ml(px(24.0 * (1.0 - pane_t.min(1.0))))
                        .opacity(pane_t.min(1.0))
                        .child(self.render_reader_card(th, cx)),
                );
            row.child(handle).child(pane)
        } else {
            row
        };
        // Padding, not margins: a margin would push the card past the window.
        let margin = self.layout.shape.card_margin();
        div()
            .flex_1()
            .min_w_0()
            .h_full()
            .pr(px(margin))
            .pb(px(margin))
            .flex()
            .flex_row()
            .child(div().flex_1().min_w_0().h_full().child(row))
            .children(self.render_contact_panel(th, cx))
            .children(self.render_agenda_panel(th, cx))
            .into_any_element()
    }
}

impl MailWindow {
    /// A line of text is centred on the middle of its ascent and descent,
    /// which puts most names below the middle of a short pill: with a long
    /// descent, as in Noto Sans, a pixel or two. Measures how far to lift
    /// it so the middle of its small and capital letters is centred.
    fn measure_pill_text(&mut self, window: &Window) {
        let family = self
            .font
            .clone()
            .unwrap_or_else(|| window.text_style().font_family.clone());
        let text = window.text_system();
        let id = text.resolve_font(&gpui::font(family));
        let size = px(100.0);
        // A font without its letter heights (an old OS/2 table, as in
        // DejaVu Sans) gets the usual ones for a sans serif.
        let height = |metric: gpui::Pixels, usual: f32| {
            let metric = unpx(metric);
            if metric > 0.0 {
                metric
            } else {
                usual * unpx(size)
            }
        };
        let x_height = height(text.x_height(id, size), 0.55);
        let cap_height = height(text.cap_height(id, size), 0.73);
        let [ascent, descent] = [text.ascent(id, size), text.descent(id, size)].map(unpx);
        // Fonts give the descent either way up.
        let lift = (ascent - descent.abs()) / 2.0 - (x_height + cap_height) / 4.0;
        self.pill_text_lift = (lift / unpx(size)).clamp(0.0, 0.2);
    }

    /// How far text of `size` goes up to look centred in a pill.
    pub(super) fn pill_lift(&self, size: f32) -> f32 {
        self.pill_text_lift * size
    }
}

impl Render for MailWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Text without a size of its own follows Settings > Appearance > Scaling.
        window.set_rem_size(px(16.0));
        // Before anything draws text that can be selected.
        self.ui_text.begin_window(window);
        self.chrome.sync_look(window, cx);
        self.track_menu_fade(cx);
        if self.detached {
            let detached = self.render_detached(window, cx);
            self.fetch_pictures(cx);
            return detached;
        }
        self.tour_new_frame();
        self.measure_pill_text(window);
        let th = self.theme(window);
        self.release_images(window, cx);
        if let Some(viewer) = &self.files.viewer {
            let corners = self.chrome.content_corners(window);
            viewer.update(cx, |viewer, _| {
                viewer.th = th;
                viewer.corners = corners;
            });
        }
        let reduce = cx.reduce_motion();
        self.update_layout(window, reduce, cx);
        let shape = self.layout.shape;
        let width = shape.width;

        // The contact panel folds the folders when it needs their room.
        let settings_room = if self.settings_open && !shape.is_phone() {
            SETTINGS_WIDTH
        } else {
            0.0
        };
        self.fold_nav_for_contact(width - shape.rail() - shape.card_margin() - settings_room);
        self.nav_spring.set(
            if self.nav_docked() || self.nav_peek || self.layout.drawer {
                1.0
            } else {
                0.0
            },
        );
        self.reserve_spring
            .set(if self.nav_docked() { 1.0 } else { 0.0 });
        self.page_side_spring
            .set(if self.page_side_open() { 1.0 } else { 0.0 });
        self.page_side_t = self.page_side_spring.tick(window, reduce);
        let search_focused = self.search.focus_handle(cx).is_focused(window);
        self.search_spring
            .set(if search_focused { 1.0 } else { 0.0 });
        // Keys that lost their place (a message sent or closed, a dialog
        // gone) come back to the list, so its keys work without a click.
        let dialog_gone = self.dialog_focus.is_focused(window)
            && self.delete_ask.is_none()
            && self.new_label.is_none()
            && self.rule_editor.is_none()
            && self.add_account.is_none()
            && self.danger.is_none();
        if dialog_gone || window.focused(cx).is_none() {
            match &self.settings_page {
                Some(page) => window.focus(&page.focus, cx),
                None if self.mail.is_ok() => window.focus(&self.list_focus, cx),
                None => {}
            }
        }
        let pane_open = self.pane_open();
        self.pane_spring.set(if pane_open { 1.0 } else { 0.0 });
        self.reader_keys = pane_open && self.reader_focus.contains_focused(window, cx);
        self.keys_spring
            .set(if self.reader_keys { 1.0 } else { 0.0 });
        self.nav_keys_shown = self.nav_focus.is_focused(window);
        self.settings_spring
            .set(if self.settings_open { 1.0 } else { 0.0 });
        self.search_panel_spring
            .set(if self.search_panel.is_some() {
                1.0
            } else {
                0.0
            });
        self.nav_t = self.nav_spring.tick(window, reduce);
        let reserve = self.reserve_spring.tick(window, reduce);
        let search_t = self.search_spring.tick(window, reduce);
        let pane_t = self.pane_spring.tick(window, reduce);
        self.keys_t = self.keys_spring.tick(window, reduce);
        let settings_t = self.settings_spring.tick(window, reduce);
        self.search_panel_spring.tick(window, reduce);
        self.measure_tabs(window);
        self.tick_tab_chips(window, reduce);
        self.tick_reorder(window, reduce, cx);
        self.tick_nav_fold(window, reduce);
        self.sync_nav_list();
        // Forget the closed conversation once its pane has slid away.
        if !self.reading
            && self.reader.is_some()
            && if self.split() {
                pane_t <= 0.0 && self.pane_spring.settled()
            } else {
                self.slides() && self.page_closed()
            }
        {
            self.reader = None;
        }

        let accent: Hsla = rgba(th.accent).into();
        self.search
            .update(cx, |search, _| search.set_accent(accent));
        self.sync_search_box(cx);

        // Widths: the cards get what the navigation and settings leave. On a
        // phone the settings float over the cards instead.
        let nav_width = NAV_WIDTH * reserve.max(0.0);
        let settings_floats = shape.is_phone();
        let settings_width = if settings_floats {
            0.0
        } else {
            SETTINGS_WIDTH * settings_t.clamp(0.0, 1.0)
        };
        let available =
            (width - shape.rail() - nav_width - shape.card_margin() - settings_width).max(200.0);
        // The contact panel takes its room from the list and the reader.
        let (contact_room, contact_target) = self.tick_contact(available, window, reduce);
        let (agenda_room, agenda_target) = self.tick_agenda(available, window, reduce);
        let available = (available - contact_room - agenda_room).max(200.0);
        self.cards_width = available;
        // The inbox tabs fold to fit the list's new width.
        self.tab_fold.set(self.tabs_fold_target());
        self.tab_fold.tick(window, reduce);
        let reader_width = if self.split() {
            ((available - SPLIT_GAP) * self.config.mail.reading_pane_share).max(0.0)
        } else {
            available
        };
        self.update_reply_row(reader_width, window, reduce);
        let (rail, margin) = if shape.is_phone() {
            (0.0, 0.0)
        } else {
            (apps::APP_RAIL_WIDTH, CARD_GAP)
        };
        let nav = if self.nav_docked() { NAV_WIDTH } else { 0.0 };
        let settings = if self.settings_open && !settings_floats {
            SETTINGS_WIDTH
        } else {
            0.0
        };
        self.cards_target =
            (width - rail - nav - margin - settings - contact_target - agenda_target).max(200.0);

        let settings = (settings_t > 0.001).then(|| self.render_settings(&th, settings_t, cx));
        let (docked_settings, floating_settings) = if settings_floats {
            (None, settings)
        } else {
            (settings, None)
        };
        if self.onboarding.is_none() && self.needs_account() {
            self.onboarding = Some(onboarding::Onboarding::new());
        }
        // The Settings page stays reachable, for example to delete data.
        let onboarding = self.onboarding() && self.settings_page.is_none();
        self.compose_shown.set(
            if self.mail.is_ok() && !self.accounts.is_empty() && !onboarding {
                1.0
            } else {
                0.0
            },
        );
        self.compose_shown.tick(window, reduce);
        // The big button heads the folders or the page's side column while
        // it is open beside the page, and waits in the rail otherwise.
        self.compose_dock
            .set(if self.nav_docked() { 1.0 } else { 0.0 });
        self.compose_dock.tick(window, reduce);
        self.reader_bar.tick(&self.reader_scroll, window, cx);
        self.title_roll.tick(window, reduce);
        let (primary_icon, primary_label) = self.primary_button();
        if self.primary_label.is_empty() {
            // The first frame shows the button as it is, with no turn.
            self.primary_icon = primary_icon;
            self.primary_label = primary_label;
        } else if primary_icon != self.primary_icon || primary_label != self.primary_label {
            self.primary_icon_from = self.primary_icon;
            self.primary_icon = primary_icon;
            self.primary_label_from = std::mem::replace(&mut self.primary_label, primary_label);
            self.primary_icon_turn.snap(0.0);
            self.primary_icon_turn.set(1.0);
        }
        let turn = self.primary_icon_turn.tick(window, reduce).clamp(0.0, 1.0);
        let word = |label: &str| compose_text_width(label, self.font.as_ref(), window);
        self.primary_label_width = if turn >= 0.999 {
            word(&self.primary_label)
        } else {
            lerp(
                word(&self.primary_label_from),
                word(&self.primary_label),
                turn,
            )
        };
        self.avatar_turn.tick(window, reduce);
        let compose_text = self.primary_label_width;
        let content = match &self.mail {
            _ if onboarding => self.render_onboarding(&th, window, cx),
            Err(err) => self.render_error(err, &th, cx),
            // Reversed so the navigation paints last, over the cards, when
            // it opens from the rail.
            Ok(_) if self.app == RailApp::Mail => div()
                .size_full()
                .flex()
                .flex_row_reverse()
                .children(docked_settings)
                .child(if self.settings_page.is_some() {
                    self.render_settings_page(&th, window, cx)
                } else {
                    self.render_cards(&th, available, cx)
                })
                .child(self.render_navigation(&th, cx))
                .child(self.render_rail_slot(&th, cx))
                .into_any_element(),
            Ok(_) => div()
                .size_full()
                .flex()
                .flex_row_reverse()
                .children(docked_settings)
                .child(if self.settings_page.is_some() {
                    self.render_settings_page(&th, window, cx)
                } else {
                    self.render_app_page(&th, window, cx)
                })
                .child(self.render_rail_slot(&th, cx))
                .into_any_element(),
        };
        // The apps move to a bar along the bottom on a phone.
        let content = div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .child(content)
                    .children(self.render_compose_button(&th, compose_text, cx))
                    // A note opened from a mail or an event, over it.
                    .children(if matches!(self.app, RailApp::Mail | RailApp::Calendar) {
                        self.render_editor(&th, false, window, cx)
                    } else {
                        None
                    }),
            )
            .children(if onboarding {
                None
            } else {
                self.render_bottom_bar(&th, cx)
            })
            .into_any_element();
        let floating_settings = floating_settings.map(|panel| {
            div()
                .absolute()
                .top_0()
                .left_0()
                .right_0()
                .bottom(px(shape.bottom_bar()))
                .overflow_hidden()
                .child(panel)
                .into_any_element()
        });

        // On a desktop the search box starts where the list starts with the
        // folders open; with them folded it grows to the left, a clear gap
        // after the app's name, and its right end stays put. On a tablet (the folders in a drawer) it starts a
        // clear gap after the app's name beside the menu button, and it
        // never comes closer than that. It grows into a pill across the top
        // bar of a phone, under its menu button and account picture.
        let (room_start, room_end) = shape.room;
        let titles = title_widths(self.font.as_ref(), window);
        let after_title =
            room_start + TITLE_LEFT + title_width(shape.title_label(), titles) + TOP_BAR_GAP;
        let list_left = if shape.is_desktop() {
            shape.rail() + NAV_WIDTH
        } else {
            0.0
        };
        let open_left = list_left.max(after_title);
        // The Activity button after the box, with the gap before it.
        let activity_room = if self.activity_shown() && shape.phone < 0.5 {
            ACTIVITY_BUTTON_WIDTH + 8.0
        } else {
            0.0
        };
        // The box gives way first, so the buttons after it keep their
        // gaps and never overlap.
        // The agenda button before Settings, with its gap.
        let agenda_room = if self.agenda_button_shown() {
            agenda::AGENDA_BUTTON_WIDTH + TOP_BAR_GAP
        } else {
            0.0
        };
        let room = width - open_left - room_end - TOP_END_WIDTH - agenda_room - TOP_BAR_GAP;
        // Too narrow for both (wider than a phone, with wide window
        // buttons): the button goes rather than cover the agenda button.
        let activity_fits = room - activity_room >= SEARCH_MIN_WIDTH;
        let open_width = (room - if activity_fits { activity_room } else { 0.0 })
            .clamp(SEARCH_MIN_WIDTH, SEARCH_WIDTH);
        let search_left = if shape.is_desktop() {
            // The title's box has room for the longest app name; the box
            // comes up to the name shown.
            let shown = text_width(
                &self.app.label(),
                TITLE_TEXT_SIZE,
                FontWeight::NORMAL,
                self.font.as_ref(),
                window,
            );
            let after_name = room_start
                + TITLE_LEFT
                + title_width(shape.title_label(), (titles.0, shown))
                + TOP_BAR_GAP;
            lerp(after_name, open_left, reserve.clamp(0.0, 1.0))
        } else {
            open_left
        };
        let regular = open_width + open_left - search_left;
        let pill = (width - 12.0 - room_start - room_end).max(200.0);
        let search_width = lerp(regular, pill, shape.phone);
        // Search options: under the box and at least as wide, widened
        // in a narrow window and kept inside it.
        let panel_width = regular.max(search_panel::MIN_WIDTH).min(width - 16.0);
        let panel_left = search_left.min(width - 8.0 - panel_width).max(8.0);
        let search_panel_width = lerp(panel_width, width - 16.0, shape.phone);
        let search_panel_left = lerp(panel_left, 8.0, shape.phone);
        // The window's inner height: its frame takes as much at the top
        // and bottom as at the sides.
        let viewport = window.viewport_size();
        let inner_height = unpx(viewport.height) - (unpx(viewport.width) - width);
        let search_panel_height = (inner_height - TOP_BAR_HEIGHT - 12.0).max(200.0);
        let search_panel = self
            .render_search_panel(
                &th,
                search_panel_left,
                search_panel_width,
                search_panel_height,
                window,
                cx,
            )
            .map(|panel| self.search_panel_layer(panel, cx));
        let fab = if onboarding {
            None
        } else {
            self.render_phone_fab(&th, cx)
        };
        let compose = self.render_compose(&th, window, reduce, cx);
        let compose_dialog = self.render_docked_compose_dialog(&th, window, cx);
        // The attach picker is over Compose, and the viewer over it to
        // look at its files.
        let files_picker = self.render_files_picker(&th, window, cx);
        let viewer_over = files_picker.is_some()
            || self.files.viewer_place == attachments::ViewerPlace::OverCompose;
        // A viewer opened from the popped-out message shows there instead.
        let in_window = self.files.viewer_place != attachments::ViewerPlace::Popout;
        let scheduled = self.render_scheduled(&th, window, cx);
        let activity = self.render_activity_report(&th, window, cx);
        let activity_menu = self.render_activity_menu(&th, window, cx);
        let account_menu = self.render_account_menu(&th, cx);
        let language_picker = self.render_language_picker(&th, window, cx);
        let add_account = self.render_add_account(&th, window, reduce, cx);
        let danger = self.render_danger(&th, window, reduce, cx);
        let delete_ask = self.render_delete_ask(&th, window, reduce, cx);
        let new_label = self.render_new_label(&th, window, reduce, cx);
        let rule_editor = self.render_rule_editor(&th, window, reduce, cx);
        self.ready_folder_pick(&th, window, cx);
        let contact_label = self.render_label_dialog(&th, window, reduce, cx);
        let scheme_editor = self.render_scheme_editor(&th, window, reduce, cx);
        let contact_qr = self.render_contact_qr(&th, window, reduce, cx);
        let whats_new = self.render_whats_new(&th, window, reduce, cx);
        let shortcuts_dialog = self.render_shortcuts_dialog(&th, window, reduce, cx);
        let share_ask = if onboarding {
            None
        } else {
            self.render_share_ask(&th, window, reduce, cx)
        };
        let about = self.render_about(&th, window, reduce, cx);
        let gallery = self.render_gallery(cx);
        let update_dialog = self.render_update_dialog(&th, window, reduce, cx);
        let print_preview = self.render_print_preview(&th, window, reduce, cx);
        let context_menu = self.render_context_menu(&th, window, cx);
        let summary_peek = self.render_summary_peek(&th, window, cx);
        let contact_sheet = self.render_contact_sheet(&th, window, cx);
        let contact_peek = self.render_contact_peek(&th, window, cx);
        let nav_menu = self.render_nav_menu(&th, cx);
        // An account's own color, from Settings > Accounts or its
        // right-click menu.
        let account_picker = self.render_color_picker(
            |t| matches!(t, scheme_color::Target::Account(_)),
            &th,
            window,
            cx,
        );
        let snooze_menu = self.render_snooze_menu(&th, cx);
        let quiet_menu = self.render_quiet_menu(&th, cx);
        let snackbar = self.render_snackbar(&th, window, reduce, cx);
        let upload_tray = self.render_upload_tray(&th, window, cx);
        let drive_share = self.render_share_dialog(&th, window, reduce, cx);
        let crash_notice = if onboarding {
            None
        } else {
            self.render_crash_notice(&th, window, reduce, cx)
        };
        let sign_in_again = if onboarding {
            None
        } else {
            self.render_sign_in_again(&th, window, reduce, cx)
        };
        let tour = self.render_tour(&th, window, cx);
        // GPUI does not clip to the frame's rounded corners, so the
        // backdrop rounds its own bottom ones.
        let (bottom_left, bottom_right) = self.chrome.content_corners(window);
        let ui_text_menu = self.render_ui_text_menu(&th, window, cx);
        let content = div()
            .key_context(WINDOW_CONTEXT)
            .map(|d| self.ui_text_root(d, cx))
            .children(ui_text_menu)
            .relative()
            .size_full()
            .bg(rgba(th.backdrop))
            .rounded_bl(px(bottom_left))
            .rounded_br(px(bottom_right))
            .text_color(rgba(th.text))
            // Where the menu bar's actions start when the focus is lost.
            .child(div().absolute().size_0().track_focus(&self.window_focus))
            .child(content)
            .children(floating_settings)
            .children(fab)
            .children(
                self.files
                    .viewer
                    .clone()
                    .filter(|_| in_window && !viewer_over),
            )
            .children(search_panel)
            .children(compose)
            .children(compose_dialog)
            .children(files_picker)
            .children(
                self.files
                    .viewer
                    .clone()
                    .filter(|_| in_window && viewer_over),
            )
            .children(scheduled)
            .children(activity)
            .children(activity_menu)
            .children(account_menu)
            .children(language_picker)
            .children(add_account)
            .children(summary_peek)
            .children(context_menu)
            .children(contact_sheet)
            .children(contact_peek)
            .children(nav_menu)
            .children(snooze_menu)
            .children(quiet_menu)
            .children(danger)
            .children(delete_ask)
            .children(new_label)
            .children(rule_editor)
            .children(contact_label)
            .children(scheme_editor)
            .children(account_picker)
            .children(contact_qr)
            .children(crash_notice)
            .children(sign_in_again)
            .children(whats_new)
            .children(shortcuts_dialog)
            .children(share_ask)
            .children(about)
            .children(gallery)
            .children(update_dialog)
            .children(print_preview)
            .children(upload_tray)
            .children(drive_share)
            .children(snackbar)
            .children(tour)
            .into_any_element();

        // The first-start pages keep the bar empty.
        let bar = Bar {
            start: if onboarding {
                self.skeleton_top_start(&th, titles)
            } else {
                self.render_top_start(&th, titles, cx)
            },
            center: if onboarding {
                Some(
                    div()
                        .w_full()
                        .pl(px(lerp(search_left, 6.0 + room_start, shape.phone)))
                        .child(skeleton::search_pill(&th, search_width, shape.phone))
                        .into_any_element(),
                )
            } else {
                self.mail.is_ok().then(|| {
                    div()
                        .w_full()
                        .pl(px(lerp(search_left, 6.0 + room_start, shape.phone)))
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(8.0))
                        .child(self.render_search(&th, search_width, search_t, window, cx))
                        .when(shape.phone < 0.5 && activity_fits, |d| {
                            d.children(self.render_activity_button(&th, cx))
                        })
                        .into_any_element()
                })
            },
            end: if onboarding {
                Vec::new()
            } else {
                self.render_top_end(&th, cx)
            },
            height: Some(TOP_BAR_HEIGHT),
            background: Some(th.backdrop),
        };
        // Pictures of people asked for while drawing.
        self.fetch_pictures(cx);
        self.fetch_saved_photos(cx);
        let frame = self.chrome.render_bar(bar, content, window, cx);
        // A phone's top bar slides up out of the window as the list moves
        // on; the content below takes its room.
        let hidden = shape.top_bar_hidden();
        let frame = if hidden > 0.01 {
            div().size_full().overflow_hidden().child(
                frame
                    .mt(px(-hidden))
                    .h(window.viewport_size().height + px(hidden)),
            )
        } else {
            frame
        };
        // The window's actions sit on its outermost element, so they run
        // wherever the keyboard focus is: in the top bar, or on something
        // that is no longer drawn (the list while Settings or a
        // conversation fills the page), where GPUI starts from the root.
        let frame = frame
            // A press anywhere that takes no keys itself (the top bar's
            // empty room, a gap between panes) still takes them from the
            // search box, as a press on the list does.
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    if std::mem::take(&mut this.search_pressed)
                        || window.default_prevented()
                        || !this.search.focus_handle(cx).is_focused(window)
                    {
                        return;
                    }
                    match &this.settings_page {
                        Some(page) => window.focus(&page.focus, cx),
                        None if this.mail.is_ok() => window.focus(&this.list_focus, cx),
                        None => window.focus(&this.window_focus, cx),
                    }
                    cx.notify();
                }),
            )
            .on_action(cx.listener(Self::focus_next))
            .on_action(cx.listener(Self::focus_previous))
            .on_action(cx.listener(Self::next_pane))
            .on_action(cx.listener(Self::previous_pane))
            .on_action(cx.listener(Self::focus_search))
            .on_action(cx.listener(Self::focus_list))
            .on_action(cx.listener(Self::list_top))
            .on_action(cx.listener(Self::toggle_navigation))
            .on_action(cx.listener(Self::toggle_settings))
            .on_action(cx.listener(Self::compose))
            .on_action(cx.listener(Self::reload))
            .on_action(cx.listener(Self::quit))
            .on_action(cx.listener(Self::reply))
            .on_action(cx.listener(Self::reply_all))
            .on_action(cx.listener(Self::forward))
            .on_action(cx.listener(Self::move_to))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::select_none))
            .on_action(cx.listener(Self::undo_action))
            .on_action(cx.listener(Self::go_to_inbox))
            .on_action(cx.listener(Self::go_to_starred))
            .on_action(cx.listener(Self::go_to_sent))
            .on_action(cx.listener(Self::go_to_drafts))
            .on_action(cx.listener(Self::go_to_all_mail))
            .on_action(cx.listener(|this, _: &ShowMail, window, cx| {
                this.show_page(RailApp::Mail, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ShowCalendar, window, cx| {
                this.show_page(RailApp::Calendar, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ShowContacts, window, cx| {
                this.show_page(RailApp::Contacts, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ShowTasks, window, cx| {
                this.show_page(RailApp::Tasks, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ShowNotes, window, cx| {
                this.show_page(RailApp::Notes, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ShowFiles, window, cx| {
                this.show_page(RailApp::Files, window, cx)
            }))
            .on_action(cx.listener(Self::open_settings))
            .on_action(cx.listener(Self::show_shortcuts))
            .on_action(cx.listener(Self::show_whats_new_action))
            .on_action(cx.listener(Self::check_for_updates_action))
            .on_action(cx.listener(Self::show_about));
        match &self.font {
            Some(font) => frame.font_family(font.clone()).into_any_element(),
            None => frame.into_any_element(),
        }
    }
}

impl Focusable for MailWindow {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.list_focus.clone()
    }
}

/// The card that fills the page for the welcome and error pages.
/// What moves `ids` back after a move to `to`: each goes back to the
/// folder the daemon takes it out of (not `to`, and not All Mail when it
/// is somewhere else too, as `katna_sync::ops::move_messages` picks it).
/// `None` when none of them moves.
fn move_back(
    mail: &Mail,
    all_mail: Option<FolderId>,
    ids: &[MessageId],
    to: Option<FolderId>,
) -> Option<Command> {
    let mut back: Vec<(FolderId, Vec<MessageId>)> = Vec::new();
    for &id in ids {
        let Some(from) = mail
            .message_folders(id)
            .into_iter()
            .filter(|f| Some(*f) != to)
            .min_by_key(|f| Some(*f) == all_mail)
        else {
            continue;
        };
        match back.iter_mut().find(|(folder, _)| *folder == from) {
            Some((_, ids)) => ids.push(id),
            None => back.push((from, vec![id])),
        }
    }
    let mut moves: Vec<Command> = back
        .into_iter()
        .map(|(folder, ids)| Command::Move(ids, folder))
        .collect();
    match moves.len() {
        0 => None,
        1 => moves.pop(),
        _ => Some(Command::Several(moves)),
    }
}

fn page_card(th: &Theme) -> gpui::Div {
    div()
        .flex_1()
        .min_h_0()
        .mx(px(16.0))
        .mb(px(16.0))
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(12.0))
        .map(|d| crate::widgets::card(d, th, th.pane(), PANEL_RADIUS, 0.0))
}

/// The undo-send countdown: a ring `size` wide in `color` on `track`,
/// whose line runs back as time passes, `share` of it left, with the
/// `seconds` left inside.
fn countdown_ring(share: f32, seconds: u64, color: u32, track: u32, size: f32) -> AnyElement {
    div()
        .relative()
        .flex_none()
        .size(px(size))
        .flex()
        .items_center()
        .justify_center()
        // The line left runs from the top clockwise and shrinks back
        // towards it.
        .child(crate::widgets::ring(
            1.0 - share.min(1.0),
            1.0,
            color,
            track,
            size,
        ))
        .child(
            div()
                .text_size(px(size * 0.43))
                .font_weight(gpui::FontWeight::MEDIUM)
                .child(katna_i18n::format::number(seconds)),
        )
        .into_any_element()
}
