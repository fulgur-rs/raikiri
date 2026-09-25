//! A TCP `Connector`/`Transport` for `ureq` that re-derives its own
//! deadline from real elapsed time on every raw socket read/write, instead
//! of trusting `ureq`'s built-in TCP transport to keep a single timeout
//! value fresh across however many raw reads/writes one logical operation
//! needs.
//!
//! `ureq` 3.4.2's own timeout accounting (`Global`/`Connect`/etc.) is
//! correctly based on an absolute deadline, recomputed each time a new
//! logical operation starts (the TLS handshake; each logical read of the
//! response). But its rustls integration snapshots that computed value
//! once per operation and reuses it, unchanged, for every raw socket
//! read/write `rustls`'s own internal retry loop performs to service that
//! one operation — so a server that keeps a slow trickle of bytes arriving
//! inside that frozen window can hold the operation open far past the
//! configured deadline, on any part of the exchange, not only the
//! handshake.
//!
//! This transport closes that gap without touching `ureq` or `rustls`
//! internals: it watches the `NextTimeout` value `ureq` passes to each raw
//! read/write. The first time a given value is seen, that marks the start
//! of a new logical operation, so this transport anchors its own deadline
//! to `now() + timeout.after` at that moment. On every subsequent call —
//! including ones carrying that exact same (frozen, stale) `NextTimeout`
//! value, which is precisely the case `ureq`'s own accounting fails to
//! re-derive — this transport still computes the real remaining time
//! against its own anchored deadline using a fresh clock reading, so the
//! effective timeout genuinely shrinks as wall-clock time passes,
//! regardless of how many raw reads/writes one operation takes.
//!
//! Connection pooling is handled the same way `ureq`'s own transport
//! handles per-request state: `is_open()` (which `ureq` calls both when a
//! pooled connection is checked out for reuse and when it is checked back
//! in) resets this transport's anchored deadline, so a second or later
//! logical request on a reused connection starts deadline tracking fresh
//! rather than inheriting whatever the previous request last saw.

use std::fmt;
use std::io::{self, Read as _, Write as _};
use std::net::TcpStream;
use std::time::{Duration as StdDuration, Instant as StdInstant};

use ureq::Error;
use ureq::Timeout;
use ureq::unversioned::transport::time::Duration as UreqDuration;
use ureq::unversioned::transport::{
    Buffers, ConnectionDetails, Connector, Either, LazyBuffers, NextTimeout, Transport,
};

/// Connector for TCP sockets whose resulting [`Transport`] tracks its own
/// deadline (see the module doc) instead of trusting a single frozen
/// per-operation timeout value. Meant to replace `ureq`'s own
/// `TcpConnector` in the connector chain — same position, same job,
/// different (deadline-safe) `Transport` output.
#[derive(Debug, Default)]
pub(crate) struct DeadlineTcpConnector(());

impl<In: Transport> Connector<In> for DeadlineTcpConnector {
    type Out = Either<In, DeadlineTcpTransport>;

