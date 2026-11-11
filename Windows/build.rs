// Embeds the icon (resource id 1), version info and an application manifest into airwand.exe.
// A proper identity (name, description, version, asInvoker manifest) is part of what makes
// SmartScreen and antivirus heuristics treat an exe as an ordinary app. Failure to embed is not fatal.
#[cfg(windows)]
fn main() {
    let mut res = winres::WindowsResource::new();
    res.set_icon("AppIcon.ico");
    res.set("ProductName", "AirWand");
    res.set("FileDescription", "AirWand - use your phone as a pointer");
    res.set("InternalName", "airwand");
    res.set("OriginalFilename", "airwand.exe");
    res.set("CompanyName", "AirWand");
    res.set("LegalCopyright", "Copyright (c) 2026 Rudraksha. MIT license");
    res.set_manifest(
        r#"<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <assemblyIdentity type="win32" name="AirWand.AirWand" version="2.6.0.0"/>
  <description>AirWand - use your phone as a pointer</description>
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security><requestedPrivileges><requestedExecutionLevel level="asInvoker" uiAccess="false"/></requestedPrivileges></security>
  </trustInfo>
  <application xmlns="urn:schemas-microsoft-com:asm.v3">
    <windowsSettings>
      <longPathAware xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">true</longPathAware>
      <activeCodePage xmlns="http://schemas.microsoft.com/SMI/2019/WindowsSettings">UTF-8</activeCodePage>
    </windowsSettings>
  </application>
  <compatibility xmlns="urn:schemas-microsoft-com:compatibility.v1">
    <application>
      <supportedOS Id="{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}"/>
    </application>
  </compatibility>
</assembly>"#,
    );
    let _ = res.compile();
}

#[cfg(not(windows))]
fn main() {}
