// SPDX-License-Identifier: MPL-2.0

use std::sync::Arc;

use ashpd::desktop::screencast::{
    CursorMode, Screencast, SelectSourcesOptions, SourceType, StartCastOptions,
};
use ashpd::desktop::{CreateSessionOptions, PersistMode, ResponseError, Session};
use cosmic::applet::{menu_button, padded_control};
use cosmic::cctk::sctk::output::OutputInfo;
use cosmic::cctk::wayland_client::protocol::wl_output::WlOutput;
use cosmic::cosmic_config::{self, CosmicConfigEntry};
use cosmic::cosmic_theme::Spacing;
use cosmic::iced::core::event::wayland::OutputEvent;
use cosmic::iced::widget::{column, row, space};
use cosmic::iced::window::Id;
use cosmic::iced::{Alignment, Event, Length, Limits, Subscription, event};
use cosmic::prelude::*;
use cosmic::surface;
use cosmic::surface::action::LiveSettings;
use cosmic::theme;
use cosmic::widget::{button, divider, icon, scrollable, text};

use crate::activation;
use crate::config::Config;
use crate::fl;

/// Size of the icons used inside popup rows.
const ICON_SIZE: u16 = 16;

// ---------------------------------------------------------------------------
// Command-line flags
// ---------------------------------------------------------------------------

/// Command-line flags passed to the applet.
#[derive(Debug, Clone, Default)]
pub struct Flags;

// ---------------------------------------------------------------------------
// Application model
// ---------------------------------------------------------------------------

/// The application model stores app-specific state used to describe its
/// interface and drive its logic.
pub struct AppModel {
    /// Application state which is managed by the COSMIC runtime.
    core: cosmic::Core,
    /// The id of the popup window, if open.
    popup: Option<Id>,
    /// Configuration data that persists between application runs.
    config: Config,
    /// Displays (outputs) currently known to the compositor, kept up to
    /// date by `Message::OutputEvent`.
    outputs: Vec<OutputState>,
    /// The currently running screen share, if any.
    share: Option<ShareStatus>,
    /// The portal session backing the running share. Kept alive so the
    /// share can be stopped with `Message::StopShare`.
    share_session: Option<ShareSession>,
    /// True while the portal selection dialog is open.
    share_pending: bool,
    /// The last error from starting a share, shown in the popup.
    share_error: Option<String>,
}

/// Messages emitted by the application and its widgets.
#[derive(Debug, Clone)]
pub enum Message {
    /// Toggle the applet's popup window.
    TogglePopup,
    /// The popup window was closed.
    PopupClosed(Id),
    /// Configuration was updated (locally or by an external tool).
    UpdateConfig(Config),
    /// A Wayland output (display) was created, removed, or changed
    /// geometry — i.e. a hotplug event.
    OutputEvent(Box<OutputEvent>, WlOutput),
    /// Start sharing the entire screen.
    StartShareScreen,
    /// Start sharing a single window (the portal dialog asks which one).
    StartShareWindow,
    /// The screen cast portal finished selecting a source.
    ShareStarted(ShareTarget, Result<(ShareStream, ShareSession), String>),
    /// The user cancelled the portal selection dialog.
    ShareCancelled,
    /// Stop the running share and close its portal session.
    StopShare,
    /// The share session has been closed.
    ShareStopped,
    /// The portal closed the session that `session` refers to — from either
    /// side — so the share is no longer active.
    SessionClosed(ShareSession),
    /// Dismiss the share error shown in the popup.
    DismissShareError,
}

impl cosmic::Application for AppModel {
    /// The async executor that will be used to run the application's commands.
    type Executor = cosmic::executor::Default;

    /// Data that the application receives in its `init` method.
    type Flags = Flags;

    /// Messages which the application and its widgets will emit.
    type Message = Message;

    /// Unique identifier in RDNN (reverse domain name notation) format.
    const APP_ID: &'static str = "io.github.nalladev.cosmic-ext-applet-screen-sharing";

