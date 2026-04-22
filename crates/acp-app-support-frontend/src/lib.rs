use std::{
    io,
    path::{Path, PathBuf},
};

pub const FRONTEND_BUNDLE_PREFIX: &str = "acp-web-frontend";
pub const FRONTEND_JAVASCRIPT_ASSET_PATH: &str = "/app/assets/acp-web-frontend.js";
pub const FRONTEND_WASM_ASSET_PATH: &str = "/app/assets/acp-web-frontend_bg.wasm";
pub const LEGACY_FRONTEND_JAVASCRIPT_ASSET_PATH: &str = "/app/assets/acp_web_frontend.js";
pub const LEGACY_FRONTEND_WASM_ASSET_PATH: &str = "/app/assets/acp_web_frontend_bg.wasm";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrontendBundleAsset {
    JavaScript,
    Wasm,
}

impl FrontendBundleAsset {
    fn suffix(self) -> &'static str {
        match self {
            Self::JavaScript => ".js",
            Self::Wasm => "_bg.wasm",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::JavaScript => "frontend javascript bundle",
            Self::Wasm => "frontend wasm bundle",
        }
    }
}

pub fn frontend_bundle_file_name(tag: &str, asset: FrontendBundleAsset) -> String {
    format!("{FRONTEND_BUNDLE_PREFIX}-{tag}{}", asset.suffix())
}

pub fn is_frontend_bundle_asset(file_name: &str, asset: FrontendBundleAsset) -> bool {
    file_name.starts_with(FRONTEND_BUNDLE_PREFIX) && file_name.ends_with(asset.suffix())
}

pub fn frontend_bundle_exists(dist: &Path, asset: FrontendBundleAsset) -> bool {
    std::fs::read_dir(dist)
        .ok()
        .into_iter()
        .flat_map(|entries| entries.filter_map(|entry| entry.ok()))
        .any(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|file_name| is_frontend_bundle_asset(file_name, asset))
        })
}

pub fn find_frontend_bundle_asset(dist: &Path, asset: FrontendBundleAsset) -> io::Result<PathBuf> {
    std::fs::read_dir(dist)?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .find(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|file_name| is_frontend_bundle_asset(file_name, asset))
        })
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("missing {}", asset.label()),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::{
        FrontendBundleAsset, find_frontend_bundle_asset, frontend_bundle_exists,
        frontend_bundle_file_name, is_frontend_bundle_asset,
    };
    use acp_app_support_temp::unique_temp_json_path;

    #[test]
    fn frontend_bundle_helpers_match_expected_names() {
        let javascript = frontend_bundle_file_name("test", FrontendBundleAsset::JavaScript);
        let wasm = frontend_bundle_file_name("test", FrontendBundleAsset::Wasm);

        assert_eq!(javascript, "acp-web-frontend-test.js");
        assert_eq!(wasm, "acp-web-frontend-test_bg.wasm");
        assert!(is_frontend_bundle_asset(
            &javascript,
            FrontendBundleAsset::JavaScript
        ));
        assert!(is_frontend_bundle_asset(&wasm, FrontendBundleAsset::Wasm));
    }

    #[test]
    fn frontend_bundle_helpers_find_matching_files() {
        let dist = unique_temp_json_path("acp-frontend-dist", "support").with_extension("");
        std::fs::create_dir_all(&dist).expect("frontend dist directory should be creatable");
        let javascript = dist.join(frontend_bundle_file_name(
            "abc",
            FrontendBundleAsset::JavaScript,
        ));
        let wasm = dist.join(frontend_bundle_file_name("abc", FrontendBundleAsset::Wasm));
        std::fs::write(&javascript, b"// js").expect("javascript bundle should write");
        std::fs::write(&wasm, b"\x00asm\x01\x00\x00\x00").expect("wasm bundle should write");

        assert!(frontend_bundle_exists(
            &dist,
            FrontendBundleAsset::JavaScript
        ));
        assert!(frontend_bundle_exists(&dist, FrontendBundleAsset::Wasm));
        assert_eq!(
            find_frontend_bundle_asset(&dist, FrontendBundleAsset::JavaScript)
                .expect("javascript bundle should be discoverable"),
            javascript
        );
        assert_eq!(
            find_frontend_bundle_asset(&dist, FrontendBundleAsset::Wasm)
                .expect("wasm bundle should be discoverable"),
            wasm
        );

        std::fs::remove_dir_all(&dist).expect("temp dist directory should be removable");
    }

    #[test]
    fn frontend_bundle_helpers_report_missing_assets_and_directories() {
        let dist = unique_temp_json_path("acp-frontend-dist-missing", "support").with_extension("");
        std::fs::create_dir_all(&dist).expect("frontend dist directory should be creatable");

        assert!(!frontend_bundle_exists(
            &dist,
            FrontendBundleAsset::JavaScript
        ));
        let error = find_frontend_bundle_asset(&dist, FrontendBundleAsset::Wasm)
            .expect_err("missing bundles should fail");
        assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
        assert!(error.to_string().contains("frontend wasm bundle"));

        std::fs::remove_dir_all(&dist).expect("temp dist directory should be removable");
    }
}
