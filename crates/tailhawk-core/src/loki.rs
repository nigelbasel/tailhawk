//! What Tailhawk would say to a Loki server, decided without saying it.
//!
//! Everything here is pure. No socket is opened, no name is resolved, nothing is read from the
//! machine. A [`Request`] is a *description* of one HTTP call — method, URL, body, content type —
//! and the shell is what turns it into bytes on a wire. That split is the same one `menu_frame_of`
//! and `rules_overlay_of` already make, and it exists for the same reason: the decisions worth
//! getting right are the ones a test can reach.
//!
//! The decisions here are `LOKI.md` §7's, and four of them are shaped by the type system rather
//! than checked at the point of use:
//!
//! - **[`Endpoint`] is an enum**, so a path is a variant and there is no function anywhere that
//!   builds a path from a string. §7 asks that no config-supplied path fragment ever reach path
//!   construction; a list of permitted paths would still leave a door, and this has none.
//! - **[`Origin::parse`] refuses** userinfo, a path, a query, a fragment, and any host outside a
//!   deliberate character set, so the only thing a configuration can contribute is a name and a
//!   port — and neither can carry a separator that a URL parser downstream would read as one.
//! - **[`Provenance`] lives inside the [`Origin`]**, set when it is parsed and never afterwards, so
//!   the loopback rule cannot be relaxed by a caller passing the wrong argument.
//! - **[`Origin::may_connect_to`] takes an address the caller has already resolved**, so the SSRF
//!   policy is a value-to-value function and the resolver stays in the shell where it belongs.
//!
//! [`query_range`] builds a **POST with a form body**, never the GET every example uses, because
//! §7's privacy clause is about what a Loki source *sends*: a filter is user-authored text that
//! routinely names a customer, and a GET writes it into every proxy access log on the path.

use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// A nanosecond instant on the server's clock, which is the unit Loki's API speaks in.
pub type Nanos = i64;

/// The endpoints Tailhawk is allowed to contact, as variants rather than as strings.
///
/// The write and administrative endpoints — `/loki/api/v1/push`, `/loki/api/v1/delete`, `/flush`,
/// `/ingester/shutdown` — are absent, and absent is stronger than forbidden. A user pastes the
/// write-scoped token from their shipper's config sooner or later; when they do, the worst this
/// can reach with it is a read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Endpoint {
    /// Records within a time range. The tail's poll asks this repeatedly.
    QueryRange,
    /// The live tail's WebSocket. Listed because it is in the allowlist; nothing opens it yet.
    Tail,
}

impl Endpoint {
    /// The path this endpoint lives at, which is the only place these strings exist.
    pub fn path(self) -> &'static str {
        match self {
            Endpoint::QueryRange => "/loki/api/v1/query_range",
            Endpoint::Tail => "/loki/api/v1/tail",
        }
    }

    /// Whether reaching this endpoint means a WebSocket upgrade, and therefore a `ws`/`wss` URL.
    ///
    /// `LOKI.md` §9 records that nobody has established whether Grafana's datasource proxy forwards
    /// an upgrade at all, so this is a property of the endpoint rather than a promise that the
    /// connection will succeed.
    pub fn is_websocket(self) -> bool {
        matches!(self, Endpoint::Tail)
    }
}

/// Why a base URL was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OriginFault {
    /// No `scheme://` at the front.
    NotAbsolute,
    /// Something other than `http` or `https`.
    UnsupportedScheme,
    /// A `user:password@` section. §7 rejects these outright rather than dropping them.
    Userinfo,
    /// A path. The endpoint owns the path; a bare trailing `/` is accepted and dropped.
    HasPath,
    /// A `?query`.
    HasQuery,
    /// A `#fragment`.
    HasFragment,
    /// Nothing between the scheme and the path.
    EmptyHost,
    /// A character in the host that a URL parser downstream might read as a separator, or a name
    /// that is not shaped like one. See [`Origin::parse`].
    BadHost,
    /// A port that is not a number, or is zero, or does not fit.
    BadPort,
    /// A bracketed host that is not an IPv6 address.
    BadAddress,
    /// Longer than [`Origin::MAX_LEN`].
    TooLong,
}

impl fmt::Display for OriginFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let said = match self {
            OriginFault::NotAbsolute => "needs to start with http:// or https://",
            OriginFault::UnsupportedScheme => "only http and https are understood",
            OriginFault::Userinfo => "must not carry a user name or password",
            OriginFault::HasPath => "must be a host and port only, with no path",
            OriginFault::HasQuery => "must not carry a query string",
            OriginFault::HasFragment => "must not carry a fragment",
            OriginFault::EmptyHost => "names no host",
            OriginFault::BadHost => "names a host that is not a host name",
            OriginFault::BadPort => "names a port that is not a number between 1 and 65535",
            OriginFault::BadAddress => "has brackets around something that is not an IPv6 address",
            OriginFault::TooLong => "is longer than a URL should be",
        };
        f.write_str(said)
    }
}

/// Where a URL came from, which is the whole of the loopback question.
///
/// §7's SSRF controls exist because a `tailhawk.toml` can be sent to someone: the attacker chooses
/// the addresses and the victim's machine does the reaching. A URL the person at the keyboard typed
/// is the opposite situation — they chose the address, and denying them their own development
/// server buys nothing and costs the only endpoint most people can dogfood against.
///
/// This is fixed when an [`Origin`] is parsed and there is no way to change it afterwards, so §7's
/// "no config-settable override" survives contact with a caller that threads its arguments wrongly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provenance {
    /// Read out of a configuration file, which may have arrived by any means at all.
    Imported,
    /// Typed, pasted or picked by the person at the keyboard.
    Typed,
}

