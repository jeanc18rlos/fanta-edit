# Fanta 1.0.0 Public Alpha

Current distribution guide, prepared **11 October 2026** for the **12 October**
launch. The public download becomes available after the final release checks
pass. [The download page](https://www.fantaisa.net/download) is the current source
for availability, installer, checksum, tested macOS versions and support.

The release uses version **1.0.0**, internal channel **stable** and public label
**Public Alpha**. It targets Apple Silicon Macs. Billing stays governed by the
current catalog. Automatic updates are disabled.

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

The public installer must be signed with Developer ID, accepted by Apple's
notarization service and stapled before publication. A Gatekeeper refusal is a
support issue. Stop and report the dialog instead of removing quarantine or
bypassing security checks. Unsigned internal QA packages are separate artifacts
and are not the public installation route.

## Requirements

- An Apple Silicon Mac. There is no Intel or universal installer for this launch.
- A macOS version listed as tested on the download page for this exact package.
  Internal QA previously ran on macOS 26.6.2, but that alone does not establish
  acceptance of the final installer or a minimum supported version.
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

Use verified community and GitHub reporting links on the download page for
reproducible product problems. Include version, macOS, steps, expected behavior
and screenshots. Remove secrets and private designs from logs and attachments.
Send account and billing reports privately to
[support@fantaisa.net](mailto:support@fantaisa.net).
