// SPDX-License-Identifier: AGPL-3.0-or-later
//! Where the desktop session token lives at rest.
//!
//! 42.06 left this open between the OS keychain and `tauri-plugin-store` (a plaintext
//! JSON file). This picks the keychain — Secret Service on Linux, Keychain on macOS,
//! Credential Manager on Windows — because the token is a full session credential with
//! a 30-day life, and a plaintext file is readable by anything running as the user.
//!
//! The fallback matters as much as the choice. A Linux box with no Secret Service
//! provider (a bare WM, a container, some CI) has no keychain to write to. Rather than
//! fail the sign-in or silently drop to plaintext, the token is held **in memory for
//! the life of the process** and the app reports that the session won't persist. Losing
//! a session across restarts is an annoyance; writing a credential to disk that the
//! user believes is protected is a lie.

use std::sync::Mutex;

const SERVICE: &str = "org.anthers.desktop";
const ACCOUNT: &str = "session";

/// Whether the token survives a restart, so the UI can be honest about it.
#[derive(Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Persistence {
	/// Stored in the OS keychain.
	Keychain,
	/// No keychain available — memory only, gone when the app exits.
	MemoryOnly,
}

#[derive(Default)]
pub struct TokenStore {
	/// Always the authoritative in-process copy; the keychain is the durable mirror.
	cached: Mutex<Option<String>>,
}

fn entry() -> Option<keyring::Entry> {
	keyring::Entry::new(SERVICE, ACCOUNT).ok()
}

impl TokenStore {
	/// Load the token at boot: memory first, then the keychain.
	pub fn load(&self) -> Option<String> {
		if let Ok(cached) = self.cached.lock() {
			if cached.is_some() {
				return cached.clone();
			}
		}
		let found = entry().and_then(|e| e.get_password().ok());
		if let (Some(token), Ok(mut cached)) = (found.clone(), self.cached.lock()) {
			*cached = Some(token);
		}
		found
	}

	/// Persist a freshly enrolled token, reporting whether it will actually survive.
	pub fn save(&self, token: &str) -> Persistence {
		if let Ok(mut cached) = self.cached.lock() {
			*cached = Some(token.to_string());
		}
		match entry().map(|e| e.set_password(token)) {
			Some(Ok(())) => Persistence::Keychain,
			_ => Persistence::MemoryOnly,
		}
	}

	/// Forget the token everywhere. Signing out must not leave a usable credential
	/// behind just because the keychain delete failed, so memory is cleared first.
	pub fn clear(&self) {
		if let Ok(mut cached) = self.cached.lock() {
			*cached = None;
		}
		if let Some(e) = entry() {
			let _ = e.delete_credential();
		}
	}
}
