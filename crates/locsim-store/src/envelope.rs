//! The storage envelope: a short header in front of a payload, so that a
//! damaged file is recognised before anything tries to interpret it.
//!
//! ```text
//! locsim-store 1
//! payload-sha256 <64 lower-case hexadecimal digits>
//! payload-bytes <length of the payload in bytes, decimal>
//!
//! <payload, byte for byte>
//! ```
//!
//! Each header line ends with a single line feed; the fourth line is empty.
//! The payload is not escaped, wrapped or re-encoded: for a scenario it is
//! exactly the text `export_scenario` returns, and removing the first four
//! lines of the file gives that document back. `sha256sum` over the payload
//! prints the digest in the header.
//!
//! # What the digest is for
//!
//! A document codec cannot tell a changed digit from the original: `1.8`
//! and `1.3` are both valid speeds. The digest covers the exact payload
//! bytes, so any accidental change to them is noticed (a random change goes
//! unnoticed with probability about 2^-256). Damage to the header shows up
//! as a wrong first line, an unsupported version, a malformed line, a wrong
//! length or a wrong digest.
//!
//! It is not protection against deliberate change: the digest is unkeyed
//! and whoever can edit the payload can recompute it. It is also unrelated
//! to the scenario fingerprint, which says which scenario a record belongs
//! to, not whether a file is intact.
//!
//! # Versions
//!
//! The number on the first line is the envelope version. A different digest
//! or header layout would be a new envelope version; this build reads
//! version 1 only and refuses the rest. The payload has its own version
//! (`schema_version` or `record_version`), handled by its codec.

use crate::sha256;
use std::fmt;

const MAGIC: &str = "locsim-store";
const DIGEST_KEY: &str = "payload-sha256";
const LENGTH_KEY: &str = "payload-bytes";

/// The envelope version this build reads and writes.
pub const ENVELOPE_VERSION: u32 = 1;

/// Why a file's bytes are not an intact envelope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvelopeError {
    /// The file has no bytes at all.
    Empty,
    /// The first line does not begin with `locsim-store `: this is some
    /// other kind of file (a bare JSON document, for instance).
    NotAnEnvelope,
    /// An envelope of a version this build cannot read.
    UnsupportedVersion { found: u64, supported: u32 },
    /// A header line is missing or not written exactly as specified.
    /// Lines are numbered from 1.
    MalformedHeader { line: u8, reason: &'static str },
    /// The payload is shorter or longer than the header says.
    LengthMismatch { declared: u64, actual: u64 },
    /// The payload is not the one the digest was computed over.
    DigestMismatch { declared: String, actual: String },
}

impl fmt::Display for EnvelopeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EnvelopeError::Empty => write!(f, "the file is empty"),
            EnvelopeError::NotAnEnvelope => {
                write!(f, "not a {MAGIC} file (the first line does not say so)")
            }
            EnvelopeError::UnsupportedVersion { found, supported } => write!(
                f,
                "unsupported {MAGIC} envelope version {found} (this build supports {supported})"
            ),
            EnvelopeError::MalformedHeader { line, reason } => {
                write!(f, "header line {line}: {reason}")
            }
            EnvelopeError::LengthMismatch { declared, actual } => write!(
                f,
                "the header declares a payload of {declared} bytes, the file holds {actual}"
            ),
            EnvelopeError::DigestMismatch { declared, actual } => write!(
                f,
                "the payload is damaged: its SHA-256 is {actual}, the header says {declared}"
            ),
        }
    }
}

impl std::error::Error for EnvelopeError {}

/// The bytes of a file holding `payload`.
pub(crate) fn seal(payload: &[u8]) -> Vec<u8> {
    let header = format!(
        "{MAGIC} {ENVELOPE_VERSION}\n{DIGEST_KEY} {}\n{LENGTH_KEY} {}\n\n",
        sha256::hex(&sha256::digest(payload)),
        payload.len()
    );
    let mut bytes = header.into_bytes();
    bytes.extend_from_slice(payload);
    bytes
}

