# Changelog

What changed in the Windows app and the phone page, newest first.

## Latest
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
