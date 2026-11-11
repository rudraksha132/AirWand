<p align="center"><img src="docs/icon.png" width="160" alt="AirWand icon"></p>

# AirWand

Turn your **iPhone** or **Apple Watch** into a pointer for your Mac. Aim it at the screen to move the cursor, tap to click, hold to drag.

No Wi-Fi and no servers: the phone or watch talks straight to the Mac over Bluetooth.

> Status: the Mac app and the iPhone app work (tested on iOS 27.2). The Apple Watch app is written but has not been tested on a real watch yet.

## What it does

- **Point like a laser.** The cursor goes where the top edge of your device points (12 o'clock on the watch). Up/down comes from gravity, so it never drifts. Works across all your displays.
- **Tap** to click, **double-tap** to double-click, **hold** to drag, a **right-click** button, and scrolling (Digital Crown on the watch, a scroll strip on the phone).
- **Trackpad mode on the iPhone.** Lay it on the table and slide a finger, with a drag lock for moving windows.
- **Center** re-aims the middle of the screen. **Pause** frees the cursor for your real trackpad.
- **Pairing with approval.** The first time a device connects, the Mac asks you to allow it. Devices you didn't allow can't move the cursor.
- **Notch panel.** Hover the notch (or the middle of the menu bar) to see the connected device, its battery, a live aim map and quick buttons. Pin it to keep it open.
- **Orbit Pop**, a small target game to try it on.

## Requirements

- Mac with macOS 14 or later, Apple silicon or Intel, with Bluetooth
- iPhone with iOS 17 or later and/or Apple Watch with watchOS 10 or later
- Xcode 15 or later (use a version whose SDK matches your iPhone's iOS)

## 1. Mac receiver

```bash
cd Mac
chmod +x build.sh
./build.sh run
```

The first launch asks for **Bluetooth** and **Accessibility** (System Settings › Privacy & Security › Accessibility). Accessibility is what lets it move the cursor. Allow both.

After a rebuild macOS may forget the Accessibility permission. If the cursor stops moving, turn AirWand off and on again in that list.

## 2. iPhone and Apple Watch apps

With [XcodeGen](https://github.com/yonaskolb/XcodeGen):

```bash
brew install xcodegen
cd Apple
xcodegen
open AirWand.xcodeproj
```

1. Sign with your Apple ID: create `Apple/Local.xcconfig` with your bundle prefix and Team ID (see `Apple/Config.xcconfig`), or just pick your Team in **Signing & Capabilities**. A free Apple ID works.
2. Choose your iPhone (or your watch) as the run destination and press **⌘R**. The first time, turn on **Developer Mode** on the iPhone (Settings › Privacy & Security) and trust your developer profile (Settings › General › VPN & Device Management).
3. Open AirWand on the device, tap **Pair** next to your Mac, then click **Allow** on the Mac.

With a free Apple ID, apps you install from Xcode stop opening after 7 days; just run them again from Xcode.

<details>
<summary>Without XcodeGen</summary>

1. Xcode › File › New › Project › iOS › App, name it `AirWand`, SwiftUI.
2. File › New › Target › watchOS › App, name it `AirWandWatch`, as a watch-only app.
3. Delete the generated `…App.swift` and `ContentView.swift` files from both targets.
4. Drag in `Apple/iOS` (iPhone target only), `Apple/Watch` (watch target only), and `Apple/Shared` plus `Shared/Protocol.swift` (both targets).
5. In each target's **Info** tab add `Privacy - Bluetooth Always Usage Description` and `Privacy - Motion Usage Description`.

</details>

## Windows + web page (Bluetooth)

This repo also has a Windows receiver (`Windows/`, Rust) and a phone web page (`Web/`) that replace the Mac app and iPhone app. Same protocol, same Bluetooth service, same aiming maths. The Mac and iPhone code is untouched.

- **Windows receiver**: tray app. Needs Windows 10 2004+ and a Bluetooth 4.0+ adapter that supports the **peripheral role** (the app tells you if it does not). Install with `AirWand-Setup.exe` from the Releases page, or build with `cd Windows && cargo build --release` (needs Rust, MSVC build tools); the result is one `airwand.exe` with no runtime to install. Tray menu: Pause, Center, Speed, Smoothing, Invert, Forget devices, live aim rate, log, the game. The notch panel is Mac-only.
- **Phone page**: static `Web/index.html` using Web Bluetooth and the motion sensors. Both need HTTPS: publish `Web/` with the included GitHub Pages workflow (Settings > Pages > Source: GitHub Actions), then open the page on the phone. Works in **Chrome on Android**. Safari on iPhone has no Web Bluetooth: use the **Bluefy** browser, or the native iPhone app above.
- **Pairing**: open the page, tap *Connect to PC*, pick your PC. The page shows a 6-digit code and the PC shows one in its Allow dialog. Click **Allow** only if they match.
- Web BLE writes are limited to about 20 bytes and roughly one per connection interval, so expect 30 to 60 aim updates a second; the receiver still eases at 240 Hz. The page learns and removes gyro bias while the phone is still, to reduce yaw drift.

### Connections: Wi-Fi and Bluetooth at the same time

- **Both links stay up.** The phone page opens Wi-Fi and Bluetooth together and keeps each one alive with its own reconnect loop (0.3 s backing off to 5 s, and an immediate retry when the page comes back to the foreground or the network returns). The page shows a chip for each link.
- **Nothing pauses when a link drops.** Every aim sample, click, Center and Pause goes out on every live link. The PC merges them per device: the first copy to arrive wins and late or duplicate copies are dropped by their timestamps. Scroll and trackpad movement are not repeatable, so they use the primary link only (Wi-Fi, else Bluetooth).
- **No jump on hand-over.** Calibration, smoothing and click history belong to the device, not to the link, so switching link never recentres or resets the cursor.
- **Dead links are noticed.** The page pings the PC every second and the PC answers; 3.5 s of silence replaces the link. The PC drops a silent Wi-Fi socket after 12 s. If the last link of a device dies while a button is held, AirWand releases the button.
- **One Allow prompt per device**, even when the phone pairs over both links at once. The answer applies to both.
- **Bluetooth backup without the picker.** Pick the PC once (button *Add Bluetooth backup*); next time Chrome's `getDevices()` reconnects without asking. The page also reconnects on its own after a reload (Android).
- Tests: `node Web/test/links.test.js` runs the real page script against a fake WebSocket and fake Web Bluetooth (both links, drop and recovery, dead-link detection, stop).

### Control panel and Wi-Fi link

- **Control panel.** Starting `airwand.exe` (or double-clicking it again, or left-clicking the tray icon) opens a window with connection status, the address for your phone, paired devices with their pairing code, pause/center, speed, smoothing, invert, start-with-Windows, security switches and the log. It runs in Edge or Chrome app mode and is only reachable from this PC.
- **Wi-Fi link.** The PC also serves the phone page on `http://airwand.local` (see below), or `http://<PC address>` and talks to it over a WebSocket, using the same packets, pairing code and Allow prompt as Bluetooth. It needs no Bluetooth peripheral support, so it works on adapters that cannot be a BLE peripheral. Windows Firewall may ask to allow AirWand on private networks; allow it.
- **Which is used.** Both, at once (see v2.5 above). Wi-Fi is the primary link; Bluetooth is a hot standby.
- **Android setting.** Chrome blocks motion sensors on `http://` pages, so once: open `chrome://flags/#unsafely-treat-insecure-origin-as-secure` on the phone, add the address shown in the panel, set Enabled, Relaunch. iPhone Safari needs HTTPS for motion sensors, so on iPhone use the native app or Bluefy over Bluetooth.
- **Security.** The Wi-Fi link has the same trust model as plain Bluetooth: pairing needs your approval on the PC, but the device ID travels unencrypted, so only use it on a network you trust. "Allow phones over Wi-Fi" off keeps the server on this PC only. The panel API refuses requests from other machines, wrong Host or Origin headers, and requests without the `X-AirWand` header.
- **Tests.** `cargo test --lib` also covers the web server (routes, WebSocket handshake and framing, origin and host checks).

### Speed

- The page sends the previous and newest aim sample together (`aimpair`, 18 bytes), so a late Bluetooth write no longer drops a sample; the PC drops duplicates by timestamp.
- The PC looks a few ms ahead along the recent aim velocity (tray: *Predict ahead*), uses a high-resolution timer for the 250 Hz cursor loop, runs at high priority, and opts out of Windows 11 efficiency-mode throttling.
- Defaults to low smoothing. For the least lag pick Smoothing > *Minimal*; for steadier aim pick *Medium* or *High*.
- The ceiling is the phone's Bluetooth connection interval, which a web page cannot control.

### Security notes

- The link is plain Bluetooth LE by default, like AirWand's original design. The trusted-device ID is sent unencrypted, so someone sniffing the radio next to you could copy it. Tray menu > *Require encrypted link* makes Windows demand an encrypted, OS-paired link (restart needed; untested with every phone).
- Receiver checks: packet length, finite and in-range numbers, sanitised device names, at most 8 peers, at most 3 Allow prompts a minute, a 60 second cooldown after Deny. Counters and a log (`%APPDATA%\AirWand\log.txt`) show what was rejected.
- Tests: `cd Windows && cargo test --lib` (protocol, validation, pairing code, filter, geometry; runs on any OS). CI also builds the Windows app.

## Using it

Hold the device like a remote with its top edge toward the screen. Point at the middle of the screen and tap **Center**.

Speed, smoothing and axis inversion live in the AirWand menu on the Mac.

Watch tips: on the iPhone's Watch app turn off **Passcode › Wrist Detection** if you hold the watch in your hand, set **Display & Brightness › Wake Duration** to 70 seconds, and **General › Return to Clock › AirWand** to one hour. Long-press the watch screen for settings and unpairing.

## Orbit Pop

Click **Game** in the notch panel or **Play the target game** in the menu. Press **F** for full screen. Rings shrink and fade; pop them before they collapse. Center hits score double, gold rings triple, and every hit in a row raises your combo.

## How it works

- **Device** (`Apple/`): reads gravity and the gyroscope 100 times a second. Up/down is the angle of the top edge above the horizon; left/right adds up turns around the true vertical. Each sample carries the device's timestamp.
- **Mac** (`Mac/`): projects the aim onto the screen like a laser hitting a wall, smooths it with a [1€ filter](https://gery.casiez.net/1euro/) using the device's timestamps, and eases the cursor toward it 240 times a second on its own thread. A click lands where you were aiming the moment your finger touched the screen, so the tap's jolt doesn't move it.
- **Bluetooth**: the Mac advertises a service; the device connects and writes small packets. The protocol is documented at the top of `Shared/Protocol.swift`.

## Credits

Inspired by [WatchPointer](https://github.com/umarz317/WatchPointer) by umarz317. AirWand is a separate implementation written from scratch.

## License

MIT

### Look, window memory, antivirus

- **Minimal layout in Geist.** The control panel and the phone page use Geist and Geist Mono (SIL Open Font License, see `Web/Geist-OFL.txt`), shipped inside the exe and served by the PC itself, so nothing is loaded from the internet. They follow the system light or dark setting.
- **The control panel remembers its window.** Resize or move it and AirWand saves size and position (`settings.json`); the next start opens the same way.
- **Fewer reasons for antivirus to complain.** The exe now has version info and a manifest. The Start with Windows switch writes the registry directly instead of running a hidden `reg.exe` on every start (that pattern looks like malware persistence to heuristics).
- The remaining cause is that the exe is unsigned and new. Fixes, best first: sign it (free for open source via SignPath Foundation, or Azure Trusted Signing); report the false positive at https://www.microsoft.com/wdsi/filesubmission ; on your own PC, build locally (no download mark) or Properties > Unblock.

### Installer, short links and QR codes

- **Real Windows app.** `Windows/installer/AirWand.iss` builds `AirWand-Setup.exe` (Inno Setup): installs for the current user with no admin prompt, adds a Start Menu entry and an uninstall entry in Settings > Apps, offers start-with-Windows, and (when installed for all users) a firewall rule for private networks. The exe manifest also turns on long paths and UTF-8. The release workflow builds the exe, installer and portable zip, signs them when signing secrets are set, and publishes checksums and a build attestation.
- **The "potentially dangerous" warning** comes from the download being unsigned and new, not from the program. Only signing (plus time) removes it. See `docs/SIGNING.md` for the options: Azure Trusted Signing, SignPath (free for open source), a certificate, or the Microsoft Store.
- **Links that stay put.** The PC now answers to **`http://airwand.local`** (a built-in multicast DNS responder; it checks that no other device owns the name and otherwise uses `airwand-<pc name>.local`). It also listens on port 80 when free, so links need no port number; if port 80 is taken, links carry `:47821`. The Android motion-sensor setting needs the address once; a name never changes, so it keeps working, unlike an IP address. The plain IP link stays as the fallback for phones that cannot resolve `.local`.
- **On the PC**, `http://localhost:47821` opens the control panel (typing `http://localhost` also works). The phone page is always at `/phone`.
- **QR codes.** The panel shows a QR code for the phone link (short name or IP address, your choice is remembered). Point the phone camera at it and tap the link. The code is drawn by AirWand itself (no internet, no library), served only to this PC.
- Tests: `cargo test --lib` also covers the QR encoder, the DNS packets and the mDNS replies.
