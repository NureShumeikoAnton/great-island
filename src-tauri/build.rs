fn main() {
    // The texts are embedded by a macro that cargo does not track on its own.
    println!("cargo:rerun-if-changed=locales");
    tauri_build::build()
}
