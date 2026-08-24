//! Stamps the binary with a PE version resource, and hands it to the linker.
//!
//! **Written by hand rather than with a crate or `rc.exe`, and that is a decision.** The obvious
//! routes are `embed-resource` — 23 transitive crates including `syn`, `serde_derive`, `toml` and
//! `cc`, to write about two hundred bytes — or invoking the Windows SDK's `rc.exe`, which then has
//! to be *found*, on a developer machine and on both legs of a CI matrix. A `.res` file is a
//! documented, simple, **architecture-neutral** container, so building one here costs no
//! dependency, no toolchain hunt, and works identically for the x64 and arm64 targets.
//!
//! The risk this trades for is getting `VS_VERSIONINFO`'s byte layout wrong — nested,
//! four-byte-aligned, UTF-16 structures where a miscounted length silently yields a resource
//! Windows declines to parse. That is answered by `version::tests`, which reads the stamp back out
//! of the built executable with `GetFileVersionInfo` — the API the whole exercise exists to serve.
//!
//! **Why the inner loop stays fast.** No `cargo:rerun-if-changed` is emitted, so Cargo falls back
//! to its default: rerun this script only when a file in the package changed. A build that changes
//! nothing does not relink, and a build that changes something gets a new stamp — which is exactly
//! when a new stamp is wanted.

use std::fmt::Write as _;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn main() {
    // Resources are a PE concept. A non-Windows host would be building nothing usable anyway —
    // `SPEC.md` §2.1 scopes this to Windows 10 1809+ — but the guard keeps `cargo check` honest on
    // any machine.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let (year, month, day, seconds_today) = now_utc();
    // §The owner's scheme, shared with their other products: the date in the first three fields and
    // a unique integer in the fourth.
    //
    // **Seconds ÷ 2, because every field is a `u16`.** A version field tops out at 65535 and there
    // are 86400 seconds in a day, so a raw second count overflows for any build after 18:12. Halving
    // it fits the day into 0..=43199 at two-second resolution, which is the same convention
    // MSBuild's own auto-revision uses — so this reads the way the owner's other builds do.
    let revision = seconds_today / 2;
    let version = format!("{year}.{month}.{day}.{revision}");

    // The icon, so Explorer, the taskbar's pinned entry and any installer see it without the
    // program running. **Read without a `cargo:rerun-if-changed`, deliberately** — see the note at
    // the top of this file: declaring any watched file would switch Cargo out of its default
    // "rerun when the package changed" mode and freeze the version stamp. Re-cutting the artwork
    // therefore needs a touched source file to take effect, which `tools/make-icon.ps1` says.
    let ico = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets this"))
        .join("../../assets/tailhawk.ico");
    let icon = std::fs::read(&ico).ok();
    if icon.is_none() {
        println!(
            "cargo:warning=no icon at {} — the binary will carry the version stamp but no icon",
            ico.display()
        );
    }

    let res = Resource::build(year, month, day, revision, &version, icon.as_deref());
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("cargo sets OUT_DIR"));
    let path = out.join("tailhawk.res");
    std::fs::write(&path, res).expect("writing the version resource");

    // MSVC's linker takes a `.res` directly.
    println!("cargo:rustc-link-arg-bins={}", path.display());
    // So the shell can show the same string it stamped, without reading its own file.
    println!("cargo:rustc-env=TAILHAWK_VERSION={version}");
}

/// The UTC date, and how far into the day it is, without pulling in a date library for four
/// numbers. Days-since-epoch to a civil date is Howard Hinnant's `civil_from_days`, which is public
/// domain arithmetic rather than anyone's code.
fn now_utc() -> (u16, u16, u16, u32) {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("the clock is after 1970")
        .as_secs();
    let days = (secs / 86_400) as i64;
    let seconds_today = (secs % 86_400) as u32;

    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };

    (y as u16, m as u16, d as u16, seconds_today)
}

