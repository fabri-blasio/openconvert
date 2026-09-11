# macOS and Linux release test checklist

Run these tests on the exact artifacts intended for the GitHub release. Do not
rename or rebuild an artifact after it has passed.

## macOS

Run on both Apple Silicon and Intel hardware (or an Intel macOS runner):

```sh
bash release/platform-tests/macos-smoke.sh OpenConvert.dmg
```

The strict command verifies the DMG, CPU architecture, all five bundled
engines, code signature, Gatekeeper assessment, stapled notarization ticket,
and launch. A locally built unsigned artifact can be inspected with
`--allow-unsigned`, but that result is not suitable for a public release.

Then test manually from a fresh macOS user account:

- Open the DMG in Finder, drag OpenConvert into Applications, then eject it.
- Launch from Applications by double-clicking. It must open without the
  “damaged”, “unidentified developer”, or quarantine warning.
- Confirm the Dock, Finder, Applications, title bar, and About icons all show
  the same folder-and-mole artwork on a rounded white tile.
- Resize the window down to its minimum and back up; its corners, shadow, and
  controls must stay intact in light and dark appearance.
- Convert PNG → WebP twice in a row without reselecting WebP.
- Convert PDF → PNG, ZIP → TAR, and WAV → FLAC.
- Add at least three image edits, drag them into a new order, delete the middle
  edit with its trash button, and verify the output follows the displayed order.
- Open the output folder and receipt; confirm the original input is unchanged.
- Quit and reopen the app, then verify settings and history remain usable.
- Remove the app from Applications and confirm no privileged helper remains.

## Linux

Run both package tests on Ubuntu 22.04 and one current Wayland distribution:

```sh
bash release/platform-tests/linux-smoke.sh OpenConvert.AppImage
bash release/platform-tests/linux-smoke.sh OpenConvert.deb
```

Then test manually:

- Run the AppImage without installing it, then install the `.deb` and launch it
  from the desktop application menu.
- Check the launcher, task switcher, window, and About icons for the same
  rounded folder-and-mole artwork.
- Repeat the conversion/edit matrix from the macOS section on both X11 and
  Wayland where available.
- Verify file drag-and-drop, native file pickers, output-folder opening, receipt
  opening, light/dark appearance, window resizing, and relaunch persistence.
- Uninstall the `.deb`, confirm the menu entry is removed, and verify user files
  and conversion outputs were not deleted.

Record the OS version, CPU architecture, desktop environment, package filename,
SHA-256, and pass/fail result for every run.
