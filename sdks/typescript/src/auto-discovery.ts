import * as fs from 'fs';
import * as path from 'path';
import * as os from 'os';

/**
 * Discover the claw-kernel socket path.
 * Priority:
 * 1. CLAW_KERNEL_SOCK environment variable
 * 2. ~/Library/Application Support/claw-kernel/claw-kernel.sock (macOS)
 * 3. ~/.local/share/claw-kernel/kernel.sock (Linux / other Unix)
 *
 * Note: the Python SDK uses `kernel.sock` as the filename; this matches that convention.
 */
export function discoverSocketPath(): string {
  // 1. Environment variable
  if (process.env.CLAW_KERNEL_SOCK) {
    return process.env.CLAW_KERNEL_SOCK;
  }

  const home = os.homedir();
  // 2. Platform-specific default
  if (process.platform === 'darwin') {
    return path.join(home, 'Library', 'Application Support', 'claw-kernel', 'kernel.sock');
  } else {
    // Linux / other Unix
    return path.join(home, '.local', 'share', 'claw-kernel', 'kernel.sock');
  }
}

/**
 * Read the auth token from the kernel token file.
 * Token file is written by the daemon at startup (mode 0o600).
 * Returns undefined if the file cannot be read.
 */
export function readAuthToken(): string | undefined {
  try {
    const home = os.homedir();
    let tokenPath: string;

    if (process.env.CLAW_KERNEL_SOCK) {
      // Try to find token file relative to socket directory
      const sockDir = path.dirname(process.env.CLAW_KERNEL_SOCK);
      tokenPath = path.join(sockDir, 'kernel.token');
    } else if (process.platform === 'darwin') {
      tokenPath = path.join(
        home,
        'Library',
        'Application Support',
        'claw-kernel',
        'kernel.token'
      );
    } else {
      tokenPath = path.join(home, '.local', 'share', 'claw-kernel', 'kernel.token');
    }

    return fs.readFileSync(tokenPath, 'utf8').trim();
  } catch {
    return undefined;
  }
}
