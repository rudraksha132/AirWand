# Changelog

What changed in the Windows app and the phone page, newest first.

## Latest
- Anti-wobble (new `steady.rs`, runs after the 1€ filter): the cursor now behaves like it has static friction. A small
  self-calibrating dead-band holds it still against hand tremor and shrinks as you move faster, so quick moves stay
  free and slow precise creeping still works. The tap or lift of your thumb no longer jolts the cursor: aim is held
  until the jolt dies down, then the jolt is forgotten. Strength follows Smoothing (Low = gentle, Medium = normal,
  High = strong); "Minimal (lowest latency)" leaves it off. In a simulated shaky hand it cuts held jitter 5-9x and
  tap shift 2-7x with no added lag.
- New aiming feel: a smaller turn reaches every corner, and up/down needs the same angle as left/right.
- Edge push: tilt a little past the edge zone and the cursor keeps drifting that way until the phone returns to its
  starting angle. Works in every direction; strength and zone are adjustable.
- Sensitivity on the phone (gear button): Sensitivity, Edge push and Scroll speed sliders with Precise / Normal / Fast
  presets. Sensitivity also scales trackpad speed; trackpad mode turns aim and edge push off, and switching modes never
  makes the cursor jump.
- The phone remembers the paired PC and hides the pairing code until the PC is waiting for approval
  ("Show code" and "Forget this PC" are in the live screen).
- Keyboard button on the phone: type text and press Backspace, Enter, Tab, Esc, Delete and arrows (with Ctrl/Shift) on the
  PC. Off until "Allow typing from phones" is switched on in the panel; approved phones only, rate limited, stops on Pause.
- Bluetooth trouble no longer opens a window (it showed again every 10 minutes); it stays in the panel and tray tip.
- Installer (`AirWand-Setup.exe`): installs for the current user without admin, appears in Settings > Apps with
  publisher and uninstall, Start Menu entry, optional start with Windows, optional firewall rule.
- Phone link without a port (`http://192.168.x.x`, port 80) and `http://airwand.local`; QR code in the panel.
- Only real Wi-Fi and Ethernet addresses are offered, never a VPN tunnel address.
- Aiming now uses the phone's fused orientation (as the original app uses iOS attitude), with the raw gyro only as a fallback.
- Motion sensors: fixed the page stopping when Chrome answers the permission request with "prompt"; the phone
  page now shows a live sensor and link status.
- Error alert windows on the PC: crash, failed start, port in use, Bluetooth failure, Windows refusing cursor control.
- Windows manifest: long paths and UTF-8.

## Earlier
- Wi-Fi and Bluetooth links at the same time, window memory, antivirus-friendlier build.
