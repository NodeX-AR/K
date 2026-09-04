[Setup]
AppName=K Language
AppVersion=1.1.0
DefaultDirName={pf}\K Language
DefaultGroupName=K Language
UninstallDisplayIcon={app}\k.exe
Compression=lzma2
SolidCompression=yes
OutputDir=.
OutputBaseFilename=K-Setup
ChangesEnvironment=yes
PrivilegesRequired=admin

[Files]
Source: "target\release\k.exe"; DestDir: "{app}"

[Icons]
Name: "{group}\K Language"; Filename: "{app}\k.exe"
Name: "{group}\Uninstall K"; Filename: "{uninstallexe}"
Name: "{commondesktop}\K Language"; Filename: "{app}\k.exe"

[Registry]
Root: HKLM; Subkey: "SYSTEM\CurrentControlSet\Control\Session Manager\Environment"; \
    ValueType: expandsz; ValueName: "Path"; ValueData: "{olddata};{app}"; \
    Check: NeedsAddPath('{app}')
Root: HKCR; Subkey: ".k"; ValueType: string; ValueData: "KLanguageFile"; Flags: uninsdeletevalue
Root: HKCR; Subkey: "KLanguageFile"; ValueType: string; ValueData: "K Language Source File"; Flags: uninsdeletekey
Root: HKCR; Subkey: "KLanguageFile\DefaultIcon"; ValueType: string; ValueData: "{app}\k.exe,0"
Root: HKCR; Subkey: "KLanguageFile\shell\open\command"; ValueType: string; ValueData: """{app}\k.exe"" gui ""%1"""

[Run]
Filename: "{app}\k.exe"; Description: "Launch K Language"; Flags: postinstall nowait skipifsilent

[Code]
function NeedsAddPath(Param: string): boolean;
var
  OrigPath: string;
begin
  if not RegQueryStringValue(HKLM, 'SYSTEM\CurrentControlSet\Control\Session Manager\Environment', 'Path', OrigPath)
  then begin
    Result := True;
    exit;
  end;
  Result := Pos(';' + Param + ';', ';' + OrigPath + ';') = 0;
end;
