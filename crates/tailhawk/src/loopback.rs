//! Where the browser hands back an authorization code.
//!
//! **Not an HTTP server, and it must not become one.** It binds one socket, accepts one
//! connection, reads only the request line, answers with one fixed page and closes. The whole job
//! is to receive a redirect the browser is already sending; anything more would be surface facing a
//! socket, and this is the only socket in the product that anything outside it could connect to.
//!
//! # Why a listening socket is allowed here
//!
//! `SPEC.md` §13.2 promises **no outbound connection of any kind** unless the user explicitly opens
//! a remote source — and says nothing about a listening one. The privacy concern it exists for is
//! Tailhawk *sending* customer log data somewhere; a socket on `127.0.0.1` that receives one
//! authorization code and sends back a thank-you page does not touch it. CI's ban on `ws2_32` was
//! a stricter proxy of our own making, and `RFC 8252` §7.3 recommends exactly this for native
//! applications.
//!
//! **It is `LoadLibraryW`, not a link, for the same reason `net.rs` is**: a run that never signs in
//! has to leave `ws2_32.dll` out of the process module list, or the conditional claim cannot be
//! tested. A static import would put it in every process whether or not anything called it.
//!
//! # What is deliberately narrow
//!
//! Bound to `127.0.0.1` and never `0.0.0.0`, so nothing off the machine can reach it even while it
//! is open. Bound to port **0**, so the operating system picks a free one rather than this guessing
//! and colliding with whatever already holds it. Opened immediately before the browser and closed
//! the moment the code arrives or the reader gives up, so the only window in which anything listens
//! is the window in which a sign-in is in progress.

use std::sync::OnceLock;

use windows::core::{PCSTR, PCWSTR};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};

const WS2_32_DLL: &str = "ws2_32.dll\0";
const ADVAPI32_DLL: &str = "advapi32.dll\0";

/// The largest request line this will read. A browser's redirect carries the code and the state in
/// a query string; this is generous for that and small enough that nothing can make the read the
/// expensive part.
const MAX_REQUEST: usize = 8 * 1024;

type SocketHandle = usize;
const INVALID_SOCKET: SocketHandle = usize::MAX;

type WsaStartupFn = unsafe extern "system" fn(u16, *mut u8) -> i32;
type SocketFn = unsafe extern "system" fn(i32, i32, i32) -> SocketHandle;
type BindFn = unsafe extern "system" fn(SocketHandle, *const SockAddrIn, i32) -> i32;
type ListenFn = unsafe extern "system" fn(SocketHandle, i32) -> i32;
type AcceptFn = unsafe extern "system" fn(SocketHandle, *mut SockAddrIn, *mut i32) -> SocketHandle;
type RecvFn = unsafe extern "system" fn(SocketHandle, *mut u8, i32, i32) -> i32;
type SendFn = unsafe extern "system" fn(SocketHandle, *const u8, i32, i32) -> i32;
type CloseSocketFn = unsafe extern "system" fn(SocketHandle) -> i32;
type GetSockNameFn = unsafe extern "system" fn(SocketHandle, *mut SockAddrIn, *mut i32) -> i32;
type RtlGenRandomFn = unsafe extern "system" fn(*mut u8, u32) -> u8;

/// `SOCKADDR_IN`, hand-declared like every other signature here.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct SockAddrIn {
    family: u16,
    /// **Network byte order**, which is why every use goes through `to_be`.
    port: u16,
    addr: u32,
    zero: [u8; 8],
}

struct Winsock {
    startup: WsaStartupFn,
    socket: SocketFn,
    bind: BindFn,
    listen: ListenFn,
    accept: AcceptFn,
    recv: RecvFn,
    send: SendFn,
    close: CloseSocketFn,
    sockname: GetSockNameFn,
}

// SAFETY: every field is a function pointer into a module that is never freed, which is the same
// contract `net.rs` relies on for WinHTTP.
unsafe impl Send for Winsock {}
unsafe impl Sync for Winsock {}

static WINSOCK: OnceLock<Option<Winsock>> = OnceLock::new();
static RANDOM: OnceLock<Option<RtlGenRandomFn>> = OnceLock::new();

