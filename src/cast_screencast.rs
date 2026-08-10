// SPDX-License-Identifier: MPL-2.0

//! Serves the captured screen to an `FCast` receiver over plain TCP.
//!
//! `FCast` receivers fetch and play a `LoadRequest::Url` stream with a plain
//! HTTP `GET`, so the sender only has to stand up a small HTTP server on the
//! local network. This module owns that server: it binds an ephemeral port,
//! waits for the receiver's request, and (once the capture pipeline feeds
//! encoded frames into it — see below) streams them back.
//!
//! The stream body is a placeholder today: the capture side produces raw
//! `PipeWire` buffers, which receivers cannot play directly. Encoding them
//! into a playable container (e.g. WebM/VP8) is future work, so each request
//! currently gets an empty response. The connection handling here is the
//! real shape of the final server; only the body needs to change.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

/// The path the receiver requests the stream from.
const STREAM_PATH: &str = "/stream";

/// Serves one `HTTP` connection: reads the request head, then answers with
/// an empty body until the capture pipeline is wired up.
fn serve(mut stream: TcpStream) {
    if read_request_head(&mut stream).is_err() {
        return;
    }
    let response = concat!(
        "HTTP/1.1 200 OK\r\n",
        "Content-Type: video/webm\r\n",
        "Content-Length: 0\r\n",
        "Connection: close\r\n",
        "\r\n",
    );
    let _ = stream.write_all(response.as_bytes());
}

/// Reads bytes until the end of the request head (`\r\n\r\n`), so the client
/// is not left waiting for its request to be consumed.
fn read_request_head(stream: &mut TcpStream) -> std::io::Result<()> {
    const END: &[u8] = b"\r\n\r\n";
    let mut byte = [0u8; 1];
    let mut matched = 0;
    while matched < END.len() {
        let read = stream.read(&mut byte)?;
        if read == 0 {
            return Ok(());
        }
        if byte[0] == END[matched] {
            matched += 1;
        } else {
            matched = usize::from(byte[0] == b'\r');
        }
    }
    Ok(())
}

/// A local `HTTP` server that streams the captured screen to a receiver.
///
/// Drop the streamer (or stop the cast) to shut the server down.
pub struct TcpStreamer {
    /// The bound local address; the receiver fetches
    /// `http://{ip}:{port}{STREAM_PATH}`.
    addr: SocketAddr,
    /// Set when the server should stop accepting connections.
    stop: Arc<AtomicBool>,
    /// The accept-loop thread, joined on drop.
    handle: Option<thread::JoinHandle<()>>,
}

impl TcpStreamer {
    /// Binds an ephemeral port and starts serving the stream.
    ///
    /// # Errors
    ///
    /// Returns an error if no local port can be bound.
    pub fn start() -> anyhow::Result<Self> {
        let listener = TcpListener::bind(("0.0.0.0", 0))?;
        let addr = listener.local_addr()?;
        let stop = Arc::new(AtomicBool::new(false));
        let handle = thread::Builder::new()
            .name("fcast-stream-server".to_owned())
            .spawn({
                let stop = Arc::clone(&stop);
                move || accept_loop(&listener, &stop)
            })?;

        Ok(Self {
            addr,
            stop,
            handle: Some(handle),
        })
    }

    /// The stream URL, built from the address the receiver can reach us at.
    pub fn url(&self, local_ip: std::net::IpAddr) -> String {
        let host = match local_ip {
            std::net::IpAddr::V4(ip) => ip.to_string(),
            std::net::IpAddr::V6(ip) => format!("[{ip}]"),
        };
        format!("http://{host}:{}{STREAM_PATH}", self.addr.port())
    }
}

impl Drop for TcpStreamer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // Unblock the accept loop by connecting to our own listener.
        let _ = TcpStream::connect(self.addr);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// Accepts connections until told to stop.
fn accept_loop(listener: &TcpListener, stop: &Arc<AtomicBool>) {
    for stream in listener.incoming() {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        let Ok(stream) = stream else {
            continue;
        };
        thread::Builder::new()
            .name("fcast-stream-connection".to_owned())
            .spawn(move || serve(stream))
            .ok();
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::{IpAddr, Ipv4Addr};

    use super::{STREAM_PATH, TcpStreamer};

    /// The stream server answers a `GET` for the stream path with an `HTTP`
    /// response for the placeholder body.
    #[test]
    fn serves_the_stream_endpoint() {
        let streamer = TcpStreamer::start().expect("streamer should start");
        let url = streamer.url(IpAddr::V4(Ipv4Addr::LOCALHOST));
        let host_port = url
            .strip_prefix("http://")
            .and_then(|rest| rest.strip_suffix(STREAM_PATH))
            .expect("stream url shape");
        let addr: std::net::SocketAddr = host_port.parse().expect("valid address");

        let mut stream = std::net::TcpStream::connect(addr).expect("connect to streamer");
        stream
            .write_all(b"GET /stream HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .expect("send request");
        let mut response = String::new();
        stream.read_to_string(&mut response).expect("read response");

        assert!(response.starts_with("HTTP/1.1 200 OK"));
        assert!(response.contains("video/webm"));
    }
}
