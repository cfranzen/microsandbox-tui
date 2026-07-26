//! User-level configuration file for default sandbox parameters.
//!
//! On startup the app looks for a TOML config file at the platform config
//! directory (e.g. `~/.config/microsandbox-tui/config.toml` on Linux) and
//! uses any values found there to prefill the "New Sandbox" dialog. A
//! missing file is not an error — the app simply falls back to its built-in
//! defaults.

use serde::Deserialize;
use std::path::PathBuf;

/// Default sandbox parameters loaded from the user's config file.
///
/// Every field is optional; only the fields present in the file override the
/// create-dialog's built-in defaults.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct AppConfig {
    /// Default container image (e.g. `"alpine"`).
    pub image: Option<String>,
    /// Default number of CPUs.
    pub cpus: Option<u32>,
    /// Default memory size in MiB.
    pub memory_mib: Option<u32>,
    /// Default hostname.
    pub hostname: Option<String>,
    /// Default working directory inside the sandbox.
    pub workdir: Option<String>,
    /// Default user to run as inside the sandbox.
    pub user: Option<String>,
    /// Default shell path (e.g. `"/bin/sh"`).
    pub shell: Option<String>,
}

impl AppConfig {
    /// Load the config file from the platform config directory, falling
    /// back to [`AppConfig::default`] (all fields `None`) when the file is
    /// missing or cannot be parsed.
    pub fn load() -> Self {
        Self::config_path()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .and_then(|contents| toml::from_str(&contents).ok())
            .unwrap_or_default()
    }

    /// Parse config contents directly (used by tests and by [`Self::load`]).
    pub fn parse(contents: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(contents)
    }

    /// Path to the config file: `<config_dir>/microsandbox-tui/config.toml`.
    pub fn config_path() -> Option<PathBuf> {
        dirs::config_dir().map(|dir| dir.join("microsandbox-tui").join("config.toml"))
    }
}

//--------------------------------------------------------------------------------------------------
// Tests
//--------------------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_empty_config_yields_defaults() {
        let cfg = AppConfig::parse("").unwrap();
        assert_eq!(cfg, AppConfig::default());
    }

    #[test]
    fn test_parse_full_config() {
        let toml = r#"
            image = "ubuntu:22.04"
            cpus = 4
            memory_mib = 2048
            hostname = "dev-box"
            workdir = "/workspace"
            user = "dev"
            shell = "/bin/bash"
        "#;
        let cfg = AppConfig::parse(toml).unwrap();
        assert_eq!(cfg.image.as_deref(), Some("ubuntu:22.04"));
        assert_eq!(cfg.cpus, Some(4));
        assert_eq!(cfg.memory_mib, Some(2048));
        assert_eq!(cfg.hostname.as_deref(), Some("dev-box"));
        assert_eq!(cfg.workdir.as_deref(), Some("/workspace"));
        assert_eq!(cfg.user.as_deref(), Some("dev"));
        assert_eq!(cfg.shell.as_deref(), Some("/bin/bash"));
    }

    #[test]
    fn test_parse_partial_config_leaves_rest_none() {
        let cfg = AppConfig::parse("image = \"alpine\"\ncpus = 2\n").unwrap();
        assert_eq!(cfg.image.as_deref(), Some("alpine"));
        assert_eq!(cfg.cpus, Some(2));
        assert_eq!(cfg.memory_mib, None);
        assert_eq!(cfg.hostname, None);
    }

    #[test]
    fn test_parse_invalid_toml_errors() {
        assert!(AppConfig::parse("not valid = = toml").is_err());
    }

    #[test]
    fn test_config_path_is_under_config_dir() {
        if let Some(path) = AppConfig::config_path() {
            assert!(path.ends_with("microsandbox-tui/config.toml"));
        }
    }
}
