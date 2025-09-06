use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::process::Command;
use std::fs;
use std::io::Write;
use serde_json::Value;
use std::collections::HashMap;
use zip::ZipArchive;
use std::fs::File;

#[derive(Deserialize, Serialize)]
struct VersionManifest {
    versions: Vec<VersionInfo>,
}

#[derive(Deserialize, Serialize)]
struct VersionInfo {
    id: String,
    url: String,
}

#[derive(Deserialize, Serialize)]
pub struct AssetIndex {
    pub id: String,
    pub url: String,
}

#[derive(Deserialize, Serialize)]
pub struct VersionDetail {
    pub downloads: Downloads,
    pub mainClass: String,
    pub libraries: Vec<Library>,
    pub arguments: Option<Arguments>,
    pub assetIndex: AssetIndex,
}

#[derive(Deserialize, Serialize)]
struct Downloads {
    client: DownloadInfo,
}

#[derive(Deserialize, Serialize)]
struct DownloadInfo {
    url: String,
}

#[derive(Deserialize, Serialize)]
pub struct Library {
    pub downloads: LibraryDownloads,
    pub name: String,
}

#[derive(Deserialize, Serialize)]
pub struct LibraryDownloads {
    pub artifact: Option<DownloadInfo>,
    pub classifiers: Option<HashMap<String, DownloadInfo>>,
}

#[derive(Deserialize, Serialize)]
pub struct Arguments {
    pub game: Option<Vec<serde_json::Value>>,
    pub jvm: Option<Vec<serde_json::Value>>,
}

pub fn fetch_versions() -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let url = "https://launchermeta.mojang.com/mc/game/version_manifest.json";
    let resp = reqwest::blocking::get(url)?.json::<VersionManifest>()?;
    Ok(resp.versions.into_iter().map(|v| v.id).collect())
}

// New function: fetch_versions_with_urls
pub fn fetch_versions_with_urls() -> Result<Vec<(String, String)>, Box<dyn std::error::Error>> {
    let url = "https://launchermeta.mojang.com/mc/game/version_manifest.json";
    let resp = reqwest::blocking::get(url)?.json::<VersionManifest>()?;
    Ok(resp.versions.into_iter().map(|v| (v.id, v.url)).collect())
}

pub fn detect_java() -> Vec<String> {
    let mut paths = Vec::new();

    // Try "java -version" to see if Java is in PATH
    if let Ok(output) = Command::new("java").arg("-version").output() {
        if output.status.success() {
            paths.push("java (from PATH)".to_string());
        }
    }

    // On Windows, check common install locations
    #[cfg(windows)]
    {
        let program_files = std::env::var("ProgramFiles").unwrap_or_default();
        let java_dir = format!("{}\\Java", program_files);
        if let Ok(entries) = std::fs::read_dir(java_dir) {
            for entry in entries.flatten() {
                let path = entry.path().join("bin").join("java.exe");
                if path.exists() {
                    paths.push(path.display().to_string());
                }
            }
        }
    }

    paths
}

// Download the client jar and version json for the selected version
pub fn download_version_files(version_id: &str, version_url: &str) -> Result<(), Box<dyn std::error::Error>> {
    // Download version JSON
    let version_json: VersionDetail = reqwest::blocking::get(version_url)?.json()?;

    // Download client jar
    let client_url = &version_json.downloads.client.url;
    let client_bytes = reqwest::blocking::get(client_url)?.bytes()?;

    // Save files
    fs::create_dir_all("minecraft")?;
    let mut jar_file = fs::File::create(format!("minecraft/{}.jar", version_id))?;
    jar_file.write_all(&client_bytes)?;

    let version_json_str = serde_json::to_string_pretty(&version_json)?;
    let mut json_file = fs::File::create(format!("minecraft/{}.json", version_id))?;
    json_file.write_all(version_json_str.as_bytes())?;

    Ok(())
}

