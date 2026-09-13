# Anthers Desktop

The desktop app for [Anthers](https://github.com/anthers-inc/anthers) — the whole
platform in a native window.

**This repository is the shell, and there is no UI in it.** The thing inside the window
is the platform's web app, bundled whole; what the shell adds is everything a browser
tab cannot do — a session that survives without cookies, native ffmpeg encoding that
doesn't tie a creator to a tab for the duration of an encode, a custom-scheme deep link,
and a real application window.

The two lived in one repository until 2026-08-14, and separated because the toolchains
have nothing in common: this one is cargo, a three-OS build matrix, code signing and
installers, on a release cadence unrelated to the platform's.

## Getting the web app

`./web-dist` is what Tauri bundles, and `scripts/resolve-web.ts` fills it from the first
source that works:

1. **`ANTHERS_WEB=/path/to/dist`** — an explicit, already-built dist. Overrides all else.
2. **A sibling `../Anthers` checkout** — built on demand. The local-dev path, and what
   keeps working across two repositories cheap: iterating on the web app and the shell
   together stays `build web → run desktop`, with no publish step in between.
3. **The pinned release asset** — downloaded from the platform's releases. What CI and a
   clean machine use. The pin is `anthersWeb` in `package.json`.

🚨 **Why a release asset and not a package on GitHub Packages**, which is the obvious
choice and the wrong one: GitHub's documentation says *"You need an access token to
publish, install, and delete private, internal, and public packages."* Authentication is
required even for **public** packages — so a package would mean every contributor and
every CI run needing a token to install a public dependency of a public, AGPL project. A
release asset downloads anonymously.

## Running it

```sh
git clone https://github.com/anthers-inc/anthers.git ../Anthers   # source 2, the easy path
bun install
bun run dev      # resolve the web app, fetch ffmpeg, open the window
```

Point it at a running local API with the platform's `make dev` in another terminal.

| Command | What it does |
|---|---|
| `bun run dev` | Run against the local dev API |
| `bun run web` | Resolve the web app into `./web-dist` and stop |
| `bun run check` | `cargo check` the Rust shell — no installers, no downloads |
| `bun run build` | Installers for **this** platform (no cross-compilation) |

Debug builds point at `http://localhost:8000`; release builds at `https://anthers.org`.
Override either with `ANTHERS_API_BASE` at build time.

The first build downloads ~150 MB of ffmpeg binaries; they are gitignored and cached
after that.

Architecture and the decisions behind it live in the wiki:
**42.06 Creator Studio Architecture**, especially *§ Desktop auth*.

## Why the window isn't just pointed at anthers.org

`invoke` is unavailable to remote documents, so native capability would mean opening
the IPC boundary to a page served over the internet. An XSS on the site would then
reach filesystem and process spawn on every user's machine — unacceptable for the one
app whose entire purpose is native capability. So the SPA is bundled, which is
what makes the auth work below necessary.

## Auth, in one paragraph

Serving from `tauri://localhost` means the `.anthers.org` session cookie is never sent,
so the app carries an `Authorization: Bearer` token instead — the same opaque session
row, in a different envelope. It's obtained by a **browser handoff**: the app opens the
authorize page in the creator's *own* browser (no password is ever typed into this
app), they confirm once, and a one-time code returns over the `anthers://` scheme.
**PKCE** binds the two halves — the verifier never leaves this process, so another
local app that hijacks the scheme and steals the code off the deep link cannot redeem
it. The token is independently revocable from Settings → Devices on anthers.org.

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
| Windows | `.msi`, `.exe` (NSIS) | unsigned; SmartScreen will warn until we buy a cert |
| macOS | `.app`, `.dmg` | signed + notarized, see below |

### Cutting a release

```
# sync the version across Cargo.toml + tauri.conf.json + package.json, then
# on each machine:
bun run build                    # (+ notarize on the Mac)
# then create a DRAFT release and upload the collected bundles
```

`desktop-release` creates the release as a **draft** — review the artifact list before
publishing. Tag names are `desktop-vX.Y.Z`, kept distinct from the platform's tags.

