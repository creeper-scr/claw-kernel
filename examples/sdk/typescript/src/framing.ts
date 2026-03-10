/**
 * claw-kernel TypeScript SDK — Frame Layer
 *
 * IPC wire format: 4-byte Big Endian length prefix followed by UTF-8 JSON payload.
 * Maximum frame size: 16 MiB (matches Rust daemon limit).
 */

const MAX_FRAME_SIZE = 16 * 1024 * 1024; // 16 MiB

/**
 * Stateful buffer that accumulates raw socket bytes and extracts complete frames.
 * Feed chunks via `push()`, receive zero-or-more complete JSON frames in return.
 */
export class FrameBuffer {
  private buf = Buffer.alloc(0);

  /**
   * Append a raw chunk received from the socket.
   * Returns an array of complete payload Buffers (one per frame extracted).
   */
  push(chunk: Buffer): Buffer[] {
    this.buf = Buffer.concat([this.buf, chunk]);
    const frames: Buffer[] = [];

    while (this.buf.length >= 4) {
      const payloadLen = this.buf.readUInt32BE(0);

      if (payloadLen > MAX_FRAME_SIZE) {
        throw new Error(`Frame too large: ${payloadLen} bytes (max ${MAX_FRAME_SIZE})`);
      }

      if (this.buf.length < 4 + payloadLen) {
        // Not enough data yet — wait for more chunks.
        break;
      }

      frames.push(this.buf.slice(4, 4 + payloadLen));
      this.buf = this.buf.slice(4 + payloadLen);
    }

    return frames;
  }

  /** Number of buffered bytes not yet consumed as a complete frame. */
  get pendingBytes(): number {
    return this.buf.length;
  }
}

/**
 * Encode a JSON value into a length-prefixed frame ready for socket.write().
 */
export function encodeFrame(payload: unknown): Buffer {
  const body = Buffer.from(JSON.stringify(payload), 'utf8');
  const header = Buffer.alloc(4);
  header.writeUInt32BE(body.length, 0);
  return Buffer.concat([header, body]);
}
