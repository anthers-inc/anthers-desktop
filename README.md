# Anthers Studio (desktop)

The Tauri shell around the Studio. It bundles the **same `apps/studio-web` build** the
browser Studio serves — this is a third consumer of `@anthers/web-shared`, not a fork —
and adds the two things a browser tab cannot do: a session that survives without
cookies, and (next stage) native ffmpeg encoding that doesn't tie the creator to a tab
for the duration of an encode.

Architecture and the decisions behind it live in the wiki:
**42.06 Creator Studio Architecture**, especially *§ Desktop auth*.

## Running it

```
make dev            # in one terminal — the API this build talks to
make desktop-dev    # in another — builds studio-web, then opens the window
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

Per-platform builds **must run on their own OS** — there is no cross-compilation here.
Distribution is GitHub Releases; no app stores.

```
make desktop-build     # installers for whatever platform you're on
```

Output lands in `apps/studio-desktop/src-tauri/target/release/bundle/`.

| Platform | Artifacts |
|---|---|
| Linux | `.deb`, `.rpm`, `.AppImage` |
| Windows | `.msi`, `.exe` (NSIS) |
| macOS | `.app`, `.dmg` |

macOS signing + notarization follow the `~/Lily` pattern (Apple credentials in `.env`,
`notarytool submit` → `stapler staple` → Gatekeeper check). Not wired up yet — see the
Studio Phase 5 task.

## Layout

```
src-tauri/
  src/main.rs      Shell: deep-link handling, the sign-in commands, runtime injection
  src/token.rs     Keychain-backed token store, with an honest in-memory fallback
  tauri.conf.json  Window, bundle targets, icons, the anthers:// scheme
  capabilities/    Webview permissions — deliberately minimal
```

The frontend is not here: `frontendDist` points at `../../studio-web/dist`, and
`beforeBuildCommand` builds it.

## Licensing note

Anthers is AGPL-3.0-or-later and `bundle.licenseFile` points at the repo `LICENSE.md`.
When the ffmpeg sidecar lands it brings its own licensing obligation — record the build
flavour (LGPL vs GPL) and its notice in `packages/brand`'s third-party docs alongside
the existing asset attributions.
