use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::{Json, Router};
use serde::Deserialize;

use crate::api::ws::AppState;
use crate::storage::db::CcliUsageRow;
use crate::storage::keyring::KeyringService;

use super::common::require_host_token;

/// The real CCLI reporting portal (confirmed via web search, since CCLI's
/// own site blocks automated fetches -- no way to verify its exact login
/// form DOM from here). `docs/CCLI_REPORTING.md`'s "Open questions" already
/// anticipates this: the autofill script below degrades gracefully (does
/// nothing) rather than guessing wrong if CCLI's actual markup doesn't match
/// its generic, type/autocomplete-based field detection.
const CCLI_LOGIN_URL: &str = "https://reporting.ccli.com/";

/// `(service, account)` the CCLI username/password are stored under via the
/// existing generic keyring routes (`/api/keyring/set` et al.,
/// `web/src/core/keyring.ts`'s `KEYRING_SERVICES.CCLI`) -- no new storage
/// mechanism needed, this just has to agree with the frontend's own
/// constant.
const CCLI_KEYRING_SERVICE: &str = "OpenSanctuary:CCLI";
const CCLI_KEYRING_ACCOUNT: &str = "credentials";

#[derive(Deserialize)]
pub(super) struct CcliReportQuery {
    pub start: i64,
    pub end: i64,
}

/// GET /api/reports/ccli-usage
///
/// JSON view of the CCLI usage report (docs/CCLI_REPORTING.md) backing the
/// report window's Reportable/Excluded-Public-Domain split. Console-only,
/// like the CSV export below -- this surfaces song titles, authors, and
/// usage counts across an arbitrary date range, which an unauthenticated
/// LAN caller has no business pulling.
pub(super) async fn get_ccli_usage(
    State(app): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<CcliReportQuery>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    require_host_token(&headers, &app)?;
    match app.db.get_ccli_usage_report(q.start, q.end) {
        Ok(rows) => Ok(Json(serde_json::json!({ "ok": true, "rows": rows }))),
        Err(e) => Ok(Json(serde_json::json!({ "ok": false, "error": e.to_string() }))),
    }
}