/// Resolves a symbol whose name already carries its terminator.
fn entry_raw(
    module: windows::Win32::Foundation::HMODULE,
    symbol: &str,
) -> Option<*const core::ffi::c_void> {
    unsafe { GetProcAddress(module, PCSTR(symbol.as_ptr())) }.map(|f| f as *const core::ffi::c_void)
}

fn entry(
    module: windows::Win32::Foundation::HMODULE,
    symbol: &str,
) -> Option<*const core::ffi::c_void> {
    let mut owned = symbol.to_owned();
    owned.push('\0');
    unsafe { GetProcAddress(module, PCSTR(owned.as_ptr())) }.map(|f| f as *const core::ffi::c_void)
}

/// Whether `ws2_32.dll` is in this process yet.
///
/// **This is what makes §13.2's conditional checkable**, exactly as `net::transport_is_loaded` does
/// for the outbound half: the claim is that nothing happens on the network unless the reader opens
/// a remote source, and the only way to show it is that a run which never did leaves the library
/// out of the module list altogether.
pub fn sockets_are_loaded() -> bool {
    let name: Vec<u16> = WS2_32_DLL.encode_utf16().collect();
    unsafe { windows::Win32::System::LibraryLoader::GetModuleHandleW(PCWSTR(name.as_ptr())) }
        .map(|h: windows::Win32::Foundation::HMODULE| !h.is_invalid())
        .unwrap_or(false)
}

fn winsock() -> Option<&'static Winsock> {
    WINSOCK
        .get_or_init(|| {
            let name: Vec<u16> = WS2_32_DLL.encode_utf16().collect();
            let module = unsafe { LoadLibraryW(PCWSTR(name.as_ptr())) }.ok()?;
            let api = Winsock {
                startup: unsafe {
                    std::mem::transmute::<*const core::ffi::c_void, WsaStartupFn>(entry(
                        module,
                        "WSAStartup",
                    )?)
                },
                socket: unsafe {
                    std::mem::transmute::<*const core::ffi::c_void, SocketFn>(entry(
                        module, "socket",
                    )?)
                },
                bind: unsafe {
                    std::mem::transmute::<*const core::ffi::c_void, BindFn>(entry(module, "bind")?)
                },
                listen: unsafe {
                    std::mem::transmute::<*const core::ffi::c_void, ListenFn>(entry(
                        module, "listen",
                    )?)
                },
                accept: unsafe {
                    std::mem::transmute::<*const core::ffi::c_void, AcceptFn>(entry(
                        module, "accept",
                    )?)
                },
                recv: unsafe {
                    std::mem::transmute::<*const core::ffi::c_void, RecvFn>(entry(module, "recv")?)
                },
                send: unsafe {
                    std::mem::transmute::<*const core::ffi::c_void, SendFn>(entry(module, "send")?)
                },
                close: unsafe {
                    std::mem::transmute::<*const core::ffi::c_void, CloseSocketFn>(entry(
                        module,
                        "closesocket",
                    )?)
                },
                sockname: unsafe {
                    std::mem::transmute::<*const core::ffi::c_void, GetSockNameFn>(entry(
                        module,
                        "getsockname",
                    )?)
                },
            };
            // Winsock needs starting once per process before any other call, and 2.2 is what every
            // supported Windows has had for a quarter of a century.
            let mut data = [0u8; 512];
            if unsafe { (api.startup)(0x0202, data.as_mut_ptr()) } != 0 {
                return None;
            }
            Some(api)
        })
        .as_ref()
}

/// Thirty-two unguessable bytes, or `None` if the platform will not give them.
///
/// **`RtlGenRandom` rather than CNG.** The hashing is hand-written, so reaching for `bcrypt.dll`
/// would reintroduce a library for one call; `advapi32` is already in every process.
pub fn random_bytes() -> Option<[u8; 32]> {
    let gen = RANDOM
        .get_or_init(|| {
            let name: Vec<u16> = ADVAPI32_DLL.encode_utf16().collect();
            let module = unsafe { LoadLibraryW(PCWSTR(name.as_ptr())) }.ok()?;
            // Exported under its internal name, which is the documented way to reach it.
            Some(unsafe {
                std::mem::transmute::<*const core::ffi::c_void, RtlGenRandomFn>(entry(
                    module,
                    "SystemFunction036",
                )?)
            })
        })
        .as_ref()?;
    let mut out = [0u8; 32];
    if unsafe { gen(out.as_mut_ptr(), out.len() as u32) } == 0 {
        return None;
    }
    Some(out)
}

