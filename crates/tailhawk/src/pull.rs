//! One configured source in, one screenful of log text out.
//!
//! **This is the function the whole Loki effort was for.** `loki.rs` decided what to say,
//! `lokiwire.rs` read what comes back, `net.rs` carried it and `secrets.rs` kept the credential —
//! and until now not one of them had a caller. The owner's standing instruction, restated more than
//! once and written into `CLAUDE.md`, is that Tailhawk tails Loki **in the app**: never by shelling
//! out to `logcli` and piping it in, which works today and is not an answer to the request.
//!
//! # What this decides, and what it deliberately does not
//!
//! It returns **text**, not a file and not a document. The one function in the product that opens a
//! socket should not also be choosing where things land on disk; the caller in `main.rs` puts the
//! spill where every other document comes from. That keeps this testable in the only way it can be
//! — by reading it — and keeps the file handling in the place that already does it correctly.
//!
//! # Provenance, and the loopback that is refused
//!
//! A source read from settings is [`Provenance::Imported`], never `Typed`. That has a real cost: a
//! Loki running on `127.0.0.1` is refused, so somebody with a local one cannot point Tailhawk at it
//! from the sources dialog.
//!
//! It is still right. `SPEC.md` §12.4 explicitly supports a **curated settings file beside the exe
//! on a share** — which is precisely the "a configuration can be sent to you" shape that §7's SSRF
//! rule exists for. `Typed` belongs to a URL entered and used in the same breath; a URL that has
//! been through a file, and may have come from somebody else's file, is not that.

use tailhawk_core::loki::{self, AddressFault, Direction, Origin, OriginFault, Provenance, Window};
use tailhawk_core::lokiwire::{self, Limits, WireFault};
use tailhawk_core::settings::Source;

use crate::net::{self, Auth, NetFault};

/// Why a pull did not produce logs.
///
/// **Every variant says which half failed.** A `401` while obtaining the token means the client
/// secret is wrong; a `401` while asking Loki means the token is not accepted for this scope. One
/// error type covering both would send the reader to look in the wrong place, which is the same
/// complaint this project has made of every silent failure it has found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PullFault {
    /// The source's URL is not one Tailhawk will contact. Carries §7's own words.
    Origin(OriginFault),
    /// The URL names an address §7 refuses — loopback, link-local, or another private range.
    Address(AddressFault),
    /// The URL is `http`. **Refused here and not only in the dialog**: `Origin::parse` allows both
    /// schemes, `Source::fault` refuses `http` only while a source is being typed, and a settings
    /// file is a thing people edit by hand. Without this, a hand-written `http://` would carry a
    /// bearer token in clear text — and the token is the credential, whatever the secret is doing.
    Insecure,
    /// The token URL is not one Tailhawk will contact.
    TokenOrigin(OriginFault),
    /// The source wants a client id but Credential Manager holds no secret for it. The commonest
    /// half-finished state, and the dialog's `Secret` column exists to make it visible before this.
    NoSecret,
    /// The token exchange could not be made at all.
    TokenTransport(NetFault),
    /// The token endpoint answered, and not with a token.
    TokenRefused { status: u16 },
    /// The token endpoint answered with something that is not a token response.
    TokenUnreadable,
    /// The query could not be made at all.
    QueryTransport(NetFault),
    /// Loki answered, and not with records.
    QueryRefused { status: u16 },
    /// Loki's answer could not be read.
    Wire(WireFault),
    /// The label name this asked for was refused before it could reach a URL. Ours, not the
    /// user's, so it means a bug here rather than a configuration mistake.
    Label(tailhawk_core::loki::LabelFault),
    /// Loki answered the label call with something this could not read as a list of values.
    LabelAnswer,
    /// The source signs the reader in, and there is no sign-in to use — either none was ever made,
    /// or it expired and could not be renewed without asking again.
    ///
    /// **Not an error the caller reports and stops at.** It is the signal to start a sign-in, which
    /// is why it is told apart from [`PullFault::NoSecret`]: that one is a configuration mistake a
    /// person has to go and fix, and this one is a thing the application asks for and then carries on.
    NotSignedIn,
}

