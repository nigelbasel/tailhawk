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
//! - **[`Origin::parse`] refuses** userinfo, a query, a fragment, any host outside a deliberate
//!   character set, and any path that is not a plain mount prefix — so a configuration can
//!   contribute a name, a port and a mount point, and nothing that a URL parser downstream would
//!   read as a separator.
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
    Tail,
    /// The values one label has taken. The picker asks this once when it opens.
    LabelValues,
}

impl Endpoint {
    /// The path this endpoint lives at, which is the only place these strings exist.
    pub fn path(self) -> &'static str {
        match self {
            Endpoint::QueryRange => "/loki/api/v1/query_range",
            Endpoint::Tail => "/loki/api/v1/tail",
            Endpoint::LabelValues => "/loki/api/v1/label",
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
    /// A path that is not a usable mount prefix — see [`Origin::parse`].
    BadPrefix,
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
            OriginFault::BadPrefix => "carries a path that is not a mount prefix",
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
    prefix: String,
    provenance: Provenance,
}

impl Origin {
    /// The longest base URL that will be looked at. A host name has a documented ceiling far below
    /// this; the cap is here so that parsing is bounded before anything else is decided.
    pub const MAX_LEN: usize = 512;

    /// The longest host name that will be accepted, from the DNS name-length limit.
    pub const MAX_HOST_LEN: usize = 253;

    /// The longest mount prefix that will be accepted.
    pub const MAX_PREFIX_LEN: usize = 128;

    /// Reduce a base URL to an origin, or say why not.
    ///
    /// A trailing `/` is accepted and dropped, because a person pasting a base URL from a browser
    /// will bring one. Anything after that `/` is a **mount prefix**, held to [`check_prefix`]'s
    /// shape. The [`Endpoint`] enum still owns the endpoint's own path, so §7's rule that no
    /// configured fragment reaches path *construction* holds: a configuration says where Loki is
    /// mounted, never what is asked of it.
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