/// Splits off the next header line, without its line feed.
fn line(bytes: &[u8], number: u8) -> Result<(&[u8], &[u8]), EnvelopeError> {
    match bytes.iter().position(|b| *b == b'\n') {
        Some(end) => Ok((&bytes[..end], &bytes[end + 1..])),
        None => Err(EnvelopeError::MalformedHeader {
            line: number,
            reason: "the file ends inside the header",
        }),
    }
}

/// The value of a `key value` header line, if the line is exactly that.
fn value<'a>(line: &'a [u8], key: &str, number: u8) -> Result<&'a [u8], EnvelopeError> {
    line.strip_prefix(key.as_bytes())
        .and_then(|rest| rest.strip_prefix(b" "))
        .ok_or(EnvelopeError::MalformedHeader {
            line: number,
            reason: "not the expected header line",
        })
}

/// A decimal number in its one canonical spelling.
fn decimal(text: &[u8]) -> Option<u64> {
    let digits = !text.is_empty() && text.iter().all(u8::is_ascii_digit);
    let canonical = digits && (text == b"0" || text[0] != b'0');
    if !canonical {
        return None;
    }
    std::str::from_utf8(text).ok()?.parse().ok()
}

/// Checks the envelope and returns the payload it protects. Nothing of the
/// payload is interpreted here.
pub(crate) fn open(bytes: &[u8]) -> Result<&[u8], EnvelopeError> {
    if bytes.is_empty() {
        return Err(EnvelopeError::Empty);
    }
    // Decided on the leading bytes alone, so that a file of another kind is
    // called that rather than a "malformed header".
    let magic = format!("{MAGIC} ");
    if !bytes.starts_with(magic.as_bytes()) {
        return Err(EnvelopeError::NotAnEnvelope);
    }
    let (first, rest) = line(bytes, 1)?;
    let version = decimal(&first[magic.len()..]).ok_or(EnvelopeError::MalformedHeader {
        line: 1,
        reason: "the envelope version is not a canonical decimal number",
    })?;
    if version != u64::from(ENVELOPE_VERSION) {
        return Err(EnvelopeError::UnsupportedVersion {
            found: version,
            supported: ENVELOPE_VERSION,
        });
    }

    let (second, rest) = line(rest, 2)?;
    let declared_digest = value(second, DIGEST_KEY, 2)?;
    let is_digest = declared_digest.len() == 64
        && declared_digest
            .iter()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b));
    if !is_digest {
        return Err(EnvelopeError::MalformedHeader {
            line: 2,
            reason: "the digest is not 64 lower-case hexadecimal digits",
        });
    }

    let (third, rest) = line(rest, 3)?;
    let declared_length =
        decimal(value(third, LENGTH_KEY, 3)?).ok_or(EnvelopeError::MalformedHeader {
            line: 3,
            reason: "the payload length is not a canonical decimal number",
        })?;

    let (fourth, payload) = line(rest, 4)?;
    if !fourth.is_empty() {
        return Err(EnvelopeError::MalformedHeader {
            line: 4,
            reason: "the line after the header is not empty",
        });
    }

    let actual_length = payload.len() as u64;
    if actual_length != declared_length {
        return Err(EnvelopeError::LengthMismatch {
            declared: declared_length,
            actual: actual_length,
        });
    }
    let actual_digest = sha256::hex(&sha256::digest(payload));
    if actual_digest.as_bytes() != declared_digest {
        return Err(EnvelopeError::DigestMismatch {
            declared: String::from_utf8_lossy(declared_digest).into_owned(),
            actual: actual_digest,
        });
    }
    Ok(payload)
}

#[cfg(test)]
mod tests {
    use super::*;
    use locsim_core::rng::Rng;

    const PAYLOAD: &[u8] = b"{\n  \"schema_version\": 1\n}\n";

    #[test]
    fn a_sealed_file_is_the_header_then_the_payload_unchanged() {
        let sealed = seal(b"abc");
        assert_eq!(
            sealed,
            b"locsim-store 1\n\
              payload-sha256 ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad\n\
              payload-bytes 3\n\
              \n\
              abc"
        );
        assert_eq!(open(&sealed), Ok(&b"abc"[..]));
    }