/// A base URL reduced to the things Tailhawk is willing to keep: scheme, host, port, and where it
/// came from.
///
/// This is also the credential key §7 asks for. A token is bound to an origin and recomputed from
/// the URL about to be contacted, so a configuration that renames a source cannot borrow the
/// token that belongs to a different host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Origin {
    scheme: String,
    host: String,
    port: u16,
    provenance: Provenance,
}

impl Origin {
    /// The longest base URL that will be looked at. A host name has a documented ceiling far below
    /// this; the cap is here so that parsing is bounded before anything else is decided.
    pub const MAX_LEN: usize = 512;

    /// The longest host name that will be accepted, from the DNS name-length limit.
    pub const MAX_HOST_LEN: usize = 253;

    /// Reduce a base URL to an origin, or say why not.
    ///
    /// A trailing `/` is accepted and dropped, because a person pasting a base URL from a browser
    /// will bring one. Anything after that `/` is [`OriginFault::HasPath`] — the endpoint owns the
    /// path, and this is where §7's rule that no configured fragment reaches path construction is
    /// enforced.
    ///
    /// **The host is held to a character set, and that is not fussiness.** Refusing only `/`, `?`,
    /// `#` and `@` leaves a host that can still carry a `\`, a CR/LF pair or a colon — and every
    /// one of those is read as a separator by something further down the wire. A `\` is a path
    /// separator to a URL parser, so `example.com\loki\api\v1\push` is a *path* by the time it
    /// reaches the network and the `Endpoint` enum has been walked around; a bare CR/LF is a
    /// request-splitting primitive; and a stray colon makes the host that gets checked and the host
    /// that gets connected to two different strings. So the host is either a bracketed address that
    /// parses as IPv6, or a name of ASCII letters, digits, `-` and `.` with no empty label.
    pub fn parse(text: &str, provenance: Provenance) -> Result<Origin, OriginFault> {
        let text = text.trim();
        if text.len() > Self::MAX_LEN {
            return Err(OriginFault::TooLong);
        }
        let (scheme, rest) = text.split_once("://").ok_or(OriginFault::NotAbsolute)?;
        let scheme = scheme.to_ascii_lowercase();
        if scheme != "http" && scheme != "https" {
            return Err(OriginFault::UnsupportedScheme);
        }

        if rest.contains('#') {
            return Err(OriginFault::HasFragment);
        }
        if rest.contains('?') {
            return Err(OriginFault::HasQuery);
        }
        if rest.contains('@') {
            return Err(OriginFault::Userinfo);
        }

        let authority = match rest.split_once('/') {
            Some((before, "")) => before,
            Some(_) => return Err(OriginFault::HasPath),
            None => rest,
        };
        if authority.is_empty() {
            return Err(OriginFault::EmptyHost);
        }

        let (host, port_text) = split_authority(authority)?;
        let host = check_host(host)?;
        let port = match port_text {
            Some(text) => text
                .parse::<u16>()
                .ok()
                .filter(|p| *p > 0)
                .ok_or(OriginFault::BadPort)?,
            None if scheme == "https" => 443,
            None => 80,
        };

        Ok(Origin {
            scheme,
            host,
            port,
            provenance,
        })
    }

    /// `https`, or `http`.
    pub fn scheme(&self) -> &str {
        &self.scheme
    }

    /// The host, lowercased. An IPv6 literal keeps its brackets, as it must to appear in a URL.
    pub fn host(&self) -> &str {
        &self.host
    }

    /// The port, defaulted from the scheme when the URL did not say.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Where this URL came from, fixed when it was parsed.
    pub fn provenance(&self) -> Provenance {
        self.provenance
    }

    /// The host with its brackets removed, which is the form an address parser wants.
    pub fn hostname(&self) -> &str {
        self.host
            .strip_prefix('[')
            .and_then(|h| h.strip_suffix(']'))
            .unwrap_or(&self.host)
    }

    /// The origin as it appears in a URL — the default port for the scheme is left off, because a
    /// URL that names it and one that does not are the same origin and must key the same.
    pub fn base(&self) -> String {
        let default = if self.scheme == "https" { 443 } else { 80 };
        if self.port == default {
            format!("{}://{}", self.scheme, self.host)
        } else {
            format!("{}://{}:{}", self.scheme, self.host, self.port)
        }
    }

    /// The credential key of `LOKI.md` §7 — origin-derived, never name-derived.
    ///
    /// **The tenant is separated by a character the origin cannot contain, and is encoded.** The
    /// obvious `format!("{base}:{tenant}")` collides: a tenant of `3100` against
    /// `https://loki.example.com` produces the same key as no tenant at all against
    /// `https://loki.example.com:3100`, and both halves of that pair come from imported
    /// configuration. §7 requires the key be recomputed from the URL about to be contacted and
    /// matched exactly with no fallback — an exact match against the wrong origin is the same
    /// exfiltration primitive the section was written to close.
    pub fn key(&self, tenant: Option<&str>) -> String {
        format!(
            "tailhawk:loki:{}|{}",
            self.base(),
            form_encode(tenant.unwrap_or_default())
        )
    }

