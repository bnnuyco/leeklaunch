use serde::Deserialize;
use tauri_plugin_http::reqwest;

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

pub async fn get_client_settings() -> Result<ClientSettingsResponse, Box<dyn std::error::Error>> {
    // FIXME: change to user-defined channel later
    let channel = "production";
    let is_default = channel.eq_ignore_ascii_case("production") || channel.eq_ignore_ascii_case("LIVE");

    let client_settings_url = if is_default {
        "https://clientsettingscdn.roblox.com/v2/client-version/WindowsPlayer".to_string()
    } else {
        format!(
            "https://clientsettingscdn.roblox.com/v2/client-version/WindowsPlayer/channel/{}",
            channel
        )
    };
    
    let response = reqwest::get(client_settings_url).await?.error_for_status()?;
    let body_text = response.text().await?;
    let client_info: ClientSettingsResponse = serde_json::from_str(&body_text)?;

    Ok(client_info)
}

pub async fn get_archive_manifest(client_version_upload: &str) -> Result<Vec<FileInfo>, Box<dyn std::error::Error>> {
    // FIXME: read the last one
    let channel = "production";
    let is_default = channel.eq_ignore_ascii_case("production") || channel.eq_ignore_ascii_case("LIVE");

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

    let parsed_manifest = manifest_content.lines().skip(1).collect::<Vec<_>>().join("\n");
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

