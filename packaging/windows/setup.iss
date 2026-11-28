#ifndef AppVersion
  #error AppVersion is required
#endif
#ifndef PackageDir
  #error PackageDir is required
#endif
[Setup]
AppId={{6CC07E9E-9036-4D4D-B31E-7F146EE32223}
AppName=Stutter Watch
AppVersion={#AppVersion}
AppPublisher=flochiii
AppPublisherURL=https://github.com/flochiii/stutter-watch
AppSupportURL=https://github.com/flochiii/stutter-watch/issues
AppUpdatesURL=https://github.com/flochiii/stutter-watch/releases
DefaultDirName={localappdata}\stutter-watch
DefaultGroupName=Stutter Watch
PrivilegesRequired=lowest
ArchitecturesAllowed=x64os
ArchitecturesInstallIn64BitMode=x64os
LicenseFile={#PackageDir}\LICENSE
SetupIconFile=..\..\assets\app.ico
UninstallDisplayIcon={app}\stutter-watch.exe
OutputDir={#OutputDir}
OutputBaseFilename=stutter-watch-{#AppVersion}-windows-x64-setup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
CloseApplications=yes
CloseApplicationsFilter=stutter-watch.exe
RestartApplications=no
DisableProgramGroupPage=yes
VersionInfoVersion={#AppVersion}

[Languages]
Name: "en"; MessagesFile: "compiler:Default.isl"
Name: "de"; MessagesFile: "compiler:Languages\German.isl"
Name: "es"; MessagesFile: "compiler:Languages\Spanish.isl"
Name: "fr"; MessagesFile: "compiler:Languages\French.isl"
Name: "pl"; MessagesFile: "compiler:Languages\Polish.isl"
Name: "pt"; MessagesFile: "compiler:Languages\Portuguese.isl"
Name: "ru"; MessagesFile: "compiler:Languages\Russian.isl"
Name: "uk"; MessagesFile: "compiler:Languages\Ukrainian.isl"
Name: "zh"; MessagesFile: "languages\ChineseSimplified.isl"
Name: "ja"; MessagesFile: "compiler:Languages\Japanese.isl"

[Files]
Source: "{#PackageDir}\stutter-watch.exe"; Flags: dontcopy
Source: "{#PackageDir}\*"; DestDir: "{app}"; Excludes: "config.toml,logs\*,*.instance.json,*.startup.json"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{userprograms}\Stutter Watch\Stutter Watch"; Filename: "{app}\stutter-watch.exe"; Parameters: "--config ""{app}\config.toml"" --panel"; WorkingDir: "{app}"; AppUserModelID: "StutterWatch.Desktop"

[Run]
Filename: "{app}\stutter-watch.exe"; Parameters: "--config ""{app}\config.toml"" --panel"; Description: "{cm:LaunchProgram,Stutter Watch}"; Flags: postinstall nowait skipifsilent

[Code]
var
  Prepared, Completed: Boolean;

function Lifecycle(Operation: String; var ExitCode: Integer): Boolean;
begin
  Result := Exec(ExpandConstant('{tmp}\stutter-watch.exe'),
    'maintenance ' + Operation + ' --directory "' + ExpandConstant('{app}') +
    '" --receipt "' + ExpandConstant('{tmp}\update.json') + '"', '', SW_HIDE,
    ewWaitUntilTerminated, ExitCode);
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
var Code: Integer;
begin
  Result := '';
  if Prepared then exit;
  ExtractTemporaryFile('stutter-watch.exe');
  if not Lifecycle('prepare', Code) or (Code <> 0) then
    Result := CustomMessage('UpdateStopFailed')
  else Prepared := True;
end;

procedure CurStepChanged(CurStep: TSetupStep);
var Code: Integer;
begin
  if CurStep = ssPostInstall then begin
    if not Lifecycle('resume', Code) or (Code <> 0) then
      RaiseException(CustomMessage('UpdateResumeFailed'));
    Completed := True;
  end;
end;

procedure DeinitializeSetup;
var Code: Integer;
begin
  if Prepared and not Completed then Lifecycle('rollback', Code);
end;

function InitializeUninstall: Boolean;
var Code: Integer;
begin
  Result := Exec(ExpandConstant('{app}\stutter-watch.exe'),
    'maintenance uninstall --directory "' + ExpandConstant('{app}') +
    '" --receipt "' + ExpandConstant('{tmp}\unused.json') + '"', '', SW_HIDE,
    ewWaitUntilTerminated, Code) and (Code = 0);
  if not Result then MsgBox(CustomMessage('UpdateStopFailed'), mbError, MB_OK);
end;

[CustomMessages]
UpdateStopFailed=Close Stutter Watch and try again. Setup could not finish saving the existing recorder. Your recordings have not been deleted.
UpdateResumeFailed=The recorder could not restart. Close Setup and start Stutter Watch again. Your recordings have not been deleted.
