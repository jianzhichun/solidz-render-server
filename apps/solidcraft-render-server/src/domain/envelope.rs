//! Envelope framing (PROTOCOL.md, "Legs and framing"): `u32 LE header_len` + JSON header +
//! payload; on leg A each envelope is preceded by its own `u32 LE` length.

use std::io::{self, Read};

use super::wire::{ErrorCode, Refusal};

/// Largest JSON header accepted.
pub const MAX_HEADER_BYTES: usize = 1 << 20;
/// Largest envelope accepted (one raw RGBA frame at 4096² fits).
pub const MAX_ENVELOPE_BYTES: usize = 64 << 20;

/// Leg A bytes for one envelope: its length, the header's length, the header, the payload.
pub fn leg_a(header: &[u8], payload: &[u8]) -> Vec<u8> {
    let envelope = 4 + header.len() + payload.len();
    let mut out = Vec::with_capacity(4 + envelope);
    out.extend_from_slice(&(envelope as u32).to_le_bytes());
    out.extend_from_slice(&(header.len() as u32).to_le_bytes());
    out.extend_from_slice(header);
    out.extend_from_slice(payload);
    out
}

/// One envelope's header and payload.
#[derive(Debug, PartialEq, Eq)]
pub struct Envelope {
    pub header: Vec<u8>,
    pub payload: Vec<u8>,
}

/// Why reading leg A stopped.
#[derive(Debug)]
pub enum ReadError {
    /// The connection failed or closed in the middle of an envelope.
    Io(io::Error),
    /// The bytes break the framing: answer with this `error`, then close.
    Refused(Refusal),
}

/// Split one envelope (without its leg-A length) into header and payload.
pub fn split(mut envelope: Vec<u8>) -> Result<Envelope, Refusal> {
    let Some(len) = envelope.get(..4) else { return Err(Refusal::new(ErrorCode::Malformed, "envelope shorter than its header length")) };
    let header_len = u32::from_le_bytes([len[0], len[1], len[2], len[3]]) as usize;
    if header_len > MAX_HEADER_BYTES {
        return Err(Refusal::new(ErrorCode::TooLarge, format!("header of {header_len} bytes")));
    }
    if 4 + header_len > envelope.len() {
        return Err(Refusal::new(ErrorCode::Malformed, "header runs past the envelope"));
    }
    let payload = envelope.split_off(4 + header_len);
    envelope.drain(..4);
    Ok(Envelope { header: envelope, payload })
}

/// The next envelope on leg A, or `None` when the peer closed cleanly between envelopes.
pub fn read_leg_a(r: &mut impl Read) -> Result<Option<Envelope>, ReadError> {
    let mut len = [0u8; 4];
    match r.read_exact(&mut len) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(ReadError::Io(e)),
    }
    let len = u32::from_le_bytes(len) as usize;
    if len > MAX_ENVELOPE_BYTES {
        return Err(ReadError::Refused(Refusal::new(ErrorCode::TooLarge, format!("envelope of {len} bytes"))));
    }
    let mut envelope = vec![0u8; len];
    r.read_exact(&mut envelope).map_err(ReadError::Io)?;
    split(envelope).map(Some).map_err(ReadError::Refused)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_envelope_reads_back_as_written() {
        let bytes = [leg_a(br#"{"k":"a"}"#, b"pay"), leg_a(b"{}", b"")].concat();
        let mut r = bytes.as_slice();
        let first = read_leg_a(&mut r).unwrap().unwrap();
        assert_eq!((first.header.as_slice(), first.payload.as_slice()), (br#"{"k":"a"}"#.as_slice(), b"pay".as_slice()));
        assert_eq!(read_leg_a(&mut r).unwrap().unwrap().payload, b"");
        assert!(read_leg_a(&mut r).unwrap().is_none(), "a clean close between envelopes");
    }

    #[test]
    fn oversized_and_inconsistent_lengths_are_refused() {
        let huge = ((MAX_ENVELOPE_BYTES + 1) as u32).to_le_bytes();
        assert!(matches!(read_leg_a(&mut huge.as_slice()), Err(ReadError::Refused(r)) if r.code == ErrorCode::TooLarge));
        let past = [9u32.to_le_bytes().as_slice(), b"{}"].concat();
        assert_eq!(split(past).unwrap_err().code, ErrorCode::Malformed);
        let big_header = ((MAX_HEADER_BYTES + 1) as u32).to_le_bytes().to_vec();
        assert_eq!(split(big_header).unwrap_err().code, ErrorCode::TooLarge);
        // Cut off in the middle of an envelope: a broken connection, not a clean close.
        let cut = &leg_a(b"{}", b"payload")[..8];
        assert!(matches!(read_leg_a(&mut &cut[..]), Err(ReadError::Io(_))));
    }
}
