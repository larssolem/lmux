//! Length-prefixed JSON framing per ADR-0015.
//!
//! Each frame = `u32-be length` followed by exactly `length` JSON bytes.
//! Frames larger than [`MAX_FRAME_BYTES`] are rejected at parse time.

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::error::{BusError, ErrorCode};

/// Maximum permitted body size for a single frame. ADR-0015 pins 4 MiB.
pub const MAX_FRAME_BYTES: usize = 4 * 1024 * 1024;

/// Read exactly one frame from `reader` and return its raw JSON body.
///
/// Returns [`BusError::FrameTooLarge`] for frames exceeding [`MAX_FRAME_BYTES`],
/// [`BusError::Io`] on read errors (including short reads / EOF mid-frame).
pub async fn read_frame<R>(reader: &mut R) -> Result<Vec<u8>, BusError>
where
    R: AsyncRead + Unpin,
{
    let mut len_buf = [0u8; 4];
    reader
        .read_exact(&mut len_buf)
        .await
        .map_err(BusError::Io)?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > MAX_FRAME_BYTES {
        return Err(BusError::FrameTooLarge {
            code: ErrorCode::FrameTooLarge,
            len,
            max: MAX_FRAME_BYTES,
        });
    }
    let mut body = vec![0u8; len];
    reader.read_exact(&mut body).await.map_err(BusError::Io)?;
    Ok(body)
}

/// Write a frame: 4-byte big-endian length followed by `body`.
///
/// Returns [`BusError::FrameTooLarge`] if `body.len() > MAX_FRAME_BYTES`.
pub async fn write_frame<W>(writer: &mut W, body: &[u8]) -> Result<(), BusError>
where
    W: AsyncWrite + Unpin,
{
    if body.len() > MAX_FRAME_BYTES {
        return Err(BusError::FrameTooLarge {
            code: ErrorCode::FrameTooLarge,
            len: body.len(),
            max: MAX_FRAME_BYTES,
        });
    }
    let len_buf = (body.len() as u32).to_be_bytes();
    writer.write_all(&len_buf).await.map_err(BusError::Io)?;
    writer.write_all(body).await.map_err(BusError::Io)?;
    writer.flush().await.map_err(BusError::Io)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use std::io::Cursor;
    use tokio::io::duplex;

    #[tokio::test]
    async fn roundtrip_small_frame() {
        let (mut a, mut b) = duplex(1024);
        let payload = b"{\"v\":2,\"kind\":\"hello\",\"id\":\"x\"}";
        let writer = tokio::spawn(async move {
            write_frame(&mut a, payload).await.unwrap();
        });
        let got = read_frame(&mut b).await.unwrap();
        writer.await.unwrap();
        assert_eq!(got, payload);
    }

    #[tokio::test]
    async fn rejects_oversize_frame_on_read() {
        let oversized_len = (MAX_FRAME_BYTES as u32 + 1).to_be_bytes();
        let mut cursor = Cursor::new(oversized_len.to_vec());
        let err = read_frame(&mut cursor).await.unwrap_err();
        assert!(matches!(err, BusError::FrameTooLarge { .. }));
    }

    #[tokio::test]
    async fn rejects_oversize_on_write() {
        let big = vec![0u8; MAX_FRAME_BYTES + 1];
        let (mut a, _b) = duplex(8);
        let err = write_frame(&mut a, &big).await.unwrap_err();
        assert!(matches!(err, BusError::FrameTooLarge { .. }));
    }

    #[tokio::test]
    async fn partial_read_is_assembled() {
        // Write a frame in two halves separated by a yield, confirm read_frame reassembles.
        let payload: Vec<u8> = (0..1000u32).map(|i| (i & 0xff) as u8).collect();
        let (mut a, mut b) = duplex(64);
        let body_clone = payload.clone();
        let writer = tokio::spawn(async move {
            let len = (body_clone.len() as u32).to_be_bytes();
            a.write_all(&len).await.unwrap();
            tokio::task::yield_now().await;
            a.write_all(&body_clone[..500]).await.unwrap();
            tokio::task::yield_now().await;
            a.write_all(&body_clone[500..]).await.unwrap();
        });
        let got = read_frame(&mut b).await.unwrap();
        writer.await.unwrap();
        assert_eq!(got, payload);
    }
}