/// A socket bound to a free loopback port, waiting for one redirect.
pub struct Redirect {
    /// **Shared, and emptied by whoever closes it first.** `accept` does not wake for a channel, so
    /// cancelling a sign-in means closing the socket underneath it — which makes `accept` fail and
    /// the worker return. Holding the handle behind a lock that is `take`n means the cancel and the
    /// `Drop` cannot both close it, which on a handle the system may already have reused would be
    /// closing somebody else's socket.
    socket: std::sync::Arc<std::sync::Mutex<Option<SocketHandle>>>,
    port: u16,
}

/// Closes a [`Redirect`]'s socket from somewhere else, which is how a waiting `accept` is cancelled.
#[derive(Clone)]
pub struct Closer {
    socket: std::sync::Arc<std::sync::Mutex<Option<SocketHandle>>>,
}

impl Closer {
    /// Closes the socket if it is still open, and does nothing if it is not.
    pub fn close(&self) {
        close_once(&self.socket);
    }
}

fn close_once(shared: &std::sync::Mutex<Option<SocketHandle>>) {
    let taken = shared
        .lock()
        .unwrap_or_else(|held| held.into_inner())
        .take();
    if let (Some(socket), Some(api)) = (taken, winsock()) {
        unsafe { (api.close)(socket) };
    }
}

/// Why a loopback redirect could not be set up or completed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoopbackFault {
    /// Winsock is not there, or would not start.
    NoSockets,
    /// The socket could not be bound or put into listening state.
    Unavailable,
    /// Nothing connected, or the connection carried no request line.
    Nothing,
}

impl Redirect {
    /// Binds a socket to `127.0.0.1` on a port the operating system chooses.
    pub fn open() -> Result<Self, LoopbackFault> {
        let api = winsock().ok_or(LoopbackFault::NoSockets)?;
        // AF_INET, SOCK_STREAM, IPPROTO_TCP.
        let socket = unsafe { (api.socket)(2, 1, 6) };
        if socket == INVALID_SOCKET {
            return Err(LoopbackFault::Unavailable);
        }
        let wanted = SockAddrIn {
            family: 2,
            port: 0,
            // 127.0.0.1 in network byte order. **Not `INADDR_ANY`**: a sign-in must not be
            // answerable from another machine.
            addr: u32::from_be_bytes([127, 0, 0, 1]).to_be(),
            zero: [0; 8],
        };
        let size = std::mem::size_of::<SockAddrIn>() as i32;
        if unsafe { (api.bind)(socket, &wanted, size) } != 0
            || unsafe { (api.listen)(socket, 1) } != 0
        {
            unsafe { (api.close)(socket) };
            return Err(LoopbackFault::Unavailable);
        }
        let mut given = SockAddrIn::default();
        let mut given_size = size;
        if unsafe { (api.sockname)(socket, &mut given, &mut given_size) } != 0 {
            unsafe { (api.close)(socket) };
            return Err(LoopbackFault::Unavailable);
        }
        Ok(Redirect {
            socket: std::sync::Arc::new(std::sync::Mutex::new(Some(socket))),
            port: u16::from_be(given.port),
        })
    }

    /// A handle that closes this socket from elsewhere, cancelling a waiting `accept`.
    pub fn closer(&self) -> Closer {
        Closer {
            socket: self.socket.clone(),
        }
    }

    /// The redirect URI to send the authorization server, naming the port it was given.
    pub fn uri(&self) -> String {
        format!("http://127.0.0.1:{}/callback", self.port)
    }

