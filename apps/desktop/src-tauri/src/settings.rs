use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopSettings {
    pub port: u16,
    pub storage_dir: String,
    pub show_qr_in_tray: bool,
    pub auto_clean_on_quit: bool,
    pub notify_on_device_connect: bool,
    #[serde(default = "default_true")]
    pub notify_on_new_drop: bool,
    #[serde(default)]
    pub pin: String,
    #[serde(default)]
    pub expire_minutes: u32,
    /// macOS only: show the Dock icon (Regular activation policy). Off by
    /// default — DropLocal lives in the menu bar.
    #[serde(default)]
    pub show_dock_icon: bool,
    #[serde(default)]
    pub launch_at_login: bool,
}

impl Default for DesktopSettings {
    fn default() -> Self {
        let default_storage = dirs::download_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("DropLocal");

        Self {
            // 0 = automatic: try 80 (portless URL), then 3000 with upward scan.
            port: 0,
            storage_dir: default_storage.to_string_lossy().to_string(),
            show_qr_in_tray: true,
            auto_clean_on_quit: false,
            notify_on_device_connect: false,
            notify_on_new_drop: true,
            pin: String::new(),
            expire_minutes: 0,
            show_dock_icon: false,
            launch_at_login: false,
        }
    }
}

fn default_true() -> bool {
    true
}

impl DesktopSettings {
    pub fn validated(mut self) -> Self {
        if self.storage_dir.trim().is_empty() {
            self.storage_dir = DesktopSettings::default().storage_dir;
        }

        self.pin = self.pin.trim().to_string();

        self
    }

    pub fn resolved_storage_dir(&self) -> PathBuf {
        expand_home(self.storage_dir.trim())
    }
}

pub async fn load_or_default(settings_path: &Path) -> anyhow::Result<DesktopSettings> {
    match tokio::fs::read_to_string(settings_path).await {
        Ok(raw) => {
            let parsed = serde_json::from_str::<DesktopSettings>(&raw)?;
            Ok(parsed.validated())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let settings = DesktopSettings::default();
            save(settings_path, &settings).await?;
            Ok(settings)
        }
        Err(error) => Err(error.into()),
    }
}

pub async fn save(settings_path: &Path, settings: &DesktopSettings) -> anyhow::Result<()> {
    if let Some(parent) = settings_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let serialized = serde_json::to_string_pretty(settings)?;
    tokio::fs::write(settings_path, serialized).await?;
    Ok(())
}

fn expand_home(raw: &str) -> PathBuf {
    if raw == "~" {
        return dirs::home_dir().unwrap_or_else(std::env::temp_dir);
    }

    if let Some(stripped) = raw.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(stripped);
        }
    }

    PathBuf::from(raw)
}

#[cfg(test)]
mod tests {
    use super::DesktopSettings;

    #[test]
    fn settings_resolve_home() {
        let settings = DesktopSettings {
            storage_dir: "~/droplocal".to_string(),
            ..DesktopSettings::default()
        };

        let path = settings.resolved_storage_dir();
        assert!(path.ends_with("droplocal"));
    }

    #[test]
    fn settings_parse_pre_1_2_file() {
        // A settings.json written by <= 1.1.x has no menu-bar fields.
        let raw = r#"{
            "port": 0,
            "storageDir": "~/Downloads/DropLocal",
            "showQrInTray": true,
            "autoCleanOnQuit": false,
            "notifyOnDeviceConnect": false
        }"#;
        let parsed: DesktopSettings = serde_json::from_str(raw).expect("parse old settings");
        assert!(!parsed.show_dock_icon);
        assert!(!parsed.launch_at_login);
    }
}
