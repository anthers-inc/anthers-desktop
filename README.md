# Anthers Desktop

The desktop app for [Anthers](https://github.com/anthers-inc/anthers) — the whole
platform in a native window.

**This repository is the shell, and there is no UI in it.** The thing inside the window
is the platform's web app, bundled whole; what the shell adds is everything a browser
tab cannot do — a session that survives without cookies, a custom-scheme deep link, a
silent whole-app updater, and a real application window. Everything else inside it is
the web experience, unchanged: the one-to-one shape is the shell's contract.

The two lived in one repository until 2026-08-14, and separated because the toolchains
have nothing in common: this one is cargo, a three-OS build matrix, code signing and
installers, on a release cadence unrelated to the platform's.

**It is not a creator's tool only — and was never meant to be.** Bundling the whole
web app already puts reading, watching and playing in the window; the plan for the
desktop is the full-spectrum replacement that implies, with desktop-improved surfaces
on both sides when they are: for creators, the upload pack — which may bring native
media handling back with it, built as a decision rather than bundled on spec; for
users, locally-downloaded Library content is the first planned addition.

## Getting the web app

`./web-dist` is what Tauri bundles, and `scripts/resolve-web.ts` fills it from the first
source that works:

1. **`ANTHERS_WEB=/path/to/dist`** — an explicit, already-built dist. Overrides all else.
2. **A sibling `../Anthers` checkout** — built on demand. The local-dev path, and what
   keeps working across two repositories cheap: iterating on the web app and the shell
   together stays `build web → run desktop`, with no publish step in between.
3. **The pinned release asset** — downloaded from the platform's releases. What CI and a
   clean machine use. The pin is `anthersWeb` in `package.json`, naming a promoted
   platform release tag (`v2026.10.23`) — the build production is verified live on;
   there is no separate web-release ceremony. `ANTHERS_WEB_SOURCE=asset` forces this
   source where a sibling checkout also exists — which is how the pin itself is exercised.

🚨 **Why a release asset and not a package on GitHub Packages**, which is the obvious
choice and the wrong one: GitHub's documentation says *"You need an access token to
publish, install, and delete private, internal, and public packages."* Authentication is
required even for **public** packages — so a package would mean every contributor and
every CI run needing a token to install a public dependency of a public, open-source project. A
release asset downloads anonymously.

### The signed web build

The platform's `release-web` workflow signs the archive it attaches (a plain Ed25519
signature; the format's owner is that repository's `scripts/sign-web-dist.ts`), and the
pinned download is **verified before it is installed, failing closed**: `scripts/web-sig.ts`
refuses an archive whose `.sig` is missing, malformed, or wrong for the key committed at
`scripts/web-dist-signing.pub`. The signature ties a download to a key CI holds rather
than to GitHub's channel, so a compromised release pipeline cannot hand the shell a
tampered bundle. A key rotation is a replacement of the secret *and* of the `.pub` file
here — a single active key; the verifier accepts nothing the rotated key did not sign.

## Running it

```sh
git clone https://github.com/anthers-inc/anthers.git ../Anthers   # source 2, the easy path
bun install
bun run dev      # resolve the web app, open the window
```

Point it at a running local API with the platform's `make dev` in another terminal.

| Command | What it does |
|---|---|
| `bun run dev` | Run against the local dev API |
| `bun run web` | Resolve the web app into `./web-dist` and stop |
| `bun run check` | `cargo check` the Rust shell — no installers, no downloads |
| `bun run build` | Installers for **this** platform (no cross-compilation) |

Every command above has a make target, and the make tier carries what the package scripts
do not: `make verify` runs `ci.yml`'s step list in the order CI runs it, the `build-*`
targets refuse on the wrong OS rather than failing inside a toolchain, and
`make build-macos` and `make notarize` are the signing flow further down. `make help`
lists the rest.

Debug builds point at `http://localhost:8000`; release builds at `https://anthers.org`.
Override either with `ANTHERS_API_BASE` at build time.

The desktop shell's own architecture, the auth design and the decisions behind the
window live here and on the wiki's codebase page (**80.00 The Codebase**) and the
Anthers Desktop lane of the roadmap — the older `42.06 Creator Studio Architecture`
pointer in this README outlived the vault reorganization that dissolved that page.

## Why the window isn't just pointed at anthers.org

`invoke` is unavailable to remote documents, so native capability would mean opening
the IPC boundary to a page served over the internet. An XSS on the site would then
reach filesystem and process spawn on every user's machine — unacceptable for an app
whose whole point is to be the full Anthers experience on the machine. So the SPA is
bundled, which is what makes the auth work below necessary.

