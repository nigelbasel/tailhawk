//! The only place Tailhawk opens a socket.
//!
//! `loki.rs` decides what to say and `lokiwire.rs` reads what comes back; this is the part in
//! between, and it is deliberately the narrowest thing that can carry a `loki::Request` to a server
//! and bring the text home.
//!
//! # Why this is `LoadLibraryW` and not a link
//!
//! `SPEC.md` §13.2 promises no outbound connection **unless the user explicitly opens a remote
//! source**, and CI checks it. That conditional is only checkable if a run which opens no remote
//! source leaves `winhttp.dll` out of the process module list altogether — so WinHTTP is resolved
//! on **first use**, never at startup, and the `windows` crate's own `Win32_Networking_WinHttp`
//! bindings are deliberately not used because they emit a *static import*: the module would be
//! present in every process, whether or not anything ever called it, and the assertion could not be
//! written at all. Refusing the convenient path is the point.
//!
//! The shape is `darkmode.rs`'s, which already reaches `uxtheme.dll` this way.
//!
//! # What §7 forbids, and is enforced here rather than remembered
//!
//! - **Redirects are not followed.** WinHTTP's documented default chases cross-origin https→https
//!   redirects up to ten hops with no application involvement, which would carry an
//!   `Authorization` header to a host the credential was never bound to. The option is turned off
//!   and a redirect comes back as a status for the caller to refuse.
//! - **WPAD auto-detect is never enabled**, and `WINHTTP_AUTOLOGON_SECURITY_LEVEL` is never
//!   lowered. The sample code an implementer copies enables a proxy chosen by a broadcast race and
//!   auto-submits domain credentials to it.
//! - **The token is a parameter, never a field.** Nothing here holds a credential between calls, so
//!   there is no transport state that could be logged or dumped with one in it.

use std::cell::RefCell;
use std::net::IpAddr;
use std::sync::OnceLock;

use windows::core::{PCSTR, PCWSTR};
use windows::Win32::Foundation::HMODULE;
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress, LoadLibraryW};

use tailhawk_core::loki::{address_verdict, AddressFault, Provenance, Request};

/// An opaque WinHTTP handle. Session, connection and request are all `HINTERNET`.
type Handle = *mut core::ffi::c_void;

type OpenFn = unsafe extern "system" fn(PCWSTR, u32, PCWSTR, PCWSTR, u32) -> Handle;
type ConnectFn = unsafe extern "system" fn(Handle, PCWSTR, u16, u32) -> Handle;
type OpenRequestFn =
    unsafe extern "system" fn(Handle, PCWSTR, PCWSTR, PCWSTR, PCWSTR, *const PCWSTR, u32) -> Handle;
type SetOptionFn = unsafe extern "system" fn(Handle, u32, *const core::ffi::c_void, u32) -> i32;
type SendRequestFn = unsafe extern "system" fn(
    Handle,
    PCWSTR,
    u32,
    *const core::ffi::c_void,
    u32,
    u32,
    usize,
) -> i32;
type ReceiveResponseFn = unsafe extern "system" fn(Handle, *mut core::ffi::c_void) -> i32;
type QueryHeadersFn = unsafe extern "system" fn(
    Handle,
    u32,
    PCWSTR,
    *mut core::ffi::c_void,
    *mut u32,
    *mut u32,
) -> i32;
type QueryDataAvailableFn = unsafe extern "system" fn(Handle, *mut u32) -> i32;
type ReadDataFn = unsafe extern "system" fn(Handle, *mut core::ffi::c_void, u32, *mut u32) -> i32;
type CloseHandleFn = unsafe extern "system" fn(Handle) -> i32;
type StatusCallback = unsafe extern "system" fn(Handle, usize, u32, *mut core::ffi::c_void, u32);
type SetStatusCallbackFn =
    unsafe extern "system" fn(Handle, Option<StatusCallback>, u32, usize) -> usize;

/// The entry points, resolved once, on the first call that actually needs them.
struct WinHttp {
    open: OpenFn,
    connect: ConnectFn,
    open_request: OpenRequestFn,
    set_option: SetOptionFn,
    send_request: SendRequestFn,
    receive_response: ReceiveResponseFn,
    query_headers: QueryHeadersFn,
    query_data_available: QueryDataAvailableFn,
    read_data: ReadDataFn,
    close: CloseHandleFn,
    set_status_callback: SetStatusCallbackFn,
}