Keep the three version fields in lockstep: the DMG filename, and therefore the notarize
step that looks for it, is derived from `Cargo.toml`.

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

Anthers Desktop adds one wrinkle Lily doesn't have: the **ffmpeg sidecars are separate
executables inside the bundle**, so the re-sign walks everything in `Contents/MacOS/`
rather than just the main binary, and signs the `.app` last — a bundle signature seals
whatever it contains at that moment.

`entitlements.plist` is deliberately minimal: network client (the API) plus JIT and
unsigned-executable-memory (WKWebView JITs JavaScript, and the hardened runtime would
otherwise take the whole webview down). Library validation is intentionally left ON —
the ffmpeg sidecars are static and load no external dylibs.

## Layout

```
src-tauri/
  src/main.rs      Shell: deep-link handling, the sign-in commands, runtime injection
  src/token.rs     Keychain-backed token store, with an honest in-memory fallback
  tauri.conf.json  Window, bundle targets, icons, the anthers:// scheme
  capabilities/    Webview permissions — deliberately minimal
icons/             Packaging icons, cut by `bunx tauri icon` from the platform's
                   packages/brand/logo/preps/antherslogo_thumb1x1_light.png
scripts/           resolve-web.ts — puts a built web app at ./web-dist
sidecar/           Build-time fetch of the bundled ffmpeg/ffprobe
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

## Native encoding

`bun run dev` / `bun run build` fetch static **ffmpeg + ffprobe** into
`src-tauri/binaries/` (gitignored, ~153 MB for the pair on Linux) and Tauri bundles
them as sidecars. `sidecar/fetch-ffmpeg.ts` is idempotent, so only the first build pays
for the download.

> [!warning] The sidecars are named `anthers-ffmpeg` / `anthers-ffprobe`, not `ffmpeg`
> Tauri installs Linux sidecars into **`/usr/bin` under their plain name**. Shipping one
> called `ffmpeg` makes the `.deb`/`.rpm` collide with the distro's own ffmpeg package,
> and dpkg refuses the install outright: *"trying to overwrite '/usr/bin/ffmpeg', which
> is also in package ffmpeg"*. Anyone who already has ffmpeg — a good chunk of the
> creators this app is for — simply couldn't install it. The namespaced names are the
> fix; keep them in sync across `fetch-ffmpeg.ts`, `externalBin`, and the
> `.sidecar("…")` lookups in `encode.rs`.

Encoding produces the **same ladder as the browser encoder** — same rungs, bitrates,
6-second keyframe interval and x264 settings — because the server's `package-video` job
remuxes the variants into HLS with `-c copy`. If the two encoders drift, the same source
yields differently-segmented ladders depending on where it was encoded, so the args live
in one visible block in `src/encode.rs` rather than being assembled cleverly.

What the native path removes, relative to the browser: `ffmpeg.wasm` is single-threaded
per rung and capped at a 300 MB source, and the creator is tied to the tab for the whole
encode. Native x264 threads across every core (~900% CPU observed), reads from disk, and
the app is a window you can leave alone.

The source is picked by **native dialog**, not `<input type=file>` — a webview `File` has
no path, and the path is the point: ffmpeg reads the source straight off disk, so a
multi-gigabyte file is never held in memory during the encode. The *upload* still reads
bytes into memory, same as the browser path; that ceiling is now the upload, not the
encode, and lifting it means moving the upload into Rust.

## Licensing note

Anthers is AGPL-3.0-or-later and `bundle.licenseFile` points at the repo `LICENSE.md`.

The bundled ffmpeg builds are **GPL**, because H.264 encoding needs libx264 and that is
what makes a build GPL rather than LGPL. AGPL-3.0-or-later is GPL-compatible, and ffmpeg
ships here as a **separate executable invoked as a subprocess**, not linked into our
binary. The obligation is to pass the licence along and say where the corresponding
source is — see `sidecar/THIRD-PARTY.md`. Swapping to an LGPL build to avoid that would
also drop libx264, and with it the feature.
