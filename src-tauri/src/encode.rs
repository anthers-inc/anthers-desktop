// SPDX-License-Identifier: AGPL-3.0-or-later
//! Native video encoding with the bundled ffmpeg sidecar.
//!
//! This is the reason the desktop app is worth building. 44.00 § 2.4 gives on-device
//! transcoding "extra urgency" because transcode compute is one of the platform's
//! larger fixed early costs, and diffusing it across creators' own machines drives it
//! toward zero. The browser already encodes on device, but with two hard limits this
//! removes: `ffmpeg.wasm` is single-threaded per rung and capped at a 300 MB source,
//! and the creator is **tied to the tab** for the whole encode.
//!
//! ## Same ladder, same output contract
//!
//! The rungs, bitrates, keyframe interval and x264 settings match
//! `packages/web-shared/src/lib/transcode.ts` exactly, because the server's
//! `package-video` job remuxes these variants into HLS with `-c copy`. If the two
//! encoders drift, the browser path and the desktop path produce differently-segmented
//! ladders from the same source — so the args live here in one visible block rather
//! than being assembled cleverly.
//!
//! ## Why files rather than bytes
//!
//! The webview hands over a *path*, ffmpeg reads and writes *files*, and only the
//! finished variants cross back into JS. A 4 GB source is never held in memory by
//! anyone — which is exactly what the browser path cannot do.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};
use tauri_plugin_shell::process::CommandEvent;
use tauri_plugin_shell::ShellExt;

/// One rung of the ladder. Mirrors `VariantSpec` in the shared transcode module.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VariantSpec {
	pub name: String,
	pub height: u32,
	pub width: u32,
	pub bitrate: String,
	pub bandwidth: u64,
}

/// An encoded rung: its spec plus where the bytes landed on disk.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EncodedVariant {
	#[serde(flatten)]
	pub spec: VariantSpec,
	pub path: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EncodeResult {
	pub variants: Vec<EncodedVariant>,
	/// JPEG poster, or null when extraction failed (the server can still derive one).
	pub thumbnail_path: Option<String>,
	pub duration_seconds: f64,
	pub width: u32,
	pub height: u32,
	/// Where the temp outputs live, so the caller can clean up after uploading.
	pub work_dir: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct EncodeProgress {
	stage: String,
	percent: u32,
}

// ─── Probe ───────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct ProbeStream {
	width: Option<u32>,
	height: Option<u32>,
}

#[derive(Deserialize)]
struct ProbeFormat {
	duration: Option<String>,
}

#[derive(Deserialize)]
struct ProbeOutput {
	streams: Vec<ProbeStream>,
	format: ProbeFormat,
}

/// Read duration + intrinsic dimensions via the ffprobe sidecar.
pub async fn probe(app: &AppHandle, source: &Path) -> Result<(f64, u32, u32), String> {
	let output = app
		.shell()
		.sidecar("ffprobe")
		.map_err(|e| format!("ffprobe sidecar missing: {e}"))?
		.args([
			"-v",
			"error",
			"-select_streams",
			"v:0",
			"-show_entries",
			"stream=width,height",
			"-show_entries",
			"format=duration",
			"-of",
			"json",
			&source.to_string_lossy(),
		])
		.output()
		.await
		.map_err(|e| format!("Could not run ffprobe: {e}"))?;

	if !output.status.success() {
		return Err("That file doesn't look like a video ffmpeg can read.".into());
	}

	let parsed: ProbeOutput = serde_json::from_slice(&output.stdout)
		.map_err(|e| format!("Could not read the video's metadata: {e}"))?;

	let stream = parsed.streams.first().ok_or("That file has no video track.")?;
	let width = stream.width.unwrap_or(0);
	let height = stream.height.unwrap_or(0);
	if width == 0 || height == 0 {
		return Err("Could not read the video's dimensions.".into());
	}
	let duration = parsed.format.duration.and_then(|d| d.parse::<f64>().ok()).unwrap_or(0.0);

	Ok((duration, width, height))
}

// ─── Ladder ──────────────────────────────────────────────────────────────────

/// Source-gated rungs — never upscale. Mirrors `ladderFor()` in the shared module.
fn ladder_for(width: u32, height: u32) -> Vec<VariantSpec> {
	let mut rungs: Vec<(&str, u32, &str, u64)> = Vec::new();
	if height >= 1080 {
		rungs.push(("1080p", 1080, "5000k", 5_000_000));
	}
	if height >= 720 {
		rungs.push(("720p", 720, "2500k", 2_500_000));
	}
	rungs.push(("480p", 480, "1000k", 1_000_000));

	let aspect = width as f64 / height as f64;
	rungs
		.into_iter()
		.map(|(name, h, bitrate, bandwidth)| VariantSpec {
			name: name.to_string(),
			height: h,
			// Even width preserving aspect — matches the server's scale=-2:H behaviour.
			width: ((h as f64 * aspect / 2.0).round() * 2.0) as u32,
			bitrate: bitrate.to_string(),
			bandwidth,
		})
		.collect()
}

// ─── Encode ──────────────────────────────────────────────────────────────────

