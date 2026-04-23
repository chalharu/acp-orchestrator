use std::{
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

pub fn unique_temp_json_path(prefix: &str, label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after the epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("{prefix}-{label}-{nanos}.json"))
}

#[cfg(test)]
mod tests {
    use super::unique_temp_json_path;

    #[test]
    fn unique_temp_json_path_uses_the_expected_shape() {
        let path = unique_temp_json_path("acp", "support");

        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .expect("temp path should have a UTF-8 file name");
        assert!(file_name.starts_with("acp-support-"));
        assert!(file_name.ends_with(".json"));
    }
}