    /// The full URL of one allowed endpoint, in the scheme that endpoint speaks.
    pub fn url(&self, endpoint: Endpoint) -> String {
        if endpoint.is_websocket() {
            let scheme = if self.scheme == "https" { "wss" } else { "ws" };
            let base = self.base();
            let authority = base.split_once("://").map(|(_, a)| a).unwrap_or(&base);
            format!("{}://{}{}", scheme, authority, endpoint.path())
        } else {
            format!("{}{}", self.base(), endpoint.path())
        }
    }

    /// May Tailhawk connect to this address, given where this origin came from?
    ///
    /// Call it again on the address actually connected to — §7 asks for the re-check against DNS
    /// rebinding, and the reason this is a plain function over an [`IpAddr`] is so that calling it
    /// twice costs nothing.
    pub fn may_connect_to(&self, address: IpAddr) -> Result<(), AddressFault> {
        address_verdict(address, self.provenance)
    }
}

/// Split `host:port`, `[v6]:port`, `host` or `[v6]` into its two halves.
fn split_authority(authority: &str) -> Result<(&str, Option<&str>), OriginFault> {
    if let Some(rest) = authority.strip_prefix('[') {
        let close = rest.find(']').ok_or(OriginFault::BadAddress)?;
        let host = &authority[..close + 2];
        let after = &rest[close + 1..];
        return match after.strip_prefix(':') {
            Some(port) => Ok((host, Some(port))),
            None if after.is_empty() => Ok((host, None)),
            None => Err(OriginFault::BadPort),
        };
    }
    match authority.rsplit_once(':') {
        Some((host, port)) => Ok((host, Some(port))),
        None => Ok((authority, None)),
    }
}

/// Hold the host to a shape, and return it lowercased.
fn check_host(host: &str) -> Result<String, OriginFault> {
    if host.is_empty() {
        return Err(OriginFault::EmptyHost);
    }
    if let Some(inner) = host.strip_prefix('[').and_then(|h| h.strip_suffix(']')) {
        return match inner.parse::<Ipv6Addr>() {
            Ok(address) => Ok(format!("[{address}]")),
            Err(_) => Err(OriginFault::BadAddress),
        };
    }
    if host.len() > Origin::MAX_HOST_LEN {
        return Err(OriginFault::TooLong);
    }
    let name = host.to_ascii_lowercase();
    if !name
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.')
    {
        return Err(OriginFault::BadHost);
    }
    let labels = name.strip_suffix('.').unwrap_or(&name);
    if labels
        .split('.')
        .any(|l| l.is_empty() || l.starts_with('-') || l.ends_with('-'))
    {
        return Err(OriginFault::BadHost);
    }
    Ok(name)
}

/// Why an address was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddressFault {
    /// `127.0.0.0/8` or `::1`, from imported configuration.
    Loopback,
    /// `169.254.0.0/16` or `fe80::/10`.
    LinkLocal,
    /// `169.254.169.254`, or AWS's IPv6 instance-metadata address. Never a Loki, always worth
    /// reaching for.
    Metadata,
    /// `0.0.0.0/8` or `::`.
    Unspecified,
    /// A multicast group.
    Multicast,
    /// `255.255.255.255`.
    Broadcast,
}

impl fmt::Display for AddressFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let said = match self {
            AddressFault::Loopback => "is on this machine, and the address came from a file",
            AddressFault::LinkLocal => "is a link-local address",
            AddressFault::Metadata => "is the cloud metadata service",
            AddressFault::Unspecified => "is the unspecified address",
            AddressFault::Multicast => "is a multicast group",
            AddressFault::Broadcast => "is the broadcast address",
        };
        f.write_str(said)
    }
}

/// The IPv4 cloud metadata address. Documented by every major cloud and reachable from inside most
/// of their instances, which is what makes it the first thing an SSRF goes looking for.
const METADATA_V4: Ipv4Addr = Ipv4Addr::new(169, 254, 169, 254);

/// AWS's IPv6 instance-metadata endpoint. A separate address rather than a form of the one above,
/// and published by AWS alone.
const METADATA_V6: Ipv6Addr = Ipv6Addr::new(0xfd00, 0x0ec2, 0, 0, 0, 0, 0, 0x254);

/// May Tailhawk connect to this address?
///
/// Prefer [`Origin::may_connect_to`], which supplies the provenance from the origin rather than
/// from the call site. This is public because the policy is worth testing on its own.
pub fn address_verdict(address: IpAddr, provenance: Provenance) -> Result<(), AddressFault> {
    match address {
        IpAddr::V4(v4) => verdict_v4(v4, provenance),
        IpAddr::V6(v6) => {
            if v6 == METADATA_V6 {
                return Err(AddressFault::Metadata);
            }
            if v6.is_unspecified() {
                return Err(AddressFault::Unspecified);
            }
            if v6.is_loopback() {
                return loopback(provenance);
            }
            if let Some(v4) = embedded_v4(v6) {
                return verdict_v4(v4, provenance);
            }
            if v6.is_multicast() {
                return Err(AddressFault::Multicast);
            }
            if (v6.segments()[0] & 0xffc0) == 0xfe80 {
                return Err(AddressFault::LinkLocal);
            }
            Ok(())
        }
    }
}

