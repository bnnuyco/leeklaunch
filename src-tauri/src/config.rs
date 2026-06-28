use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct PlayerState {
    pub version_guid: String,
    pub hash_tree: HashMap<String, String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct RobloxState {
    pub player: PlayerState,
}

impl RobloxState {
    pub fn load() -> Result<Self, Box<dyn std::error::Error>> {
        let path = get_roblox_state_file_path()?;
        if !path.exists() {
            return Ok(RobloxState::default());
        }
        let data = fs::read_to_string(&path)?;
        let state: RobloxState = serde_json::from_str(&data)?;
        Ok(state)
    }

    pub fn save(&self) -> Result<(), Box<dyn std::error::Error>> {
        let path = get_roblox_state_file_path()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let data = serde_json::to_string_pretty(self)?;
        fs::write(&path, data)?;
        Ok(())
    }
}

pub fn get_app_config_dir() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let base_dir = if cfg!(target_os = "windows") {
        std::env::var("APPDATA").map(PathBuf::from)?
    } else {
        let home = std::env::var("HOME").map(PathBuf::from)?;
        home.join(".config")
    };

    Ok(base_dir.join("leeklaunch"))
}

pub fn get_roblox_state_file_path() -> Result<PathBuf, Box<dyn std::error::Error>> {
    Ok(get_app_config_dir()?.join("roblox_state.json"))
}
