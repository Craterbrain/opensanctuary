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

/**
 * Curated, not auto-detected: the online catalog (ChurchApps/OpenLP/Bolls.life)
 * mixes public-domain and copyrighted translations with no reliable license
 * field to filter on, so this list is hand-picked -- everything here is
 * pre-1928 (US public domain by age) or, for BSB/WEB, explicitly released
 * public domain by its own translation project. Verified present in the live
 * catalog under these exact abbreviations before adding them here.
 */
const PUBLIC_DOMAIN_TRANSLATIONS: Array<{ abbr: string; label: string }> = [
  { abbr: "KJV", label: "King James Version (1769)" },
  { abbr: "ASV", label: "American Standard Version (1901)" },
  { abbr: "WEB", label: "World English Bible" },
  { abbr: "BSB", label: "Berean Standard Bible" },
  { abbr: "YLT", label: "Young's Literal Translation (1898)" },
  { abbr: "DBY", label: "Darby Translation (1890)" },
  { abbr: "DRB", label: "Douay-Rheims Bible (1899)" },
  { abbr: "GNV", label: "Geneva Bible (1599)" },
];

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
        <div style="text-align:center; padding: 12px 0;">
          <div style="font-size: 42px; margin-bottom: 8px;">👋</div>
          <h2 style="margin: 0 0 8px;">Welcome to OpenSanctuary</h2>
          <p style="color: var(--text-dim); max-width: 480px; margin: 0 auto;">
            A few quick steps to get your sanctuary name, Bible/song content, and network
            details set up. Everything here can be changed later in Settings.
          </p>
        </div>`;

    case "data-dir": {
      const info: any = ftsServerInfo || {};
      const legacyDir = info.legacy_library_detected;
      const legacyBanner = legacyDir
        ? `
        <div id="fts-legacy-library-banner" style="padding: 10px 12px; background: rgba(255, 167, 38, 0.1); border: 1px solid var(--os-brand-amber, #ffa726); border-radius: 6px; display: flex; flex-direction: column; gap: 8px;">
          <div style="font-size: 13px;">
            📦 Found an existing library at <code>${escapeHtml(legacyDir)}</code> — looks like an
            older portable install. Use it instead of starting fresh?
          </div>
          <div>
            <button class="btn btn-primary" id="fts-adopt-legacy-library" data-legacy-dir="${escapeHtml(legacyDir)}">Use This Library Instead</button>
          </div>
        </div>`
        : "";
      return `
        <h3 style="margin: 0;">Where your data lives</h3>
        <p style="color: var(--text-dim);">
          Your library database, Bible/song content, and downloaded media are stored here:
        </p>
        <div class="settings-readonly-value" style="padding: 10px 12px; background: var(--bg-elevated, #1a1a1a); border-radius: 6px; font-family: monospace;">
          ${escapeHtml(info.data_dir || "(unavailable)")}
        </div>
        ${legacyBanner}
        <p style="color: var(--text-dim); font-size: 12px;">
          You can point individual content directories (Bibles, Songs, Media Cache) at a
          different location later in Settings &gt; Storage.
        </p>`;
    }

    case "church-name": {
      const current = getAppOptions().churchName || "";
      return `
        <h3 style="margin: 0;">Church / Sanctuary Name</h3>
        <p style="color: var(--text-dim);">
          Shown on the live output footer and advertised to Roku/Android TV clients
          discovering this console on the network.
        </p>
        <input type="text" id="fts-church-name" class="search-input" placeholder="Your Church Name" value="${escapeHtml(current)}">`;
    }

    case "bibles": {
      const options = PUBLIC_DOMAIN_TRANSLATIONS
        .map((t) => `<option value="${escapeHtml(t.abbr)}">${escapeHtml(t.label)}</option>`)
        .join("");
      return `
        <h3 style="margin: 0;">Bible Content</h3>
        <p style="color: var(--text-dim);">
          No Bible translations are bundled by default (most are copyrighted) -- download a
          public-domain translation now, or point at content you already have.
        </p>
        <div style="display:flex; gap:8px; align-items:center;">
          <select id="fts-bible-select" class="search-input" style="flex:1;">${options}</select>
          <button class="btn btn-primary" id="fts-download-bible">⬇ Download</button>
        </div>
        <div style="height: 1px; background: var(--border-color, #333); margin: 4px 0;"></div>
        <p style="color: var(--text-dim); font-size: 12px; margin: 0;">Already have Bible files (.db) from another install?</p>
        <div style="display:flex; gap:8px; align-items:center;">
          <input type="text" id="fts-bibles-dir" class="search-input" style="flex:1;" placeholder="Path to a folder of Bible .db files">
          <button class="btn" id="fts-bibles-browse" type="button">Browse…</button>
        </div>`;
    }

    case "songs":
      return `
        <h3 style="margin: 0;">Song Content</h3>
        <p style="color: var(--text-dim);">
          A handful of public-domain hymns are already included. If you have a larger song
          library (e.g. from another install), point at it here -- otherwise, skip.
        </p>
        <div style="display:flex; gap:8px; align-items:center;">
          <input type="text" id="fts-songs-dir" class="search-input" style="flex:1;" placeholder="Path to a folder of song databases">
          <button class="btn" id="fts-songs-browse" type="button">Browse…</button>
        </div>`;

    case "network": {
      const { protocol, hostname, port } = currentOrigin();
      const base = `${protocol}//${hostname}${port ? ":" + port : ""}`;
      const hostAdvert = getAppOptions().networkHostname || "opensanctuary";
      return `
        <h3 style="margin: 0;">Your Network Setup</h3>
        <p style="color: var(--text-dim);">
          This console is reachable at <code>${escapeHtml(base)}</code>, and advertises
          itself on the network as <code>${escapeHtml(hostAdvert)}.local</code>.
        </p>
        <ul style="color: var(--text-dim); line-height: 1.8; margin: 0; padding-left: 20px;">
          <li><strong>Operator Console</strong> — this window, where you build and run the schedule.</li>
          <li><strong>Live Output</strong> — <code>${escapeHtml(base)}/live.html</code>, what the congregation sees. Open on a second display/projector.</li>
          <li><strong>Stage Foldback</strong> — <code>${escapeHtml(base)}/stage.html</code>, a monitor-facing view for the stage/platform.</li>
          <li><strong>Remote Control</strong> — pair a phone from the Live menu to control the schedule remotely.</li>
        </ul>
        <p style="color: var(--text-dim); font-size: 12px;">More detail and hostname/port options are in Settings &gt; Network.</p>`;
    }

    case "done":
      return `
        <div style="text-align:center; padding: 12px 0;">
          <div style="font-size: 42px; margin-bottom: 8px;">✅</div>
          <h2 style="margin: 0 0 8px;">You're all set</h2>
          <p style="color: var(--text-dim);">Revisit any of this anytime in Settings.</p>
        </div>`;
  }
}

