<p align="center"><img src="docs/icon.png" width="160" alt="AirWand icon"></p>

# AirWand

Turn your **smartphone** into an air-mouse pointer, trackpad, and remote keyboard for your **Windows PC** (with support for macOS and Apple devices).

Aim your phone at the screen like a laser remote to move the cursor, tap to click, hold to drag, scroll, and type. No phone app installation required — simply open the web client in your mobile browser, scan the QR code or visit `http://airwand.local`, and connect over local Wi-Fi and/or Bluetooth with seamless zero-jump failover.

> **Status**: The Windows native companion app (`Windows/`, written in Rust) and the phone web client (`Web/`) are fully implemented and tested. Official Windows installer (`AirWand-Setup.exe`) and portable builds are available. The original macOS receiver and native iPhone / Apple Watch apps (`Mac/`, `Apple/`) are also included in this repository.

---

## Highlights & What It Does

- **Point like a laser.** The cursor follows where the top edge of your phone points. Fused orientation (gyroscope + gravity) keeps vertical aim drift-free across all your monitors.
- **Zero-install mobile web client.** No App Store or Play Store downloads needed. Just scan the QR code or open `http://airwand.local` in your phone's browser (Chrome on Android, Safari on iOS).
- **Anti-wobble cursor stabilization.** Built-in static friction simulation (`steady.rs`) eliminates hand tremor when aiming still, automatically vanishing when moving fast. Tap-jolt suppression holds the cursor steady during physical taps and lifts so clicking never jerks the cursor.
- **Dual Wi-Fi and Bluetooth connections at the same time.** Both links remain live simultaneously. If Wi-Fi stutters or Bluetooth drops, the other carries the signal instantly with zero pauses and zero cursor jumps. Works on any PC even without Bluetooth peripheral support.
- **Trackpad mode.** Lay the phone flat on a desk and glide your finger like a laptop touchpad, complete with drag lock for moving windows.
- **Remote typing and keyboard controls.** Type text on your phone to send keystrokes directly to the active window on your PC. Includes dedicated keys for Backspace, Enter, Tab, Esc, Delete, Arrow keys, plus Ctrl and Shift modifiers.
- **Edge push.** Tilt past the screen edge zone and the cursor smoothly drifts across displays until the phone returns to its starting angle — navigate wide multi-monitor setups without lifting your arm.
- **Native Windows control panel.** Built-in local web dashboard (Geist font, dark/light theme, remembers window size and position) with live aim rate, connection status, QR code generator, and customizable tuning.
- **Security with approval code.** Every new device requires your approval on the PC with a matching 6-digit verification code. Device IDs are whitelisted and rate-limited.
- **Orbit Pop.** Built-in target-shooting minigame to practice pointing accuracy, speed, and reaction time.

---

## Quick Start (Windows)

### 1. Install or Run AirWand on Windows

1. Download **`AirWand-Setup.exe`** from [Releases](../../releases) (or grab `AirWand-portable.zip` for standalone use).
   - The installer installs for the current user with no administrator privileges needed, adds a Start Menu entry, and registers an uninstaller in **Settings > Apps**.
2. Run AirWand. A tray icon appears in your taskbar, and the local control panel opens automatically in your browser (at `http://localhost:47821`).
3. If Windows Firewall asks to allow AirWand on private networks, click **Allow**.

### 2. Connect Your Phone

1. Make sure your phone and PC are connected to the same Wi-Fi network.
2. In the AirWand control panel on your PC, you will see a **QR code** and a link (`http://airwand.local` or `http://<PC-IP>`).
3. Scan the QR code with your phone camera, or type the address into your mobile browser:
   - **Android**: Open in **Google Chrome**. *(See one-time sensor flag note below).*
   - **iPhone**: Open in **Safari** over local Wi-Fi, or use **Bluefy** if connecting via Web Bluetooth. Alternatively, use the native iOS app in `Apple/`.
4. Tap **Connect to PC** on your phone.
5. On the first connection, the phone displays a 6-digit pairing code and your PC displays an approval prompt. Verify that the codes match and click **Allow** on the PC.

---

## Mobile Browser Setup (Android & iOS)

### Android (Chrome)
Chrome requires motion sensors to be on HTTPS by default. Because local network links use HTTP:
1. In Chrome on your phone, navigate to:
   ```text
   chrome://flags/#unsafely-treat-insecure-origin-as-secure
   ```
2. Paste the address shown in your AirWand control panel (you can use the **Copy all addresses for the setting** button in the PC panel).
3. Set the flag to **Enabled** and tap **Relaunch**.
4. You only need to do this once — the `http://airwand.local` address will persist across IP changes.