/// Encode one rung, reporting 0–1 progress via `on_fraction`.
///
/// One ffmpeg process per rung, run SEQUENTIALLY: unlike `ffmpeg.wasm` — which is
/// single-threaded, so the browser runs rungs concurrently to use more than one core —
/// native x264 already threads across every core, so a second concurrent encode would
/// contend rather than help.
async fn encode_variant(
	app: &AppHandle,
	source: &Path,
	spec: &VariantSpec,
	out: &Path,
	duration: f64,
	mut on_fraction: impl FnMut(f64),
) -> Result<(), String> {
	let bitrate_num: u32 = spec.bitrate.trim_end_matches('k').parse().unwrap_or(1000);
	let bufsize = format!("{}k", bitrate_num * 2);

	let (mut rx, _child) = app
		.shell()
		.sidecar("ffmpeg")
		.map_err(|e| format!("ffmpeg sidecar missing: {e}"))?
		.args([
			"-hide_banner",
			"-nostdin",
			"-y",
			"-i",
			&source.to_string_lossy(),
			"-vf",
			&format!("scale=-2:{}", spec.height),
			"-c:v",
			"libx264",
			"-preset",
			"veryfast",
			"-b:v",
			&spec.bitrate,
			"-maxrate",
			&spec.bitrate,
			"-bufsize",
			&bufsize,
			// Force a keyframe every 6s so the server can segment HLS on copy.
			"-force_key_frames",
			"expr:gte(t,n_forced*6)",
			"-c:a",
			"aac",
			"-b:a",
			"128k",
			"-movflags",
			"+faststart",
			// Machine-readable progress on stdout instead of parsing the human log.
			"-progress",
			"pipe:1",
			"-loglevel",
			"error",
			&out.to_string_lossy(),
		])
		.spawn()
		.map_err(|e| format!("Could not start ffmpeg: {e}"))?;

	let mut stderr_tail = String::new();
	while let Some(event) = rx.recv().await {
		match event {
			CommandEvent::Stdout(line) => {
				let text = String::from_utf8_lossy(&line);
				// `-progress` emits `out_time_us=<micros>` per update.
				for part in text.lines() {
					if let Some(us) = part.strip_prefix("out_time_us=") {
						if let Ok(us) = us.trim().parse::<f64>() {
							if duration > 0.0 {
								on_fraction((us / 1_000_000.0 / duration).clamp(0.0, 1.0));
							}
						}
					}
				}
			}
			CommandEvent::Stderr(line) => {
				// Keep only the tail: ffmpeg's error output is what the creator needs to
				// see, but the whole log would be noise.
				stderr_tail.push_str(&String::from_utf8_lossy(&line));
				if stderr_tail.len() > 2000 {
					let cut = stderr_tail.len() - 2000;
					stderr_tail = stderr_tail.split_off(cut);
				}
			}
			CommandEvent::Terminated(status) => {
				if status.code != Some(0) {
					return Err(format!(
						"Encoding {} failed.{}",
						spec.name,
						if stderr_tail.trim().is_empty() {
							String::new()
						} else {
							format!(" ffmpeg said: {}", stderr_tail.trim())
						}
					));
				}
				on_fraction(1.0);
				return Ok(());
			}
			_ => {}
		}
	}
	Err(format!("Encoding {} ended unexpectedly.", spec.name))
}

/// Extract a poster frame. Best-effort — a failure here must not fail the upload.
async fn extract_poster(app: &AppHandle, source: &Path, at: f64, out: &Path) -> Option<String> {
	let result = app
		.shell()
		.sidecar("ffmpeg")
		.ok()?
		.args([
			"-hide_banner",
			"-nostdin",
			"-y",
			"-ss",
			&format!("{at:.2}"),
			"-i",
			&source.to_string_lossy(),
			"-frames:v",
			"1",
			"-q:v",
			"3",
			"-loglevel",
			"error",
			&out.to_string_lossy(),
		])
		.output()
		.await
		.ok()?;
	if result.status.success() && out.exists() {
		Some(out.to_string_lossy().to_string())
	} else {
		None
	}
}

/// Encode a source video into the MP4 variant ladder + poster, natively.
pub async fn encode_ladder(app: AppHandle, source_path: String) -> Result<EncodeResult, String> {
	let source = PathBuf::from(&source_path);
	if !source.is_file() {
		return Err("That file no longer exists.".into());
	}

	let emit = |stage: &str, percent: u32| {
		let _ = app.emit("encode-progress", EncodeProgress { stage: stage.into(), percent });
	};

	emit("Reading video", 1);
	let (duration, width, height) = probe(&app, &source).await?;
	let specs = ladder_for(width, height);

	// A per-run temp directory, so a failed run never collides with the next and the
	// caller can delete the whole thing once uploads finish.
	let work_dir = std::env::temp_dir().join(format!("anthers-encode-{}", std::process::id()));
	std::fs::create_dir_all(&work_dir).map_err(|e| format!("Could not create a work folder: {e}"))?;

	let poster_path = extract_poster(
		&app,
		&source,
		(duration * 0.25).max(1.0),
		&work_dir.join("poster.jpg"),
	)
	.await;

	let mut variants = Vec::with_capacity(specs.len());
	let total = specs.len() as f64;
	for (i, spec) in specs.iter().enumerate() {
		let out = work_dir.join(format!("{}.mp4", spec.name));
		let label = format!("Encoding {} ({}/{})", spec.name, i + 1, specs.len());
		encode_variant(&app, &source, spec, &out, duration, |f| {
			// Map this rung's 0–1 onto the 3–95 band shared with the browser encoder.
			let overall = (i as f64 + f) / total;
			let _ = app.emit(
				"encode-progress",
				EncodeProgress { stage: label.clone(), percent: (3.0 + overall * 92.0) as u32 },
			);
		})
		.await?;

		variants.push(EncodedVariant {
			spec: spec.clone(),
			path: out.to_string_lossy().to_string(),
		});
	}

	emit("Finishing up", 96);
	Ok(EncodeResult {
		variants,
		thumbnail_path: poster_path,
		duration_seconds: duration,
		width,
		height,
		work_dir: work_dir.to_string_lossy().to_string(),
	})
}