impl std::fmt::Display for PullFault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PullFault::Origin(why) => write!(f, "The source's URL {why}."),
            // **Which rule refused it, not merely that one did.** `AddressFault` distinguishes
            // loopback from a link-local address from the cloud metadata service, and those are
            // three different mistakes with three different remedies — the same argument the
            // transport arms below carry their cause for.
            PullFault::Address(why) => {
                write!(f, "That URL names an address Tailhawk will not contact — it {why}.")
            }
            PullFault::Insecure => f.write_str("That URL is http; a token may only be sent over https."),
            PullFault::TokenOrigin(why) => write!(f, "The token URL {why}."),
            PullFault::NoSecret => f.write_str(
                "No secret is stored for this source — open Tools ▸ Remote sources and paste it.",
            ),
            PullFault::NotSignedIn => f.write_str("You are not signed in to this source."),
            // The same reason the `Wire` arm below carries its cause: "could not reach it" names
            // the half that failed and nothing a person could act on. Whether this Windows has no
            // WinHTTP, the name resolved somewhere §7 refuses, or the server answered a redirect
            // are different problems with different remedies, and the fault is holding the answer.
            PullFault::TokenTransport(why) => {
                write!(f, "Could not reach the token endpoint: {why}.")
            }
            PullFault::TokenRefused { status: 401 | 400 } => {
                f.write_str("The token endpoint rejected the client secret.")
            }
            PullFault::TokenRefused { status } => {
                write!(f, "The token endpoint answered {status}.")
            }
            PullFault::TokenUnreadable => {
                f.write_str("The token endpoint answered with something that is not a token.")
            }
            PullFault::QueryTransport(why) => write!(f, "Could not reach Loki: {why}."),
            PullFault::QueryRefused { status: 401 | 403 } => {
                f.write_str("Loki refused the token — check the scope the client is allowed.")
            }
            PullFault::QueryRefused { status } => write!(f, "Loki answered {status}."),
            // The inner fault names what was wrong with the answer — a size, a shape, a status — and
            // hiding it behind one sentence cost a morning: every tail poll was failing and the
            // status bar could only say that it had.
            PullFault::Wire(why) => write!(f, "Loki's answer could not be read: {why}"),
            // **Not the reader's mistake.** The label is `APP_LABEL`, a constant this program
            // supplies, so this branch means a bug here — which is what the variant's own
            // documentation says a few lines up, while the sentence it printed said the opposite.
            PullFault::Label(_) => f.write_str(
                "Tailhawk asked Loki for a label name it would not accept. That is a fault here, not in the source.",
            ),
            PullFault::LabelAnswer => {
                f.write_str("Loki's list of applications could not be read.")
            }
        }
    }
}

/// What one pull returned.
pub struct Pulled {
    /// The records as CLEF NDJSON — `LOKI.md` §4's spill, which `format.rs`'s `ndjson` detector
    /// already recognises, so a Loki source becomes an ordinary document with no new document type.
    pub clef: String,
    /// How many records came back.
    pub records: usize,
    /// Records the response held beyond the cap and this did not keep. **Never silently zero.**
    pub dropped: usize,
    /// The newest timestamp in what came back, if anything did — where a tail resumes from.
    ///
    /// **From the records, not from the window that was asked for.** Advancing on the window would
    /// skip whatever Loki had not yet indexed when the query ran: records that exist, are late, and
    /// would never be asked for again.
    pub newest: Option<tailhawk_core::loki::Nanos>,
    /// Label values the parser cut to its cap rather than refusing the answer for. Said, per §6.
    pub truncated: usize,
}