fn verdict_v4(v4: Ipv4Addr, provenance: Provenance) -> Result<(), AddressFault> {
    if v4 == METADATA_V4 {
        return Err(AddressFault::Metadata);
    }
    if v4.octets()[0] == 0 {
        return Err(AddressFault::Unspecified);
    }
    if v4.is_broadcast() {
        return Err(AddressFault::Broadcast);
    }
    if v4.is_multicast() {
        return Err(AddressFault::Multicast);
    }
    if v4.is_link_local() {
        return Err(AddressFault::LinkLocal);
    }
    if v4.is_loopback() {
        return loopback(provenance);
    }
    Ok(())
}

fn loopback(provenance: Provenance) -> Result<(), AddressFault> {
    match provenance {
        Provenance::Typed => Ok(()),
        Provenance::Imported => Err(AddressFault::Loopback),
    }
}

/// The IPv4 address inside an IPv6 one, in either the mapped (`::ffff:a.b.c.d`) or the deprecated
/// compatible (`::a.b.c.d`) form.
///
/// Writing an address the long way round is the oldest way past a policy that looks at only one of
/// the two forms, and it defeated this one until it was reviewed. `::` and `::1` are excluded
/// because they are IPv6 addresses in their own right with their own rules, and reading them as
/// `0.0.0.0` and `0.0.0.1` would answer the wrong question.
fn embedded_v4(v6: Ipv6Addr) -> Option<Ipv4Addr> {
    if let Some(v4) = v6.to_ipv4_mapped() {
        return Some(v4);
    }
    if v6.is_unspecified() || v6.is_loopback() {
        return None;
    }
    if v6.segments()[..6] == [0, 0, 0, 0, 0, 0] {
        let o = v6.octets();
        return Some(Ipv4Addr::new(o[12], o[13], o[14], o[15]));
    }
    None
}

/// A half-open window of server time, `[start, end)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Window {
    /// Inclusive.
    pub start: Nanos,
    /// Exclusive.
    pub end: Nanos,
}

impl Window {
    /// Is there any time in here at all?
    pub fn is_empty(self) -> bool {
        self.end <= self.start
    }
}

/// Which end of the window Loki should fill the limit from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Oldest first. What a tail wants, because the limit should cut the *newest* records off and
    /// leave them for the next poll rather than cutting off the ones just missed.
    Forward,
    /// Newest first. What opening a source at "now" wants.
    Backward,
}

impl Direction {
    fn word(self) -> &'static str {
        match self {
            Direction::Forward => "forward",
            Direction::Backward => "backward",
        }
    }
}

/// One HTTP call, described.
///
/// `body` is already form-encoded and `content_type` says so. Nothing here is a secret: the
/// `Authorization` header is attached by the shell at the moment of sending, against the origin it
/// is bound to, so a `Request` can be logged, shown in a dialog or compared in a test.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// `POST` or `GET`.
    pub method: &'static str,
    /// The absolute URL, built from an [`Origin`] and an [`Endpoint`].
    pub url: String,
    /// The form-encoded body, empty for a `GET`.
    pub body: String,
    /// The body's media type, or `None` when there is no body.
    pub content_type: Option<&'static str>,
}

/// The most records one response may carry. Loki's own default is 100 and its ceiling is
/// configurable per deployment; this is Tailhawk's, and it is a cap rather than a request.
pub const MAX_LIMIT: u32 = 5_000;

/// Ask for the records in a window.
///
/// **POST, with the selector in the body.** `LOKI.md` §7: a stream selector is user-authored text
/// that names services, environments and — through a line filter — customers. The GET form puts it
/// in the request line, where every proxy on the path writes it to disk.
pub fn query_range(
    origin: &Origin,
    selector: &str,
    window: Window,
    limit: u32,
    direction: Direction,
) -> Request {
    let limit = limit.clamp(1, MAX_LIMIT);
    let mut body = String::new();
    push_field(&mut body, "query", selector);
    push_field(&mut body, "start", &window.start.to_string());
    push_field(&mut body, "end", &window.end.to_string());
    push_field(&mut body, "limit", &limit.to_string());
    push_field(&mut body, "direction", direction.word());
    Request {
        method: "POST",
        url: origin.url(Endpoint::QueryRange),
        body,
        content_type: Some("application/x-www-form-urlencoded"),
    }
}

fn push_field(body: &mut String, name: &str, value: &str) {
    if !body.is_empty() {
        body.push('&');
    }
    body.push_str(name);
    body.push('=');
    form_encode_into(body, value);
}

/// Percent-encode for `application/x-www-form-urlencoded`.
///
/// This is the form-encoding set and **not** RFC 3986's unreserved set, which is wider: `~` is
/// unreserved in a URL and still encoded here, because a form body is read back by a different
/// rule than a path is. Over-encoding is always safe and under-encoding never is, so where the two
/// sets disagree this takes the narrower one. ASCII letters, digits, `-`, `_` and `.` pass; a
/// space becomes `+`; everything else becomes `%XX` over the UTF-8 bytes.
///
/// `{`, `}`, `"`, `=` and `|` all appear in an ordinary stream selector, and a selector that
/// survives being read back is the whole point.
fn form_encode_into(out: &mut String, value: &str) {
    for byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' => {
                out.push(*byte as char);
            }
            b' ' => out.push('+'),
            other => {
                out.push('%');
                out.push(hex_digit(other >> 4));
                out.push(hex_digit(other & 0x0f));
            }
        }
    }
}

