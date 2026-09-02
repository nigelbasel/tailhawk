//! Where the Loki client secret lives — Windows Credential Manager, and nowhere else.
//!
//! **The owner's requirement, 2026-09-02:** *"The secret should be stored in secure storage on the
//! local machine. So there needs to be a user interface that a user can paste the secret into which
//! will store it securely."* This is the storage half; the dialog that pastes into it is
//! `dialog.rs`.
//!
//! **Credential Manager rather than a `CryptProtectData` blob in `%APPDATA%`.** Both encrypt to the
//! signed-in user and neither is readable by another account, so the difference is not
//! cryptographic — it is whether the person whose secret it is can find it again. A DPAPI file is a
//! file they cannot read, cannot audit and cannot revoke except by deleting something unrecognisable
//! from a folder they did not choose. A generic credential appears in Control Panel under a name
//! they will recognise, and deleting it there is a supported gesture with a predictable result.
//!
//! **`tailhawk.settings.toml` never sees a secret.** `SPEC.md` §12.4 describes a file a user is
//! expected to open and edit by hand, and a credential in it would be a credential in every backup,
//! every screen share and every "here is my config" paste. The rest of a source — its name, base
//! URL, token endpoint, client id and scope — is ordinary configuration and stays there in the
//! clear.

use windows::core::PWSTR;
use windows::Win32::Foundation::{ERROR_NOT_FOUND, FILETIME};
use windows::Win32::Security::Credentials::{
    CredDeleteW, CredFree, CredReadW, CredWriteW, CREDENTIALW, CRED_PERSIST_LOCAL_MACHINE,
    CRED_TYPE_GENERIC,
};

/// The prefix every credential Tailhawk stores is filed under.
///
/// Control Panel lists generic credentials by this name, so it is the first thing the owner of the
/// secret reads when they go looking. `Tailhawk/loki/dev` says what it is for, which application put
/// it there, and which of their sources it belongs to.
const PREFIX: &str = "Tailhawk/loki/";

/// The longest a source name may be. `CREDENTIALW::TargetName` is documented as at most 32,767
/// characters, so this is not the platform's limit — it is a limit on how silly a name can get
/// before it stops being a name.
const NAME_MAX: usize = 64;

/// The Credential Manager target name for a source, or `None` if the name cannot be one.
///
/// **The only key the store has**, which is why it is composed here and tested rather than built
/// inline at the call. Two sources that resolved to one target would silently hand the second
/// source the first one's secret — the sort of fault that looks like a server rejecting a
/// credential, and that nobody would think to look for here.
///
/// A name may hold letters, digits, and the three separators a person actually types. It may not
/// hold `/`, which is the separator this scheme uses, nor a backslash, nor control characters, nor
/// leading or trailing space.
pub fn target_for(name: &str) -> Option<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() || trimmed.len() > NAME_MAX || trimmed != name {
        return None;
    }
    let usable = |c: char| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ' ');
    if !name.chars().all(usable) {
        return None;
    }
    Some(format!("{PREFIX}{name}"))
}

/// Stores `secret` against `name`, replacing whatever was there. Reports whether it landed.
///
/// The secret is written as UTF-8 bytes rather than UTF-16: `CredentialBlob` is a byte buffer with
/// a length, the value is a base64 client secret, and round-tripping it through UTF-16 would only
/// add a conversion that could be got wrong in one direction.
pub fn store(name: &str, secret: &str) -> bool {
    let Some(target) = target_for(name) else {
        return false;
    };
    let mut target: Vec<u16> = target.encode_utf16().chain(std::iter::once(0)).collect();
    let mut who: Vec<u16> = "tailhawk"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let mut blob = secret.as_bytes().to_vec();
    let credential = CREDENTIALW {
        Flags: Default::default(),
        Type: CRED_TYPE_GENERIC,
        TargetName: PWSTR(target.as_mut_ptr()),
        Comment: PWSTR::null(),
        LastWritten: FILETIME::default(),
        CredentialBlobSize: blob.len() as u32,
        CredentialBlob: blob.as_mut_ptr(),
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        AttributeCount: 0,
        Attributes: std::ptr::null_mut(),
        TargetAlias: PWSTR::null(),
        UserName: PWSTR(who.as_mut_ptr()),
    };
    unsafe { CredWriteW(&credential, 0) }.is_ok()
}

