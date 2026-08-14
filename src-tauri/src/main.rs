// SPDX-License-Identifier: AGPL-3.0-or-later
// Prevents an extra console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Anthers — the desktop shell around the whole Anthers app.
//!
//! It bundles the SAME `apps/studio-web` build the browser Studio serves; this is a
//! third consumer of `@anthers/web-shared`, not a fork. What the shell adds is the
//! things a browser tab cannot do: a session that survives without cookies, and
//! (next stage) native ffmpeg encoding that doesn't tie the creator to a tab.
//!
//! ## Why the window is not just pointed at studio.anthers.org
//!
//! `invoke` is unavailable to remote documents, so native capability would mean
//! opening the IPC boundary to a page served over the internet — an XSS on the Studio
//! would then reach filesystem and process spawn on every creator's machine. The SPA
//! is bundled instead, which costs the auth work below. See 42.06 § Desktop auth.
//!
//! ## Auth
//!
//! Serving from `tauri://localhost` means no session cookie is ever sent, so the app
//! carries an `Authorization: Bearer` token instead. It is obtained by a browser
//! handoff: the app opens the authorize page in the SYSTEM browser (no password is
//! ever typed here), the creator confirms, and a one-time code comes back over the
//! `anthers://` scheme. PKCE binds the two halves — the verifier never leaves this
//! process, so another local app that hijacks the scheme and steals the code cannot
//! redeem it.
//!
//! ## How the webview learns any of this
//!
//! `initialization_script` runs before any app JS, defining
//! `globalThis.__ANTHERS_DESKTOP__` = `{ apiBaseUrl, getToken(), fetch }` — the seam
//! `apiFetch()` in `@anthers/web-shared` already reads. `fetch` is routed through
//! `tauri-plugin-http`, so requests leave from Rust and CORS never applies.

mod encode;
mod token;

use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, Runtime, Url};
use tauri_plugin_deep_link::DeepLinkExt;

use crate::token::{Persistence, TokenStore};

/// The API origin this build talks to. Override at build time for a staging host:
/// `ANTHERS_API_BASE=https://staging.example.org bunx tauri build`.
fn api_base_url() -> String {
	option_env!("ANTHERS_API_BASE")
		.map(str::to_string)
		.unwrap_or_else(|| {
			// Debug builds target the local dev API so `bunx tauri dev` works against
			// `make dev` with no configuration.
			if cfg!(debug_assertions) {
				"http://localhost:8000".to_string()
			} else {
				"https://anthers.org".to_string()
			}
		})
}

/// The CONSUMER SITE's origin — where the authorize page lives.
///
/// Deliberately separate from `api_base_url()`. In production they are the same apex
/// origin, which makes it tempting to reuse one value; in dev they are NOT (the SPA is
/// on :3000, the API on :8000), so building the authorize URL from the API base opens
/// `localhost:8000/desktop/authorize` — a route the API does not serve. That is a
/// dev-only 404, i.e. broken in exactly the environment this gets tested in.
fn web_base_url() -> String {
	option_env!("ANTHERS_WEB_BASE")
		.map(str::to_string)
		.unwrap_or_else(|| {
			if cfg!(debug_assertions) {
				"http://localhost:3000".to_string()
			} else {
				"https://anthers.org".to_string()
			}
		})
}

/// The PKCE verifier for the enrolment currently in flight. Never leaves this process
/// — that is precisely what makes a stolen callback code useless.
#[derive(Default)]
struct PendingVerifier(Mutex<Option<String>>);

/// A callback code that arrived before the webview was listening (cold start, or the
/// link that launched the app). The webview pulls it once on boot.
#[derive(Default)]
struct PendingCode(Mutex<Option<String>>);

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SessionState {
	token: Option<String>,
	api_base_url: String,
}

/// POST a JSON body using the http plugin's bundled reqwest.
///
/// Serialized by hand rather than via `.json()`, because the plugin does not enable
/// reqwest's `json` feature and declaring our own `reqwest` alongside it would risk a
/// second, version-skewed copy of the whole stack for one convenience method.
async fn post_json(
	url: &str,
	body: &serde_json::Value,
) -> Result<tauri_plugin_http::reqwest::Response, String> {
	tauri_plugin_http::reqwest::Client::new()
		.post(url)
		.header("Content-Type", "application/json")
		.body(body.to_string())
		.send()
		.await
		.map_err(|e| format!("Could not reach Anthers: {e}"))
}