        let (authority, prefix) = match rest.split_once('/') {
            Some((before, after)) => (before, check_prefix(after)?),
            None => (rest, String::new()),
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
            prefix,
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

    /// The mount prefix, with no surrounding slashes. Empty when the base URL named none.
    pub fn prefix(&self) -> &str {
        &self.prefix
    }

    /// The origin as it appears in a URL — the default port for the scheme is left off, because a
    /// URL that names it and one that does not are the same origin and must key the same.
    ///
    /// **The mount prefix is not part of this.** An origin is a scheme, a host and a port; the
    /// prefix is where Loki happens to be mounted on that origin, and a credential is bound to the
    /// origin rather than to the mount. Two sources on the same host at different prefixes are the
    /// same security principal and must key alike.
    pub fn base(&self) -> String {
        let default = if self.scheme == "https" { 443 } else { 80 };
        if self.port == default {
            format!("{}://{}", self.scheme, self.host)
        } else {
            format!("{}://{}:{}", self.scheme, self.host, self.port)
        }
    }

    /// The mount prefix as it appears in a URL — `/loki`, or empty.
    fn mount(&self) -> String {
        if self.prefix.is_empty() {
            String::new()
        } else {
            format!("/{}", self.prefix)
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
    ///
    /// The mount prefix sits between the origin and the endpoint's own path, which is why a real
    /// deployment behind a prefix-stripping proxy produces a doubled-looking segment. That is
    /// correct: the proxy consumes the first and Loki is served the second.
    pub fn url(&self, endpoint: Endpoint) -> String {
        if endpoint.is_websocket() {
            let scheme = if self.scheme == "https" { "wss" } else { "ws" };
            let base = self.base();
            let authority = base.split_once("://").map(|(_, a)| a).unwrap_or(&base);
            format!(
                "{}://{}{}{}",
                scheme,
                authority,
                self.mount(),
                endpoint.path()
            )
        } else {
            format!("{}{}{}", self.base(), self.mount(), endpoint.path())
        }
    }

    /// The origin's own URL — base plus mount prefix, with no endpoint after it.
    ///
    /// For a Loki base this is not a useful address; it exists for the **token endpoint**, whose
    /// entire path is configuration rather than one of [`Endpoint`]'s. Parsing it as an origin is
    /// what subjects a provider's published token URL to the same §7 checks as the query URL:
    /// scheme, host charset, length, no userinfo, no query, no fragment.
    pub fn mounted_url(&self) -> String {
        format!("{}{}", self.base(), self.mount())
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

/// Hold a mount prefix to a shape, and return it with no leading or trailing slash.
///
/// **A prefix is a mount point, not a path.** Loki is commonly reached through a reverse proxy that
/// matches a prefix, strips it, and forwards the rest — so a real base URL is not always bare
/// `scheme://host:port`, and refusing every path outright would leave Tailhawk unable to reach such
/// a deployment at all. §7's rule is that no config-supplied fragment may reach *path construction*,
/// and that survives: the [`Endpoint`] enum remains the only thing that decides the endpoint's own
/// path, and this is prepended to it after being held to a character set with no way to escape
/// upwards.
///
/// Empty is fine and is the ordinary case. `.` and `..` segments are refused rather than resolved,
/// because resolving them is how a prefix becomes a way to reach a different path.
fn check_prefix(text: &str) -> Result<String, OriginFault> {
    let trimmed = text.trim_matches('/');
    if trimmed.is_empty() {
        return Ok(String::new());
    }
    if trimmed.len() > Origin::MAX_PREFIX_LEN {
        return Err(OriginFault::TooLong);
    }
    let ok = |b: u8| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~' | b'/');
    if !trimmed.bytes().all(ok) {
        return Err(OriginFault::BadPrefix);
    }
    if trimmed
        .split('/')
        .any(|seg| seg.is_empty() || seg == "." || seg == "..")
    {
        return Err(OriginFault::BadPrefix);
    }
    Ok(trimmed.to_owned())
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

/// The token a client-credentials exchange returned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    /// The bearer value. **Not `Debug`-printed by this type's own doc contract** — it is here
    /// because it has to be somewhere, and the shell is expected to hand it straight to `send` and
    /// drop it.
    pub access_token: String,
    /// Seconds the token is good for, as the server said. Zero when it did not say.
    pub expires_in: u64,
    /// The refresh token, when the server issued one — which it does for a user sign-in that asked
    /// for `offline_access`, and does not for a client-credentials exchange, where re-presenting
    /// the secret is the refresh. **A credential**, stored like one.
    pub refresh_token: Option<String>,
}

/// The request that exchanges a client secret for a bearer token — **without the secret**.
///
/// **The secret is deliberately absent, and that is the whole design of this function.**
/// [`Request`]'s own contract is that nothing in it is a secret, so it may be logged, shown in a
/// dialog or compared in a test. Putting `client_secret=…` in `body` would quietly break that for
/// every existing reader of a `Request`. The secret is appended by the transport at the moment of
/// sending, against the origin it is bound to — exactly as the `Authorization` header already is.
///
/// **The token endpoint is a second origin and gets §7's controls.** Its whole URL is configuration,
/// unlike a Loki endpoint whose path is one of ours: OAuth providers publish different paths
/// (`/connect/token`, `/oauth/token`, `/oauth2/v2.0/token`), so the path arrives as the origin's
/// mount prefix and is charset-checked, length-capped and refused a query, fragment or userinfo by
/// [`Origin::parse`] like any other.
pub fn token_request(origin: &Origin, client_id: &str, scope: &str) -> Request {
    let mut body = format!(
        "grant_type=client_credentials&client_id={}",
        form_encode(client_id)
    );
    if !scope.is_empty() {
        body.push_str(&format!("&scope={}", form_encode(scope)));
    }
    Request {
        method: "POST",
        url: origin.mounted_url(),
        body,
        content_type: Some("application/x-www-form-urlencoded"),
    }
}

/// Reads `access_token` and `expires_in` out of a token response.
///
/// **A hand-written scan rather than a parser**, for the same reason `lokiwire.rs` gives: there is
/// no JSON crate in the tree, and this needs two fields from a small, flat document. It is bounded
/// by [`MAX_TOKEN_RESPONSE`] before anything is looked at, so a server that answers with a
/// gigabyte cannot make this the expensive part.
///
/// Returns `None` when there is no `access_token` — including when the server sent an OAuth error
/// document, which is the common case for a wrong secret and is *not* something to guess around.
pub fn token_from_json(body: &str) -> Option<Token> {
    if body.len() > MAX_TOKEN_RESPONSE {
        return None;
    }
    let access_token = json_string(body, "access_token")?;
    if access_token.is_empty() {
        return None;
    }
    Some(Token {
        access_token,
        expires_in: json_number(body, "expires_in").unwrap_or(0),
        refresh_token: json_string(body, "refresh_token").filter(|t| !t.is_empty()),
    })
}

/// The largest token response that will be looked at. A JWT is a few kilobytes; this is generous
/// and exists so the scan is bounded before it begins.
pub const MAX_TOKEN_RESPONSE: usize = 64 * 1024;

/// RFC 8628's grant type, spelled as the specification spells it.
pub const DEVICE_GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:device_code";

/// RFC 8628 §3.2: the poll interval to use when the server names none.
pub const DEVICE_INTERVAL_DEFAULT: u64 = 5;

/// RFC 8628 §3.5: what a `slow_down` adds to the interval. The specification is explicit that this
/// *increases* the interval rather than replacing it, and that it never goes back down.
pub const DEVICE_SLOW_DOWN_STEP: u64 = 5;

/// RFC 8628 §3.2: what the device-authorization endpoint answered.
///
/// **`user_code` and the verification URI are for the reader's eyes**, and `device_code` is not:
/// it is the credential this client polls with, and anything that shows a grant to a person shows
/// the first two and never the third.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceGrant {
    /// The code this client polls the token endpoint with. **A credential.**
    pub device_code: String,
    /// The short code the person types at the verification URI.
    pub user_code: String,
    /// Where the person goes to approve it.
    pub verification_uri: String,
    /// The same place with the code already in it, when the server offers one. Optional in the
    /// specification, so a client that requires it is a client that breaks against a compliant
    /// server.
    pub verification_uri_complete: Option<String>,
    /// Seconds until the grant stops being pollable.
    pub expires_in: u64,
    /// Seconds to wait between polls, already defaulted to [`DEVICE_INTERVAL_DEFAULT`].
    pub interval: u64,
}

/// What one poll of the token endpoint means.
///
/// **`Pending` and `SlowDown` are not failures**, and that is the whole reason this is a type
/// rather than a `Result`. RFC 8628 §3.5 defines them as the ordinary answers while a person is
/// still typing their password, and a client that treats an error document as an error would give
/// up on the first one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Poll {
    /// The person approved it and the server issued a token.
    Granted(Token),
    /// Still waiting for the person. Poll again after the interval.
    Pending,
    /// Polling too fast. Add [`DEVICE_SLOW_DOWN_STEP`] and poll again.
    SlowDown,
    /// The grant timed out. A new device authorization is needed.
    Expired,
    /// The person said no.
    Denied,
    /// Anything else, with the server's own `error` value when it sent one.
    Failed(String),
}

/// RFC 8628 §3.1: the request that starts a device authorization.
///
/// **No secret and no PKCE.** A device-flow client is public: there is nothing to prove possession
/// of, and the `device_code` the server returns is itself the secret for the poll that follows.
pub fn device_request(origin: &Origin, client_id: &str, scope: &str) -> Request {
    let mut body = format!("client_id={}", form_encode(client_id));
    if !scope.is_empty() {
        body.push_str(&format!("&scope={}", form_encode(scope)));
    }
    Request {
        method: "POST",
        url: origin.mounted_url(),
        body,
        content_type: Some("application/x-www-form-urlencoded"),
    }
}

/// Reads RFC 8628 §3.2's response, defaulting `interval` the way the specification says to.
///
/// Returns `None` when there is no `device_code` or no `user_code` — including when the server
/// answered with an error document, which is what a client id the server does not know looks like.
pub fn device_grant_from_json(body: &str) -> Option<DeviceGrant> {
    if body.len() > MAX_TOKEN_RESPONSE {
        return None;
    }
    let device_code = json_string(body, "device_code").filter(|t| !t.is_empty())?;
    let user_code = json_string(body, "user_code").filter(|t| !t.is_empty())?;
    // **The verification URI is checked here because the shell hands it to `ShellExecuteW`.** It
    // arrives from a server named by a settings file, which `LOKI.md` §7 already treats as
    // something that may have been sent to the user — and `ShellExecuteW` on `file:///…\x.exe` or
    // `\\host\share\x` runs it. A grant whose URI is not https is not a grant.
    let verification_uri = json_string(body, "verification_uri").filter(|u| is_https_url(u))?;
    Some(DeviceGrant {
        device_code,
        user_code,
        verification_uri,
        // The same rule, and dropped rather than refused: it is an optional convenience, so a
        // server that offers a bad one loses the convenience and not the sign-in.
        verification_uri_complete: json_string(body, "verification_uri_complete")
            .filter(|u| is_https_url(u)),
        // **A missing lifetime is a default, not a zero.** Zero made the poll give up before its
        // first request, which reads to the user as a sign-in that failed instantly for no reason.
        expires_in: json_number(body, "expires_in")
            .filter(|s| *s > 0)
            .unwrap_or(DEVICE_EXPIRY_DEFAULT),
        interval: json_number(body, "interval")
            .filter(|i| *i > 0)
            .unwrap_or(DEVICE_INTERVAL_DEFAULT),
    })
}

/// How long a device grant is assumed good for when the server does not say. RFC 8628 gives no
/// default; fifteen minutes is what the specification's own examples use.
pub const DEVICE_EXPIRY_DEFAULT: u64 = 900;

/// Whether `url` is an absolute `https` URL with a host — the only thing this will put in front of
/// a reader or hand to the shell.
fn is_https_url(url: &str) -> bool {
    url.strip_prefix("https://")
        .is_some_and(|rest| !rest.is_empty() && !rest.starts_with('/'))
}

/// RFC 8628 §3.4: one poll of the token endpoint.
pub fn device_poll_request(origin: &Origin, client_id: &str, device_code: &str) -> Request {
    Request {
        method: "POST",
        url: origin.mounted_url(),
        body: format!(
            "grant_type={}&client_id={}&device_code={}",
            form_encode(DEVICE_GRANT_TYPE),
            form_encode(client_id),
            form_encode(device_code)
        ),
        content_type: Some("application/x-www-form-urlencoded"),
    }
}

/// RFC 8628 §3.5: what the token endpoint's answer to a poll means.
///
/// **Read from the body, not from the status code.** The specification has the server answer a
/// pending poll with HTTP 400 and `error: authorization_pending`, so a client that branches on the
/// status first sees a failure every few seconds and never gets to the word that says otherwise.
pub fn poll_outcome(body: &str) -> Poll {
    if let Some(token) = token_from_json(body) {
        return Poll::Granted(token);
    }
    match json_string(body, "error").unwrap_or_default().as_str() {
        "authorization_pending" => Poll::Pending,
        "slow_down" => Poll::SlowDown,
        "expired_token" => Poll::Expired,
        "access_denied" => Poll::Denied,
        "" => Poll::Failed("the server's answer held neither a token nor an error".to_owned()),
        other => Poll::Failed(other.to_owned()),
    }
}

/// RFC 6749 §6: exchange a refresh token for a new access token.
///
/// **A tail outlives its access token.** `LOKI.md` §5's tail runs for hours and an expiry of an
/// hour is ordinary, so without this a remote source stops returning records mid-session and looks
/// like a source that went quiet.
pub fn refresh_request(origin: &Origin, client_id: &str, refresh_token: &str) -> Request {
    Request {
        method: "POST",
        url: origin.mounted_url(),
        body: format!(
            "grant_type=refresh_token&client_id={}&refresh_token={}",
            form_encode(client_id),
            form_encode(refresh_token)
        ),
        content_type: Some("application/x-www-form-urlencoded"),
    }
}

/// The OAuth `error` code in a failed answer, when there is one.
///
/// **Which error it is decides whether a stored sign-in is thrown away.** RFC 6749 §5.2 makes
/// `invalid_grant` the one that means the refresh token is spent, revoked or expired; everything
/// else — a 503, a rate limit, a misconfigured client — is a reason to try later rather than a
/// reason to sign the reader out.
pub fn oauth_error(body: &str) -> Option<String> {
    json_string(body, "error").filter(|e| !e.is_empty())
}

/// The interval after a `slow_down`, which only ever grows.
pub fn slowed(interval: u64) -> u64 {
    interval.saturating_add(DEVICE_SLOW_DOWN_STEP)
}

/// RFC 4648 §5's base64url, **unpadded**.
///
/// The padding is dropped because every value this encodes travels in a URL or in an OAuth field
/// where RFC 7636 §4.1 asks for exactly this alphabet, and `=` would have to be escaped on the way.
pub fn base64url(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity((bytes.len() * 4).div_ceil(3));
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let triple = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        // One output character per 6 bits, and one fewer than four whenever the chunk was short —
        // which is what "unpadded" means in practice.
        for i in 0..chunk.len() + 1 {
            let at = (triple >> (18 - 6 * i)) & 0x3f;
            out.push(ALPHABET[at as usize] as char);
        }
    }
    out
}

/// The PKCE pair: what the client keeps, and what it shows the authorization server.
///
/// **The verifier never leaves this machine until the exchange**, which is the whole of RFC 7636:
/// an authorization code intercepted on its way back is useless without it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pkce {
    /// Kept secret until the code is exchanged. **A credential.**
    pub verifier: String,
    /// The `S256` challenge sent with the authorization request.
    pub challenge: String,
}

/// The PKCE pair for `random` (32 bytes) whose SHA-256 is `digest`.
///
/// **The hashing is the caller's**, because it is the only impure part and this module has no I/O
/// and no platform in it. The shell hands in `bcrypt.dll`'s answer.
///
/// The verifier is the base64url of 32 bytes — 43 characters, the shortest RFC 7636 §4.1 permits,
/// and every character already unreserved so nothing needs escaping on the way out.
pub fn pkce_from(random: &[u8], digest: &[u8]) -> Pkce {
    Pkce {
        verifier: base64url(random),
        challenge: base64url(digest),
    }
}

/// RFC 6749 §4.1.1's authorization request, as a URL for the browser to open.
///
/// **A GET, and the only place in this product where that is right.** §7's privacy clause is about
/// filter text a reader authored; these parameters are a client id, a scope and two opaque values
/// the server just issued, and the authorization endpoint is defined to take them in the query.
pub fn authorize_url(
    origin: &Origin,
    client_id: &str,
    redirect_uri: &str,
    scope: &str,
    state: &str,
    challenge: &str,
) -> String {
    let mut url = format!(
        "{}?response_type=code&client_id={}&redirect_uri={}&state={}&code_challenge={}&code_challenge_method=S256",
        origin.mounted_url(),
        form_encode(client_id),
        form_encode(redirect_uri),
        form_encode(state),
        form_encode(challenge)
    );
    if !scope.is_empty() {
        url.push_str(&format!("&scope={}", form_encode(scope)));
    }
    url
}

/// Why a callback did not yield an authorization code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallbackFault {
    /// Not a callback for this application at all.
    NotOurs,
    /// **The `state` did not match the one sent.** RFC 6749 §10.12, and the reason it is checked
    /// rather than merely sent: this callback arrives as a *command line*, so anyone who can start
    /// a process can hand us one. There is no network for an attacker to be on.
    WrongState,
    /// The server said no, in its own word.
    Refused(String),
    /// A callback with neither a code nor an error in it.
    Empty,
}

/// The authorization code in a `tailhawk://` callback, once `state` has been checked.
pub fn code_from_callback(uri: &str, expected_state: &str) -> Result<String, CallbackFault> {
    let query = uri
        .split_once('?')
        .map(|(_, q)| q)
        .ok_or(CallbackFault::NotOurs)?;
    let mut code = None;
    let mut state = None;
    let mut error = None;
    for pair in query.split('&') {
        let Some((key, value)) = pair.split_once('=') else {
            continue;
        };
        match key {
            "code" => code = Some(form_decode(value)),
            "state" => state = Some(form_decode(value)),
            "error" => error = Some(form_decode(value)),
            _ => {}
        }
    }
    // **State first, before the code is even looked at.** A forged callback carrying a valid-looking
    // code must be refused on the state alone, and checking the code first would mean deciding what
    // to do with a value we have already decided not to trust.
    if state.as_deref() != Some(expected_state) {
        return Err(CallbackFault::WrongState);
    }
    if let Some(why) = error {
        return Err(CallbackFault::Refused(why));
    }
    code.filter(|c| !c.is_empty()).ok_or(CallbackFault::Empty)
}

/// RFC 6749 §4.1.3: exchange the authorization code for a token, proving possession of the verifier.
pub fn code_exchange_request(
    origin: &Origin,
    client_id: &str,
    code: &str,
    verifier: &str,
    redirect_uri: &str,
) -> Request {
    Request {
        method: "POST",
        url: origin.mounted_url(),
        body: format!(
            "grant_type=authorization_code&client_id={}&code={}&redirect_uri={}&code_verifier={}",
            form_encode(client_id),
            form_encode(code),
            form_encode(redirect_uri),
            form_encode(verifier)
        ),
        content_type: Some("application/x-www-form-urlencoded"),
    }
}

/// The inverse of the form encoding, for reading values out of a callback's query.
fn form_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                match u8::from_str_radix(&value[i + 1..i + 3], 16) {
                    Ok(byte) => out.push(byte),
                    // Not a hex escape after all, so it is a literal per cent.
                    Err(_) => out.push(b'%'),
                }
                i += 3;
            }
            other => {
                out.push(other);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The string value of `key`, honouring backslash escapes so a token containing a quote cannot cut
/// the scan short.
fn json_string(body: &str, key: &str) -> Option<String> {
    let at = body.find(&format!("\"{key}\""))? + key.len() + 2;
    let rest = body.get(at..)?.trim_start();
    let rest = rest.strip_prefix(':')?.trim_start();
    let rest = rest.strip_prefix('"')?;
    let mut out = String::new();
    let mut chars = rest.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Some(out),
            '\\' => match chars.next()? {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                'r' => out.push('\r'),
                other => out.push(other),
            },
            other => out.push(other),
        }
    }
    None
}