/// Ask `source` which values one label has taken, so the picker can offer them.
///
/// **The same journey as [`pull`] and every §7 control on it** — https only, no literal address the
/// rules forbid, the secret fetched and dropped around one exchange, the bearer used for the one
/// call. A label list is a cheaper question than a window of records and it is not a safer one: it
/// goes to the same server with the same credential.
///
/// The window is the last day rather than the last hour. A label list is a menu, and a service that
/// logged nothing since lunchtime is still a service the user may want to look at; asking over too
/// short a window offers a shorter menu with nothing to say what is missing from it.
pub fn label_values(source: &Source, label: &str) -> Result<Vec<String>, PullFault> {
    let origin = Origin::parse(&source.url, Provenance::Imported).map_err(PullFault::Origin)?;
    refuse_insecure(&origin)?;
    refuse_literal_address(&origin).map_err(PullFault::Address)?;

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as i64)
        .unwrap_or_default();
    let window = Window {
        start: now.saturating_sub(LABEL_WINDOW_NANOS).max(0),
        end: now,
    };
    let request = loki::label_values(&origin, label, window).map_err(PullFault::Label)?;

    let token = if source.client_id.trim().is_empty() {
        None
    } else {
        Some(fetch_token(source)?)
    };
    let auth = match token.as_deref() {
        Some(bearer) => Auth::Bearer(bearer),
        None => Auth::None,
    };
    let answer =
        net::send(&request, Provenance::Imported, auth).map_err(PullFault::QueryTransport)?;
    if answer.status != 200 {
        return Err(PullFault::QueryRefused {
            status: answer.status,
        });
    }
    loki::label_values_from_json(&answer.body).ok_or(PullFault::LabelAnswer)
}

/// How far back the picker asks for label values. A day, for the reason [`label_values`] gives.
const LABEL_WINDOW_NANOS: i64 = 24 * 60 * 60 * 1_000_000_000;

/// Fetch a window of records from `source`.
///
/// The secret is read immediately before the exchange and dropped immediately after — it is never
/// held across the query, which carries the shorter-lived bearer token instead.
pub fn pull(
    source: &Source,
    window: Window,
    limit: u32,
    direction: Direction,
) -> Result<Pulled, PullFault> {
    let origin = Origin::parse(&source.url, Provenance::Imported).map_err(PullFault::Origin)?;
    refuse_insecure(&origin)?;
    refuse_literal_address(&origin).map_err(PullFault::Address)?;

    let token = if source.client_id.trim().is_empty() {
        None
    } else {
        Some(fetch_token(source)?)
    };

    let request = loki::query_range(&origin, &source.query, window, limit, direction);
    let auth = match token.as_deref() {
        Some(bearer) => Auth::Bearer(bearer),
        None => Auth::None,
    };
    let answer =
        net::send(&request, Provenance::Imported, auth).map_err(PullFault::QueryTransport)?;
    if answer.status != 200 {
        return Err(PullFault::QueryRefused {
            status: answer.status,
        });
    }

    let batch =
        lokiwire::parse_query_range(&answer.body, &Limits::default()).map_err(PullFault::Wire)?;
    Ok(Pulled {
        clef: lokiwire::clef_spill(&batch),
        records: batch.entries.len(),
        dropped: batch.dropped,
        newest: batch.entries.iter().map(|entry| entry.timestamp).max(),
        truncated: batch.truncated_labels,
    })
}

/// Refuse an address §7 forbids **before anything is opened**, when the URL names one literally.
///
/// The transport checks every address it actually connects to — that is the DNS-rebinding guard,
/// and it is the one that matters. This is the cheaper half: when the host *is* an address, there
/// is nothing to resolve and no reason to load the transport, open a session and reach a callback
/// before saying no. It also makes the refusal testable without a network.
fn refuse_insecure(origin: &Origin) -> Result<(), PullFault> {
    (origin.scheme() == "https")
        .then_some(())
        .ok_or(PullFault::Insecure)
}

fn refuse_literal_address(origin: &Origin) -> Result<(), AddressFault> {
    match origin.hostname().parse::<std::net::IpAddr>() {
        Ok(address) => origin.may_connect_to(address),
        // A name, not an address: only the resolver can answer, and `net.rs` asks it.
        Err(_) => Ok(()),
    }
}

/// Exchange the stored client secret for a bearer token.
///
/// **The secret exists here and nowhere else in this module.** It is fetched, handed to the send as
/// an argument, and dropped when this function returns — `net::Auth::ClientSecret` composes it into
/// the body for the duration of the call and puts it nowhere else.
fn fetch_token(source: &Source) -> Result<String, PullFault> {
    let at =
        Origin::parse(&source.token_url, Provenance::Imported).map_err(PullFault::TokenOrigin)?;
    refuse_insecure(&at)?;
    refuse_literal_address(&at).map_err(PullFault::Address)?;
    if source.signs_in() {
        return signed_in_token(source, &at);
    }
    let secret = crate::secrets::load(&source.name).ok_or(PullFault::NoSecret)?;
    if secret.is_empty() {
        return Err(PullFault::NoSecret);
    }

    let request = loki::token_request(&at, &source.client_id, &source.scope);
    let answer = net::send(&request, Provenance::Imported, Auth::ClientSecret(&secret))
        .map_err(PullFault::TokenTransport)?;
    if answer.status != 200 {
        return Err(PullFault::TokenRefused {
            status: answer.status,
        });
    }
    loki::token_from_json(&answer.body)
        .map(|token| token.access_token)
        .ok_or(PullFault::TokenUnreadable)
}

