// SPDX-License-Identifier: GPL-3.0-or-later

//! Settings > Experimental > Look & Feel: Katna's own window frame instead
//! of the desktop's, a blurred, translucent window background, and frosted
//! menus and dialogs. They apply at once to every open window (`katna_chrome::Look`); on Windows
//! the frame changes when a window next opens.

use gpui::{AnyElement, Context, FontWeight, SharedString, div, prelude::*, rgba};
use katna_chrome::{DecorationMode, Desktop, Look, Session};
use katna_core::config::{Config, WindowFrame};
use katna_i18n::tr;
use katna_ui::px;

use super::MailWindow;
use super::settings::{Change, heading};
use crate::theme::Theme;
use crate::widgets::switch;

/// How much more solid a blurred window background is than the frost's
/// tint: part of the way from the tint to solid, so the folders and the top
/// bar stay readable over any wallpaper.
const WINDOW_TINT: f32 = 0.6;

/// The look the settings ask for.
pub fn look(config: &Config) -> Look {
    let experimental = &config.experimental;
    Look {
        own_frame: experimental.window_frame == WindowFrame::Katna,
        blur: experimental.blur,
        radius: experimental.window_radius,
        border: experimental.window_border,
        border_opacity: experimental.window_border_opacity,
        blur_opacity: experimental
            .custom_frost
            .then(|| window_opacity(experimental.frost_opacity)),
    }
}

/// A blurred window background's opacity, in percent, for the frost's tint
/// `tint`, in percent.
pub(super) fn window_opacity(tint: u8) -> u8 {
    let tint = f32::from(tint.min(100));
    (tint + (100.0 - tint) * WINDOW_TINT).round() as u8
}

