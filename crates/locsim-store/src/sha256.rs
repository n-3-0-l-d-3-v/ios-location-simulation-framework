//! SHA-256 (FIPS 180-4), used as the digest of a stored payload.
//!
//! Written here so that the store has no dependency. It is used to notice
//! accidental damage to a file, not to resist anyone: the digest is unkeyed
//! and stored beside the data it covers. The implementation is checked
//! against the standard's test vectors and against digests computed by
//! other tools.

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

const INITIAL: [u32; 8] = [
    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
];

fn compress(state: &mut [u32; 8], block: &[u8]) {
    let mut w = [0u32; 64];
    for (word, bytes) in w.iter_mut().zip(block.chunks_exact(4)) {
        *word = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    }
    for t in 16..64 {
        let s0 = w[t - 15].rotate_right(7) ^ w[t - 15].rotate_right(18) ^ (w[t - 15] >> 3);
        let s1 = w[t - 2].rotate_right(17) ^ w[t - 2].rotate_right(19) ^ (w[t - 2] >> 10);
        w[t] = w[t - 16]
            .wrapping_add(s0)
            .wrapping_add(w[t - 7])
            .wrapping_add(s1);
    }
    let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = *state;
    for t in 0..64 {
        let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
        let choose = (e & f) ^ (!e & g);
        let t1 = h
            .wrapping_add(s1)
            .wrapping_add(choose)
            .wrapping_add(K[t])
            .wrapping_add(w[t]);
        let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
        let majority = (a & b) ^ (a & c) ^ (b & c);
        let t2 = s0.wrapping_add(majority);
        h = g;
        g = f;
        f = e;
        e = d.wrapping_add(t1);
        d = c;
        c = b;
        b = a;
        a = t1.wrapping_add(t2);
    }
    for (value, add) in state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
        *value = value.wrapping_add(add);
    }
}

/// The SHA-256 digest of `data`.
pub(crate) fn digest(data: &[u8]) -> [u8; 32] {
    let mut state = INITIAL;
    let mut blocks = data.chunks_exact(64);
    for block in &mut blocks {
        compress(&mut state, block);
    }
    // Padding: a single 1 bit, zeros, then the length in bits as 64 bits.
    let rest = blocks.remainder();
    let mut tail = [0u8; 128];
    tail[..rest.len()].copy_from_slice(rest);
    tail[rest.len()] = 0x80;
    let padded = if rest.len() < 56 { 64 } else { 128 };
    let bits = (data.len() as u64).wrapping_mul(8);
    tail[padded - 8..padded].copy_from_slice(&bits.to_be_bytes());
    for block in tail[..padded].chunks_exact(64) {
        compress(&mut state, block);
    }
    let mut out = [0u8; 32];
    for (bytes, word) in out.chunks_exact_mut(4).zip(state) {
        bytes.copy_from_slice(&word.to_be_bytes());
    }
    out
}

/// A digest as 64 lower-case hexadecimal digits.
pub(crate) fn hex(digest: &[u8; 32]) -> String {
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn of(data: &[u8]) -> String {
        hex(&digest(data))
    }

    #[test]
    fn matches_the_fips_180_4_vectors() {
        assert_eq!(
            of(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            of(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            of(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
        assert_eq!(
            of(&vec![b'a'; 1_000_000]),
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
        );
    }

    /// Every length from 0 to 300 bytes, which crosses each padding
    /// boundary several times. The expected value is the SHA-256, computed
    /// with Python's hashlib, of the 301 digests concatenated.
    #[test]
    fn matches_another_implementation_at_every_length_up_to_300() {
        let pattern: Vec<u8> = (0..400u32).map(|i| ((i * 31 + 7) % 251) as u8).collect();
        let mut all = Vec::new();
        for n in 0..=300 {
            all.extend_from_slice(&digest(&pattern[..n]));
        }
        assert_eq!(
            of(&all),
            "3a9fcbf6bfd4421754cdc62d7b0a1a846bcd8fcaae5e64d29ca4bb3a3861610d"
        );
    }

    /// The digest `sha256sum` prints for `Examples/Scenarios/walking.json`.
    #[test]
    fn matches_sha256sum_on_an_example_file() {
        let file = include_bytes!("../../../Examples/Scenarios/walking.json");
        assert_eq!(
            of(file),
            "ffa44c8d998b3245b5a4101fa397a33cc6813ffefc2441852e984141f7d7ed96"
        );
    }

    #[test]
    fn one_changed_bit_changes_the_digest() {
        let file = include_bytes!("../../../Examples/Scenarios/walking.json");
        let reference = digest(file);
        for position in (0..file.len()).step_by(7) {
            for bit in 0..8 {
                let mut damaged = file.to_vec();
                damaged[position] ^= 1 << bit;
                assert_ne!(digest(&damaged), reference, "byte {position} bit {bit}");
            }
        }
    }
}
