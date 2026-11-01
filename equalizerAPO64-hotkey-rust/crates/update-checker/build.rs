fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    eapo_winicon::embed_icon("UpdateChecker", "../../assets/icons/UpdateChecker.ico");
}
