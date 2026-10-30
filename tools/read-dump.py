"""Reads a Windows minidump with nothing but the standard library: every thread's registers, the
module each stack word points into, and the ntdll export nearest the first thread's instruction
pointer.

**Why this exists.** The owner's install was found running a day after its window had closed, and
the only way to see where was a dump — `rundll32.exe comsvcs.dll, MiniDump <pid> <path> full` —
with no debugger on the machine and the shipped binary stripped. Module names and offsets were
enough: the stack read `RtlUserThreadStart` → the Rust runtime → `ExitProcess` → process
shutdown's thread-local destructors → tailhawk.exe → d3d11.dll → the graphics driver → an SRW
lock, which is the exit hang `main`'s `ShellDropsBeforeExit` now prevents (2026-10-30).

    python tools/read-dump.py <dump.dmp>

A full dump keeps thread stacks in its Memory64List rather than behind each thread record, which
is why the stack is read from there. The dump a `rundll32` writes may carry an ACL the user cannot
read: `icacls <dump> /grant <user>:(R)` first.
"""
import struct, sys

path = sys.argv[1]
data = open(path, "rb").read()
sig, ver, nstreams, dir_rva = struct.unpack_from("<4sIII", data, 0)
streams = {}
for i in range(nstreams):
    stype, size, rva = struct.unpack_from("<III", data, dir_rva + 12 * i)
    streams.setdefault(stype, []).append((rva, size))


def rd_string(rva):
    (n,) = struct.unpack_from("<I", data, rva)
    return data[rva + 4: rva + 4 + n].decode("utf-16-le", "replace")


mods = []
rva, size = streams[4][0]
(count,) = struct.unpack_from("<I", data, rva)
off = rva + 4
for _ in range(count):
    base, msize, _cs, _ts, name_rva = struct.unpack_from("<QIIII", data, off)
    mods.append((base, msize, rd_string(name_rva).rsplit("\\", 1)[-1]))
    off += 108


def module_of(addr):
    for base, msize, name in mods:
        if base <= addr < base + msize:
            return name, addr - base
    return None


rva, size = streams[3][0]
(count,) = struct.unpack_from("<I", data, rva)
off = rva + 4
tid, susp, pclass, prio, teb, sstart, ssize, srva, csize, crva = struct.unpack_from("<IIIIQQIIII", data, off)
(rsp,) = struct.unpack_from("<Q", data, crva + 0x98)
(rip,) = struct.unpack_from("<Q", data, crva + 0xF8)
(rbp,) = struct.unpack_from("<Q", data, crva + 0xA0)
print("rip=%#x rsp=%#x rbp=%#x stack %#x..%#x (%d bytes)" % (rip, rsp, rbp, sstart, sstart + ssize, ssize))
regs = ["Rax", "Rcx", "Rdx", "Rbx", "Rsp", "Rbp", "Rsi", "Rdi", "R8", "R9", "R10", "R11", "R12", "R13", "R14", "R15"]
for i, r in enumerate(regs):
    (v,) = struct.unpack_from("<Q", data, crva + 0x78 + 8 * i)
    m = module_of(v)
    print("  %-3s = %#018x %s" % (r, v, ("(%s+%#x)" % m) if m else ""))
# A full dump keeps the stack in the Memory64List, not behind the thread record's rva.
mem = []
if 9 in streams:
    mrva, msize_ = streams[9][0]
    mcount, base_rva = struct.unpack_from("<QQ", data, mrva)
    moff = mrva + 16
    file_off = base_rva
    for _ in range(mcount):
        start, msz = struct.unpack_from("<QQ", data, moff)
        mem.append((start, msz, file_off))
        moff += 16
        file_off += msz


def read_mem(addr, n):
    for start, msz, foff in mem:
        if start <= addr < start + msz:
            take = min(n, start + msz - addr)
            return data[foff + (addr - start): foff + (addr - start) + take]
    return b""


stack = read_mem(sstart, ssize)
print("stack bytes recovered from the memory list: %d (thread record rva=%#x)" % (len(stack), srva))
words = struct.unpack_from("<%dQ" % (len(stack) // 8), stack, 0)
print("stack words pointing into a module, or looking like stack addresses:")
for i, w in enumerate(words):
    if w == 0:
        continue
    m = module_of(w)
    if m:
        print("  [%#x] %#018x  <- %s+%#x" % (sstart + 8 * i, w, m[0], m[1]))

# ntdll's exports from the on-disk image (same machine), nearest below the rip.
nt = [m for m in mods if m[2].lower() == "ntdll.dll"][0]
pe = open(r"C:\Windows\System32\ntdll.dll", "rb").read()
(e_lfanew,) = struct.unpack_from("<I", pe, 0x3C)
opt = e_lfanew + 24
(nsections,) = struct.unpack_from("<H", pe, e_lfanew + 6)
(opt_size,) = struct.unpack_from("<H", pe, e_lfanew + 20)
exp_rva, exp_size = struct.unpack_from("<II", pe, opt + 112)
sections = []
sec = opt + opt_size
for i in range(nsections):
    name, vsize, vaddr, rsize, raw = struct.unpack_from("<8sIIII", pe, sec + 40 * i)
    sections.append((vaddr, vsize, raw))


def rva2off(r):
    for vaddr, vsize, raw in sections:
        if vaddr <= r < vaddr + max(vsize, 1):
            return raw + (r - vaddr)
    raise ValueError(hex(r))


e = rva2off(exp_rva)
nfuncs, nnames, funcs_rva, names_rva, ords_rva = struct.unpack_from("<IIIII", pe, e + 20)
names = []
for i in range(nnames):
    (nrva,) = struct.unpack_from("<I", pe, rva2off(names_rva) + 4 * i)
    (ordn,) = struct.unpack_from("<H", pe, rva2off(ords_rva) + 2 * i)
    (frva,) = struct.unpack_from("<I", pe, rva2off(funcs_rva) + 4 * ordn)
    s = pe[rva2off(nrva):]
    names.append((frva, s[:s.index(b"\0")].decode()))
names.sort()
target = rip - nt[0]
below = [n for n in names if n[0] <= target][-4:]
above = [n for n in names if n[0] > target][:2]
print("ntdll exports around rip (+%#x):" % target)
for r, n in below + above:
    print("  %#08x %s%s" % (r, n, "   <-- nearest below" if (r, n) == below[-1] else ""))
