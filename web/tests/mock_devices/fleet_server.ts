/**
 * Control server for the mock-device fleet, for a local agent (e.g. the Qwen soak) to drive devices that
 * normally need a TV or a phone. Plain JSON over HTTP on 127.0.0.1:
 *
 *   GET  /state                                  every TV and phone, and what the console says is live
 *   POST /tv/add        {name}
 *   POST /tv/pair       {name, pinned?: bool}     -> the TV's screen after pairing by QR (pinned: it was installed over ADB)
 *   POST /tv/screen     {name}                    -> {kind: pairing-qr|wrong-console|connection-failed|live}
 *   POST /tv/live       {name}                    open the display socket
 *   POST /tv/close      {name}
 *   POST /tv/reboot     {name}                    same preferences, new process
 *   POST /tv/reset      {name}                    Settings > Reset Pairing
 *   POST /tv/remove     {name}                    the operator removes it in Settings > Paired Devices
 *   POST /tv/server     {name, ip, port}          operator retypes the console address
 *   POST /remote/add    {name}                    mint a remote and connect it
 *   POST /remote/send   {name, cmd}               a ShowCommand, e.g. "NextSlide" or {"GoLive":{...}}
 *   POST /revoke        {device_id}
 *   POST /check         {settle_ms?}              do all connected devices show what the console says?
 *
 * Usage: bun tests/mock_devices/fleet_server.ts --console http://127.0.0.1:8080 --https-port 8443 [--listen 9500]
 */
import { Fleet } from "./fleet.ts";

function arg(name: string, fallback?: string) {
  const i = process.argv.indexOf(`--${name}`);
  return i >= 0 ? process.argv[i + 1] : fallback;
}

export function serveFleet(fleet: Fleet, port: number) {
  return Bun.serve({
    port, hostname: "127.0.0.1",
    async fetch(req) {
      const url = new URL(req.url);
      try {
        if (req.method === "GET" && url.pathname === "/state") {
          return Response.json({
            tvs: [...fleet.tvs].map(([name, t]) => ({ name, device_id: t.deviceId, paired: !!t.deviceToken, pinned: t.pinned, live_open: t.liveOpen, last_sequence: t.lastSequence })),
            phones: [...fleet.phones].map(([name, p]) => ({ name, device_id: p.deviceId, connected: p.ws?.readyState === WebSocket.OPEN, last_sequence: p.lastSnapshot?.sequence_number ?? null, rejected: p.rejected.length })),
            console: (await fleet.check(0)).console,
          });
        }
        const b: any = req.method === "POST" ? await req.json().catch(() => ({})) : {};
        const routes: Record<string, () => unknown> = {
          "/tv/add": () => fleet.addTv(b.name),
          "/tv/pair": () => fleet.pairTv(b.name, b.pinned ?? true),
          "/tv/screen": () => fleet.tvScreen(b.name),
          "/tv/live": () => fleet.tvOpenLive(b.name),
          "/tv/close": () => fleet.tvClose(b.name),
          "/tv/reboot": () => fleet.tvReboot(b.name),
          "/tv/reset": () => fleet.tvResetPairing(b.name),
          "/tv/remove": () => fleet.removeTv(b.name),
          "/tv/server": () => fleet.tvSetServer(b.name, b.ip, Number(b.port)),
          "/remote/add": () => fleet.addRemote(b.name),
          "/remote/send": () => fleet.phoneSend(b.name, b.cmd),
          "/revoke": () => fleet.revoke(b.device_id),
          "/check": () => fleet.check(b.settle_ms ?? 1500),
        };
        const handler = routes[url.pathname];
        if (!handler) return Response.json({ error: `unknown route ${url.pathname}` }, { status: 404 });
        return Response.json(await handler());
      } catch (e) {
        return Response.json({ error: (e as Error).message }, { status: 400 });
      }
    },
  });
}

if (import.meta.main) {
  const consoleBase = arg("console", "http://127.0.0.1:8080")!;
  const httpsPort = Number(arg("https-port", "8443"));
  const fleet = await new Fleet(consoleBase, "127.0.0.1", httpsPort).start();
  const server = serveFleet(fleet, Number(arg("listen", "9500")));
  console.log(`mock-device fleet listening on http://127.0.0.1:${server.port} (console ${consoleBase}, https ${httpsPort})`);
}
