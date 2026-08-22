//! Round-trips the real bundled rclone against a `local` remote (a temp dir).
//! Mirrors how trim_integration drives the real ffmpeg binary.

use std::path::PathBuf;
use std::process::Command;

fn rclone_bin() -> PathBuf {
    let (suffix, ext) = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => ("x86_64-unknown-linux-gnu", ""),
        ("windows", "x86_64") => ("x86_64-pc-windows-msvc", ".exe"),
        ("macos", "aarch64") => ("aarch64-apple-darwin", ""),
        ("macos", "x86_64") => ("x86_64-apple-darwin", ""),
        (os, arch) => panic!("no bundled rclone sidecar known for {os}/{arch}"),
    };
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("binaries/rclone-{suffix}{ext}"))
}

fn run(args: &[String]) -> (bool, String) {
    let out = Command::new(rclone_bin()).args(args).output().expect("spawn rclone");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).to_string()
            + &String::from_utf8_lossy(&out.stderr),
    )
}

#[test]
fn upload_list_download_delete_roundtrip_on_local_remote() {
    let tmp = std::env::temp_dir().join(format!("clipsnip_sync_it_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("store")).unwrap();
    let conf = tmp.join("rclone.conf");

    // create the `clipsnip` remote as a local backend rooted anywhere;
    // remote paths below are absolute into our store dir
    let mut create = clipsnip_lib::remote::base_args(&conf);
    create.extend(["config".into(), "create".into(), "clipsnip".into(), "local".into()]);
    assert!(run(&create).0, "config create failed");

    let store = tmp.join("store").to_string_lossy().to_string();
    let dir = format!("clipsnip:{store}/ClipSnip/Clips");
    assert!(run(&clipsnip_lib::remote::mkdir_args(&conf, &dir)).0);

    let local = tmp.join("a.mp4");
    std::fs::write(&local, b"fake video bytes").unwrap();
    let remote_file = format!("{dir}/sub/a.mp4");
    let up = clipsnip_lib::remote::copyto_args(&conf, &local.to_string_lossy(), &remote_file);
    assert!(run(&up).0, "upload failed");
    // idempotent re-upload
    assert!(run(&up).0, "re-upload failed");

    let (ok, json) = run(&clipsnip_lib::remote::lsjson_args(&conf, &dir));
    assert!(ok);
    let files = clipsnip_lib::remote::parse_lsjson(&json).unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].path, "sub/a.mp4");
    assert_eq!(files[0].size, 16);

    let back = tmp.join("back.mp4");
    assert!(run(&clipsnip_lib::remote::download_args(&conf, &remote_file, &back.to_string_lossy())).0);
    assert_eq!(std::fs::read(&back).unwrap(), b"fake video bytes");

    assert!(run(&clipsnip_lib::remote::deletefile_args(&conf, &remote_file)).0);
    let (ok, json) = run(&clipsnip_lib::remote::lsjson_args(&conf, &dir));
    assert!(ok);
    assert!(clipsnip_lib::remote::parse_lsjson(&json).unwrap().is_empty());

    let _ = std::fs::remove_dir_all(&tmp);
}