    fn core(&self) -> &cosmic::Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut cosmic::Core {
        &mut self.core
    }

    /// Initializes the application with any given flags and startup commands.
    fn init(
        core: cosmic::Core,
        _flags: Self::Flags,
    ) -> (Self, Task<cosmic::Action<Self::Message>>) {
        // Load the persistent configuration.  The config context is not kept
        // yet: it is only needed to write settings back, and the applet has
        // no writable settings so far.  Re-add it together with the first
        // settings row (see the eyedropper applet for the pattern).
        let config = cosmic_config::Config::new(Self::APP_ID, Config::VERSION)
            .map(|context| match Config::get_entry(&context) {
                Ok(config) => config,
                Err((_errors, config)) => config,
            })
            .unwrap_or_default();

        let app = AppModel {
            core,
            config,
            popup: None,
            outputs: Vec::new(),
            share: None,
            share_session: None,
            share_pending: false,
            share_error: None,
        };

        (app, Task::none())
    }

    fn on_close_requested(&self, id: Id) -> Option<Message> {
        (self.popup == Some(id)).then_some(Message::PopupClosed(id))
    }

    /// Draw the applet button in the panel.
    fn view(&self) -> Element<'_, Self::Message> {
        let mut button = self.core.applet.icon_button("display-projector-symbolic");
        // Highlight the button while a share is running.
        if self.share.is_some() {
            button = button.selected(true);
        }
        button.on_press(Message::TogglePopup).into()
    }

    /// Draw a window — the applet's popup.
    fn view_window(&self, id: Id) -> Element<'_, Self::Message> {
        if self.popup == Some(id) {
            self.view_popup()
        } else {
            space::horizontal().width(Length::Fixed(1.0)).into()
        }
    }

    /// Register subscriptions for this application.
    fn subscription(&self) -> Subscription<Self::Message> {
        Subscription::batch(vec![
            // Watch for configuration changes (also triggered by the
            // cosmic-settings-daemon when the config file changes externally).
            self.core()
                .watch_config::<Config>(Self::APP_ID)
                .map(|update| Message::UpdateConfig(update.config)),
            // D-Bus activation (single-instance, external launchers).
            activation::subscription(),
            // Wayland output (display) hotplug and geometry events.
            event::listen_with(|e, _, _| match e {
                Event::PlatformSpecific(event::PlatformSpecific::Wayland(
                    event::wayland::Event::Output(o_event, wl_output),
                )) => Some(Message::OutputEvent(Box::new(o_event), wl_output)),
                _ => None,
            }),
        ])
    }

    /// Handles messages emitted by the application and its widgets.
    fn update(&mut self, message: Self::Message) -> Task<cosmic::Action<Self::Message>> {
        match message {
            Message::TogglePopup => {
                if let Some(p) = self.popup.take() {
                    surface::surface_task(surface::action::destroy_popup(p))
                } else {
                    surface::surface_task(surface::action::app_popup(
                        |_| LiveSettings::default(),
                        |app: &mut AppModel| {
                            let new_id = Id::unique();
                            app.popup.replace(new_id);
                            let mut popup_settings = app.core.applet.get_popup_settings(
                                app.core.main_window_id().unwrap(),
                                new_id,
                                None,
                                None,
                                None,
                            );
                            popup_settings.positioner.size_limits = Limits::NONE
                                .max_width(372.0)
                                .min_width(300.0)
                                .min_height(200.0)
                                .max_height(1080.0);
                            popup_settings
                        },
                        None,
                    ))
                }
            }

            Message::PopupClosed(id) => {
                if self.popup.as_ref() == Some(&id) {
                    self.popup = None;
                }
                Task::none()
            }

            Message::UpdateConfig(config) => {
                self.config = config;
                Task::none()
            }

            // A display was plugged in, unplugged, or changed geometry.
            Message::OutputEvent(o_event, wl_output) => {
                self.update_output_event(*o_event, wl_output);
                Task::none()
            }

            // Screen sharing.
            Message::StartShareScreen
            | Message::StartShareWindow
            | Message::ShareStarted(..)
            | Message::ShareCancelled
            | Message::StopShare
            | Message::ShareStopped
            | Message::SessionClosed(..)
            | Message::DismissShareError => self.update_share(message),
        }
    }

    fn system_theme_update(
        &mut self,
        _keys: &[&'static str],
        new_theme: &cosmic::cosmic_theme::Theme,
    ) -> Task<cosmic::Action<Self::Message>> {
        let _ = new_theme;
        Task::none()
    }

    fn style(&self) -> Option<cosmic::iced::theme::Style> {
        Some(cosmic::applet::style())
    }
}