/// A `.res` file under construction.
///
/// The format is a flat sequence of entries, each a header followed by four-byte-aligned data, and
/// the file opens with one empty entry that marks it as 32-bit. Everything here is little-endian,
/// as PE is.
struct Resource(Vec<u8>);

impl Resource {
    const RT_ICON: u16 = 3;
    const RT_VERSION: u16 = 16;
    const RT_GROUP_ICON: u16 = 14;
    const RT_MANIFEST: u16 = 24;
    /// `CREATEPROCESS_MANIFEST_RESOURCE_ID` — the id the loader reads at process start.
    const MANIFEST_ID: u16 = 1;
    /// US English, and the "Unicode" code page 1200 — the pair every version resource is keyed by.
    const LANG_EN_US: u16 = 0x0409;
    const CODEPAGE_UNICODE: u32 = 1200;
    /// The application icon's group. **Shell32 picks the numerically lowest `RT_GROUP_ICON`** as
    /// the executable's icon, so this being 1 is what makes Explorer show the hawk.
    const ICON_GROUP_ID: u16 = 1;

    /// The whole file: the leading empty entry, then the icon, then `VS_VERSIONINFO`.
    fn build(
        year: u16,
        month: u16,
        day: u16,
        revision: u32,
        version: &str,
        ico: Option<&[u8]>,
    ) -> Vec<u8> {
        let mut body = Blob::new();
        body.version_info_root(year, month, day, revision, version);

        let mut file = Resource(Vec::new());
        file.empty_entry();
        if let Some(ico) = ico {
            file.icon_group(ico);
        }
        file.entry(Self::RT_VERSION, 1, &body.0);
        file.entry(Self::RT_MANIFEST, Self::MANIFEST_ID, MANIFEST.as_bytes());
        file.0
    }

    /// An `.ico` file becomes one `RT_ICON` per image plus an `RT_GROUP_ICON` directory naming
    /// them.
    ///
    /// **The directory is not the `.ico`'s own.** They are deliberately different structures: an
    /// `ICONDIRENTRY` ends with a 32-bit *file offset*, a `GRPICONDIRENTRY` ends with a 16-bit
    /// *resource id*, so the entry is fourteen bytes rather than sixteen. Copying the file's
    /// directory across — the obvious shortcut — produces a group Windows reads as garbage.
    fn icon_group(&mut self, ico: &[u8]) {
        const FILE_HEADER: usize = 6;
        const FILE_ENTRY: usize = 16;

        if ico.len() < FILE_HEADER {
            return;
        }
        let count = u16::from_le_bytes([ico[4], ico[5]]) as usize;
        let mut directory = Vec::with_capacity(FILE_HEADER + count * 14);
        directory.extend_from_slice(&0u16.to_le_bytes()); // reserved
        directory.extend_from_slice(&1u16.to_le_bytes()); // type: icon
        directory.extend_from_slice(&(count as u16).to_le_bytes());

        for i in 0..count {
            let at = FILE_HEADER + i * FILE_ENTRY;
            let Some(entry) = ico.get(at..at + FILE_ENTRY) else {
                continue;
            };
            let bytes = u32::from_le_bytes([entry[8], entry[9], entry[10], entry[11]]) as usize;
            let offset = u32::from_le_bytes([entry[12], entry[13], entry[14], entry[15]]) as usize;
            let Some(image) = ico.get(offset..offset + bytes) else {
                continue;
            };

            // Ids start at 1 and are the group's own business; nothing else refers to them.
            let id = i as u16 + 1;
            self.entry(Self::RT_ICON, id, image);

            directory.extend_from_slice(&entry[0..8]); // width, height, colours, reserved, planes, bits
            directory.extend_from_slice(&(bytes as u32).to_le_bytes());
            directory.extend_from_slice(&id.to_le_bytes());
        }
        self.entry(Self::RT_GROUP_ICON, Self::ICON_GROUP_ID, &directory);
    }