    fn connect(
        &self,
        details: &ConnectionDetails,
        chained: Option<In>,
    ) -> Result<Option<Self::Out>, Error> {
        // cov:ignore: unreachable given this crate's own connector chain
        // (`().chain(DeadlineTcpConnector::default()).chain(RustlsConnector::default())`,
        // see `UreqHttpProvider::new`) — nothing precedes this connector
        // that could produce a transport, so `chained` is always `None`
        // here in practice. Handled anyway because `Connector<In>`'s
        // signature requires it, matching `ureq`'s own `TcpConnector`.
        if chained.is_some() {
            return Ok(chained.map(Either::A));
        }

        let deadline = anchor(StdInstant::now(), details.timeout.after);
        let mut last_err: Option<Error> = None;

        for addr in details.addrs.iter() {
            let per_addr_timeout = match deadline {
                None => None,
                Some(deadline) => match remaining_or_timeout(deadline, details.timeout.reason) {
                    Ok(remaining) => remaining,
                    Err(e) => return Err(last_err.unwrap_or(e)),
                },
            };

            let attempt = match per_addr_timeout {
                Some(t) => TcpStream::connect_timeout(addr, t),
                None => TcpStream::connect(addr),
            };

            match attempt {
                Ok(stream) => {
                    if details.config.no_delay() {
                        // Best-effort: a failure here doesn't invalidate
                        // the connection, just its latency characteristics.
                        let _ = stream.set_nodelay(true);
                    }
                    let buffers = LazyBuffers::new(
                        details.config.input_buffer_size(),
                        details.config.output_buffer_size(),
                    );
                    return Ok(Some(Either::B(DeadlineTcpTransport::new(stream, buffers))));
                }
                // cov:ignore: needs a genuinely black-holed address (no
                // RST, no response) to time out `connect_timeout` itself
                // rather than fail fast — not reproducible deterministically
                // in a sandboxed/CI network without an unroutable address
                // reserved for this purpose.
                Err(e) if e.kind() == io::ErrorKind::TimedOut => {
                    last_err = Some(Error::Timeout(details.timeout.reason));
                }
                Err(e) if is_addr_specific_error(&e) => {
                    // This address specifically refused/was unreachable;
                    // the next resolved address might still work.
                    last_err = Some(e.into());
                }
                // cov:ignore: needs an OS-level connect error outside
                // `is_addr_specific_error`'s set (e.g. a permission error
                // on a privileged port) — platform-dependent, not
                // reproducible deterministically here.
                Err(e) => return Err(e.into()),
            }
        }

        // cov:ignore: the `Resolver` trait's contract guarantees at least
        // one address on a successful resolve (see
        // `ureq::unversioned::resolver::Resolver::resolve`'s doc), so the
        // loop above always sets `last_err` before falling through here —
        // this closure is dead code, kept only to satisfy
        // `unwrap_or_else`'s signature for the case an empty address list
        // would otherwise produce.
        Err(last_err.unwrap_or_else(|| {
            Error::Io(io::Error::new(
                io::ErrorKind::ConnectionRefused,
                "Connection refused",
            ))
        }))
    }
}

/// Whether a failed connect concerns only the address tried, meaning the
/// next resolved address might still succeed. Mirrors `ureq`'s own
/// `TcpConnector` classification (its version of this function is
/// `pub(crate)` to `ureq`, so it isn't reachable from here).
fn is_addr_specific_error(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::ConnectionRefused
            | io::ErrorKind::HostUnreachable
            | io::ErrorKind::NetworkUnreachable
            | io::ErrorKind::AddrNotAvailable
    )
}

/// Anchors a deadline at `now + duration`, or returns `None` if `duration`
/// is `NotHappening` (no timeout configured for this operation — the
/// resulting transport then never sets a socket-level read/write timeout).
///
/// Checks `is_not_happening()` (the only thing this needs from `ureq`'s
/// `Duration` wrapper) before ever dereferencing it to a plain
/// `std::time::Duration`: `NotHappening` derefs to `Duration::from_secs(u64::MAX)`,
/// which would overflow plain `Instant + Duration` arithmetic if added
/// directly rather than special-cased first.
fn anchor(now: StdInstant, duration: UreqDuration) -> Option<StdInstant> {
    if duration.is_not_happening() {
        None
    } else {
        Some(now + *duration)
    }
}

/// The real remaining time until `deadline`, or `Err` if it has already
/// passed. `reason` is only used to label that error.
fn remaining_or_timeout(
    deadline: StdInstant,
    reason: Timeout,
) -> Result<Option<StdDuration>, Error> {
    let now = StdInstant::now();
    if now >= deadline {
        return Err(Error::Timeout(reason));
    }
    Ok(Some(deadline - now))
}

/// Whether an I/O error means the just-attempted read/write ran out of the
/// time budget it was given. `ureq`'s own TCP transport treats both kinds
/// as equivalent (some platforms surface an expired `SO_RCVTIMEO`/
/// `SO_SNDTIMEO` as `WouldBlock` rather than `TimedOut`).
fn is_timeout_like(e: &io::Error) -> bool {
    let kind = e.kind();
    kind == io::ErrorKind::WouldBlock || kind == io::ErrorKind::TimedOut
}

/// TCP transport that re-derives its own deadline from real elapsed time on
/// every raw read/write. See the module doc for why this is necessary and
/// how it works.
pub(crate) struct DeadlineTcpTransport {
    stream: TcpStream,
    buffers: LazyBuffers,
    /// The last `NextTimeout` value seen from `ureq`. A change signals the
    /// start of a new logical operation; no change means this transport is
    /// still inside the same operation `ureq`'s own accounting has already
    /// (correctly) budgeted for, so `deadline` is kept as-is rather than
    /// re-anchored to `timeout.after` again (which would just re-apply the
    /// same stale duration `ureq` handed over).
    last_seen: Option<NextTimeout>,
    /// This operation's real deadline, anchored once when `last_seen` was
    /// first set (or changed), independent of whatever `NextTimeout` value
    /// subsequent calls within the same operation keep repeating.
    deadline: Option<StdInstant>,
    applied_read_timeout: Option<StdDuration>,
    applied_write_timeout: Option<StdDuration>,
}