// ---------------------------------------------------------------------------
// Output (display) tracking
// ---------------------------------------------------------------------------

/// The kind of a display, derived from its connector name.
///
/// This is a heuristic: compositors report the connector name of a `wl_output`
/// (e.g. `"HDMI-A-1"`, `"eDP-1"`), and we map known prefixes to a category.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputKind {
    /// The built-in panel of a laptop (e.g. `eDP-1`, `LVDS-1`).
    Builtin,
    /// An external display attached over a wired connector
    /// (`DisplayPort`, HDMI, DVI, or VGA).
    Wired,
    /// A virtual output created by the compositor (e.g. `WL-1`, `X11-1`).
    Virtual,
    /// A connector name we do not recognize.
    Unknown,
}

/// Tracked state for a single output (display).
///
/// `WlOutput` proxies are `Clone + Send`, so they can be passed through
/// iced messages safely.
#[derive(Debug, Clone)]
pub struct OutputState {
    /// The Wayland output object (from the iced/event-loop connection).
    output: WlOutput,
    /// Connector name (e.g. `"DP-1"`, `"eDP-1"`, `"HDMI-A-1"`).
    name: String,
    /// Logical size in compositor coordinates.
    logical_size: (u32, u32),
    /// Logical position in compositor coordinate space.
    logical_pos: (i32, i32),
}

/// Classify a connector name into an [`OutputKind`].
///
/// The `name` reported by the compositor is not guaranteed to be a DRM
/// connector name, so this is a best-effort heuristic.
fn classify_output(name: &str) -> OutputKind {
    if name.starts_with("eDP-") || name.starts_with("LVDS-") {
        OutputKind::Builtin
    } else if name.starts_with("DP-")
        || name.starts_with("HDMI-")
        || name.starts_with("DVI-")
        || name.starts_with("VGA-")
    {
        OutputKind::Wired
    } else if name.starts_with("WL-") || name.starts_with("X11-") || name.starts_with("Virtual-") {
        OutputKind::Virtual
    } else {
        OutputKind::Unknown
    }
}

impl OutputState {
    /// Build the tracked state for an output from its [`OutputInfo`].
    ///
    /// Returns `None` until the compositor reports a connector name and
    /// logical geometry — those can arrive asynchronously after the
    /// `Created` event.
    fn from_info(output: WlOutput, info: OutputInfo) -> Option<Self> {
        let (name, (w, h), logical_pos) = (info.name?, info.logical_size?, info.logical_position?);
        Some(Self {
            output,
            name,
            logical_size: (u32::try_from(w).unwrap_or(0), u32::try_from(h).unwrap_or(0)),
            logical_pos,
        })
    }

    /// Classify the output by its connector name.
    fn kind(&self) -> OutputKind {
        classify_output(&self.name)
    }
}

// ---------------------------------------------------------------------------
// Screen sharing
// ---------------------------------------------------------------------------

/// A handle to an active screen cast portal session, shared between the
/// message that starts the share and the model that later stops it.
type ShareSession = Arc<Session<Screencast>>;

