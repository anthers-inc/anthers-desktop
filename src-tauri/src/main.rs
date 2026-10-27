// SPDX-License-Identifier: Apache-2.0
// Prevents an extra console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Anthers — the desktop shell around the whole Anthers app.
//!
//! It bundles the SAME built web app the browser serves, so the whole platform is
//! in the window — reading, watching and creating. This is a third consumer of
//! `@anthers/web-shared`, not a fork. What the shell adds is the things a browser
//! tab cannot do: a session that survives without cookies, a real window, a
//! custom-scheme deep link, and a silent whole-app updater. Everything else inside
//! it is the web experience, unchanged — that one-to-one shape is the shell's
//! contract, and `tauri.conf.json`'s `externalBin` must stay absent.
//!
//! ## Why the window is not just pointed at anthers.org
//!
//! `invoke` is unavailable to remote documents, so native capability would mean
//! opening the IPC boundary to a page served over the internet — an XSS on the site
//! would then reach filesystem and process spawn on every user's machine. The SPA
//! is bundled instead, which costs the auth work below. The README's auth section
//! carries the whole design.
//!
//! ## Auth
//!
//! Serving from `tauri://localhost` means no session cookie is ever sent, so the app
//! carries an `Authorization: Bearer` token instead. The DEFAULT method signs in inside
//! the app: the person types the handle or address on the identity Anthers hosts and the
//! password they chose for it, which `/desktop/password` spends in one request and turns
//! into the same authorization step the browser path uses. That is the first-party door
//! the website's own login is, and the password is never stored — the session is.
//! The SYSTEM-browser handoff stays for Bluesky, whose OAuth cannot complete inside this
//! webview: the app opens the authorize page there, the person confirms, and a one-time
//! code comes back over the `anthers://` scheme. PKCE binds both methods' two halves —
//! the verifier never leaves this process, so another local app that hijacks the scheme
//! and steals a callback code cannot redeem it.
//!
//! ## How the webview learns any of this
//!
//! `initialization_script` runs before any app JS, defining
//! `globalThis.__ANTHERS_DESKTOP__` = `{ apiBaseUrl, getToken(), fetch }` — the seam
//! `apiFetch()` in `@anthers/web-shared` already reads. `fetch` is routed through
//! `tauri-plugin-http`, so requests leave from Rust and CORS never applies.

mod token;
mod updates;

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

/// The PKCE verifier for the enrollment currently in flight. Never leaves this process
/// — that is precisely what makes a stolen callback code useless. Both sign-in methods
/// (the in-app emailed code and the browser handoff) park their verifier here; one is
/// in flight at a time, and a second method's start simply replaces it.
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
	hasher
		.finalize()
		.iter()
		.map(|b| format!("{b:02x}"))
		.collect()
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
	SessionState {
		token: store.load(),
		api_base_url: api_base_url(),
	}
}

/// Open an enrollment: mint a PKCE verifier and register its challenge with the API.
/// Shared by both sign-in methods — the browser handoff and the in-app emailed code —
/// which is why the opener is not in here. Returns the challenge; the verifier stays
/// in `pending`.
async fn enroll(pending: &PendingVerifier, label: &str) -> Result<String, String> {
	let verifier = random_hex();
	let challenge = sha256_hex(&verifier);

	let base = api_base_url();
	let res = post_json(
		&format!("{base}/api/auth/desktop/start"),
		&serde_json::json!({ "challenge": challenge, "label": label }),
	)
	.await?;
	if !res.status().is_success() {
		return Err(format!(
			"Anthers refused the sign-in request ({})",
			res.status()
		));
	}

	// Only store the verifier once the server has accepted its challenge, so a failed
	// start can't leave a stale verifier that a later callback would try to use.
	if let Ok(mut slot) = pending.0.lock() {
		*slot = Some(verifier);
	}

	Ok(challenge)
}

/// Begin the browser handoff: enrol, then open the authorize page in the system
/// browser. (The in-app method enrols through `start_password_sign_in` and keeps the
/// browser closed.)
///
/// Returns the URL that was opened so the UI can offer it as copyable text — on a
/// machine with no default browser handler, "click here" is otherwise a dead end.
#[tauri::command]
async fn begin_sign_in(
	pending: tauri::State<'_, PendingVerifier>,
	label: String,
) -> Result<String, String> {
	let challenge = enroll(&pending, &label).await?;

	// The authorize page is served by the consumer SPA, not the API — see web_base_url().
	let url = format!("{}/desktop/authorize?challenge={challenge}", web_base_url());
	tauri_plugin_opener::open_url(&url, None::<&str>)
		.map_err(|e| format!("Could not open your browser: {e}"))?;
	Ok(url)
}

/// Begin the in-app password method: enrol the way the handoff does, and open no
/// browser. The flow continues with `password_sign_in` in this window.
#[tauri::command]
async fn start_password_sign_in(
	pending: tauri::State<'_, PendingVerifier>,
	label: String,
) -> Result<(), String> {
	enroll(&pending, &label).await?;
	Ok(())
}

