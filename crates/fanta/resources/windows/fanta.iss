[Setup]
AppId={#AppId}
AppName={#AppName}
AppVerName={#AppDisplayName}
AppPublisher=Fanta
AppPublisherURL=https://www.fantaisa.net/
AppSupportURL=https://www.fantaisa.net/
AppUpdatesURL=https://www.fantaisa.net/download
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
DisableReadyPage=yes
AllowNoIcons=yes
OutputDir={#OutputDir}
OutputBaseFilename={#AppSetupName}
Compression=lzma
SolidCompression=yes
AppMutex={code:GetAppMutex}
SetupMutex={#AppMutex}Setup
SetupIconFile={#ResourcesDir}\{#AppIconName}.ico
UninstallDisplayIcon={app}\{#AppExeName}.exe
ChangesEnvironment=true
ChangesAssociations=true
MinVersion=10.0.16299
SourceDir={#SourceDir}
AppVersion={#Version}
; Windows version resources take numbers only, so this is Version without its pre-release tag.
VersionInfoVersion={#VersionInfoVersion}
ShowLanguageDialog=auto
WizardStyle=modern

CloseApplications=force

#if GetEnv("FANTA_WINDOWS_SIGN") == "1"
SignTool=Defaultsign
#endif

DefaultDirName={autopf}\{#AppName}
PrivilegesRequired=lowest

ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl,{#ResourcesDir}\messages\en.isl"
Name: "simplifiedChinese"; MessagesFile: "{#ResourcesDir}\messages\Default.zh-cn.isl,{#ResourcesDir}\messages\zh-cn.isl"

[UninstallDelete]
; Delete logs
Type: filesandordirs; Name: "{app}\tools"
Type: filesandordirs; Name: "{app}\updates"
; Delete what an interrupted update leaves behind (see crates/auto_update_helper)
Type: filesandordirs; Name: "{app}\install"
Type: filesandordirs; Name: "{app}\old"
; Delete newer files which may not have been added by the initial installation
Type: filesandordirs; Name: "{app}\x64"
Type: filesandordirs; Name: "{app}\arm64"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked
Name: "associatewithfiles"; Description: "{cm:AssociateWithFiles,{#AppDisplayName}}"; GroupDescription: "{cm:Other}"
Name: "addtopath"; Description: "{cm:AddToPath}"; GroupDescription: "{cm:Other}"

[Dirs]
Name: "{app}"; AfterInstall: DisableAppDirInheritance

[Files]
Source: "{#ResourcesDir}\Fanta.exe"; DestDir: "{code:GetInstallDir}"; Flags: ignoreversion
Source: "{#ResourcesDir}\bin\*"; DestDir: "{code:GetInstallDir}\bin"; Flags: ignoreversion
Source: "{#ResourcesDir}\tools\*"; DestDir: "{app}\tools"; Flags: ignoreversion
#ifexist ResourcesDir + "\amd_ags_x64.dll"
Source: "{#ResourcesDir}\amd_ags_x64.dll"; DestDir: "{app}"; Flags: ignoreversion
#endif
#ifexist ResourcesDir + "\x64\OpenConsole.exe"
Source: "{#ResourcesDir}\x64\OpenConsole.exe"; DestDir: "{code:GetInstallDir}\x64"; Flags: ignoreversion
#endif
#ifexist ResourcesDir + "\arm64\OpenConsole.exe"
Source: "{#ResourcesDir}\arm64\OpenConsole.exe"; DestDir: "{code:GetInstallDir}\arm64"; Flags: ignoreversion
#endif
Source: "{#ResourcesDir}\conpty.dll"; DestDir: "{code:GetInstallDir}"; Flags: ignoreversion

[Icons]
Name: "{group}\{#AppName}"; Filename: "{app}\{#AppExeName}.exe"; AppUserModelID: "{#AppUserId}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExeName}.exe"; Tasks: desktopicon; AppUserModelID: "{#AppUserId}"

[Run]
Filename: "{app}\{#AppExeName}.exe"; Description: "{cm:LaunchProgram,{#AppName}}"; Flags: nowait postinstall; Check: WizardNotSilent

[Registry]
; Fanta design sources open in Fanta by default, as on macOS.
Root: HKCU; Subkey: "Software\Classes\.fnx"; ValueType: string; ValueName: ""; ValueData: "{#RegValueName}.fnx"; Flags: uninsdeletevalue; Tasks: associatewithfiles
Root: HKCU; Subkey: "Software\Classes\.fnx\OpenWithProgids"; ValueType: string; ValueName: "{#RegValueName}.fnx"; ValueData: ""; Flags: uninsdeletevalue; Tasks: associatewithfiles
Root: HKCU; Subkey: "Software\Classes\{#RegValueName}.fnx"; ValueType: string; ValueName: ""; ValueData: "{cm:FantaDesignSource}"; Flags: uninsdeletekey; Tasks: associatewithfiles
Root: HKCU; Subkey: "Software\Classes\{#RegValueName}.fnx"; ValueType: string; ValueName: "AppUserModelID"; ValueData: "{#AppUserId}"; Tasks: associatewithfiles
Root: HKCU; Subkey: "Software\Classes\{#RegValueName}.fnx\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExeName}.exe"",0"; Tasks: associatewithfiles
Root: HKCU; Subkey: "Software\Classes\{#RegValueName}.fnx\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExeName}.exe"" ""%1"""; Tasks: associatewithfiles

; Figma keeps the default for .fig files; Fanta is only offered under Open with, as on macOS.
Root: HKCU; Subkey: "Software\Classes\.fig\OpenWithProgids"; ValueType: string; ValueName: "{#RegValueName}.fig"; ValueData: ""; Flags: uninsdeletevalue; Tasks: associatewithfiles
Root: HKCU; Subkey: "Software\Classes\{#RegValueName}.fig"; ValueType: string; ValueName: ""; ValueData: "{cm:FigmaDesign}"; Flags: uninsdeletekey; Tasks: associatewithfiles
Root: HKCU; Subkey: "Software\Classes\{#RegValueName}.fig"; ValueType: string; ValueName: "AppUserModelID"; ValueData: "{#AppUserId}"; Tasks: associatewithfiles
Root: HKCU; Subkey: "Software\Classes\{#RegValueName}.fig\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExeName}.exe"",0"; Tasks: associatewithfiles
Root: HKCU; Subkey: "Software\Classes\{#RegValueName}.fig\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExeName}.exe"" ""%1"""; Tasks: associatewithfiles

Root: HKCU; Subkey: "Software\Classes\Applications\{#AppExeName}.exe"; ValueType: none; ValueName: ""; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\Applications\{#AppExeName}.exe\DefaultIcon"; ValueType: none; Flags: deletekey
Root: HKCU; Subkey: "Software\Classes\Applications\{#AppExeName}.exe\shell\open"; ValueType: string; ValueName: "Icon"; ValueData: """{app}\{#AppExeName}.exe"""
Root: HKCU; Subkey: "Software\Classes\Applications\{#AppExeName}.exe\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExeName}.exe"" ""%1"""
Root: HKCU; Subkey: "Software\Classes\Applications\{#AppExeName}.exe\SupportedTypes"; ValueType: string; ValueName: ".fnx"; ValueData: ""
Root: HKCU; Subkey: "Software\Classes\Applications\{#AppExeName}.exe\SupportedTypes"; ValueType: string; ValueName: ".fig"; ValueData: ""

; Environment
Root: HKCU; Subkey: "Environment"; ValueType: expandsz; ValueName: "Path"; ValueData: "{code:AddToPath|{app}\bin}"; Tasks: addtopath; Check: NeedsAddToPath(ExpandConstant('{app}\bin'))

; URI Scheme
Root: HKCU; Subkey: "Software\Classes\fanta"; ValueType: "string"; ValueData: "URL:fanta Protocol"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\fanta"; ValueType: "string"; ValueName: "URL Protocol"; ValueData: ""
Root: HKCU; Subkey: "Software\Classes\fanta\DefaultIcon"; ValueType: "string"; ValueData: "{app}\{#AppExeName}.exe,0"
Root: HKCU; Subkey: "Software\Classes\fanta\shell\open\command"; ValueType: "string"; ValueData: """{app}\{#AppExeName}.exe"" ""%1"""

[Code]
function WizardNotSilent(): Boolean;
begin
  Result := not WizardSilent();
end;

// https://stackoverflow.com/a/23838239/261019
procedure Explode(var Dest: TArrayOfString; Text: String; Separator: String);
var
  i, p: Integer;
begin
  i := 0;
  repeat
    SetArrayLength(Dest, i+1);
    p := Pos(Separator,Text);
    if p > 0 then begin
      Dest[i] := Copy(Text, 1, p-1);
      Text := Copy(Text, p + Length(Separator), Length(Text));
      i := i + 1;
    end else begin
      Dest[i] := Text;
      Text := '';
    end;
  until Length(Text)=0;
end;

function NeedsAddToPath(path: string): boolean;
var
  OrigPath: string;
begin
  if not RegQueryStringValue(HKCU, 'Environment', 'Path', OrigPath)
  then begin
    Result := True;
    exit;
  end;
  Result := Pos(';' + path + ';', ';' + OrigPath + ';') = 0;
end;

function AddToPath(path: string): string;
var
  OrigPath: string;
begin
  RegQueryStringValue(HKCU, 'Environment', 'Path', OrigPath)

  if (Length(OrigPath) > 0) and (OrigPath[Length(OrigPath)] = ';') then
    Result := OrigPath + path
  else
    Result := OrigPath + ';' + path
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  Path: string;
  InstalledPath: string;
  Parts: TArrayOfString;
  NewPath: string;
  i: Integer;
begin
  if not CurUninstallStep = usUninstall then begin
    exit;
  end;
  if not RegQueryStringValue(HKCU, 'Environment', 'Path', Path)
  then begin
    exit;
  end;
  NewPath := '';
  InstalledPath := ExpandConstant('{app}\bin')
  Explode(Parts, Path, ';');
  for i:=0 to GetArrayLength(Parts)-1 do begin
    if CompareText(Parts[i], InstalledPath) <> 0 then begin
      NewPath := NewPath + Parts[i];

      if i < GetArrayLength(Parts) - 1 then begin
        NewPath := NewPath + ';';
      end;
    end;
  end;
  RegWriteExpandStringValue(HKCU, 'Environment', 'Path', NewPath);
end;

// https://docs.microsoft.com/en-us/windows-server/administration/windows-commands/icacls
// https://docs.microsoft.com/en-US/windows/security/identity-protection/access-control/security-identifiers
procedure DisableAppDirInheritance();
var
  ResultCode: Integer;
  Permissions: string;
begin
  Permissions := '/grant:r "*S-1-5-18:(OI)(CI)F" /grant:r "*S-1-5-32-544:(OI)(CI)F" /grant:r "*S-1-5-11:(OI)(CI)RX" /grant:r "*S-1-5-32-545:(OI)(CI)RX"';

  Permissions := Permissions + Format(' /grant:r "*S-1-3-0:(OI)(CI)F" /grant:r "%s:(OI)(CI)F"', [GetUserNameString()]);

  Exec(ExpandConstant('{sys}\icacls.exe'), ExpandConstant('"{app}" /inheritancelevel:r ') + Permissions, '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
end;

function SwitchHasValue(Name: string; Value: string): Boolean;
begin
  Result := CompareText(ExpandConstant('{param:' + Name + '}'), Value) = 0;
end;

function IsUpdating(): Boolean;
begin
  Result := SwitchHasValue('update', 'true') and WizardSilent();
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssPostInstall then
  begin
    if IsUpdating() then
    begin
      SaveStringToFile(ExpandConstant('{app}\updates\versions.txt'), '{#Version}' + #13#10, True);
    end
  end;
end;

function GetAppMutex(Param: string): string;
begin
  if IsUpdating() then
    Result := ''
  else
    Result := '{#AppMutex}';
end;

function GetInstallDir(Param: string): string;
begin
  if IsUpdating() then
    Result := ExpandConstant('{app}\install')
  else
    Result := ExpandConstant('{app}');
end;
