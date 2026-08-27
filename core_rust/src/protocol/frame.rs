use std::io::{self, Read, Write};

use serde::{Serialize, de::DeserializeOwned};
use thiserror::Error;

pub const MAX_CONTROL_FRAME_BYTES: usize = 1024 * 1024;

#[derive(Debug, Error)]
pub enum FrameError {
    #[error("frame I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error("frame length {0} is outside the allowed range")]
    InvalidLength(usize),
    #[error("frame contains invalid JSON: {0}")]
    InvalidJson(#[from] serde_json::Error),
}

pub fn read_json_frame<R, T>(reader: &mut R, max_bytes: usize) -> Result<T, FrameError>
where
    R: Read,
    T: DeserializeOwned,
{
    let mut prefix = [0_u8; 4];
    reader.read_exact(&mut prefix)?;
    let length = u32::from_be_bytes(prefix) as usize;
    if length == 0 || length > max_bytes {
        return Err(FrameError::InvalidLength(length));
    }
    let mut bytes = vec![0_u8; length];
    reader.read_exact(&mut bytes)?;
    Ok(serde_json::from_slice(&bytes)?)
}

pub fn write_json_frame<W, T>(writer: &mut W, value: &T) -> Result<(), FrameError>
where
    W: Write,
    T: Serialize,
{
    write_json_frame_with_limit(writer, value, MAX_CONTROL_FRAME_BYTES)
}

pub fn write_json_frame_with_limit<W, T>(
    writer: &mut W,
    value: &T,
    max_bytes: usize,
) -> Result<(), FrameError>
where
    W: Write,
    T: Serialize,
{
    let bytes = serde_json::to_vec(value)?;
    if bytes.is_empty() || bytes.len() > max_bytes {
        return Err(FrameError::InvalidLength(bytes.len()));
    }
    let length = u32::try_from(bytes.len()).map_err(|_| FrameError::InvalidLength(bytes.len()))?;
    writer.write_all(&length.to_be_bytes())?;
    writer.write_all(&bytes)?;
    writer.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Read};

    use serde::{Deserialize, Serialize};

    use super::*;

    #[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
    struct Sample {
        text: String,
        count: u32,
    }

    struct OneByteReader<R>(R);

    impl<R: Read> Read for OneByteReader<R> {
        fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
            let length = bytes.len().min(1);
            self.0.read(&mut bytes[..length])
        }
    }

    #[test]
    fn frame_round_trips_when_every_read_returns_one_byte() {
        let sample = Sample {
            text: "今晚传照片".to_owned(),
            count: 7,
        };
        let mut encoded = Vec::new();
        write_json_frame(&mut encoded, &sample).unwrap();
        let mut reader = OneByteReader(Cursor::new(encoded));
        let decoded: Sample = read_json_frame(&mut reader, MAX_CONTROL_FRAME_BYTES).unwrap();
        assert_eq!(decoded, sample);
    }

    #[test]
    fn invalid_lengths_are_rejected_before_allocation() {
        let mut zero = Cursor::new(0_u32.to_be_bytes());
        assert!(matches!(
            read_json_frame::<_, Sample>(&mut zero, MAX_CONTROL_FRAME_BYTES),
            Err(FrameError::InvalidLength(0))
        ));

        let too_large = u32::try_from(MAX_CONTROL_FRAME_BYTES + 1).unwrap();
        let mut oversized = Cursor::new(too_large.to_be_bytes());
        assert!(matches!(
            read_json_frame::<_, Sample>(&mut oversized, MAX_CONTROL_FRAME_BYTES),
            Err(FrameError::InvalidLength(_))
        ));
    }

    #[test]
    fn explicit_large_frame_limit_does_not_expand_the_normal_limit() {
        let sample = Sample {
            text: "x".repeat(MAX_CONTROL_FRAME_BYTES),
            count: 1,
        };
        assert!(matches!(
            write_json_frame(&mut Vec::new(), &sample),
            Err(FrameError::InvalidLength(_))
        ));

        let mut encoded = Vec::new();
        write_json_frame_with_limit(&mut encoded, &sample, MAX_CONTROL_FRAME_BYTES * 2).unwrap();
        let decoded: Sample =
            read_json_frame(&mut Cursor::new(encoded), MAX_CONTROL_FRAME_BYTES * 2).unwrap();
        assert_eq!(decoded, sample);
    }

    #[test]
    fn truncated_frame_reports_io_error() {
        let mut bytes = Vec::from(20_u32.to_be_bytes());
        bytes.extend_from_slice(b"{}");
        let mut reader = Cursor::new(bytes);
        assert!(matches!(
            read_json_frame::<_, Sample>(&mut reader, MAX_CONTROL_FRAME_BYTES),
            Err(FrameError::Io(_))
        ));
    }

    #[test]
    fn concatenated_frames_are_decoded_in_order() {
        let first = Sample {
            text: "first".to_owned(),
            count: 1,
        };
        let second = Sample {
            text: "second".to_owned(),
            count: 2,
        };
        let mut encoded = Vec::new();
        write_json_frame(&mut encoded, &first).unwrap();
        write_json_frame(&mut encoded, &second).unwrap();

        let mut reader = Cursor::new(encoded);
        assert_eq!(
            read_json_frame::<_, Sample>(&mut reader, MAX_CONTROL_FRAME_BYTES).unwrap(),
            first
        );
        assert_eq!(
            read_json_frame::<_, Sample>(&mut reader, MAX_CONTROL_FRAME_BYTES).unwrap(),
            second
        );
    }

    #[test]
    fn invalid_utf8_and_malformed_json_are_protocol_errors() {
        let mut invalid_utf8 = Vec::from(2_u32.to_be_bytes());
        invalid_utf8.extend_from_slice(&[0xff, 0xfe]);
        assert!(matches!(
            read_json_frame::<_, Sample>(&mut Cursor::new(invalid_utf8), MAX_CONTROL_FRAME_BYTES),
            Err(FrameError::InvalidJson(_))
        ));

        let malformed = b"{not-json";
        let mut invalid_json = Vec::from((malformed.len() as u32).to_be_bytes());
        invalid_json.extend_from_slice(malformed);
        assert!(matches!(
            read_json_frame::<_, Sample>(&mut Cursor::new(invalid_json), MAX_CONTROL_FRAME_BYTES),
            Err(FrameError::InvalidJson(_))
        ));
    }

    #[test]
    fn unknown_fields_are_ignored_but_required_fields_and_types_are_enforced() {
        let with_unknown = br#"{"text":"ok","count":3,"future_field":true}"#;
        let mut encoded = Vec::from((with_unknown.len() as u32).to_be_bytes());
        encoded.extend_from_slice(with_unknown);
        assert_eq!(
            read_json_frame::<_, Sample>(&mut Cursor::new(encoded), MAX_CONTROL_FRAME_BYTES)
                .unwrap(),
            Sample {
                text: "ok".to_owned(),
                count: 3,
            }
        );

        for invalid in [
            br#"{"text":"missing-count"}"#.as_slice(),
            br#"{"text":7,"count":"wrong"}"#.as_slice(),
        ] {
            let mut encoded = Vec::from((invalid.len() as u32).to_be_bytes());
            encoded.extend_from_slice(invalid);
            assert!(matches!(
                read_json_frame::<_, Sample>(&mut Cursor::new(encoded), MAX_CONTROL_FRAME_BYTES),
                Err(FrameError::InvalidJson(_))
            ));
        }
    }
}