/// Asks the identity server to start a sign-in, and returns what the reader has to approve.
///
/// **The device endpoint is a third origin and gets §7's controls**, exactly as the token endpoint
/// does: its whole URL is provider configuration, so it is parsed, refused `http`, and refused a
/// literal address in the ranges §7 names.
/// The client's own credential for a sign-in, when it has one.
///
/// **A device-flow client may be confidential, and this estate's is.** RFC 8628 is often read as
/// implying a public client, but the grant says nothing of the sort: the *reader* authenticates in
/// the browser, and the client authenticates however it already does. `tailhawk` is registered with
/// `RequireClientSecret`, so its device-authorization and poll requests must carry the secret it has
/// always carried — and that is the whole reason adding one grant type is the only change the server
/// needs.
///
/// `None` when nothing is stored, which is a genuinely public client and equally valid.
fn client_authentication(source: &Source) -> Option<String> {
    crate::secrets::load(&source.name).filter(|secret| !secret.is_empty())
}

/// The borrow of [`client_authentication`]'s answer that `net::send` takes.
fn as_auth(held: &Option<String>) -> Auth<'_> {
    match held {
        Some(secret) => Auth::ClientSecret(secret),
        None => Auth::None,
    }
}

pub fn begin_sign_in(source: &Source) -> Result<loki::DeviceGrant, PullFault> {
    let at =
        Origin::parse(&source.device_url, Provenance::Imported).map_err(PullFault::TokenOrigin)?;
    refuse_insecure(&at)?;
    refuse_literal_address(&at).map_err(PullFault::Address)?;
    let request = loki::device_request(&at, &source.client_id, &source.scope);
    let held = client_authentication(source);
    let answer = net::send(&request, Provenance::Imported, as_auth(&held))
        .map_err(PullFault::TokenTransport)?;
    if answer.status != 200 {
        return Err(PullFault::TokenRefused {
            status: answer.status,
        });
    }
    loki::device_grant_from_json(&answer.body).ok_or(PullFault::TokenUnreadable)
}

