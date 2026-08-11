# Anthers Studio (desktop)

The Tauri shell around the Studio. It bundles the **same `apps/web` build** the site serves, opening it at `/studio` — the
browser Studio serves — this is a third consumer of `@anthers/web-shared`, not a fork —
and adds the two things a browser tab cannot do: a session that survives without
cookies, and (next stage) native ffmpeg encoding that doesn't tie the creator to a tab
for the duration of an encode.

Architecture and the decisions behind it live in the wiki:
**42.06 Creator Studio Architecture**, especially *§ Desktop auth*.

## Running it

```
make dev            # in one terminal — the API this build talks to
make desktop-dev    # in another — builds apps/web, then opens the window
```

Debug builds point at `http://localhost:8000`; release builds at `https://anthers.org`.
Override either with `ANTHERS_API_BASE` at build time:

```
ANTHERS_API_BASE=https://staging.example.org make desktop-build
```

`make desktop-check` type-checks the Rust without building installers — much faster
than a full build when you only touched `src-tauri/`.

## Why the window isn't just pointed at studio.anthers.org

`invoke` is unavailable to remote documents, so native capability would mean opening
the IPC boundary to a page served over the internet. An XSS on the Studio would then
reach filesystem and process spawn on every creator's machine — unacceptable for the
one app whose entire purpose is native capability. So the SPA is bundled, which is
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
make desktop-build     # installers for whatever platform you're on
```

Output lands in `src-tauri/target/release/bundle/`.

| Platform | Artifacts | Notes |
|---|---|---|
| Linux | `.deb`, `.rpm`, `.AppImage` | no signing |
| Windows | `.msi`, `.exe` (NSIS) | unsigned; SmartScreen will warn until we buy a cert |
| macOS | `.app`, `.dmg` | signed + notarized, see below |

### Cutting a release

```
make desktop-version V=0.2.0     # sync Cargo.toml + tauri.conf.json + package.json
# on each machine:
make desktop-build               # (+ make desktop-notarize on the Mac)
# from any machine holding the collected bundles:
make desktop-release V=0.2.0     # creates a DRAFT release and uploads them
```

`desktop-release` creates the release as a **draft** — review the artifact list before
publishing (`gh release view studio-v0.2.0 --web`). Tag names are `studio-vX.Y.Z`, kept
distinct from any web-app tagging.

Keep the three version fields in lockstep: the DMG filename, and therefore the notarize
step that looks for it, is derived from `Cargo.toml`.

### macOS signing + notarization

Credentials live in a gitignored `.env` (see `.env.example`): `APPLE_SIGNING_IDENTITY`,
`APPLE_ID`, `APPLE_PASSWORD` (an **app-specific** password), `APPLE_TEAM_ID`.

```
make desktop-build-macos   # build, then re-sign, then rebuild the DMG
make desktop-notarize      # notarytool submit --wait → stapler staple → spctl
```

Two things in that flow are non-obvious, both learned from `~/Lily`:

1. **Tauri's bundler signs without `--timestamp`, and notarization rejects that.** So
   the build re-signs afterwards, then rebuilds the DMG *from the re-signed `.app`* —
   signing the old DMG would just seal the bad signature inside.
2. **`APPLE_ID` / `APPLE_PASSWORD` / `APPLE_TEAM_ID` are unset while the bundler runs.**
   Left set, Tauri notarizes inline — wasting a round trip to Apple on a binary that is
   about to be replaced by the re-sign.

Anthers Studio adds one wrinkle Lily doesn't have: the **ffmpeg sidecars are separate
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
```

The frontend is not here: `frontendDist` points at `../../web/dist`, and
`beforeBuildCommand` builds it.

> [!warning] Those two paths are relative to *different* directories
> `frontendDist` resolves from `src-tauri/` (so `../../web/dist`), while
> `beforeBuildCommand` / `beforeDevCommand` run from the **app** dir (so
> `../web`). Writing both with the same number of `../` looks right and fails
> only when a real `tauri build` runs — `cargo build` and launching the binary directly
> never invoke the before-commands, so it can sit broken for a long time.

## Native encoding

`make desktop-dev` / `desktop-build` fetch static **ffmpeg + ffprobe** into
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
