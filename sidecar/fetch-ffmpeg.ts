// SPDX-License-Identifier: Apache-2.0
/**
 * Fetch the static ffmpeg + ffprobe binaries the desktop Studio bundles as Tauri
 * sidecars, naming them with the Rust target triple Tauri's `externalBin` expects
 * (`ffmpeg-x86_64-unknown-linux-gnu`, …).
 *
 * They are NOT committed — ~80 MB each per platform would dominate the repo. This
 * runs from `beforeBuildCommand`, and is idempotent, so a rebuild costs nothing once
 * they're present.
 *
 * ## Licensing (read before changing the source URLs)
 *
 * These are **GPL** builds, because H.264 encoding needs libx264 and that is what
 * makes a build GPL rather than LGPL. Anthers is Apache-2.0, which is compatible
 * with GPL-3.0, and ffmpeg ships here as a **separate executable invoked as a
 * subprocess** — not linked into our binary. The obligation that follows is to pass
 * the licence along and to say where the corresponding source is; both live in
 * `THIRD-PARTY.md` beside this script. If you ever swap in an LGPL build to avoid
 * that, you also lose libx264 and the whole feature with it.
 */

const VERSION_PIN = "7.1"; // The ffmpeg release these URLs are expected to yield.

interface Source {
	/** Rust target triple — the suffix Tauri resolves a sidecar by. */
	triple: string;
	url: string;
	/** Where the binaries sit inside the downloaded archive. */
	kind: "tar.xz" | "zip";
}

/**
 * Where each platform's static build comes from. Keep the licence note above in sync.
 *
 * Linux uses johnvansickle rather than BtbN deliberately: both are GPL builds with
 * libx264, but johnvansickle's are roughly half the size (77 MB vs 139 MB per binary),
 * and these ship inside every installer twice over.
 */
const SOURCES: Record<string, Source> = {
	"linux-x64": {
		triple: "x86_64-unknown-linux-gnu",
		url: "https://johnvansickle.com/ffmpeg/releases/ffmpeg-release-amd64-static.tar.xz",
		kind: "tar.xz",
	},
	"linux-arm64": {
		triple: "aarch64-unknown-linux-gnu",
		url: "https://johnvansickle.com/ffmpeg/releases/ffmpeg-release-arm64-static.tar.xz",
		kind: "tar.xz",
	},
	"win32-x64": {
		triple: "x86_64-pc-windows-msvc",
		url: "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl.zip",
		kind: "zip",
	},
	"darwin-x64": {
		triple: "x86_64-apple-darwin",
		url: "https://evermeet.cx/ffmpeg/getrelease/zip",
		kind: "zip",
	},
	"darwin-arm64": {
		triple: "aarch64-apple-darwin",
		url: "https://www.osxexperts.net/ffmpeg711arm.zip",
		kind: "zip",
	},
};

import { chmod, copyFile, mkdir, rm } from "node:fs/promises";
import { join, resolve } from "node:path";

function hostKey(): string {
	const platform = process.platform;
	const arch = process.arch === "arm64" ? "arm64" : "x64";
	return `${platform}-${arch}`;
}

// Resolved with node:path, not `new URL(...).pathname`: on Windows that yields a
// POSIX-shaped path (`/C:/Users/…`) that curl cannot write to, which is how the
// first Windows run died with curl exit 23.
const BIN_DIR = resolve(import.meta.dir, "../src-tauri/binaries");

async function exists(path: string): Promise<boolean> {
	return await Bun.file(path).exists();
}

async function run(cmd: string[], cwd?: string): Promise<void> {
	const proc = Bun.spawn(cmd, { cwd, stdout: "inherit", stderr: "inherit" });
	const code = await proc.exited;
	if (code !== 0) throw new Error(`${cmd[0]} exited ${code}`);
}

async function main() {
	const key = hostKey();
	const source = SOURCES[key];
	if (!source) {
		console.error(`No ffmpeg sidecar source configured for ${key}.`);
		console.error(`Add one to SOURCES in ${import.meta.url}.`);
		process.exit(1);
	}

	const exeSuffix = process.platform === "win32" ? ".exe" : "";
	// Namespaced on purpose. Tauri installs Linux sidecars into /usr/bin under their
	// plain name, so shipping one called `ffmpeg` makes our .deb/.rpm collide with the
	// distro's own ffmpeg package — dpkg refuses the install outright ("trying to
	// overwrite '/usr/bin/ffmpeg', which is also in package ffmpeg"). `name` is what we
	// fetch from the archive; `binary` is what we ship as.
	const targets = [
		{ name: "ffmpeg", binary: "anthers-ffmpeg" },
		{ name: "ffprobe", binary: "anthers-ffprobe" },
	].map((t) => ({
		...t,
		dest: join(BIN_DIR, `${t.binary}-${source.triple}${exeSuffix}`),
	}));

	if ((await Promise.all(targets.map((t) => exists(t.dest)))).every(Boolean)) {
		console.log(`ffmpeg sidecars already present for ${source.triple} — nothing to do.`);
		return;
	}

	await mkdir(BIN_DIR, { recursive: true });
	const work = join(BIN_DIR, ".download");
	await rm(work, { recursive: true, force: true });
	await mkdir(work, { recursive: true });

	console.log(`Fetching ffmpeg (${VERSION_PIN}-class GPL static) for ${source.triple}…`);
	const archive = join(work, "archive");
	await run(["curl", "-fsSL", "-o", archive, source.url]);

	// Extracted whole rather than with --strip-components, because the layouts differ
	// per source (and change upstream); the Glob below locates the binaries wherever
	// they land, which is one less thing to keep in sync. Zip archives ride `tar`,
	// not `unzip`: `tar` is BSD's on both darwin and Windows runners and reads zip
	// natively, while `unzip` is a GNU-side extra.
	if (source.kind === "tar.xz") {
		await run(["tar", "-xJf", archive, "-C", work]);
	} else {
		await run(["tar", "-xf", archive, "-C", work]);
	}

	for (const t of targets) {
		// Located wherever it landed in the extracted tree — upstream layouts differ
		// and change; macOS's Apple-double `__MACOSX` metadata entries don't match.
		const glob = new Bun.Glob(`**/${t.name}${exeSuffix}`);
		const src = glob.scanSync({ cwd: work, onlyFiles: true }).next().value;
		if (!src) {
			throw new Error(
				`${t.name} not found in the downloaded archive for ${source.triple}. ` +
					`The upstream layout may have changed — check ${source.url}.`,
			);
		}
		await copyFile(join(work, src), t.dest);
		// A no-op on Windows, where the executable bit doesn't exist.
		if (process.platform !== "win32") {
			await chmod(t.dest, 0o755);
		}
		console.log(`  → ${t.dest}`);
	}

	await rm(work, { recursive: true, force: true });
	console.log("ffmpeg sidecars ready.");
}

await main();
