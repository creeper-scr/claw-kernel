//! IPC frame encoding and decoding.
//!
//! Wire format: 4-byte big-endian length prefix followed by the payload bytes.
//! Maximum frame payload size: 16 MiB (0x100_0000 bytes).

use futures::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::error::IpcError;

/// Maximum allowed payload size (16 MiB).
const MAX_FRAME_SIZE: usize = 0x100_0000;

/// Write a single frame: 4-byte big-endian length prefix then payload.
///
/// Returns `IpcError::InvalidMessage` if `data` exceeds [`MAX_FRAME_SIZE`].
pub async fn write_frame<W: AsyncWrite + Unpin>(
    writer: &mut W,
    data: &[u8],
) -> Result<(), IpcError> {
    if data.len() > MAX_FRAME_SIZE {
        return Err(IpcError::InvalidMessage);
    }
    let len = data.len() as u32;
    let header = len.to_be_bytes();
    writer
        .write_all(&header)
        .await
        .map_err(|_| IpcError::BrokenPipe)?;
    writer
        .write_all(data)
        .await
        .map_err(|_| IpcError::BrokenPipe)?;
    Ok(())
}

/// Read a single frame: first 4-byte big-endian length, then payload bytes.
///
/// Returns `IpcError::InvalidMessage` if the declared length exceeds [`MAX_FRAME_SIZE`].
/// Returns `IpcError::BrokenPipe` if the underlying reader is closed.
pub async fn read_frame<R: AsyncRead + Unpin>(reader: &mut R) -> Result<Vec<u8>, IpcError> {
    let mut header = [0u8; 4];
    reader
        .read_exact(&mut header)
        .await
        .map_err(|_| IpcError::BrokenPipe)?;
    let len = u32::from_be_bytes(header) as usize;
    if len > MAX_FRAME_SIZE {
        return Err(IpcError::InvalidMessage);
    }
    let mut buf = vec![0u8; len];
    reader
        .read_exact(&mut buf)
        .await
        .map_err(|_| IpcError::BrokenPipe)?;
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::io::Cursor;

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    /// Round-trips `data` through write_frame + read_frame using an in-memory buffer.
    async fn roundtrip(data: &[u8]) -> Result<Vec<u8>, IpcError> {
        let mut buf = Vec::new();
        write_frame(&mut buf, data).await?;
        let mut cursor = Cursor::new(buf);
        read_frame(&mut cursor).await
    }

    // -----------------------------------------------------------------------
    // Tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_write_read_frame_roundtrip() {
        let data = b"hello, world!";
        let result = roundtrip(data).await.expect("roundtrip should succeed");
        assert_eq!(result, data);
    }

    #[tokio::test]
    async fn test_write_read_empty_frame() {
        let data: &[u8] = b"";
        let result = roundtrip(data).await.expect("empty frame should succeed");
        assert_eq!(result, data);
    }

    #[tokio::test]
    async fn test_write_read_large_frame() {
        // 1 MiB payload
        let data: Vec<u8> = (0..1024 * 1024).map(|i| (i % 256) as u8).collect();
        let result = roundtrip(&data).await.expect("1 MiB frame should succeed");
        assert_eq!(result, data);
    }

    #[tokio::test]
    async fn test_read_frame_too_large() {
        // Craft a header claiming MAX_FRAME_SIZE + 1 bytes.
        let too_large = (MAX_FRAME_SIZE as u32 + 1).to_be_bytes();
        let mut cursor = Cursor::new(too_large.to_vec());
        let err = read_frame(&mut cursor)
            .await
            .expect_err("oversized frame must fail");
        assert_eq!(err, IpcError::InvalidMessage);
    }

    #[tokio::test]
    async fn test_multiple_frames() {
        let messages: &[&[u8]] = &[b"first", b"second", b"third"];
        let mut buf = Vec::new();
        for msg in messages {
            write_frame(&mut buf, msg).await.unwrap();
        }
        let mut cursor = Cursor::new(buf);
        for expected in messages {
            let got = read_frame(&mut cursor).await.expect("frame read should succeed");
            assert_eq!(&got, expected);
        }
    }

    #[tokio::test]
    async fn test_write_frame_exactly_max_size() {
        // A payload of exactly MAX_FRAME_SIZE bytes must succeed.
        let data: Vec<u8> = vec![0xABu8; MAX_FRAME_SIZE];
        let mut buf = Vec::new();
        // write_frame should not error
        write_frame(&mut buf, &data)
            .await
            .expect("exactly MAX_FRAME_SIZE should succeed");
        // read_frame should also succeed
        let mut cursor = Cursor::new(buf);
        let got = read_frame(&mut cursor)
            .await
            .expect("read back of MAX_FRAME_SIZE should succeed");
        assert_eq!(got.len(), MAX_FRAME_SIZE);
        assert!(got.iter().all(|&b| b == 0xABu8));
    }

    #[tokio::test]
    async fn test_write_frame_over_max_size_fails() {
        let data: Vec<u8> = vec![0u8; MAX_FRAME_SIZE + 1];
        let mut buf = Vec::new();
        let err = write_frame(&mut buf, &data)
            .await
            .expect_err("over MAX_FRAME_SIZE write must fail");
        assert_eq!(err, IpcError::InvalidMessage);
    }
}
