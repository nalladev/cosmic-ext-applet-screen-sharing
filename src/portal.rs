// SPDX-License-Identifier: MPL-2.0

//! Screen capture through the `XDG` Desktop Portal.
//!
//! Both the wired share flow (`app.rs`) and the wireless cast flow
//! (`cast_screencast.rs`) capture the screen through the same screencast
//! portal session, so the portal plumbing lives here.

use std::sync::Arc;

use ashpd::desktop::screencast::{
    CursorMode, Screencast, SelectSourcesOptions, SourceType, StartCastOptions,
};
use ashpd::desktop::{CreateSessionOptions, PersistMode, Session};

/// A handle to an active screen cast portal session, shared between the
/// message that starts the capture and the model that later stops it.
pub type SessionHandle = Arc<Session<Screencast>>;

/// A live capture stream from the screencast portal.
#[derive(Debug, Clone)]
pub struct ShareStream {
    /// The `PipeWire` node id of the capture stream.
    pub node_id: u32,
    /// The size of the captured stream in compositor coordinates, when the
    /// portal reports one.
    pub size: Option<(i32, i32)>,
}

/// Runs a screen cast session through the `XDG` Desktop Portal and returns
/// the `PipeWire` node id of the capture stream, together with the session
/// handle that must be kept alive (and later closed) to stop the share.
pub async fn run_screencast(
    source: SourceType,
) -> anyhow::Result<(ShareStream, Session<Screencast>)> {
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
