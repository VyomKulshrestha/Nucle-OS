//! Proves per-tenant pool isolation end to end: two tenants storing
//! under the same `--pool-dir` never see each other's files, the
//! untenanted base pool is completely unaffected by either, `nucle
//! tenants` lists exactly the tenants that have data, `NUCLEOS_TENANT`
//! works like `--tenant`, and an invalid tenant name is rejected before
//! it ever becomes a path component. Spawns the real, compiled
//! `nucle-cli` binary (`CARGO_BIN_EXE_nucle-cli`).

use std::path::PathBuf;
use std::process::Command;

fn nucle_cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_nucle-cli")
}

fn scratch_dir(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("nucle_cli_tenants_test_{}_{}", name, std::process::id()))
}

#[test]
fn two_tenants_never_see_each_others_files() {
    let pool_dir = scratch_dir("isolation_pool");
    let src_dir = scratch_dir("isolation_src");
    let _ = std::fs::remove_dir_all(&pool_dir);
    let _ = std::fs::remove_dir_all(&src_dir);
    std::fs::create_dir_all(&src_dir).unwrap();

    let acme_file = src_dir.join("acme.txt");
    let globex_file = src_dir.join("globex.txt");
    std::fs::write(&acme_file, b"acme's data").unwrap();
    std::fs::write(&globex_file, b"globex's data").unwrap();

    for (tenant, file) in [("acme", &acme_file), ("globex", &globex_file)] {
        let store = Command::new(nucle_cli_bin())
            .arg("--pool-dir").arg(&pool_dir)
            .arg("--tenant").arg(tenant)
            .arg("store").arg(file)
            .arg("--redundancy").arg("0")
            .output()
            .expect("failed to spawn nucle-cli for store");
        assert!(store.status.success(), "stderr: {}", String::from_utf8_lossy(&store.stderr));
    }

    let acme_list = Command::new(nucle_cli_bin())
        .arg("--pool-dir").arg(&pool_dir)
        .arg("--tenant").arg("acme")
        .arg("list")
        .output()
        .expect("failed to spawn nucle-cli for acme list");
    let acme_out = String::from_utf8_lossy(&acme_list.stdout);
    assert!(acme_out.contains("acme.txt"));
    assert!(!acme_out.contains("globex.txt"));

    let globex_list = Command::new(nucle_cli_bin())
        .arg("--pool-dir").arg(&pool_dir)
        .arg("--tenant").arg("globex")
        .arg("list")
        .output()
        .expect("failed to spawn nucle-cli for globex list");
    let globex_out = String::from_utf8_lossy(&globex_list.stdout);
    assert!(globex_out.contains("globex.txt"));
    assert!(!globex_out.contains("acme.txt"));

    let _ = std::fs::remove_dir_all(&pool_dir);
    let _ = std::fs::remove_dir_all(&src_dir);
}

#[test]
fn the_untenanted_base_pool_is_unaffected_by_tenant_stores() {
    let pool_dir = scratch_dir("backcompat_pool");
    let src_dir = scratch_dir("backcompat_src");
    let _ = std::fs::remove_dir_all(&pool_dir);
    let _ = std::fs::remove_dir_all(&src_dir);
    std::fs::create_dir_all(&src_dir).unwrap();

    let file = src_dir.join("tenant_only.txt");
    std::fs::write(&file, b"belongs to a tenant, not the base pool").unwrap();

    Command::new(nucle_cli_bin())
        .arg("--pool-dir").arg(&pool_dir)
        .arg("--tenant").arg("some_tenant")
        .arg("store").arg(&file)
        .arg("--redundancy").arg("0")
        .output()
        .expect("failed to spawn nucle-cli for tenant store");

    let base_list = Command::new(nucle_cli_bin())
        .arg("--pool-dir").arg(&pool_dir)
        .arg("list")
        .output()
        .expect("failed to spawn nucle-cli for base list");
    assert!(base_list.status.success());
    assert!(String::from_utf8_lossy(&base_list.stdout).contains("No files stored"));

    // No state.json directly under pool_dir either -- the tenant's data
    // lives only under tenants/some_tenant/.
    assert!(!pool_dir.join("state.json").exists());
    assert!(pool_dir.join("tenants").join("some_tenant").join("state.json").exists());

    let _ = std::fs::remove_dir_all(&pool_dir);
    let _ = std::fs::remove_dir_all(&src_dir);
}