// SAFETY: every field is a function pointer into a module that is never freed — the `LoadLibraryW`
// reference is deliberately leaked, because a process that has opened a remote source once will
// open another.
unsafe impl Send for WinHttp {}
unsafe impl Sync for WinHttp {}

static WINHTTP: OnceLock<Option<WinHttp>> = OnceLock::new();

/// The name is written once, here, so that a search of this repository for the string finds the
/// single place the transport can enter the process.
const WINHTTP_DLL: &str = "winhttp.dll\0";

/// Whether `winhttp.dll` is in this process at all.
///
/// **This is the observable half of `SPEC.md` §13.2's conditional claim**, and it is `pub` so a
/// test can ask. A run that has not opened a remote source must answer `false`; if it ever answers
/// `true` after purely local work, something has linked the transport in rather than loading it,
/// and the promise is broken whether or not a socket was opened.
pub fn transport_is_loaded() -> bool {
    let name: Vec<u16> = WINHTTP_DLL.encode_utf16().collect();
    unsafe { GetModuleHandleW(PCWSTR(name.as_ptr())) }
        .map(|h: HMODULE| !h.is_invalid())
        .unwrap_or(false)
}

fn winhttp() -> Option<&'static WinHttp> {
    WINHTTP
        .get_or_init(|| {
            let name: Vec<u16> = WINHTTP_DLL.encode_utf16().collect();
            let module = unsafe { LoadLibraryW(PCWSTR(name.as_ptr())) }.ok()?;
            let entry = |symbol: &str| -> Option<*const core::ffi::c_void> {
                let mut owned = symbol.to_owned();
                owned.push('\0');
                unsafe { GetProcAddress(module, PCSTR(owned.as_ptr())) }
                    .map(|f| f as *const core::ffi::c_void)
            };
            Some(WinHttp {
                open: unsafe {
                    std::mem::transmute::<*const core::ffi::c_void, OpenFn>(entry("WinHttpOpen")?)
                },
                connect: unsafe {
                    std::mem::transmute::<*const core::ffi::c_void, ConnectFn>(entry(
                        "WinHttpConnect",
                    )?)
                },
                open_request: unsafe {
                    std::mem::transmute::<*const core::ffi::c_void, OpenRequestFn>(entry(
                        "WinHttpOpenRequest",
                    )?)
                },
                set_option: unsafe {
                    std::mem::transmute::<*const core::ffi::c_void, SetOptionFn>(entry(
                        "WinHttpSetOption",
                    )?)
                },
                send_request: unsafe {
                    std::mem::transmute::<*const core::ffi::c_void, SendRequestFn>(entry(
                        "WinHttpSendRequest",
                    )?)
                },
                receive_response: unsafe {
                    std::mem::transmute::<*const core::ffi::c_void, ReceiveResponseFn>(entry(
                        "WinHttpReceiveResponse",
                    )?)
                },
                query_headers: unsafe {
                    std::mem::transmute::<*const core::ffi::c_void, QueryHeadersFn>(entry(
                        "WinHttpQueryHeaders",
                    )?)
                },
                query_data_available: unsafe {
                    std::mem::transmute::<*const core::ffi::c_void, QueryDataAvailableFn>(entry(
                        "WinHttpQueryDataAvailable",
                    )?)
                },
                read_data: unsafe {
                    std::mem::transmute::<*const core::ffi::c_void, ReadDataFn>(entry(
                        "WinHttpReadData",
                    )?)
                },
                close: unsafe {
                    std::mem::transmute::<*const core::ffi::c_void, CloseHandleFn>(entry(
                        "WinHttpCloseHandle",
                    )?)
                },
                set_status_callback: unsafe {
                    std::mem::transmute::<*const core::ffi::c_void, SetStatusCallbackFn>(entry(
                        "WinHttpSetStatusCallback",
                    )?)
                },
            })
        })
        .as_ref()
}

/// What came back, or why nothing did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
    /// The HTTP status line's code.
    pub status: u16,
    /// The body, decoded as UTF-8 with invalid sequences replaced — a server that sends malformed
    /// UTF-8 gets read as best it can rather than taking the source down.
    pub body: String,
}

