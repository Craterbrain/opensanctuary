; OpenSanctuary Windows installer (docs/installer.md). Built with Inno Setup 6
; (https://jrsoftware.org, ISCC.exe -- also runs fine under Wine on Linux, see
; packaging/build-windows-installer.sh).
;
; MyAppVersion and SourceDir are passed in via /D from the build script,
; reading the real Cargo.toml version rather than hardcoding it here.
#ifndef MyAppVersion
  #define MyAppVersion "0.0.0-dev"
#endif
#ifndef SourceDir
  #define SourceDir "stage"
#endif

#define MyAppName "OpenSanctuary"
#define MyAppPublisher "OpenSanctuary Core Team"
#define MyAppURL "https://github.com/Craterbrain/opensanctuary"
#define MyAppExeName "OpenSanctuary.exe"

[Setup]
; Fixed forever -- this is what lets the installer recognize "this is an
; upgrade of the same app" across versions. Do not regenerate.
AppId={{8F2B6B7B-6C3E-4B2A-9C7C-0B7B6B7B6B7B}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
AppPublisherURL={#MyAppURL}
AppSupportURL={#MyAppURL}
AppUpdatesURL={#MyAppURL}
; Per-user by default (no admin/UAC prompt) -- matters for church volunteers
; who often don't have admin rights on the sanctuary machine. The dialog
; override still lets someone explicitly choose a per-machine install.
DefaultDirName={autopf}\OpenSanctuary
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
DisableProgramGroupPage=yes
OutputDir=..\target\installer
OutputBaseFilename=opensanctuary-setup-x64
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
; Unsigned for the alpha -- see docs/installer.md "Code signing". SmartScreen
; will warn; acceptable for now, revisit before a real public release.
SetupIconFile=app.ico
UninstallDisplayIcon={app}\web\favicon.ico

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#SourceDir}\OpenSanctuary.exe"; DestDir: "{app}"; Flags: ignoreversion
; No WebView2Loader.dll: the MSVC build (docs/installer.md) statically links
; WebView2LoaderStatic instead of dynamically importing WebView2Loader.dll.
Source: "{#SourceDir}\web\*"; DestDir: "{app}\web"; Flags: ignoreversion recursesubdirs createallsubdirs
; Bundled Android TV client APK (docs/CLIENT_PAIRING.md's ADB-provisioning
; plan) -- resolved at runtime by src/storage/paths.rs::resolve_tv_apk_path().
Source: "{#SourceDir}\tv-client\*"; DestDir: "{app}\tv-client"; Flags: ignoreversion recursesubdirs createallsubdirs skipifsourcedoesntexist
; Bundled ffmpeg/ffprobe (LGPLv3, BtbN/FFmpeg-Builds -- see
; packaging/build-windows-installer.sh's "Bundled ffmpeg/ffprobe" comment
; and ffmpeg\SOURCE.txt for the corresponding-source pointer this license
; requires) -- resolved at runtime by
; src/storage/paths.rs::resolve_ffmpeg_command()/resolve_ffprobe_command().
Source: "{#SourceDir}\ffmpeg\*"; DestDir: "{app}\ffmpeg"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{autoprograms}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; IconFilename: "{app}\web\favicon.ico"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; Tasks: desktopicon; IconFilename: "{app}\web\favicon.ico"

[Run]
; Best-effort firewall pre-registration (docs/installer.md "Firewall") so a
; church volunteer doesn't have to click through a Defender Firewall prompt
; on first launch. Only attempted when the installer itself is elevated
; (IsAdminInstallMode) -- the default per-user/lowest-privilege install has
; no rights to add firewall rules, and Inno doesn't fail the install if this
; command errors or is skipped; Defender's own first-launch prompt is still
; there as a fallback either way. Covers the whole fallback port range
; src/main.rs tries (8443-8453), not just the default 8443, since the actual
; bound port depends on what's free at boot.
Filename: "{sys}\netsh.exe"; Parameters: "advfirewall firewall add rule name=""OpenSanctuary HTTPS"" dir=in action=allow protocol=TCP localport=8443-8453 program=""{app}\{#MyAppExeName}"""; Flags: runhidden; Check: IsAdminInstallMode
Filename: "{app}\{#MyAppExeName}"; Description: "{cm:LaunchProgram,{#MyAppName}}"; Flags: nowait postinstall skipifsilent

[UninstallRun]
; Mirrors the [Run] rule above -- only removes what we might have added, and
; only when uninstalling elevated; harmless no-op (netsh reports "no rules
; match", ignored) if it was never added or already gone.
Filename: "{sys}\netsh.exe"; Parameters: "advfirewall firewall delete rule name=""OpenSanctuary HTTPS"""; Flags: runhidden; Check: IsAdminInstallMode

; Never touches %APPDATA%\OpenSanctuary (the data dir -- see docs/paths.md) --
; nothing in [Files]/[UninstallDelete] references it, so uninstall leaves it
; alone by construction, same guarantee the .deb makes.