#[test]
fn nucle_tenants_lists_exactly_the_tenants_with_data() {
    let pool_dir = scratch_dir("list_tenants_pool");
    let src_dir = scratch_dir("list_tenants_src");
    let _ = std::fs::remove_dir_all(&pool_dir);
    let _ = std::fs::remove_dir_all(&src_dir);
    std::fs::create_dir_all(&src_dir).unwrap();

    let none_yet = Command::new(nucle_cli_bin())
        .arg("--pool-dir").arg(&pool_dir)
        .arg("tenants")
        .output()
        .expect("failed to spawn nucle-cli for tenants (empty)");
    assert!(none_yet.status.success());
    assert!(String::from_utf8_lossy(&none_yet.stdout).contains("No tenants found"));

    let file = src_dir.join("a.txt");
    std::fs::write(&file, b"data").unwrap();
    for tenant in ["zebra", "acme"] {
        Command::new(nucle_cli_bin())
            .arg("--pool-dir").arg(&pool_dir)
            .arg("--tenant").arg(tenant)
            .arg("store").arg(&file)
            .arg("--redundancy").arg("0")
            .output()
            .expect("failed to spawn nucle-cli for store");
    }

    let listed = Command::new(nucle_cli_bin())
        .arg("--pool-dir").arg(&pool_dir)
        .arg("--json")
        .arg("tenants")
        .output()
        .expect("failed to spawn nucle-cli for tenants (populated)");
    assert!(listed.status.success());
    let names: Vec<String> = serde_json::from_slice(&listed.stdout).unwrap();
    assert_eq!(names, vec!["acme".to_string(), "zebra".to_string()], "expected sorted tenant names");

    let _ = std::fs::remove_dir_all(&pool_dir);
    let _ = std::fs::remove_dir_all(&src_dir);
}

#[test]
fn nucleos_tenant_env_var_works_like_the_flag() {
    let pool_dir = scratch_dir("env_var_pool");
    let src_dir = scratch_dir("env_var_src");
    let _ = std::fs::remove_dir_all(&pool_dir);
    let _ = std::fs::remove_dir_all(&src_dir);
    std::fs::create_dir_all(&src_dir).unwrap();

    let file = src_dir.join("a.txt");
    std::fs::write(&file, b"via env var").unwrap();

    Command::new(nucle_cli_bin())
        .arg("--pool-dir").arg(&pool_dir)
        .arg("store").arg(&file)
        .arg("--redundancy").arg("0")
        .env("NUCLEOS_TENANT", "env_tenant")
        .output()
        .expect("failed to spawn nucle-cli for store via env var");

    let list = Command::new(nucle_cli_bin())
        .arg("--pool-dir").arg(&pool_dir)
        .arg("--tenant").arg("env_tenant")
        .arg("list")
        .output()
        .expect("failed to spawn nucle-cli for list");
    assert!(String::from_utf8_lossy(&list.stdout).contains("a.txt"));

    let _ = std::fs::remove_dir_all(&pool_dir);
    let _ = std::fs::remove_dir_all(&src_dir);
}

#[test]
fn an_invalid_tenant_name_is_rejected_before_touching_any_path() {
    let pool_dir = scratch_dir("invalid_tenant_pool");
    let _ = std::fs::remove_dir_all(&pool_dir);

    for bad_name in ["../escape", "with/slash", ".."] {
        let result = Command::new(nucle_cli_bin())
            .arg("--pool-dir").arg(&pool_dir)
            .arg("--tenant").arg(bad_name)
            .arg("pool")
            .output()
            .expect("failed to spawn nucle-cli with an invalid tenant name");
        assert!(!result.status.success(), "expected '{}' to be rejected", bad_name);
        assert!(String::from_utf8_lossy(&result.stderr).contains("Invalid --tenant"));
    }

    // Nothing should have been created outside the intended pool_dir tree.
    assert!(!pool_dir.parent().unwrap().join("escape").exists());

    let _ = std::fs::remove_dir_all(&pool_dir);
}