fn hex_digit(nibble: u8) -> char {
    match nibble {
        0..=9 => (b'0' + nibble) as char,
        _ => (b'A' + nibble - 10) as char,
    }
}

/// Percent-encode a value for a form body.
pub fn form_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    form_encode_into(&mut out, value);
    out
}

/// The poll that makes a Loki source a tail.
///
/// `LOKI.md` §6 settles the shape: polling is the correctness mechanism and the `/tail` WebSocket
/// is an accelerator allowed to fail. So this is not scaffolding to be replaced — it is the part
/// that has to be right, and the socket, when it lands, is an optimisation layered over it.
///
/// **The settling band is why this is not just "everything since last time".** A record's timestamp
/// is the emitter's, and it reaches a queryable state some time later; asking right up to `now`
/// returns a window that is still filling, and the records that arrive after the answer are never
/// asked for again. So each poll stops short of `now` by [`Follow::settle`], and the ground it
/// gives up is made back by the next poll rather than lost.
///
/// **[`Follow::reach`] shortens a poll; it never skips one.** A source left paused overnight has a
/// long backlog, and asking for all of it in one question is a way to be refused by the server. So
/// a poll asks for at most `reach` of time *starting from where the last one stopped*, and the
/// backlog is caught up over as many polls as it takes. Clamping the far end rather than the near
/// one is the whole difference between catching up and quietly discarding the night — the first
/// draft of this clamped the near one, and threw away everything but the last five minutes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Follow {
    /// The exclusive end of everything already asked for. The next window starts here.
    pub cursor: Nanos,
    /// How far behind `now` a poll stops. A property of the deployment's ingestion lag; nobody has
    /// measured this one, so it is a parameter and not a constant. Read as zero if negative.
    pub settle: Nanos,
    /// The most time one poll will ask for. Read as one nanosecond if smaller, so that a poll
    /// always makes progress.
    pub reach: Nanos,
}

impl Follow {
    /// One second, in nanoseconds. The unit everything here is expressed in.
    pub const SECOND: Nanos = 1_000_000_000;

    /// Start following from an instant, with a two-second settling band and no single poll asking
    /// for more than five minutes of time.
    pub fn from(cursor: Nanos) -> Follow {
        Follow {
            cursor,
            settle: 2 * Self::SECOND,
            reach: 300 * Self::SECOND,
        }
    }

    /// The window to ask for now, or `None` if the settling band has not been cleared yet.
    ///
    /// Returning `None` is the ordinary case between polls and means *ask again later*, not that
    /// anything is wrong. A caller that treats it as an error will poll a Loki server flat.
    ///
    /// A returned window is never empty and never inverted.
    pub fn next(&self, now: Nanos) -> Option<Window> {
        let horizon = now.saturating_sub(self.settle.max(0));
        if horizon <= self.cursor {
            return None;
        }
        let start = self.cursor;
        let end = horizon.min(start.saturating_add(self.reach.max(1)));
        Some(Window { start, end })
    }

    /// Record that a window was answered in full, so the next one starts where it left off.
    pub fn answered(&mut self, window: Window) {
        self.cursor = self.cursor.max(window.end);
    }