/// Cryptographically random 64-char hex, matching the server's token shape.
fn random_hex() -> String {
	let mut bytes = [0u8; 32];
	getrandom::getrandom(&mut bytes).expect("system RNG unavailable");
	bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn sha256_hex(input: &str) -> String {
	use sha2::{Digest, Sha256};
	let mut hasher = Sha256::new();
	hasher.update(input.as_bytes());
	hasher.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// Pull the `code` out of an `anthers://auth/callback?code=…` deep link.
///
/// Custom-scheme URLs parse inconsistently across platforms — `anthers://auth/callback`
/// puts `auth` in the host on some and in the path on others — so this checks the
/// scheme and reads the query, without insisting on a particular host/path split.
fn callback_code(raw: &str) -> Option<String> {
	let url = Url::parse(raw).ok()?;
	if url.scheme() != "anthers" {
		return None;
	}
	url.query_pairs()
		.find(|(k, _)| k == "code")
		.map(|(_, v)| v.into_owned())
}

fn reveal_main_window<R: Runtime>(app: &AppHandle<R>) {
	if let Some(window) = app.get_webview_window("main") {
		let _ = window.show();
		let _ = window.unminimize();
		let _ = window.set_focus();
	}
}

/// Stash the code for the boot-time pull, tell a live webview, and surface the window.
fn dispatch_code<R: Runtime>(app: &AppHandle<R>, code: String) {
	if let Some(pending) = app.try_state::<PendingCode>() {
		if let Ok(mut slot) = pending.0.lock() {
			*slot = Some(code.clone());
		}
	}
	let _ = app.emit("desktop-auth-code", code);
	reveal_main_window(app);
}

fn handle_urls<R: Runtime, S: AsRef<str>>(app: &AppHandle<R>, raws: &[S]) {
	for raw in raws {
		if let Some(code) = callback_code(raw.as_ref()) {
			dispatch_code(app, code);
			return;
		}
	}
}

// ─── Commands ────────────────────────────────────────────────────────────────

/// What the app knows at boot: the stored token (if any) and which API to talk to.
#[tauri::command]
fn session_state(store: tauri::State<'_, TokenStore>) -> SessionState {
	SessionState { token: store.load(), api_base_url: api_base_url() }
}

/// Begin enrolment: mint a PKCE verifier, register the challenge with the API, and
/// open the authorize page in the system browser.
///
/// Returns the URL that was opened so the UI can offer it as copyable text — on a
/// machine with no default browser handler, "click here" is otherwise a dead end.
#[tauri::command]
async fn begin_sign_in(
	pending: tauri::State<'_, PendingVerifier>,
	label: String,
) -> Result<String, String> {
	let verifier = random_hex();
	let challenge = sha256_hex(&verifier);

	let base = api_base_url();
	let res = post_json(
		&format!("{base}/api/auth/desktop/start"),
		&serde_json::json!({ "challenge": challenge, "label": label }),
	)
	.await?;
	if !res.status().is_success() {
		return Err(format!("Anthers refused the sign-in request ({})", res.status()));
	}

	// Only store the verifier once the server has accepted its challenge, so a failed
	// start can't leave a stale verifier that a later callback would try to use.
	if let Ok(mut slot) = pending.0.lock() {
		*slot = Some(verifier);
	}

	// The authorize page is served by the consumer SPA, not the API — see web_base_url().
	let url = format!("{}/desktop/authorize?challenge={challenge}", web_base_url());
	tauri_plugin_opener::open_url(&url, None::<&str>)
		.map_err(|e| format!("Could not open your browser: {e}"))?;
	Ok(url)
}

/// Redeem a callback code for the session token, and persist it.
#[tauri::command]
async fn complete_sign_in(
	pending: tauri::State<'_, PendingVerifier>,
	store: tauri::State<'_, TokenStore>,
	code: String,
) -> Result<SignInResult, String> {
	let verifier = pending
		.0
		.lock()
		.ok()
		.and_then(|mut slot| slot.take())
		.ok_or("This sign-in didn't start here. Try signing in again.")?;

	let base = api_base_url();
	let res = post_json(
		&format!("{base}/api/auth/desktop/exchange"),
		&serde_json::json!({ "code": code, "verifier": verifier }),
	)
	.await?;
	if !res.status().is_success() {
		return Err("That sign-in link is no longer valid. Try again.".into());
	}

	let text = res.text().await.map_err(|e| format!("Unexpected reply from Anthers: {e}"))?;
	let body: serde_json::Value =
		serde_json::from_str(&text).map_err(|e| format!("Unexpected reply from Anthers: {e}"))?;
	let token = body
		.get("token")
		.and_then(|t| t.as_str())
		.ok_or("Anthers did not return a session.")?;

	let persistence = store.save(token);
	Ok(SignInResult { token: token.to_string(), persistence })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SignInResult {
	token: String,
	persistence: Persistence,
}

/// Pull a code captured before the webview mounted. Null if there wasn't one.
#[tauri::command]
fn take_pending_code(pending: tauri::State<'_, PendingCode>) -> Option<String> {
	pending.0.lock().ok().and_then(|mut slot| slot.take())
}

/// Forget the local credential. The server-side session is ended separately by the
/// app's own sign-out call, but this must succeed regardless — a network failure
/// should never leave a usable token sitting on the machine.
#[tauri::command]
fn sign_out(store: tauri::State<'_, TokenStore>) {
	store.clear();
}

// ─── Native encoding ─────────────────────────────────────────────────────────

/// Encode a video into the variant ladder with the bundled ffmpeg. Runs on a blocking
/// task: a long encode must not stall the IPC runtime that carries its own progress
/// events.
#[tauri::command]
async fn encode_video(app: AppHandle, path: String) -> Result<encode::EncodeResult, String> {
	encode::encode_ladder(app, path).await
}

/// Delete a finished encode's temp directory. Called after the variants are uploaded;
/// failure is not worth surfacing, since the OS reclaims the temp dir anyway.
#[tauri::command]
fn cleanup_encode(work_dir: String) {
	// Refuse anything that isn't one of our own encode folders — this deletes a tree,
	// and a bad path from a compromised webview should not be able to aim it.
	let path = std::path::PathBuf::from(&work_dir);
	let is_ours = path
		.file_name()
		.and_then(|n| n.to_str())
		.is_some_and(|n| n.starts_with("anthers-encode-"));
	if is_ours && path.starts_with(std::env::temp_dir()) {
		let _ = std::fs::remove_dir_all(path);
	}
}

// ─── Entry ───────────────────────────────────────────────────────────────────

/// Script evaluated before any app JS, defining the seam `@anthers/web-shared`'s
/// `apiFetch()` reads. The token is injected at boot and refreshed by the app after
/// enrolment via `__ANTHERS_DESKTOP__.setToken`.
///
/// No `fetch` is supplied, so `apiFetch()` uses the webview's own — a plain
/// cross-origin request from `tauri://localhost` carrying a bearer header and NO
/// cookie, which the API's CORS allowlist admits. 42.06 preferred routing everything
/// through `tauri-plugin-http` (fetch from Rust, CORS never applies); the objection it
/// raised to the CORS route was that it could only ever be *non-credentialed* — which
/// is exactly what bearer auth makes it, so the objection no longer bites. The plugin
/// is still used in Rust for the two pre-auth enrolment calls, which have no token to
/// present and must not depend on the allowlist. Moving all traffic onto the plugin
/// stays a live option; it needs `@tauri-apps/plugin-http` inside `studio-web` and a
/// binary-body story for multipart uploads, so it is not worth doing blind.
fn initialization_script(api_base: &str, token: Option<&str>) -> String {
	let token_literal = match token {
		Some(t) => serde_json::to_string(t).unwrap_or_else(|_| "null".into()),
		None => "null".into(),
	};
	format!(
		r#"(() => {{
	let token = {token_literal};
	globalThis.__ANTHERS_DESKTOP__ = {{
		apiBaseUrl: {api_base},
		getToken: () => token,
		setToken: (t) => {{ token = t; }},
	}};
}})();"#,
		token_literal = token_literal,
		api_base = serde_json::to_string(api_base).unwrap_or_else(|_| "\"\"".into()),
	)
}

