//! Hands install root. See `browser_mcp::home` for the resolution rules.

pub use browser_mcp::home::{hands_data_dir, hands_logs_dir, resolve_from_process, HomeSource};

#[cfg(test)]
mod tests {
    use super::HomeSource;
    use browser_mcp::home::{resolve_home, HomeEnv};
    use std::path::PathBuf;

    fn env<'a>(
        hands_home: Option<&'a str>,
        cpc_root: Option<&'a str>,
        local_app_data: Option<&'a str>,
        windows: bool,
    ) -> HomeEnv<'a> {
        HomeEnv {
            hands_home,
            cpc_root,
            local_app_data,
            user_profile: None,
            xdg_data_home: None,
            home: None,
            windows,
        }
    }

    #[test]
    fn hands_home_overrides_cpc_root_and_default() {
        let (path, source) = resolve_home(&env(
            Some(r"D:\Hands"),
            Some(r"D:\Legacy"),
            Some(r"C:\Users\anyone\AppData\Local"),
            true,
        ));
        assert_eq!(path, PathBuf::from(r"D:\Hands"));
        assert_eq!(source, HomeSource::HandsHome);
    }

    #[test]
    fn cpc_root_is_only_a_back_compat_alias() {
        let (path, source) = resolve_home(&env(
            Some("   "),
            Some(r"D:\LegacyHands"),
            Some(r"C:\Users\anyone\AppData\Local"),
            true,
        ));
        assert_eq!(path, PathBuf::from(r"D:\LegacyHands"));
        assert_eq!(source, HomeSource::CpcRoot);
        assert_eq!(source.as_str(), "CPC_ROOT");
    }

    #[test]
    fn windows_default_is_localappdata_aiwander_hands() {
        let local = r"C:\Users\anyone\AppData\Local";
        let (path, source) = resolve_home(&env(None, None, Some(local), true));
        assert_eq!(path, PathBuf::from(local).join("AIWander").join("Hands"));
        assert_eq!(source, HomeSource::Default);
    }

    #[test]
    fn windows_default_uses_userprofile_when_localappdata_missing() {
        let profile = r"C:\Users\anyone";
        let (path, source) = resolve_home(&HomeEnv {
            hands_home: None,
            cpc_root: None,
            local_app_data: None,
            user_profile: Some(profile),
            xdg_data_home: None,
            home: None,
            windows: true,
        });
        assert_eq!(
            path,
            PathBuf::from(profile)
                .join("AppData")
                .join("Local")
                .join("AIWander")
                .join("Hands")
        );
        assert_eq!(source, HomeSource::Default);
    }

    #[test]
    fn unix_default_prefers_xdg_then_home() {
        let (xdg, source) = resolve_home(&HomeEnv {
            hands_home: None,
            cpc_root: None,
            local_app_data: None,
            user_profile: None,
            xdg_data_home: Some("/var/data"),
            home: Some("/home/anyone"),
            windows: false,
        });
        assert_eq!(
            xdg,
            PathBuf::from("/var/data").join("AIWander").join("Hands")
        );
        assert_eq!(source, HomeSource::Default);

        let (home, _) = resolve_home(&HomeEnv {
            hands_home: None,
            cpc_root: None,
            local_app_data: None,
            user_profile: None,
            xdg_data_home: None,
            home: Some("/home/anyone"),
            windows: false,
        });
        assert_eq!(
            home,
            PathBuf::from("/home/anyone")
                .join(".local")
                .join("share")
                .join("AIWander")
                .join("Hands")
        );
    }

    #[test]
    fn resolver_has_no_compiled_developer_path() {
        let (path, _) = resolve_home(&env(None, None, Some(r"C:\LocalApp"), true));
        let rendered = path.display().to_string();
        assert!(!rendered.contains("CPC"));
        assert!(!rendered.to_ascii_lowercase().contains("josep"));
        assert!(rendered.ends_with(r"AIWander\Hands") || rendered.ends_with("AIWander/Hands"));
    }
}
