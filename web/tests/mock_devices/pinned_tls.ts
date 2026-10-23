/**
 * Certificate pinning the way the Android TV app does it (apps/android-tv/.../PairingManager.kt
 * `pinningTrustManager`): the console serves a self-signed certificate, so the client pins the
 * SHA-256 of the exact certificate it talks to. With nothing pinned yet (first connection by
 * hand-typed IP) it trusts on first use; after that a different certificate is refused.
 *
 * Bun's `fetch`/`WebSocket` ignore `checkServerIdentity` when verification is off, so the pin is
 * enforced by handing the pinned certificate to TLS as the only trusted CA: any other certificate
 * fails the handshake natively. On a failure we look at what the server actually presented to tell
 * "pin mismatch" (what the TV app reports) apart from "unreachable".
 */
import { createHash } from "crypto";
import * as tls from "tls";

export class PinMismatchError extends Error {
  constructor(public expected: string, public actual: string) {
    super(`Server certificate does not match the pinned certificate (expected ${expected}, got ${actual}) -- refusing to connect`);
    this.name = "PinMismatchError";
  }
}

export interface PeerCert { fingerprint: string; pem: string }

/** SHA-256 of the raw DER, lowercase hex: must equal src/network/tls.rs `cert_fingerprint_sha256`. */
export function sha256Hex(der: Buffer): string {
  return createHash("sha256").update(der).digest("hex");
}

export function derToPem(der: Buffer): string {
  return `-----BEGIN CERTIFICATE-----\n${der.toString("base64").match(/.{1,64}/g)!.join("\n")}\n-----END CERTIFICATE-----\n`;
}

/** Reads the certificate a server presents without trusting it (no request is sent). */
export function peekCertificate(host: string, port: number, timeoutMs = 5000): Promise<PeerCert> {
  return new Promise((resolve, reject) => {
    const sock = tls.connect({ host, port, rejectUnauthorized: false }, () => {
      const raw: Buffer = (sock.getPeerCertificate(true) as any).raw;
      sock.end();
      resolve({ fingerprint: sha256Hex(raw), pem: derToPem(raw) });
    });
    sock.setTimeout(timeoutMs, () => { sock.destroy(); reject(new Error("timed out reading the server certificate")); });
    sock.on("error", reject);
  });
}

export class PinnedHttps {
  /** Pinned fingerprint, as the TV app stores it in preferences (`pinned_cert_fingerprint`). */
  constructor(public host: string, public port: number, public pinned: string | null = null) {}

  private pem: string | null = null;

  get baseUrl() { return `https://${this.host}:${this.port}`; }

  /** Pins a fingerprint delivered over a trusted side channel (the ADB launch intent). */
  async pinFromTrustedChannel(fingerprint: string) { this.pinned = fingerprint; this.pem = null; }

  /** Resolves the PEM for the pinned fingerprint, doing trust-on-first-use if nothing is pinned. */
  private async trustAnchor(): Promise<string> {
    const peer = await peekCertificate(this.host, this.port);
    if (this.pinned == null) {
      this.pinned = peer.fingerprint; // trust on first use
    } else if (peer.fingerprint !== this.pinned) {
      // Refuse before any request (and any token in it) is sent.
      throw new PinMismatchError(this.pinned, peer.fingerprint);
    }
    this.pem = peer.pem;
    return peer.pem;
  }

  private async tlsOptions() {
    // Re-verify the presented certificate on every connection, like the TV app's trust manager does.
    const ca = await this.trustAnchor();
    return { ca, checkServerIdentity: () => undefined };
  }

  async fetch(path: string, init: RequestInit = {}): Promise<Response> {
    const tlsOpts = await this.tlsOptions();
    return fetch(`${this.baseUrl}${path}`, { ...init, tls: tlsOpts } as any);
  }

  async json(path: string, init: RequestInit = {}): Promise<any> {
    const r = await this.fetch(path, init);
    const text = await r.text();
    if (!r.ok) throw new Error(`HTTP ${r.status}: ${text}`);
    return JSON.parse(text);
  }

  async webSocket(path = "/ws"): Promise<WebSocket> {
    const tlsOpts = await this.tlsOptions();
    return new WebSocket(`wss://${this.host}:${this.port}${path}`, { tls: tlsOpts } as any);
  }
}
