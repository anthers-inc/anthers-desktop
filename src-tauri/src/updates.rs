// SPDX-License-Identifier: Apache-2.0
//! The whole-application updater: check the release channel at startup, download in
//! the background, and apply at restart. This is the freshness mechanism Parker chose
//! for the shell (2026-10-13) — full signed installers through the bundled app,
//! the field's baseline — rather than a custom web-layer refresh protocol.
//!
//! Deliberately shell-owned and silent: the web app stays browser-shaped and never
//! learns about updates; the desktop window just restarts into the new version
//! whenever the next launch lands after an applied update. Release builds only — a
//! dev build must never self-update over the code the developer is editing.

use tauri::AppHandle;
use tauri_plugin_updater::UpdaterExt;

/// Check once, and if a newer release exists, download and install it.
///
/// `download_and_install` applies the update on this run's exit (Windows runs the
/// new installer passively; macOS and Linux move files so a relaunch picks them
/// up), which is the whole flow: the user closes the app at some point, the next
/// open is the new version. No user prompt, no in-app surface.
pub async fn auto_update(app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
	// A dev build must never self-update over the code the developer is editing;
	// the check runs in both profiles so neither one carries hidden dead code.
	if cfg!(debug_assertions) {
		return Ok(());
	}

	let update = app.updater()?.check().await?;
	let Some(update) = update else {
		return Ok(());
	};

	eprintln!(
		"Update available: {} → {} — downloading in the background",
		update.current_version, update.version
	);

	// The chunks stream straight to /dev/null today; if a progress surface ever
	// lands (a tray nudge, a window badge), the callback is its seam.
	update
		.download_and_install(|_chunk, _remaining| {}, || {})
		.await?;

	eprintln!(
		"Update {} installed — it applies at the next launch.",
		update.version
	);
	Ok(())
}