    #[test]
    fn any_payload_comes_back_byte_for_byte() {
        let mut rng = Rng::from_seed(0xE7);
        let mut payloads: Vec<Vec<u8>> = vec![
            Vec::new(),
            b"\n".to_vec(),
            b"\n\n\n".to_vec(),
            b"locsim-store 1\npayload-bytes 0\n\n".to_vec(),
            vec![0; 70_000],
            (0..=255).collect(),
        ];
        for _ in 0..300 {
            let len = (rng.next_u64() % 400) as usize;
            payloads.push((0..len).map(|_| rng.next_u64() as u8).collect());
        }
        for payload in payloads {
            assert_eq!(open(&seal(&payload)), Ok(payload.as_slice()));
        }
    }

    #[test]
    fn files_of_another_kind_are_called_that() {
        assert_eq!(open(b""), Err(EnvelopeError::Empty));
        for other in [
            &b"{\n  \"schema_version\": 1\n}\n"[..],
            b"\n",
            b" locsim-store 1\n",
            b"Locsim-store 1\n",
            b"locsim-store",
            b"locsim-store\n",
            b"locsim-stor",
            b"\xef\xbb\xbflocsim-store 1\n",
            b"\0\0\0\0",
        ] {
            assert_eq!(open(other), Err(EnvelopeError::NotAnEnvelope), "{other:?}");
        }
    }

    #[test]
    fn other_envelope_versions_are_refused_before_anything_else() {
        for (written, found) in [("0", 0), ("2", 2), ("18446744073709551615", u64::MAX)] {
            let text = String::from_utf8(seal(PAYLOAD)).unwrap().replacen(
                "locsim-store 1\n",
                &format!("locsim-store {written}\n"),
                1,
            );
            assert_eq!(
                open(text.as_bytes()),
                Err(EnvelopeError::UnsupportedVersion {
                    found,
                    supported: 1
                })
            );
        }
        for written in [
            "",
            "01",
            "+1",
            "1 ",
            " 1",
            "1.0",
            "one",
            "18446744073709551616",
        ] {
            let text = String::from_utf8(seal(PAYLOAD)).unwrap().replacen(
                "locsim-store 1\n",
                &format!("locsim-store {written}\n"),
                1,
            );
            assert!(
                matches!(
                    open(text.as_bytes()),
                    Err(EnvelopeError::MalformedHeader { line: 1, .. })
                ),
                "{written:?}"
            );
        }
    }

    #[test]
    fn the_header_has_exactly_one_spelling() {
        let sealed = String::from_utf8(seal(PAYLOAD)).unwrap();
        let digest = sha256::hex(&sha256::digest(PAYLOAD));
        let cases: [(&str, String, u8); 12] = [
            ("payload-sha256 ", "payload-sha256  ".into(), 2),
            ("payload-sha256 ", "payload-sha256\t".into(), 2),
            ("payload-sha256 ", "Payload-SHA256 ".into(), 2),
            ("payload-sha256 ", "payload-sha512 ".into(), 2),
            (&digest, digest.to_uppercase(), 2),
            (&digest, digest[1..].to_string(), 2),
            (&digest, format!("{digest}0"), 2),
            ("payload-bytes 26", "payload-bytes 026".into(), 3),
            ("payload-bytes 26", "payload-bytes +26".into(), 3),
            ("payload-bytes 26", "payload-bytes 26 ".into(), 3),
            ("payload-bytes 26", "payload-length 26".into(), 3),
            ("payload-bytes 26\n\n", "payload-bytes 26\n \n".into(), 4),
        ];
        for (from, to, line) in cases {
            assert_eq!(sealed.matches(from).count(), 1, "{from:?}");
            let damaged = sealed.replacen(from, &to, 1);
            assert!(
                matches!(
                    open(damaged.as_bytes()),
                    Err(EnvelopeError::MalformedHeader { line: l, .. }) if l == line
                ),
                "{to:?}: {:?}",
                open(damaged.as_bytes())
            );
        }
        // A file whose line endings were converted is not the file written.
        let crlf = sealed.replace('\n', "\r\n");
        assert!(matches!(
            open(crlf.as_bytes()),
            Err(EnvelopeError::MalformedHeader { line: 1, .. })
        ));
        // Lines in another order, or one missing.
        let lines: Vec<&str> = sealed.splitn(4, '\n').collect();
        let swapped = format!("{}\n{}\n{}\n{}", lines[0], lines[2], lines[1], lines[3]);
        assert!(matches!(
            open(swapped.as_bytes()),
            Err(EnvelopeError::MalformedHeader { line: 2, .. })
        ));
        let missing = format!("{}\n{}\n{}", lines[0], lines[1], lines[3]);
        assert!(matches!(
            open(missing.as_bytes()),
            Err(EnvelopeError::MalformedHeader { line: 3, .. })
        ));
    }