## Auth, in one paragraph

Serving from `tauri://localhost` means the `.anthers.org` session cookie is never sent,
so the app carries an `Authorization: Bearer` token instead — the same opaque session
row, in a different envelope. Signing in happens in **the person's own browser**, at
whichever server holds their identity, because that server is the only party an OAuth
grant may be given by and a password typed into this window would be a second host
asking for a credential it is not the authority on. The app enrols, opens the authorize
page in the system browser, the person confirms once, and a one-time code returns over
the `anthers://` scheme. **PKCE** binds the two halves — the verifier never leaves this
process, so another local app that hijacks the scheme and steals a callback code cannot
redeem it. The token is independently revocable from Settings → Devices on anthers.org.

The webview learns all of this from an `initialization_script` that defines
`globalThis.__ANTHERS_DESKTOP__` before any app JS — the seam `apiFetch()` in
`@anthers/web-shared` already reads.

### Token at rest

The OS keychain (Secret Service / Keychain / Credential Manager), not a file. A
plaintext `tauri-plugin-store` JSON is readable by anything running as the user, and
this is a full session credential with a 30-day life.

**When no keychain is available** — a bare WM, a container, some CI — the token is
held in memory for the life of the process and the UI says so. Losing a session across
restarts is an annoyance; writing a credential to disk that the user believes is
protected is a lie.

## Packaging

Per-platform builds **must run on their own OS** — there is no cross-compilation, and
each platform's target refuses politely rather than failing deep in a toolchain.
Distribution is GitHub Releases; no app stores.

```
bun run build          # installers for whatever platform you're on
```

Output lands in `src-tauri/target/release/bundle/`.

| Platform | Artifacts | Notes |
|---|---|---|
| Linux | `.deb`, `.rpm`, `.AppImage` | no signing |
| Windows | `.exe` (NSIS) | unsigned; SmartScreen will warn until we buy a cert |
| macOS | `.app`, `.dmg` | CI proof, not distributable, until signing credentials land (below) |

Windows ships the NSIS installer only. An MSI is built by WiX, which caps an MSI's
major version at 255, and calver's major is the year; producing one would mean a second,
derived ProductVersion carried alongside the real one, for an installer the updater never
touches.

A release also carries the updater artifacts (signed update bundles and `.sig`
files) from which the workflow rewrites `latest.json` for the update channel above.

### Versions

The shell's version is **calver, `YYYY.M.N`**, the same scheme as the platform and
numbered on its own line: the first release of a month is `.0`, each further release
that month counts up, and a new month resets it. `2026.10.0` is October's first shell
release whatever the platform is calling itself that week.