pub fn download_libraries(libraries: &[Library]) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let mut paths = Vec::new();
    fs::create_dir_all("minecraft/libs")?;
    for lib in libraries {
        if let Some(artifact) = &lib.downloads.artifact {
            let url = &artifact.url;
            let name = lib.name.replace(":", "-");
            let path = format!("minecraft/libs/{}.jar", name);
            if !std::path::Path::new(&path).exists() {
                let bytes = reqwest::blocking::get(url)?.bytes()?;
                let mut file = fs::File::create(&path)?;
                file.write_all(&bytes)?;
            }
            paths.push(path);
        }
    }
    Ok(paths)
}

pub fn download_assets(asset_index: &AssetIndex) -> Result<(), Box<dyn std::error::Error>> {
    let asset_index_json: Value = reqwest::blocking::get(&asset_index.url)?.json()?;
    let objects = asset_index_json["objects"].as_object().unwrap();

    fs::create_dir_all("minecraft/assets/objects")?;
    fs::create_dir_all("minecraft/assets/indexes")?;

    // Save asset index JSON
    let index_path = format!("minecraft/assets/indexes/{}.json", asset_index.id);
    let mut index_file = fs::File::create(&index_path)?;
    index_file.write_all(serde_json::to_string_pretty(&asset_index_json)?.as_bytes())?;

    // Collect asset info into a Vec for parallel processing
    let assets: Vec<(String, String, String)> = objects.iter().map(|(_name, obj)| {
        let hash = obj["hash"].as_str().unwrap().to_string();
        let subdir = hash[0..2].to_string();
        let url = format!("https://resources.download.minecraft.net/{}/{}", subdir, hash);
        let path = format!("minecraft/assets/objects/{}/{}", subdir, hash);
        (url, path, subdir)
    }).collect();

    // Download in parallel using threads
    let handles: Vec<_> = assets.into_iter().map(|(url, path, subdir)| {
        std::thread::spawn(move || {
            if !std::path::Path::new(&path).exists() {
                let _ = fs::create_dir_all(format!("minecraft/assets/objects/{}", subdir));
                if let Ok(bytes) = reqwest::blocking::get(&url).and_then(|r| r.bytes()) {
                    if let Ok(mut file) = fs::File::create(&path) {
                        let _ = file.write_all(&bytes);
                    }
                }
            }
        })
    }).collect();

    for handle in handles {
        let _ = handle.join();
    }

    // For legacy versions, create virtual/legacy structure
    let virtual_dir = "minecraft/assets/virtual/legacy";
    std::fs::create_dir_all(virtual_dir)?;

    for (name, obj) in objects {
        let hash = obj["hash"].as_str().unwrap();
        let subdir = &hash[0..2];
        let object_path = format!("minecraft/assets/objects/{}/{}", subdir, hash);
        let virtual_path = format!("{}/{}", virtual_dir, name);

        // Create parent directories for virtual path
        if let Some(parent) = std::path::Path::new(&virtual_path).parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Only copy if the source exists
        if std::path::Path::new(&object_path).exists() && !std::path::Path::new(&virtual_path).exists() {
            std::fs::copy(&object_path, &virtual_path)?;
        }
    }

    Ok(())
}

pub fn extract_natives(libraries: &[Library]) -> Result<(), Box<dyn std::error::Error>> {
    fs::create_dir_all("minecraft/natives")?;
    for lib in libraries {
        if let Some(native) = lib.downloads.classifiers.as_ref().and_then(|c| c.get("natives-windows")) {
            let url = &native.url;
            let name = lib.name.replace(":", "-");
            let path = format!("minecraft/libs/{}.jar", name);
            if !std::path::Path::new(&path).exists() {
                let bytes = reqwest::blocking::get(url)?.bytes()?;
                let mut file = fs::File::create(&path)?;
                file.write_all(&bytes)?;
            }
            // Extract DLLs
            let file = File::open(&path)?;
            let mut archive = ZipArchive::new(file)?;
            for i in 0..archive.len() {
                let mut file = archive.by_index(i)?;
                let outpath = match file.enclosed_name() {
                    Some(p) => p.to_owned(),
                    None => continue,
                };
                if outpath.extension().map_or(false, |e| e == "dll") {
                    let mut outfile = File::create(format!("minecraft/natives/{}", outpath.file_name().unwrap().to_string_lossy()))?;
                    std::io::copy(&mut file, &mut outfile)?;
                }
            }
        }
    }
    Ok(())
}