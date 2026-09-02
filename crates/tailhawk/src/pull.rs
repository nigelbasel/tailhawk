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
}

impl std::fmt::Display for PullFault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PullFault::Origin(why) => write!(f, "The source's URL {why}."),
            PullFault::Address(_) => f.write_str("That URL names an address Tailhawk will not contact."),
            PullFault::Insecure => f.write_str("That URL is http; a token may only be sent over https."),
            PullFault::TokenOrigin(why) => write!(f, "The token URL {why}."),
            PullFault::NoSecret => f.write_str(
                "No secret is stored for this source — open Settings ▸ Remote sources and paste it.",
            ),
            PullFault::TokenTransport(_) => f.write_str("Could not reach the token endpoint."),
            PullFault::TokenRefused { status: 401 | 400 } => {
                f.write_str("The token endpoint rejected the client secret.")
            }
            PullFault::TokenRefused { status } => {
                write!(f, "The token endpoint answered {status}.")
            }
            PullFault::TokenUnreadable => {
                f.write_str("The token endpoint answered with something that is not a token.")
            }
            PullFault::QueryTransport(_) => f.write_str("Could not reach Loki."),
            PullFault::QueryRefused { status: 401 | 403 } => {
                f.write_str("Loki refused the token — check the scope the client is allowed.")
            }
            PullFault::QueryRefused { status } => write!(f, "Loki answered {status}."),
            PullFault::Wire(_) => f.write_str("Loki's answer could not be read."),
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
}

/// Fetch a window of records from `source`.
///
/// The secret is read immediately before the exchange and dropped immediately after — it is never
/// held across the query, which carries the shorter-lived bearer token instead.
pub fn pull(source: &Source, window: Window, limit: u32) -> Result<Pulled, PullFault> {
    let origin = Origin::parse(&source.url, Provenance::Imported).map_err(PullFault::Origin)?;
    refuse_insecure(&origin)?;
    refuse_literal_address(&origin).map_err(PullFault::Address)?;

    let token = if source.client_id.trim().is_empty() {
        None
    } else {
        Some(fetch_token(source)?)
    };

    let request = loki::query_range(&origin, &source.query, window, limit, Direction::Backward);
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
                pull(&loopback, Window { start: 0, end: 1 }, 10),
                Err(PullFault::Address(_))
            ),
            "a settings-borne URL is Imported, so loopback is refused — see the module note"
        );

        let bad_token_url = Source {
            token_url: "http://identity.example/connect/token".to_owned(),
            ..source()
        };
        assert!(matches!(
            pull(&bad_token_url, Window { start: 0, end: 1 }, 10),
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
            pull(&source(), Window { start: 0, end: 1 }, 10).err(),
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
                pull(&insecure, Window { start: 0, end: 1 }, 10).err(),
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
    }
}