### iPhone (iOS)
Safari restricts motion sensors on non-HTTPS web pages. You can:
- Use the web client over Bluetooth in the **Bluefy** browser.
- Or install and run the native SwiftUI iPhone app from `Apple/` (see [macOS & Apple Devices](#macos--apple-devices-original--alternative) below).

---

## How to Use

- **Aim & Move**: Hold the phone like a TV remote with the top edge facing the monitor. Aim at the middle of the screen and tap **Center** to calibrate.
- **Click**: Tap the pad once for left-click; double-tap for double-click.
- **Drag**: Press and hold your thumb on the screen to grab and drag windows or select text.
- **Right Click**: Tap the **Right click** button.
- **Scroll**: Slide your thumb vertically on the dedicated scroll strip.
- **Pause**: Tap **Pause** to freeze the pointer and free your hand for your physical mouse or keyboard.
- **Trackpad Mode**: Switch to **Trackpad** mode in the top segmented control. Lay the phone on a table and slide your finger across the pad.
- **Keyboard & Typing**:
  - Tap **Keyboard** to open the virtual typing input and navigation keys.
  - Type text and press Enter, Backspace, Tab, Esc, Delete, or Arrow keys (with optional Ctrl / Shift toggles).
  - *Note: Ensure **Allow typing from phones** is switched on in the PC Control Panel.*
- **Sensitivity & Settings**:
  - Tap the **Gear** (⚙️) icon on the phone to adjust Sensitivity, Edge push, and Scroll speed sliders with **Precise**, **Normal**, and **Fast** presets.

---

## Anti-Wobble & Aim Stabilization

AirWand uses a multi-stage stabilization pipeline (`steady.rs`):

```
Phone Motion (Gyro + Gravity)
             │
             ▼
  1€ Adaptive Low-Pass Filter
             │
             ▼
  Anti-Wobble Stabilization (steady.rs)
   ├─ Dynamic Static-Friction Deadband (tremor cancellation)
   └─ Tap-Jolt Suppression (absorbs thumb impact on clicks)
             │
             ▼
  Velocity Predictor Ahead (bridges transmission jitter)
             │
             ▼
  250 Hz Native Windows Cursor Engine
```

1. **Virtual Static Friction**: When aiming still, a self-calibrating deadband holds the cursor steady against physiological hand tremor. As your movement accelerates, the deadband dynamically contracts to zero so quick motions remain instantaneous.
2. **Tap-Jolt Suppression**: Tapping or lifting a finger on a mobile screen introduces mechanical shock. AirWand freezes aim during the impact window and discards the displacement, keeping clicks perfectly centered.
3. **Smoothing Presets**:
   - **Minimal**: Raw 1€ filter with lowest latency (anti-wobble off).
   - **Low**: Gentle tremor damping.
   - **Medium**: Balanced precision and responsiveness (recommended).
   - **High**: Rock-solid stability for presentations or fine desktop targeting.

---

## Dual Simultaneous Links: Wi-Fi & Bluetooth

AirWand supports concurrent Wi-Fi and Bluetooth LE links:

- **Simultaneous Hot Standby**: The phone web page opens both Wi-Fi (WebSocket) and Web Bluetooth, maintaining separate reconnect loops.
- **Zero-Jump Failover**: Aim updates, clicks, Center, and Pause commands are transmitted across all active links. The PC processes the first arriving packet and drops late or duplicate packets by timestamp.
- **Consistent Calibration**: Calibration, smoothing state, and button history belong to the device instance rather than the transport link, ensuring seamless handovers without recentering.
- **Dead-Link Detection**: Periodic keep-alive heartbeats detect disconnected links within 3.5 seconds. If all links drop while a button is held down, AirWand automatically releases the mouse button.
- **No Bluetooth Peripheral Required**: If your PC's Bluetooth adapter does not support BLE peripheral mode, AirWand operates seamlessly over local Wi-Fi.

---

## Windows Control Panel & Tray Features

Opening `http://localhost:47821` (or double-clicking the tray icon) brings up the AirWand Control Panel:

- **Connection Status**: Real-time badges for Wi-Fi and Bluetooth links, device ping, and active address.
- **QR Code Pairing**: Offline QR code rendered directly by AirWand (no external libraries or internet access required).
- **mDNS Hostname**: Built-in multicast DNS responder serving `http://airwand.local` (or `http://airwand-<pc-name>.local`). Automatic port 80 binding with fallback to `:47821`.
- **Paired Devices**: Whitelist of trusted devices with their pairing codes, seen timestamps, and one-click unpairing.
- **Pointer Controls**: Pause, Center, Speed (Fast / Normal / Precise), Smoothing (Minimal / Low / Medium / High), Predict ahead, and Horizontal/Vertical axis inversion.
- **Startup & Security Settings**:
  - **Start AirWand with Windows**: Runs silently in the system tray at user login (direct registry configuration without suspicious shell scripts).
  - **Allow phones over Wi-Fi**: Restricts the server to localhost when turned off.
  - **Allow typing from phones**: Security switch permitting approved devices to send keyboard inputs.
  - **Require encrypted Bluetooth**: Enforces OS-level encrypted BLE pairing.
- **Activity & Diagnostics**: Live aim update frequency counter (aims/sec), rejected packet metrics, and real-time log viewer.

---

## Packaging, Antivirus & SmartScreen

- **Installer (`AirWand-Setup.exe`)**: Built with Inno Setup (`Windows/installer/AirWand.iss`). Installs into the user profile without requiring admin elevation, places a shortcut in the Start Menu, creates a clean uninstall entry in Windows Settings, and configures private network firewall rules.
- **Portable Zip (`AirWand-portable.zip`)**: Standalone archive containing `airwand.exe`, license, and documentation.
- **Windows Manifest**: Includes application metadata, DPI awareness, UTF-8 code page support, and long path awareness.
- **Antivirus & SmartScreen Notes**: Because open-source binaries are newly compiled and initially lack cryptographic reputation, Windows SmartScreen or Chrome may present a warning.
  - For local use: Right-click the downloaded executable > **Properties** > Check **Unblock** > Click **Apply**.
  - Releases are built via GitHub Actions (`.github/workflows/release.yml`) with cryptographic provenance attestations and `SHA256SUMS.txt`. For signing setup, refer to [`docs/SIGNING.md`](file:///C:/Users/PC/Downloads/AirWand/docs/SIGNING.md).

---

## Building from Source (Windows)

### Requirements
- Windows 10 (version 2004+) or Windows 11
- [Rust](https://www.rust-lang.org/) (stable toolchain)
- Microsoft Visual C++ (MSVC) Build Tools

### Build the Windows Receiver
```powershell
cd Windows
cargo build --release
```
The compiled executable will be located at `Windows/target/release/airwand.exe`.

### Run Unit and Protocol Tests
```powershell
cd Windows
cargo test --lib
```
Runs comprehensive tests for packet validation, protocol encoding, pairing algorithms, steady filter maths, mDNS parsing, and QR encoding.

### Test Web Client Links
```bash
node Web/test/links.test.js
```
Executes mock link failover, duplicate dropping, and reconnect tests.

---

## macOS & Apple Devices (Original / Alternative)

AirWand retains complete support for its original macOS receiver and native Apple Watch / iPhone companion apps.

### 1. macOS Receiver (`Mac/`)
Requirements: macOS 14+, Apple silicon or Intel, Bluetooth.

```bash
cd Mac
chmod +x build.sh
./build.sh run
```

- **Permissions**: On first run, grant **Bluetooth** and **Accessibility** permissions (System Settings › Privacy & Security › Accessibility).
- **Notch Panel**: Hover over the MacBook notch or the center of the menu bar to reveal a live aiming map, battery level, connected device details, and quick action buttons.

### 2. Native iPhone & Apple Watch Apps (`Apple/`)
Requirements: iPhone with iOS 17+, Apple Watch with watchOS 10+, Xcode 15+.

Using [XcodeGen](https://github.com/yonaskolb/XcodeGen):
```bash
brew install xcodegen
cd Apple
xcodegen
open AirWand.xcodeproj
```

1. Select your Apple Development Team under **Signing & Capabilities**.
2. Select your iPhone or Apple Watch as the run target and press **⌘R**.
3. Enable **Developer Mode** on your iPhone (Settings › Privacy & Security) and trust your developer certificate.
4. Launch AirWand on your phone or watch, tap **Pair**, and click **Allow** on your Mac.

<details>
<summary>Building without XcodeGen</summary>

1. In Xcode: File › New › Project › iOS › App, named `AirWand` (SwiftUI).
2. File › New › Target › watchOS › App, named `AirWandWatch` (watch-only).
3. Remove the generated template app and content view files.
4. Add `Apple/iOS` (iPhone target), `Apple/Watch` (watch target), and `Apple/Shared` + `Shared/Protocol.swift` (both targets).
5. In each target's `Info.plist`, add `Privacy - Bluetooth Always Usage Description` and `Privacy - Motion Usage Description`.

</details>

---

## Orbit Pop Minigame

Click **Play the target game** in the PC Control Panel or system tray menu:

- Press **F** to enter full-screen mode.
- Targets expand, shrink, and fade. Point and click to pop them before they collapse.
- Direct center hits score double, gold rings score triple, and uninterrupted consecutive hits build your combo multiplier.

---

## Architecture & How It Works

- **Motion Tracking**: Reads device orientation via gravity vector and gyroscope integration. Vertical tilt is bounded by physical gravity (eliminating pitch drift), while horizontal yaw is bias-compensated during resting states.
- **Packet Structure**: Compact binary packets (18-byte `aimpair` samples containing newest and previous sample) streamed over WebSocket and Web Bluetooth.
- **Native Input Simulation**: The Windows engine uses low-level Windows input APIs (`SendInput`) operating at 250 Hz on a high-precision multimedia timer.
- **Multiplexer**: Windows receiver multiplexes dual sockets, correlates device IDs, and deduplicates frames before passing them to the steady filter and cursor engine.

---

## Authors & Credits

- **Rudraksha** ([rudraksha132](https://github.com/rudraksha132)): Author and creator of the AirWand Windows companion app, mobile web client, dual-link networking, anti-wobble stabilization, control panel, installer, and documentation.
- Inspired by [WatchPointer](https://github.com/umarz317/WatchPointer) by umarz317.

---

## License

This project is licensed under the [MIT License](file:///C:/Users/PC/Downloads/AirWand/LICENSE).
