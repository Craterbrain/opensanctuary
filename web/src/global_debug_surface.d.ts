// Ambient declaration for OpenSanctuary's intentional debug-introspection
// surface: app_core.ts and app_ui.ts mirror a copy of a few of their internal,
// module-scoped values/functions onto the global object (wrapped in
// `try { ... } catch (_) {}` so a failed mirror can never break the app
// itself), purely so a developer can inspect/call them from the browser
// devtools console. This file is the single place that surface is declared,
// so a call site only needs `globalThis.NAME`, not a repeated `as any` cast.
//
// This is NOT how app_core.ts and app_ui.ts actually talk to each other --
// real cross-file dependencies between them are normal ES module
// imports/exports (see the top of each file). Anything declared here is,
// deliberately, dead weight the app itself never reads back. (A much larger
// version of this file used to also cover ~200 names that were a genuine
// inter-module globalThis bridge between app_core.ts and app_ui.ts -- that
// bridge was removed in favor of real imports/exports and a `registerUiCallbacks`
// handback for the one direction that can't statically import; see each
// file's own top-of-file comments for how they actually talk to each other
// now.)
//
// A second block below covers the handful of names mirrored the same way
// that are NOT dead weight -- Playwright E2E tests drive the live app
// directly via `(window as any).NAME`, outside this module graph entirely.

export {};

declare global {
  var activeSelectedSlide: any;
  var clearActiveSelectedSlide: any;
  var clearActiveUndoPlaceholder: any;
  var deleteScheduleItemByIndex: any;
  var getActiveSelectedSlide: any;
  var getSlideBadge: any;
  var getSlideBadgeColor: any;
  var setActiveSelectedSlide: any;
  var switchToRemoteTab: any;
  var updatePairingQrCode: any;
  var updateRemoteQrCode: any;
  var ws: any;

  // Real (if untidy) hidden dependencies -- Playwright E2E tests reach these
  // directly via `(window as any).NAME` (tests/e2e_compare_windows_linux.test.ts,
  // e2e_slide_templates.test.ts, e2e_bulk_paste.test.ts, e2e_image_crop.test.ts,
  // e2e_rich_text_selection.test.ts, theme_picker.test.ts).
  var __debugApplyTextStyle: any;
  var __debugInsertElement: any;
  var getCanvasSlideEditorDebugState: any;
  var openImportModal: any;
  var openSlideEditor: any;
  var openThemePicker: any;
  var sendCommand: any;
}
