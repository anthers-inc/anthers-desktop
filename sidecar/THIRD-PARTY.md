Third-party binaries bundled with the desktop Studio. These are **not** in this repo — `fetch-ffmpeg.ts` downloads them at build time — but they ship inside the installers, so their licences travel with our releases and are recorded here.

# FFmpeg (ffmpeg, ffprobe)

**Licence: GPL-3.0-or-later** (as built), with components under LGPL-2.1-or-later.

These are **GPL** builds rather than LGPL because H.264 encoding requires **libx264**, and linking it is what makes an FFmpeg build GPL. Dropping to an LGPL build would remove libx264 and with it the entire on-device encoding feature, so the GPL flavour is deliberate.

Anthers is **AGPL-3.0-or-later**, which is GPL-compatible. FFmpeg is bundled as a **separate executable invoked as a subprocess** — it is not linked into, and shares no address space with, the Anthers binary. The obligations we carry are therefore to distribute the licence text with the binaries and to make the corresponding source available.

## Where the binaries come from

| Platform | Source | Corresponding source |
|---|---|---|
| Linux x64 / arm64 | [johnvansickle.com/ffmpeg](https://johnvansickle.com/ffmpeg/) release static builds | Linked from the same page; each release ships a matching source tarball |
| Windows x64 | [BtbN/FFmpeg-Builds](https://github.com/BtbN/FFmpeg-Builds) (`-gpl` variant) | Build scripts and pinned upstream revision in that repository |
| macOS x64 | [evermeet.cx/ffmpeg](https://evermeet.cx/ffmpeg/) | Source archives published alongside each build |
| macOS arm64 | [osxexperts.net](https://www.osxexperts.net/) | Upstream FFmpeg release the build is cut from |

Linux uses johnvansickle over BtbN deliberately: both are GPL builds with libx264, but johnvansickle's are roughly half the size (≈77 MB vs ≈139 MB per binary), and each installer carries two of them.

## Upstream

FFmpeg — <https://ffmpeg.org> · source: <https://git.ffmpeg.org/ffmpeg.git>
Licensing detail: <https://ffmpeg.org/legal.html>

> [!warning] Before changing a source URL
> Check the licence flavour of the replacement. An `-lgpl` build will silently lack libx264, and the encoder will fail at runtime rather than at build time.
