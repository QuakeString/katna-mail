// SPDX-License-Identifier: GPL-3.0-or-later

//! Quick settings: a panel that slides in from the right with the reading
//! pane (three or two panes), density, theme, app names, inbox tabs, undo
//! send, the signature, conversation view, the tour, What's new and About.
//! Changes apply at once and are saved to `config.toml`.

use std::time::Duration;

use gpui::{
    Animation, AnimationExt, AnyElement, App, Context, Div, FontWeight, SharedString,
    SpringAnimation, Stateful, div, prelude::*, rgba,
};
use katna_core::config::{
    AccountsShown, AutoAdvance, Clock, Density, FileGroup, FilesPage, MarkRead, OpenIn,
    ReadingPane, ReduceMotion, SoundEvent, Theme as ThemeChoice, TrayStyle, UNDO_SEND_CHOICES,
    WindowFrame,
};
use katna_i18n::tr;
use katna_ui::Ripple;
use katna_ui::motion;
use katna_ui::px;

use super::{CARD_GAP, MailWindow, SETTINGS_WIDTH};
use crate::schemes;
use crate::theme::{Accent, Theme, mix};
use crate::widgets::FocusRing;
use crate::widgets::{
    CARD_SHADOW_ROOM, ScaledEdge, card_outline, icon, icon_button, radio, switch, tip,
};

/// One loop of the reading-pane demo.
const PANE_DEMO: Duration = Duration::from_millis(2600);

