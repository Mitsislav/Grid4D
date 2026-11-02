# Packages and releases

> Our own specification (not drawn from Maxon's documentation): what a user
> downloads, how each package installs and opens Grid4D, and how a release is
> made. Covers implementation-plan step **14.6** (decision D19).

## Purpose

Anyone can install Grid4D on Windows, macOS or Linux without building it,
from a GitHub Release or from itch.io, and open `.g4d` files with it.

## What a release holds

Each release `v<version>` (for example `v1.0.0-rc.1`) has these downloads:

| File | System | What it is |
|------|--------|------------|
| `Grid4D-<version>-windows-x64.zip` | Windows 10+ (64-bit) | `grid4d.exe` and the licence files, to unpack anywhere and run |
| `Grid4D-<version>-windows-x64.msi` | Windows 10+ (64-bit) | An installer: Grid4D in *Program Files*, a Start menu entry, `.g4d` files opening with Grid4D, removable from *Apps* |
| `Grid4D-<version>-macos-universal.dmg` | macOS 11+ (Apple silicon and Intel) | A disk image with `Grid4D.app`, to drag into *Applications* |
| `Grid4D-<version>-linux-x64.tar.gz` | 64-bit Linux | `grid4d`, the icons, a `.desktop` file and the licences |
| `Grid4D-<version>-linux-x64.AppImage` | 64-bit Linux | One file: make it executable and run it |
| `SHA256SUMS.txt` | — | The SHA-256 of every file above |

Every package contains `LICENSE-MIT`, `LICENSE-APACHE` and
`THIRD_PARTY_LICENSES.txt` (on macOS inside the app's *Resources*).

## Behaviour

- **The application's identity.** The name is *Grid4D* everywhere (window,
  Start menu, Dock, launcher); the icon is Grid4D's own (`assets/icon/`),
  at every size the system asks for. The version is the release's: Help ▸
  About shows it, and the Windows file properties, the macOS *Get Info* and
  the installer show it too.
- **Opening documents.** The installer (Windows), the app bundle (macOS) and
  the `.desktop` file (Linux) declare the `.g4d` document type ("Grid4D
  Document", with the application's icon). On Windows and Linux,
  double-clicking a `.g4d` file starts Grid4D with it; on macOS it starts
  Grid4D, and the file is opened with File ▸ Open or by dropping it on the
  window (the macOS "open document" event does not reach the app yet).
- **Unsigned for 1.0.** The packages are not code-signed (R4). Windows
  SmartScreen may say the publisher is unknown: *More info ▸ Run anyway*.
  macOS may say the app cannot be checked: right-click `Grid4D.app` ▸
  *Open* ▸ *Open*, once.
- **Settings and data** live in the user's data folder whichever package
  was used (`%APPDATA%\grid4d\data`, `~/Library/Application Support/grid4d`,
  `~/.local/share/grid4d`); uninstalling leaves them, so a reinstall keeps
  the preferences, scripts and recovery copies.
- **Uninstalling.** Windows: *Settings ▸ Apps* (the `.msi`), or delete the
  unpacked folder (the `.zip`). macOS: drag `Grid4D.app` to the Bin. Linux:
  delete the AppImage or the unpacked folder.

## How a release is made

- Pushing a tag `v*` runs the release workflow
  (`.github/workflows/release.yml`):
  1. three optimised builds — Windows x64, macOS universal (an x86-64 and an
     arm64 build joined with `lipo`, macOS 11 as the oldest), Linux x64
     (built on an older Ubuntu so it runs on older systems);
  2. the packages above (the `.msi` with WiX, the `.dmg` with `hdiutil`,
     the AppImage with `appimagetool`);
  3. `SHA256SUMS.txt`;
  4. a **draft** GitHub Release named after the tag, its text the
     version's section of `CHANGELOG.md`, the files attached — published
     by hand after checking it;
  5. the same packages pushed to itch.io with `butler`, to the channels
     `windows`, `mac` and `linux`, with the version from the tag — only
     when the repository has a `BUTLER_API_KEY` secret (skipped otherwise).
- The workflow can also be run by hand (*Run workflow*): it builds the
  packages and keeps them as workflow artifacts, without a release or an
  upload — a dry run before tagging.
- The version: the tag without its `v`, equal to the workspace version in
  `Cargo.toml` (the workflow fails if they differ). The installer's own
  version is the numeric part (`1.0.0`); a later package of the same
  numeric version replaces it.

## Grid4D mapping

| Piece | Where |
|-------|-------|
| Icons: `grid4d.svg` (master), `grid4d-small.svg` (≤ 32 px), `png/<size>.png` (16 – 1024), `grid4d.ico` (Windows), `grid4d.icns` (macOS) | `assets/icon/` |
| The Windows installer (WiX v5: per-machine, Start menu, `.g4d` association, major upgrades) | `assets/windows/grid4d.wxs` |
| The macOS bundle's `Info.plist` (identifier `io.github.mitsislav.grid4d`, document type, version from the tag) | `assets/macos/Info.plist` |
| The Linux launcher and the `.g4d` MIME type | `assets/linux/grid4d.desktop`, `assets/linux/grid4d.xml` |
| The workflow | `.github/workflows/release.yml` |
| The release notes | `CHANGELOG.md` (a section per version) |
| The itch.io page's fields | `assets/itch/page.md` |

## Decisions (2026-11-02)

1. **Our own workflow, not `cargo-dist`** (D19): every step visible, the
   bundle and the associations ours.
2. **A draft release**: nothing is public until a person has looked at it.
3. **A dry run by hand** builds the same packages without publishing, so
   the workflow can be tried before a tag exists.
4. **Generated icons are committed** (rendered from the SVG masters), so a
   release needs no image tools beyond the system's.

## Acceptance criteria

- [ ] A dry run of the workflow builds every package.
- [ ] The release candidate's packages each install and open on a clean
      Windows, macOS and Linux system: the demo, a saved document, a
      script, a plugin object.
- [ ] `.g4d` files open with Grid4D from the file manager on Windows and
      Linux.