    #[test]
    fn every_truncation_is_detected() {
        let sealed = seal(PAYLOAD);
        let header = sealed.len() - PAYLOAD.len();
        for end in 0..sealed.len() {
            let result = open(&sealed[..end]);
            let expected_kind = match end {
                0 => matches!(result, Err(EnvelopeError::Empty)),
                e if e < "locsim-store ".len() => {
                    matches!(result, Err(EnvelopeError::NotAnEnvelope))
                }
                e if e < header => matches!(result, Err(EnvelopeError::MalformedHeader { .. })),
                _ => matches!(
                    result,
                    Err(EnvelopeError::LengthMismatch { declared: 26, .. })
                ),
            };
            assert!(expected_kind, "cut at {end}: {result:?}");
        }
    }

    #[test]
    fn trailing_bytes_are_a_length_mismatch() {
        let mut sealed = seal(PAYLOAD);
        sealed.push(b'\n');
        assert_eq!(
            open(&sealed),
            Err(EnvelopeError::LengthMismatch {
                declared: 26,
                actual: 27
            })
        );
    }

    #[test]
    fn every_single_bit_flip_anywhere_in_the_file_is_detected() {
        let sealed = seal(PAYLOAD);
        let header = sealed.len() - PAYLOAD.len();
        let mut in_payload = 0;
        for position in 0..sealed.len() {
            for bit in 0..8 {
                let mut damaged = sealed.clone();
                damaged[position] ^= 1 << bit;
                let result = open(&damaged);
                assert!(result.is_err(), "byte {position} bit {bit} went unnoticed");
                if position >= header {
                    // Damage to the payload is always a digest mismatch.
                    assert!(
                        matches!(result, Err(EnvelopeError::DigestMismatch { .. })),
                        "byte {position} bit {bit}: {result:?}"
                    );
                    in_payload += 1;
                }
            }
        }
        assert_eq!(in_payload, PAYLOAD.len() * 8);
    }

    #[test]
    fn a_digit_changed_into_another_digit_is_a_digest_mismatch() {
        // The case a document codec cannot see.
        let payload = b"{\"max_speed_mps\": 1.8}\n";
        let sealed = String::from_utf8(seal(payload)).unwrap();
        let damaged = sealed.replace("1.8}", "1.3}");
        match open(damaged.as_bytes()) {
            Err(EnvelopeError::DigestMismatch { declared, actual }) => {
                assert_eq!(declared, sha256::hex(&sha256::digest(payload)));
                assert_eq!(
                    actual,
                    sha256::hex(&sha256::digest(b"{\"max_speed_mps\": 1.3}\n"))
                );
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn random_damage_of_any_kind_is_detected() {
        let mut rng = Rng::from_seed(0xE8);
        let sealed = seal(include_bytes!("../../../Examples/Scenarios/walking.json"));
        for _ in 0..20_000 {
            let mut damaged = sealed.clone();
            let position = (rng.next_u64() % damaged.len() as u64) as usize;
            match rng.next_u64() % 4 {
                0 => {
                    damaged.remove(position);
                }
                1 => damaged.insert(position, rng.next_u64() as u8),
                2 => damaged.truncate(position),
                _ => {
                    let other = rng.next_u64() as u8;
                    if other == damaged[position] {
                        continue;
                    }
                    damaged[position] = other;
                }
            }
            assert!(open(&damaged).is_err());
        }
    }
}
