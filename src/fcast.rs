// SPDX-License-Identifier: MPL-2.0

//! `FCast` wireless receiver discovery over `mDNS`.
//!
//! `FCast` receivers advertise themselves on the local network as
//! `_fcast._tcp` services on port 46899. This module watches for those
//! advertisements so the applet can list the receivers a user could cast to.
//!
//! Sending (playback) is intentionally not implemented yet: `FCast` has no
//! programmatic sender API for desktop environments — see
//! <https://github.com/futo-org/fcast/issues/62>.

use std::collections::HashMap;
use std::net::IpAddr;

use cosmic::iced::futures::channel::mpsc;
use cosmic::iced::futures::stream::{self, BoxStream, StreamExt};
use mdns_sd::{ResolvedService, ScopedIp, ServiceDaemon, ServiceEvent};

/// The `mDNS` service type advertised by `FCast` receivers.
const SERVICE_TYPE: &str = "_fcast._tcp.local.";

/// A wireless `FCast` receiver discovered on the local network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FcastReceiver {
    /// Instance name, e.g. `FCast-ABCDEF Android TV`.
    pub name: String,
    /// The receiver's hostname.
    pub host: String,
    /// A reachable address, when the advertisement carried one.
    pub addr: Option<IpAddr>,
    /// The `FCast` control port (default 46899).
    pub port: u16,
}

/// Identity of the discovery subscription; keeps it alive across updates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DiscoverId;

/// Returns a stream that emits the current list of `FCast` receivers whenever
/// the network view changes (a receiver appeared or disappeared).
//
// The builder signature is fixed by `Subscription::run_with` (`fn(&D) -> S`),
// so the `&DiscoverId` parameter cannot be passed by value.
#[must_use]
#[allow(clippy::trivially_copy_pass_by_ref)]
pub fn discover(_: &DiscoverId) -> BoxStream<'static, Vec<FcastReceiver>> {
    let daemon = match ServiceDaemon::new() {
        Ok(daemon) => daemon,
        Err(error) => {
            log::warn!("failed to start the mDNS daemon: {error}");
            return stream::empty().boxed();
        }
    };

    let events = match daemon.browse(SERVICE_TYPE) {
        Ok(events) => events,
        Err(error) => {
            log::warn!("failed to browse {SERVICE_TYPE}: {error}");
            return stream::empty().boxed();
        }
    };

    // Bridge the blocking mDNS event channel onto an async channel. The
    // daemon is kept alive by the stream state below.
    let (tx, rx) = mpsc::unbounded();
    std::thread::spawn(move || {
        while let Ok(event) = events.recv() {
            if tx.unbounded_send(event).is_err() {
                break;
            }
        }
    });

    stream::unfold(
        State {
            _daemon: daemon,
            events: rx,
            receivers: HashMap::new(),
        },
        |mut state| async move {
            let event = state.events.next().await?;
            apply_event(&mut state.receivers, &event);
            Some((snapshot(&state.receivers), state))
        },
    )
    .boxed()
}

/// The state of the discovery stream.
struct State {
    /// Keeps the mDNS responder alive for the lifetime of the stream.
    _daemon: ServiceDaemon,
    /// Bridged mDNS events.
    events: mpsc::UnboundedReceiver<ServiceEvent>,
    /// Receivers keyed by their full mDNS name.
    receivers: HashMap<String, FcastReceiver>,
}

/// Applies a single mDNS event to the receiver map.
fn apply_event(receivers: &mut HashMap<String, FcastReceiver>, event: &ServiceEvent) {
    match event {
        ServiceEvent::ServiceResolved(resolved) => {
            if let Some((fullname, receiver)) = FcastReceiver::from_resolved(resolved) {
                receivers.insert(fullname, receiver);
            }
        }
        ServiceEvent::ServiceRemoved(_, fullname) => {
            receivers.remove(fullname);
        }
        _ => {}
    }
}

/// The display name of a receiver, derived from its full mDNS name.
fn instance_name(fullname: &str) -> String {
    fullname
        .strip_suffix(SERVICE_TYPE)
        .map_or(fullname, |name| name.trim_end_matches('.'))
        .to_owned()
}

/// The receivers sorted by name, for a stable list.
fn snapshot(receivers: &HashMap<String, FcastReceiver>) -> Vec<FcastReceiver> {
    let mut list: Vec<FcastReceiver> = receivers.values().cloned().collect();
    list.sort_by(|a, b| a.name.cmp(&b.name));
    list
}

impl FcastReceiver {
    /// Builds a receiver from a resolved mDNS service, keyed by its full name.
    fn from_resolved(resolved: &ResolvedService) -> Option<(String, FcastReceiver)> {
        if !resolved.is_valid() {
            return None;
        }

        let mut addrs: Vec<IpAddr> = resolved
            .get_addresses()
            .iter()
            .map(ScopedIp::to_ip_addr)
            .collect();
        addrs.sort_unstable();
        // Prefer IPv4; multicast DNS is most commonly IPv4 on home networks.
        let addr = addrs
            .iter()
            .find(|addr| addr.is_ipv4())
            .or_else(|| addrs.first());

        let fullname = resolved.get_fullname().to_owned();
        let receiver = FcastReceiver {
            name: instance_name(&fullname),
            host: resolved.get_hostname().to_owned(),
            addr: addr.copied(),
            port: resolved.get_port(),
        };
        Some((fullname, receiver))
    }
}

#[cfg(test)]
mod tests {
    use super::{FcastReceiver, instance_name, snapshot};
    use std::collections::HashMap;

    #[test]
    fn strips_service_type_from_fullname() {
        assert_eq!(
            instance_name("FCast-ABCDEF Android TV._fcast._tcp.local."),
            "FCast-ABCDEF Android TV"
        );
        assert_eq!(instance_name("laptop._fcast._tcp.local."), "laptop");
        // Names that do not match the service type are kept as-is.
        assert_eq!(instance_name("other._tcp.local."), "other._tcp.local.");
    }

    #[test]
    fn snapshot_sorts_by_name() {
        let receiver = |name: &str| FcastReceiver {
            name: name.to_owned(),
            host: format!("{name}.local"),
            addr: None,
            port: 46899,
        };
        let mut map = HashMap::new();
        map.insert("b._fcast._tcp.local.".to_owned(), receiver("B"));
        map.insert("a._fcast._tcp.local.".to_owned(), receiver("A"));

        let snapshot = snapshot(&map);
        let names: Vec<&str> = snapshot.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["A", "B"]);
    }
}