/// What a screen share captures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShareTarget {
    /// The entire screen (all monitors).
    Screen,
    /// A single window, chosen by the user in the portal dialog.
    Window,
}

impl ShareTarget {
    /// The portal source type for this target.
    fn source_type(self) -> SourceType {
        match self {
            Self::Screen => SourceType::Monitor,
            Self::Window => SourceType::Window,
        }
    }
}

/// A live capture stream from the screencast portal.
#[derive(Debug, Clone)]
pub struct ShareStream {
    /// The `PipeWire` node id of the capture stream.
    node_id: u32,
    /// The size of the captured stream in compositor coordinates, when the
    /// portal reports one.
    size: Option<(i32, i32)>,
}

/// A running screen share.
#[derive(Debug, Clone)]
pub struct ShareStatus {
    /// The live capture stream.
    stream: ShareStream,
    /// What is being shared.
    target: ShareTarget,
}

/// Runs a screen cast session through the `XDG` Desktop Portal and returns
/// the `PipeWire` node id of the capture stream, together with the session
/// handle that must be kept alive (and later closed) to stop the share.
async fn run_screencast(source: SourceType) -> anyhow::Result<(ShareStream, Session<Screencast>)> {
    let proxy = Screencast::new().await?;
    let session = proxy
        .create_session(CreateSessionOptions::default())
        .await?;
    proxy
        .select_sources(
            &session,
            SelectSourcesOptions::default()
                .set_sources(Some(source.into()))
                .set_multiple(false)
                .set_cursor_mode(CursorMode::Embedded)
                .set_persist_mode(PersistMode::DoNot),
        )
        .await?;
    let response = proxy
        .start(&session, None, StartCastOptions::default())
        .await?
        .response()?;
    let stream = response
        .streams()
        .first()
        .map(|stream| ShareStream {
            node_id: stream.pipe_wire_node_id(),
            size: stream.size(),
        })
        .ok_or_else(|| anyhow::anyhow!("no stream was selected"))?;
    Ok((stream, session))
}

/// Waits until the portal closes `session` — from either side — and reports
/// it, so the UI can drop a stale running-share row.
async fn monitor_session(session: ShareSession) -> Message {
    use cosmic::iced::futures::StreamExt;

    // The signal stream borrows `session` (Rust 2024 `impl Trait` capture),
    // so the message carries a clone of the `Arc`; identity is preserved.
    let subscription = session.receive_closed().await;
    let mut closed = match subscription {
        Ok(closed) => closed,
        Err(error) => {
            // Without the subscription the share state is best-effort; the
            // portal session is most likely gone already.
            log::debug!("could not subscribe to session close: {error}");
            return Message::SessionClosed(session.clone());
        }
    };
    let _ = closed.next().await;
    Message::SessionClosed(session.clone())
}

// ---------------------------------------------------------------------------
// Helper methods on AppModel
// ---------------------------------------------------------------------------

impl AppModel {
    /// Apply a Wayland output (display) event to the tracked output list.
    fn update_output_event(&mut self, o_event: OutputEvent, wl_output: WlOutput) {
        match o_event {
            OutputEvent::Created(Some(info)) => {
                if let Some(state) = OutputState::from_info(wl_output, info) {
                    log::debug!(
                        "[display] created: {} ({:?}) at {:?}, {}x{} logical",
                        state.name,
                        state.kind(),
                        state.logical_pos,
                        state.logical_size.0,
                        state.logical_size.1,
                    );
                    self.outputs.push(state);
                }
            }
            OutputEvent::Created(None) => {}
            OutputEvent::Removed => {
                if let Some(removed) = self.outputs.iter().find(|o| o.output == wl_output) {
                    log::debug!("[display] removed: {}", removed.name);
                }
                self.outputs.retain(|o| o.output != wl_output);
            }
            OutputEvent::InfoUpdate(info) => {
                if let Some(state) = self.outputs.iter_mut().find(|o| o.output == wl_output) {
                    if let Some(name) = info.name {
                        state.name = name;
                    }
                    if let Some((w, h)) = info.logical_size {
                        state.logical_size =
                            (u32::try_from(w).unwrap_or(0), u32::try_from(h).unwrap_or(0));
                    }
                    if let Some(pos) = info.logical_position {
                        state.logical_pos = pos;
                    }
                    log::debug!(
                        "[display] updated: {} ({:?}) at {:?}, {}x{} logical",
                        state.name,
                        state.kind(),
                        state.logical_pos,
                        state.logical_size.0,
                        state.logical_size.1,
                    );
                } else if let Some(state) = OutputState::from_info(wl_output, info) {
                    // Some compositors report full info without a prior
                    // `Created` event — track the output anyway.
                    log::debug!(
                        "[display] created via info update: {} ({:?})",
                        state.name,
                        state.kind(),
                    );
                    self.outputs.push(state);
                }
            }
        }
    }