/// The verifier of the enrollment in flight and its challenge — for a code request,
/// which must not consume the verifier: the redemption still needs it.
fn in_flight(pending: &PendingVerifier) -> Result<(String, String), String> {
	let verifier = pending
		.0
		.lock()
		.ok()
		.and_then(|slot| slot.clone())
		.ok_or("This sign-in didn't start here. Try signing in again.")?;
	let challenge = sha256_hex(&verifier);
	Ok((verifier, challenge))
}

/// What a sign-in attempt answered, in the shape the webview switches on.
#[derive(Serialize)]
#[serde(rename_all = "camelCase", tag = "status")]
enum SignInOutcome {
	/// The password was right and the API authorized the enrollment; `token` is in the store.
	Session {
		token: String,
		persistence: Persistence,
	},
	/// The password was right and the identity server wants the emailed code as well.
	TwoFactor,
}

/// Sign in with the identity's own password: authorize the enrollment in flight, then
/// redeem the result the same way the browser handoff spends its callback.
///
/// ⚠️ **The password is not kept.** It goes into one request body and the process lets
/// it drop; what survives is the session the API mints, which is what the keychain holds.
#[tauri::command]
async fn password_sign_in(
	pending: tauri::State<'_, PendingVerifier>,
	store: tauri::State<'_, TokenStore>,
	identifier: String,
	password: String,
	auth_factor_token: Option<String>,
) -> Result<SignInOutcome, String> {
	let (_, challenge) = in_flight(&pending)?;

	let mut body = serde_json::json!({
		"challenge": challenge,
		"identifier": identifier,
		"password": password,
	});
	if let Some(token) = auth_factor_token.filter(|t| !t.trim().is_empty()) {
		if let Some(obj) = body.as_object_mut() {
			obj.insert("authFactorToken".into(), serde_json::Value::String(token));
		}
	}

	let base = api_base_url();
	let res = post_json(&format!("{base}/api/auth/desktop/password"), &body).await?;
	if !res.status().is_success() {
		// A missing second factor is not a failure to report — it is the next step, and the
		// app must keep its password field on screen rather than sending the person back to the
		// start of a flow whose enrollment is still live.
		let text = res.text().await.unwrap_or_default();
		let parsed: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
		if parsed.get("twoFactor").and_then(|v| v.as_bool()) == Some(true) {
			return Ok(SignInOutcome::TwoFactor);
		}
		// The API's own line when it sent one: it speaks one voice for a wrong password, an
		// unknown identifier and a deactivated account, and a separate one for a suspension or
		// an outage. Each is better surfaced than wrapped in a shell generic.
		return Err(parsed
			.get("error")
			.and_then(|e| e.as_str())
			.map(str::to_string)
			.unwrap_or_else(|| "That sign-in didn't go through. Try again.".into()));
	}

	let text = res
		.text()
		.await
		.map_err(|e| format!("Unexpected reply from Anthers: {e}"))?;
	let parsed: serde_json::Value =
		serde_json::from_str(&text).map_err(|e| format!("Unexpected reply from Anthers: {e}"))?;
	let code = parsed
		.get("code")
		.and_then(|c| c.as_str())
		.ok_or("Anthers did not return a sign-in code.")?;

	let result = redeem_and_store(&pending, &store, code.to_string()).await?;
	Ok(SignInOutcome::Session {
		token: result.token,
		persistence: result.persistence,
	})
}

/// Redeem a code for the session token, and persist it — the shared tail of both
/// sign-in methods. Consumes the enrollment's verifier: a code is single-use, and so
/// is the enrollment behind it.
async fn redeem_and_store(
	pending: &PendingVerifier,
	store: &TokenStore,
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

	let text = res
		.text()
		.await
		.map_err(|e| format!("Unexpected reply from Anthers: {e}"))?;
	let body: serde_json::Value =
		serde_json::from_str(&text).map_err(|e| format!("Unexpected reply from Anthers: {e}"))?;
	let token = body
		.get("token")
		.and_then(|t| t.as_str())
		.ok_or("Anthers did not return a session.")?;

	let persistence = store.save(token);
	Ok(SignInResult {
		token: token.to_string(),
		persistence,
	})
}

