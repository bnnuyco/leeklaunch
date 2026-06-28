use futures::future::join_all;
use serde::Deserialize;
use std::sync::Arc;
use tauri_plugin_http::reqwest;
use tokio::sync::Semaphore;

#[allow(dead_code)]
#[derive(Deserialize, Debug, Clone)]
pub struct ClientSettingsResponse {
    pub version: String,
    #[serde(rename = "clientVersionUpload")]
    pub client_version_upload: String,
    #[serde(rename = "bootstrapperVersion")]
    pub bootstrapper_version: String,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct FileInfo {
    pub filename: String,
    pub md5_hash: String,
    pub compressed_size: u64,
    pub uncompressed_size: u64,
}

const APP_SETTINGS: &str = "<Settings>
<ContentFolder>content</ContentFolder>
<BaseUrl>http://www.roblox.com</BaseUrl>
</Settings>";

pub async fn get_client_settings() -> Result<ClientSettingsResponse, Box<dyn std::error::Error>> {
    // FIXME: change to user-defined channel later
    let channel = "production";
    let is_default =
        channel.eq_ignore_ascii_case("production") || channel.eq_ignore_ascii_case("LIVE");

    let client_settings_url = if is_default {
        "https://clientsettingscdn.roblox.com/v2/client-version/WindowsPlayer".to_string()
    } else {
        format!(
            "https://clientsettingscdn.roblox.com/v2/client-version/WindowsPlayer/channel/{}",
            channel
        )
    };

    let response = reqwest::get(client_settings_url)
        .await?
        .error_for_status()?;
    let body_text = response.text().await?;
    let client_info: ClientSettingsResponse = serde_json::from_str(&body_text)?;

    Ok(client_info)
}

pub async fn get_archive_manifest(
    client_version_upload: &str,
) -> Result<Vec<FileInfo>, Box<dyn std::error::Error>> {
    // FIXME: read the last one
    let channel = "production";
    let is_default =
        channel.eq_ignore_ascii_case("production") || channel.eq_ignore_ascii_case("LIVE");

    let manifest_url = if !is_default {
        format!(
            "https://setup.rbxcdn.com/channel/common/{}-rbxPkgManifest.txt",
            client_version_upload
        )
    } else {
        format!(
            "https://setup.rbxcdn.com/{}-rbxPkgManifest.txt",
            client_version_upload
        )
    };

    let response = reqwest::get(manifest_url).await?.error_for_status()?;
    let manifest_content = response.text().await?;

    if manifest_content.lines().next() != Some("v0") {
        return Err("Invalid manifest version".into());
    }

    let parsed_manifest = manifest_content
        .lines()
        .skip(1)
        .collect::<Vec<_>>()
        .join("\n");
    let mut file_list = Vec::new();
    let mut lines = parsed_manifest.lines();
    loop {
        let filename = match lines.next() {
            Some(line) => line,
            None => break,
        };
        let md5_hash = match lines.next() {
            Some(line) => line,
            None => break,
        };
        let compressed_size = match lines.next() {
            Some(line) => line,
            None => break,
        };
        let uncompressed_size = match lines.next() {
            Some(line) => line,
            None => break,
        };

        file_list.push(FileInfo {
            filename: filename.to_string(),
            md5_hash: md5_hash.to_string(),
            compressed_size: compressed_size.parse()?,
            uncompressed_size: uncompressed_size.parse()?,
        });
    }

    Ok(file_list)
}

pub async fn save_deployment(
    client_version_upload: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let files = get_archive_manifest(client_version_upload).await?;
    let app_config_dir = crate::config::get_app_config_dir()?;
    // FIXME: move every path related hardcode into a config.rs path map
    let install_dir = app_config_dir.join("versions").join("player");
    let cache_dir = app_config_dir.join("cache");

    if install_dir.exists() {
        println!(
            "Performing clean installation: deleting {}",
            install_dir.display()
        );
        if let Err(e) = std::fs::remove_dir_all(&install_dir) {
            eprintln!(
                "Failed to clean installation directory {}: {}",
                install_dir.display(),
                e
            );
        }
    }

    std::fs::create_dir_all(&install_dir)?;
    std::fs::create_dir_all(&cache_dir)?;

    println!(
        "Installing version {} to {}",
        client_version_upload,
        install_dir.display()
    );

    let semaphore = Arc::new(Semaphore::new(4));
    let mut futures = Vec::new();

    for file in &files {
        let sem = Arc::clone(&semaphore);
        let version = client_version_upload;
        let install_path = &install_dir;
        let cache_path = &cache_dir;

        let fut = async move {
            let _permit = sem.acquire().await.unwrap();

            let cache_file_path = cache_path.join(file.md5_hash.to_lowercase());
            let mut cached_bytes = None;

            if cache_file_path.exists() {
                if let Ok(b) = std::fs::read(&cache_file_path) {
                    let computed_hash = format!("{:x}", md5::compute(&b));
                    if computed_hash.to_lowercase() == file.md5_hash.to_lowercase() {
                        println!("Using cached package for {}", file.filename);
                        cached_bytes = Some(b);
                    } else {
                        println!(
                            "Cached package for {} is corrupted, re-downloading...",
                            file.filename
                        );
                        let _ = std::fs::remove_file(&cache_file_path);
                    }
                }
            }

            let bytes = match cached_bytes {
                // The cache has the package, use the cached file
                Some(b) => b,
                None => {
                    let archive_url =
                        format!("https://setup.rbxcdn.com/{}-{}", version, file.filename);
                    let max_attempts = 5;
                    let mut attempts = 0;

                    let downloaded_bytes = loop {
                        attempts += 1;
                        let attempt_result = async {
                            let response = reqwest::get(&archive_url)
                                .await
                                .map_err(|e| e.to_string())?
                                .error_for_status()
                                .map_err(|e| e.to_string())?;
                            let b = response.bytes().await.map_err(|e| e.to_string())?;
                            let computed_hash = format!("{:x}", md5::compute(&b));
                            if computed_hash.to_lowercase() != file.md5_hash.to_lowercase() {
                                return Err(format!(
                                    "MD5 checksum failed: expected {}, got {}",
                                    file.md5_hash, computed_hash
                                ));
                            }
                            Ok::<_, String>(b.to_vec())
                        }
                        .await;

                        match attempt_result {
                            Ok(b) => break b,
                            Err(e) => {
                                eprintln!(
                                    "Attempt {}/{} failed to download {}: {}",
                                    attempts, max_attempts, file.filename, e
                                );
                                if attempts >= max_attempts {
                                    eprintln!(
                                        "Failed to download {} after {} attempts. Aborting.",
                                        file.filename, max_attempts
                                    );
                                    return Err("Download failed after maximum attempts".into());
                                }
                                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                            }
                        }
                    };

                    if let Err(e) = std::fs::write(&cache_file_path, &downloaded_bytes) {
                        eprintln!("Failed to write cache file for {}: {}", file.filename, e);
                    }
                    downloaded_bytes
                }
            };

            let reader = std::io::Cursor::new(&bytes);
            let extraction_root = get_extraction_root(&file.filename);

            match zip::ZipArchive::new(reader) {
                Ok(mut archive) => {
                    let extract_path = install_path.join(extraction_root);
                    println!(
                        "Extracting {} to {}...",
                        file.filename,
                        extract_path.display()
                    );

                    for i in 0..archive.len() {
                        let mut zip_file = match archive.by_index(i) {
                            Ok(f) => f,
                            Err(_) => continue,
                        };
                        let outpath = match zip_file.enclosed_name() {
                            Some(path) => extract_path.join(path),
                            None => continue,
                        };

                        if zip_file.is_dir() {
                            let _ = std::fs::create_dir_all(&outpath);
                        } else {
                            if let Some(p) = outpath.parent() {
                                let _ = std::fs::create_dir_all(p);
                            }
                            if let Ok(mut outfile) = std::fs::File::create(&outpath) {
                                let _ = std::io::copy(&mut zip_file, &mut outfile);
                            }
                        }
                    }
                }
                Err(_) => {
                    let outpath = install_path.join(&file.filename);
                    if let Some(p) = outpath.parent() {
                        let _ = std::fs::create_dir_all(p);
                    }
                    let _ = std::fs::write(&outpath, &bytes);
                }
            }
            Ok::<(), Box<dyn std::error::Error>>(())
        };

        futures.push(fut);
    }

    let results = join_all(futures).await;
    for res in results {
        res?;
    }

    let app_settings_path = install_dir.join("AppSettings.xml");
    std::fs::write(&app_settings_path, APP_SETTINGS)?;
    println!("AppSettings.xml written to {}", app_settings_path.display());

    write_deployment_state(client_version_upload, files)?;

    if let Err(e) = cleanup_package_cache(&cache_dir) {
        eprintln!("Failed to clean up package cache: {}", e);
    }

    Ok(())
}

fn write_deployment_state(
    client_version_upload: &str,
    files: Vec<FileInfo>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut hash_tree = std::collections::HashMap::new();
    for file in files {
        hash_tree.insert(file.filename, file.md5_hash);
    }

    let mut state = crate::config::RobloxState::load()?;
    state.player = crate::config::PlayerState {
        version_guid: client_version_upload.to_string(),
        hash_tree,
    };
    state.save()?;
    Ok(())
}

fn cleanup_package_cache(cache_dir: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    let state = crate::config::RobloxState::load()?;
    let mut valid_hashes = std::collections::HashSet::new();
    for hash in state.player.hash_tree.values() {
        valid_hashes.insert(hash.to_lowercase());
    }

    if cache_dir.exists() {
        for entry in std::fs::read_dir(cache_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_file() {
                if let Some(filename_str) = path.file_name().and_then(|s| s.to_str()) {
                    let filename_lower = filename_str.to_lowercase();
                    if filename_lower.len() == 32
                        && filename_lower.chars().all(|c| c.is_ascii_hexdigit())
                    {
                        // Delete unused cache that aren't in the new version manifest
                        if !valid_hashes.contains(&filename_lower) {
                            println!("Deleting unused cached package: {}", filename_lower);
                            let _ = std::fs::remove_file(&path);
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

pub fn get_extraction_root(filename: &str) -> &'static str {
    // FIXME: move this to a remote mapping like everyone else
    match filename {
        "RobloxApp.zip" => "./",
        "redist.zip" => "./",
        "shaders.zip" => "./shaders",
        "ssl.zip" => "./ssl",
        "WebView2.zip" => "./",
        "WebView2RuntimeInstaller.zip" => "./WebView2RuntimeInstaller",
        "content-avatar.zip" => "./content/avatar",
        "content-configs.zip" => "./content/configs",
        "content-fonts.zip" => "./content/fonts",
        "content-sky.zip" => "./content/sky",
        "content-sounds.zip" => "./content/sounds",
        "content-textures2.zip" => "./content/textures",
        "content-models.zip" => "./content/models",
        "content-platform-fonts.zip" => "./PlatformContent/pc/fonts",
        "content-platform-dictionaries.zip" => {
            "./PlatformContent/pc/shared_compression_dictionaries"
        }
        "content-terrain.zip" => "./PlatformContent/pc/terrain",
        "content-textures3.zip" => "./PlatformContent/pc/textures",
        "extracontent-places.zip" => "./ExtraContent/places",
        "extracontent-luapackages.zip" => "./ExtraContent/LuaPackages",
        "extracontent-translations.zip" => "./ExtraContent/translations",
        "extracontent-models.zip" => "./ExtraContent/models",
        "extracontent-textures.zip" => "./ExtraContent/textures",
        _ => "./",
    }
}
