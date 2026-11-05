//! Runtime Hands home directory.
//!
//! Resolution order:
//! 1. `HANDS_HOME`
//! 2. `CPC_ROOT` (back-compat alias for the same directory)
//! 3. `%LOCALAPPDATA%\AIWander\Hands` on Windows, otherwise
//!    `$XDG_DATA_HOME/AIWander/Hands` or `$HOME/.local/share/AIWander/Hands`
//!
//! No hostname, IP address, or per-developer path is compiled in.

use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HomeSource {
    HandsHome,
    CpcRoot,
    Default,
}

impl HomeSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::HandsHome => "HANDS_HOME",
            Self::CpcRoot => "CPC_ROOT",
            Self::Default => "default",
        }
    }
}

/// Inputs for [`resolve_home`]. Empty and whitespace-only values are unset.
pub struct HomeEnv<'a> {
    pub hands_home: Option<&'a str>,
    pub cpc_root: Option<&'a str>,
    pub local_app_data: Option<&'a str>,
    pub user_profile: Option<&'a str>,
    pub xdg_data_home: Option<&'a str>,
    pub home: Option<&'a str>,
    pub windows: bool,
}

fn nonempty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

pub fn resolve_home(env: &HomeEnv<'_>) -> (PathBuf, HomeSource) {
    if let Some(path) = nonempty(env.hands_home) {
        return (PathBuf::from(path), HomeSource::HandsHome);
    }
    if let Some(path) = nonempty(env.cpc_root) {
        return (PathBuf::from(path), HomeSource::CpcRoot);
    }
    (default_home(env), HomeSource::Default)
}

fn default_home(env: &HomeEnv<'_>) -> PathBuf {
    if env.windows {
        if let Some(local) = nonempty(env.local_app_data) {
            return PathBuf::from(local).join("AIWander").join("Hands");
        }
        if let Some(profile) = nonempty(env.user_profile) {
            return PathBuf::from(profile)
                .join("AppData")
                .join("Local")
                .join("AIWander")
                .join("Hands");
        }
        return PathBuf::from("AIWander").join("Hands");
    }
    if let Some(xdg) = nonempty(env.xdg_data_home) {
        return PathBuf::from(xdg).join("AIWander").join("Hands");
    }
    if let Some(home) = nonempty(env.home) {
        return PathBuf::from(home)
            .join(".local")
            .join("share")
            .join("AIWander")
            .join("Hands");
    }
    PathBuf::from(".local")
        .join("share")
        .join("AIWander")
        .join("Hands")
}

fn env_string(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

pub fn resolve_from_process() -> (PathBuf, HomeSource) {
    let hands_home = env_string("HANDS_HOME");
    let cpc_root = env_string("CPC_ROOT");
    let local_app_data = env_string("LOCALAPPDATA");
    let user_profile = env_string("USERPROFILE");
    let xdg_data_home = env_string("XDG_DATA_HOME");
    let home = env_string("HOME");
    resolve_home(&HomeEnv {
        hands_home: hands_home.as_deref(),
        cpc_root: cpc_root.as_deref(),
        local_app_data: local_app_data.as_deref(),
        user_profile: user_profile.as_deref(),
        xdg_data_home: xdg_data_home.as_deref(),
        home: home.as_deref(),
        windows: cfg!(windows),
    })
}

pub fn hands_home() -> PathBuf {
    resolve_from_process().0
}

pub fn home_source() -> HomeSource {
    resolve_from_process().1
}

pub fn hands_logs_dir() -> PathBuf {
    hands_home().join("logs")
}

pub fn hands_data_dir() -> PathBuf {
    hands_home().join("data")
}

pub fn chrome_debug_profile() -> PathBuf {
    hands_home().join("chrome-debug-profile")
}

pub fn install_readme() -> PathBuf {
    hands_home().join("README.md")
}

pub fn install_agents() -> PathBuf {
    hands_home().join("AGENTS.md")
}

pub fn getting_started_skill() -> PathBuf {
    hands_home()
        .join("skills")
        .join("hands-getting-started")
        .join("SKILL.md")
}
