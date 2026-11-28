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
CloseApplications=no
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
de.UpdateStopFailed=Schließen Sie Stutter Watch und versuchen Sie es erneut. Die Aufnahme konnte nicht vollständig gespeichert werden. Ihre Aufnahmen wurden nicht gelöscht.
de.UpdateResumeFailed=Die Aufzeichnung konnte nicht neu gestartet werden. Schließen Sie das Setup und starten Sie Stutter Watch erneut. Ihre Aufnahmen wurden nicht gelöscht.
es.UpdateStopFailed=Cierra Stutter Watch e inténtalo de nuevo. No se pudo terminar de guardar la grabación. Tus grabaciones no se han eliminado.
es.UpdateResumeFailed=No se pudo reiniciar la grabación. Cierra el instalador y abre Stutter Watch de nuevo. Tus grabaciones no se han eliminado.
fr.UpdateStopFailed=Fermez Stutter Watch et réessayez. L’enregistrement n’a pas pu être entièrement sauvegardé. Vos enregistrements n’ont pas été supprimés.
fr.UpdateResumeFailed=L’enregistrement n’a pas pu redémarrer. Fermez l’installation et relancez Stutter Watch. Vos enregistrements n’ont pas été supprimés.
pl.UpdateStopFailed=Zamknij Stutter Watch i spróbuj ponownie. Nie udało się dokończyć zapisywania nagrania. Nagrania nie zostały usunięte.
pl.UpdateResumeFailed=Nie udało się ponownie uruchomić nagrywania. Zamknij instalator i uruchom Stutter Watch ponownie. Nagrania nie zostały usunięte.
pt.UpdateStopFailed=Feche o Stutter Watch e tente novamente. Não foi possível concluir a gravação. As suas gravações não foram eliminadas.
pt.UpdateResumeFailed=Não foi possível reiniciar a gravação. Feche o instalador e abra o Stutter Watch novamente. As suas gravações não foram eliminadas.
ru.UpdateStopFailed=Закройте Stutter Watch и повторите попытку. Не удалось завершить сохранение записи. Ваши записи не удалены.
ru.UpdateResumeFailed=Не удалось возобновить запись. Закройте установщик и запустите Stutter Watch снова. Ваши записи не удалены.
uk.UpdateStopFailed=Закрийте Stutter Watch і спробуйте ще раз. Не вдалося завершити збереження запису. Ваші записи не видалено.
uk.UpdateResumeFailed=Не вдалося відновити запис. Закрийте інсталятор і запустіть Stutter Watch знову. Ваші записи не видалено.
zh.UpdateStopFailed=请关闭 Stutter Watch 后重试。无法完成保存当前记录。您的记录未被删除。
zh.UpdateResumeFailed=无法重新开始记录。请关闭安装程序并重新启动 Stutter Watch。您的记录未被删除。
ja.UpdateStopFailed=Stutter Watch を閉じてから再試行してください。記録の保存を完了できませんでした。記録は削除されていません。
ja.UpdateResumeFailed=記録を再開できませんでした。セットアップを閉じて Stutter Watch を再起動してください。記録は削除されていません。
