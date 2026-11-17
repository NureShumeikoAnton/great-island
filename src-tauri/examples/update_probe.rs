//! The network side of an update against a real server:
//! `GREAT_ISLAND_UPDATE_URL=http://127.0.0.1:8123/latest.json cargo run --example update_probe -- [public key, base64]`.
//! Fetches the manifest, downloads the installer it names and checks its signature (with the built-in
//! key, or the one given).

use great_island_lib::update::{
    adapters::{self, http},
    signature,
};

fn main() {
    let key = std::env::args().nth(1);
    let manifest = adapters::fetch_manifest().expect("the manifest");
    println!(
        "manifest: version {}, {}",
        manifest.version, manifest.installer.url
    );
    let path = adapters::download_dir().join("probe-installer.exe");
    let mut shown = 0;
    http::download(&manifest.installer.url, &path, |done, total| {
        if done - shown >= 256 * 1024 {
            shown = done;
            println!("  {done} of {total}");
        }
    })
    .expect("the installer");
    let bytes = std::fs::read(&path).expect("the file");
    println!("downloaded {} bytes (announced {})", bytes.len(), manifest.installer.size);
    let checked = match &key {
        Some(key) => signature::verify_with(key, &bytes, &manifest.installer.signature),
        None => signature::verify(&bytes, &manifest.installer.signature),
    };
    match checked {
        Ok(()) => println!("signature OK"),
        Err(failed) => println!("signature NOT valid: {}", failed.detail),
    }
    let _ = std::fs::remove_file(path);
}