/// GET /api/reports/ccli-csv
///
/// CSV download of the same report, shaped for CCLI's own manual
/// entry/upload (Title, Author, CCLI Song Number, Times Used, First Used,
/// Last Used -- docs/CCLI_REPORTING.md). Rows flagged Public Domain are
/// excluded from the file entirely, matching the report window's visible
/// "Excluded -- Public Domain" section: never silently filtered, but never
/// written to the file CCLI actually sees either.
pub(super) async fn get_ccli_csv(
    State(app): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<CcliReportQuery>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    require_host_token(&headers, &app).map_err(|(status, Json(body))| {
        (status, body.get("error").and_then(|v| v.as_str()).unwrap_or("Unauthorized").to_string())
    })?;
    let rows = app
        .db
        .get_ccli_usage_report(q.start, q.end)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let csv = build_ccli_csv_text(&rows);

    axum::http::Response::builder()
        .status(StatusCode::OK)
        .header(axum::http::header::CONTENT_TYPE, "text/csv")
        .header(
            axum::http::header::CONTENT_DISPOSITION,
            "attachment; filename=\"ccli-usage-report.csv\"",
        )
        .body(axum::body::Body::from(csv))
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

/// Builds the CSV text both `get_ccli_csv` (download) and
/// `post_open_ccli_assist` (baked into the upload-assist window below) use
/// -- one shared place for the column shape so the two never drift apart.
fn build_ccli_csv_text(rows: &[CcliUsageRow]) -> String {
    let mut csv = String::from("Title,Author,CCLI Song Number,Times Used,First Used,Last Used\n");
    for row in rows.iter().filter(|r| !r.is_public_domain) {
        csv.push_str(&format!(
            "{},{},{},{},{},{}\n",
            csv_field(&row.title),
            csv_field(&row.author),
            csv_field(row.ccli_number.as_deref().unwrap_or("")),
            row.use_count,
            format_report_date(row.first_used_ms),
            format_report_date(row.last_used_ms),
        ));
    }
    csv
}

/// Quotes a CSV field only when it actually needs it (contains a comma,
/// quote, or newline) -- matches how every spreadsheet tool round-trips
/// plain fields, and keeps the common case (a plain title) readable.
fn csv_field(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

fn format_report_date(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|dt| dt.format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

/// Injected into the CCLI assist window on every page load within it (`wry`'s
/// `with_initialization_script`, which re-runs across navigations, not just
/// the window's first page -- see `src/webview/mod.rs`'s `DisplayEvent::OpenExternal`).
/// Autofill only, never auto-submit -- the user still clicks CCLI's own Log In
/// and Upload buttons themselves (docs/CCLI_REPORTING.md's "narrowed design").
/// Field detection is generic (a password-type input, a file-type input) since
/// CCLI's real DOM structure couldn't be verified from here (their site blocks
/// automated fetches); if a field can't be found, this script simply does
/// nothing on that page rather than guessing wrong.
const CCLI_ASSIST_SCRIPT_TEMPLATE: &str = r#"
(function() {
  var USERNAME = __USERNAME_JSON__;
  var PASSWORD = __PASSWORD_JSON__;
  var CSV_TEXT = __CSV_JSON__;
  var CSV_FILENAME = "ccli-usage-report.csv";

  function setNativeValue(el, value) {
    var proto = el.tagName === "TEXTAREA" ? window.HTMLTextAreaElement.prototype : window.HTMLInputElement.prototype;
    var desc = Object.getOwnPropertyDescriptor(proto, "value");
    if (desc && desc.set) {
      desc.set.call(el, value);
    } else {
      el.value = value;
    }
    el.dispatchEvent(new Event("input", { bubbles: true }));
    el.dispatchEvent(new Event("change", { bubbles: true }));
  }

  // Login autofill: only ever acts on a real type="password" field -- never
  // guesses at a password field by name/id matching, only the username
  // field (which has no equivalently reliable type to key off).
  function tryAutofillLogin() {
    if (!USERNAME && !PASSWORD) return;
    var pw = document.querySelector('input[type="password"]');
    if (!pw || pw.value) return;
    var userField = document.querySelector(
      'input[autocomplete="username"], input[autocomplete="email"], input[type="email"], input[name*="user" i], input[name*="email" i], input[id*="user" i], input[id*="email" i]'
    );
    if (userField && USERNAME && !userField.value) setNativeValue(userField, USERNAME);
    if (PASSWORD) setNativeValue(pw, PASSWORD);
  }

  // Upload autofill: hands the already-generated CSV to the first file
  // input that appears, as if the user had browsed to and selected it
  // themselves. Never clicks Upload/Submit.
  function tryAutofillUpload() {
    if (!CSV_TEXT) return;
    var fileInput = document.querySelector('input[type="file"]');
    if (!fileInput || (fileInput.files && fileInput.files.length)) return;
    try {
      var file = new File([CSV_TEXT], CSV_FILENAME, { type: "text/csv" });
      var dt = new DataTransfer();
      dt.items.add(file);
      fileInput.files = dt.files;
      fileInput.dispatchEvent(new Event("change", { bubbles: true }));
    } catch (e) { /* DataTransfer construction unsupported -- leave the field untouched */ }
  }

  function tick() {
    try { tryAutofillLogin(); } catch (e) {}
    try { tryAutofillUpload(); } catch (e) {}
  }

  // Re-checked on an interval rather than once, since the real form may
  // render asynchronously after this script's first run (document start) --
  // bounded to ~20s per page load, not indefinitely.
  var attempts = 0;
  var intervalId = setInterval(function() {
    tick();
    attempts += 1;
    if (attempts > 40) clearInterval(intervalId);
  }, 500);
  tick();
})();
"#;

fn build_ccli_assist_init_script(username: &str, password: &str, csv: &str) -> String {
    let username_json = serde_json::to_string(username).unwrap_or_else(|_| "\"\"".to_string());
    let password_json = serde_json::to_string(password).unwrap_or_else(|_| "\"\"".to_string());
    let csv_json = serde_json::to_string(csv).unwrap_or_else(|_| "\"\"".to_string());
    CCLI_ASSIST_SCRIPT_TEMPLATE
        .replace("__USERNAME_JSON__", &username_json)
        .replace("__PASSWORD_JSON__", &password_json)
        .replace("__CSV_JSON__", &csv_json)
}

/// POST /api/reports/ccli-open-assist
///
/// Opens CCLI's reporting portal in the app's own native webview window
/// (docs/CCLI_REPORTING.md's "CCLI.com upload" section), with the operator's
/// saved username/password (if any -- via the existing generic keyring,
/// `OpenSanctuary:CCLI`/`credentials`) and the CSV for the given date range
/// baked into an autofill script. Host-token-gated like the reports above --
/// this reads a stored credential and opens a window on the server's own
/// machine, exactly the kind of action already reserved for the console.
pub(super) async fn post_open_ccli_assist(
    State(app): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<CcliReportQuery>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    require_host_token(&headers, &app)?;

    let Some(display_manager) = app.display_manager.as_ref() else {
        return Ok(Json(serde_json::json!({
            "ok": false,
            "error": "Not available in this build (no native desktop window).",
        })));
    };
    if !display_manager.is_ready() {
        return Ok(Json(serde_json::json!({
            "ok": false,
            "error": "Not available -- no native desktop window in this session (headless/browser-only mode).",
        })));
    }

    let rows = app
        .db
        .get_ccli_usage_report(q.start, q.end)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() }))))?;
    let csv = build_ccli_csv_text(&rows);

    let (username, password) = KeyringService::get_secret(CCLI_KEYRING_SERVICE, CCLI_KEYRING_ACCOUNT)
        .ok()
        .flatten()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
        .map(|v| {
            (
                v.get("username").and_then(|u| u.as_str()).unwrap_or("").to_string(),
                v.get("password").and_then(|p| p.as_str()).unwrap_or("").to_string(),
            )
        })
        .unwrap_or_default();

    let init_script = build_ccli_assist_init_script(&username, &password, &csv);

    match display_manager.open_external("CCLI Upload Assistant".to_string(), CCLI_LOGIN_URL.to_string(), init_script) {
        Ok(()) => Ok(Json(serde_json::json!({ "ok": true }))),
        Err(e) => Ok(Json(serde_json::json!({ "ok": false, "error": e }))),
    }
}

