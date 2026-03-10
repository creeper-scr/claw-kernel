const MAX_FRAME_SIZE = 16 * 1024 * 1024; // 16 MiB

/**
 * Encode a JSON-serializable payload as a 4-byte Big Endian framed buffer.
 */
export function encodeFrame(payload: unknown): Buffer {
  const json = JSON.stringify(payload);
  const body = Buffer.from(json, 'utf8');
  const header = Buffer.allocUnsafe(4);
  header.writeUInt32BE(body.length, 0);
  return Buffer.concat([header, body]);
}

/**
 * Frame reader — accumulates bytes from a socket and emits complete frames.
 * The IPC protocol uses 4-byte Big Endian length prefix + JSON payload.
 */
export class FrameReader {
  private buffer: Buffer = Buffer.alloc(0);
  private onFrame: (data: Buffer) => void;

  constructor(onFrame: (data: Buffer) => void) {
    this.onFrame = onFrame;
  }

  feed(chunk: Buffer): void {
    this.buffer = Buffer.concat([this.buffer, chunk]);
    this.processBuffer();
  }

  private processBuffer(): void {
    while (this.buffer.length >= 4) {
      const frameLen = this.buffer.readUInt32BE(0);
      if (frameLen > MAX_FRAME_SIZE) {
        throw new Error(`Frame too large: ${frameLen} bytes`);
      }
      if (this.buffer.length < 4 + frameLen) break;
      const frame = this.buffer.subarray(4, 4 + frameLen);
      this.buffer = this.buffer.subarray(4 + frameLen);
      this.onFrame(frame);
    }
  }
}