    /// Waits for the browser's redirect and returns the request target — the path and query the
    /// authorization server sent it to.
    ///
    /// **Blocks, so this belongs on a worker.** `accept` waits as long as it takes somebody to
    /// sign in.
    pub fn wait(&self) -> Result<String, LoopbackFault> {
        let api = winsock().ok_or(LoopbackFault::NoSockets)?;
        let mut from = SockAddrIn::default();
        let mut from_size = std::mem::size_of::<SockAddrIn>() as i32;
        let listening = *self.socket.lock().unwrap_or_else(|held| held.into_inner());
        let listening = listening.ok_or(LoopbackFault::Nothing)?;
        let client = unsafe { (api.accept)(listening, &mut from, &mut from_size) };
        if client == INVALID_SOCKET {
            return Err(LoopbackFault::Nothing);
        }
        let mut buffer = [0u8; MAX_REQUEST];
        let read = unsafe { (api.recv)(client, buffer.as_mut_ptr(), buffer.len() as i32, 0) };
        let target = if read > 0 {
            request_target(&buffer[..read as usize])
        } else {
            None
        };
        // Answered whatever happened, so the reader sees a finished page rather than a browser
        // error, and told to close because nothing here will serve it anything else.
        let page = b"HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nConnection: close\r\nContent-Length: 96\r\n\r\n<!doctype html><title>Signed in</title><p>You are signed in. You can close this tab and go back.";
        let _ = unsafe { (api.send)(client, page.as_ptr(), page.len() as i32, 0) };
        unsafe { (api.close)(client) };
        target.ok_or(LoopbackFault::Nothing)
    }
}

impl Drop for Redirect {
    /// **Closed on the way out, whatever happened.** The window in which anything is listening is
    /// the window in which a sign-in is in progress, and a leaked socket would widen it to the life
    /// of the process.
    fn drop(&mut self) {
        close_once(&self.socket);
    }
}

/// The request target out of an HTTP request line — `GET /callback?code=… HTTP/1.1`.
///
/// **Only the first line is looked at, and only its middle field.** Headers, body and method are
/// not this function's business: the one thing wanted is the query the authorization server
/// redirected to, and parsing more would be inventing an HTTP server.
pub fn request_target(request: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(request);
    let line = text.lines().next()?;
    let mut fields = line.split(' ');
    let _method = fields.next()?;
    let target = fields.next()?;
    (target.starts_with('/')).then(|| target.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **Compiling the socket in does not open one**, which is the behavioural half of the CI
    /// assertion that now admits `ws2_32.dll` to the allow-list. The static scan can only say the
    /// name is in the binary; this says the library is not in a process that has not signed in —
    /// and it would fail the moment anything resolved it eagerly or linked it statically.
    ///
    /// It is the same shape as `net::compiling_the_transport_in_does_not_load_it`, deliberately:
    /// one assertion per library, each one naming the guarantee it protects.
    #[test]
    fn compiling_the_socket_in_does_not_open_one() {
        assert!(
            !sockets_are_loaded(),
            "ws2_32.dll is in this process and nothing has signed in"
        );
    }

    /// **The request line, and nothing else.** A browser sends headers after it and this must not
    /// be confused by them, nor by a request that is not a redirect at all.
    #[test]
    fn the_request_target_is_read_from_the_first_line_only() {
        let request = b"GET /callback?code=abc&state=xyz HTTP/1.1\r\nHost: 127.0.0.1:5173\r\nUser-Agent: whatever\r\n\r\n";
        assert_eq!(
            request_target(request).as_deref(),
            Some("/callback?code=abc&state=xyz")
        );

        // A bare path is still a target; the query is optional as far as this is concerned.
        assert_eq!(
            request_target(b"GET /callback HTTP/1.1\r\n\r\n").as_deref(),
            Some("/callback")
        );

        // Not a request line at all, and a line whose target is not a path: neither is ours.
        assert_eq!(request_target(b""), None);
        assert_eq!(request_target(b"GET\r\n"), None);
        assert_eq!(request_target(b"GET http://elsewhere/ HTTP/1.1\r\n"), None);
        assert_eq!(
            request_target(b"\r\nGET /callback HTTP/1.1\r\n"),
            None,
            "the first line is the request line, and an empty one is not it"
        );
    }

    /// The redirect URI names loopback explicitly and carries the port the system handed out, which
    /// is what the authorization server has to be told.
    #[test]
    fn the_redirect_uri_is_loopback_and_carries_its_port() {
        let pretend = Redirect {
            socket: std::sync::Arc::new(std::sync::Mutex::new(None)),
            port: 51789,
        };
        assert_eq!(pretend.uri(), "http://127.0.0.1:51789/callback");
        assert!(
            !pretend.uri().contains("localhost"),
            "the literal address, not a name a resolver could point elsewhere"
        );
        // `Drop` runs here with an invalid socket, which must not be a problem.
    }
}
