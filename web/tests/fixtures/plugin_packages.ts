import { mkdirSync, writeFileSync, readFileSync, existsSync } from "node:fs";
import { join } from "node:path";

// Plugin packages (zips) for e2e tests: plain, and signed with a throwaway minisign key.

/** Builds a zip with python's zipfile (no zip library in the web tests). */
export function zipOf(dir: string, name: string, files: Record<string, string>): string {
  const src = join(dir, name);
  mkdirSync(src, { recursive: true });
  for (const [f, t] of Object.entries(files)) { mkdirSync(join(src, f, ".."), { recursive: true }); writeFileSync(join(src, f), t); }
  const out = join(dir, `${name}.zip`);
  const r = Bun.spawnSync(["python3", "-c", "import sys,zipfile,os\nz=zipfile.ZipFile(sys.argv[2],'w',zipfile.ZIP_DEFLATED)\nfor r,_,fs in os.walk(sys.argv[1]):\n  for f in fs:\n    p=os.path.join(r,f); z.write(p, os.path.relpath(p,sys.argv[1]))\nz.close()", src, out]);
  if (r.exitCode !== 0) throw new Error("zip failed");
  return out;
}

export const manifest = (id: string, extra: Record<string, unknown> = {}) => JSON.stringify({
  manifest_version: 1, id, name: "Installed Hello", version: "1.0.0", publisher: "Me", description: "e2e",
  web: { entry: "index.html", sandbox: true },
  permissions: [{ id: "ui.panel", reason: "x" }], ui: { placements: [{ slot: "dock" }] }, ...extra,
});
export const html = `<!doctype html><html><head><meta charset="utf-8"><link rel="stylesheet" href="__os/theme.css"></head><body><div id="hi">hello from an installed plugin</div><script src="__os/sdk.js"></script></body></html>`;

export const hasMinisign = Bun.which("minisign") !== null;

/** A signed package: files listed in the manifest (or not), manifest signed with a throwaway minisign key. */
export function signedPackage(dir: string, name: string, id: string, opts: { listFiles: boolean; tamper?: boolean; keyName?: string; publisher?: string }): string {
  const src = join(dir, `${name}-src`);
  mkdirSync(src, { recursive: true });
  writeFileSync(join(src, "index.html"), html);
  writeFileSync(join(src, "app.js"), "console.log('hello');");
  const sh = (args: string[]) => { const r = Bun.spawnSync(args, { cwd: dir }); if (r.exitCode !== 0) throw new Error(`${args[0]} failed: ${r.stderr}`); };
  const keyName = opts.keyName ?? name;
  const pub = join(dir, `${keyName}.pub`), sec = join(dir, `${keyName}.key`);
  if (!existsSync(pub)) sh(["minisign", "-G", "-W", "-f", "-p", pub, "-s", sec]);
  const publisher_key = readFileSync(pub, "utf8").trim().split("\n")[1];
  const m: any = JSON.parse(manifest(id, opts.publisher ? { publisher: opts.publisher } : {}));
  m.publisher_key = publisher_key;
  if (opts.listFiles) {
    m.files = Object.fromEntries(["index.html", "app.js"].map((f) => [f, new Bun.CryptoHasher("sha256").update(readFileSync(join(src, f))).digest("hex")]));
  }
  writeFileSync(join(src, "plugin.json"), JSON.stringify(m, null, 2));
  sh(["minisign", "-S", "-s", sec, "-m", join(src, "plugin.json"), "-x", join(src, "plugin.json.minisig")]);
  if (opts.tamper) writeFileSync(join(src, "app.js"), "steal();");
  const out = join(dir, `${name}.zip`);
  const r = Bun.spawnSync(["python3", "-c", "import sys,zipfile,os\nz=zipfile.ZipFile(sys.argv[2],'w',zipfile.ZIP_DEFLATED)\nfor r,_,fs in os.walk(sys.argv[1]):\n  for f in fs:\n    p=os.path.join(r,f); z.write(p, os.path.relpath(p,sys.argv[1]))\nz.close()", src, out]);
  if (r.exitCode !== 0) throw new Error("zip failed");
  return out;
}