/// What a quick setting changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Change {
    UndoSend(u32),
    /// Where writing help with AI goes (`ai.source`).
    AiSource(katna_core::config::AiSource),
    /// The user's own AI service, by `katna_ai::provider::Preset::id`.
    AiProvider(&'static str),
    /// AI finishes sentences (`ai.autocomplete`).
    AiAutocomplete(bool),
    /// Those suggestions also send the mail being answered.
    AiAnswered(bool),
    /// Rephrase is offered for encrypted mail.
    AiEncrypted(bool),
    /// An event's sound on or off.
    Sound(SoundEvent, bool),
    /// The sound an event plays, by its `katna_platform::sound` name;
    /// empty for its set's.
    SoundChoice(SoundEvent, &'static str),
    /// The set of sounds, by `katna_platform::sound::Set::id`.
    SoundSet(&'static str),
    Pane(ReadingPane),
    Density(Density),
    Theme(ThemeChoice),
    /// The desktop's color scheme (on), or Katna's own (off).
    DesktopColors(bool),
    /// The color scheme, by id (`crate::schemes`).
    Colors(&'static str),
    Accent(Accent),
    Tabs(bool),
    Conversations(bool),
    AppLabels(bool),
    SenderPictures(bool),
    NewestFirst(bool),
    FullHeaders(bool),
    FullNames(bool),
    SingleKeys(bool),
    OpenIn(FileGroup, OpenIn),
    AccountsShown(AccountsShown),
    /// The unified inbox over the accounts in the folder pane.
    UnifiedInbox(bool),
    /// An account is in the unified inbox (`true`) or only in the account
    /// card.
    InUnified(katna_core::AccountId, bool),
    /// The tray icon, shown by the daemon.
    Tray(bool),
    TrayStyle(TrayStyle),
    /// The unread count on the taskbar icon, shown by the daemon.
    UnreadBadge(bool),
    /// Katna's own window frame, or the desktop's.
    WindowFrame(WindowFrame),
    /// The blurred, translucent window background.
    Blur(bool),
    /// Frosted menus, popovers, dialogs and viewer bars.
    FrostedPopups(bool),
    /// The frost's blur and opacity set by hand, or following the desktop.
    CustomFrost(bool),
    /// The frost's blur, in pixels.
    FrostBlur(u8),
    /// The frost's tint opacity, in percent.
    FrostOpacity(u8),
    /// The corner radius of Katna's frame, in pixels.
    WindowRadius(u8),
    /// The line around Katna's frame.
    WindowBorder(bool),
    /// That line's opacity, in percent.
    WindowBorderOpacity(u8),
    /// In a blurred window, the cards let the blur through.
    FrostedPanes(bool),
    /// How opaque those cards are, in percent.
    PaneOpacity(u8),
    /// In a blurred window, the room behind a chat's bubbles lets the
    /// blur through.
    FrostedChat(bool),
    /// In a blurred window, the open search box lets the blur through.
    FrostedSearch(bool),
    FrostedHeaders(bool),
    /// Conversations between people open as a group chat.
    ChatView(bool),
    /// Days of mail the daemon downloads ahead of time; 0 for all mail.
    OfflineDays(u32),
    /// Crash reports written on this computer (Settings > User feedback).
    SaveCrashReports(bool),
    /// Crash reports sent to Katna's crash tracker: "Help improve Katna".
    SendCrashReports(bool),
    /// The interface scale, in percent.
    Scale(u16),
    /// Katna's own animation speed, as a percentage of normal length, or
    /// `None` for the desktop's.
    AnimationSpeed(Option<u16>),
    /// Whether animations are turned off.
    ReduceMotion(ReduceMotion),
    /// 12- or 24-hour times.
    Clock(Clock),
    /// How tall the hours of Calendar's Day and Week are.
    CalendarDensity(katna_core::config::CalendarDensity),
    /// How many days Calendar's custom view shows.
    CustomDays(u8),
    /// The Birthdays calendar shows.
    Birthdays(bool),
    /// An account is shown in an app (`true`) or left out of it.
    AppAccount(katna_core::config::AppKind, katna_core::AccountId, bool),
    /// An account is connected (`true`) or taken offline until brought
    /// back.
    AccountOnline(katna_core::AccountId, bool),
    /// What Katna starts at login, if anything (an autostart entry).
    StartAtLogin(Option<crate::autostart::Start>),
    MarkRead(MarkRead),
    /// What opens after the open conversation is moved away.
    AutoAdvance(AutoAdvance),
    /// Ask before deleting two or more conversations.
    ConfirmDelete(bool),
    RemoteImages(bool),
    ReplyAll(bool),
    ImportantMarkers(bool),
    /// Settings > Folders & rules: an unread count beside every folder.
    FolderUnreadCounts(bool),
    LimitWidth(bool),
    DarkMail(bool),
    AttachmentPreviews(bool),
    OpenSavedFolder(bool),
    /// The Files page leaves out small pictures (signature logos).
    LeaveOutSmallPictures(bool),
    /// Pictures under this many KB are small.
    SmallPictureKb(u32),
    /// Pictures under this many pixels wide or tall are small.
    SmallPicturePx(u32),
    /// Files and the attach pickers show the drive of an account (its
    /// store id).
    DriveInFiles(i64, bool),
    /// New-mail notifications, shown by the daemon.
    NewMailNotices(bool),
    /// New versions of Katna downloaded as soon as the daemon finds them.
    AutoDownloadUpdates(bool),
    PlainText(bool),
    SpellCheck(bool),
    /// The interface's language, a tag; empty follows the desktop.
    Language(&'static str),
    /// Grammar mistakes underlined while writing (English only).
    GrammarCheck(bool),
    WritingSuggestions(bool),
    /// Offer to translate mail in other languages.
    TranslateOffer(bool),
    /// A language (a LibreTranslate code) always translated, or no longer.
    TranslateAlways(&'static str, bool),
    /// A language never offered for translation, or offered again.
    TranslateNever(&'static str, bool),
    /// The language mail is translated into, a tag; empty follows the
    /// interface.
    ReadingLanguage(&'static str),
}

impl MailWindow {
    pub(super) fn render_settings(&self, th: &Theme, t: f32, cx: &mut Context<Self>) -> AnyElement {
        let view = &self.config.mail;
        // On a phone the panel is a page of its own, over the whole window
        // below the top bar.
        let phone = self.layout.shape.is_phone();
        let inner = SETTINGS_WIDTH - CARD_GAP;
        let panel = div()
            .id("settings")
            .map(|d| if phone { d.w_full() } else { d.w(px(inner)) })
            .h_full()
            .flex()
            .flex_col()
            .relative()
            .map(|d| {
                let (radius, shadow) = if phone {
                    (0.0, 0.0)
                } else {
                    (
                        super::PANEL_RADIUS,
                        t.min(1.0) * self.layout.shape.card_outline(),
                    )
                };
                crate::widgets::card(d, th, th.pane(), radius, shadow)
            })
            .child(
                div()
                    .flex_none()
                    .h(px(56.0))
                    .pl(px(20.0))
                    .pr(px(8.0))
                    .flex()
                    .flex_row()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(16.0))
                            .font_weight(FontWeight::MEDIUM)
                            .child(tr!("quick-title")),
                    )
                    .child(
                        icon_button("settings-close", "close", 20.0, th)
                            .tooltip(tip(tr!("reader-close"), th))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.toggle_settings(&super::ToggleSettings, window, cx)
                            })),
                    ),
            )
            .child(
                div()
                    .id("settings-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    // One block that keeps its height, so the rows are
                    // scrolled rather than squeezed.
                    .child(
                        div()
                            .flex_none()
                            .px(px(20.0))
                            .pb(px(20.0))
                            .flex()
                            .flex_col()
                            .gap(px(4.0))
                            .child(
                                div().pt(px(4.0)).pb(px(8.0)).flex().child(
                                    crate::widgets::outlined_button(
                                        "see-all-settings",
                                        tr!("quick-see-all"),
                                        th,
                                    )
                                    .flex_1()
                                    .justify_center()
                                    .on_click(cx.listener(
                                        |this, _, window, cx| this.open_settings_here(window, cx),
                                    )),
                                ),
                            )
                            .child(heading(tr!("quick-reading-pane"), th))
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .gap(px(12.0))
                                    .child(self.pane_choice(
                                        ReadingPane::Right,
                                        tr!("quick-pane-right"),
                                        th,
                                        cx,
                                    ))
                                    .child(self.pane_choice(
                                        ReadingPane::None,
                                        tr!("quick-pane-none"),
                                        th,
                                        cx,
                                    )),
                            )
                            .child(divider(th))
                            .child(heading(tr!("quick-density"), th))
                            .child(self.radio_row(
                                "density-default",
                                tr!("quick-density-default"),
                                view.density == Density::Default,
                                Change::Density(Density::Default),
                                th,
                                cx,
                            ))
                            .child(self.radio_row(
                                "density-compact",
                                tr!("quick-density-compact"),
                                view.density == Density::Compact,
                                Change::Density(Density::Compact),
                                th,
                                cx,
                            ))
                            .child(divider(th))
                            .child(heading(tr!("quick-theme"), th))
                            .children(
                                [
                                    (ThemeChoice::System, "theme-system", "quick-theme-system"),
                                    (ThemeChoice::Light, "theme-light", "quick-theme-light"),
                                    (ThemeChoice::Dark, "theme-dark", "quick-theme-dark"),
                                ]
                                .map(|(choice, id, label)| {
                                    self.radio_row(
                                        id,
                                        tr!(label),
                                        view.theme == choice,
                                        Change::Theme(choice),
                                        th,
                                        cx,
                                    )
                                }),
                            )
                            .child(self.switch_row(
                                "desktop-colors",
                                tr!("quick-desktop-colors"),
                                tr!("quick-desktop-colors-detail"),
                                view.colors() == schemes::SYSTEM,
                                Change::DesktopColors(view.colors() != schemes::SYSTEM),
                                th,
                                cx,
                            ))
                            .child(self.switch_row(
                                "app-labels",
                                tr!("quick-app-names"),
                                tr!("quick-app-names-detail"),
                                view.app_labels,
                                Change::AppLabels(!view.app_labels),
                                th,
                                cx,
                            ))
                            .child(divider(th))
                            .child(heading(tr!("folder-inbox"), th))
                            .child(self.switch_row(
                                "tabs",
                                tr!("quick-inbox-tabs"),
                                tr!("quick-inbox-tabs-detail"),
                                view.inbox_tabs,
                                Change::Tabs(!view.inbox_tabs),
                                th,
                                cx,
                            ))
                            .child(self.link_row(
                                "quick-tabs",
                                tr!("quick-choose-tabs"),
                                tr!("quick-choose-tabs-detail").into(),
                                super::settings_page::Section::Inbox,
                                th,
                                cx,
                            ))
                            .child(divider(th))
                            .child(heading(tr!("quick-sending"), th))
                            .child(self.undo_send_choice(th, cx))
                            .child(self.link_row(
                                "quick-signatures",
                                tr!("quick-signatures"),
                                self.signature_summary(),
                                super::settings_page::Section::Signatures,
                                th,
                                cx,
                            ))
                            .child(divider(th))
                            .child(heading(tr!("quick-threading"), th))
                            .child(self.switch_row(
                                "conversations",
                                tr!("quick-conversation-view"),
                                tr!("quick-conversation-view-detail"),
                                view.conversations,
                                Change::Conversations(!view.conversations),
                                th,
                                cx,
                            ))
                            .child(divider(th))
                            .child(heading(tr!("quick-help"), th))
                            .child(
                                help_row("take-tour", "tour", tr!("quick-tour"), th).on_click(
                                    cx.listener(|this, _, window, cx| {
                                        this.start_tour(false, window, cx)
                                    }),
                                ),
                            )
                            .child(
                                help_row("whats-new", "sparkle", tr!("quick-whats-new"), th)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.show_whats_new(window, cx)
                                    })),
                            )
                            .child(
                                help_row(
                                    "check-updates",
                                    "refresh",
                                    tr!("quick-check-updates"),
                                    th,
                                )
                                .on_click(cx.listener(
                                    |this, _, window, cx| {
                                        this.check_for_updates_action(
                                            &super::CheckForUpdates,
                                            window,
                                            cx,
                                        )
                                    },
                                )),
                            )
                            .child(help_row("about", "info", tr!("quick-about"), th).on_click(
                                cx.listener(|this, _, window, cx| this.open_about(window, cx)),
                            )),
                    ),
            )
            .children(card_outline(
                th,
                super::PANEL_RADIUS,
                self.layout.shape.card_outline(),
            ));
        // The panel keeps its width and slides out from under the edge. The
        // page of a phone fades in as it comes in from the right.
        let t = t.clamp(0.0, 1.0);
        if phone {
            return div()
                .id("settings-phone")
                .occlude()
                .size_full()
                .ml(px(24.0 * (1.0 - t)))
                .opacity(t)
                .child(panel)
                .into_any_element();
        }
        // The clip reaches a little past the panel's left and top edges, so
        // its shadow is never cut.
        let room = CARD_SHADOW_ROOM;
        div()
            .flex_none()
            .h_full()
            .w(px(SETTINGS_WIDTH * t + room))
            .ml(px(-room))
            .mt(px(-room))
            .pl(px(room))
            .pt(px(room))
            .pb(px(CARD_GAP - room))
            .overflow_hidden()
            .child(
                div()
                    .h_full()
                    .pr(px(CARD_GAP))
                    .ml(px(24.0 * (1.0 - t)))
                    .opacity(t)
                    .child(panel),
            )
            .into_any_element()
    }

    /// Undo send: how long a sent message waits before it goes out.
    pub(super) fn undo_send_choice(&self, th: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let now = self.config.sending.undo_send_seconds;
        let chips = UNDO_SEND_CHOICES.into_iter().map(|seconds| {
            let on = seconds == now;
            self.page_control(div().id(("undo-send", seconds as usize)), th, cx)
                .px(px(10.0))
                .h(px(28.0))
                .flex()
                .items_center()
                .rounded(px(8.0))
                .border_1()
                .border_color(rgba(if on { th.nav_selected } else { th.outline }))
                .bg(rgba(if on { th.nav_selected } else { th.surface }))
                .text_color(rgba(if on {
                    th.nav_selected_text
                } else {
                    th.text_dim
                }))
                .text_size(px(13.0))
                .cursor_pointer()
                .hover(|s| s.bg(rgba(th.hover)))
                .on_click(
                    cx.listener(move |this, _, _, cx| this.apply(Change::UndoSend(seconds), cx)),
                )
                .child(if seconds == 0 {
                    tr!("quick-undo-send-off")
                } else {
                    tr!("quick-undo-send-seconds", seconds = seconds)
                })
        });
        div()
            .px(px(8.0))
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(div().text_size(px(14.0)).child(tr!("quick-undo-send")))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .gap(px(6.0))
                    .children(chips),
            )
            .into_any_element()
    }

    pub(super) fn apply(&mut self, change: Change, cx: &mut Context<Self>) {
        let sending = &mut self.config.sending;
        let view = &mut self.config.mail;
        let mut relist = false;
        match change {
            Change::AiSource(_)
            | Change::AiProvider(_)
            | Change::AiAutocomplete(_)
            | Change::AiAnswered(_)
            | Change::AiEncrypted(_) => {
                self.apply_ai(change, cx);
                return;
            }
            Change::Pane(pane) => {
                if view.reading_pane == pane {
                    return;
                }
                view.reading_pane = pane;
                // With a conversation open the cards change places and fade
                // in; with only the list showing nothing moves, so the list
                // stays as it is.
                if self.reading {
                    self.card_seq += 1;
                } else {
                    self.reader = None;
                }
            }
            Change::UndoSend(seconds) => sending.undo_send_seconds = seconds,
            Change::Density(density) => view.density = density,
            Change::Scale(percent) => {
                if view.scale == percent {
                    return;
                }
                view.scale = percent;
                katna_ui::scale::set_scale(f32::from(percent) / 100.0);
                // Every row is a new height, and every window a new size.
                self.list_state.remeasure();
                cx.refresh_windows();
            }
            Change::AnimationSpeed(speed) => {
                view.animation_speed = speed.map(|percent| f32::from(percent) / 100.0);
                super::colors::apply_motion(view, self.desktop_colors.motion, cx);
            }
            Change::ReduceMotion(reduce) => {
                view.reduce_motion = reduce;
                super::colors::apply_motion(view, self.desktop_colors.motion, cx);
            }
            Change::Theme(theme) => view.theme = theme,
            Change::DesktopColors(on) => {
                view.set_colors(if on { schemes::SYSTEM } else { schemes::KATNA });
            }
            Change::Colors(id) => view.set_colors(id),
            Change::Accent(accent) => view.accent = accent.setting(),
            Change::AppLabels(on) => view.app_labels = on,
            Change::SenderPictures(on) => view.sender_pictures = on,
            Change::NewestFirst(on) => view.newest_first = on,
            Change::FullHeaders(on) => view.full_headers = on,
            Change::FullNames(on) => view.full_names = on,
            Change::OpenIn(group, open) => view.open.set(group, open),
            Change::Tabs(on) => {
                view.inbox_tabs = on;
                relist = true;
            }
            Change::Conversations(on) => {
                view.conversations = on;
                relist = true;
            }
            Change::AccountsShown(shown) => {
                view.accounts_shown = shown;
                // The account on screen stays: it becomes the one shown.
                if let Some(account) = self.account() {
                    self.set_shown_account(account);
                }
                self.rebuild_nav();
            }
            Change::WindowFrame(frame) => {
                self.config.experimental.window_frame = frame;
                cx.set_global(super::look(&self.config));
            }
            Change::Blur(on) => {
                self.config.experimental.blur = on;
                cx.set_global(super::look(&self.config));
            }
            Change::FrostedPopups(on) => self.config.experimental.frosted_popups = on,
            Change::CustomFrost(on) => {
                self.config.experimental.custom_frost = on;
                cx.set_global(super::look(&self.config));
            }
            Change::FrostBlur(blur) => self.config.experimental.frost_blur = blur,
            Change::FrostOpacity(opacity) => {
                self.config.experimental.frost_opacity = opacity;
                cx.set_global(super::look(&self.config));
            }
            Change::WindowRadius(radius) => {
                self.config.experimental.window_radius = Some(radius);
                cx.set_global(super::look(&self.config));
                self.sync_radius_field(radius, cx);
            }
            Change::WindowBorder(on) => {
                self.config.experimental.window_border = on;
                cx.set_global(super::look(&self.config));
            }
            Change::WindowBorderOpacity(opacity) => {
                self.config.experimental.window_border_opacity = Some(opacity);
                cx.set_global(super::look(&self.config));
            }
            Change::FrostedPanes(on) => self.config.experimental.frosted_panes = on,
            Change::PaneOpacity(opacity) => self.config.experimental.pane_opacity = opacity,
            Change::FrostedChat(on) => self.config.experimental.frosted_chat = on,
            Change::FrostedSearch(on) => self.config.experimental.frosted_search = on,
            Change::FrostedHeaders(on) => self.config.experimental.frosted_headers = on,
            Change::ChatView(on) => {
                self.config.experimental.chat_view = on;
                self.open_chat_as_set();
            }
            Change::SaveCrashReports(on) => self.config.feedback.save_crash_reports = on,
            Change::MarkRead(when) => view.mark_read = when,
            Change::AutoAdvance(then) => view.auto_advance = then,
            Change::ConfirmDelete(on) => view.confirm_delete = on,
            Change::RemoteImages(on) => {
                view.remote_images = on;
                self.remote.always = on;
                self.fetch_remote(cx);
            }
            Change::ReplyAll(on) => view.reply_all = on,
            Change::FolderUnreadCounts(on) => view.folder_unread_counts = on,
            Change::ImportantMarkers(on) => {
                view.important_markers = on;
                self.list_state.remeasure();
            }
            Change::LimitWidth(on) => view.limit_width = on,
            Change::DarkMail(on) => view.dark_mail = on,
            Change::AttachmentPreviews(on) => {
                view.attachment_previews = on;
                self.request_thumbnails(cx);
            }
            Change::OpenSavedFolder(on) => view.open_saved_folder = on,
            Change::DriveInFiles(account, on) => {
                let off = &mut view.files.drives_off;
                off.retain(|a| *a != account);
                if !on {
                    off.push(account);
                }
                self.save_config();
                self.load_drives();
                cx.notify();
                return;
            }
            Change::LeaveOutSmallPictures(_)
            | Change::SmallPictureKb(_)
            | Change::SmallPicturePx(_) => {
                let files = &mut view.files;
                match change {
                    Change::LeaveOutSmallPictures(on) => files.leave_out_small = on,
                    Change::SmallPictureKb(kb) => {
                        files.small_kb =
                            kb.clamp(*FilesPage::KB_RANGE.start(), *FilesPage::KB_RANGE.end());
                    }
                    Change::SmallPicturePx(side) => {
                        files.small_px =
                            side.clamp(*FilesPage::PX_RANGE.start(), *FilesPage::PX_RANGE.end());
                    }
                    _ => {}
                }
                self.save_config();
                // The page read once is read again with the new rule.
                if self.library_loaded() {
                    self.load_library(cx);
                }
                cx.notify();
                return;
            }
            Change::PlainText(on) => sending.plain_text = on,
            Change::SpellCheck(on) => sending.spell_check = on,
            Change::StartAtLogin(start) => {
                if let Err(err) = crate::autostart::set(start) {
                    tracing::warn!(%err, "cannot change opening at login");
                    self.show_snackbar(
                        tr!("settings-open-at-login-failed", error = err.to_string()),
                        None,
                        cx,
                    );
                }
                if let Some(page) = self.settings_page.as_mut() {
                    page.start_at_login = crate::autostart::get();
                }
                cx.notify();
                return;
            }
            Change::NewMailNotices(_)
            | Change::Sound(..)
            | Change::SoundChoice(..)
            | Change::SoundSet(_) => {
                match change {
                    Change::NewMailNotices(on) => self.config.notifications.new_mail = on,
                    Change::Sound(event, on) => self.config.sounds.get_mut(event).on = on,
                    Change::SoundChoice(event, id) => {
                        // The set's sound stays unnamed, so it follows a
                        // change of set.
                        let set = katna_platform::sound::set(&self.config.sounds.set).id;
                        let id = if id == katna_platform::sound::usual(event, set) {
                            ""
                        } else {
                            id
                        };
                        self.set_event_sound(event, id.to_owned());
                    }
                    Change::SoundSet(id) => {
                        self.config.sounds.set = if id == katna_platform::sound::USUAL_SET {
                            String::new()
                        } else {
                            id.to_owned()
                        };
                    }
                    _ => {}
                }
                self.save_config();
                self.send(crate::daemon::Command::ReloadConfig, None, None, true, cx);
                cx.notify();
                return;
            }
            Change::SingleKeys(on) => {
                self.config.shortcuts.single_keys = on;
                self.shortcuts_changed(cx);
                return;
            }
            Change::OfflineDays(days) => {
                if self.config.sync.offline_days == days {
                    return;
                }
                self.config.sync.offline_days = days;
                self.save_config();
                self.send(crate::daemon::Command::ReloadConfig, None, None, true, cx);
                cx.notify();
                return;
            }
            Change::AutoDownloadUpdates(on) => {
                self.config.updates.auto_download = on;
                self.save_config();
                self.send(crate::daemon::Command::ReloadConfig, None, None, true, cx);
                cx.notify();
                return;
            }
            Change::SendCrashReports(on) => {
                self.config.feedback.send_crash_reports = Some(on);
                self.save_config();
                self.send(crate::daemon::Command::ReloadConfig, None, None, true, cx);
                cx.notify();
                return;
            }
            Change::CalendarDensity(density) => {
                self.set_calendar_density(density, cx);
                return;
            }
            Change::CustomDays(days) => {
                self.keep_custom_days(days, cx);
                return;
            }
            Change::AppAccount(app, id, shown) => {
                self.set_app_account_shown(app, id, shown, cx);
                return;
            }
            Change::AccountOnline(id, online) => {
                let time = (!online).then_some(super::offline::OfflineFor::Now);
                self.set_account_offline(id, time, cx);
                return;
            }
            Change::Birthdays(on) => {
                if self.config.contacts.hide_birthdays == on {
                    self.toggle_birthdays(cx);
                }
                return;
            }
            Change::Clock(clock) => {
                if self.config.general.clock == clock {
                    return;
                }
                self.config.general.clock = clock;
                crate::format::set_clock(clock);
                self.save_config();
                // Every open window shows times.
                cx.refresh_windows();
                return;
            }
            Change::Language(tag) => {
                if self.config.general.language == tag {
                    return;
                }
                self.config.general.language = tag.to_owned();
                katna_i18n::apply(&self.config.general.language);
                // Text set once rather than at every frame.
                let placeholder = if self.settings_page.is_some() {
                    katna_i18n::tr!("search-settings")
                } else {
                    katna_i18n::tr!("search-mail")
                };
                self.search
                    .update(cx, |search, _| search.set_placeholder(placeholder));
                self.save_config();
                // The menu bar is built from text too.
                super::refresh_menu_bar(cx);
                // The daemon's notifications, tray and dock menu follow.
                self.send(crate::daemon::Command::ReloadConfig, None, None, true, cx);
                cx.notify();
                return;
            }
            Change::GrammarCheck(on) => {
                self.config.sending.grammar_check = on;
                self.save_config();
                self.grammar_changed(cx);
                cx.notify();
                return;
            }
            Change::WritingSuggestions(on) => {
                self.config.sending.writing_suggestions = on;
                self.save_config();
                self.suggestions_changed(cx);
                cx.notify();
                return;
            }
            Change::UnifiedInbox(on) => {
                self.set_unified_inbox(on, cx);
                return;
            }
            Change::InUnified(id, on) => {
                self.set_in_unified(id, on, cx);
                return;
            }
            Change::TranslateOffer(on) => view.translation.offer = on,
            Change::TranslateAlways(code, on) | Change::TranslateNever(code, on) => {
                let translation = &mut view.translation;
                let (list, other) = if matches!(change, Change::TranslateAlways(..)) {
                    (&mut translation.always, &mut translation.never)
                } else {
                    (&mut translation.never, &mut translation.always)
                };
                list.retain(|l| l != code);
                if on {
                    list.push(code.to_owned());
                    // A language is either always translated or never
                    // offered.
                    other.retain(|l| l != code);
                }
            }
            Change::ReadingLanguage(tag) => {
                view.translation.reading_language = tag.to_owned();
                self.translations.forget_sources();
            }
            Change::Tray(_) | Change::TrayStyle(_) | Change::UnreadBadge(_) => {
                let general = &mut self.config.general;
                match change {
                    Change::Tray(on) => general.show_in_tray = on,
                    Change::TrayStyle(style) => general.tray_style = style,
                    Change::UnreadBadge(on) => general.unread_badge = on,
                    _ => {}
                }
                self.save_config();
                self.send(crate::daemon::Command::ReloadConfig, None, None, true, cx);
                cx.notify();
                return;
            }
        }
        self.save_config();
        if relist {
            self.relist(cx);
        }
        cx.notify();
    }

    /// Lists the open folder again after a setting changed what it shows.
    pub(super) fn relist(&mut self, cx: &mut Context<Self>) {
        self.reader = None;
        self.reading = false;
        if self.folder.is_some() || self.unified.is_some() {
            self.card_seq += 1;
            self.open_listed(cx);
        }
        cx.notify();
    }

    /// A reading-pane option: a small drawing of the layout and its name.
    pub(super) fn pane_choice(
        &self,
        pane: ReadingPane,
        label: impl Into<SharedString>,
        th: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let on = self.config.mail.reading_pane == pane;
        // At rest the drawing shows the layout; under the pointer it plays
        // opening a mail in it, over and over.
        let hovered = self.pane_hover == Some(pane);
        let rest = if pane == ReadingPane::Right { 1.0 } else { 0.0 };
        let picture = if hovered && !cx.reduce_motion() {
            let th = *th;
            div()
                .with_animation(
                    ("pane-demo", pane as usize),
                    Animation::new(katna_ui::motion::time(PANE_DEMO)).repeat(),
                    move |el, t| el.child(pane_picture(pane, demo_open(t), &th)),
                )
                .into_any_element()
        } else {
            pane_picture(pane, rest, th).into_any_element()
        };
        self.page_control(
            div().id(match pane {
                ReadingPane::Right => "pane-right",
                ReadingPane::None => "pane-none",
            }),
            th,
            cx,
        )
        .relative()
        .overflow_hidden()
        .flex_1()
        .p(px(6.0))
        .flex()
        .flex_col()
        .gap(px(8.0))
        .rounded(px(12.0))
        .border_px(2.0)
        .cursor_pointer()
        .hover(|s| s.bg(rgba(th.hover)))
        .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
            let now = hovered.then_some(pane);
            if *hovered || this.pane_hover == Some(pane) {
                this.pane_hover = now;
                cx.notify();
            }
        }))
        .on_click(cx.listener(move |this, _, _, cx| this.apply(Change::Pane(pane), cx)))
        .child(Ripple::new(("pane-ripple", pane as usize), rgba(th.ripple)).rounded(12.0))
        .child(picture)
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .px(px(2.0))
                .pb(px(2.0))
                .text_size(px(13.0))
                .child(animated_radio(("pane-radio", pane as usize), on, th))
                .child(label.into()),
        )
        .with_spring(
            ("pane-border", pane as usize),
            SpringAnimation::new(katna_ui::motion::scaled(motion::SMOOTH)).to(if on {
                1.0
            } else {
                0.0
            }),
            {
                let (off, accent) = (th.divider, th.accent);
                move |el, s: f32| el.border_color(rgba(mix(off, accent, s.clamp(0.0, 1.0))))
            },
        )
        .into_any_element()
    }

    /// On the Settings page a control is a Tab stop. The quick settings
    /// panel leaves the focus in the list, so its keys keep working.
    pub(super) fn page_control(
        &self,
        control: Stateful<Div>,
        th: &Theme,
        cx: &App,
    ) -> Stateful<Div> {
        match &self.settings_page {
            Some(page) => control.focus_ring_in(page.tab_stops(), th, cx),
            None => control,
        }
    }

    pub(super) fn radio_row(
        &self,
        id: &'static str,
        label: impl Into<SharedString>,
        on: bool,
        change: Change,
        th: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        // A long label wraps onto a second line in a narrow window.
        self.page_control(crate::widgets::row(id, false, th), th, cx)
            .gap(px(14.0))
            .on_click(cx.listener(move |this, _, _, cx| this.apply(change, cx)))
            .child(animated_radio((id, 2_usize), on, th))
            .child(div().flex_1().min_w_0().child(label.into()))
            .into_any_element()
    }

    /// A row that opens a section of the Settings page.
    fn link_row(
        &self,
        id: &'static str,
        label: impl Into<SharedString>,
        detail: SharedString,
        section: super::settings_page::Section,
        th: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.page_control(crate::widgets::row(id, false, th), th, cx)
            .on_click(
                cx.listener(move |this, _, window, cx| {
                    this.open_settings_page(section, window, cx)
                }),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(div().text_size(px(14.0)).child(label.into()))
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(rgba(th.text_faint))
                            .truncate()
                            .child(detail),
                    ),
            )
            .child(crate::widgets::icon("chevron-right", th.text_dim, 20.0))
            .into_any_element()
    }

    /// "Work, by default" or "None yet".
    fn signature_summary(&self) -> SharedString {
        let sending = &self.config.sending;
        match (
            sending.signatures.len(),
            sending.signature(sending.new_mail_signature),
        ) {
            (0, _) => tr!("quick-signatures-none").into(),
            (n, Some(default)) => {
                let name = if default.name.trim().is_empty() {
                    tr!("quick-signature-untitled")
                } else {
                    default.name.clone()
                };
                if n == 1 {
                    tr!("quick-signatures-one", name = name).into()
                } else {
                    tr!("quick-signatures-many", count = n, name = name).into()
                }
            }
            (n, None) => tr!("quick-signatures-no-default", count = n).into(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn switch_row(
        &self,
        id: impl Into<gpui::ElementId>,
        label: impl Into<SharedString>,
        detail: impl Into<SharedString>,
        on: bool,
        change: Change,
        th: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.switch_row_with(id, label, detail, on, change, None, th, cx)
    }

    /// A [`Self::switch_row`] with `extra` controls just before the switch.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn switch_row_with(
        &self,
        id: impl Into<gpui::ElementId>,
        label: impl Into<SharedString>,
        detail: impl Into<SharedString>,
        on: bool,
        change: Change,
        extra: Option<AnyElement>,
        th: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id: gpui::ElementId = id.into();
        self.page_control(crate::widgets::row(id.clone(), false, th), th, cx)
            .on_click(cx.listener(move |this, _, _, cx| this.apply(change, cx)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(div().text_size(px(14.0)).child(label.into()))
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(rgba(th.text_faint))
                            .child(detail.into()),
                    ),
            )
            .children(extra)
            .child(div().with_spring(
                (id, "switch"),
                SpringAnimation::new(katna_ui::motion::scaled(motion::SLIDE)).to(if on {
                    1.0
                } else {
                    0.0
                }),
                {
                    let th = *th;
                    move |el, s: f32| el.child(switch(s.clamp(0.0, 1.0), &th))
                },
            ))
            .into_any_element()
    }
}

pub(super) fn animated_radio(id: impl Into<gpui::ElementId>, on: bool, th: &Theme) -> AnyElement {
    let th = *th;
    div()
        .with_spring(
            id,
            SpringAnimation::new(katna_ui::motion::scaled(motion::SLIDE)).to(if on {
                1.0
            } else {
                0.0
            }),
            move |el, s: f32| el.child(radio(s.clamp(0.0, 1.0), &th)),
        )
        .into_any_element()
}

/// How far the demo of a reading-pane choice has opened its mail at `t`
/// through the loop: closed, opening, open for a while, closing.
fn demo_open(t: f32) -> f32 {
    let ease = |x: f32| {
        let x = x.clamp(0.0, 1.0);
        x * x * (3.0 - 2.0 * x)
    };
    ease((t - 0.15) / 0.25) - ease((t - 0.75) / 0.18)
}

/// A small drawing of a layout: the navigation, the list and the open
/// mail, which is `open` (0 to 1) of the way in: beside the list for
/// `Right`, in its place for `None`.
fn pane_picture(pane: ReadingPane, open: f32, th: &Theme) -> Div {
    let open = open.clamp(0.0, 1.0);
    let lines = |first: u32| {
        div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .p(px(5.0))
            .children((0..4).map(move |i| {
                div()
                    .h(px(4.0))
                    .rounded_full()
                    .bg(rgba(if i == 0 { first } else { th.divider }))
            }))
    };
    // The mail being opened is marked in the list.
    let first = mix(th.divider, th.accent, open);
    let message = || {
        div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .p(px(5.0))
            .child(
                div()
                    .h(px(6.0))
                    .w(px(28.0))
                    .rounded_full()
                    .bg(rgba(th.text_faint)),
            )
            .children((0..2).map(|_| div().h(px(3.0)).rounded_full().bg(rgba(th.divider))))
    };
    let card = || div().h_full().rounded(px(3.0)).bg(rgba(th.surface));
    let picture = div()
        .h(px(62.0))
        .p(px(5.0))
        .flex()
        .flex_row()
        .rounded(px(8.0))
        .bg(rgba(th.page))
        .child(
            div()
                .w(px(14.0))
                .mr(px(4.0))
                .h_full()
                .rounded(px(3.0))
                .bg(rgba(th.nav_selected)),
        );
    match pane {
        ReadingPane::Right => picture
            .child(card().flex_1().min_w_0().child(lines(first)))
            .child(
                card()
                    .flex_none()
                    .w(px(46.0 * open))
                    .ml(px(4.0 * open))
                    .overflow_hidden()
                    .opacity(open)
                    .child(message()),
            ),
        ReadingPane::None => picture.child(
            card()
                .relative()
                .flex_1()
                .min_w_0()
                .child(lines(first).opacity(1.0 - open))
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .opacity(open)
                        .child(message()),
                ),
        ),
    }
}

pub(super) fn heading(text: impl Into<SharedString>, th: &Theme) -> Div {
    div()
        .pt(px(12.0))
        .pb(px(8.0))
        .text_size(px(12.0))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgba(th.text_dim))
        .child(text.into().to_uppercase())
}

pub(super) fn divider(th: &Theme) -> Div {
    div().mt(px(12.0)).h(px(1.0)).bg(rgba(th.divider))
}

/// A line under Help: an icon and what it opens.
fn help_row(
    id: &'static str,
    name: &str,
    label: impl Into<SharedString>,
    th: &Theme,
) -> gpui::Stateful<gpui::Div> {
    crate::widgets::row(id, false, th)
        .gap(px(14.0))
        .child(icon(name, th.text_dim, 20.0))
        .child(label.into())
}
