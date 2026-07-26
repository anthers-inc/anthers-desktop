// SPDX-License-Identifier: AGPL-3.0-or-later
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
 * makes a build GPL rather than LGPL. Anthers is AGPL-3.0-or-later, which is
 * GPL-compatible, and ffmpeg ships here as a **separate executable invoked as a
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

function hostKey(): string {
	const platform = process.platform;
	const arch = process.arch === "arm64" ? "arm64" : "x64";
	return `${platform}-${arch}`;
}

const BIN_DIR = new URL("../src-tauri/binaries/", import.meta.url).pathname;

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
	const targets = ["ffmpeg", "ffprobe"].map((name) => ({
		name,
		dest: `${BIN_DIR}${name}-${source.triple}${exeSuffix}`,
	}));

	if ((await Promise.all(targets.map((t) => exists(t.dest)))).every(Boolean)) {
		console.log(`ffmpeg sidecars already present for ${source.triple} — nothing to do.`);
		return;
	}

	await run(["mkdir", "-p", BIN_DIR]);
	const work = `${BIN_DIR}.download`;
	await run(["rm", "-rf", work]);
	await run(["mkdir", "-p", work]);

	console.log(`Fetching ffmpeg (${VERSION_PIN}-class GPL static) for ${source.triple}…`);
	const archive = `${work}/archive`;
	await run(["curl", "-fsSL", "-o", archive, source.url]);

	// Extracted whole rather than with --strip-components, because the layouts differ
	// per source (and change upstream); the `find` below locates the binaries wherever
	// they land, which is one less thing to keep in sync.
	if (source.kind === "tar.xz") {
		await run(["tar", "-xJf", archive, "-C", work]);
	} else {
		await run(["unzip", "-q", "-o", archive, "-d", work]);
	}

	for (const t of targets) {
		// The archive layouts differ; find the binary wherever it landed.
		const found = Bun.spawnSync([
			"find",
			work,
			"-type",
			"f",
			"-name",
			`${t.name}${exeSuffix}`,
			"-print",
			"-quit",
		]);
		const src = found.stdout.toString().trim();
		if (!src) {
			throw new Error(
				`${t.name} not found in the downloaded archive for ${source.triple}. ` +
					`The upstream layout may have changed — check ${source.url}.`,
			);
		}
		await run(["cp", src, t.dest]);
		await run(["chmod", "+x", t.dest]);
		console.log(`  → ${t.dest}`);
	}

	await run(["rm", "-rf", work]);
	console.log("ffmpeg sidecars ready.");
}

await main();