    /// Handles messages related to screen sharing.
    fn update_share(&mut self, message: Message) -> Task<cosmic::Action<Message>> {
        match message {
            Message::StartShareScreen => self.start_share(ShareTarget::Screen),

            Message::StartShareWindow => self.start_share(ShareTarget::Window),

            // The portal reported a live capture stream.
            Message::ShareStarted(target, Ok((stream, session))) => {
                self.share_pending = false;
                self.share = Some(ShareStatus { stream, target });
                self.share_session = Some(session.clone());
                // Watch for the portal closing the session (e.g. the
                // compositor ends the capture) to clear the status row.
                cosmic::task::future(monitor_session(session))
            }

            // The portal (or D-Bus) reported an error.
            Message::ShareStarted(_, Err(error)) => {
                self.share_pending = false;
                self.share_error = Some(error);
                Task::none()
            }

            // The user dismissed the portal selection dialog; not an error.
            Message::ShareCancelled => {
                self.share_pending = false;
                Task::none()
            }

            Message::StopShare => {
                let session = self.share_session.take();
                self.share = None;
                self.share_pending = false;
                if let Some(session) = session {
                    cosmic::task::future(async move {
                        let _ = session.close().await;
                        Message::ShareStopped
                    })
                } else {
                    Task::none()
                }
            }

            // The portal closed the session; only clear the status if this
            // is still the tracked share (a late signal from an earlier
            // session must not clear a newer one).
            Message::SessionClosed(session) => {
                if self
                    .share_session
                    .as_ref()
                    .is_some_and(|current| Arc::ptr_eq(current, &session))
                {
                    self.share = None;
                    self.share_session = None;
                    self.share_pending = false;
                }
                Task::none()
            }

            Message::DismissShareError => {
                self.share_error = None;
                Task::none()
            }

            _ => Task::none(),
        }
    }

    /// Render the applet's popup window.
    fn view_popup(&self) -> Element<'_, Message> {
        let Spacing {
            space_xxs, space_s, ..
        } = theme::active().cosmic().spacing;

        let heading =
            row![text::title4(fl!("app-title")), space::horizontal(),].align_y(Alignment::Center);

        let divider = || padded_control(divider::horizontal::default()).padding([space_xxs, 0]);

