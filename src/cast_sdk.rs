// SPDX-License-Identifier: MPL-2.0

//! Wraps the `fcast-sender-sdk` (pinned to the revision shipped with the
//! official `org.fcast.Sender` flatpak) for casting to a discovered
//! `FCast` receiver.
//!
//! The SDK speaks the `FCast` control protocol over TCP (port 46899 by
//! default). Connecting is non-blocking: the SDK runs its own async worker on
//! a runtime owned by the [`CastContext`], and reports state changes through
//! a [`DeviceEventHandler`]. This module translates those callbacks into
//! plain [`CastEvent`]s on an mpsc channel that the UI can turn into a stream
//! of iced messages.

use std::net::IpAddr as StdIpAddr;
use std::sync::Arc;

use cosmic::iced::futures::channel::mpsc;
use fcast_sender_sdk::IpAddr;
use fcast_sender_sdk::context::CastContext;
use fcast_sender_sdk::device::{
    ApplicationInfo, CastingDevice, DeviceConnectionState, DeviceEventHandler, DeviceInfo,
    KeyEvent, LoadRequest, MediaEvent, PlaybackState, Source,
};

use crate::fcast::FcastReceiver;
use crate::fl;

/// The interval between the SDK's automatic reconnect attempts, in
/// milliseconds.
const RECONNECT_INTERVAL_MILLIS: u64 = 5_000;

/// An event from the cast control connection to a receiver.
///
/// Deliberately carries only plain data so the UI layer does not depend on
/// the SDK's types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CastEvent {
    /// The control connection was established. `local_addr` is the address
    /// of *our* side of the connection — the address the receiver can reach
    /// us at on the local network.
    Connected { local_addr: StdIpAddr },
    /// The control connection was lost, either by the receiver or because we
    /// stopped the cast.
    Disconnected,
    /// The playback state on the receiver changed.
    Playback { playing: bool },
    /// The receiver reported a playback error.
    Error(String),
}

/// Owns the connection to a receiver.
///
/// Kept alive by the app model for the duration of the cast. Dropping it
/// drops the SDK's device and runtime, which tears down the connection.
pub struct CastHandle {
    /// The SDK context; owns the async runtime the device worker runs on.
    _context: CastContext,
    /// The casting device (the connected receiver).
    device: Arc<dyn CastingDevice>,
    /// The receiver's display name.
    name: String,
}

impl CastHandle {
    /// The receiver's display name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Ask the receiver to fetch and play `url` as `content_type`.
    pub fn load(&self, url: String, content_type: String) -> Result<(), String> {
        self.device
            .load(LoadRequest::Url {
                content_type,
                url,
                resume_position: None,
                speed: None,
                volume: None,
                metadata: None,
                request_headers: None,
            })
            .map_err(|error| error.to_string())
    }

    /// Stop playback on the receiver and drop the connection.
    pub fn stop(&self) {
        let _ = self.device.stop_playback();
        let _ = self.device.disconnect();
    }
}

/// Connects to `receiver` and returns a handle together with the receiver
/// side of the event channel.
///
/// The returned handle must be kept alive while the cast runs; dropping it
/// disconnects. Events are delivered to `events` on the SDK's worker thread
/// and can be turned into an iced task with [`event_stream`].
pub fn connect(
    receiver: &FcastReceiver,
    events: mpsc::UnboundedSender<CastEvent>,
) -> anyhow::Result<CastHandle> {
    let addr = receiver
        .addr
        .ok_or_else(|| anyhow::anyhow!("receiver has no network address"))?;

    let context = CastContext::new()?;
    let info = DeviceInfo::fcast(
        receiver.name.clone(),
        vec![IpAddr::from(&addr)],
        receiver.port,
    );
    let device = context.create_device_from_info(info);

    device
        .connect(
            Some(ApplicationInfo {
                name: env!("CARGO_PKG_NAME").to_owned(),
                version: env!("CARGO_PKG_VERSION").to_owned(),
                display_name: fl!("app-title"),
            }),
            Arc::new(CastEventHandler { events }),
            RECONNECT_INTERVAL_MILLIS,
        )
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;

    Ok(CastHandle {
        _context: context,
        device,
        name: receiver.name.clone(),
    })
}

/// The receiving side of a [`connect`] call, as a stream of events.
pub fn event_stream(
    rx: mpsc::UnboundedReceiver<CastEvent>,
) -> impl cosmic::iced::futures::Stream<Item = CastEvent> {
    rx
}

/// Bridges the SDK's callback interface onto the event channel.
struct CastEventHandler {
    events: mpsc::UnboundedSender<CastEvent>,
}

impl DeviceEventHandler for CastEventHandler {
    fn connection_state_changed(&self, state: DeviceConnectionState) {
        let event = match state {
            DeviceConnectionState::Connected {
                local_addr,
                used_remote_addr: _,
            } => CastEvent::Connected {
                local_addr: StdIpAddr::from(&local_addr),
            },
            // The UI shows a pending state while connecting or reconnecting.
            DeviceConnectionState::Connecting | DeviceConnectionState::Reconnecting => return,
            DeviceConnectionState::Disconnected => CastEvent::Disconnected,
        };
        let _ = self.events.unbounded_send(event);
    }

    fn volume_changed(&self, _volume: f64) {}

    fn time_changed(&self, _time: f64) {}

    fn playback_state_changed(&self, state: PlaybackState) {
        let _ = self.events.unbounded_send(CastEvent::Playback {
            playing: matches!(state, PlaybackState::Playing),
        });
    }

    fn duration_changed(&self, _duration: f64) {}

    fn speed_changed(&self, _speed: f64) {}

    fn source_changed(&self, _source: Source) {}

    fn key_event(&self, _event: KeyEvent) {}

    fn media_event(&self, _event: MediaEvent) {}

    fn playback_error(&self, message: String) {
        let _ = self.events.unbounded_send(CastEvent::Error(message));
    }
}
