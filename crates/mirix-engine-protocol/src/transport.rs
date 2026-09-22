use crate::{FRAME_HEADER_SIZE, MAX_FRAME_SIZE, decode_frame, encode_frame};
use serde::{Serialize, de::DeserializeOwned};
use std::io::{self, Read, Write};

#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    #[error("I/O: {0}")]
    Io(#[from] io::Error),
    #[error(transparent)]
    Frame(#[from] crate::FrameError),
    #[error("incomplete protocol frame")]
    Closed,
    #[error("protocol message is too large ({0} bytes)")]
    TooLarge(usize),
}

pub fn write_message<W: Write, T: Serialize>(
    writer: &mut W,
    message: &T,
) -> Result<(), TransportError> {
    writer.write_all(&encode_frame(message)?)?;
    writer.flush()?;
    Ok(())
}

pub fn read_message<R: Read, T: DeserializeOwned>(reader: &mut R) -> Result<T, TransportError> {
    let mut header = [0_u8; 4];
    reader.read_exact(&mut header)?;
    let length = u32::from_le_bytes(header) as usize;
    if length > MAX_FRAME_SIZE {
        return Err(TransportError::TooLarge(length));
    }
    let mut version = [0_u8; 2];
    reader.read_exact(&mut version)?;
    crate::frame::validate_version(u16::from_le_bytes(version))?;
    let mut frame = Vec::with_capacity(FRAME_HEADER_SIZE + length);
    frame.extend_from_slice(&header);
    frame.extend_from_slice(&version);
    frame.resize(FRAME_HEADER_SIZE + length, 0);
    reader.read_exact(&mut frame[FRAME_HEADER_SIZE..])?;
    decode_frame(&frame)?
        .map(|(message, _)| message)
        .ok_or(TransportError::Closed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::EngineRequest as Request;

    #[test]
    fn reads_consecutive_frames_without_consuming_the_next_one() {
        let mut bytes = Vec::new();
        write_message(&mut bytes, &Request::Hello).unwrap();
        write_message(&mut bytes, &Request::Reset).unwrap();
        let mut reader = bytes.as_slice();
        assert!(matches!(read_message(&mut reader).unwrap(), Request::Hello));
        assert!(matches!(read_message(&mut reader).unwrap(), Request::Reset));
        assert!(reader.is_empty());
    }

    #[test]
    fn rejects_oversized_header_before_reading_payload() {
        let header = ((MAX_FRAME_SIZE + 1) as u32).to_le_bytes();
        assert!(matches!(
            read_message::<_, Request>(&mut header.as_slice()),
            Err(TransportError::TooLarge(_))
        ));
        assert!(matches!(
            decode_frame::<Request>(&header),
            Err(crate::FrameError::TooLarge(_))
        ));
    }

    #[test]
    fn truncated_payload_returns_unexpected_eof() {
        let mut bytes = encode_frame(&Request::Hello).unwrap();
        bytes.pop();
        assert!(matches!(
            read_message::<_, Request>(&mut bytes.as_slice()),
            Err(TransportError::Io(error)) if error.kind() == io::ErrorKind::UnexpectedEof
        ));
    }

    #[test]
    fn incompatible_peer_is_rejected_before_reading_its_payload() {
        let mut header = &b"\xff\xff\x00\x00\xff\xff"[..];
        assert!(matches!(
            read_message::<_, Request>(&mut header),
            Err(TransportError::Frame(
                crate::FrameError::UnsupportedVersion {
                    received: 65535,
                    ..
                }
            ))
        ));
    }
}