/// Turns a credential blob into the text it holds, whichever encoding put it there.
///
/// **This exists because the store is deliberately user-visible, and that cuts both ways.** The
/// argument for Credential Manager over a DPAPI file is that the owner of the secret can see, audit
/// and revoke it in Control Panel — so they can also *create* it there, and Control Panel and
/// `cmdkey` write the blob as UTF-16LE where this application writes UTF-8.
///
/// **Reading UTF-16 as UTF-8 does not fail; it succeeds and returns rubbish.** ASCII encoded as
/// UTF-16LE is a run of `byte, 0x00` pairs, and every one of those is valid UTF-8 — so
/// `utf16-secret-123` came back as `"u\0t\0f\01\06\0-\0…"`, a `String` the caller would have sent
/// as the client secret. The server answers 401 and the reader goes to look at the server. That is
/// the failure this function exists to make impossible, and it is why the rule is *interior NUL*
/// rather than *invalid UTF-8*: the bytes are perfectly valid, they are simply not the text.
///
/// A secret is text and text has no interior NUL, so a NUL is the signal. Nothing guesses at
/// encodings beyond these two, because these two are what can actually reach the store.
pub fn decode_blob(bytes: &[u8]) -> Option<String> {
    if let Ok(text) = std::str::from_utf8(bytes) {
        if !text.contains('\0') {
            return Some(text.to_owned());
        }
    }
    if !bytes.len().is_multiple_of(2) {
        return None;
    }
    let wide: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .copied()
        .map(u16::from_le_bytes)
        .collect();
    let text = String::from_utf16(&wide).ok()?;
    // A trailing NUL is how a C string got here; anything else interior means this is not text.
    let text = text.trim_end_matches('\0');
    (!text.contains('\0')).then(|| text.to_owned())
}

/// Reads the secret stored against `name`, or `None` if there is none.
///
/// **The buffer belongs to the store and is freed before returning**, so the secret exists in this
/// process only as the `String` handed back — which the caller is expected to drop as soon as the
/// request carrying it has been sent, exactly as `net.rs` treats a token.
pub fn load(name: &str) -> Option<String> {
    let target = target_for(name)?;
    let target: Vec<u16> = target.encode_utf16().chain(std::iter::once(0)).collect();
    let mut out = std::ptr::null_mut();
    unsafe {
        CredReadW(
            PWSTR(target.as_ptr() as *mut u16),
            CRED_TYPE_GENERIC,
            0,
            &mut out,
        )
    }
    .ok()?;
    if out.is_null() {
        return None;
    }
    let secret = unsafe {
        let credential = &*out;
        let bytes = std::slice::from_raw_parts(
            credential.CredentialBlob,
            credential.CredentialBlobSize as usize,
        );
        decode_blob(bytes)
    };
    unsafe { CredFree(out as *const core::ffi::c_void) };
    secret
}

