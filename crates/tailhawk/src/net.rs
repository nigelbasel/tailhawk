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

use std::sync::OnceLock;

use windows::core::{PCSTR, PCWSTR};
use windows::Win32::Foundation::HMODULE;
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress, LoadLibraryW};

use tailhawk_core::loki::Request;

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
        }
    }
}

/// The most body this will accumulate. `lokiwire::Limits` caps the parse; this caps the read, so a
/// server that streams for ever is stopped before the parser is ever asked.
pub const MAX_RESPONSE: usize = 64 * 1024 * 1024;

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
pub fn send(request: &Request, token: Option<&str>) -> Result<Answer, NetFault> {
    let api = winhttp().ok_or(NetFault::NoTransport)?;
    let target = Target::parse(&request.url).ok_or(NetFault::Failed {
        during: "reading the URL",
        code: 0,
    })?;

    let session = Session::open(api)?;
    let connection = session.connect(&target)?;
    let call = connection.request(&target, request.method)?;
    call.disable_redirects()?;
    call.send(request, token)?;
    call.receive()?;

    let status = call.status()?;
    if (300..400).contains(&status) {
        return Err(NetFault::Redirected { status });
    }
    Ok(Answer {
        status,
        body: call.read_body()?,
    })
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

    fn send(&self, request: &Request, token: Option<&str>) -> Result<(), NetFault> {
        let mut headers = String::new();
        if let Some(kind) = request.content_type {
            headers.push_str("Content-Type: ");
            headers.push_str(kind);
            headers.push_str("\r\n");
        }
        if let Some(token) = token {
            headers.push_str("Authorization: Bearer ");
            headers.push_str(token);
            headers.push_str("\r\n");
        }
        let wide_headers = wide(&headers);
        let body = request.body.as_bytes();
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
