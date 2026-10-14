/**
 * OpenSanctuary / OS-Next First-Time Setup Wizard (docs/first-time.md).
 *
 * Shown automatically once, the first time `library.db` is created (see
 * `first_run` in /api/server-info, set from `src/main.rs`) -- and re-openable
 * any time from Settings > About > "Re-run First-Time Setup". Renders as an
 * overlay on top of the normal (empty) console rather than a full takeover,
 * so dismissing it early never strands the user.
 */

import { api } from "../core/api_client.ts";
import { showToast } from "../core/ui_utils.ts";
import { showModal, closeModal, registerModalReset } from "./dialog_manager.ts";
import { loadSettingsServerInfo } from "./settings_dialog.ts";
import { escapeHtml } from "../core/presentation_helpers.ts";
import { appOptions, saveAppOptions } from "../app_core.ts";

const MODAL_ID = "first-time-setup-modal";

const STEPS = ["welcome", "data-dir", "church-name", "bibles", "songs", "network", "done"] as const;
type Step = typeof STEPS[number];

let stepIndex = 0;
let ftsServerInfo: any = null;

/** Called once at boot. Shows the wizard only if this run created library.db. */
export async function maybeShowFirstTimeSetup(): Promise<void> {
  try {
    const info = await loadSettingsServerInfo();
    if (info && (info as any).first_run) {
      ftsServerInfo = info;
      stepIndex = 0;
      renderStep();
      showModal(MODAL_ID);
    }
  } catch (_) { /* not worth blocking boot over */ }
}

/** Opens the wizard from the start -- a manual re-run from Settings > About. */
export async function showFirstTimeSetup(): Promise<void> {
  try {
    ftsServerInfo = await loadSettingsServerInfo();
  } catch (_) { /* data-dir step just shows "(unavailable)" */ }
  stepIndex = 0;
  renderStep();
  showModal(MODAL_ID);
}

function closeWizard(): void {
  closeModal(MODAL_ID);
}

function getAppOptions(): Record<string, any> {
  return appOptions || {};
}

function currentOrigin(): { protocol: string; hostname: string; port: string } {
  const loc = window.location;
  return { protocol: loc.protocol, hostname: loc.hostname, port: loc.port };
}

function renderStep(): void {
  const body = document.getElementById("first-time-setup-body");
  const backBtn = document.getElementById("btn-first-time-back") as HTMLButtonElement | null;
  const nextBtn = document.getElementById("btn-first-time-next") as HTMLButtonElement | null;
  const skipBtn = document.getElementById("btn-first-time-skip") as HTMLButtonElement | null;
  if (!body) return;

  const step = STEPS[stepIndex];
  if (backBtn) backBtn.style.visibility = stepIndex === 0 ? "hidden" : "visible";
  if (skipBtn) skipBtn.style.display = step === "welcome" || step === "done" ? "none" : "";
  if (nextBtn) nextBtn.textContent = step === "done" ? "Done" : step === "welcome" ? "Let's Go ➜" : "Next ➜";

  body.innerHTML = stepHtml(step);
  wireStep(step, body);
}