Calver rather than semver is a settled decision, not a default (2026-10-01, recorded in
the wiki's release-versioning page). The updater asks one question — is this newer than
what I run — and a date answers it for the person reading a release page and for the
installer doing the comparison. `YYYY.M.N` is valid semver, so the bundler and the
updater parse it without a translation layer, and the numbering only ever moves
forward, so a change of scheme is never a downgrade trap.

The version lives in four places and they are moved together or not at all:

```
bun run scripts/promote-version.ts            # computes the next calver, writes all four
bun run scripts/promote-version.ts --check    # what CI runs: shape and agreement, no writes
```

`package.json`, `src-tauri/Cargo.toml`, `src-tauri/tauri.conf.json` and the
`anthers-desktop` entry in `src-tauri/Cargo.lock`. Four hand-edited numbers is four
chances to disagree, and a disagreement ships silently — every tool reads its own copy
and the build succeeds against whichever one it happened to find. The DMG filename, and
therefore the notarize step that will look for it, is derived from `Cargo.toml`.

### Cutting a release

```
bun run scripts/promote-version.ts        # bump the four fields, commit them through a PR
git tag v2026.10.0 && git push origin v2026.10.0
```

The tag is `v<version>`, the platform's shape on the shell's own line. The `release`
workflow then builds the matrix and opens a **draft** release named
`Anthers Desktop <version>`, carrying every bundle, the updater's signed artifacts and
`latest.json`, with the mechanical changelist as its body —
`scripts/release-notes.sh` writes `## What shipped in <version>` and one line per
squash commit, the same audit-trail shape the platform publishes and never edits.

**Publishing the draft is the release.** Until then the update channel has not moved:
`releases/latest/download/latest.json` still serves the previous version, and every
installed shell keeps updating from it. Publishing fixes forward — a bad publish is
corrected by the next release, never by withdrawing what installed machines already
update from.

A `workflow_dispatch` run of the workflow builds and uploads bundles as workflow
artifacts without touching a release — how the matrix is re-proven between tags.
`macos-latest` builds the Apple-Silicon shell; an x64 macOS distribution would need its
own runner job, added when signing does.

An agent runs these same steps as one procedure: `/promote-desktop`
(`~/.config/opencode/commands/promote-desktop.md`) audits the four version fields,
computes and bumps them through a PR, cuts and pushes the tag, watches the workflow,
verifies the draft mechanically in place of the Releases-page review, publishes, and
reads `releases/latest/download/latest.json` back. Invoked only on an explicit
instruction, since publishing is the moment the update channel moves;
this section's text remains the human-eye version of the same steps.

### macOS signing + notarization

Credentials live in a gitignored `.env` (see `.env.example`): `APPLE_SIGNING_IDENTITY`,
`APPLE_ID`, `APPLE_PASSWORD` (an **app-specific** password), `APPLE_TEAM_ID`.

```
# build, then re-sign, then rebuild the DMG; then
# notarytool submit --wait → stapler staple → spctl
```

Two things in that flow are non-obvious, both learned from `~/Lily`:

1. **Tauri's bundler signs without `--timestamp`, and notarization rejects that.** So
   the build re-signs afterwards, then rebuilds the DMG *from the re-signed `.app`* —
   signing the old DMG would just seal the bad signature inside.
2. **`APPLE_ID` / `APPLE_PASSWORD` / `APPLE_TEAM_ID` are unset while the bundler runs.**
   Left set, Tauri notarizes inline — wasting a round trip to Apple on a binary that is
   about to be replaced by the re-sign.

A bundle signature seals whatever `Contents/MacOS/` holds at that moment, so the
re-sign walks every executable there and signs the `.app` last.

`entitlements.plist` is deliberately minimal: network client (the API) plus JIT and
unsigned-executable-memory (WKWebView JITs JavaScript, and the hardened runtime would
otherwise take the whole webview down). Library validation is intentionally left ON —
the bundle carries no native binaries that load external dylibs.

## The updater

A release build checks its update channel once at startup (`src/updates.rs`), and if a
newer release is published it silently downloads that release's update
artifact, verifies it against `plugins.updater.pubkey` in `tauri.conf.json`, and
applies it at exit — the next launch is the new version. No prompt and no in-app
surface, per the freshness decision this whole mechanism is; an unreachable channel
or a signature miss is a log line, never a dialog. The channel is the release list
itself — `releases/latest/download/latest.json`, rewritten by every published
release.

Two pinned keys guard the desktop: the web-build signature (above), which decides
what the shell *builds against*, and this one, which decides what an *installed* app
becomes. The private half rides the release workflow as `TAURI_SIGNING_PRIVATE_KEY`
(plus its password secret), with its recovery copy in the team's credential store.
A rotation replaces the secret and this `pubkey` in one release, and nothing the old
key signed installs afterwards. The web-build signature and the updater key are
separate keys on purpose — one protects a build input, the other protects a running
installation, and a single key would make them one compromise wide.

## Layout

```
src-tauri/
  src/main.rs      Shell: deep-link handling, the sign-in commands, runtime injection
  src/token.rs     Keychain-backed token store, with an honest in-memory fallback
  tauri.conf.json  Window, bundle targets, icons, the anthers:// scheme
  capabilities/    Webview permissions — deliberately minimal
icons/             Packaging icons, cut by `bunx tauri icon` from the platform's
                   packages/brand/logo/preps/anthersthumb_1x1_light.png
scripts/           resolve-web.ts — puts a built web app at ./web-dist
web-dist/          The web app (gitignored, resolved in — never committed)
```

The frontend is not here: `frontendDist` points at `../web-dist`, which
`beforeBuildCommand` fills via `scripts/resolve-web.ts`.

> [!warning] Config paths resolve from *different* directories, and it fails late
> `frontendDist` and `licenseFile` resolve from **`src-tauri/`**, while
> `beforeBuildCommand` / `beforeDevCommand` run from the **app** dir. Writing them with
> the same number of `../` looks right and breaks only when a real `tauri build` runs —
> `cargo check` and launching the binary directly never invoke the before-commands, nor
> read `licenseFile`, so it can sit broken for a long time. Both were wrong immediately
> after the 2026-08-14 split (`licenseFile` still pointed three levels up into the
> monorepo) and neither showed up in `cargo check`.

## Licensing note

Anthers is Apache-2.0 and `bundle.licenseFile` points at the repo `LICENSE.md`.