    /// The 32-byte null entry every `.res` begins with.
    fn empty_entry(&mut self) {
        self.0.extend_from_slice(&0u32.to_le_bytes()); // data size
        self.0.extend_from_slice(&32u32.to_le_bytes()); // header size
        self.0.extend_from_slice(&[0xFF, 0xFF, 0x00, 0x00]); // type: ordinal 0
        self.0.extend_from_slice(&[0xFF, 0xFF, 0x00, 0x00]); // name: ordinal 0
        self.0.extend_from_slice(&0u32.to_le_bytes()); // data version
        self.0.extend_from_slice(&0u16.to_le_bytes()); // memory flags
        self.0.extend_from_slice(&0u16.to_le_bytes()); // language
        self.0.extend_from_slice(&0u32.to_le_bytes()); // version
        self.0.extend_from_slice(&0u32.to_le_bytes()); // characteristics
    }

    /// One resource: an ordinal type and ordinal name, then its data.
    fn entry(&mut self, kind: u16, id: u16, data: &[u8]) {
        self.0.extend_from_slice(&(data.len() as u32).to_le_bytes());
        self.0.extend_from_slice(&32u32.to_le_bytes());
        self.0.extend_from_slice(&0xFFFFu16.to_le_bytes());
        self.0.extend_from_slice(&kind.to_le_bytes());
        self.0.extend_from_slice(&0xFFFFu16.to_le_bytes());
        self.0.extend_from_slice(&id.to_le_bytes());
        self.0.extend_from_slice(&0u32.to_le_bytes()); // data version
        self.0.extend_from_slice(&0x0030u16.to_le_bytes()); // MOVEABLE | PURE
        self.0.extend_from_slice(&Self::LANG_EN_US.to_le_bytes());
        self.0.extend_from_slice(&0u32.to_le_bytes()); // version
        self.0.extend_from_slice(&0u32.to_le_bytes()); // characteristics
        self.0.extend_from_slice(data);
        while !self.0.len().is_multiple_of(4) {
            self.0.push(0);
        }
    }
}

/// A `VS_VERSIONINFO` node under construction.
///
/// Every node is `wLength`, `wValueLength`, `wType`, a UTF-16 key, padding to four bytes, then a
/// value and children. `wLength` covers the node and everything inside it — which is only knowable
/// once the children are written, so each node is opened, filled, and then patched.
struct Blob(Vec<u8>);

impl Blob {
    fn new() -> Self {
        Blob(Vec::new())
    }

    /// Opens a node, returning the offset of its `wLength` for [`close`](Self::close).
    fn open(&mut self, key: &str, value_len: u16, text: bool) -> usize {
        self.align();
        let at = self.0.len();
        self.0.extend_from_slice(&0u16.to_le_bytes()); // wLength, patched by `close`
        self.0.extend_from_slice(&value_len.to_le_bytes());
        self.0.extend_from_slice(&u16::from(text).to_le_bytes());
        for unit in key.encode_utf16().chain(std::iter::once(0)) {
            self.0.extend_from_slice(&unit.to_le_bytes());
        }
        self.align();
        at
    }

    fn close(&mut self, at: usize) {
        let len = (self.0.len() - at) as u16;
        self.0[at..at + 2].copy_from_slice(&len.to_le_bytes());
    }

    fn align(&mut self) {
        while !self.0.len().is_multiple_of(4) {
            self.0.push(0);
        }
    }

    fn utf16(&mut self, text: &str) {
        for unit in text.encode_utf16().chain(std::iter::once(0)) {
            self.0.extend_from_slice(&unit.to_le_bytes());
        }
    }

    /// A `String` node — a key and its text. `wValueLength` is counted in **UTF-16 code units,
    /// including the terminator**, which is the one field in this format not measured in bytes.
    fn string(&mut self, key: &str, value: &str) {
        let units = value.encode_utf16().count() as u16 + 1;
        let at = self.open(key, units, true);
        self.utf16(value);
        self.close(at);
    }

