// SPDX-License-Identifier: MPL-2.0

use crate::activation;
use crate::config::Config;
use crate::fl;
use cosmic::applet::padded_control;
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
use cosmic::widget::{divider, text};

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
        };

        (app, Task::none())
    }

    fn on_close_requested(&self, id: Id) -> Option<Message> {
        (self.popup == Some(id)).then_some(Message::PopupClosed(id))
    }

    /// Draw the applet button in the panel.
    fn view(&self) -> Element<'_, Self::Message> {
        self.core
            .applet
            .icon_button("display-symbolic")
            .on_press(Message::TogglePopup)
            .into()
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
                match *o_event {
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
                        if let Some(state) = self.outputs.iter_mut().find(|o| o.output == wl_output)
                        {
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
                Task::none()
            }
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
// Helper methods on AppModel
// ---------------------------------------------------------------------------

impl AppModel {
    /// Render the applet's popup window.
    fn view_popup(&self) -> Element<'_, Message> {
        let Spacing { space_xxs, .. } = theme::active().cosmic().spacing;

        let heading =
            row![text::title4(fl!("app-title")), space::horizontal(),].align_y(Alignment::Center);

        let placeholder = text::body(fl!("popup-placeholder"));

        let content = column![
            padded_control(heading),
            padded_control(divider::horizontal::default()).padding([space_xxs, 0]),
            padded_control(placeholder),
        ]
        .padding([space_xxs, 0])
        .spacing(0);

        self.core.applet.popup_container(content).into()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::{OutputKind, classify_output};

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
}
