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
    field(1, &product_record(code, version, active, install))
}

fn standalone_product(code: &str, version: &str, active: &str, install: &str) -> Vec<u8> {
    let mut record = string(1, code); // ProductInstall.uid, not a Database entry.
    record.extend(product_record(code, version, active, install));
    record
}

fn product_record(code: &str, version: &str, active: &str, install: &str) -> Vec<u8> {
    let mut base = string(7, version);
    base.extend(string(14, active));
    base.extend(string(16, install));
    base.extend(string(12, RETAIL_KEY)); // Completed, not active.
    base.extend(string(18, RETAIL_KEY)); // Incomplete, not active.
    base.extend([0x10, 0x00]); // playable = false: not a read gate.
    let mut install_record = string(2, code);
    install_record.extend(field(4, &field(1, &base)));
    install_record
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
fn reads_standalone_product_install_with_uid() {
    let dir = tempdir().unwrap();
    let mut record = standalone_product("wowf", "1.60.1.70634", FOREVER_KEY, RETAIL_KEY);
    record.extend(field(3, &string(1, "D:/Games/World of Warcraft")));
    record.extend(string(6, "wow")); // Product family, not the selected product.
    record.extend([0x38, 0x00]); // hidden = false.
    record.extend(string(8, "{}")); // Unknown persistent storage is ignored.
    write_db(dir.path(), &record);

    let selected = read_installed_product(dir.path(), "wowf").unwrap();
    assert_eq!(selected.product, "wowf");
    assert_eq!(selected.version, "1.60.1.70634");
    assert_eq!(selected.build_key, FOREVER_KEY);
    assert_eq!(selected.install_key, RETAIL_KEY);
}

#[test]
fn standalone_wrong_product_stays_not_found() {
    let dir = tempdir().unwrap();
    write_db(dir.path(), &standalone_product("wow", "1", RETAIL_KEY, ""));
    let err = read_installed_product(dir.path(), "wowf").unwrap_err();
    assert!(matches!(err, ProductDbError::NotFound { .. }), "{err}");
}

#[test]
fn valid_wrapper_not_found_does_not_retry_standalone_selection() {
    let dir = tempdir().unwrap();
    let mut db = product("wow", "1", RETAIL_KEY, "");
    // These fields are unknown to Database, but form valid standalone metadata.
    // A semantic NotFound must not reinterpret this valid wrapped database.
    db.extend(product_record("wowf", "2", FOREVER_KEY, ""));
    write_db(dir.path(), &db);
    let err = read_installed_product(dir.path(), "wowf").unwrap_err();
    assert!(matches!(err, ProductDbError::NotFound { .. }), "{err}");
}

#[test]
fn standalone_without_uid_does_not_retry_a_successful_empty_database_decode() {
    let dir = tempdir().unwrap();
    // Without UID field 1, every field is unknown to the minimal Database schema.
    // Matching TACTLib, fallback is only for structural decode failures.
    write_db(dir.path(), &product_record("wowf", "1", FOREVER_KEY, ""));
    let err = read_installed_product(dir.path(), "wowf").unwrap_err();
    assert!(matches!(err, ProductDbError::NotFound { .. }), "{err}");
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
        for db in [
            product("wow_forever", version, key, install),
            standalone_product("wow_forever", version, key, install),
        ] {
            write_db(dir.path(), &db);
            let err = read_installed_product(dir.path(), "wow_forever").unwrap_err();
            assert!(matches!(err, ProductDbError::InvalidField { .. }), "{err}");
            assert!(err.to_string().contains(invalid_field), "{err}");
            assert!(err.to_string().contains("wow_forever"));
            assert!(err.to_string().contains(".product.db"));
        }
    }
}

#[test]
fn rejects_missing_cached_and_base_metadata_in_both_formats() {
    let dir = tempdir().unwrap();
    let missing_cached_state = string(2, "wowf");
    let mut missing_base_state = missing_cached_state.clone();
    missing_base_state.extend(field(4, &[]));
    for record in [missing_cached_state, missing_base_state] {
        let mut standalone = string(1, "wowf");
        standalone.extend(&record);
        for db in [field(1, &record), standalone] {
            write_db(dir.path(), &db);
            let err = read_installed_product(dir.path(), "wowf").unwrap_err();
            assert!(
                matches!(
                    err,
                    ProductDbError::InvalidField {
                        field: "cached_product_state.base_product_state",
                        ..
                    }
                ),
                "{err}"
            );
        }
    }
}

#[test]
fn rejects_invalid_protobuf() {
    let dir = tempdir().unwrap();
    for bytes in [vec![0x0a, 0xff], vec![0x0a, 0x02, 0x12, 0xff]] {
        // The second wrapper has an invalid nested product. The minimal standalone
        // schema can skip its unknown UID bytes, but that empty record is not a fallback.
        write_db(dir.path(), &bytes);
        let err = read_installed_product(dir.path(), "wow").unwrap_err();
        assert!(matches!(err, ProductDbError::Decode { .. }), "{err}");
        assert!(err.to_string().contains(".product.db"));
        assert!(err.to_string().contains("wow"));
    }
}

#[test]
fn rejects_truncated_records_and_preserves_the_wrapper_decode_error() {
    let dir = tempdir().unwrap();
    for mut db in [
        product("wowf", "1", FOREVER_KEY, RETAIL_KEY),
        standalone_product("wowf", "1", FOREVER_KEY, RETAIL_KEY),
    ] {
        db.pop();
        write_db(dir.path(), &db);
        let err = read_installed_product(dir.path(), "wowf").unwrap_err();
        assert!(matches!(err, ProductDbError::Decode { .. }), "{err}");
    }
    let mut malformed = string(1, "wowf");
    malformed.extend([0x22, 0xff]); // Standalone cached state has a truncated length.
    write_db(dir.path(), &malformed);
    let err = read_installed_product(dir.path(), "wowf").unwrap_err();
    let ProductDbError::Decode { source, .. } = err else {
        panic!("expected Decode, got {err}");
    };
    // Database fails inside UID bytes; standalone fails later on its length.
    assert!(source.to_string().contains("invalid wire type"), "{source}");
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

#[test]
#[ignore = "requires explicit CASCETTE_STANDALONE_TEST_* metadata and a real read-only installation"]
fn real_standalone_install_is_read_without_changing_source() {
    let root = std::env::var("CASCETTE_STANDALONE_TEST_ROOT").expect("set installation root");
    let product = std::env::var("CASCETTE_STANDALONE_TEST_PRODUCT").expect("set expected product");
    let version = std::env::var("CASCETTE_STANDALONE_TEST_VERSION").expect("set expected version");
    let build_key = std::env::var("CASCETTE_STANDALONE_TEST_KEY").expect("set expected build key");
    let root = Path::new(&root);
    let path = root.join(".product.db");
    let before = fs::read(&path).unwrap();
    let metadata = fs::metadata(&path).unwrap();
    let modified = metadata.modified().unwrap();

    let selected = read_installed_product(root, &product).unwrap();
    assert_eq!(selected.product, product);
    assert_eq!(selected.version, version);
    assert_eq!(selected.build_key, build_key);
    assert_eq!(fs::read(&path).unwrap(), before);
    let after = fs::metadata(&path).unwrap();
    assert_eq!(after.len(), metadata.len());
    assert_eq!(after.modified().unwrap(), modified);
}
