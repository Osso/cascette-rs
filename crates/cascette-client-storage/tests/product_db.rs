use std::{fs, path::Path};

use cascette_client_storage::{ProductDbError, read_installed_product};
use tempfile::tempdir;

const FOREVER_KEY: &str = "3bd89ce2721f7c75e7525dc83741076f";
const RETAIL_KEY: &str = "0123456789abcdef0123456789abcdef";

// Encode fixtures independently of the production prost message definitions.
fn varint(mut n: usize) -> Vec<u8> {
    let mut bytes = Vec::new();
    while n >= 128 {
        bytes.push((n as u8 & 0x7f) | 0x80);
        n >>= 7;
    }
    bytes.push(n as u8);
    bytes
}

fn field(tag: usize, value: &[u8]) -> Vec<u8> {
    let mut bytes = varint((tag << 3) | 2);
    bytes.extend(varint(value.len()));
    bytes.extend(value);
    bytes
}

fn string(tag: usize, value: &str) -> Vec<u8> {
    field(tag, value.as_bytes())
}

fn product(code: &str, version: &str, active: &str, install: &str) -> Vec<u8> {
    let mut base = string(7, version);
    base.extend(string(14, active));
    base.extend(string(16, install));
    base.extend(string(12, RETAIL_KEY)); // Completed, not active.
    base.extend(string(18, RETAIL_KEY)); // Incomplete, not active.
    base.extend([0x10, 0x00]); // playable = false: not a read gate.
    let mut install_record = string(2, code);
    install_record.extend(field(4, &field(1, &base)));
    field(1, &install_record)
}

fn write_db(root: &Path, bytes: &[u8]) {
    fs::write(root.join(".product.db"), bytes).unwrap();
}

#[test]
fn selects_active_forever_without_build_info_product_row_or_playability() {
    let dir = tempdir().unwrap();
    fs::write(
        dir.path().join(".build.info"),
        "Version!STRING:0|Product!STRING:0\n12.0.5|wow\n",
    )
    .unwrap();
    let mut db = product("wow", "12.0.5.1", RETAIL_KEY, "");
    db.extend(product(
        "wow_forever",
        "1.60.1.69977",
        FOREVER_KEY,
        RETAIL_KEY,
    ));
    write_db(dir.path(), &db);

    let selected = read_installed_product(dir.path(), "wow_forever").unwrap();
    assert_eq!(selected.product, "wow_forever");
    assert_eq!(selected.version, "1.60.1.69977");
    assert_eq!(selected.build_key, FOREVER_KEY);
    assert_eq!(selected.install_key, RETAIL_KEY);
}

#[test]
fn retail_profile_selects_its_own_product() {
    let dir = tempdir().unwrap();
    let mut db = product("wow_forever", "1.60.1.69977", FOREVER_KEY, "");
    db.extend(product("wow", "12.0.5.1", RETAIL_KEY, ""));
    write_db(dir.path(), &db);
    let selected = read_installed_product(dir.path(), "wow").unwrap();
    assert_eq!(selected.version, "12.0.5.1");
    assert_eq!(selected.build_key, RETAIL_KEY);
    assert_eq!(selected.install_key, "");
}

#[test]
fn missing_db_does_not_fall_back_to_build_info() {
    let dir = tempdir().unwrap();
    fs::write(
        dir.path().join(".build.info"),
        "Version!STRING:0|Product!STRING:0\n1|wow\n",
    )
    .unwrap();
    let err = read_installed_product(dir.path(), "wow").unwrap_err();
    assert!(matches!(err, ProductDbError::Io { .. }));
    assert!(err.to_string().contains(".product.db"));
    assert!(err.to_string().contains("wow"));
}

#[test]
fn rejects_missing_and_duplicate_products() {
    let dir = tempdir().unwrap();
    write_db(dir.path(), &product("wow", "1", RETAIL_KEY, ""));
    let err = read_installed_product(dir.path(), "wow_forever").unwrap_err();
    assert!(matches!(err, ProductDbError::NotFound { .. }));
    assert!(err.to_string().contains("wow_forever"));
    let mut db = product("wow", "1", RETAIL_KEY, "");
    db.extend(product("wow", "2", FOREVER_KEY, ""));
    write_db(dir.path(), &db);
    let err = read_installed_product(dir.path(), "wow").unwrap_err();
    assert!(matches!(err, ProductDbError::Duplicate { .. }));
    assert!(err.to_string().contains(".product.db"));
}

#[test]
fn rejects_missing_or_invalid_active_keys_and_empty_versions() {
    let dir = tempdir().unwrap();
    for (version, key, install, invalid_field) in [
        ("", FOREVER_KEY, "", "version"),
        ("1", "", "", "build_key"),
        ("1", "xyz", "", "build_key"),
        ("1", FOREVER_KEY, "not-hex", "install_key"),
    ] {
        write_db(dir.path(), &product("wow_forever", version, key, install));
        let err = read_installed_product(dir.path(), "wow_forever").unwrap_err();
        assert!(matches!(err, ProductDbError::InvalidField { .. }), "{err}");
        assert!(err.to_string().contains(invalid_field), "{err}");
        assert!(err.to_string().contains("wow_forever"));
        assert!(err.to_string().contains(".product.db"));
    }
}

#[test]
fn rejects_invalid_protobuf() {
    let dir = tempdir().unwrap();
    write_db(dir.path(), &[0x0a, 0xff]);
    let err = read_installed_product(dir.path(), "wow").unwrap_err();
    assert!(matches!(err, ProductDbError::Decode { .. }));
    assert!(err.to_string().contains(".product.db"));
    assert!(err.to_string().contains("wow"));
}

#[test]
fn bounds_database_reads() {
    let dir = tempdir().unwrap();
    write_db(dir.path(), &vec![0; 16 * 1024 * 1024 + 1]);
    let err = read_installed_product(dir.path(), "wow").unwrap_err();
    assert!(matches!(err, ProductDbError::TooLarge { .. }));
    assert!(err.to_string().contains("wow"));
    assert!(err.to_string().contains(".product.db"));
}

#[test]
fn real_install_when_explicitly_requested() {
    let Ok(root) = std::env::var("CASCETTE_TEST_PRODUCT_DB_ROOT") else {
        return;
    };
    let selected = read_installed_product(Path::new(&root), "wow_classic_beta").unwrap();
    assert_eq!(selected.version, "1.60.1.69977");
    assert_eq!(selected.build_key, FOREVER_KEY);
}
