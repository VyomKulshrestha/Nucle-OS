//! Proves the hierarchical, path-like namespace: storing under a
//! relative path like "docs/readme.txt" doesn't collide
//! with "downloads/readme.txt", and `nucle list <prefix>` filters by it.
//! Spawns the real, compiled `nucle-cli` binary (`CARGO_BIN_EXE_nucle-cli`).

use std::path::PathBuf;
use std::process::Command;

fn nucle_cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_nucle-cli")
}

fn scratch_dir(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("nucle_cli_namespace_test_{}_{}", name, std::process::id()))
}

#[test]
fn same_leaf_name_under_different_prefixes_stores_and_lists_independently() {
    let pool_dir = scratch_dir("pool");
    let src_dir = scratch_dir("src");
    let _ = std::fs::remove_dir_all(&pool_dir);
    let _ = std::fs::remove_dir_all(&src_dir);
    std::fs::create_dir_all(src_dir.join("docs")).unwrap();
    std::fs::create_dir_all(src_dir.join("downloads")).unwrap();
    std::fs::write(src_dir.join("docs/readme.txt"), b"the docs one").unwrap();
    std::fs::write(src_dir.join("downloads/readme.txt"), b"the downloads one").unwrap();

    // Run with the source directory as cwd, so the relative
    // "docs/readme.txt" argument keeps its directory prefix as given.
    for rel in ["docs/readme.txt", "downloads/readme.txt"] {
        let store = Command::new(nucle_cli_bin())
            .current_dir(&src_dir)
            .arg("--pool-dir").arg(&pool_dir)
            .arg("store").arg(rel)
            .arg("--redundancy").arg("1")
            .output()
            .expect("failed to spawn nucle-cli for store");
        assert!(store.status.success(), "store {} failed: {}", rel, String::from_utf8_lossy(&store.stderr));
    }

    let list_docs = Command::new(nucle_cli_bin())
        .arg("--pool-dir").arg(&pool_dir)
        .arg("list").arg("docs/")
        .output()
        .expect("failed to spawn nucle-cli for list");
    assert!(list_docs.status.success());
    let stdout = String::from_utf8_lossy(&list_docs.stdout);
    assert!(stdout.contains("docs/readme.txt"), "list docs/ output: {}", stdout);
    assert!(!stdout.contains("downloads/readme.txt"), "list docs/ should not show downloads/readme.txt: {}", stdout);

    let retrieve_docs = Command::new(nucle_cli_bin())
        .arg("--pool-dir").arg(&pool_dir)
        .arg("retrieve").arg("docs/readme.txt")
        .output()
        .expect("failed to spawn nucle-cli for retrieve");
    assert!(retrieve_docs.status.success());
    assert!(String::from_utf8_lossy(&retrieve_docs.stdout).contains("the docs one"));

    let retrieve_downloads = Command::new(nucle_cli_bin())
        .arg("--pool-dir").arg(&pool_dir)
        .arg("retrieve").arg("downloads/readme.txt")
        .output()
        .expect("failed to spawn nucle-cli for retrieve");
    assert!(retrieve_downloads.status.success());
    assert!(String::from_utf8_lossy(&retrieve_downloads.stdout).contains("the downloads one"));

    let _ = std::fs::remove_dir_all(&pool_dir);
    let _ = std::fs::remove_dir_all(&src_dir);
}

#[test]
fn an_absolute_source_path_still_stores_under_its_bare_leaf_name() {
    let pool_dir = scratch_dir("abs_pool");
    let src_dir = scratch_dir("abs_src");
    let _ = std::fs::remove_dir_all(&pool_dir);
    let _ = std::fs::remove_dir_all(&src_dir);
    std::fs::create_dir_all(&src_dir).unwrap();
    let abs_source = src_dir.join("bare.txt");
    std::fs::write(&abs_source, b"bare filename test").unwrap();
    assert!(abs_source.is_absolute());

    let store = Command::new(nucle_cli_bin())
        .arg("--pool-dir").arg(&pool_dir)
        .arg("store").arg(&abs_source)
        .arg("--redundancy").arg("1")
        .output()
        .expect("failed to spawn nucle-cli for store");
    assert!(store.status.success());

    // Stored under "bare.txt", not the full absolute path.
    let retrieve = Command::new(nucle_cli_bin())
        .arg("--pool-dir").arg(&pool_dir)
        .arg("retrieve").arg("bare.txt")
        .output()
        .expect("failed to spawn nucle-cli for retrieve");
    assert!(retrieve.status.success(), "retrieve failed: {}", String::from_utf8_lossy(&retrieve.stderr));
    assert!(String::from_utf8_lossy(&retrieve.stdout).contains("bare filename test"));

    let _ = std::fs::remove_dir_all(&pool_dir);
    let _ = std::fs::remove_dir_all(&src_dir);
}

#[test]
fn list_with_no_prefix_shows_everything() {
    let pool_dir = scratch_dir("list_all_pool");
    let src_dir = scratch_dir("list_all_src");
    let _ = std::fs::remove_dir_all(&pool_dir);
    let _ = std::fs::remove_dir_all(&src_dir);
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::write(src_dir.join("only.txt"), b"only file").unwrap();

    let store = Command::new(nucle_cli_bin())
        .current_dir(&src_dir)
        .arg("--pool-dir").arg(&pool_dir)
        .arg("store").arg("only.txt")
        .arg("--redundancy").arg("1")
        .output()
        .expect("failed to spawn nucle-cli for store");
    assert!(store.status.success());

    let list = Command::new(nucle_cli_bin())
        .arg("--pool-dir").arg(&pool_dir)
        .arg("list")
        .output()
        .expect("failed to spawn nucle-cli for list");
    assert!(list.status.success());
    assert!(String::from_utf8_lossy(&list.stdout).contains("only.txt"));

    let _ = std::fs::remove_dir_all(&pool_dir);
    let _ = std::fs::remove_dir_all(&src_dir);
}
