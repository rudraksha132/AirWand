# Why Windows says "potentially dangerous", and how to stop it

Chrome ("potentially dangerous"), Edge and Windows SmartScreen ("Windows protected your PC") judge a
download by two things: is it **signed** by a publisher they can name, and have enough other people
downloaded it without trouble (**reputation**). A new, unsigned `.exe` fails both, whatever it contains.
Nothing inside the program can change that. These are the real fixes, best first.

1. **Sign the files.** The release workflow (`.github/workflows/release.yml`) signs `airwand.exe` and the
   installer when these are set in the repository (Settings > Secrets and variables > Actions):
   - secrets `AZURE_TENANT_ID`, `AZURE_CLIENT_ID`, `AZURE_CLIENT_SECRET`
   - variables `TS_ENDPOINT` (for example `https://eus.codesigning.azure.net`), `TS_ACCOUNT`, `TS_PROFILE`

   Source of the certificate, any one of:
   - **Azure Trusted Signing**: about 10 USD a month, identity checked by Microsoft, and its certificates
     carry reputation with SmartScreen from the start. Easiest path for a one-person project.
   - **SignPath Foundation**: free signing for open-source projects (apply at signpath.org).
   - An **OV or EV code-signing certificate** from a certificate authority. Use your own `signtool` step.

   Signed builds usually lose the Chrome warning at once. SmartScreen can still say "unrecognized app" for a
   few days until reputation builds, unless the certificate is Trusted Signing or EV.

2. **Ship the installer, not a bare exe.** `Windows/installer/AirWand.iss` makes `AirWand-Setup-x.y.z.exe`:
   per-user install without admin rights, Start Menu entry, optional start with Windows, an entry in
   Settings > Apps so it can be uninstalled, and an optional firewall rule. It is the shape Windows expects
   from a real app. Signing it is what removes the warning.

3. **Microsoft Store.** A Store (MSIX) package is signed by Microsoft and gets no download warning. Needs a
   Partner Center account (one-time fee for individuals).

4. **Report the false positive** for each release at https://www.microsoft.com/wdsi/filesubmission and, if a
   scanner such as VirusTotal flags it, to that vendor. This helps reputation, not a guarantee.

5. **On your own PC.** Build locally (`cd Windows && cargo build --release`): a file you built has no
   "downloaded from the internet" mark. Or right-click the file > Properties > **Unblock**.

The release also publishes `SHA256SUMS.txt` and a GitHub build attestation, so anyone can check that a
download is exactly what the workflow built from this source.
