/**
 * OpenSanctuary Unified Engine WebSocket Client
 *
 * Centralized, resilient real-time client providing:
 * - Automatic protocol selection (ws: / wss:) and dynamic host resolution
 * - Exponential backoff reconnection with random jitter
 * - Mobile/browser lifecycle integration (online/offline & visibilitychange)
 * - Transparent schedule retention when backend sends null schedule deltas
 * - Optional schema protocol validation
 * - WebSocket send with automatic HTTP /api/command fallback
 */

import { validateSnapshotProtocol } from './protocol.ts';

export type WsConnectionStatus = 'connecting' | 'connected' | 'disconnected';

export interface WsClientOptions {
  url?: string;
  autoReconnect?: boolean;
  initialReconnectDelayMs?: number;
  maxReconnectDelayMs?: number;
  validateProtocol?: boolean;
  /**
   * A paired device token (from /api/pairing/remote-session, scanned via QR
   * in the console's Pairing menu). When set, every command this client
   * sends is authenticated against it, server-side — see `sendCommand`.
   * Leave unset for trusted, unauthenticated callers (the operator console).
   */
  deviceToken?: string;
  onOpen?: () => void;
  onClose?: (event?: any) => void;
  onError?: (err?: any) => void;
  onStatusChange?: (status: WsConnectionStatus) => void;
  onSnapshot?: (snapshot: any) => void;
  onProtocolMismatch?: (errorMessage: string) => void;
}

export class EngineWebSocketClient {
  private ws: WebSocket | null = null;
  private currentSnapshot: any = null;
  private status: WsConnectionStatus = 'disconnected';
  private reconnectTimer: any = null;
  private reconnectDelay: number;
  private isExplicitlyClosed: boolean = false;
  private boundOnlineListener: (() => void) | null = null;
  private boundVisibilityListener: (() => void) | null = null;
  private options: WsClientOptions;

  constructor(options: WsClientOptions = {}) {
    this.options = {
      autoReconnect: true,
      initialReconnectDelayMs: 1000,
      maxReconnectDelayMs: 30000,
      validateProtocol: false,
      ...options,
    };
    this.reconnectDelay = this.options.initialReconnectDelayMs || 1000;
    this.initLifecycleListeners();
  }

  /**
   * Resolves the target WebSocket URL based on current page location if not explicitly provided.
   */
  public resolveUrl(): string {
    if (this.options.url) return this.options.url;
    if (typeof location !== 'undefined' && location.host) {
      const protocol = location.protocol === 'https:' ? 'wss:' : 'ws:';
      return `${protocol}//${location.host}/ws`;
    }
    return 'ws://localhost:8080/ws';
  }

  /**
   * Connects to the OpenSanctuary Rust WebSocket server.
   */
  public connect(): void {
    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }

    if (this.ws && (this.ws.readyState === WebSocket.CONNECTING || this.ws.readyState === WebSocket.OPEN)) {
      return;
    }

    this.isExplicitlyClosed = false;
    this.setStatus('connecting');

