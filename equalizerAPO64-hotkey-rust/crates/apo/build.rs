use std::path::{Path, PathBuf};
use std::process::Command;

fn vswhere_install() -> PathBuf {
    let vswhere = Path::new(r"C:\Program Files (x86)\Microsoft Visual Studio\Installer\vswhere.exe");
    let output = Command::new(vswhere)
        .args([
            "-latest",
            "-requires",
            "Microsoft.VisualStudio.Component.VC.Tools.x86.x64",
            "-property",
            "installationPath",
        ])
        .output()
        .expect("vswhere.exe failed to run; install the MSVC C++ toolchain");
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !output.status.success() || text.is_empty() {
        panic!("no Visual Studio installation with the C++ toolchain was found");
    }
    PathBuf::from(text)
}

fn newest_msvc_tools(vs_root: &Path) -> PathBuf {
    let tools_root = vs_root.join(r"VC\Tools\MSVC");
    let mut versions: Vec<_> = std::fs::read_dir(&tools_root)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", tools_root.display()))
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|p| p.join(r"bin\Hostx64\x64\cl.exe").is_file())
        .collect();
    versions.sort();
    versions.pop().expect("no MSVC toolset with cl.exe found")
}

fn run(program: &Path, args: &[String]) {
    let output = Command::new(program)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("failed to run {}: {e}", program.display()));
    if !output.status.success() {
        panic!(
            "{} failed\nstdout:\n{}\nstderr:\n{}",
            program.display(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

fn kits_root_and_version() -> (PathBuf, String) {
    // Prefer the INCLUDE environment set up by a Visual Studio prompt.
    if let Ok(include) = std::env::var("INCLUDE") {
        for dir in include.split(';') {
            let dir = PathBuf::from(dir);
            if dir
                .file_name()
                .is_some_and(|name| name.eq_ignore_ascii_case("um"))
            {
                if let Some(version) = dir
                    .parent()
                    .and_then(|parent| parent.file_name())
                    .and_then(|name| name.to_str())
                {
                    if let Some(kits) = dir
                        .parent()
                        .and_then(|parent| parent.parent())
                        .and_then(|parent| parent.parent())
                    {
                        return (kits.to_path_buf(), version.to_string());
                    }
                }
            }
        }
    }
    let mut candidates: Vec<PathBuf> = vec![r"C:\Program Files (x86)\Windows Kits\10".into()];
    for drive in ["D", "E", "F"] {
        candidates.push(PathBuf::from(format!(r"{drive}:\Windows Kits\10")));
    }
    for kits in candidates {
        let include_root = kits.join("Include");
        let Ok(entries) = std::fs::read_dir(&include_root) else {
            continue;
        };
        let mut versions: Vec<String> = entries
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                let path = entry.path();
                path.join("um").is_dir() && path.join("ucrt").is_dir()
            })
            .filter_map(|entry| entry.file_name().to_str().map(|name| name.to_string()))
            .collect();
        versions.sort();
        if let Some(version) = versions.pop() {
            return (kits, version);
        }
    }
    (
        PathBuf::from(r"C:\Program Files (x86)\Windows Kits\10"),
        "10.0.26100.0".into(),
    )
}

fn main() {
    println!("cargo:rerun-if-changed=src/cxx/apo_shim.cpp");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_LIVE_PROBE");

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let (kits, kit_version) = kits_root_and_version();

    let msvc = newest_msvc_tools(&vswhere_install());
    let cl = msvc.join(r"bin\Hostx64\x64\cl.exe");
    let lib_exe = msvc.join(r"bin\Hostx64\x64\lib.exe");

    let mut args: Vec<String> = vec![
        "/c".into(),
        "/nologo".into(),
        "/EHsc".into(),
        "/std:c++17".into(),
        "/MD".into(),
        "/O2".into(),
        format!("/Fo{}", out_dir.join("apo_shim.obj").display()),
    ];
    for include in [
        msvc.join("include"),
        kits.join(format!(r"Include\{kit_version}\ucrt")),
        kits.join(format!(r"Include\{kit_version}\shared")),
        kits.join(format!(r"Include\{kit_version}\um")),
    ] {
        args.push(format!("/I{}", include.display()));
    }
    if std::env::var_os("CARGO_FEATURE_LIVE_PROBE").is_some() {
        args.push("/DEAPO_LIVE_PROBE".into());
    }
    args.push("src/cxx/apo_shim.cpp".into());
    run(&cl, &args);

    let shim_lib = out_dir.join("eapo_cxx_shim.lib");
    run(
        &lib_exe,
        &[
            "/nologo".into(),
            format!("/OUT:{}", shim_lib.display()),
            out_dir.join("apo_shim.obj").to_string_lossy().into_owned(),
        ],
    );

    let module_def = out_dir.join("exports.def");
    std::fs::write(
        &module_def,
        "LIBRARY EqualizerAPO\nEXPORTS\n\
         \x20   DllCanUnloadNow\n\
         \x20   DllGetClassObject\n\
         \x20   DllRegisterServer\n\
         \x20   DllUnregisterServer\n",
    )
    .expect("failed to write the module definition file");
    println!("cargo:rustc-cdylib-link-arg=/DEF:{}", module_def.display());

    println!(
        r"cargo:rustc-link-search=native={}",
        kits.join(format!(r"Lib\{kit_version}\um\x64")).display()
    );
    println!("cargo:rustc-link-search=native={}", out_dir.display());
    for l in [
        "audiobaseprocessingobject",
        "audioeng",
        "audiomediatypecrt",
        "ksuser",
        "ole32",
        "oleaut32",
        "uuid",
        "advapi32",
        "shlwapi",
        "eapo_cxx_shim",
    ] {
        println!("cargo:rustc-link-lib={l}");
    }
}
