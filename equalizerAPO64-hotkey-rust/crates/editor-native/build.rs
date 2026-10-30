fn main() {
    println!("cargo:rerun-if-changed=Editor.manifest");
    let manifest = std::path::Path::new("Editor.manifest")
        .canonicalize()
        .expect("Editor.manifest not found");
    println!("cargo:rustc-link-arg-bin=Editor=/MANIFEST:EMBED");
    println!(
        "cargo:rustc-link-arg-bin=Editor=/MANIFESTINPUT:{}",
        manifest.display()
    );
    println!(
        "cargo:rustc-link-arg-bin=Editor=/MANIFESTUAC:level='asInvoker' uiAccess='false'"
    );
}