    try {
      const url = this.resolveUrl();
      this.ws = new WebSocket(url);

      this.ws.onopen = () => {
        this.setStatus('connected');
        this.reconnectDelay = this.options.initialReconnectDelayMs || 1000;
        if (this.options.onOpen) {
          this.options.onOpen();
        }
      };

      this.ws.onmessage = (event: MessageEvent) => {
        this.handleMessage(event);
      };

      this.ws.onerror = (err: Event) => {
        if (this.options.onError) {
          this.options.onError(err);
        }
        try {
          if (this.ws) this.ws.close();
        } catch (_) {}
      };

      this.ws.onclose = (event: CloseEvent) => {
        this.setStatus('disconnected');
        if (this.options.onClose) {
          this.options.onClose(event);
        }
        if (!this.isExplicitlyClosed && this.options.autoReconnect) {
          this.scheduleReconnect();
        }
      };
    } catch (err) {
      this.setStatus('disconnected');
      if (this.options.onError) {
        this.options.onError(err);
      }
      if (!this.isExplicitlyClosed && this.options.autoReconnect) {
        this.scheduleReconnect();
      }
    }
  }

  /**
   * Disconnects the WebSocket cleanly and stops reconnection timers.
   */
  public disconnect(): void {
    this.isExplicitlyClosed = true;
    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }
    if (this.ws) {
      try {
        this.ws.close();
      } catch (_) {}
      this.ws = null;
    }
    this.setStatus('disconnected');
  }

  /**
   * Handles incoming message parsing, protocol validation, and schedule delta retention.
   */
  private handleMessage(event: MessageEvent): void {
    try {
      const snapshot = typeof event.data === 'string' ? JSON.parse(event.data) : event.data;
      if (!snapshot) return;

      if (this.options.validateProtocol) {
        try {
          const validation = validateSnapshotProtocol(snapshot);
          if (!validation.valid && this.options.onProtocolMismatch) {
            this.options.onProtocolMismatch(validation.error || 'Protocol version mismatch');
          }
        } catch (_) {}
      }

      // Backend omits schedule (sends null) on live/staged updates — retain cached schedule
      if (snapshot.schedule === null && this.currentSnapshot && this.currentSnapshot.schedule) {
        snapshot.schedule = this.currentSnapshot.schedule;
      }

      this.currentSnapshot = snapshot;

      if (this.options.onSnapshot) {
        this.options.onSnapshot(snapshot);
      }
    } catch (e) {
      console.error('[WebSocket] Snapshot parse error:', e);
    }
  }

  /**
   * Sends an engine command over WebSocket if open; falls back to HTTP POST /api/command.
   */
  public async sendCommand(cmd: any): Promise<any> {
    const deviceToken = this.options.deviceToken;
    const text = typeof cmd === 'string' && (cmd.startsWith('{') || cmd.startsWith('"') || cmd.startsWith('['))
      ? cmd
      : JSON.stringify(cmd);

    if (this.ws && this.ws.readyState === WebSocket.OPEN) {
      try {
        // A raw WS text frame has no header to carry a credential in, so a
        // paired remote client wraps the command in an envelope the server
        // unwraps after checking device_token against paired_devices.
        // Unauthenticated callers (the operator console) send `text` as-is.
        const wireText = deviceToken
          ? JSON.stringify({ cmd, device_token: deviceToken })
          : text;
        this.ws.send(wireText);
        return { success: true, via: 'websocket' };
      } catch (err) {
        console.warn('[WebSocket] Send failed, attempting HTTP fallback:', err);
      }
    }

    // Fallback to HTTP POST /api/command
    try {
      const res = await fetch('/api/command', {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          ...(deviceToken ? { 'x-device-token': deviceToken } : {}),
        },
        body: text,
      });
      if (res.ok) {
        const data = await res.json().catch(() => ({}));
        return { success: true, ...data, via: 'http' };
      }
    } catch (httpErr) {
      console.warn('[WebSocket] HTTP fallback failed:', httpErr);
    }

    return { success: false, via: 'failed' };
  }

  /**
   * Sends raw string or object directly without fallback.
   */
  public sendRaw(data: any): boolean {
    if (this.ws && this.ws.readyState === WebSocket.OPEN) {
      try {
        const text = typeof data === 'string' ? data : JSON.stringify(data);
        this.ws.send(text);
        return true;
      } catch (_) {
        return false;
      }
    }
    return false;
  }

  /**
   * Calculates exponential backoff with jitter and schedules the next connection attempt.
   */
  private scheduleReconnect(): void {
    if (this.reconnectTimer || this.isExplicitlyClosed) return;

    const jitter = Math.random() * 500;
    const maxDelay = this.options.maxReconnectDelayMs || 30000;
    const delay = Math.min(this.reconnectDelay + jitter, maxDelay);
    this.reconnectDelay = Math.min(this.reconnectDelay * 2, maxDelay);

    this.reconnectTimer = setTimeout(() => {
      this.reconnectTimer = null;
      this.connect();
    }, delay);
  }

  private setStatus(newStatus: WsConnectionStatus): void {
    if (this.status !== newStatus) {
      this.status = newStatus;
      if (this.options.onStatusChange) {
        this.options.onStatusChange(newStatus);
      }
    }
  }

  public getStatus(): WsConnectionStatus {
    return this.status;
  }

  public isConnected(): boolean {
    return this.status === 'connected' && !!this.ws && this.ws.readyState === WebSocket.OPEN;
  }

  public getSnapshot(): any {
    return this.currentSnapshot;
  }

  public getRawSocket(): WebSocket | null {
    return this.ws;
  }

  /**
   * Attach mobile wake / network online listeners to automatically reconnect immediately.
   */
  private initLifecycleListeners(): void {
    if (typeof window !== 'undefined') {
      this.boundOnlineListener = () => {
        if (!this.isConnected() && !this.isExplicitlyClosed) {
          this.reconnectDelay = this.options.initialReconnectDelayMs || 1000;
          this.connect();
        }
      };
      window.addEventListener('online', this.boundOnlineListener);
    }

    if (typeof document !== 'undefined') {
      this.boundVisibilityListener = () => {
        if (document.visibilityState === 'visible' && !this.isConnected() && !this.isExplicitlyClosed) {
          this.reconnectDelay = this.options.initialReconnectDelayMs || 1000;
          this.connect();
        }
      };
      document.addEventListener('visibilitychange', this.boundVisibilityListener);
    }
  }

  /**
   * Clean up lifecycle listeners on dispose.
   */
  public destroy(): void {
    this.disconnect();
    if (typeof window !== 'undefined' && this.boundOnlineListener) {
      window.removeEventListener('online', this.boundOnlineListener);
    }
    if (typeof document !== 'undefined' && this.boundVisibilityListener) {
      document.removeEventListener('visibilitychange', this.boundVisibilityListener);
    }
  }
}

/**
 * Factory helper to construct and initialize an EngineWebSocketClient.
 */
export function createEngineWebSocket(options: WsClientOptions = {}): EngineWebSocketClient {
  const client = new EngineWebSocketClient(options);
  client.connect();
  return client;
}
