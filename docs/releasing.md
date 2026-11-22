# Releasing Cortex

Releases are GitHub releases of this repository. A tag push builds the app for macOS, Windows and
Linux, signs the update packages, and publishes them with the manifest the app's updater reads.
Installed copies find the new version on their own.

## Channels

| Channel | Gets | Reads |
|---|---|---|
| **Stable** | Published releases (`v0.6.0`) | `releases/latest/download/latest.json`: GitHub's "latest" release, which is never a pre-release |
| **Preview** | Pre-releases too (`v0.6.0-beta.1`), as soon as they are out | `releases/download/updater/preview.json`: a file on the rolling `updater` release that every release, stable or preview, replaces |

The user picks a channel in **Settings → General → Update channel**. Until they choose, a copy
follows the kind of build it is: a preview build follows Preview, a stable build Stable. The app
looks at start (unless that is turned off, same page) and from **Check for Updates** in the app
menu and the tray.

Updates only go forward. Someone on `0.6.0-beta.2` who switches back to Stable stays where they
are until a stable release newer than that (`0.6.0`) comes out.

## One-time setup

Update packages are signed, and the app installs only what verifies against the public key it
was built with. Make a key pair once, and keep the private key safe: losing it means installed
copies can no longer take updates.

1. Make the key pair (it asks for a password):

   ```sh
   npx tauri signer generate -w ~/.tauri/cortex.key
   ```

2. In the repository's **Settings → Secrets and variables → Actions**, add:
   - the secret `TAURI_SIGNING_PRIVATE_KEY`: the contents of `~/.tauri/cortex.key`;
   - the secret `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`: its password;
   - the variable `UPDATER_PUBKEY`: the contents of `~/.tauri/cortex.key.pub`.

3. Put the same public key in `plugins.updater.pubkey` in `src-tauri/tauri.conf.json`, so builds
   made outside the release workflow trust your releases too. (Release builds get it from
   `UPDATER_PUBKEY` either way.)

The release workflow stops at its first step, with a message, until the key and secrets exist.

## Making a release

```sh
node scripts/set-version.mjs 0.6.0-beta.1   # or 0.6.0 for a stable release
git commit -am "Cortex 0.6.0-beta.1"
git tag v0.6.0-beta.1
git push origin HEAD v0.6.0-beta.1
```

`set-version.mjs` writes the version into `package.json`, `tauri.conf.json` and `Cargo.toml`
(and their lock files); the workflow refuses a tag that does not match all of them, since a
release whose version did not change is one no installed copy would take.

`.github/workflows/release.yml` then:

1. checks the version and opens a **draft** release with notes generated from the merged pull
   requests;
2. builds macOS (Apple silicon and Intel), Windows and Linux, and uploads the installers, the
   signed update packages and `latest.json` to the draft, and builds the Flatpak
   (`.github/workflows/flatpak.yml`);
3. publishes it, as GitHub's latest release when it is stable, as a pre-release when the version
   has a pre-release part (`-beta.1`, `-rc.1`);
4. attaches `Cortex.flatpak` to it, and copies its `latest.json` to `preview.json` on the
   `updater` release.

Nothing reaches users until step 3, so a failed build leaves only a draft to delete.

## What to know

- **Code signing.** The workflow builds without an Apple Developer ID or a Windows certificate:
  macOS builds are signed ad hoc, so the first launch needs right-click → Open; Windows shows a
  SmartScreen notice. This is separate from update signing, which is always on. To sign and
  notarize, add Apple's and Microsoft's certificates as secrets and drop the overrides in
  `src-tauri/tauri.github.conf.json`.
- **Windows installers.** A stable release has an NSIS installer (`setup.exe`, the one the
  updater uses) and an MSI. A pre-release has only the NSIS installer, since an MSI's version
  cannot carry a pre-release part such as `-beta.2`.
- **A failed build.** A re-run builds the same commit with the same workflow, so after a fix,
  delete the draft release and tag the next version (`0.6.0-beta.3`).
- **Linux.** The AppImage updates itself; `.deb` and `.rpm` installs are updated by installing
  the new package.
- **Flatpak.** `flatpak/` holds the manifest, desktop entry and metainfo; the package is the
  Linux build's `.deb` on the GNOME runtime, with the tray icon's library from Flathub's shared
  modules. Inside a Flatpak the app's own updater stands aside and says so: Flatpak updates it.
  To build one yourself: `npx tauri build --bundles deb`, then
  `scripts/build-flatpak.sh src-tauri/target/release/bundle/deb/*.deb`; install it with
  `flatpak install --user Cortex.flatpak`. Actions → Flatpak → Run workflow builds one from any
  branch. Its permissions are listed, with a reason each, in the manifest. Code sessions and the
  sandbox run inside the Flatpak's own sandbox, so the code sandbox's extra isolation
  (bubblewrap, the Podman workbench) is not available there.
- **A required update.** Add `"minimum_supported_version": "0.6.0"` to a release's
  `latest.json` (and to `preview.json`) to make every older copy update before it can be used.
- **Forks.** The workflow builds the app to read releases from the repository it runs in
  (`CORTEX_RELEASES_REPO`), so a fork with its own key publishes and updates on its own.
- **Testing an update.** Run a build with `CORTEX_UPDATE_MANIFEST_URL` set to any manifest to
  see what it would offer.