/// Why a request could not be completed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetFault {
    /// `winhttp.dll` is not present, or does not export what this needs. Every supported Windows
    /// has it, so this means something unusual rather than something to design around.
    NoTransport,
    /// A WinHTTP call failed. The number is `GetLastError`, which for this API is the documented
    /// `ERROR_WINHTTP_*` range.
    Failed {
        /// What was being attempted.
        during: &'static str,
        /// The Windows error code.
        code: u32,
    },
    /// The server answered with a redirect. §7 forbids following one, because the next hop may be
    /// a different origin and the credential is bound to this one.
    Redirected {
        /// The 3xx status returned.
        status: u16,
    },
    /// The response is larger than [`MAX_RESPONSE`].
    TooLarge,
    /// The name resolved to an address §7 refuses. **This is the DNS-rebinding check**, made
    /// against the address WinHTTP actually connected to rather than one we resolved separately.
    Refused {
        /// The address that came back.
        address: String,
        /// Why it is not allowed.
        fault: AddressFault,
    },
}

impl std::fmt::Display for NetFault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NetFault::NoTransport => f.write_str("this Windows has no usable WinHTTP"),
            NetFault::Failed { during, code } => write!(f, "{during} failed (error {code})"),
            NetFault::Redirected { status } => {
                write!(
                    f,
                    "the server answered {status} — a redirect, which is not followed"
                )
            }
            NetFault::TooLarge => f.write_str("the response is larger than will be read"),
            NetFault::Refused { address, fault } => {
                write!(f, "the name resolved to {address}, which {fault}")
            }
        }
    }
}

/// What the connection watcher saw, for the request being made on this thread.
///
/// **Thread-local because a request is synchronous here**: [`send`] installs the watcher, makes one
/// call and reads the answer before returning, so there is exactly one request in flight per thread
/// and no ambiguity about which one a notification belongs to. If this ever becomes concurrent, the
/// context pointer `WinHttpSetStatusCallback` carries is the place to put this instead.
struct Watch {
    /// The provenance the address must be judged against — see `loki::Provenance`.
    provenance: Provenance,
    /// The first refusal seen, if any. Recorded rather than acted on inside the callback.
    refused: Option<(String, AddressFault)>,
    /// Whether any address was seen at all, which is how a connection that never resolved is told
    /// apart from one that resolved to something allowed.
    seen: bool,
}

thread_local! {
    static WATCH: RefCell<Option<Watch>> = const { RefCell::new(None) };
}

/// Judge one address as WinHTTP reports it — the text form it hands the status callback.
///
/// Separated out because it is the whole of the decision and can be tested without a socket: the
/// callback's only job is to hand this a string and remember what it said.
fn verdict_of_text(text: &str, provenance: Provenance) -> Result<(), AddressFault> {
    match text.trim().parse::<IpAddr>() {
        Ok(address) => address_verdict(address, provenance),
        // A notification that is not an address is not a licence to connect to it, but neither is
        // it evidence of anything; WinHTTP documents this field as the server's address, so an
        // unparseable one means the notification was not the one we think it was.
        Err(_) => Ok(()),
    }
}

/// WinHTTP's status callback, watching for the address it is about to connect to.
///
/// **Records; does not decide.** Aborting from inside a callback is documented but delicate, and a
/// refusal that raced the send would be worse than useless — so the verdict is remembered here and
/// enforced by [`send`], which refuses to read or return anything when a refusal was recorded. The
/// residual exposure is stated plainly in [`send`]'s own note rather than papered over.
unsafe extern "system" fn watch_connection(
    _handle: Handle,
    _context: usize,
    status: u32,
    info: *mut core::ffi::c_void,
    length: u32,
) {
    if status != WINHTTP_CALLBACK_STATUS_CONNECTING_TO_SERVER || info.is_null() {
        return;
    }
    let text = unsafe { wide_to_string(info.cast::<u16>(), length) };
    WATCH.with(|w| {
        let mut w = w.borrow_mut();
        let Some(watch) = w.as_mut() else {
            return;
        };
        watch.seen = true;
        if watch.refused.is_none() {
            if let Err(fault) = verdict_of_text(&text, watch.provenance) {
                watch.refused = Some((text, fault));
            }
        }
    });
}