        let mut children: Vec<Element<'_, Message>> = vec![
            padded_control(heading).into(),
            divider().into(),
            action_row(
                "display-projector-symbolic",
                fl!("share-screen"),
                self.can_start_share().then_some(Message::StartShareScreen),
            ),
            action_row(
                "computer-symbolic",
                fl!("share-window"),
                self.can_start_share().then_some(Message::StartShareWindow),
            ),
            wired_section(&self.outputs, self.can_start_share()),
            divider().into(),
            wireless_section(),
        ];

        if self.share_pending {
            children.push(
                padded_control(
                    row![
                        cosmic::widget::progress_bar::indeterminate_circular().size(16.0),
                        text::body(fl!("share-waiting")),
                    ]
                    .align_y(Alignment::Center)
                    .spacing(space_s),
                )
                .into(),
            );
        }

        if let Some(status) = &self.share {
            children.push(share_status_row(status));
        }

        if let Some(error) = &self.share_error {
            children.push(share_error_row(error));
        }

        let content = column::with_children(children)
            .padding([space_xxs, 0])
            .spacing(0);

        self.core.applet.popup_container(scrollable(content)).into()
    }

    /// Whether a new share can be started right now.
    fn can_start_share(&self) -> bool {
        self.share.is_none() && !self.share_pending
    }

    /// Start a screen cast for `target`, unless another share is already
    /// running or waiting for the portal dialog.
    fn start_share(&mut self, target: ShareTarget) -> Task<cosmic::Action<Message>> {
        if !self.can_start_share() {
            return Task::none();
        }

        self.share_pending = true;
        self.share_error = None;
        let source = target.source_type();

        cosmic::task::future(async move {
            match run_screencast(source).await {
                Ok((stream, session)) => {
                    Message::ShareStarted(target, Ok((stream, Arc::new(session))))
                }
                Err(error) => {
                    // Cancelling the dialog is not an error.
                    if let Some(ashpd::Error::Response(ResponseError::Cancelled)) =
                        error.downcast_ref::<ashpd::Error>()
                    {
                        Message::ShareCancelled
                    } else if error.downcast_ref::<ashpd::Error>().is_some() {
                        // The portal returned a plain failure; it carries no
                        // useful detail, so show a friendly message.
                        Message::ShareStarted(target, Err(fl!("share-error-portal")))
                    } else {
                        // D-Bus / transport failures: keep the raw message.
                        Message::ShareStarted(target, Err(error.to_string()))
                    }
                }
            }
        })
    }
}

/// A row that starts a share, for a given source type.
fn action_row(
    icon_name: &str,
    label: String,
    action: Option<Message>,
) -> Element<'static, Message> {
    let Spacing { space_s, .. } = theme::active().cosmic().spacing;

    menu_button(
        row![
            icon::from_name(icon_name).size(ICON_SIZE),
            text::body(label),
            space::horizontal(),
            icon::from_name("media-playback-start-symbolic").size(ICON_SIZE),
        ]
        .align_y(Alignment::Center)
        .spacing(space_s),
    )
    .on_press_maybe(action)
    .into()
}

/// The list of wired displays with a share action per row.
fn wired_section(outputs: &[OutputState], can_start: bool) -> Element<'_, Message> {
    let Spacing {
        space_s, space_m, ..
    } = theme::active().cosmic().spacing;

    let header = cosmic::widget::container(text::caption_heading(fl!("section-wired")))
        .padding([space_s, space_m])
        .width(Length::Fill);

    let mut children: Vec<Element<'_, Message>> = vec![header.into()];
    let mut wired: Vec<&OutputState> = outputs
        .iter()
        .filter(|o| o.kind() == OutputKind::Wired)
        .collect();
    // Keep the list stable regardless of hotplug order.
    wired.sort_by_key(|output| &output.name);

    if wired.is_empty() {
        children.push(padded_control(text::caption(fl!("no-wired-displays"))).into());
    } else {
        children.extend(wired.iter().map(|output| display_row(output, can_start)));
    }

    column::with_children(children).into()
}

/// A single wired display row: name, logical size, and a share action.
fn display_row(output: &OutputState, can_start: bool) -> Element<'_, Message> {
    let Spacing {
        space_xxs, space_s, ..
    } = theme::active().cosmic().spacing;

    let size = format!("{} × {}", output.logical_size.0, output.logical_size.1);

    menu_button(
        row![
            icon::from_name("display-projector-symbolic").size(ICON_SIZE),
            column![text::body(&output.name), text::caption(size),]
                .spacing(space_xxs)
                .align_x(Alignment::Start),
            space::horizontal(),
            icon::from_name("media-playback-start-symbolic").size(ICON_SIZE),
        ]
        .align_y(Alignment::Center)
        .spacing(space_s),
    )
    .on_press_maybe(can_start.then_some(Message::StartShareScreen))
    .into()
}