impl MailWindow {
    pub(super) fn experimental_section(&self, th: &Theme, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .pt(px(20.0))
                    .pb(px(4.0))
                    .text_size(px(13.0))
                    .text_color(rgba(th.text_dim))
                    .child(tr!("look-intro")),
            )
            .child(div().pt(px(12.0)).child(heading(tr!("look-heading"), th)))
            .child(self.row(
                tr!("look-window-frame"),
                Some(&tr!("look-window-frame-detail")),
                self.frame_choice(th, cx),
                th,
            ))
            .child(self.row(
                tr!("look-blurred-background"),
                Some(&tr!("look-blurred-background-detail")),
                self.blur_switches(th, cx),
                th,
            ))
            .child(div().pt(px(12.0)).child(heading(tr!("chat-heading"), th)))
            .child(self.row(
                tr!("chat-view"),
                Some(&tr!("chat-view-detail")),
                self.switch_row(
                    "page-chat-view",
                    tr!("chat-view-switch"),
                    tr!("chat-view-switch-detail"),
                    self.config.experimental.chat_view,
                    Change::ChatView(!self.config.experimental.chat_view),
                    th,
                    cx,
                ),
                th,
            ))
            .into_any_element()
    }

    fn frame_choice(&self, th: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let env = self.chrome.environment();
        // GNOME on Wayland leaves every frame to the app: Native already is
        // Katna's frame there.
        if env.native_decorations() == DecorationMode::Client {
            return div()
                .flex()
                .flex_col()
                .child(explain(tr!("look-frame-client-side"), th))
                .child(self.frame_corners(th, cx))
                .into_any_element();
        }
        let note = match env.desktop {
            _ if cfg!(windows) => tr!("look-frame-katna-note-windows"),
            Desktop::Kde => tr!("look-frame-katna-note-named", desktop = "KDE"),
            Desktop::Gnome => tr!("look-frame-katna-note-named", desktop = "GNOME"),
            Desktop::Other(_) => tr!("look-frame-katna-note"),
        };
        let frame = self.config.experimental.window_frame;
        div()
            .flex()
            .flex_col()
            .gap(px(2.0))
            .child(self.radio_row(
                "page-frame-native",
                match env.desktop {
                    _ if cfg!(windows) => tr!("look-frame-native-windows"),
                    Desktop::Kde => tr!("look-frame-native-kde"),
                    _ => tr!("look-frame-native"),
                },
                frame == WindowFrame::Native,
                Change::WindowFrame(WindowFrame::Native),
                th,
                cx,
            ))
            .child(self.radio_row(
                "page-frame-katna",
                tr!("look-frame-katna"),
                frame == WindowFrame::Katna,
                Change::WindowFrame(WindowFrame::Katna),
                th,
                cx,
            ))
            .when(frame == WindowFrame::Katna, |d| {
                d.child(explain(note, th)).child(self.frame_corners(th, cx))
            })
            // Windows sets a window's frame when it opens.
            .when(self.chrome.frame_on_reopen(), |d| {
                d.child(explain(tr!("look-frame-on-reopen"), th))
            })
            .into_any_element()
    }

    /// The roundness of Katna's frame and the line around it, where Katna
    /// draws them: not on Windows, which rounds the corners itself, nor on
    /// tiling compositors, where the frame stays square.
    fn frame_corners(&self, th: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let env = self.chrome.environment();
        if cfg!(windows)
            || !env.full_client_frame()
            || env.requested_decorations() != DecorationMode::Client
        {
            return div().into_any_element();
        }
        let border = self.config.experimental.window_border;
        div()
            .flex()
            .flex_col()
            .pt(px(4.0))
            .child(self.frame_sliders(false, th, cx))
            .child(self.switch_row(
                "page-window-border",
                tr!("look-window-border"),
                tr!("look-window-border-detail"),
                border,
                Change::WindowBorder(!border),
                th,
                cx,
            ))
            .when(border, |d| d.child(self.frame_sliders(true, th, cx)))
            .into_any_element()
    }

    fn blur_switches(&self, th: &Theme, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .child(self.window_blur_switch(th, cx))
            .child(self.frosted_popups_switch(th, cx))
            .when(
                self.config.experimental.frosted_popups && katna_ui::frost::supported(),
                |d| d.child(self.custom_frost_switch(th, cx)),
            )
            .when(self.chrome.environment().desktop == Desktop::Kde, |d| {
                d.child(kde_blur_line(th, cx))
            })
            .into_any_element()
    }

    /// The frost's blur and opacity by hand, or the desktop's.
    fn custom_frost_switch(&self, th: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let custom = self.config.experimental.custom_frost;
        let detail = match (custom, self.chrome.environment().desktop) {
            (true, _) => tr!("look-custom-frost-on"),
            (false, Desktop::Kde) => tr!("look-custom-frost-off-kde"),
            (false, _) => tr!("look-custom-frost-off"),
        };
        div()
            .flex()
            .flex_col()
            .child(self.switch_row(
                "page-custom-frost",
                tr!("look-custom-frost"),
                detail,
                custom,
                Change::CustomFrost(!custom),
                th,
                cx,
            ))
            .when(custom, |d| d.child(self.frost_sliders(th, cx)))
            .into_any_element()
    }

    fn window_blur_switch(&self, th: &Theme, cx: &mut Context<Self>) -> AnyElement {
        if Look::blur_available() {
            let blur = self.config.experimental.blur;
            return div()
                .flex()
                .flex_col()
                .child(self.switch_row(
                    "page-blur",
                    tr!("look-blur"),
                    tr!("look-blur-detail"),
                    blur,
                    Change::Blur(!blur),
                    th,
                    cx,
                ))
                .when(blur, |d| d.child(self.pane_switches(th, cx)))
                .into_any_element();
        }
        let env = self.chrome.environment();
        let why = match (&env.desktop, env.session) {
            (Desktop::Kde, _) => tr!("look-blur-off-kde"),
            (Desktop::Gnome, _) => tr!("look-blur-none-gnome"),
            (_, Session::X11) => tr!("look-blur-none-x11"),
            (_, Session::Wayland) => tr!("look-blur-none-wayland"),
        };
        unavailable(tr!("look-blur"), why, th)
    }

    /// What else lets the window's blur through: the cards, with how
    /// opaque they are, the room behind a chat's bubbles and the open
    /// search box and the bars things scroll under, each on its own
    /// switch.
    fn pane_switches(&self, th: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let experimental = &self.config.experimental;
        let (panes, chat, search, headers) = (
            experimental.frosted_panes,
            experimental.frosted_chat,
            experimental.frosted_search,
            experimental.frosted_headers,
        );
        div()
            .flex()
            .flex_col()
            .child(self.switch_row(
                "page-frosted-panes",
                tr!("look-frosted-panes"),
                tr!("look-frosted-panes-detail"),
                panes,
                Change::FrostedPanes(!panes),
                th,
                cx,
            ))
            .when(panes, |d| d.child(self.pane_slider(th, cx)))
            .child(self.switch_row(
                "page-frosted-chat",
                tr!("look-frosted-chat"),
                tr!("look-frosted-chat-detail"),
                chat,
                Change::FrostedChat(!chat),
                th,
                cx,
            ))
            .child(self.switch_row(
                "page-frosted-search",
                tr!("look-frosted-search"),
                tr!("look-frosted-search-detail"),
                search,
                Change::FrostedSearch(!search),
                th,
                cx,
            ))
            .child(self.switch_row(
                "page-frosted-headers",
                tr!("look-frosted-headers"),
                tr!("look-frosted-headers-detail"),
                headers,
                Change::FrostedHeaders(!headers),
                th,
                cx,
            ))
            .into_any_element()
    }

    /// Katna blurs under its own menus, so this needs no compositor.
    fn frosted_popups_switch(&self, th: &Theme, cx: &mut Context<Self>) -> AnyElement {
        if katna_ui::frost::supported() {
            return self.switch_row(
                "page-frosted-popups",
                tr!("look-frosted-popups"),
                tr!("look-frosted-popups-detail"),
                self.config.experimental.frosted_popups,
                Change::FrostedPopups(!self.config.experimental.frosted_popups),
                th,
                cx,
            );
        }
        let why = if cfg!(windows) {
            tr!("look-frosted-popups-none-windows")
        } else {
            tr!("look-frosted-popups-none")
        };
        unavailable(tr!("look-frosted-popups"), why, th)
    }
}

