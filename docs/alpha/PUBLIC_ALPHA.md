# Fanta 1.0.0 Public Alpha

Current distribution guide, updated **12 October 2026**. The **Public Alpha is
available**. The signed package has passed the distribution and tested
installation checks below. Three captioned workflows and a launch overview
have been reviewed on this signed app. Use [the download page](https://www.fantaisa.net/download)
for the verified installer, checksum and current requirements.

The release uses version **1.0.0**, internal channel **stable** and public label
**Public Alpha**. It targets Apple Silicon Macs. Billing stays governed by the
current catalog. Automatic updates are disabled.

## Accepted package and publication status

The Apple Silicon **Build45** package is Developer ID signed, accepted by Apple's
notarization service and stapled. Its source is
`57d86330289153ead8a3bdbd114e1fa530a74e03`. The accepted `Fanta-aarch64.dmg`
has this SHA-256:

```text
5e6b68605bbb601e70778acb2b8fcf72186d1b9a368b7947554f316ab96b87b8
```

On **12 October 2026**, an authenticated Chrome download of the draft installer
retained quarantine and passed Gatekeeper as Notarized Developer ID. The
installed app passed native edit, Save, normal Quit and reopen checks. A Finder
**Replace** with the same signed build preserved the saved change on reopening;
the installed bundle matched all 498 records of the accepted signed bundle.

These checks cover the authenticated draft download and same-build replacement.
They do not establish anonymous public download, a clean-account installation
or migration from an older version. [The reviewed recordings](DEMO_GUIDE.md)
use this signed app; package and recording acceptance do not make the release
publicly available.

## Download and install

1. Visit the download page and confirm it says the Public Alpha is available.
2. Download `Fanta-aarch64.dmg` and `SHA256SUMS.txt` from the linked GitHub
   prerelease `alpha-1.0.0-20261012`.
3. In the folder containing both downloads, verify the checksum:

   ```sh
   shasum -a 256 -c SHA256SUMS.txt
   ```

   Continue only when the installer reports `OK`.
4. Open the disk image and drag **Fanta** into **Applications**. Eject the image,
   then open **Fanta** from Applications. macOS may ask you to confirm opening an
   app downloaded from the internet.
5. Complete the [ten-minute sample exercise](DEMO_GUIDE.md#the-ten-minute-exercise).

The accepted package has passed Developer ID signing, Apple notarization,
stapling and Gatekeeper checks. A Gatekeeper refusal is a support issue. Stop
and report the dialog instead of removing quarantine or bypassing security
checks. Earlier unsigned or ad-hoc-signed internal QA packages retain their
original identities and are not the public installation route.

## Requirements

- An Apple Silicon Mac. There is no Intel or universal installer for this launch.
- The signed Build45 package was tested on an Apple Silicon Mac running
  **macOS 27.0 (26A428)**. A minimum supported macOS version has not been
  established. Check the download page for the tested versions of each package.
- Enough free disk space for the app, your project and imported assets.
- Network access and configured credentials for external agents or hosted
  generation. Local design editing does not consume hosted generation credits.

## Manual updates

1. Save your work and close Fanta. Back up the complete project folder, including
   source, identity sidecars, assets and manifest.
2. Download the newer public installer and verify its published checksum.
3. Drag its Fanta app into Applications and choose **Replace**. Keep your project
   and application data folders.
4. Open Fanta and check its version. Reopen the sample, make an edit, save, close
   and reopen before continuing important work.

An update has its own immutable release identity and checksum. Do not replace a
published installer with different bytes under the same identity.

## Current limitations

- Figma import preserves editable supported layers, with warnings for recognized
  flattening or omissions. Rendering and component overrides can differ.
- Code supports FNX and managed JSON in saved projects. Invalid drafts retain
  the last valid file and canvas. Save across several buffers is sequential and
  does not roll back an earlier successful save.
- **Code refresh warning in Build45:** a 12 October 2026 recording showed
  `Could not refresh saved source: oneshot canceled` after a canvas Save. The
  saved FNX retained the edit, and closing and reopening that project restored
  the Code view. If this warning appears, verify the saved change and resolve
  any unsaved drafts before closing, then reopen the project before editing
  source. Report a persistent warning or a mismatch between saved source and
  canvas. This one recovery does not establish that every refresh warning is
  harmless.
- A source draft can lock conflicting canvas edits until you save or discard it.
  **Saved Code** in Dev mode is read-only.
- Review trust prompts when opening a project from another person. Do not enable
  global trust or discard Git history just to hide a prompt.
- Concurrent external writes can require conflict resolution. Keep a backup and
  avoid editing the same file simultaneously from multiple tools.
- Large projects can take longer to import and save. The alpha does not promise
  complete coverage of every effect, prototype, rich text or media workflow.

See the [capability guide](../fanta/capabilities.md) for current details and exact
acceptance scopes. Earlier [0.1.0-alpha.1 release notes](RELEASE_NOTES.md) and
[known issues](KNOWN_ISSUES.md) describe their original builds.

## Support

Join [Fanta Community on Discord](https://discord.gg/qrFtymBYxk) for installation
help, workflow questions and project showcases.
Use verified community and GitHub reporting links on the download page for
reproducible product problems. Include version, macOS, steps, expected behavior
and screenshots. Remove secrets and private designs from logs and attachments.
Send account and billing reports privately to
[support@fantaisa.net](mailto:support@fantaisa.net).
