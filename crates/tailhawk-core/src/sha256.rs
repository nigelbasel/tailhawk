//! SHA-256, as FIPS PUB 180-4 defines it.
//!
//! **One function, because one thing needs it.** PKCE's `S256` challenge is the SHA-256 of the
//! code verifier (RFC 7636 §4.2), and this estate's identity client requires `S256` because
//! `AllowPlainTextPkce` is false. Nothing else in Tailhawk hashes anything.
//!
//! **Hand-written rather than taken from Windows CNG**, which was the first design and was the
//! wrong one: `bcrypt.dll` would have bought exactly this hash, at the cost of a DLL dependency and
//! a function no unit test could reach. This crate already hand-writes a streaming JSON reader
//! rather than take a JSON crate, for the same three reasons — the thing is small, the
//! specification is public, and a pure function can be checked against the publication's own
//! vectors.
//!
//! **The constants below are transcribed from the publication, not recalled.** `CLEANROOM.md` §5
//! records two occasions where a constant table written from memory was plausible, confident and
//! wrong; sixty-four round constants would have been a third.

/// FIPS 180-4 §4.2.2: the first thirty-two bits of the fractional parts of the cube roots of the
/// first sixty-four primes.
const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

/// FIPS 180-4 §5.3.3: the first thirty-two bits of the fractional parts of the square roots of the
/// first eight primes.
const H0: [u32; 8] = [
    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
];

/// The SHA-256 digest of `message`.
///
/// **A slice in, an array out, and no streaming interface.** The only input this will ever have is
/// a thirty-two byte verifier; an incremental API would be surface that nothing exercises.
pub fn sha256(message: &[u8]) -> [u8; 32] {
    let mut h = H0;

    // §5.1.1: append a single one bit, then zeros, then the length in bits as a 64-bit big-endian
    // integer, so that the whole is a multiple of 512 bits.
    let mut padded = message.to_vec();
    padded.push(0x80);
    while padded.len() % 64 != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&((message.len() as u64) * 8).to_be_bytes());

    for block in padded.chunks_exact(64) {
        // §6.2.2 step 1: the message schedule.
        let mut w = [0u32; 64];
        for (i, word) in block.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }

        // §6.2.2 steps 2 and 3: the working variables and the sixty-four rounds.
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);

            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }

        // §6.2.2 step 4.
        for (slot, value) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *slot = slot.wrapping_add(value);
        }
    }

    let mut out = [0u8; 32];
    for (chunk, word) in out.chunks_exact_mut(4).zip(h) {
        chunk.copy_from_slice(&word.to_be_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(digest: &[u8; 32]) -> String {
        digest.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// **The publication's own vectors, which is the only reason to trust this at all.** A
    /// hand-written hash that has only been read is a hash nobody has checked, and these three
    /// cover the cases that go wrong: the empty message, a message shorter than one block, and the
    /// fifty-six byte case where the length field pushes the padding into a second block.
    #[test]
    fn the_published_vectors_hold() {
        assert_eq!(
            hex(&sha256(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            hex(&sha256(b"")),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            hex(&sha256(
                b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"
            )),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
    }

    /// **The block boundary, from both sides.** §5.1.1's padding needs nine spare bytes — one for
    /// the terminator and eight for the length — so a message of exactly fifty-six bytes in a
    /// sixty-four byte block is the first that needs a whole extra block. Getting that wrong gives
    /// a digest that is right for every short message and wrong for a long one.
    ///
    /// **These four digests are regression anchors, not citations.** Unlike
    /// [`the_published_vectors_hold`], they were recorded from this implementation *after* it
    /// passed the published vectors — the boundary itself is already covered by FIPS's own
    /// fifty-six byte vector above, and these exist to catch a later change, not to prove the
    /// present one. Said plainly because this file's own note is about constants written from
    /// memory being plausible, confident and wrong.
    #[test]
    fn a_message_that_straddles_the_padding_boundary_is_hashed_whole() {
        // 55, 56, 57 and 64 bytes: the last that fits, the first that does not, and either side.
        let of = |n: usize| hex(&sha256(&vec![b'a'; n]));
        assert_eq!(
            of(55),
            "9f4390f8d30c2dd92ec9f095b65e2b9ae9b0a925a5258e241c9f1e910f734318"
        );
        assert_eq!(
            of(56),
            "b35439a4ac6f0948b6d6f9e3c6af0f5f590ce20f1bde7090ef7970686ec6738a"
        );
        assert_eq!(
            of(57),
            "f13b2d724659eb3bf47f2dd6af1accc87b81f09f59f2b75e5c0bed6589dfe8c6"
        );
        assert_eq!(
            of(64),
            "ffe054fe7ae0cb6dc65c3af9b61d5209f439851db43d0ba5997337df154668eb"
        );
    }

    /// A one-bit change changes the digest everywhere, which is the property the challenge rests on.
    #[test]
    fn a_single_bit_changes_the_whole_digest() {
        let a = sha256(b"verifier");
        let b = sha256(b"verifiet");
        assert_ne!(a, b);
        let shared = a.iter().zip(b.iter()).filter(|(x, y)| x == y).count();
        assert!(
            shared < 8,
            "two digests sharing {shared} of 32 bytes is not an avalanche"
        );
    }
}