fn main() {
	let store = TokenStore::default();
	let token = store.load();
	let api_base = api_base_url();

	tauri::Builder::default()
		// Must be first: an OS deep-link activation can spawn a fresh process, and the
		// callback code must reach the instance that holds the PKCE verifier — a second
		// instance has no verifier and could only fail the exchange.
		.plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
			reveal_main_window(app);
			handle_urls(app, &argv);
		}))
		.plugin(tauri_plugin_http::init())
		// Spawns the bundled ffmpeg/ffprobe sidecars.
		.plugin(tauri_plugin_shell::init())
		// Lets the webview read finished variants back for upload, and pick a source
		// file by real path (no 300 MB in-memory ceiling).
		.plugin(tauri_plugin_fs::init())
		.plugin(tauri_plugin_dialog::init())
		.plugin(tauri_plugin_opener::init())
		.plugin(tauri_plugin_deep_link::init())
		.manage(store)
		.manage(PendingVerifier::default())
		.manage(PendingCode::default())
		.setup(move |app| {
			if let Some(window) = app.get_webview_window("main") {
				let _ = window.eval(&initialization_script(&api_base, token.as_deref()));
			}

			// In dev there is no installed bundle to own the scheme, so bind it to the
			// running dev binary so `xdg-open 'anthers://auth/callback?code=…'` works.
			// Release relies on the installer's association instead.
			#[cfg(debug_assertions)]
			{
				let _ = app.deep_link().register("anthers");
			}

			// Cold start (Linux/Windows carry the launch URL in argv) and live opens
			// (macOS, and re-deliveries) funnel through the same dispatch.
			if let Ok(Some(urls)) = app.deep_link().get_current() {
				let raws: Vec<String> = urls.iter().map(|u| u.as_str().to_string()).collect();
				handle_urls(app.handle(), &raws);
			}
			let handle = app.handle().clone();
			app.deep_link().on_open_url(move |event| {
				let raws: Vec<String> = event.urls().iter().map(|u| u.as_str().to_string()).collect();
				handle_urls(&handle, &raws);
			});
			Ok(())
		})
		.invoke_handler(tauri::generate_handler![
			session_state,
			begin_sign_in,
			complete_sign_in,
			take_pending_code,
			sign_out,
			encode_video,
			cleanup_encode
		])
		.run(tauri::generate_context!())
		.expect("error while running Anthers");
}