function wireStep(step: Step, body: HTMLElement): void {
  if (step === "data-dir") {
    const adoptBtn = body.querySelector("#fts-adopt-legacy-library") as HTMLButtonElement | null;
    adoptBtn?.addEventListener("click", () => adoptLegacyLibrary(adoptBtn));
  } else if (step === "bibles") {
    const downloadBtn = body.querySelector("#fts-download-bible") as HTMLButtonElement | null;
    const select = body.querySelector("#fts-bible-select") as HTMLSelectElement | null;
    downloadBtn?.addEventListener("click", () => {
      const abbr = select?.value || "KJV";
      const label = PUBLIC_DOMAIN_TRANSLATIONS.find((t) => t.abbr === abbr)?.label || abbr;
      downloadBible(abbr, label, downloadBtn);
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

async function downloadBible(abbr: string, label: string, btn: HTMLButtonElement): Promise<void> {
  const originalText = btn.textContent;
  btn.disabled = true;
  btn.textContent = `⏳ Downloading ${label}…`;
  try {
    let translation = abbr.toLowerCase();
    let provider = "churchapps";
    let sourceKey: string | null = null;
    try {
      const data = await api.bibles.onlineCatalog();
      const list: any[] = Array.isArray(data) ? data : data.catalog || [];
      const entry = list.find((b) => (b.abbreviation || "").toUpperCase() === abbr);
      if (entry) {
        translation = entry.id;
        provider = entry.provider || "churchapps";
        sourceKey = entry.source_key || null;
      }
    } catch (_) { /* fall through with the abbreviation itself as translation id */ }

    const result = await api.bibles.downloadOnline({ translation, provider, source_key: sourceKey });
    showToast(`✓ Downloaded ${label} (${result.verses_count ?? "?"} verses)`, "success");
    btn.textContent = `✓ ${label} Installed`;
  } catch (e: any) {
    showToast(`Could not download ${label}: ${e?.message || e}`, "error");
    btn.disabled = false;
    btn.textContent = originalText || "⬇ Download";
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