    /// The root node, its fixed part, and the two children Windows looks for.
    fn version_info_root(&mut self, year: u16, month: u16, day: u16, revision: u32, version: &str) {
        const FIXED_LEN: u16 = 52;
        let root = self.open("VS_VERSION_INFO", FIXED_LEN, false);

        // VS_FIXEDFILEINFO. The version appears twice — as the file's and as the product's —
        // because Explorer reads one and installers read the other.
        let hi = (u32::from(year) << 16) | u32::from(month);
        let lo = (u32::from(day) << 16) | revision;
        self.0.extend_from_slice(&0xFEEF_04BDu32.to_le_bytes()); // signature
        self.0.extend_from_slice(&0x0001_0000u32.to_le_bytes()); // struct version 1.0
        self.0.extend_from_slice(&hi.to_le_bytes()); // file version, high
        self.0.extend_from_slice(&lo.to_le_bytes()); // file version, low
        self.0.extend_from_slice(&hi.to_le_bytes()); // product version, high
        self.0.extend_from_slice(&lo.to_le_bytes()); // product version, low
        self.0.extend_from_slice(&0x3Fu32.to_le_bytes()); // VS_FFI_FILEFLAGSMASK
        self.0.extend_from_slice(&0u32.to_le_bytes()); // no flags: not a debug or prerelease build
        self.0.extend_from_slice(&0x0004_0004u32.to_le_bytes()); // VOS_NT_WINDOWS32
        self.0.extend_from_slice(&0x0000_0001u32.to_le_bytes()); // VFT_APP
        self.0.extend_from_slice(&0u32.to_le_bytes()); // no subtype
        self.0.extend_from_slice(&0u32.to_le_bytes()); // file date, high
        self.0.extend_from_slice(&0u32.to_le_bytes()); // file date, low

        let sfi = self.open("StringFileInfo", 0, true);
        let mut key = String::new();
        write!(
            key,
            "{:04X}{:04X}",
            Resource::LANG_EN_US,
            Resource::CODEPAGE_UNICODE
        )
        .expect("writing to a String cannot fail");
        let table = self.open(&key, 0, true);
        self.string("CompanyName", "Nigel Basel");
        self.string("FileDescription", "Tailhawk — a Windows log viewer");
        self.string("FileVersion", version);
        self.string("InternalName", "tailhawk");
        self.string("LegalCopyright", "MIT OR Apache-2.0");
        self.string("OriginalFilename", "tailhawk.exe");
        self.string("ProductName", "Tailhawk");
        self.string("ProductVersion", version);
        self.close(table);
        self.close(sfi);

        let vfi = self.open("VarFileInfo", 0, true);
        let var = self.open("Translation", 4, false);
        let translation = u32::from(Resource::LANG_EN_US) | (Resource::CODEPAGE_UNICODE << 16);
        self.0.extend_from_slice(&translation.to_le_bytes());
        self.close(var);
        self.close(vfi);

        self.close(root);
    }
}

/// The application manifest, as an `RT_MANIFEST` resource in the same `.res` the version stamp
/// rides in — no separate compilation step, which is what kept a manifest out until now.
///
/// It declares exactly one thing: the comctl32 **v6** side-by-side dependency, without which every
/// native dialog this program shows — `TaskDialogIndirect` refuses outright, `ChooseFontW` and the
/// template dialogs draw Windows-95 controls — falls back to v5. Per-monitor DPI awareness is
/// **deliberately not declared here**: `main` sets it by API before any window exists, that path is
/// proven, and `CLEANROOM.md`'s 2026-08-07 row records the deviation.
const MANIFEST: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <assemblyIdentity version="1.0.0.0" processorArchitecture="*" name="Tailhawk" type="win32"/>
  <dependency>
    <dependentAssembly>
      <assemblyIdentity type="win32" name="Microsoft.Windows.Common-Controls" version="6.0.0.0" processorArchitecture="*" publicKeyToken="6595b64144ccf1df" language="*"/>
    </dependentAssembly>
  </dependency>
</assembly>
"#;