/// Waits for the reader to approve `grant`, then keeps what the server issued.
///
/// **This blocks, and it must only ever be called on a worker.** It sleeps between polls for as
/// long as the server asked, which is seconds at a time — on the message loop that would be a
/// frozen window for the length of somebody typing their password.
///
/// **`slow_down` lengthens the wait rather than failing.** RFC 8628 defines it as an instruction,
/// not an error, and a client that gave up on it would be one the server had asked to be patient.
pub fn poll_sign_in(
    source: &Source,
    grant: &loki::DeviceGrant,
    cancelled: &std::sync::mpsc::Receiver<()>,
) -> Result<(), PullFault> {
    let at =
        Origin::parse(&source.token_url, Provenance::Imported).map_err(PullFault::TokenOrigin)?;
    refuse_insecure(&at)?;
    refuse_literal_address(&at).map_err(PullFault::Address)?;
    let request = loki::device_poll_request(&at, &source.client_id, &grant.device_code);
    let held = client_authentication(source);
    let mut interval = grant.interval;
    // **Bounded by the grant's own lifetime, not by a count.** The server said how long the code is
    // good for; polling past that is asking about something that no longer exists.
    let give_up_at = now_seconds().saturating_add(grant.expires_in.max(1));
    // **A blip must not end a sign-in somebody is halfway through.** RFC 8628 §3.5 has the client
    // keep polling and back off; a single dropped connection or a gateway's 502 would otherwise
    // throw away a code the reader is at that moment typing into a browser.
    let mut blips = 0u32;
    loop {
        // **The wait is also how the box closing is noticed.** A dropped sender disconnects, which
        // ends this within one interval instead of leaving a thread polling — and storing a
        // sign-in — for the rest of the grant's life.
        match cancelled.recv_timeout(std::time::Duration::from_secs(interval)) {
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            _ => return Err(PullFault::NotSignedIn),
        }
        if now_seconds() >= give_up_at {
            return Err(PullFault::NotSignedIn);
        }
        let answer = match net::send(&request, Provenance::Imported, as_auth(&held)) {
            Ok(answer) => answer,
            Err(why) => {
                blips += 1;
                if blips > POLL_BLIPS_ALLOWED {
                    return Err(PullFault::TokenTransport(why));
                }
                interval = loki::slowed(interval);
                continue;
            }
        };
        match loki::poll_outcome(&answer.body) {
            loki::Poll::Granted(token) => {
                let mut cache = access_cache()
                    .lock()
                    .unwrap_or_else(|held| held.into_inner());
                remember(&mut cache, &source.name, token)?;
                return Ok(());
            }
            loki::Poll::Pending => blips = 0,
            loki::Poll::SlowDown => {
                blips = 0;
                interval = loki::slowed(interval);
            }
            loki::Poll::Expired | loki::Poll::Denied => return Err(PullFault::NotSignedIn),
            // **A 5xx is the gateway, not the grant.** `poll_outcome` cannot tell an OAuth error
            // document from a proxy's HTML, so the status is what separates "this sign-in is
            // refused" from "this server is briefly unwell".
            loki::Poll::Failed(_) if answer.status >= 500 => {
                blips += 1;
                if blips > POLL_BLIPS_ALLOWED {
                    return Err(PullFault::TokenRefused {
                        status: answer.status,
                    });
                }
                interval = loki::slowed(interval);
            }
            loki::Poll::Failed(_) => {
                return Err(PullFault::TokenRefused {
                    status: answer.status,
                })
            }
        }
    }
}

/// How many consecutive unwell answers a sign-in tolerates before giving up on it.
const POLL_BLIPS_ALLOWED: u32 = 3;

/// Unix seconds now, or zero if the clock is before 1970 and the question is meaningless.
pub fn now_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

/// The bearer token for a source that signs the reader in.
///
/// **Three outcomes, and the third is not a failure.** A stored token that is still good is used as
/// it stands; one that has expired is renewed silently with its refresh token, because a tail that
/// stopped every hour to ask again would be worse than the client secret it replaced; and when
/// there is neither, [`PullFault::NotSignedIn`] tells the caller to ask the reader rather than
/// reporting that something broke.
fn signed_in_token(source: &Source, at: &Origin) -> Result<String, PullFault> {
    // **The lock is held across the whole load-refresh-store, and that is its point.** These
    // refresh tokens are single-use: an opening pull and a tail poll racing would each spend one,
    // and the loser's would come back `invalid_grant` — signing the reader out mid-tail because two
    // parts of this program asked at the same moment.
    let mut cache = access_cache()
        .lock()
        .unwrap_or_else(|held| held.into_inner());
    if let Some(cached) = cache.get(&source.name) {
        if crate::secrets::usable(cached, now_seconds()) {
            return Ok(cached.access.clone());
        }
    }
    let refresh = crate::secrets::load_refresh(&source.name).ok_or(PullFault::NotSignedIn)?;
    let request = loki::refresh_request(at, &source.client_id, &refresh);
    let held = client_authentication(source);
    let answer = net::send(&request, Provenance::Imported, as_auth(&held))
        .map_err(PullFault::TokenTransport)?;
    if answer.status != 200 {
        // **Only `invalid_grant` ends a sign-in.** RFC 6749 §5.2 defines it as the spent, revoked or
        // expired refresh token; a 503 or a rate limit is a reason to try again later, and throwing
        // the credential away for one of those would sign the reader out over a blip.
        if loki::oauth_error(&answer.body).as_deref() == Some("invalid_grant") {
            crate::secrets::forget_refresh(&source.name);
            cache.remove(&source.name);
            return Err(PullFault::NotSignedIn);
        }
        return Err(PullFault::TokenRefused {
            status: answer.status,
        });
    }
    let renewed = loki::token_from_json(&answer.body).ok_or(PullFault::TokenUnreadable)?;
    let access = renewed.access_token.clone();
    remember(&mut cache, &source.name, renewed)?;
    Ok(access)
}

