// SPDX-License-Identifier: MPL-2.0

//! D-Bus activation support.
//!
//! Serves the standard `org.freedesktop.DbusActivation` interface on the
//! session bus so that external launchers (keyboard shortcuts, launchers,
//! file managers, etc.) can activate the applet without starting a second
//! instance.
//!
//! The interface is served at the conventional path
//! `/io/github/nalladev/cosmic-ext-applet-screen-sharing` under the app's
//! well-known name.  If another instance already owns the name (or the
//! session bus is unavailable, e.g. in a sandbox without `--own-name`), this
//! subscription simply stays idle and the applet behaves normally.
//!
//! Applet-specific actions (e.g. opening the sharing popup from a shortcut)
//! will be forwarded to the application through the message channel as the
//! applet gains features; for now the standard handlers are no-ops.

use std::any::TypeId;
use std::collections::HashMap;

use crate::app::{AppModel, Message};
use cosmic::Application;
use cosmic::iced::Subscription;
use cosmic::iced::futures::channel::mpsc::Sender;
use zbus::interface;
use zbus::zvariant::Value;

/// Object server implementing `org.freedesktop.DbusActivation`.
struct DbusActivation;

// The `async` handlers without awaits are deliberate no-ops; unused
// parameters are discarded explicitly to satisfy zbus' generated dispatch.
#[allow(clippy::unused_async)]
#[interface(name = "org.freedesktop.DbusActivation")]
impl DbusActivation {
    async fn activate(&mut self, platform_data: HashMap<&str, Value<'_>>) {
        // Plain activation (e.g. `gio launch`) has no dedicated behaviour
        // beyond the applet popup yet.
        let _ = platform_data;
    }

    async fn open(&mut self, uris: Vec<&str>, platform_data: HashMap<&str, Value<'_>>) {
        // Opening URIs is not supported by the applet.
        let _ = (uris, platform_data);
    }

    async fn activate_action(
        &mut self,
        action: &str,
        parameter: Vec<&str>,
        platform_data: HashMap<&str, Value<'_>>,
    ) {
        // No named actions are registered yet; future actions (e.g. "open")
        // will be matched here and forwarded to the application.
        let _ = (action, parameter, platform_data);
    }
}

/// Subscribe to activation requests on the session bus.
///
/// The stream claims the app's well-known name on the session bus and serves
/// the `org.freedesktop.DbusActivation` interface at the conventional path
/// derived from the app ID, keeping the applet single-instance.
pub fn subscription() -> Subscription<Message> {
    Subscription::run_with(TypeId::of::<DbusActivation>(), |_| {
        cosmic::iced::stream::channel(10, move |_output: Sender<Message>| async move {
            if let Ok(builder) = zbus::connection::Builder::session() {
                let path: String = format!("/{}", AppModel::APP_ID.replace('.', "/"));
                if let Ok(conn) = builder.build().await
                    && conn.object_server().at(path.clone(), DbusActivation).await == Ok(true)
                    && conn.request_name(AppModel::APP_ID).await.is_ok()
                {
                    log::info!("[activation] serving D-Bus activation interface at {path}");
                }
            }

            // Keep the subscription alive for the applet's lifetime.
            loop {
                cosmic::iced::futures::pending!();
            }
        })
    })
}
