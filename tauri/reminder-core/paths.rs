// Copyright (c) 2026 Ma Xin <maxin.zhang@users.noreply.github.com>
// SPDX-License-Identifier: MIT
//
// The single place where configuration paths are resolved.
//
// 组织名、应用名、文件名一旦分散在多个模块，前端与迁移模块就会各写一份，
// 拼出来的目录不一样（曾经前端用 `MaXin`、legacy 模块用 `LocalTools`，
// 而 legacy 的导入函数根本没有调用方，升级用户看到的永远是空列表）。
// 现在所有配置路径都从这里解析，目录名也集中在这里一处定义。

use std::path::{Path, PathBuf};

/// 组织名：本程序（Tauri / Win32 Rust 版）使用的。
pub const APP_ORG: &str = "MaXin";

/// 应用名：配置目录的第二级。
pub const APP_NAME: &str = "WorkReminder";

/// 旧版（Qt）程序使用的组织名 —— 只作为迁移来源，不可与当前目录混用。
pub const LEGACY_ORG: &str = "LocalTools";

/// 旧版程序使用的应用名。
pub const LEGACY_APP: &str = "WorkReminder";

/// 配置文件名。
pub const CONFIG_FILE: &str = "reminders.json";

/// 配置根目录：`%APPDATA%`，缺失时为 `None`。
pub fn config_base() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(PathBuf::from)
}

/// 在给定根目录下拼出 `<root>/<org>/<app>/reminders.json`。
pub fn config_path_under(root: &Path, org: &str, app: &str) -> PathBuf {
    root.join(org).join(app).join(CONFIG_FILE)
}

/// 本程序的配置文件路径：`%APPDATA%\MaXin\WorkReminder\reminders.json`。
///
/// `%APPDATA%` 缺失时退回当前目录（相对路径），与旧实现一致：拿不到用户目录
/// 时总要给出一个可解析的路径，而不是 `None` 让调用方各写一套兜底。
pub fn current_config_path() -> PathBuf {
    let root = config_base().unwrap_or_else(|| PathBuf::from("."));
    config_path_under(&root, APP_ORG, APP_NAME)
}

/// 旧版程序的配置文件路径：`%APPDATA%\LocalTools\WorkReminder\reminders.json`。
///
/// `%APPDATA%` 缺失时为 `None`：没有用户目录就没有旧配置可找，
/// 迁移应当直接跳过，而不是去一个假想的目录里读文件。
pub fn legacy_config_path() -> Option<PathBuf> {
    config_base().map(|root| config_path_under(&root, LEGACY_ORG, LEGACY_APP))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_path_under_joins_org_app_and_file() {
        let root = Path::new("/base");
        assert_eq!(
            config_path_under(root, "Org", "App"),
            PathBuf::from("/base/Org/App/reminders.json")
        );
    }

    #[test]
    fn current_config_path_uses_appdata_layout() {
        let s = current_config_path().to_string_lossy().replace('\\', "/");
        assert!(
            s.ends_with("MaXin/WorkReminder/reminders.json"),
            "got {}",
            s
        );
    }

    #[test]
    fn legacy_config_path_points_at_the_old_program_directory() {
        let base = config_base().expect("APPDATA");
        let p = legacy_config_path().expect("legacy path");
        assert_eq!(p, config_path_under(&base, LEGACY_ORG, LEGACY_APP));
        let s = p.to_string_lossy().replace('\\', "/");
        assert!(
            s.ends_with("LocalTools/WorkReminder/reminders.json"),
            "got {}",
            s
        );
    }

    #[test]
    fn current_and_legacy_paths_share_one_base_and_differ_only_in_org() {
        // 两个路径必须来自同一个根目录，否则"统一入口"只是形式上的。
        let base = config_base().expect("APPDATA");
        let current = current_config_path();
        let legacy = legacy_config_path().expect("legacy path");
        // <base>/<org>/<app>/reminders.json
        assert_eq!(current.parent().unwrap().parent().unwrap().parent().unwrap(), base);
        assert_eq!(legacy.parent().unwrap().parent().unwrap().parent().unwrap(), base);
        assert_eq!(current.parent().unwrap().file_name().unwrap(), APP_NAME);
        assert_eq!(legacy.parent().unwrap().file_name().unwrap(), LEGACY_APP);
        assert_eq!(current.parent().unwrap().parent().unwrap().file_name().unwrap(), APP_ORG);
        assert_eq!(legacy.parent().unwrap().parent().unwrap().file_name().unwrap(), LEGACY_ORG);
        assert_ne!(current, legacy);
    }

    /// A39 回归：目录名只能在 `paths` 里出现一次，且迁移必须接线。
    ///
    /// 缺陷版本里 `main.rs` 写着 `"MaXin"`、`legacy.rs` 写着 `"LocalTools"`，
    /// 两处各自拼路径；一旦某个名字改了、另一处没改，两个目录就再无关联。
    /// 更要紧的是迁移根本没有调用方，升级用户打开的是空列表。
    /// 这里直接读源码来守住这两条约束。
    #[test]
    fn no_other_module_hardcodes_the_config_directory_names() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        for (rel, forbidden) in [
            ("../src/main.rs", APP_ORG),
            ("legacy.rs", LEGACY_ORG),
        ] {
            let src = std::fs::read_to_string(root.join(rel))
                .unwrap_or_else(|e| panic!("读取 {} 失败: {e}", rel));
            assert!(
                !src.contains(&format!("\"{forbidden}\"")),
                "{rel} 里出现了硬编码的目录名 {forbidden}，配置路径必须经由 paths 模块解析"
            );
        }
    }

    /// A39 回归：Tauri 前端启动时必须调用 legacy 迁移。
    ///
    /// 缺陷版本里 `legacy::import_from_disk` 除自身测试外没有调用方，
    /// 升级用户的旧配置永远不会被读走。
    #[test]
    fn tauri_front_end_wires_the_legacy_migration() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let src = std::fs::read_to_string(root.join("../src/main.rs"))
            .unwrap_or_else(|e| panic!("读取 main.rs 失败: {e}"));
        assert!(
            src.contains("legacy::migrate_missing"),
            "main.rs 必须调用 legacy::migrate_missing，否则升级用户看不到旧配置"
        );
    }
}