/// The access tokens this process holds, by source name.
///
/// **In memory rather than in the store, and that is forced rather than preferred.** `CredWriteW`
/// refuses a blob over 2,560 bytes and a signed JWT access token from this estate is routinely more
/// than half of that, so keeping it beside the refresh token would fail the write *after* the reader
/// had approved the sign-in — and the device code is single-use, so the remedy would be to do the
/// whole thing again. It is also the part worth keeping least: it expires within the hour, and the
/// refresh token mints another.
fn access_cache(
) -> &'static std::sync::Mutex<std::collections::HashMap<String, crate::secrets::Tokens>> {
    static CACHE: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<String, crate::secrets::Tokens>>,
    > = std::sync::OnceLock::new();
    CACHE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

/// Keeps what the server just issued: the refresh token in the store, the access token in memory.
///
/// **A rotated refresh token that cannot be written signs the reader out now rather than later.**
/// Duende rotates them, so the one already in the store is spent — leaving it there would fail at
/// some arbitrary later moment with no connection to what caused it.
fn remember(
    cache: &mut std::collections::HashMap<String, crate::secrets::Tokens>,
    name: &str,
    token: loki::Token,
) -> Result<(), PullFault> {
    if let Some(rotated) = token.refresh_token.as_deref() {
        if !crate::secrets::store_refresh(name, rotated) {
            crate::secrets::forget_refresh(name);
            cache.remove(name);
            return Err(PullFault::NotSignedIn);
        }
    }
    cache.insert(
        name.to_owned(),
        crate::secrets::Tokens {
            access: token.access_token,
            refresh: None,
            expires_at: expiry_from(token.expires_in),
        },
    );
    Ok(())
}