/// Forgets the secret stored against `name`.
///
/// Reports `true` when there is no longer a credential under that name — including when there was
/// none to begin with, which is the answer a caller asking "make sure this is gone" wants.
pub fn forget(name: &str) -> bool {
    let Some(target) = target_for(name) else {
        return false;
    };
    let target: Vec<u16> = target.encode_utf16().chain(std::iter::once(0)).collect();
    match unsafe { CredDeleteW(PWSTR(target.as_ptr() as *mut u16), CRED_TYPE_GENERIC, 0) } {
        Ok(()) => true,
        Err(e) => e.code() == ERROR_NOT_FOUND.to_hresult(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **A source name is the whole key**, so anything that could make two names collide, escape the
    /// prefix, or reach a part of the store this application does not own is refused before it gets
    /// near `CredWriteW`.
    #[test]
    fn a_name_that_could_collide_or_escape_is_not_a_name() {
        assert_eq!(target_for("dev").as_deref(), Some("Tailhawk/loki/dev"));
        assert_eq!(
            target_for("live eu-west").as_deref(),
            Some("Tailhawk/loki/live eu-west"),
            "a space is something a person types"
        );

        assert_eq!(target_for(""), None, "empty");
        assert_eq!(target_for("   "), None, "space is not a name");
        assert_eq!(target_for(" dev"), None, "leading space");
        assert_eq!(target_for("dev "), None, "trailing space");
        assert_eq!(target_for(&"x".repeat(NAME_MAX + 1)), None, "too long");
        assert_eq!(
            target_for(&"x".repeat(NAME_MAX)).is_some(),
            true,
            "at the limit"
        );

        assert_eq!(target_for("a/b"), None, "the scheme's own separator");
        assert_eq!(target_for("a\\b"), None, "and the other one");
        assert_eq!(
            target_for("a\0b"),
            None,
            "an embedded NUL truncates a C string"
        );
        assert_eq!(target_for("a\nb"), None, "control characters");

        // **`..` is allowed, and that is not an oversight.** A credential target is a flat string
        // key, not a path — there is nothing above `Tailhawk/loki/` to traverse into, so refusing
        // it would be a guard against a threat this store does not have. What the checks above
        // actually defend is collision (`/`), truncation (`\0`) and names that cannot be typed
        // back. A source called `..` is a silly name, not a dangerous one.
        assert_eq!(target_for("..").as_deref(), Some("Tailhawk/loki/.."));
    }

    /// Two names that differ produce two targets. Obvious, and the thing whose failure would hand
    /// one source another source's credential without any error anywhere.
    #[test]
    fn different_names_never_share_a_target() {
        let names = ["dev", "qa", "live", "dev2", "Dev"];
        let mut targets: Vec<String> = names.iter().filter_map(|n| target_for(n)).collect();
        assert_eq!(targets.len(), names.len(), "every one of these is a name");
        targets.sort();
        targets.dedup();
        assert_eq!(targets.len(), names.len(), "and none of them collide");
    }

    /// **The real store, round-tripped.** Credential Manager needs no window and no desktop, so this
    /// is one of the few Win32 surfaces in this project that a test can exercise for real rather
    /// than model. It cleans up after itself whether or not it passes.
    #[test]
    fn a_secret_survives_a_round_trip_through_the_real_store() {
        let name = "tailhawk-selftest";
        let _ = forget(name);
        assert_eq!(load(name), None, "nothing there to begin with");

        assert!(store(name, "s3cret-value=="), "stored");
        assert_eq!(load(name).as_deref(), Some("s3cret-value=="));

        assert!(store(name, "replaced"), "storing again replaces");
        assert_eq!(load(name).as_deref(), Some("replaced"));

        assert!(forget(name), "forgotten");
        assert_eq!(load(name), None, "and it is gone");
        assert!(
            forget(name),
            "forgetting what is not there is not a failure"
        );
    }

    /// **The decode, with no store in it.** The store round-trip below needs Credential Manager;
    /// this needs nothing, so the case that actually bit — UTF-16 read as UTF-8 *succeeding* and
    /// returning rubbish — is pinned where it can never depend on a machine having `cmdkey`.
    #[test]
    fn a_utf16_blob_is_not_read_as_utf8_that_happens_to_parse() {
        assert_eq!(
            decode_blob(b"plain-secret").as_deref(),
            Some("plain-secret")
        );
        assert_eq!(decode_blob(b"").as_deref(), Some(""), "an empty secret");

        // What Control Panel writes: ASCII as UTF-16LE. Every byte pair is valid UTF-8, which is
        // exactly why reading it as UTF-8 succeeded and returned nonsense.
        let utf16: Vec<u8> = "utf16-secret"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        assert!(
            std::str::from_utf8(&utf16).is_ok(),
            "the trap: these bytes ARE valid UTF-8, so validity cannot be the test"
        );
        assert_eq!(decode_blob(&utf16).as_deref(), Some("utf16-secret"));

        // The same, NUL-terminated as a C string reaches the store.
        let mut terminated = utf16.clone();
        terminated.extend_from_slice(&[0, 0]);
        assert_eq!(decode_blob(&terminated).as_deref(), Some("utf16-secret"));

        // Non-ASCII survives both ways round.
        assert_eq!(
            decode_blob("naïve-π".as_bytes()).as_deref(),
            Some("naïve-π")
        );
        let wide: Vec<u8> = "naïve-π"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        assert_eq!(decode_blob(&wide).as_deref(), Some("naïve-π"));

        // Not text at all: refused rather than handed back as a secret.
        assert_eq!(
            decode_blob(&[0xFF, 0xFE, 0x01]),
            None,
            "odd length, not UTF-8"
        );
        assert_eq!(decode_blob(&[0x00, 0x01, 0x02]), None, "an interior NUL");
    }

    /// **A credential the user created themselves must be readable.** The whole argument for
    /// Credential Manager over a DPAPI blob is that the owner of the secret can see it, audit it and
    /// revoke it in Control Panel — and anything they can edit there, they can also *create* there.
    /// Control Panel and `cmdkey` write the blob as UTF-16LE; this application writes UTF-8. A
    /// reader that understood only its own encoding would reject the credential the user had just
    /// typed in, and say nothing more useful than "no secret".
    #[test]
    fn a_credential_written_the_way_control_panel_writes_one_is_readable() {
        let name = "tailhawk-utf16-selftest";
        let target = target_for(name).expect("a name");
        let _ = forget(name);

        // `cmdkey` is Control Panel's own path into the store, and writes UTF-16LE like it does.
        let wrote = std::process::Command::new("cmdkey")
            .arg(format!("/generic:{target}"))
            .arg("/user:tailhawk")
            .arg("/pass:utf16-secret-123")
            .output();
        let Ok(output) = wrote else {
            return; // No `cmdkey` on this machine: nothing to assert against.
        };
        if !output.status.success() {
            return;
        }

        assert_eq!(
            load(name).as_deref(),
            Some("utf16-secret-123"),
            "a credential the user created in Control Panel must read back"
        );
        assert!(forget(name));
    }

    /// A name the store will not take is refused by every entry point, rather than half of them.
    #[test]
    fn an_impossible_name_fails_every_way_in() {
        assert!(!store("a/b", "x"));
        assert_eq!(load("a/b"), None);
        assert!(!forget("a/b"));
    }
}