/// KDE sets how strongly the window background blurs; this opens its
/// Blur settings.
fn kde_blur_line(th: &Theme, cx: &mut Context<MailWindow>) -> AnyElement {
    div()
        .px(px(8.0))
        .pt(px(12.0))
        .flex()
        .flex_col()
        .gap(px(2.0))
        .text_size(px(12.0))
        .line_height(px(17.0))
        .child(
            div()
                .text_color(rgba(th.text_faint))
                .child(tr!("look-kde-blur-note")),
        )
        .child(
            div()
                .id("page-kde-blur")
                .text_color(rgba(th.accent))
                .cursor_pointer()
                .hover(|s| s.underline())
                .on_click(cx.listener(|_, _, _, _| open_kde_blur_settings()))
                .child(tr!("look-kde-blur-open")),
        )
        .into_any_element()
}

/// Opens Desktop Effects in KDE's System Settings, where Blur is.
fn open_kde_blur_settings() {
    let opened = std::process::Command::new("kcmshell6")
        .arg("kcm_kwin_effects")
        .spawn()
        .or_else(|_| {
            std::process::Command::new("systemsettings")
                .arg("kcm_kwin_effects")
                .spawn()
        });
    if let Err(err) = opened {
        tracing::warn!("could not open KDE's Blur settings: {err}");
    }
}

/// A switch that cannot be used here: off and out of reach, with why.
fn unavailable(label: String, why: String, th: &Theme) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .child(
            div()
                .py(px(8.0))
                .px(px(8.0))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(12.0))
                .opacity(0.45)
                .child(div().flex_1().min_w_0().text_size(px(14.0)).child(label))
                .child(switch(0.0, th)),
        )
        .child(explain(why, th))
        .into_any_element()
}

fn explain(text: impl Into<SharedString>, th: &Theme) -> AnyElement {
    div()
        .px(px(8.0))
        .pt(px(4.0))
        .text_size(px(12.0))
        .line_height(px(17.0))
        .font_weight(FontWeight::NORMAL)
        .text_color(rgba(th.text_faint))
        .child(text.into())
        .into_any_element()
}
