# Dustoff

Lock your Mac's keyboard and trackpad so you can wipe them down without shutting the laptop off. The screen stays usable for watching: Dustoff has no window, just a menu-bar icon and a small notice at the bottom of the screen.

**To exit, hold the trackpad down for 5 seconds.** A countdown appears while you hold; let go early and it cancels.

## What it does

- Swallows every input event at the HID level: keys, media keys, clicks, pointer movement, scrolling and gestures.
- Shows "Keyboard and trackpad locked" for 3 seconds once input is blocked, and a menu-bar icon for as long as it runs (dimmed while it waits for permission).
- Quitting by any route, including a crash, gives input straight back: the block lives only as long as the process.

The power button can't be blocked. Holding it is the last-resort way out.

## Install

Requires macOS and Rust 1.99.1 or newer ([rustup](https://rustup.rs)).

```sh
./bundle.sh
open ~/Applications/Dustoff.app
```

`bundle.sh` builds a release binary and installs `~/Applications/Dustoff.app`. On first launch macOS asks to allow Dustoff under **System Settings → Privacy & Security → Accessibility**; it starts blocking as soon as you do. Until then the menu-bar icon has a Quit item.

The first run of `bundle.sh` also creates a self-signed code-signing certificate, "Dustoff Local Signing", in your login keychain. Signing every build with it keeps the app's identity stable, so the Accessibility grant survives rebuilds. Delete it in Keychain Access to remove it.

`cargo run` works too, but then macOS asks for Accessibility on behalf of your terminal app instead of Dustoff.

## How it works

- `src/main.rs` is the whole app. An active `CGEventTap` on its own thread drops all input and watches for a left-button press.
- The countdown and notice are an overlay drawn with [Rune](https://github.com/anomalyco/rune); the menu-bar item is a plain AppKit `NSStatusItem`.
- `assets/` holds the menu-bar icon (an SVG drawn as a template image) and the app icon.

## License

[MIT](LICENSE)