/// The wireless receivers section (a stub until `FCast` is supported).
fn wireless_section() -> Element<'static, Message> {
    let Spacing {
        space_s, space_m, ..
    } = theme::active().cosmic().spacing;

    column![
        cosmic::widget::container(text::caption_heading(fl!("section-wireless")))
            .padding([space_s, space_m])
            .width(Length::Fill),
        padded_control(
            row![
                icon::from_name("network-wireless-symbolic").size(ICON_SIZE),
                text::caption(fl!("wireless-stub")),
            ]
            .align_y(Alignment::Center)
            .spacing(space_s),
        ),
    ]
    .into()
}

/// A row describing the running share, with a stop button.
fn share_status_row(status: &ShareStatus) -> Element<'_, Message> {
    let Spacing {
        space_xxs, space_s, ..
    } = theme::active().cosmic().spacing;

    let label = match status.target {
        ShareTarget::Screen => fl!("share-active-screen"),
        ShareTarget::Window => fl!("share-active-window"),
    };

    let caption = match status.stream.size {
        Some((width, height)) => fl!(
            "share-stream",
            node = status.stream.node_id,
            width = width,
            height = height,
        ),
        None => fl!("share-node", node = status.stream.node_id),
    };

    padded_control(
        row![
            icon::from_name("media-playback-stop-symbolic").size(ICON_SIZE),
            column![text::body(label), text::caption(caption),]
                .spacing(space_xxs)
                .align_x(Alignment::Start),
            space::horizontal(),
            button::destructive(fl!("stop-share")).on_press(Message::StopShare),
        ]
        .align_y(Alignment::Center)
        .spacing(space_s),
    )
    .into()
}

/// A row showing the last share error, with a dismiss button.
fn share_error_row(error: &str) -> Element<'_, Message> {
    let Spacing { space_s, .. } = theme::active().cosmic().spacing;

    padded_control(
        row![
            icon::from_name("dialog-error-symbolic").size(ICON_SIZE),
            text::body(error),
            space::horizontal(),
            button::standard(fl!("dismiss")).on_press(Message::DismissShareError),
        ]
        .align_y(Alignment::Center)
        .spacing(space_s),
    )
    .into()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::{OutputKind, ShareTarget, classify_output};

    /// Connector-name classification for the display list.
    #[test]
    fn classifies_connector_names() {
        assert_eq!(classify_output("eDP-1"), OutputKind::Builtin);
        assert_eq!(classify_output("LVDS-1"), OutputKind::Builtin);
        assert_eq!(classify_output("DP-1"), OutputKind::Wired);
        assert_eq!(classify_output("HDMI-A-1"), OutputKind::Wired);
        assert_eq!(classify_output("DVI-I-1"), OutputKind::Wired);
        assert_eq!(classify_output("VGA-1"), OutputKind::Wired);
        assert_eq!(classify_output("WL-1"), OutputKind::Virtual);
        assert_eq!(classify_output("X11-1"), OutputKind::Virtual);
        assert_eq!(classify_output("Virtual-1"), OutputKind::Virtual);
        assert_eq!(classify_output("Mystery-1"), OutputKind::Unknown);
    }

    /// The portal source type for each share target.
    #[test]
    fn share_target_source_type() {
        use ashpd::desktop::screencast::SourceType;

        assert_eq!(ShareTarget::Screen.source_type(), SourceType::Monitor);
        assert_eq!(ShareTarget::Window.source_type(), SourceType::Window);
    }

    /// The embedded fallback language parses without errors; accessing the
    /// loader panics if the bundled FTL is invalid.
    #[test]
    fn fallback_language_loads() {
        drop(crate::i18n::localizer());
    }
}
