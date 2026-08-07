// SPDX-License-Identifier: MPL-2.0

use crate::activation;
use crate::config::Config;
use crate::fl;
use cosmic::applet::padded_control;
use cosmic::cosmic_config::{self, CosmicConfigEntry};
use cosmic::cosmic_theme::Spacing;
use cosmic::iced::widget::{column, row, space};
use cosmic::iced::window::Id;
use cosmic::iced::{Alignment, Length, Limits, Subscription};
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
}

impl cosmic::Application for AppModel {
    /// The async executor that will be used to run the application's commands.
    type Executor = cosmic::executor::Default;

    /// Data that the application receives in its `init` method.
    type Flags = Flags;

    /// Messages which the application and its widgets will emit.
    type Message = Message;

    /// Unique identifier in RDNN (reverse domain name notation) format.
    const APP_ID: &'static str = "io.github.nalladev.CosmicExtAppletScreenSharing";

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