/// Read a UTF-16 string WinHTTP owns, bounded **twice**.
///
/// `bytes` is the notification's own `dwStatusInformationLength`, which is the only thing that
/// knows how long the buffer really is; the 256 is a second ceiling in case a notification lies or
/// carries something that is not a string at all. Taking the smaller of the two is what stops a
/// missing terminator walking the heap — and the reason it matters is that this callback used to
/// be handed the wrong notification entirely, so the buffer it read was never a string.
unsafe fn wide_to_string(ptr: *const u16, bytes: u32) -> String {
    let ceiling = (bytes as usize / 2).min(256);
    let mut len = 0usize;
    while len < ceiling && unsafe { *ptr.add(len) } != 0 {
        len += 1;
    }
    String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(ptr, len) })
}

/// The most body this will accumulate. `lokiwire::Limits` caps the parse; this caps the read, so a
/// server that streams for ever is stopped before the parser is ever asked.
pub const MAX_RESPONSE: usize = 64 * 1024 * 1024;

/// **`4`, and it was `0x20` for a day.** `0x20` is `WINHTTP_CALLBACK_STATUS_REQUEST_SENT`, so the
/// watcher was handed a byte count where it expected an address, read it as a string, failed to
/// parse it, and allowed the connection. §7's rebinding check — the reason the status callback was
/// chosen over resolving the name ourselves — could never fire. Nothing here could have caught it:
/// the value is only meaningful to WinHTTP, and no test in this crate talks to WinHTTP.
/// [`tests::the_hand_declared_constants_match_the_official_bindings`] is the answer to that.
const WINHTTP_CALLBACK_STATUS_CONNECTING_TO_SERVER: u32 = 0x0000_0004;
const WINHTTP_CALLBACK_FLAG_ALL_NOTIFICATIONS: u32 = 0xffff_ffff;
/// `WINHTTP_INVALID_STATUS_CALLBACK` — what `WinHttpSetStatusCallback` returns on failure.
const INVALID_STATUS_CALLBACK: usize = usize::MAX;
const WINHTTP_FLAG_SECURE: u32 = 0x0080_0000;
const WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY: u32 = 4;
const WINHTTP_OPTION_DISABLE_FEATURE: u32 = 63;
const WINHTTP_DISABLE_REDIRECTS: u32 = 0x0000_0002;
const WINHTTP_OPTION_SECURE_PROTOCOLS: u32 = 84;
/// TLS 1.2 and 1.3 only. §7 asks for a settled TLS posture with no insecure toggle anywhere; the
/// older protocols are not offered, and there is no configuration that can put them back.
const TLS12_AND_13: u32 = 0x0000_0800 | 0x0000_2000;
const WINHTTP_QUERY_STATUS_CODE: u32 = 19;
const WINHTTP_QUERY_FLAG_NUMBER: u32 = 0x2000_0000;