/// The browser handoff's step 4: the callback code that arrived over `anthers://`.
#[tauri::command]
async fn complete_sign_in(
	pending: tauri::State<'_, PendingVerifier>,
	store: tauri::State<'_, TokenStore>,
	code: String,
) -> Result<SignInResult, String> {
	redeem_and_store(&pending, &store, code).await
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

// ─── Entry ───────────────────────────────────────────────────────────────────

/// Script evaluated before any app JS, defining the seam `@anthers/web-shared`'s
/// `apiFetch()` reads. The token is injected at boot and refreshed by the app after
/// enrollment via `__ANTHERS_DESKTOP__.setToken`.
///
/// No `fetch` is supplied, so `apiFetch()` uses the webview's own — a plain
/// cross-origin request from `tauri://localhost` carrying a bearer header and NO
/// cookie, which the API's CORS allowlist admits. 42.06 preferred routing everything
/// through `tauri-plugin-http` (fetch from Rust, CORS never applies); the objection it
/// raised to the CORS route was that it could only ever be *non-credentialed* — which
/// is exactly what bearer auth makes it, so the objection no longer bites. The plugin
/// is still used in Rust for the two pre-auth enrollment calls, which have no token to
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
		.plugin(tauri_plugin_opener::init())
		.plugin(tauri_plugin_deep_link::init())
		// The whole-app updater: release builds check the channel and apply at restart.
		.plugin(tauri_plugin_updater::Builder::new().build())
		.manage(store)
		.manage(PendingVerifier::default())
		.manage(PendingCode::default())
		.setup(move |app| {
			if let Some(window) = app.get_webview_window("main") {
				let _ = window.eval(initialization_script(&api_base, token.as_deref()));
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
				let raws: Vec<String> = event
					.urls()
					.iter()
					.map(|u| u.as_str().to_string())
					.collect();
				handle_urls(&handle, &raws);
			});

			// Spawned so the check and download never block startup; a failure here is
			// a log line, never a crash. The function itself no-ops on dev builds.
			{
				let handle = app.handle().clone();
				tauri::async_runtime::spawn(async move {
					if let Err(err) = updates::auto_update(&handle).await {
						eprintln!("Updater check failed (not fatal): {err}");
					}
				});
			}
			Ok(())
		})
		.invoke_handler(tauri::generate_handler![
			session_state,
			begin_sign_in,
			start_password_sign_in,
			password_sign_in,
			complete_sign_in,
			take_pending_code,
			sign_out
		])
		.run(tauri::generate_context!())
		.expect("error while running Anthers");
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn callback_code_reads_the_query_from_a_standard_callback_url() {
		assert_eq!(
			callback_code("anthers://auth/callback?code=abc123"),
			Some("abc123".into())
		);
	}

	#[test]
	fn callback_code_does_not_insist_on_the_platforms_host_path_split() {
		// Custom-scheme URL parsing differs per platform — some put `auth` in the host,
		// which here changes only what precedes the query; every anthers:// spelling
		// that carries the query must yield the code.
		assert_eq!(
			callback_code("anthers://callback?code=abc123"),
			Some("abc123".into())
		);
		assert_eq!(
			callback_code("anthers:auth/callback?code=abc123"),
			Some("abc123".into())
		);
	}

	#[test]
	fn callback_code_rejects_other_schemes_and_codeless_callbacks() {
		assert_eq!(
			callback_code("https://anthers.org/desktop/authorize?code=abc123"),
			None
		);
		assert_eq!(callback_code("anthers://auth/callback"), None);
		assert_eq!(callback_code("not a url"), None);
	}

	#[test]
	fn the_conf_paths_resolve_from_the_bases_the_readme_warns_about() {
		// The README's config-path warning: `frontendDist` and `licenseFile` resolve
		// from src-tauri/, while the before-commands run from the app dir — a wrong
		// number of `../` breaks only at a real `tauri build` and never at `cargo
		// check`. This asserts each target exists at the base its setting resolves
		// from, so a path edit that looks right again cannot sit broken.
		let conf_text =
			std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tauri.conf.json"))
				.unwrap();
		let conf: serde_json::Value = serde_json::from_str(&conf_text).unwrap();
		let src_tauri = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
		let dist = conf["build"]["frontendDist"].as_str().unwrap();
		let license = conf["bundle"]["licenseFile"].as_str().unwrap();
		assert!(src_tauri.join(dist).join("index.html").is_file());
		assert!(src_tauri.join(license).is_file());
		// The before-commands run from the app dir and name only repo-local scripts.
		for cmd in [
			conf["build"]["beforeDevCommand"].as_str().unwrap(),
			conf["build"]["beforeBuildCommand"].as_str().unwrap(),
		] {
			assert!(
				cmd.starts_with("bun "),
				"before-command must run under bun, got {cmd}"
			);
		}
	}

	#[test]
	fn the_shell_carries_no_native_sidecars() {
		// The shell's contract is the one-to-one web experience: it bundles the web
		// app and adds only the session handoff, the deep link, the keychain token
		// and the updater. tauri-build validates `bundle.externalBin` at compile
		// time, so a sidecar sneaking back into the config fails here before CI —
		// and if native capability returns for a desktop feature, removing this
		// assertion belongs in that decision's diff, beside the capability itself.
		let conf_text =
			std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tauri.conf.json"))
				.unwrap();
		let conf: serde_json::Value = serde_json::from_str(&conf_text).unwrap();
		assert!(
			conf["bundle"]["externalBin"].is_null(),
			"the wrap-only shell bundles no external binaries"
		);
		for cmd in [
			conf["build"]["beforeDevCommand"].as_str().unwrap(),
			conf["build"]["beforeBuildCommand"].as_str().unwrap(),
		] {
			assert!(
				!cmd.contains("sidecar") && !cmd.contains("fetch-ffmpeg"),
				"before-commands resolve only the web app, got {cmd}"
			);
		}
	}
}
