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

; Leaves %APPDATA%\OpenSanctuary (the data dir -- see docs/paths.md) alone unless the
; user ticks "Also delete my library and settings" ([Code] below) --
; nothing in [Files]/[UninstallDelete] references it, so uninstall leaves it
; alone by construction, same guarantee the .deb makes.

[Code]
// "Also delete my library and settings" on the uninstall confirmation (docs/installer.md "Uninstall").
// Unchecked by default; a silent uninstall never deletes data.
var
  DeleteData: Boolean;

function ConfigDir: String;
begin
  Result := ExpandConstant('{userappdata}\OpenSanctuary');
end;

// install.json holds a data_dir entry when the library was moved (docs/paths.md).
// A plain text search is enough for that one small file.
function MovedDataDir: String;
var
  Raw: AnsiString;
  S: String;
  P, Q: Integer;
begin
  Result := '';
  if not LoadStringFromFile(ConfigDir + '\install.json', Raw) then Exit;
  S := String(Raw);
  P := Pos('"data_dir"', S);
  if P = 0 then Exit;
  S := Copy(S, P + Length('"data_dir"'), MaxInt);
  P := Pos('"', S);
  if P = 0 then Exit;
  S := Copy(S, P + 1, MaxInt);
  Q := Pos('"', S);
  if Q = 0 then Exit;
  S := Copy(S, 1, Q - 1);
  StringChangeEx(S, '\\', '\', True);
  StringChangeEx(S, '/', '\', True);
  Result := S;
end;

// Only ever remove a folder that is really an OpenSanctuary library.
procedure RemoveLibrary(Dir: String);
begin
  Dir := RemoveBackslash(Dir);
  if (Dir <> '') and (Length(Dir) > 3) and FileExists(Dir + '\library.db') then
    DelTree(Dir, True, True, True);
end;

function InitializeUninstall: Boolean;
var
  Form: TSetupForm;
  Info: TNewStaticText;
  Box: TNewCheckBox;
  Ok, Cancel: TNewButton;
begin
  Result := True;
  DeleteData := False;
  if UninstallSilent then Exit;

  Form := CreateCustomForm(ScaleX(420), ScaleY(190), False, True);
  try
    Form.Caption := 'Uninstall OpenSanctuary';
    Form.Position := poScreenCenter;

    Info := TNewStaticText.Create(Form);
    Info.Parent := Form;
    Info.Left := ScaleX(16);
    Info.Top := ScaleY(16);
    Info.Width := Form.ClientWidth - ScaleX(32);
    Info.AutoSize := False;
    Info.WordWrap := True;
    Info.Height := ScaleY(70);
    Info.Caption := 'This removes the OpenSanctuary program. Your songs, Bibles, media and settings are kept, so a later install picks up where you left off.';

    Box := TNewCheckBox.Create(Form);
    Box.Parent := Form;
    Box.Left := ScaleX(16);
    Box.Top := ScaleY(92);
    Box.Width := Form.ClientWidth - ScaleX(32);
    Box.Height := ScaleY(34);
    Box.Checked := False;
    Box.Caption := 'Also delete my library and settings (cannot be undone)';

    Cancel := TNewButton.Create(Form);
    Cancel.Parent := Form;
    Cancel.Caption := 'Cancel';
    Cancel.ModalResult := mrCancel;
    Cancel.Cancel := True;
    Cancel.Width := ScaleX(80);
    Cancel.Height := ScaleY(26);
    Cancel.Left := Form.ClientWidth - Cancel.Width - ScaleX(16);
    Cancel.Top := Form.ClientHeight - Cancel.Height - ScaleY(14);

    Ok := TNewButton.Create(Form);
    Ok.Parent := Form;
    Ok.Caption := 'Uninstall';
    Ok.ModalResult := mrOk;
    Ok.Default := True;
    Ok.Width := ScaleX(80);
    Ok.Height := ScaleY(26);
    Ok.Left := Cancel.Left - Ok.Width - ScaleX(8);
    Ok.Top := Cancel.Top;

    Result := Form.ShowModal = mrOk;
    DeleteData := Result and Box.Checked;
  finally
    Form.Free;
  end;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  Moved: String;
begin
  if (CurUninstallStep = usPostUninstall) and DeleteData then
  begin
    Moved := MovedDataDir;
    if Moved <> '' then RemoveLibrary(Moved);
    // The config folder (install.json, plugins) is also the default data folder.
    if FileExists(ConfigDir + '\library.db') or FileExists(ConfigDir + '\install.json') then
      DelTree(ConfigDir, True, True, True);
  end;
end;
