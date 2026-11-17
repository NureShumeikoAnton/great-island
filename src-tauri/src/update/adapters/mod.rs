//! The Windows and disk side of the updates: the two small files, fetching from the network
//! (`http`) and running an installer or the update window (`process`). They return `Result<_, Failed>`
//! and decide nothing: what a release means is in `crate::update`.

pub mod http;
pub mod process;

use super::{Manifest, Memory, Status};
use crate::{fault::Failed, persist};
use std::{fs, path::Path};

/// What a check may fetch at most: a manifest is a few KB.
const MANIFEST_LIMIT: usize = 256 * 1024;

pub fn manifest_url() -> String {
    // For trying an update without a release: a local server.
    std::env::var("GREAT_ISLAND_UPDATE_URL").unwrap_or_else(|_| super::MANIFEST_URL.to_string())
}

pub fn fetch_manifest() -> Result<Manifest, Failed> {
    let bytes = http::get(&manifest_url(), MANIFEST_LIMIT)?;
    Manifest::parse(&String::from_utf8_lossy(&bytes))
}

pub fn load_memory() -> Memory {
    read(&persist::update_memory_path()).unwrap_or_default()
}

pub fn save_memory(memory: &Memory) -> Result<(), Failed> {
    write(&persist::update_memory_path(), memory)
}

/// What the views read. A missing or damaged file is "nothing is known".
pub fn load_status() -> Status {
    read(&persist::update_status_path()).unwrap_or_default()
}

pub fn save_status(status: &Status) -> Result<(), Failed> {
    write(&persist::update_status_path(), status)
}

pub fn load_offer() -> Option<Manifest> {
    read(&persist::update_offer_path())
}

pub fn save_offer(manifest: &Manifest) -> Result<(), Failed> {
    write(&persist::update_offer_path(), manifest)
}

pub fn clear_offer() {
    let _ = fs::remove_file(persist::update_offer_path());
}

pub fn download_dir() -> std::path::PathBuf {
    std::env::temp_dir().join("Great Island").join("Update")
}

fn read<T: serde::de::DeserializeOwned>(path: &Path) -> Option<T> {
    serde_json::from_str(&fs::read_to_string(path).ok()?).ok()
}

/// Through a temp file: the island and the update window both write these, and a cut-short write must not leave half a file.
fn write<T: serde::Serialize>(path: &Path, value: &T) -> Result<(), Failed> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    let text = serde_json::to_string(value)
        .map_err(|e| Failed::because(crate::fault::Cause::Unknown, e))?;
    fs::write(&tmp, text)?;
    fs::rename(tmp, path)?;
    Ok(())
}
