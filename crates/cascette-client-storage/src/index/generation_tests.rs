use super::IndexManager;
use cascette_crypto::EncodingKey;
use std::path::Path;

fn key(hex: &str) -> EncodingKey {
    EncodingKey::from_hex(hex).expect("valid encoding key")
}

fn write_generation(path: &Path, version: u32, entries: &[(&EncodingKey, u16, u32)]) {
    let mut writer = IndexManager::new(path);
    let bucket = IndexManager::bucket_for_key(entries[0].0);
    for (key, archive_id, offset) in entries {
        assert_eq!(IndexManager::bucket_for_key(key), bucket);
        writer
            .add_entry(key, *archive_id, *offset, 32)
            .expect("add index entry");
    }
    writer.save_all().expect("write v7 index");
    std::fs::rename(
        path.join(format!("{bucket:02x}00000001.idx")),
        path.join(format!("{bucket:02x}{version:08x}.idx")),
    )
    .expect("name index generation");
}

#[tokio::test]
async fn generation_contract_load_all_uses_highest_numeric_generation_without_merging() {
    let shared = key("01010000000000000000000000000000");
    let old_only = key("02020000000000000000000000000000");
    let new_only = key("03030000000000000000000000000000");

    for newest_written_first in [false, true] {
        let dir = tempfile::tempdir().expect("temp index directory");
        let old_entries = [(&shared, 11, 128), (&old_only, 12, 256)];
        let new_entries = [(&shared, 89, 512), (&new_only, 90, 768)];
        if newest_written_first {
            write_generation(dir.path(), 0xac, &new_entries);
            write_generation(dir.path(), 0xa8, &old_entries);
        } else {
            write_generation(dir.path(), 0xa8, &old_entries);
            write_generation(dir.path(), 0xac, &new_entries);
        }

        let mut reader = IndexManager::new(dir.path());
        reader.load_all().await.expect("load index generations");
        let shared_location = reader
            .lookup(&shared)
            .expect("shared key from newest index");
        assert_eq!(
            (
                shared_location.archive_id(),
                shared_location.archive_offset()
            ),
            (89, 512)
        );
        let new_location = reader
            .lookup(&new_only)
            .expect("new-only key from newest index");
        assert_eq!(
            (new_location.archive_id(), new_location.archive_offset()),
            (90, 768)
        );
        assert!(
            reader.lookup(&old_only).is_none(),
            "old generation must not be merged"
        );
    }
}

#[tokio::test]
async fn generation_contract_load_all_rejects_invalid_newest_without_old_fallback() {
    let old_key = key("02020000000000000000000000000000");
    let bucket = IndexManager::bucket_for_key(&old_key);

    for newest_written_first in [false, true] {
        let dir = tempfile::tempdir().expect("temp index directory");
        let newest = dir.path().join(format!("{bucket:02x}000000ac.idx"));
        if newest_written_first {
            std::fs::write(&newest, b"invalid index").expect("write invalid newest index");
            write_generation(dir.path(), 0xa8, &[(&old_key, 11, 128)]);
        } else {
            write_generation(dir.path(), 0xa8, &[(&old_key, 11, 128)]);
            std::fs::write(&newest, b"invalid index").expect("write invalid newest index");
        }

        let mut reader = IndexManager::new(dir.path());
        assert!(
            reader.load_all().await.is_err(),
            "invalid newest must fail closed"
        );
    }
}
