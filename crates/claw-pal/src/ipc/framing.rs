//! IPC frame encoding and decoding.
//!
//! Wire format: **4-byte Big Endian (BE)** length prefix followed by the payload bytes.
//! This follows the architecture specification requiring Big Endian byte order for
//! all IPC frame length headers to ensure cross-platform consistency.
//!
//! Maximum frame payload size: 16 MiB (0x100_0000 bytes).

use futures::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::error::IpcError;

/// Default maximum allowed payload size (16 MiB).
pub const DEFAULT_MAX_FRAME_SIZE: usize = 0x100_0000;

/// Maximum allowed payload size (configurable).
static mut MAX_FRAME_SIZE: usize = DEFAULT_MAX_FRAME_SIZE;

/// Frame configuration builder.
///
/// Allows customization of IPC framing parameters.
pub struct FrameConfig;

impl FrameConfig {
    /// Get the current maximum frame size.
    pub fn max_frame_size() -> usize {
        unsafe { MAX_FRAME_SIZE }
    }

    /// Set the maximum frame size.
    ///
    /// # Safety
    /// This function is unsafe because it modifies a global static variable.
    /// It should only be called during initialization before any IPC operations.
    pub unsafe fn set_max_frame_size(size: usize) {
        MAX_FRAME_SIZE = size;
    }

    /// Reset to the default maximum frame size (16 MiB).
    ///
    /// # Safety
    /// This function modifies a global static variable and should only be called
    /// during initialization before any IPC operations.
    pub unsafe fn reset() {
        MAX_FRAME_SIZE = DEFAULT_MAX_FRAME_SIZE;
    }
}

/// Write a single frame: 4-byte **Big Endian (BE)** length prefix then payload.
///
/// Returns `IpcError::InvalidMessage` if `data` exceeds the maximum frame size
/// (see [`FrameConfig::max_frame_size()`]).
pub async fn write_frame<W: AsyncWrite + Unpin>(
    writer: &mut W,
    data: &[u8],
) -> Result<(), IpcError> {
    if data.len() > FrameConfig::max_frame_size() {
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

/// Read a single frame: first 4-byte **Big Endian (BE)** length, then payload bytes.
///
/// Returns `IpcError::InvalidMessage` if the declared length exceeds the maximum
/// frame size (see [`FrameConfig::max_frame_size()`]).
/// Returns `IpcError::BrokenPipe` if the underlying reader is closed.
pub async fn read_frame<R: AsyncRead + Unpin>(reader: &mut R) -> Result<Vec<u8>, IpcError> {
    let mut header = [0u8; 4];
    reader
        .read_exact(&mut header)
        .await
        .map_err(|_| IpcError::BrokenPipe)?;
    let len = u32::from_be_bytes(header) as usize;
    if len > FrameConfig::max_frame_size() {
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

    fn max_frame_size() -> usize {
        FrameConfig::max_frame_size()
    }

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
        let too_large = (max_frame_size() as u32 + 1).to_be_bytes();
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
            let got = read_frame(&mut cursor)
                .await
                .expect("frame read should succeed");
            assert_eq!(&got, expected);
        }
    }

    #[tokio::test]
    async fn test_write_frame_exactly_max_size() {
        // A payload of exactly MAX_FRAME_SIZE bytes must succeed.
        let data: Vec<u8> = vec![0xABu8; max_frame_size()];
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
        assert_eq!(got.len(), max_frame_size());
        assert!(got.iter().all(|&b| b == 0xABu8));
    }

    #[tokio::test]
    async fn test_write_frame_over_max_size_fails() {
        let data: Vec<u8> = vec![0u8; max_frame_size() + 1];
        let mut buf = Vec::new();
        let err = write_frame(&mut buf, &data)
            .await
            .expect_err("over MAX_FRAME_SIZE write must fail");
        assert_eq!(err, IpcError::InvalidMessage);
    }

    #[tokio::test]
    async fn test_frame_config_default() {
        assert_eq!(FrameConfig::max_frame_size(), DEFAULT_MAX_FRAME_SIZE);
        assert_eq!(FrameConfig::max_frame_size(), 0x100_0000);
    }
}