/// The integer value of `key`. Seconds, so a fractional or negative answer is not one.
fn json_number(body: &str, key: &str) -> Option<u64> {
    let at = body.find(&format!("\"{key}\""))? + key.len() + 2;
    let rest = body.get(at..)?.trim_start();
    let rest = rest.strip_prefix(':')?.trim_start();
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// Why a label name was refused before it could reach a URL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LabelFault {
    /// Empty, too long, or carrying anything outside Prometheus's label grammar. **A label name is
    /// the one part of a Loki path that is not ours**, so it is checked with the same suspicion
    /// [`Origin::parse`] gives a configured URL: a name that could carry `/` or `..` is a
    /// configuration string choosing a path segment.
    NotALabelName,
}

/// The longest label name that will be believed. Prometheus imposes no limit; a name longer than
/// this is a mistake or an attack, and either way not a label anyone made on purpose.
pub const MAX_LABEL_NAME: usize = 64;

/// The most values one label response may yield. Far above the eighty-odd a real estate has, and
/// far below what a shared deployment could answer with if a label were used for an identifier.
pub const MAX_LABEL_VALUES: usize = 10_000;

/// The largest label response that will be read. A label list is names, not log lines.
pub const MAX_LABEL_RESPONSE: usize = 1024 * 1024;