pub(super) fn router() -> Router<AppState> {
    use axum::routing::{get, post};
    Router::new()
        .route("/api/reports/ccli-usage", get(get_ccli_usage))
        .route("/api/reports/ccli-csv", get(get_ccli_csv))
        .route("/api/reports/ccli-open-assist", post(post_open_ccli_assist))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(title: &str, ccli_number: Option<&str>, is_public_domain: bool) -> CcliUsageRow {
        CcliUsageRow {
            title: title.to_string(),
            author: "An Author".to_string(),
            ccli_number: ccli_number.map(|s| s.to_string()),
            use_count: 3,
            first_used_ms: 1_700_000_000_000,
            last_used_ms: 1_700_100_000_000,
            is_public_domain,
        }
    }

    #[test]
    fn csv_text_excludes_public_domain_rows() {
        let rows = vec![
            row("Copyrighted Song", Some("1234567"), false),
            row("Amazing Grace", None, true),
        ];
        let csv = build_ccli_csv_text(&rows);
        assert!(csv.contains("Copyrighted Song"));
        assert!(!csv.contains("Amazing Grace"), "public domain rows must never reach the CSV CCLI actually sees");
        assert!(csv.starts_with("Title,Author,CCLI Song Number,Times Used,First Used,Last Used\n"));
    }

    #[test]
    fn csv_text_quotes_fields_containing_commas() {
        let rows = vec![row("Title, With Comma", Some("1"), false)];
        let csv = build_ccli_csv_text(&rows);
        assert!(csv.contains("\"Title, With Comma\""));
    }

    /// The init script embeds credentials/CSV as JS string literals built
    /// from `serde_json::to_string` -- this proves a value containing
    /// characters that would otherwise break out of the literal (quotes,
    /// backslashes, newlines) round-trips exactly, instead of corrupting the
    /// generated script or, worse, enabling injection into it.
    #[test]
    fn assist_script_safely_escapes_values_with_quotes_and_backslashes() {
        let tricky_password = "p\"a\\ss'word\nwith-newline";
        let script = build_ccli_assist_init_script("user@example.com", tricky_password, "Title,Author\n");

        let password_line = script
            .lines()
            .find(|l| l.trim_start().starts_with("var PASSWORD ="))
            .expect("script must define PASSWORD");
        let literal = password_line
            .trim_start()
            .trim_start_matches("var PASSWORD = ")
            .trim_end_matches(';');
        let decoded: String = serde_json::from_str(literal).expect("embedded PASSWORD must be valid JSON");
        assert_eq!(decoded, tricky_password);
    }

    #[test]
    fn assist_script_never_calls_submit_or_click() {
        let script = build_ccli_assist_init_script("u", "p", "csv");
        // The whole point of "autofill, don't auto-submit" (docs/CCLI_REPORTING.md) --
        // this is a cheap guardrail against someone adding a submit call later.
        assert!(!script.contains(".submit("), "must never submit a form");
        assert!(!script.contains(".click("), "must never click a button on the user's behalf");
    }

    #[test]
    fn assist_script_omits_autofill_when_nothing_to_fill() {
        let script = build_ccli_assist_init_script("", "", "");
        assert!(script.contains(r#"var USERNAME = "";"#));
        assert!(script.contains(r#"var CSV_TEXT = "";"#));
    }
}
