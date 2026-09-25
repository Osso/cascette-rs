# cascette-client-storage

Local CASC storage implementation for game installations.

## Status

Working implementation with index management, archive I/O, content resolution,
and shared memory IPC.

## Features

- Bucket-based .idx index files with 18-byte entries and sorted key lookup
- Memory-mapped .data archive files with BLTE compression and decompression
- Content resolution chain: path/FileDataID -> ContentKey -> EncodingKey -> archive location
- Multi-installation storage management with CASC directory structure validation
- Shared memory IPC for communication with game clients (Windows and Unix)
- Archive compaction with configurable fragmentation thresholds
- Round-trip validation framework for binary format testing

## Modules

- `index` - Index file (.idx) management with bucket algorithm, big-endian 9-byte
  truncated encoding keys, and archive location bit-packing
- `archive` - Archive file (.data) management with memory-mapped I/O, BLTE
  compression modes (none, zlib, lz4), and compaction support
- `resolver` - Content resolution pipeline using root file, encoding file, and
  DashMap-based caches for path, FileDataID, and content key lookups
- `installation` - High-level async API for reading and writing files in a single
  CASC installation, including 30-byte local archive header parsing
- `storage` - Multi-installation management with CASC directory structure creation
  and validation (indices, data, config, shmem directories)
- `shmem` - Shared memory IPC with binrw-serialized messages, platform-specific
  implementations (Windows `CreateFileMapping`, Unix `shm_open`), connection
  tracking, and CASC-compatible shmem file format
- `config` - Storage configuration with builder pattern
- `validation` - `BinaryFormatValidator` trait for round-trip testing, batch
  validation runner, and property-based testing utilities

## Usage

```rust,ignore
use cascette_client_storage::{Storage, StorageConfig};

// Initialize storage system
let config = StorageConfig::default()
    .with_path("/path/to/wow/data");

let storage = Storage::new(config)?;
let installation = storage.open_installation("wow_retail")?;
```

## Installed product metadata

With `local-install`, `read_installed_product(install_root, product)` reads only
`<install_root>/.product.db` and returns the unique product's version, active
build key, and optional active install key. The read is limited to 16 MiB.
It does not use `.build.info`, check `playable`, or prove installation completeness.
Active-key selection is a library policy, not verified Battle.net behavior.
The minimal protobuf field definitions follow
[TACTLib's generated ProtoDatabase.cs](https://github.com/overtools/TACTLib/blob/master/TACTLib/Agent/Protobuf/ProtoDatabase.cs):
Database.productInstall (1), ProductInstall.productCode (2) and
cachedProductState (4), CachedProductState.baseProductState (1),
BaseProductState.currentVersionStr (7), activeBuildKey (14), and
activeInstallKey (16). Completed (12) and incomplete (18) keys are ignored.

## Dependencies

- `cascette-formats` - BLTE, encoding, and root file parsers
- `cascette-crypto` - Content keys, encoding keys, and Jenkins96 hashing
- `binrw` - Binary format serialization for index entries and IPC messages
- `prost` 0.14 - Decode the minimal product database protobuf messages; avoids
  handwritten wire parsing and external `protoc` code generation
- `memmap2` - Memory-mapped file I/O for archive access
- `tokio` - Async runtime for installation operations
- `dashmap` - Concurrent hash maps for caching
- `parking_lot` - Synchronous read-write locks
- `tracing` - Logging and diagnostics
- `winapi` - Windows shared memory *(Windows only)*
- `libc` - Unix shared memory *(Unix only)*

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](../../LICENSE-APACHE) or
  <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](../../LICENSE-MIT) or
  <http://opensource.org/licenses/MIT>)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.

---

**Note**: This project is not affiliated with Blizzard Entertainment. It is
an independent implementation based on reverse engineering by the World of
Warcraft emulation community.
