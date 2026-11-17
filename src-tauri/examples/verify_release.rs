//! Checks a signed installer the way the island does:
//! `cargo run --example verify_release -- <installer> <installer.sig> [public key, base64]`.
//! Used by `scripts/release.mjs` before a release is uploaded.

use great_island_lib::update::signature;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [installer, sig, rest @ ..] = args.as_slice() else {
        eprintln!("usage: verify_release <installer> <installer.sig> [public key]");
        std::process::exit(2);
    };
    let bytes = std::fs::read(installer).expect("the installer is readable");
    let signature_text = std::fs::read_to_string(sig).expect("the .sig file is readable");
    let checked = match rest.first() {
        Some(key) => signature::verify_with(key, &bytes, &signature_text),
        None => signature::verify(&bytes, &signature_text),
    };
    match checked {
        Ok(()) => println!("signature OK ({} bytes)", bytes.len()),
        Err(failed) => {
            eprintln!("signature NOT valid: {}", failed.detail);
            std::process::exit(1);
        }
    }
}