/// Whether `name` is a label name Prometheus and Loki would accept: `[a-zA-Z_][a-zA-Z0-9_]*`.
pub fn is_label_name(name: &str) -> bool {
    if name.is_empty() || name.len() > MAX_LABEL_NAME {
        return false;
    }
    let mut chars = name.chars();
    let first = chars.next().unwrap_or('\0');
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Ask which values one label has taken in a window — `LOKI.md` §2's "seventy `app` values", so a
/// person can pick from them instead of spelling them.
///
/// **A `GET`, unlike [`query_range`], and the difference is the point.** §7 keeps user-authored
/// selector text out of the request line because every proxy on the path logs it; there is no
/// selector here. What is in the line is a label name this program asked for, checked against the
/// label grammar before it is written, and a window of timestamps.
pub fn label_values(origin: &Origin, label: &str, window: Window) -> Result<Request, LabelFault> {
    if !is_label_name(label) {
        return Err(LabelFault::NotALabelName);
    }
    Ok(Request {
        method: "GET",
        url: format!(
            "{}{}{}/{}/values?start={}&end={}",
            origin.base(),
            origin.mount(),
            Endpoint::LabelValues.path(),
            label,
            window.start,
            window.end
        ),
        body: String::new(),
        content_type: None,
    })
}

/// The values in a label response — `{"status":"success","data":["one","two"]}`.
///
/// A hand-written scan for the reason `token_from_json` gives: there is no JSON crate in the tree
/// and this is one array of strings in a small, flat document. Bounded before anything is looked
/// at, and bounded again in the number of values, so a server answering with a label used as an
/// identifier cannot make this the expensive part.
///
/// A body that is not a success envelope, or whose `data` is not an array of strings, yields
/// `None` rather than an empty list: "the server said no values" and "the server said something I
/// did not understand" are different answers and the caller says different things about them.
pub fn label_values_from_json(body: &str) -> Option<Vec<String>> {
    if body.len() > MAX_LABEL_RESPONSE {
        return None;
    }
    let at = body.find("\"data\"")? + "\"data\"".len();
    let rest = body.get(at..)?.trim_start();
    let rest = rest.strip_prefix(':')?.trim_start();
    let mut rest = rest.strip_prefix('[')?;

    let mut values = Vec::new();
    loop {
        rest = rest.trim_start();
        if let Some(after) = rest.strip_prefix(']') {
            let _ = after;
            return Some(values);
        }
        let (value, after) = json_string_at(rest)?;
        if values.len() == MAX_LABEL_VALUES {
            return Some(values);
        }
        values.push(value);
        rest = after.trim_start();
        match rest.strip_prefix(',') {
            Some(next) => rest = next,
            None => {
                return rest.strip_prefix(']').map(|_| values);
            }
        }
    }
}

/// Reads one quoted JSON string from the front of `text`, returning it and what follows.
///
/// **A value this cannot decode becomes `U+FFFD`; it never discards the answer.** The alternative
/// was tried on the tail and cost a day: a single label value one byte over its cap made the parser
/// refuse a whole response, and the tail stalled for ever on one record. The same shape applies
/// here with a bigger blast radius — one surrogate pair from a proxy that re-encodes would mean no
/// picker at all, for seventy applications whose names are ASCII.
fn json_string_at(text: &str) -> Option<(String, &str)> {
    let mut chars = text.strip_prefix('"')?.char_indices();
    let body = text.get(1..)?;
    let mut out = String::new();
    while let Some((at, c)) = chars.next() {
        match c {
            '"' => return Some((out, body.get(at + 1..)?)),
            '\\' => match chars.next()?.1 {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                'r' => out.push('\r'),
                'b' => out.push('\u{8}'),
                'f' => out.push('\u{c}'),
                'u' => {
                    let first = hex_escape(body, &mut chars)?;
                    // A surrogate pair is two escapes for one character. Go's encoder does not
                    // emit them, but a proxy or a re-encoder between here and Loki may, and half a
                    // character rendered as a replacement is better than no list of applications.
                    let decoded = match first {
                        0xD800..=0xDBFF => {
                            let low = paired_escape(body, &mut chars);
                            match low {
                                Some(low @ 0xDC00..=0xDFFF) => {
                                    let joined =
                                        0x10000 + ((first - 0xD800) << 10) + (low - 0xDC00);
                                    char::from_u32(joined)
                                }
                                _ => None,
                            }
                        }
                        code => char::from_u32(code),
                    };
                    out.push(decoded.unwrap_or(char::REPLACEMENT_CHARACTER));
                }
                other => out.push(other),
            },
            other => out.push(other),
        }
    }
    None
}

/// The four hex digits after a `\u`, consuming them.
fn hex_escape(body: &str, chars: &mut std::str::CharIndices) -> Option<u32> {
    let start = chars.next()?.0;
    let hex = body.get(start..start + 4)?;
    let _ = chars.next();
    let _ = chars.next();
    let _ = chars.next();
    u32::from_str_radix(hex, 16).ok()
}

/// The second half of a surrogate pair — `\uXXXX` immediately after the first — or `None` if what
/// follows is anything else. Consumes only what it reads.
fn paired_escape(body: &str, chars: &mut std::str::CharIndices) -> Option<u32> {
    let mut ahead = chars.clone();
    if ahead.next()?.1 != '\\' || ahead.next()?.1 != 'u' {
        return None;
    }
    let low = hex_escape(body, &mut ahead)?;
    *chars = ahead;
    Some(low)
}

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

    /// **The client secret is not in the request, and this is the test that says so.**
    /// [`Request`]'s own doc promises it can be logged, shown in a dialog or compared in a test —
    /// a promise every existing reader relies on. Putting the secret in `body` would break it
    /// silently and everywhere at once, so the secret is attached by the transport at the moment
    /// of sending, exactly as the `Authorization` header is.
    #[test]
    fn a_token_request_carries_everything_except_the_secret() {
        let at = origin("https://identity.example/connect/token");
        let request = token_request(&at, "tailhawk", "telemetry:read");

        assert_eq!(request.method, "POST");
        assert_eq!(request.url, "https://identity.example/connect/token");
        assert_eq!(
            request.content_type,
            Some("application/x-www-form-urlencoded")
        );
        assert_eq!(
            request.body,
            "grant_type=client_credentials&client_id=tailhawk&scope=telemetry%3Aread"
        );

        // The whole point, said two ways: nothing in the request mentions a secret, and nothing
        // that renders it can leak one.
        assert!(!request.body.contains("client_secret"));
        let rendered = format!("{request:?}");
        for word in ["secret", "password"] {
            assert!(
                !rendered.to_lowercase().contains(word),
                "a Debug-printed Request must never carry a {word}"
            );
        }

        // An empty scope is omitted rather than sent blank — some providers reject `scope=`.
        let scopeless = token_request(&at, "tailhawk", "");
        assert_eq!(
            scopeless.body,
            "grant_type=client_credentials&client_id=tailhawk"
        );

        // The client id is form-encoded like any other value.
        let odd = token_request(&at, "a b&c", "s");
        assert!(odd.body.contains("client_id=a+b%26c") || odd.body.contains("client_id=a%20b%26c"));
    }

    /// An origin for the device and token endpoints, so each test below says what it is about
    /// rather than re-parsing a URL.
    fn an_endpoint() -> Origin {
        Origin::parse("https://identity.example.com/connect", Provenance::Imported)
            .expect("a plain https origin with a mount prefix")
    }

    /// **RFC 8628 §3.1 asks for the client id and the scope, and nothing else.** A device-flow
    /// client is public: there is no secret to send and none is sent, which is the entire reason
    /// the owner asked for this over a pasted client secret.
    #[test]
    fn a_device_authorization_asks_only_for_the_client_and_the_scope() {
        let request = device_request(&an_endpoint(), "tailhawk", "telemetry:read offline_access");
        assert_eq!(request.method, "POST");
        assert_eq!(
            request.content_type,
            Some("application/x-www-form-urlencoded")
        );
        assert_eq!(
            request.body,
            "client_id=tailhawk&scope=telemetry%3Aread+offline_access"
        );
        assert!(!request.body.contains("client_secret"));

        // An empty scope is omitted rather than sent blank, exactly as the token request does it.
        assert_eq!(
            device_request(&an_endpoint(), "tailhawk", "").body,
            "client_id=tailhawk"
        );
    }

    /// The grant is read, and `interval` defaults the way §3.2 says rather than to zero — a zero
    /// would poll in a tight loop, which is the one behaviour the interval exists to prevent.
    #[test]
    fn a_device_grant_is_read_and_its_interval_defaulted() {
        let body = r#"{"device_code":"dc","user_code":"WDJB-MJHT",
            "verification_uri":"https://identity.example.com/device",
            "verification_uri_complete":"https://identity.example.com/device?user_code=WDJB-MJHT",
            "expires_in":900,"interval":5}"#;
        let grant = device_grant_from_json(body).expect("a well-formed grant");
        assert_eq!(grant.device_code, "dc");
        assert_eq!(grant.user_code, "WDJB-MJHT");
        assert_eq!(
            grant.verification_uri,
            "https://identity.example.com/device"
        );
        assert_eq!(
            grant.verification_uri_complete.as_deref(),
            Some("https://identity.example.com/device?user_code=WDJB-MJHT")
        );
        assert_eq!(grant.expires_in, 900);
        assert_eq!(grant.interval, 5);

        // §3.2 makes both `interval` and `verification_uri_complete` optional. A server that sends
        // neither is compliant, and a client that needs them is not.
        let spare = r#"{"device_code":"d","user_code":"U",
            "verification_uri":"https://i.example/d","expires_in":600}"#;
        let grant = device_grant_from_json(spare).expect("a minimal but compliant grant");
        assert_eq!(grant.interval, DEVICE_INTERVAL_DEFAULT);
        assert_eq!(grant.verification_uri_complete, None);

        // An error document is not a grant. An unknown client id looks exactly like this.
        assert_eq!(
            device_grant_from_json(r#"{"error":"invalid_client"}"#),
            None
        );
    }

    /// RFC 4648 §10's own vectors, in base64url and without padding — which is where the length of
    /// the output differs from plain base64 and where a hand-written encoder gets it wrong.
    #[test]
    fn base64url_matches_the_specifications_vectors() {
        assert_eq!(base64url(b""), "");
        assert_eq!(base64url(b"f"), "Zg");
        assert_eq!(base64url(b"fo"), "Zm8");
        assert_eq!(base64url(b"foo"), "Zm9v");
        assert_eq!(base64url(b"foob"), "Zm9vYg");
        assert_eq!(base64url(b"fooba"), "Zm9vYmE");
        assert_eq!(base64url(b"foobar"), "Zm9vYmFy");

        // **The two characters that make it url-safe.** Plain base64 gives `+` and `/` here, both of
        // which would have to be escaped in a query — the reason RFC 7636 asks for this alphabet.
        assert_eq!(base64url(&[0xfb, 0xff, 0xbe]), "-_--");
        assert!(!base64url(&[0xfb, 0xff, 0xbe]).contains(['+', '/', '=']));
    }

    /// A verifier is the 43 characters RFC 7636 §4.1 asks for, all of them already unreserved.
    #[test]
    fn a_pkce_verifier_is_the_shortest_the_specification_allows() {
        let pair = pkce_from(&[0x41; 32], &[0x42; 32]);
        assert_eq!(pair.verifier.len(), 43);
        assert_eq!(pair.challenge.len(), 43);
        assert!(
            pair.verifier
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_' | '~')),
            "every character must be unreserved: {}",
            pair.verifier
        );
        assert_ne!(
            pair.verifier, pair.challenge,
            "the challenge is the digest, not the verifier — sending the verifier would defeat it"
        );
    }

    /// The authorization request carries what RFC 6749 §4.1.1 and RFC 7636 §4.3 require, and the
    /// redirect URI is encoded rather than left to the browser to interpret.
    #[test]
    fn an_authorize_url_carries_the_challenge_and_not_the_verifier() {
        let at = Origin::parse("https://identity.example.com/connect", Provenance::Imported)
            .expect("an https origin");
        let url = authorize_url(
            &at,
            "tailhawk",
            "tailhawk://auth/callback",
            "telemetry:read offline_access",
            "st4te",
            "ch4llenge",
        );
        assert!(
            url.starts_with("https://identity.example.com/connect?"),
            "{url}"
        );
        for required in [
            "response_type=code",
            "client_id=tailhawk",
            "code_challenge=ch4llenge",
            "code_challenge_method=S256",
            "state=st4te",
            "redirect_uri=tailhawk%3A%2F%2Fauth%2Fcallback",
            "scope=telemetry%3Aread+offline_access",
        ] {
            assert!(url.contains(required), "missing {required} in {url}");
        }
        assert!(
            !url.contains("code_verifier"),
            "the verifier must never travel with the authorization request: {url}"
        );
    }

    /// **`state` is checked before the code is even read.** The callback arrives as a command line
    /// from a registered URI scheme, so anyone who can start a process can hand us one — RFC 6749
    /// §10.12's concern, without an attacker needing to be on a network at all.
    #[test]
    fn a_callback_with_the_wrong_state_is_refused_before_its_code_is_believed() {
        let ours = "tailhawk://auth/callback?code=abc123&state=expected";
        assert_eq!(
            code_from_callback(ours, "expected"),
            Ok("abc123".to_owned())
        );

        assert_eq!(
            code_from_callback(
                "tailhawk://auth/callback?code=abc123&state=forged",
                "expected"
            ),
            Err(CallbackFault::WrongState)
        );
        assert_eq!(
            code_from_callback("tailhawk://auth/callback?code=abc123", "expected"),
            Err(CallbackFault::WrongState),
            "no state at all is not a match"
        );

        // The server's refusal is reported in its own word, but only once the state proves the
        // callback is ours to read.
        assert_eq!(
            code_from_callback(
                "tailhawk://auth/callback?error=access_denied&state=expected",
                "expected"
            ),
            Err(CallbackFault::Refused("access_denied".to_owned()))
        );
        assert_eq!(
            code_from_callback(
                "tailhawk://auth/callback?error=access_denied&state=forged",
                "expected"
            ),
            Err(CallbackFault::WrongState),
            "a forged callback is refused on the state even when it carries an error"
        );

        assert_eq!(
            code_from_callback("tailhawk://auth/callback?state=expected", "expected"),
            Err(CallbackFault::Empty)
        );
        assert_eq!(
            code_from_callback("not-a-callback", "expected"),
            Err(CallbackFault::NotOurs)
        );

        // A percent-escaped code survives the trip back.
        assert_eq!(
            code_from_callback(
                "tailhawk://auth/callback?code=a%2Fb%2Bc&state=expected",
                "expected"
            ),
            Ok("a/b+c".to_owned())
        );
    }

    /// The exchange proves possession of the verifier, and carries no secret — the client is public.
    #[test]
    fn the_code_exchange_sends_the_verifier_and_no_secret() {
        let at = Origin::parse(
            "https://identity.example.com/connect/token",
            Provenance::Imported,
        )
        .expect("an https origin");
        let request = code_exchange_request(
            &at,
            "tailhawk",
            "the-code",
            "the-verifier",
            "tailhawk://auth/callback",
        );
        assert_eq!(request.method, "POST");
        assert!(request.body.contains("grant_type=authorization_code"));
        assert!(request.body.contains("code=the-code"));
        assert!(request.body.contains("code_verifier=the-verifier"));
        assert!(!request.body.contains("client_secret"));
    }

    /// **The estate's own identity server, in its own words.** Every other test here is written
    /// from the specification; this one is the body `/connect/deviceauthorization` actually returned
    /// on 2026-10-01, so the parser is checked against the server rather than against my reading of
    /// RFC 8628. The `device_code` is a placeholder of the right shape — it is a credential, and
    /// this repository is public.
    #[test]
    fn the_real_servers_answer_parses() {
        let body = r#"{"device_code":"0000000000000000000000000000000000000000000000000000000000000000","user_code":"667094687","verification_uri":"https://identity-dev.nurtur.tech/device","verification_uri_complete":"https://identity-dev.nurtur.tech/device?userCode=667094687","expires_in":300,"interval":5}"#;
        let grant = device_grant_from_json(body).expect("the server's own answer must parse");
        assert_eq!(grant.user_code, "667094687");
        assert_eq!(
            grant.verification_uri,
            "https://identity-dev.nurtur.tech/device"
        );
        assert_eq!(
            grant.verification_uri_complete.as_deref(),
            Some("https://identity-dev.nurtur.tech/device?userCode=667094687")
        );
        assert_eq!(grant.expires_in, 300);
        assert_eq!(grant.interval, 5);
        assert_eq!(grant.device_code.len(), 64);
    }

    /// **A missing lifetime must not mean "already over".** A zero here made the poll give up
    /// before its first request, which reads as a sign-in that failed instantly for no reason.
    #[test]
    fn a_grant_with_no_stated_lifetime_still_gets_one() {
        let body =
            r#"{"device_code":"d","user_code":"U","verification_uri":"https://i.example/d"}"#;
        let grant = device_grant_from_json(body).expect("a grant");
        assert_eq!(grant.expires_in, DEVICE_EXPIRY_DEFAULT);
        assert!(grant.expires_in > 0, "a zero lifetime never polls once");
    }

    /// **The verification URI is handed to `ShellExecuteW`, so it is checked before it is trusted.**
    /// It comes from a server named in a settings file, which §7 already treats as something that
    /// may have been sent to the reader — and the shell will happily run a `file:` URL or a UNC path.
    #[test]
    fn a_verification_uri_that_is_not_https_is_refused_rather_than_opened() {
        let with = |uri: &str| {
            format!(
                r#"{{"device_code":"d","user_code":"U","verification_uri":"{uri}","expires_in":600}}"#
            )
        };
        assert!(device_grant_from_json(&with("https://i.example/device")).is_some());
        for hostile in [
            "file:///C:/Windows/System32/calc.exe",
            r"\\attacker\share\x.exe",
            "http://i.example/device",
            "javascript:alert(1)",
            "https://",
            "https:///no-host",
            "",
        ] {
            assert_eq!(
                device_grant_from_json(&with(hostile)),
                None,
                "a grant naming {hostile:?} must not become a thing the shell opens"
            );
        }

        // The complete URI is a convenience, so a bad one costs the convenience and not the
        // sign-in: the grant still stands, without it.
        let mixed = r#"{"device_code":"d","user_code":"U",
            "verification_uri":"https://i.example/device",
            "verification_uri_complete":"file:///C:/x.exe","expires_in":600}"#;
        let grant = device_grant_from_json(mixed).expect("the sign-in survives");
        assert_eq!(grant.verification_uri_complete, None);
    }

    /// **§3.5's answers, each one of them.** `authorization_pending` and `slow_down` arrive with
    /// HTTP 400 while a person is still signing in, so reading the status instead of the body
    /// would make every poll look like a failure.
    #[test]
    fn every_poll_answer_the_specification_defines_is_told_apart() {
        assert_eq!(
            poll_outcome(r#"{"error":"authorization_pending"}"#),
            Poll::Pending
        );
        assert_eq!(poll_outcome(r#"{"error":"slow_down"}"#), Poll::SlowDown);
        assert_eq!(poll_outcome(r#"{"error":"expired_token"}"#), Poll::Expired);
        assert_eq!(poll_outcome(r#"{"error":"access_denied"}"#), Poll::Denied);
        assert_eq!(
            poll_outcome(r#"{"error":"invalid_client"}"#),
            Poll::Failed("invalid_client".to_owned())
        );
        assert_eq!(
            poll_outcome(r#"{"access_token":"t","expires_in":60,"refresh_token":"r"}"#),
            Poll::Granted(Token {
                access_token: "t".to_owned(),
                expires_in: 60,
                refresh_token: Some("r".to_owned()),
            })
        );

        // Neither a token nor a recognised error. Saying so beats guessing at either.
        assert!(matches!(poll_outcome("{}"), Poll::Failed(_)));
    }

    /// The poll carries the grant type spelled as RFC 8628 spells it — a URN, which must be
    /// form-encoded or the colons end up in the body raw.
    #[test]
    fn a_poll_names_the_grant_type_the_specification_defines() {
        let request = device_poll_request(&an_endpoint(), "tailhawk", "dc");
        assert_eq!(request.method, "POST");
        assert!(
            request
                .body
                .contains("grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Adevice_code"),
            "the grant type must be form-encoded: {}",
            request.body
        );
        assert!(request.body.contains("device_code=dc"));
        assert!(request.body.contains("client_id=tailhawk"));
        assert!(!request.body.contains("client_secret"));
    }

    /// A refresh is RFC 6749 §6, and it is what keeps an hours-long tail alive past the access
    /// token's hour.
    #[test]
    fn a_refresh_exchanges_the_refresh_token_and_nothing_else() {
        let request = refresh_request(&an_endpoint(), "tailhawk", "r/t+v");
        assert_eq!(request.method, "POST");
        assert!(request.body.contains("grant_type=refresh_token"));
        assert!(
            request.body.contains("refresh_token=r%2Ft%2Bv"),
            "the refresh token is form-encoded: {}",
            request.body
        );
        assert!(!request.body.contains("client_secret"));
    }

    /// §3.5: a `slow_down` *increases* the interval and it never comes back down.
    #[test]
    fn a_slow_down_only_ever_lengthens_the_wait() {
        assert_eq!(slowed(5), 5 + DEVICE_SLOW_DOWN_STEP);
        assert!(slowed(slowed(5)) > slowed(5));
    }

    /// **The token endpoint is a second origin and gets §7's checks.** Its path is provider
    /// configuration rather than one of ours, so it arrives as the mount prefix — and everything
    /// `Origin::parse` refuses for a Loki base is refused here too.
    #[test]
    fn a_token_url_is_held_to_the_same_rules_as_the_query_url() {
        assert_eq!(
            origin("https://identity.example/connect/token").mounted_url(),
            "https://identity.example/connect/token"
        );
        assert_eq!(
            origin("https://identity.example").mounted_url(),
            "https://identity.example",
            "no prefix, no trailing slash"
        );

        // Each of these is refused for a token URL for the same reason it is for a query URL.
        for bad in [
            "https://user:pw@identity.example/connect/token",
            "https://identity.example/connect/token?x=1",
            "https://identity.example/connect/token#f",
            "identity.example/connect/token",
        ] {
            let _ = fault(bad);
        }
    }

    /// A token response yields its two fields, and an error document yields nothing rather than a
    /// guess — a wrong secret is the common case and must not read as a token.
    #[test]
    fn a_token_response_is_read_or_refused_but_never_guessed() {
        let good = r#"{"access_token":"abc.def.ghi","expires_in":3600,"token_type":"Bearer"}"#;
        assert_eq!(
            token_from_json(good),
            Some(Token {
                access_token: "abc.def.ghi".to_owned(),
                expires_in: 3600,
                refresh_token: None
            })
        );

        // A client-credentials answer carries no refresh token, and a user sign-in does.
        let user = r#"{"access_token":"a","expires_in":60,"refresh_token":"r"}"#;
        assert_eq!(
            token_from_json(user).and_then(|t| t.refresh_token),
            Some("r".to_owned())
        );

        // Order does not matter, and whitespace does not either.
        let spaced = "{ \"expires_in\" : 60 , \"access_token\" : \"t\" }";
        assert_eq!(token_from_json(spaced).map(|t| t.expires_in), Some(60));

        // No `expires_in` is not a failure; it is zero, and the caller refreshes eagerly.
        let bare = r#"{"access_token":"t"}"#;
        assert_eq!(token_from_json(bare).map(|t| t.expires_in), Some(0));

        // An escaped quote inside the token must not cut the scan short.
        let escaped = r#"{"access_token":"a\"b"}"#;
        assert_eq!(
            token_from_json(escaped).map(|t| t.access_token),
            Some("a\"b".to_owned())
        );

        for refused in [
            r#"{"error":"invalid_client"}"#,
            r#"{"access_token":""}"#,
            "not json at all",
            "",
        ] {
            assert_eq!(token_from_json(refused), None, "{refused:?}");
        }

        // Bounded before it is read.
        let huge = format!(
            "{{\"access_token\":\"{}\"}}",
            "x".repeat(MAX_TOKEN_RESPONSE)
        );
        assert_eq!(token_from_json(&huge), None, "a response past the cap");
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
            ("https://loki.example.com/a//b", OriginFault::BadPrefix),
            ("https://loki.example.com/a/../b", OriginFault::BadPrefix),
            ("https://loki.example.com/a/./b", OriginFault::BadPrefix),
            ("https://loki.example.com/a b", OriginFault::BadPrefix),
            ("https://loki.example.com/a%2fb", OriginFault::BadPrefix),
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

    /// The shape the owner's own deployment actually has: a reverse proxy matches a prefix, strips
    /// it, and forwards the rest to Loki. Refusing every path — which this did until the estate was
    /// looked at on 2026-08-27 — left Tailhawk unable to reach the only Loki there is.
    #[test]
    fn a_loki_mounted_under_a_prefix_can_be_reached() {
        let o = origin("https://telemetry.example.com/loki");
        assert_eq!(o.prefix(), "loki");
        assert_eq!(
            o.url(Endpoint::QueryRange),
            "https://telemetry.example.com/loki/loki/api/v1/query_range",
            "the proxy eats the first segment and Loki is served the second"
        );
        assert_eq!(
            o.url(Endpoint::Tail),
            "wss://telemetry.example.com/loki/loki/api/v1/tail"
        );
        assert_eq!(origin("https://h/a/b/c").prefix(), "a/b/c");
        assert_eq!(origin("https://h/loki/").prefix(), "loki");
        assert_eq!(origin("https://h/").prefix(), "");
        assert_eq!(origin("https://h").prefix(), "");
    }

    #[test]
    fn a_prefix_cannot_be_used_to_climb_out_of_its_mount() {
        for text in [
            "https://h/..",
            "https://h/a/..",
            "https://h/../a",
            "https://h/a/../../b",
            "https://h/a/%2e%2e/b",
            "https://h/a?b",
            "https://h/a#b",
        ] {
            assert!(
                Origin::parse(text, Provenance::Typed).is_err(),
                "{text} was accepted"
            );
        }
        assert_eq!(
            fault(&format!("https://h/{}", "a".repeat(200))),
            OriginFault::TooLong
        );
    }

    #[test]
    fn a_credential_is_bound_to_the_origin_and_not_to_the_mount() {
        let bare = origin("https://loki.example.com");
        let mounted = origin("https://loki.example.com/loki");
        assert_eq!(
            bare.key(None),
            mounted.key(None),
            "the same host at two mounts is one security principal"
        );
        assert_ne!(
            bare.key(None),
            origin("https://other.example.com").key(None)
        );
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

    /// **A label name is the one part of a Loki URL that is not ours**, so it is checked before it
    /// is written into one. The refusals below are the shapes that would otherwise choose a path
    /// segment rather than name a label.
    #[test]
    fn a_label_name_is_checked_against_the_label_grammar() {
        assert!(is_label_name("app"));
        assert!(is_label_name("_app"));
        assert!(is_label_name("app_2"));
        assert!(!is_label_name(""));
        assert!(!is_label_name("2app"), "a name cannot start with a digit");
        assert!(!is_label_name("app/values/../../admin"));
        assert!(!is_label_name("app values"));
        assert!(!is_label_name("app%2f"));
        assert!(!is_label_name(&"a".repeat(MAX_LABEL_NAME + 1)));
    }

    #[test]
    fn the_label_request_names_the_label_and_the_window() {
        let origin = Origin::parse("https://telemetry.example.com/loki", Provenance::Imported)
            .expect("origin");
        let request = label_values(&origin, "app", Window { start: 10, end: 20 }).expect("request");
        assert_eq!(request.method, "GET");
        assert_eq!(
            request.url,
            "https://telemetry.example.com/loki/loki/api/v1/label/app/values?start=10&end=20"
        );
        assert!(request.body.is_empty(), "a GET carries nothing to log");

        assert_eq!(
            label_values(&origin, "app/../push", Window { start: 1, end: 2 }),
            Err(LabelFault::NotALabelName),
            "a name that could choose a path segment never reaches a URL"
        );
    }

    #[test]
    fn the_values_are_read_out_of_the_success_envelope() {
        let body = r#"{"status":"success","data":["gateway","nurtur-identity-server","dw.api"]}"#;
        assert_eq!(
            label_values_from_json(body).expect("values"),
            ["gateway", "nurtur-identity-server", "dw.api"]
        );
        assert_eq!(
            label_values_from_json(r#"{"status":"success","data":[]}"#).expect("values"),
            Vec::<String>::new(),
            "a label with no values is an answer, not a failure to understand one"
        );
    }

    /// **"No values" and "I did not understand that" are different answers**, and the caller says
    /// different things about them — so an envelope this cannot read is `None`, never an empty list
    /// that would render as a picker with nothing in it and no reason why.
    #[test]
    fn an_envelope_this_cannot_read_is_not_an_empty_list() {
        assert_eq!(label_values_from_json("not json at all"), None);
        assert_eq!(
            label_values_from_json(r#"{"status":"error","error":"too many outstanding requests"}"#),
            None
        );
        assert_eq!(
            label_values_from_json(r#"{"data":{"app":"gateway"}}"#),
            None
        );
        assert_eq!(
            label_values_from_json(&format!(
                "{{\"data\":[\"{}\"]}}",
                "x".repeat(MAX_LABEL_RESPONSE)
            )),
            None,
            "a response beyond the cap is refused before it is walked"
        );
    }

    #[test]
    fn escapes_in_a_value_are_read_as_the_character_they_stand_for() {
        let body = r#"{"status":"success","data":["a\"b","c\\d","eéf"]}"#;
        assert_eq!(
            label_values_from_json(body).expect("values"),
            ["a\"b", "c\\d", "eéf"]
        );
    }

    /// The value cap is the one that bites on a label used as an identifier: twelve thousand short
    /// values are well under the body cap, so without this the picker would walk them all.
    #[test]
    fn more_values_than_the_cap_are_cut_rather_than_walked() {
        let values: Vec<String> = (0..MAX_LABEL_VALUES + 2_000)
            .map(|n| format!("\"v{n}\""))
            .collect();
        let body = format!("{{\"status\":\"success\",\"data\":[{}]}}", values.join(","));
        assert!(
            body.len() < MAX_LABEL_RESPONSE,
            "the body cap must not be what stops this"
        );
        let read = label_values_from_json(&body).expect("values");
        assert_eq!(read.len(), MAX_LABEL_VALUES);
        assert_eq!(
            read[0], "v0",
            "and they are the first, not an arbitrary slice"
        );
    }

    /// **One value nobody can decode must not cost the whole list.** The tail learned this the
    /// expensive way in September: a parser that refused a response for one bad value stalled for
    /// ever on one record. Here the same shape would mean no picker at all, for seventy
    /// applications whose names are plain ASCII.
    #[test]
    fn an_escape_this_cannot_decode_costs_one_character_not_the_list() {
        let paired = r#"{"data":["a\ud83d\ude00b","gateway"]}"#;
        assert_eq!(
            label_values_from_json(paired).expect("values"),
            ["a\u{1F600}b", "gateway"],
            "a surrogate pair is one character, not two replacements"
        );

        let lone = r#"{"data":["a\ud800b","gateway"]}"#;
        let read = label_values_from_json(lone).expect("values");
        assert_eq!(read.len(), 2, "the second value survives the first");
        assert_eq!(read[0], "a\u{FFFD}b");
        assert_eq!(read[1], "gateway");
    }

    #[test]
    fn the_short_escapes_decode_to_what_they_stand_for() {
        let body = r#"{"data":["a\nb","c\td","e\bf","g\fh","i\/j","kAl"]}"#;
        assert_eq!(
            label_values_from_json(body).expect("values"),
            ["a\nb", "c\td", "e\u{8}f", "g\u{c}h", "i/j", "kAl"]
        );
    }

    /// A string that never closes is not a value; the answer is "I did not understand this",
    /// which the caller says differently from "there are none".
    #[test]
    fn an_unterminated_value_is_not_read_as_a_value() {
        assert_eq!(label_values_from_json(r#"{"data":["gateway]}"#), None);
        assert_eq!(label_values_from_json(r#"{"data":["a","#), None);
        assert_eq!(label_values_from_json(r#"{"data":["a\"#), None);
    }
}
