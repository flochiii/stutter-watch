fn main() {
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        let root = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
        let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
        let version = std::env::var("CARGO_PKG_VERSION").unwrap();
        let parts: Vec<_> = ["MAJOR", "MINOR", "PATCH"]
            .map(|part| std::env::var(format!("CARGO_PKG_VERSION_{part}")).unwrap())
            .into_iter()
            .chain(std::iter::once("0".into()))
            .collect();
        let manifest = out.join("app.manifest");
        let template = std::fs::read_to_string(root.join("app.manifest")).unwrap();
        std::fs::write(
            &manifest,
            template.replace("@APP_VERSION@", &parts.join(".")),
        )
        .unwrap();
        println!("cargo:rustc-link-arg-bin=stutter-watch=/MANIFEST:EMBED");
        println!(
            "cargo:rustc-link-arg-bin=stutter-watch=/MANIFESTINPUT:{}",
            manifest.display()
        );
        let icon = root.join("assets/app.ico");
        let rc = out.join("app.rc");
        let res = out.join("app.res");
        let product = std::fs::read_to_string(root.join("assets/product-name.txt")).unwrap();
        let source = format!(
            r#"1 ICON "{}"
1 VERSIONINFO
 FILEVERSION {numeric_version}
 PRODUCTVERSION {numeric_version}
 FILEFLAGSMASK 0x3fL
 FILEFLAGS 0
 FILEOS 0x40004L
 FILETYPE 1
BEGIN
 BLOCK "StringFileInfo"
 BEGIN
  BLOCK "040904b0"
  BEGIN
   VALUE "FileDescription", "{product}\0"
   VALUE "ProductName", "{product}\0"
   VALUE "FileVersion", "{version}\0"
   VALUE "ProductVersion", "{version}\0"
   VALUE "InternalName", "stutter-watch\0"
   VALUE "OriginalFilename", "stutter-watch.exe\0"
  END
 END
 BLOCK "VarFileInfo"
 BEGIN
  VALUE "Translation", 0x409, 1200
 END
END
"#,
            icon.display().to_string().replace('\\', "/"),
            numeric_version = parts.join(",")
        );
        let mut encoded = vec![0xff, 0xfe];
        encoded.extend(source.encode_utf16().flat_map(u16::to_le_bytes));
        std::fs::write(&rc, encoded).unwrap();
        let sdk = std::path::PathBuf::from(std::env::var("ProgramFiles(x86)").unwrap())
            .join("Windows Kits/10/bin");
        let mut versions: Vec<_> = std::fs::read_dir(sdk)
            .expect("Windows SDK")
            .filter_map(Result::ok)
            .map(|v| v.path())
            .filter(|p| p.join("x64/rc.exe").is_file())
            .collect();
        versions.sort();
        let compiler = versions
            .last()
            .expect("Windows SDK resource compiler")
            .join("x64/rc.exe");
        assert!(
            std::process::Command::new(compiler)
                .arg("/nologo")
                .arg("/fo")
                .arg(&res)
                .arg(rc)
                .status()
                .unwrap()
                .success(),
            "compile app icon"
        );
        println!("cargo:rustc-link-arg-bin=stutter-watch={}", res.display());
    }
    println!("cargo:rerun-if-changed=app.manifest");
    println!("cargo:rerun-if-changed=assets/app.ico");
    println!("cargo:rerun-if-changed=assets/product-name.txt");
}
