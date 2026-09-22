use serde::{Deserialize, Serialize};

pub const MAX_FRAME_SIZE: usize = 1024 * 1024;
/// Wire contract shared by engine and daemon IPC messages, independent of crate versions.
pub const PROTOCOL_VERSION: u16 = 1;
pub const SUPPORTED_PROTOCOL_VERSIONS: &[u16] = &[PROTOCOL_VERSION];
pub const FRAME_HEADER_SIZE: usize = 6;

#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("frame too large ({0} bytes)")]
    TooLarge(usize),
    #[error("unsupported Mirix protocol version {received}; supported versions: {supported:?}")]
    UnsupportedVersion {
        received: u16,
        supported: &'static [u16],
    },
}

pub(crate) fn validate_version(version: u16) -> Result<(), FrameError> {
    if !SUPPORTED_PROTOCOL_VERSIONS.contains(&version) {
        return Err(FrameError::UnsupportedVersion {
            received: version,
            supported: SUPPORTED_PROTOCOL_VERSIONS,
        });
    }
    Ok(())
}

pub fn encode_frame<T: Serialize>(msg: &T) -> Result<Vec<u8>, FrameError> {
    let body = serde_json::to_vec(msg)?;
    if body.len() > MAX_FRAME_SIZE {
        return Err(FrameError::TooLarge(body.len()));
    }
    let mut out = Vec::with_capacity(FRAME_HEADER_SIZE + body.len());
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&PROTOCOL_VERSION.to_le_bytes());
    out.extend_from_slice(&body);
    Ok(out)
}

/// Decode `[JSON byte length: u32 LE][protocol version: u16 LE][UTF-8 JSON]`.
/// Checks the version before waiting for or deserializing the payload.
pub fn decode_frame<'a, T: Deserialize<'a>>(
    buf: &'a [u8],
) -> Result<Option<(T, usize)>, FrameError> {
    if buf.len() < 4 {
        return Ok(None);
    }
    let len = u32::from_le_bytes(buf[0..4].try_into().unwrap()) as usize;
    if len > MAX_FRAME_SIZE {
        return Err(FrameError::TooLarge(len));
    }
    if buf.len() < FRAME_HEADER_SIZE {
        return Ok(None);
    }
    validate_version(u16::from_le_bytes(buf[4..6].try_into().unwrap()))?;
    if buf.len() < FRAME_HEADER_SIZE + len {
        return Ok(None);
    }
    let msg = serde_json::from_slice(&buf[FRAME_HEADER_SIZE..FRAME_HEADER_SIZE + len])?;
    Ok(Some((msg, FRAME_HEADER_SIZE + len)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::EngineRequest;

    #[test]
    fn v1_wire_fixture_and_fragmented_headers() {
        // A literal fixture fixes the public layout independently of the encoder.
        let fixture = b"\x07\x00\x00\x00\x01\x00\"Hello\"";
        assert_eq!(encode_frame(&EngineRequest::Hello).unwrap(), fixture);
        for end in 0..fixture.len() {
            assert!(
                decode_frame::<EngineRequest>(&fixture[..end])
                    .unwrap()
                    .is_none()
            );
        }
        let (message, consumed) = decode_frame::<EngineRequest>(fixture).unwrap().unwrap();
        assert!(matches!(message, EngineRequest::Hello));
        assert_eq!(consumed, fixture.len());
    }

    #[test]
    fn unsupported_version_is_rejected_without_a_payload() {
        let header = b"\xff\xff\x00\x00\xff\xff";
        assert!(matches!(
            decode_frame::<EngineRequest>(header),
            Err(FrameError::UnsupportedVersion {
                received: 65535,
                supported: &[1]
            })
        ));
    }
}