/// The absolute moment a token issued now stops being any use, or zero when the server said nothing.
pub fn expiry_from(expires_in: u64) -> u64 {
    if expires_in == 0 {
        return 0;
    }
    now_seconds().saturating_add(expires_in)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source() -> Source {
        Source {
            name: "pull-selftest".to_owned(),
            url: "https://telemetry.example/loki".to_owned(),
            token_url: "https://identity.example/connect/token".to_owned(),
            client_id: "tailhawk".to_owned(),
            scope: "telemetry:read".to_owned(),
            query: "{environment=\"dev\"}".to_owned(),
            device_url: String::new(),
        }
    }

    /// **A bad URL is refused before anything is opened**, and says which URL — the two are
    /// configured in different boxes of the same dialog, so "the URL is wrong" is not an answer.
    #[test]
    fn a_url_that_cannot_be_contacted_is_refused_before_the_transport_is_touched() {
        let loopback = Source {
            url: "https://127.0.0.1:3100".to_owned(),
            ..source()
        };
        assert!(
            matches!(
                pull(
                    &loopback,
                    Window { start: 0, end: 1 },
                    10,
                    Direction::Backward
                ),
                Err(PullFault::Address(_))
            ),
            "a settings-borne URL is Imported, so loopback is refused — see the module note"
        );

        let bad_token_url = Source {
            token_url: "http://identity.example/connect/token".to_owned(),
            ..source()
        };
        assert!(matches!(
            pull(
                &bad_token_url,
                Window { start: 0, end: 1 },
                10,
                Direction::Backward
            ),
            Err(PullFault::Insecure)
        ));

        // **Nothing above loaded the transport.** §13.2's claim is that a run which opens no remote
        // source leaves `winhttp.dll` out of the process entirely, and a refusal is not an opening.
        assert!(
            !net::transport_is_loaded(),
            "a refused URL must not have loaded the transport"
        );
    }

    /// A source configured for a client it has no secret for says so, rather than sending an
    /// exchange that cannot succeed. This is the state the dialog's `Secret` column exists to show.
    #[test]
    fn a_source_with_no_stored_secret_says_so_rather_than_asking() {
        let _ = crate::secrets::forget("pull-selftest");
        assert_eq!(
            pull(
                &source(),
                Window { start: 0, end: 1 },
                10,
                Direction::Backward
            )
            .err(),
            Some(PullFault::NoSecret)
        );
        assert!(
            !net::transport_is_loaded(),
            "and still nothing has been opened"
        );
    }

    /// **`http` is refused by the caller, not only by the dialog** — for either URL.
    ///
    /// `Origin::parse` accepts both schemes and `settings::Source::fault` refuses `http` only while
    /// a source is being typed. A settings file is a thing §12.4 expects people to edit by hand, and
    /// one that named `http://` would have sent a bearer token in clear text. The token *is* the
    /// credential once the exchange has happened, so this matters as much as the secret does.
    #[test]
    fn http_is_refused_for_the_query_url_and_the_token_url_alike() {
        for insecure in [
            Source {
                url: "http://telemetry.example/loki".to_owned(),
                ..source()
            },
            Source {
                token_url: "http://identity.example/connect/token".to_owned(),
                ..source()
            },
        ] {
            assert_eq!(
                pull(
                    &insecure,
                    Window { start: 0, end: 1 },
                    10,
                    Direction::Backward
                )
                .err(),
                Some(PullFault::Insecure),
                "http must be refused however it got into the settings file"
            );
        }
        assert!(
            !net::transport_is_loaded(),
            "and refusing it opened nothing"
        );
    }

    /// Every fault says which half failed and reads as something a person can act on.
    #[test]
    fn each_fault_names_the_half_that_failed() {
        let said = |fault: PullFault| fault.to_string();
        assert!(said(PullFault::TokenRefused { status: 401 }).contains("client secret"));
        assert!(said(PullFault::QueryRefused { status: 401 }).contains("scope"));
        assert!(said(PullFault::NoSecret).contains("Remote sources"));
        assert_ne!(
            said(PullFault::TokenTransport(NetFault::NoTransport)),
            said(PullFault::QueryTransport(NetFault::NoTransport)),
            "reaching the token endpoint and reaching Loki are different failures"
        );
        assert!(said(PullFault::QueryRefused { status: 503 }).contains("503"));

        // **The menu path must be one that exists.** This message said `Settings ▸ Remote sources`
        // for eight days after the Settings menu became Tools — an instruction a user could follow
        // to a menu that was not there. `menubar.rs` builds `&Remote sources…` under `&Tools`, so
        // that is the path, and this asserts the whole of it rather than the item's name alone.
        assert!(
            said(PullFault::NoSecret).contains("Tools ▸ Remote sources"),
            "the path names the menu it is actually under: {}",
            said(PullFault::NoSecret)
        );
        assert!(
            !said(PullFault::NoSecret).contains("Settings"),
            "and never the menu that no longer exists"
        );

        // **The last arm that was handed a fault and discarded it.** Not one of the three findings
        // this batch set out to fix — a review asked what was left inconsistent afterwards, and
        // this was: `AddressFault` says which address rule refused the URL, and "will not contact"
        // alone leaves a person guessing between loopback, link-local and a metadata service.
        let address = said(PullFault::Address(AddressFault::Metadata));
        assert!(
            address.contains("cloud metadata service"),
            "the address rule that refused it travels with the refusal: {address}"
        );

        // **A transport arm holds its cause and used to throw it away.** The `Wire` arm already
        // interpolates the inner fault, and hiding that one cost a morning of tail polls failing
        // with a status bar that could only say they had.
        for said in [
            said(PullFault::TokenTransport(NetFault::NoTransport)),
            said(PullFault::QueryTransport(NetFault::NoTransport)),
        ] {
            assert!(
                said.contains("WinHTTP"),
                "the cause travels with the failure: {said}"
            );
        }

        // **Not the user's mistake.** The label name is `APP_LABEL`, a constant this program
        // supplies; a sentence blaming whoever is reading for it is blaming them for a Tailhawk
        // bug. (Routing this away from the status bar altogether belongs with finding 11, which
        // owns the call site.)
        // **Asserted whole, not by substring.** The first version of this checked that one dead
        // phrase was absent and the word `Tailhawk` present, which a sentence still blaming the
        // reader passes easily — "The label name Tailhawk was given is wrong." satisfies both. A
        // review broke it exactly that way, so the sentence is pinned as a sentence.
        assert_eq!(
            said(PullFault::Label(
                tailhawk_core::loki::LabelFault::NotALabelName
            )),
            "Tailhawk asked Loki for a label name it would not accept. \
             That is a fault here, not in the source.",
        );
    }
}