function stepHtml(step: Step): string {
  switch (step) {
    case "welcome":
      return `
        <div class="fts-hero">
          <div class="fts-hero-icon">👋</div>
          <h2 class="fts-title">Welcome to OpenSanctuary</h2>
          <p class="fts-text fts-text--center">
            A few quick steps to get your sanctuary name, Bible/song content, and network
            details set up. Everything here can be changed later in Settings.
          </p>
        </div>`;

    case "data-dir": {
      const info: any = ftsServerInfo || {};
      const legacyDir = info.legacy_library_detected;
      const legacyBanner = legacyDir
        ? `
        <div id="fts-legacy-library-banner" class="os-callout os-callout--info fts-callout">
          <div>
            📦 Found an existing library at <code>${escapeHtml(legacyDir)}</code> — looks like an
            older portable install. Use it instead of starting fresh?
          </div>
          <div>
            <button class="btn btn-primary" id="fts-adopt-legacy-library" data-legacy-dir="${escapeHtml(legacyDir)}">Use This Library Instead</button>
          </div>
        </div>`
        : "";
      return `
        <h3 class="fts-step-title">Where your data lives</h3>
        <p class="fts-text">
          Your library database, Bible/song content, and downloaded media are stored here:
        </p>
        <div class="os-code-box fts-path">
          ${escapeHtml(info.data_dir || "(unavailable)")}
        </div>
        ${legacyBanner}
        <p class="form-help">
          You can point individual content directories (Bibles, Songs, Media Cache) at a
          different location later in Settings &gt; Storage.
        </p>`;
    }

    case "church-name": {
      const current = getAppOptions().churchName || "";
      return `
        <h3 class="fts-step-title">Church / Sanctuary Name</h3>
        <p class="fts-text">
          Shown on the live output footer and advertised to Roku/Android TV clients
          discovering this console on the network.
        </p>
        <input type="text" id="fts-church-name" class="search-input" placeholder="Your Church Name" value="${escapeHtml(current)}">`;
    }

    case "bibles": {
      return `
        <h3 class="fts-step-title">Bible Content</h3>
        <p class="fts-text">
          No Bible translations are bundled by default (most are copyrighted). Downloading them is
          done by the <strong>Online Bibles</strong> plugin. Turning it on lets this server fetch Bible
          text from bolls.life, api.churchapps.org, bible-api.com and GitHub (and nowhere else) and add
          it to your Bible library. You can review or turn it off any time in Settings &rarr; Plugins.
        </p>
        <div class="form-row">
          <button class="btn btn-primary" id="fts-enable-online-bibles" type="button">Turn on Online Bibles</button>
          <button class="btn" id="fts-open-plugins" type="button">Review in Settings</button>
        </div>
        <div class="os-divider"></div>
        <p class="form-help">Already have Bible files (.db) from another install?</p>
        <div class="form-row">
          <input type="text" id="fts-bibles-dir" class="search-input" placeholder="Path to a folder of Bible .db files">
          <button class="btn" id="fts-bibles-browse" type="button">Browse…</button>
        </div>`;
    }

    case "songs":
      return `
        <h3 class="fts-step-title">Song Content</h3>
        <p class="fts-text">
          A handful of public-domain hymns are already included. If you have a larger song
          library (e.g. from another install), point at it here -- otherwise, skip.
        </p>
        <div class="form-row">
          <input type="text" id="fts-songs-dir" class="search-input" placeholder="Path to a folder of song databases">
          <button class="btn" id="fts-songs-browse" type="button">Browse…</button>
        </div>`;

    case "network": {
      const { protocol, hostname, port } = currentOrigin();
      const base = `${protocol}//${hostname}${port ? ":" + port : ""}`;
      const hostAdvert = getAppOptions().networkHostname || "opensanctuary";
      return `
        <h3 class="fts-step-title">Your Network Setup</h3>
        <p class="fts-text">
          This console is reachable at <code>${escapeHtml(base)}</code>, and advertises
          itself on the network as <code>${escapeHtml(hostAdvert)}.local</code>.
        </p>
        <ul class="fts-list">
          <li><strong>Operator Console</strong> — this window, where you build and run the schedule.</li>
          <li><strong>Live Output</strong> — <code>${escapeHtml(base)}/live.html</code>, what the congregation sees. Open on a second display/projector.</li>
          <li><strong>Stage Foldback</strong> — <code>${escapeHtml(base)}/stage.html</code>, a monitor-facing view for the stage/platform.</li>
          <li><strong>Remote Control</strong> — pair a phone from the Live menu to control the schedule remotely.</li>
        </ul>
        <p class="form-help">More detail and hostname/port options are in Settings &gt; Network.</p>`;
    }

    case "done":
      return `
        <div class="fts-hero">
          <div class="fts-hero-icon">✅</div>
          <h2 class="fts-title">You're all set</h2>
          <p class="fts-text">Revisit any of this anytime in Settings.</p>
        </div>`;
  }
}

function wireStep(step: Step, body: HTMLElement): void {
  if (step === "data-dir") {
    const adoptBtn = body.querySelector("#fts-adopt-legacy-library") as HTMLButtonElement | null;
    adoptBtn?.addEventListener("click", () => adoptLegacyLibrary(adoptBtn));
  } else if (step === "bibles") {
    const enableBtn = body.querySelector("#fts-enable-online-bibles") as HTMLButtonElement | null;
    enableBtn?.addEventListener("click", () => enableOnlineBibles(enableBtn));
    body.querySelector("#fts-open-plugins")?.addEventListener("click", () => {
      window.dispatchEvent(new CustomEvent("os-open-plugins-settings"));
    });
    wireDirectoryBrowse(body, "fts-bibles-dir", "fts-bibles-browse");
  } else if (step === "songs") {
    wireDirectoryBrowse(body, "fts-songs-dir", "fts-songs-browse");
  }
}

function wireDirectoryBrowse(body: HTMLElement, inputId: string, buttonId: string): void {
  const input = body.querySelector(`#${inputId}`) as HTMLInputElement | null;
  const btn = body.querySelector(`#${buttonId}`) as HTMLButtonElement | null;
  if (!input || !btn) return;
  btn.addEventListener("click", async () => {
    btn.disabled = true;
    try {
      const result = await api.system.pickFolder();
      if (!result.available) {
        showToast("Folder picker is only available in the desktop app — type the path instead.", "warning");
      } else if (result.path) {
        input.value = result.path;
      }
    } catch (_) {
      showToast("Could not open the folder picker.", "error");
    } finally {
      btn.disabled = false;
    }
  });
}