impl DeadlineTcpTransport {
    fn new(stream: TcpStream, buffers: LazyBuffers) -> Self {
        Self {
            stream,
            buffers,
            last_seen: None,
            deadline: None,
            applied_read_timeout: None,
            applied_write_timeout: None,
        }
    }

    /// The real remaining time until this call's effective deadline,
    /// anchoring a new deadline first if `timeout` marks the start of a
    /// new logical operation. See the module doc.
    fn remaining(&mut self, timeout: NextTimeout) -> Result<Option<StdDuration>, Error> {
        if self.last_seen != Some(timeout) {
            self.last_seen = Some(timeout);
            self.deadline = anchor(StdInstant::now(), timeout.after);
        }

        match self.deadline {
            None => Ok(None),
            Some(deadline) => remaining_or_timeout(deadline, timeout.reason),
        }
    }
}

impl Transport for DeadlineTcpTransport {
    fn buffers(&mut self) -> &mut dyn Buffers {
        &mut self.buffers
    }

    fn transmit_output(&mut self, amount: usize, timeout: NextTimeout) -> Result<(), Error> {
        let remaining = self.remaining(timeout)?;
        if self.applied_write_timeout != remaining {
            self.stream.set_write_timeout(remaining)?;
            self.applied_write_timeout = remaining;
        }

        let output = &self.buffers.output()[..amount];
        match self.stream.write_all(output) {
            Ok(()) => Ok(()),
            Err(e) if is_timeout_like(&e) => Err(Error::Timeout(timeout.reason)),
            // cov:ignore: needs a genuine non-timeout OS-level write error
            // (e.g. a peer RST) on an otherwise-healthy loopback socket.
            // Forcing that deterministically needs `TcpStream::set_linger`,
            // still unstable on this toolchain (rust-lang/rust#88494), or
            // an extra dependency (`socket2`) just for this one test.
            Err(e) => Err(e.into()),
        }
    }

    fn await_input(&mut self, timeout: NextTimeout) -> Result<bool, Error> {
        let remaining = self.remaining(timeout)?;
        if self.applied_read_timeout != remaining {
            self.stream.set_read_timeout(remaining)?;
            self.applied_read_timeout = remaining;
        }

        let input = self.buffers.input_append_buf();
        let amount = match self.stream.read(input) {
            Ok(v) => v,
            Err(e) if is_timeout_like(&e) => return Err(Error::Timeout(timeout.reason)),
            // cov:ignore: same limitation as `transmit_output`'s generic
            // arm above — needs a genuine non-timeout OS-level read error
            // on an otherwise-healthy loopback socket, not reproducible
            // deterministically without an unstable API or an extra
            // dependency.
            Err(e) => return Err(e.into()),
        };
        self.buffers.input_appended(amount);

        Ok(amount > 0)
    }

    fn is_open(&mut self) -> bool {
        // Reset deadline tracking: `ureq` calls `is_open` both when a
        // pooled connection is checked out for reuse and when it's checked
        // back in, so a reused connection's next logical request starts
        // this transport's deadline tracking fresh instead of inheriting
        // whatever the previous request last saw.
        self.last_seen = None;
        self.deadline = None;
        probe(&mut self.stream)
    }
}

impl fmt::Debug for DeadlineTcpTransport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DeadlineTcpTransport")
            .field("addr", &self.stream.peer_addr().ok())
            .finish()
    }
}

/// Non-blocking peek to check the connection is still usable, without
/// consuming any pending data. Mirrors `ureq`'s own TCP transport probe
/// (also `pub(crate)` to `ureq`, so not reachable from here): a `WouldBlock`
/// on a non-blocking read means nothing is pending and the socket is
/// healthy; unsolicited bytes or an error mean it should not be reused.
fn probe(stream: &mut TcpStream) -> bool {
    // cov:ignore: `set_nonblocking` failing on an already-connected,
    // still-open socket is not something a test can force deterministically
    // (it requires the underlying fd itself to be invalid or closed
    // out-of-band, which would also break every other operation on it).
    if stream.set_nonblocking(true).is_err() {
        return false;
    }

    let mut buf = [0u8; 1];
    let healthy = matches!(
        stream.read(&mut buf),
        Err(e) if e.kind() == io::ErrorKind::WouldBlock
    );

    let _ = stream.set_nonblocking(false);
    healthy
}

#[cfg(test)]
mod tests;