/// Send one request and read the answer.
///
/// `token` is attached as a bearer credential for this call only. The caller is responsible for
/// having checked, through `loki::Origin::may_connect_to`, that the address this resolves to is one
/// §7 permits — this function performs the request it is given and does not re-litigate it.
/// How one request proves who is asking.
///
/// **A credential is an argument to the send, never a field on the request.** `loki::Request`'s
/// contract is that it holds no secret and may therefore be logged or shown; this is the type that
/// keeps that true, by carrying the credential separately and only for the duration of the call.
#[derive(Debug, Clone, Copy)]
pub enum Auth<'a> {
    /// No credential — a Loki with `auth_enabled: false` and no proxy in front of it.
    None,
    /// `Authorization: Bearer …`, for a query once a token has been obtained.
    Bearer(&'a str),
    /// A client secret appended to the form body, for the token exchange itself.
    ///
    /// It goes in the **body**, not the query string: a secret in a URL is a secret in every proxy
    /// access log on the path, which is the same argument `LOKI.md` §7 makes for `query_range`
    /// being a POST.
    ClientSecret(&'a str),
}

pub fn send(request: &Request, provenance: Provenance, auth: Auth<'_>) -> Result<Answer, NetFault> {
    let api = winhttp().ok_or(NetFault::NoTransport)?;
    let target = Target::parse(&request.url).ok_or(NetFault::Failed {
        during: "reading the URL",
        code: 0,
    })?;

    let _watching = Watching::begin(provenance);
    let session = Session::open(api)?;
    session.watch()?;
    let connection = session.connect(&target)?;
    let call = connection.request(&target, request.method)?;
    call.disable_redirects()?;

    // The send is where the address is resolved and connected to, so the refusal check comes
    // straight after it and **before** anything is read.
    let sent = call.send(request, auth);
    if let Some((address, fault)) = Watching::refusal() {
        return Err(NetFault::Refused { address, fault });
    }
    sent?;
    call.receive()?;
    if let Some((address, fault)) = Watching::refusal() {
        return Err(NetFault::Refused { address, fault });
    }

    let status = call.status()?;
    if (300..400).contains(&status) {
        return Err(NetFault::Redirected { status });
    }
    Ok(Answer {
        status,
        body: call.read_body()?,
    })
}

/// Installs the per-request watch and takes it down again however the request ends.
struct Watching;

impl Watching {
    fn begin(provenance: Provenance) -> Watching {
        WATCH.with(|w| {
            *w.borrow_mut() = Some(Watch {
                provenance,
                refused: None,
                seen: false,
            });
        });
        Watching
    }

    fn refusal() -> Option<(String, AddressFault)> {
        WATCH.with(|w| w.borrow().as_ref().and_then(|watch| watch.refused.clone()))
    }

    /// Whether any address was reported at all. Used by the tests; a request that connected without
    /// the watch ever firing would mean the notification is not arriving and the check is dead.
    #[cfg(test)]
    fn saw_an_address() -> bool {
        WATCH.with(|w| w.borrow().as_ref().is_some_and(|watch| watch.seen))
    }
}

impl Drop for Watching {
    fn drop(&mut self) {
        WATCH.with(|w| *w.borrow_mut() = None);
    }
}

/// A URL split into the pieces WinHTTP wants separately.
struct Target {
    host: Vec<u16>,
    port: u16,
    path: Vec<u16>,
    secure: bool,
}

impl Target {
    fn parse(url: &str) -> Option<Target> {
        let (scheme, rest) = url.split_once("://")?;
        let secure = match scheme {
            "https" | "wss" => true,
            "http" | "ws" => false,
            _ => return None,
        };
        let (authority, path) = match rest.find('/') {
            Some(at) => (&rest[..at], &rest[at..]),
            None => (rest, "/"),
        };
        let (host, port) = match authority.rsplit_once(':') {
            Some((h, p)) => (h, p.parse::<u16>().ok()?),
            None => (authority, if secure { 443 } else { 80 }),
        };
        Some(Target {
            host: wide(host),
            port,
            path: wide(path),
            secure,
        })
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn failure(during: &'static str) -> NetFault {
    NetFault::Failed {
        during,
        code: unsafe { windows::Win32::Foundation::GetLastError() }.0,
    }
}

/// A WinHTTP handle that closes itself.
struct Owned {
    api: &'static WinHttp,
    handle: Handle,
}

impl Drop for Owned {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            unsafe { (self.api.close)(self.handle) };
        }
    }
}

struct Session(Owned);

impl Session {
    fn open(api: &'static WinHttp) -> Result<Session, NetFault> {
        // **`AUTOMATIC_PROXY` rather than the sample's WPAD auto-detect.** §7's second correction:
        // the code an implementer copies enables DHCP/DNS auto-discovery and sets
        // `fAutoLogonIfChallenged`, which submits domain credentials to a proxy chosen by a
        // broadcast race. This asks Windows for the machine's configured proxy and nothing more.
        let agent = wide(concat!("Tailhawk/", env!("CARGO_PKG_VERSION")));
        let handle = unsafe {
            (api.open)(
                PCWSTR(agent.as_ptr()),
                WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
                PCWSTR::null(),
                PCWSTR::null(),
                0,
            )
        };
        if handle.is_null() {
            return Err(failure("opening a session"));
        }
        let session = Session(Owned { api, handle });
        let protocols = TLS12_AND_13;
        unsafe {
            (api.set_option)(
                handle,
                WINHTTP_OPTION_SECURE_PROTOCOLS,
                (&protocols as *const u32).cast(),
                4,
            )
        };
        Ok(session)
    }

    /// Ask WinHTTP to report what it is connecting to, so §7's address check can be made against
    /// the address actually used rather than one resolved separately.
    fn watch(&self) -> Result<(), NetFault> {
        let previous = unsafe {
            (self.0.api.set_status_callback)(
                self.0.handle,
                Some(watch_connection),
                WINHTTP_CALLBACK_FLAG_ALL_NOTIFICATIONS,
                0,
            )
        };
        if previous == INVALID_STATUS_CALLBACK {
            return Err(failure("watching the connection"));
        }
        Ok(())
    }

    fn connect(&self, target: &Target) -> Result<Connection, NetFault> {
        let handle = unsafe {
            (self.0.api.connect)(self.0.handle, PCWSTR(target.host.as_ptr()), target.port, 0)
        };
        if handle.is_null() {
            return Err(failure("connecting"));
        }
        Ok(Connection(Owned {
            api: self.0.api,
            handle,
        }))
    }
}

/// A connection to one host and port.
///
/// The handles are closed in reverse declaration order in [`send`] — call, then connection, then
/// session — which is the order WinHTTP documents. That ordering is Rust's own drop order rather
/// than anything this has to arrange.
struct Connection(Owned);

struct Call(Owned);

impl Call {
    fn disable_redirects(&self) -> Result<(), NetFault> {
        let disable = WINHTTP_DISABLE_REDIRECTS;
        let ok = unsafe {
            (self.0.api.set_option)(
                self.0.handle,
                WINHTTP_OPTION_DISABLE_FEATURE,
                (&disable as *const u32).cast(),
                4,
            )
        };
        if ok == 0 {
            return Err(failure("refusing redirects"));
        }
        Ok(())
    }

    fn send(&self, request: &Request, auth: Auth<'_>) -> Result<(), NetFault> {
        let mut headers = String::new();
        if let Some(kind) = request.content_type {
            headers.push_str("Content-Type: ");
            headers.push_str(kind);
            headers.push_str("\r\n");
        }
        if let Auth::Bearer(token) = auth {
            headers.push_str("Authorization: Bearer ");
            headers.push_str(token);
            headers.push_str("\r\n");
        }
        // **The client secret is composed into the body here and nowhere else.** `loki::Request`
        // promises that nothing in it is a secret — it may be logged, shown in a dialog, compared
        // in a test — so the one place a secret may exist is this local, for as long as this call
        // takes. It is deliberately not stored, not returned and not put back on the request.
        let composed;
        let body = match auth {
            Auth::ClientSecret(secret) => {
                composed = format!(
                    "{}&client_secret={}",
                    request.body,
                    tailhawk_core::loki::form_encode(secret)
                );
                composed.as_bytes()
            }
            _ => request.body.as_bytes(),
        };
        let wide_headers = wide(&headers);
        let ok = unsafe {
            (self.0.api.send_request)(
                self.0.handle,
                if headers.is_empty() {
                    PCWSTR::null()
                } else {
                    PCWSTR(wide_headers.as_ptr())
                },
                if headers.is_empty() { 0 } else { u32::MAX },
                body.as_ptr().cast(),
                body.len() as u32,
                body.len() as u32,
                0,
            )
        };
        if ok == 0 {
            return Err(failure("sending the request"));
        }
        Ok(())
    }

    fn receive(&self) -> Result<(), NetFault> {
        let ok = unsafe { (self.0.api.receive_response)(self.0.handle, std::ptr::null_mut()) };
        if ok == 0 {
            return Err(failure("waiting for the response"));
        }
        Ok(())
    }

    fn status(&self) -> Result<u16, NetFault> {
        let mut code: u32 = 0;
        let mut size: u32 = 4;
        let ok = unsafe {
            (self.0.api.query_headers)(
                self.0.handle,
                WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
                PCWSTR::null(),
                (&mut code as *mut u32).cast(),
                &mut size,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(failure("reading the status"));
        }
        Ok(code as u16)
    }

    fn read_body(&self) -> Result<String, NetFault> {
        let mut out: Vec<u8> = Vec::new();
        loop {
            let mut waiting: u32 = 0;
            if unsafe { (self.0.api.query_data_available)(self.0.handle, &mut waiting) } == 0 {
                return Err(failure("asking how much is waiting"));
            }
            if waiting == 0 {
                break;
            }
            if out.len() + waiting as usize > MAX_RESPONSE {
                return Err(NetFault::TooLarge);
            }
            let at = out.len();
            out.resize(at + waiting as usize, 0);
            let mut read: u32 = 0;
            let ok = unsafe {
                (self.0.api.read_data)(
                    self.0.handle,
                    out[at..].as_mut_ptr().cast(),
                    waiting,
                    &mut read,
                )
            };
            if ok == 0 {
                return Err(failure("reading the body"));
            }
            out.truncate(at + read as usize);
            if read == 0 {
                break;
            }
        }
        Ok(String::from_utf8_lossy(&out).into_owned())
    }
}

impl Connection {
    fn request(&self, target: &Target, method: &'static str) -> Result<Call, NetFault> {
        let verb = wide(method);
        let flags = if target.secure {
            WINHTTP_FLAG_SECURE
        } else {
            0
        };
        let handle = unsafe {
            (self.0.api.open_request)(
                self.0.handle,
                PCWSTR(verb.as_ptr()),
                PCWSTR(target.path.as_ptr()),
                PCWSTR::null(),
                PCWSTR::null(),
                std::ptr::null(),
                flags,
            )
        };
        if handle.is_null() {
            return Err(failure("opening the request"));
        }
        Ok(Call(Owned {
            api: self.0.api,
            handle,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_url_splits_into_the_pieces_winhttp_asks_for_separately() {
        let t = Target::parse("https://loki.example.com/loki/api/v1/query_range").expect("parsed");
        assert_eq!(
            String::from_utf16_lossy(&t.host[..t.host.len() - 1]),
            "loki.example.com"
        );
        assert_eq!(t.port, 443);
        assert!(t.secure);
        assert_eq!(
            String::from_utf16_lossy(&t.path[..t.path.len() - 1]),
            "/loki/api/v1/query_range"
        );
    }

    /// The mount prefix `loki.rs` allows has to survive into the path WinHTTP is given, doubled
    /// segment and all — that doubling is what the owner's own deployment actually needs.
    #[test]
    fn a_mounted_path_reaches_the_transport_whole() {
        let t = Target::parse("https://telemetry.example.com/loki/loki/api/v1/query_range")
            .expect("parsed");
        assert_eq!(
            String::from_utf16_lossy(&t.path[..t.path.len() - 1]),
            "/loki/loki/api/v1/query_range"
        );
    }

    #[test]
    fn a_port_and_a_plain_scheme_are_read_rather_than_assumed() {
        let t = Target::parse("http://127.0.0.1:3100/loki/api/v1/labels").expect("parsed");
        assert_eq!(t.port, 3100);
        assert!(!t.secure);
        let d = Target::parse("http://h/x").expect("parsed");
        assert_eq!(d.port, 80, "http defaults to 80");
    }

    /// `Endpoint::Tail` yields a `wss://` URL, and the transport has to recognise it as secure
    /// rather than refusing a scheme it has not been told about.
    #[test]
    fn the_websocket_schemes_are_understood_as_schemes() {
        assert!(Target::parse("wss://h/x").expect("wss").secure);
        assert!(!Target::parse("ws://h/x").expect("ws").secure);
        assert!(Target::parse("ftp://h/x").is_none(), "and nothing else is");
        assert!(Target::parse("not a url").is_none());
    }

    /// **Every hand-declared Win32 number in this module, against the official bindings.**
    ///
    /// The functions are resolved by name at run time rather than bound, because binding them
    /// would emit a static import and `SPEC.md` §13.2's conditional claim depends on there being
    /// none. That leaves the *constants* written out by hand, and a wrong one is invisible: it
    /// compiles, it runs, and it quietly means something else.
    ///
    /// One of them was wrong. `WINHTTP_CALLBACK_STATUS_CONNECTING_TO_SERVER` was `0x20`, which is
    /// `REQUEST_SENT` — so §7's DNS-rebinding check was handed a byte count instead of an address
    /// and allowed every connection. It was found by a review reading the bindings, not by any
    /// test, because no test in this crate talks to WinHTTP. This one does, without linking
    /// anything: the bindings are a dev-dependency, so they exist for `cargo test` and not for
    /// `cargo build`, and CI's network assertion proves the shipped binary still imports nothing.
    #[test]
    fn the_hand_declared_constants_match_the_official_bindings() {
        use windows::Win32::Networking::WinHttp as w;
        assert_eq!(
            WINHTTP_CALLBACK_STATUS_CONNECTING_TO_SERVER,
            w::WINHTTP_CALLBACK_STATUS_CONNECTING_TO_SERVER,
            "the address notification"
        );
        assert_eq!(
            WINHTTP_CALLBACK_FLAG_ALL_NOTIFICATIONS,
            w::WINHTTP_CALLBACK_FLAG_ALL_NOTIFICATIONS
        );
        assert_eq!(WINHTTP_FLAG_SECURE, w::WINHTTP_FLAG_SECURE.0);
        assert_eq!(
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            w::WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY.0
        );
        assert_eq!(
            WINHTTP_OPTION_DISABLE_FEATURE,
            w::WINHTTP_OPTION_DISABLE_FEATURE
        );
        assert_eq!(WINHTTP_DISABLE_REDIRECTS, w::WINHTTP_DISABLE_REDIRECTS);
        assert_eq!(
            WINHTTP_OPTION_SECURE_PROTOCOLS,
            w::WINHTTP_OPTION_SECURE_PROTOCOLS
        );
        assert_eq!(
            TLS12_AND_13,
            w::WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_2 | w::WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_3,
            "the TLS posture is 1.2 and 1.3 and nothing older"
        );
        assert_eq!(WINHTTP_QUERY_STATUS_CODE, w::WINHTTP_QUERY_STATUS_CODE);
        assert_eq!(WINHTTP_QUERY_FLAG_NUMBER, w::WINHTTP_QUERY_FLAG_NUMBER);
    }

    /// **§7's rebinding check, over the text WinHTTP hands its callback.**
    ///
    /// The callback's only job is to pass this string along; everything that decides lives here,
    /// which is why the decision has tests and the callback does not need one.
    #[test]
    fn the_address_winhttp_reports_is_judged_by_the_same_policy_as_any_other() {
        // Refused under both provenances — these are never a Loki.
        for text in [
            "169.254.169.254",
            "169.254.1.1",
            "0.0.0.0",
            "255.255.255.255",
        ] {
            assert!(
                verdict_of_text(text, Provenance::Typed).is_err(),
                "{text} was allowed"
            );
            assert!(
                verdict_of_text(text, Provenance::Imported).is_err(),
                "{text}"
            );
        }
        // The whole point of doing this at connect time: a name that resolved to loopback.
        assert_eq!(verdict_of_text("127.0.0.1", Provenance::Typed), Ok(()));
        assert!(verdict_of_text("127.0.0.1", Provenance::Imported).is_err());
        // And the long way round, which is what a rebinding attack would try.
        assert!(verdict_of_text("::ffff:169.254.169.254", Provenance::Typed).is_err());
        assert_eq!(verdict_of_text("93.184.216.34", Provenance::Typed), Ok(()));
    }

    /// WinHTTP surrounds the address with nothing in particular, and a notification that is not an
    /// address at all is not evidence of anything — it must not be read as a refusal *or* silently
    /// treated as an approved address.
    #[test]
    fn a_notification_that_is_not_an_address_decides_nothing() {
        assert_eq!(verdict_of_text("  10.1.2.3  ", Provenance::Typed), Ok(()));
        assert_eq!(verdict_of_text("not-an-address", Provenance::Typed), Ok(()));
        assert_eq!(verdict_of_text("", Provenance::Typed), Ok(()));
    }

    /// The watch is per-request and must not leak into the next one: a refusal recorded for one
    /// call would otherwise refuse a later, innocent call on the same thread.
    #[test]
    fn a_watch_is_torn_down_however_the_request_ends() {
        {
            let _watching = Watching::begin(Provenance::Imported);
            WATCH.with(|w| {
                w.borrow_mut().as_mut().unwrap().refused =
                    Some(("127.0.0.1".to_owned(), AddressFault::Loopback));
            });
            assert!(Watching::refusal().is_some());
        }
        assert!(
            Watching::refusal().is_none(),
            "the refusal outlived the request it belonged to"
        );
        assert!(!Watching::saw_an_address());
    }

    /// **The transport must not be in the process merely because this module is compiled in.**
    ///
    /// This is the unit-test half of `SPEC.md` §13.2's conditional claim. The integration test
    /// beside it makes the stronger statement over a whole local-file run; this one catches the
    /// cheapest way to break the promise — a `use` that turns into a static import, or a stray
    /// call at start-up — as soon as anyone builds.
    #[test]
    fn compiling_the_transport_in_does_not_load_it() {
        assert!(
            !transport_is_loaded(),
            "winhttp.dll is in this process and nothing has opened a remote source"
        );
    }
}