async function adoptLegacyLibrary(btn: HTMLButtonElement): Promise<void> {
  const legacyDir = btn.dataset.legacyDir || "";
  if (!legacyDir) return;
  if (!window.confirm(
    `Use the existing library at "${legacyDir}" instead of the fresh one just created?\n\n` +
    `OpenSanctuary will need to be restarted for this to take effect. The newly-created, ` +
    `still-empty database at the default location is left in place, unused.`
  )) {
    return;
  }
  btn.disabled = true;
  const originalText = btn.textContent;
  btn.textContent = "Switching…";
  try {
    const result = await api.system.adoptLegacyLibrary(legacyDir);
    if (result.ok) {
      btn.textContent = "✓ Restart OpenSanctuary to finish";
      showToast("Restart OpenSanctuary to start using your existing library.", "success");
    } else {
      showToast(result.error || "Could not switch to that library.", "error");
      btn.disabled = false;
      btn.textContent = originalText || "Use This Library Instead";
    }
  } catch (e: any) {
    showToast(`Could not switch to that library: ${e?.message || e}`, "error");
    btn.disabled = false;
    btn.textContent = originalText || "Use This Library Instead";
  }
}

/** Approves the bundled Online Bibles plugin with everything it asks for (the step's text says what that is). */
async function enableOnlineBibles(btn: HTMLButtonElement): Promise<void> {
  const originalText = btn.textContent;
  btn.disabled = true;
  btn.textContent = "Turning on…";
  try {
    const plugins = await api.plugins.list();
    const plugin = plugins.find((p) => p.id === "online_bibles");
    if (!plugin) throw new Error("the Online Bibles plugin isn't installed");
    if (plugin.state !== "active") {
      const updated = await api.plugins.approve(plugin.id, plugin.digest, plugin.requested.map((r) => r.id));
      window.dispatchEvent(new CustomEvent("os-plugin-changed", { detail: { plugin: updated } }));
    }
    showToast("Online Bibles is on: use its panel at the bottom right to download a translation.", "success");
    btn.textContent = "✓ Online Bibles is on";
  } catch (e: any) {
    showToast(`Could not turn on Online Bibles: ${e?.message || e}`, "error");
    btn.disabled = false;
    btn.textContent = originalText || "Turn on Online Bibles";
  }
}

/** Persists whatever the current step's inputs hold before advancing/finishing. */
async function commitStep(step: Step, body: HTMLElement): Promise<void> {
  if (step === "church-name") {
    const input = body.querySelector("#fts-church-name") as HTMLInputElement | null;
    const value = input ? input.value.trim() : "";
    if (value) await saveAppOptions({ churchName: value });
  } else if (step === "bibles") {
    const input = body.querySelector("#fts-bibles-dir") as HTMLInputElement | null;
    const value = input ? input.value.trim() : "";
    if (value) {
      await api.settings.save({ biblesDirectory: value });
      showToast("Bibles directory saved — takes effect next time OpenSanctuary starts.", "info");
    }
  } else if (step === "songs") {
    const input = body.querySelector("#fts-songs-dir") as HTMLInputElement | null;
    const value = input ? input.value.trim() : "";
    if (value) {
      await api.settings.save({ songsDirectory: value });
      showToast("Songs directory saved — takes effect next time OpenSanctuary starts.", "info");
    }
  }
}

export function initFirstTimeSetup(): void {
  const closeBtn = document.getElementById("btn-close-first-time-setup");
  const backBtn = document.getElementById("btn-first-time-back");
  const nextBtn = document.getElementById("btn-first-time-next");
  const skipBtn = document.getElementById("btn-first-time-skip");
  const body = document.getElementById("first-time-setup-body");

  // Fires on ANY close path (X button, Escape, or a normal Finish) --
  // dismissing the wizard early should never leave it nagging on every
  // subsequent reload. "Re-run First-Time Setup" in Settings doesn't check
  // this flag at all, so it stays reachable regardless.
  registerModalReset(MODAL_ID, () => {
    api.settings.save({ firstTimeSetupCompleted: "true" }).catch(() => {});
  });

  closeBtn?.addEventListener("click", () => closeWizard());
  skipBtn?.addEventListener("click", () => {
    stepIndex = Math.min(stepIndex + 1, STEPS.length - 1);
    renderStep();
  });
  backBtn?.addEventListener("click", () => {
    stepIndex = Math.max(stepIndex - 1, 0);
    renderStep();
  });
  nextBtn?.addEventListener("click", async () => {
    if (!body) return;
    const step = STEPS[stepIndex];
    await commitStep(step, body);
    if (step === "done") {
      closeWizard();
      showToast("Setup complete — revisit anytime in Settings.", "success");
      return;
    }
    stepIndex = Math.min(stepIndex + 1, STEPS.length - 1);
    renderStep();
  });
}
