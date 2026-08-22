use snippit_lib::setup::complete_setup_at;
use std::fs;
use std::path::PathBuf;

fn unique_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "snippit_setup_it_{name}_{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn complete_setup_end_to_end_import_mode_populates_clips_and_settings() {
    let root = unique_dir("root");
    let config_dir = unique_dir("config");
    let source = unique_dir("source");
    fs::write(source.join("gameplay.mp4"), b"fake video bytes").unwrap();
    fs::write(source.join("readme.txt"), b"not a clip").unwrap();

    let result = complete_setup_at(&root, &config_dir, "import", Some(&source)).unwrap();

    assert!(result.warnings.is_empty());
    assert!(root.join("Clips").join("gameplay.mp4").exists());
    assert!(!root.join("Clips").join("readme.txt").exists());
    assert!(root.join("Exports").is_dir());
    assert!(root.join("Cache").is_dir());
    assert!(root.join("settings.json").exists());
    assert!(config_dir.join("root.json").exists());

    let _ = fs::remove_dir_all(&root);
    let _ = fs::remove_dir_all(&config_dir);
    let _ = fs::remove_dir_all(&source);
}
