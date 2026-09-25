/**
 * NTP-style Time Synchronization for Web Clients
 * Calculates the exact offset between the local browser clock and the Rust server clock.
 */
export class TimeSync {
    private offsetMs: number = 0;
    private isSynced: boolean = false;

    constructor() {
        console.log("[TimeSync] Initializing...");
    }

    /**
     * Perform an NTP-style ping sequence to align clocks.
     */
    async synchronize(samples: number = 5) {
        const offsets: number[] = [];

        for (let i = 0; i < samples; i++) {
            const start = performance.now();
            try {
                const res = await fetch('/api/time');
                if (!res.ok) continue;
                const data = await res.json();
                const end = performance.now();
                
                const rtt = end - start;
                // Assume symmetric network delay
                const oneWayLatency = rtt / 2;
                
                // Actual time when the server processed it is approximately data.server_time_ms
                // So local time at that exact moment was (Date.now() - oneWayLatency)
                const localNowAtServerTime = Date.now() - oneWayLatency;
                
                const offset = data.server_time_ms - localNowAtServerTime;
                offsets.push(offset);
            } catch (e) {
                console.warn("[TimeSync] Ping failed", e);
            }
        }

        if (offsets.length > 0) {
            this.offsetMs = computeMedianOffset(offsets);
            this.isSynced = true;
            console.log(`[TimeSync] Synchronized. Offset: ${this.offsetMs}ms. (Server is ${this.offsetMs > 0 ? 'ahead' : 'behind'})`);
        } else {
            console.error("[TimeSync] Failed to synchronize clock with server.");
        }
    }

    private syncInterval: any = null;

    /**
     * Start periodic background clock synchronization (default every 15s).
     */
    startPeriodicSync(intervalMs: number = 15000): void {
        if (this.syncInterval) clearInterval(this.syncInterval);
        this.synchronize().catch(() => {});
        this.syncInterval = setInterval(() => {
            this.synchronize().catch(() => {});
        }, intervalMs);
    }

    /**
     * Stop periodic background clock synchronization.
     */
    stopPeriodicSync(): void {
        if (this.syncInterval) {
            clearInterval(this.syncInterval);
            this.syncInterval = null;
        }
    }

    /**
     * Get the current highly accurate server time based on the local clock + offset.
     */
    getServerTimeMs(): number {
        return Date.now() + this.offsetMs;
    }

    /**
     * Alias for getServerTimeMs() matching standard Clock / NTP interface.
     */
    now(): number {
        return this.getServerTimeMs();
    }

    /**
     * Indicates whether at least one successful NTP clock synchronization has completed.
     */
    public get isSynchronized(): boolean {
        return this.isSynced;
    }
}

export const clock = new TimeSync();

/**
 * Computes the median offset from an array of clock sample measurements,
 * filtering out network latency spikes and outliers.
 */
export function computeMedianOffset(offsets: number[]): number {
    if (!offsets || offsets.length === 0) return 0;
    const sorted = [...offsets].sort((a, b) => a - b);
    const mid = Math.floor(sorted.length / 2);
    return sorted[mid];
}