    /// Record that a window came back at the limit, with `newest` the last record's timestamp.
    ///
    /// The rest of that window has not been seen, so the cursor stops after the last record
    /// delivered rather than at the window's end, and the next poll picks the remainder up.
    ///
    /// `newest` is clamped into the window it is reported against. A server whose clock has run
    /// ahead can otherwise walk the cursor past ground no window ever asked for, and one whose
    /// records arrive behind the cursor — which §6 says happens routinely — would otherwise leave
    /// it exactly where it was, re-asking the same truncated window for ever.
    pub fn truncated_at(&mut self, window: Window, newest: Nanos) {
        let last = newest.clamp(window.start, window.end.saturating_sub(1));
        self.cursor = self.cursor.max(last.saturating_add(1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn origin(text: &str) -> Origin {
        Origin::parse(text, Provenance::Typed).expect("should parse")
    }

    fn fault(text: &str) -> OriginFault {
        Origin::parse(text, Provenance::Typed).expect_err("should be refused")
    }

    #[test]
    fn a_base_url_keeps_only_scheme_host_and_port() {
        assert_eq!(
            origin("https://loki.example.com").base(),
            "https://loki.example.com"
        );
        assert_eq!(
            origin("https://loki.example.com/").base(),
            "https://loki.example.com"
        );
        assert_eq!(
            origin("http://127.0.0.1:3100").base(),
            "http://127.0.0.1:3100"
        );
        assert_eq!(
            origin("HTTPS://LOKI.Example.COM").base(),
            "https://loki.example.com"
        );
    }

    #[test]
    fn the_default_port_is_not_written_back_so_two_spellings_key_alike() {
        assert_eq!(
            origin("https://loki.example.com:443").base(),
            origin("https://loki.example.com").base()
        );
        assert_eq!(
            origin("http://loki.example.com:80").base(),
            origin("http://loki.example.com").base()
        );
        assert_ne!(
            origin("https://loki.example.com:3100").base(),
            origin("https://loki.example.com").base()
        );
    }

    #[test]
    fn an_ipv6_literal_keeps_its_brackets_in_the_url_and_loses_them_for_a_resolver() {
        let o = origin("http://[::1]:3100");
        assert_eq!(o.base(), "http://[::1]:3100");
        assert_eq!(o.host(), "[::1]");
        assert_eq!(o.hostname(), "::1");
        assert_eq!(origin("http://[fd00::1]").port(), 80);
        assert_eq!(origin("http://[FD00::0001]").host(), "[fd00::1]");
    }

    #[test]
    fn a_base_url_carrying_anything_but_a_host_is_refused() {
        let cases = [
            ("loki.example.com", OriginFault::NotAbsolute),
            ("ftp://loki.example.com", OriginFault::UnsupportedScheme),
            ("file://loki.example.com", OriginFault::UnsupportedScheme),
            ("https://user:pass@loki.example.com", OriginFault::Userinfo),
            (
                "https://loki.example.com/loki/api/v1/push",
                OriginFault::HasPath,
            ),
            ("https://loki.example.com/anything", OriginFault::HasPath),
            ("https://loki.example.com?a=b", OriginFault::HasQuery),
            ("https://loki.example.com#x", OriginFault::HasFragment),
            ("https://", OriginFault::EmptyHost),
            ("https://:3100", OriginFault::EmptyHost),
            ("https://loki.example.com:0", OriginFault::BadPort),
            ("https://loki.example.com:99999", OriginFault::BadPort),
            ("https://loki.example.com:http", OriginFault::BadPort),
            ("http://[::1", OriginFault::BadAddress),
        ];
        for (text, want) in cases {
            assert_eq!(fault(text), want, "{text}");
        }
        assert_eq!(
            fault(&format!("https://{}", "a".repeat(600))),
            OriginFault::TooLong
        );
    }

    #[test]
    fn a_host_that_could_be_read_as_a_path_or_a_second_request_is_refused() {
        let refused = [
            r"http://loki.example.com\loki\api\v1\push",
            "http://loki.example.com\r\nX-Evil: 1",
            "http://loki.example.com\nX-Evil: 1",
            "http://a b.com",
            "http://a\u{0}b.com",
            "http://a\u{ff0f}b.com",
            "http://loki.example.com%2f.evil.com",
            "http://..",
            "http://.loki.example.com",
            "http://loki..example.com",
            "http://-loki.example.com",
            "http://fd00::1",
            "http://::1:3100",
            "https://h:80:90",
        ];
        for text in refused {
            let got = Origin::parse(text, Provenance::Typed);
            assert!(got.is_err(), "{text:?} parsed as {got:?}");
        }
        assert_eq!(fault("http://[]"), OriginFault::BadAddress);
        assert_eq!(fault("http://[]:3100"), OriginFault::BadAddress);
        assert_eq!(fault("http://[not-an-address]"), OriginFault::BadAddress);
    }

    #[test]
    fn an_ordinary_host_name_is_still_accepted() {
        for text in [
            "https://loki.example.com",
            "https://loki-prod.eu-west-1.example.com",
            "https://loki.example.com.",
            "http://localhost:3100",
            "http://loki:3100",
            "http://10.0.0.7:3100",
        ] {
            assert!(Origin::parse(text, Provenance::Typed).is_ok(), "{text}");
        }
    }

    #[test]
    fn the_endpoint_owns_the_path_so_configuration_cannot_reach_a_write_route() {
        let o = origin("https://loki.example.com");
        assert_eq!(
            o.url(Endpoint::QueryRange),
            "https://loki.example.com/loki/api/v1/query_range"
        );
        assert_eq!(
            o.url(Endpoint::Tail),
            "wss://loki.example.com/loki/api/v1/tail"
        );
        assert_eq!(
            origin("http://127.0.0.1:3100").url(Endpoint::Tail),
            "ws://127.0.0.1:3100/loki/api/v1/tail"
        );
    }

    #[test]
    fn the_credential_key_is_the_origin_and_never_the_name() {
        let o = origin("https://loki.example.com:3100");
        assert_eq!(o.key(None), "tailhawk:loki:https://loki.example.com:3100|");
        assert_eq!(
            o.key(Some("team-a")),
            "tailhawk:loki:https://loki.example.com:3100|team-a"
        );
        assert_eq!(o.key(Some("")), o.key(None));
        assert_ne!(
            o.key(None),
            origin("https://evil.example.com:3100").key(None)
        );
    }

    #[test]
    fn a_tenant_that_looks_like_a_port_cannot_borrow_another_hosts_token() {
        let no_port = origin("https://loki.example.com");
        let with_port = origin("https://loki.example.com:3100");
        assert_ne!(no_port.key(Some("3100")), with_port.key(None));
        assert_ne!(no_port.key(Some("3100|")), with_port.key(Some("")));
    }

    #[test]
    fn the_addresses_that_are_never_a_loki_are_refused_however_the_url_arrived() {
        let never = [
            ("169.254.169.254", AddressFault::Metadata),
            ("169.254.1.1", AddressFault::LinkLocal),
            ("0.0.0.0", AddressFault::Unspecified),
            ("0.1.2.3", AddressFault::Unspecified),
            ("255.255.255.255", AddressFault::Broadcast),
            ("224.0.0.1", AddressFault::Multicast),
            ("fe80::1", AddressFault::LinkLocal),
            ("::", AddressFault::Unspecified),
            ("ff02::1", AddressFault::Multicast),
            ("fd00:ec2::254", AddressFault::Metadata),
        ];
        for (text, want) in never {
            let address: IpAddr = text.parse().unwrap();
            for provenance in [Provenance::Typed, Provenance::Imported] {
                assert_eq!(
                    address_verdict(address, provenance),
                    Err(want),
                    "{text} {provenance:?}"
                );
            }
        }
    }

    #[test]
    fn an_address_written_the_long_way_round_is_the_same_address() {
        let mapped = [
            ("::ffff:169.254.169.254", AddressFault::Metadata),
            ("::169.254.169.254", AddressFault::Metadata),
            ("::ffff:169.254.1.1", AddressFault::LinkLocal),
            ("::ffff:0.0.0.0", AddressFault::Unspecified),
            ("::ffff:255.255.255.255", AddressFault::Broadcast),
            ("::ffff:224.0.0.1", AddressFault::Multicast),
        ];
        for (text, want) in mapped {
            let address: IpAddr = text.parse().unwrap();
            for provenance in [Provenance::Typed, Provenance::Imported] {
                assert_eq!(
                    address_verdict(address, provenance),
                    Err(want),
                    "{text} {provenance:?}"
                );
            }
        }
        for text in ["::ffff:127.0.0.1", "::127.0.0.1"] {
            let address: IpAddr = text.parse().unwrap();
            assert_eq!(
                address_verdict(address, Provenance::Typed),
                Ok(()),
                "typed {text}"
            );
            assert_eq!(
                address_verdict(address, Provenance::Imported),
                Err(AddressFault::Loopback),
                "imported {text}"
            );
        }
    }

    #[test]
    fn loopback_turns_on_where_the_url_came_from() {
        for text in ["127.0.0.1", "127.1.2.3", "::1"] {
            let address: IpAddr = text.parse().unwrap();
            assert_eq!(
                address_verdict(address, Provenance::Typed),
                Ok(()),
                "typed {text}"
            );
            assert_eq!(
                address_verdict(address, Provenance::Imported),
                Err(AddressFault::Loopback),
                "imported {text}"
            );
        }
    }

    #[test]
    fn an_origin_carries_its_own_provenance_so_a_caller_cannot_relax_the_rule() {
        let loopback: IpAddr = "127.0.0.1".parse().unwrap();
        let typed = Origin::parse("http://localhost:3100", Provenance::Typed).unwrap();
        let imported = Origin::parse("http://localhost:3100", Provenance::Imported).unwrap();
        assert_eq!(typed.may_connect_to(loopback), Ok(()));
        assert_eq!(
            imported.may_connect_to(loopback),
            Err(AddressFault::Loopback)
        );
    }

    #[test]
    fn an_ordinary_address_is_allowed_under_both_provenances() {
        for text in [
            "10.1.2.3",
            "192.168.0.5",
            "93.184.216.34",
            "fd00::1",
            "2606:2800:220:1::1",
        ] {
            let address: IpAddr = text.parse().unwrap();
            for provenance in [Provenance::Typed, Provenance::Imported] {
                assert_eq!(
                    address_verdict(address, provenance),
                    Ok(()),
                    "{text} {provenance:?}"
                );
            }
        }
    }

    #[test]
    fn a_query_is_a_post_and_the_selector_is_in_the_body() {
        let request = query_range(
            &origin("https://loki.example.com"),
            r#"{app="checkout"} |= "customer-42""#,
            Window {
                start: 1_000,
                end: 2_000,
            },
            100,
            Direction::Forward,
        );
        assert_eq!(request.method, "POST");
        assert_eq!(
            request.url,
            "https://loki.example.com/loki/api/v1/query_range"
        );
        assert_eq!(
            request.content_type,
            Some("application/x-www-form-urlencoded")
        );
        assert!(!request.url.contains("customer-42"));
        assert!(!request.url.contains('?'));
        assert!(request.body.contains("customer-42"));
    }

    #[test]
    fn a_selector_survives_being_form_encoded() {
        assert_eq!(form_encode(r#"{app="a b"}"#), "%7Bapp%3D%22a+b%22%7D");
        assert_eq!(form_encode("a|~b"), "a%7C%7Eb");
        assert_eq!(form_encode("plain-Text_1.0"), "plain-Text_1.0");
        assert_eq!(form_encode("é"), "%C3%A9");
        assert_eq!(form_encode("100%"), "100%25");
        assert_eq!(form_encode("a+b"), "a%2Bb");
    }

    #[test]
    fn the_body_names_every_parameter_the_query_needs() {
        let request = query_range(
            &origin("http://127.0.0.1:3100"),
            "{app=\"x\"}",
            Window { start: 7, end: 9 },
            250,
            Direction::Backward,
        );
        let fields: Vec<&str> = request.body.split('&').collect();
        assert_eq!(fields[0], "query=%7Bapp%3D%22x%22%7D");
        assert!(fields.contains(&"start=7"));
        assert!(fields.contains(&"end=9"));
        assert!(fields.contains(&"limit=250"));
        assert!(fields.contains(&"direction=backward"));
    }

    #[test]
    fn the_limit_is_capped_rather_than_believed() {
        let ask = |limit| {
            let r = query_range(
                &origin("http://127.0.0.1:3100"),
                "{a=\"b\"}",
                Window { start: 0, end: 1 },
                limit,
                Direction::Forward,
            );
            r.body
                .split('&')
                .find_map(|f| f.strip_prefix("limit="))
                .unwrap()
                .to_owned()
        };
        assert_eq!(ask(0), "1");
        assert_eq!(ask(100), "100");
        assert_eq!(ask(u32::MAX), MAX_LIMIT.to_string());
    }

    #[test]
    fn a_poll_stops_short_of_now_by_the_settling_band() {
        let follow = Follow::from(0);
        let window = follow.next(10 * Follow::SECOND).expect("a window");
        assert_eq!(window.end, 8 * Follow::SECOND);
        assert_eq!(window.start, 0);
    }

    #[test]
    fn a_poll_inside_the_settling_band_asks_for_nothing_at_all() {
        let follow = Follow::from(8 * Follow::SECOND);
        assert_eq!(follow.next(9 * Follow::SECOND), None);
        assert_eq!(follow.next(10 * Follow::SECOND), None);
        assert!(follow.next(10 * Follow::SECOND + 1).is_some());
    }

    #[test]
    fn consecutive_polls_leave_no_gap_and_no_overlap() {
        let mut follow = Follow::from(0);
        let mut asked: Vec<Window> = Vec::new();
        for tick in 1..=8 {
            if let Some(window) = follow.next(tick * Follow::SECOND) {
                asked.push(window);
                follow.answered(window);
            }
        }
        assert!(asked.len() > 1, "the band should not silence every poll");
        for pair in asked.windows(2) {
            assert_eq!(
                pair[0].end, pair[1].start,
                "a gap or an overlap between polls"
            );
        }
    }

    #[test]
    fn a_backlog_longer_than_the_reach_is_caught_up_and_never_skipped() {
        let day = 86_400 * Follow::SECOND;
        let mut follow = Follow::from(0);
        let mut asked: Vec<Window> = Vec::new();
        while let Some(window) = follow.next(day) {
            asked.push(window);
            follow.answered(window);
            assert!(asked.len() < 1_000, "the catch-up should terminate");
        }
        assert!(asked.len() > 1, "a day should take more than one poll");
        assert_eq!(asked[0].start, 0, "the backlog starts where the cursor was");
        for pair in asked.windows(2) {
            assert_eq!(pair[0].end, pair[1].start, "a gap in the catch-up");
        }
        assert_eq!(asked.last().unwrap().end, day - 2 * Follow::SECOND);
    }

    #[test]
    fn one_poll_asks_for_no_more_than_its_reach() {
        let follow = Follow::from(0);
        let window = follow.next(86_400 * Follow::SECOND).expect("a window");
        assert_eq!(window.end - window.start, follow.reach);
    }

    #[test]
    fn a_window_is_never_empty_or_inverted_however_the_fields_are_set() {
        let odd = [
            Follow {
                cursor: 0,
                settle: 0,
                reach: 0,
            },
            Follow {
                cursor: 0,
                settle: 0,
                reach: -5,
            },
            Follow {
                cursor: 0,
                settle: -5,
                reach: 10,
            },
            Follow {
                cursor: -100,
                settle: 0,
                reach: 1,
            },
        ];
        for follow in odd {
            for now in [-10, 0, 1, 10, 1_000] {
                if let Some(window) = follow.next(now) {
                    assert!(!window.is_empty(), "{follow:?} at {now} gave {window:?}");
                    assert!(
                        window.end <= now,
                        "{follow:?} at {now} asked into the future"
                    );
                }
            }
        }
    }

    #[test]
    fn a_truncated_answer_resumes_after_its_last_record_rather_than_skipping_the_rest() {
        let mut follow = Follow::from(0);
        let window = follow.next(10 * Follow::SECOND).expect("a window");
        follow.truncated_at(window, 3 * Follow::SECOND);
        let next = follow.next(10 * Follow::SECOND).expect("more to ask for");
        assert_eq!(next.start, 3 * Follow::SECOND + 1);
        assert_eq!(next.end, window.end);
    }

    #[test]
    fn a_truncated_answer_cannot_walk_the_cursor_past_ground_nobody_asked_for() {
        let mut follow = Follow::from(0);
        let window = follow.next(10 * Follow::SECOND).expect("a window");
        follow.truncated_at(window, 20 * Follow::SECOND);
        assert_eq!(follow.cursor, window.end);
    }

    #[test]
    fn a_truncated_answer_older_than_the_cursor_still_makes_progress() {
        let mut follow = Follow::from(5 * Follow::SECOND);
        let window = follow.next(20 * Follow::SECOND).expect("a window");
        let before = follow.cursor;
        follow.truncated_at(window, 0);
        assert!(
            follow.cursor > before,
            "the poll would re-ask the same window for ever"
        );
        assert!(follow.cursor <= window.end);
    }

    #[test]
    fn the_cursor_never_goes_backwards() {
        let mut follow = Follow::from(5 * Follow::SECOND);
        follow.answered(Window {
            start: 0,
            end: Follow::SECOND,
        });
        assert_eq!(follow.cursor, 5 * Follow::SECOND);
        follow.truncated_at(
            Window {
                start: 0,
                end: Follow::SECOND,
            },
            0,
        );
        assert_eq!(follow.cursor, 5 * Follow::SECOND);
    }

    #[test]
    fn an_empty_window_knows_it_is_empty() {
        assert!(Window { start: 5, end: 5 }.is_empty());
        assert!(Window { start: 5, end: 4 }.is_empty());
        assert!(!Window { start: 4, end: 5 }.is_empty());
    }
}
