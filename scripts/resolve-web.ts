// SPDX-License-Identifier: AGPL-3.0-or-later
//
// Put a built Anthers web app at ./web-dist, which is what Tauri bundles.
//
// This repository holds the desktop SHELL. The thing inside the window is the platform's
// web app, built from anthers-inc/anthers — the shell has no UI of its own and never has,
// even when the two lived in one repository. So the only real question here is where the
// build comes from, and there are three answers in priority order:
//
//   1. ANTHERS_WEB=/path/to/dist   — an explicit, already-built dist. Overrides everything.
//   2. a sibling ../Anthers checkout — built on demand. The working setup for local dev.
//   3. the pinned release asset     — downloaded. What CI and a clean machine use.
//
// 🚨 Why a RELEASE ASSET and not a package on GitHub Packages, which is the obvious
// choice and the wrong one: GitHub's own documentation says "You need an access token to
// publish, install, and delete private, internal, and public packages." Authentication is
// required even for PUBLIC packages, so a package would mean every contributor and every
// CI run needed a token to install a public dependency of a public, AGPL project. A
// release asset downloads anonymously. Verified both ways before choosing.
//
// The sibling-checkout path is what keeps the split cheap. Iterating on the web app and
// the shell together stays `build web → run desktop`, with no publish step in between —
// the same shape `BRAND_SOURCE` uses for the icon library in the platform repo.

import { existsSync, rmSync, mkdirSync } from "node:fs";
import { cp, readFile } from "node:fs/promises";
import { join, resolve } from "node:path";

const ROOT = join(import.meta.dir, "..");
const DEST = join(ROOT, "web-dist");
const SIBLING = resolve(ROOT, "..", "Anthers");

/** The platform release this shell is built against. Bump deliberately. */
const pinned = JSON.parse(await readFile(join(ROOT, "package.json"), "utf8")).anthersWeb as
	| string
	| undefined;

function place(from: string, how: string) {
	rmSync(DEST, { recursive: true, force: true });
	mkdirSync(DEST, { recursive: true });
	return cp(from, DEST, { recursive: true }).then(() => {
		console.log(`[web] ${how}\n[web] → ${DEST}`);
	});
}

// ── 1. explicit ──────────────────────────────────────────────────────────────
const explicit = process.env.ANTHERS_WEB;
if (explicit) {
	const from = resolve(explicit);
	if (!existsSync(from)) {
		console.error(`[web] ANTHERS_WEB is set to ${from}, which does not exist.`);
		process.exit(1);
	}
	await place(from, `using ANTHERS_WEB (${from})`);
	process.exit(0);
}

// ── 2. sibling checkout ──────────────────────────────────────────────────────
const siblingWeb = join(SIBLING, "apps", "web");
if (existsSync(siblingWeb)) {
	console.log(`[web] building from the sibling checkout at ${SIBLING}`);
	const build = Bun.spawnSync(["bun", "run", "build"], {
		cwd: siblingWeb,
		stdout: "inherit",
		stderr: "inherit",
	});
	if (build.exitCode !== 0) {
		console.error("[web] the sibling build failed — fix it there, then run this again.");
		process.exit(1);
	}
	await place(join(siblingWeb, "dist"), "built from the sibling checkout");
	process.exit(0);
}

// ── 3. the pinned release asset ──────────────────────────────────────────────
if (!pinned) {
	console.error(
		"[web] No web build available, and no release pinned.\n" +
			"      Do one of:\n" +
			`        • clone the platform beside this repo:  git clone https://github.com/anthers-inc/anthers.git ${SIBLING}\n` +
			"        • point at an existing build:           ANTHERS_WEB=/path/to/apps/web/dist\n" +
			'        • pin a release in package.json:        "anthersWeb": "web-v0.1.0"',
	);
	process.exit(1);
}

const url = `https://github.com/anthers-inc/anthers/releases/download/${pinned}/web-dist.tar.gz`;
console.log(`[web] downloading ${pinned} …`);
const res = await fetch(url);
if (!res.ok) {
	console.error(
		`[web] ${res.status} fetching ${url}\n` +
			"      Either that release has no web-dist.tar.gz asset, or the pin in package.json\n" +
			"      names a release that does not exist.",
	);
	process.exit(1);
}
rmSync(DEST, { recursive: true, force: true });
mkdirSync(DEST, { recursive: true });
const tar = join(ROOT, ".web-dist.tar.gz");
await Bun.write(tar, await res.arrayBuffer());
const untar = Bun.spawnSync(["tar", "xzf", tar, "-C", DEST, "--strip-components=1"], {
	stdout: "inherit",
	stderr: "inherit",
});
rmSync(tar, { force: true });
if (untar.exitCode !== 0) {
	console.error("[web] could not unpack the downloaded archive.");
	process.exit(1);
}
console.log(`[web] unpacked ${pinned} → ${DEST}`);
