//! Parser for Blizzard TVFS file manifests.
//!
//! The older [`TvfsFile`](super::TvfsFile) structs model a convenient tree/table
//! representation used by the builder. Real WoW manifests use the compact
//! Agent/CascLib layout: path fragments plus `0xff + NodeValue` records,
//! recursive folder nodes, variable-width container table offsets, and VFS span
//! records. This module parses that shipped layout directly.

use cascette_crypto::{ContentKey, EncodingKey};

use crate::tvfs::error::{TvfsError, TvfsResult};

const TVFS_MAGIC: &[u8; 4] = b"TVFS";
const TVFS_FOLDER_NODE: u32 = 0x8000_0000;
const TVFS_FOLDER_SIZE_MASK: u32 = 0x7fff_ffff;

/// Parsed TVFS file manifest.
#[derive(Debug, Clone)]
pub struct TvfsManifest {
    /// TVFS header and table layout.
    pub header: TvfsManifestHeader,
    /// File records found in the manifest.
    pub entries: Vec<TvfsManifestEntry>,
}

/// TVFS manifest header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TvfsManifestHeader {
    /// Format version. Currently expected to be 1.
    pub format_version: u8,
    /// Header size in bytes.
    pub header_size: u8,
    /// Encoding key byte width. TACT uses 9-byte truncated EKeys here.
    pub ekey_size: u8,
    /// Patch key byte width.
    pub pkey_size: u8,
    /// TVFS flags.
    pub flags: u32,
    /// Path table offset.
    pub path_table_offset: u32,
    /// Path table size.
    pub path_table_size: u32,
    /// VFS table offset.
    pub vfs_table_offset: u32,
    /// VFS table size.
    pub vfs_table_size: u32,
    /// Container file table offset.
    pub cft_table_offset: u32,
    /// Container file table size.
    pub cft_table_size: u32,
    /// Maximum path depth.
    pub max_depth: u16,
    /// Encoding spec table offset, present when flag 0x02 is set.
    pub est_table_offset: Option<u32>,
    /// Encoding spec table size, present when flag 0x02 is set.
    pub est_table_size: Option<u32>,
    cft_offset_size: usize,
}

/// TVFS manifest file entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TvfsManifestEntry {
    /// Manifest path. WoW generic manifests encode metadata in this string.
    pub path: String,
    /// File data ID, when the path is a WoW generic name.
    pub file_data_id: Option<u32>,
    /// Locale flags, when the path is a WoW generic name.
    pub locale_flags: Option<u32>,
    /// Content flags, when the path is a WoW generic name.
    pub content_flags: Option<u32>,
    /// Content key, when present in the path or container file table.
    pub content_key: Option<ContentKey>,
    /// Truncated encoding key from the container file table.
    pub encoding_key: EncodingKey,
    /// File offset inside a multi-span logical file.
    pub file_offset: u32,
    /// Span size / content size.
    pub file_size: u32,
}

impl TvfsManifest {
    /// Parse a decompressed TVFS manifest.
    pub fn parse(data: &[u8]) -> TvfsResult<Self> {
        let header = TvfsManifestHeader::parse(data)?;
        let mut entries = Vec::new();
        let path_start = header.path_table_offset as usize;
        let path_end = checked_end(
            path_start,
            header.path_table_size as usize,
            data.len(),
            "path table",
        )?;
        let mut cursor = path_start;
        let mut end = path_end;

        if cursor + 5 <= end && data[cursor] == 0xff {
            let node_value = read_u32_be(data, cursor + 1)?;
            if node_value & TVFS_FOLDER_NODE == 0 {
                return Err(invalid_node(cursor, "root node is not a folder"));
            }
            let folder_len = (node_value & TVFS_FOLDER_SIZE_MASK) as usize;
            end = checked_end(cursor + 1, folder_len, data.len(), "root folder")?;
            cursor += 5;
        }

        parse_path_entries(data, header, cursor, end, String::new(), &mut entries)?;
        Ok(Self { header, entries })
    }
}

impl TvfsManifestHeader {
    /// Parse a TVFS manifest header.
    pub fn parse(data: &[u8]) -> TvfsResult<Self> {
        if data.len() < 38 {
            return Err(invalid_node(0, "TVFS header is truncated"));
        }
        if &data[0..4] != TVFS_MAGIC {
            let mut magic = [0u8; 4];
            magic.copy_from_slice(&data[0..4]);
            return Err(TvfsError::InvalidMagic(magic));
        }

        let format_version = data[4];
        if format_version != 1 {
            return Err(TvfsError::UnsupportedVersion(format_version));
        }

        let header_size = data[5];
        if header_size < 38 || data.len() < header_size as usize {
            return Err(TvfsError::InvalidHeaderSize(header_size));
        }

        let ekey_size = data[6];
        let pkey_size = data[7];
        if ekey_size == 0 || ekey_size > 16 || pkey_size == 0 || pkey_size > 16 {
            return Err(TvfsError::InvalidKeySize {
                ekey: ekey_size,
                pkey: pkey_size,
            });
        }

        let flags = read_u32_be(data, 8)?;
        let path_table_offset = read_u32_be(data, 12)?;
        let path_table_size = read_u32_be(data, 16)?;
        let vfs_table_offset = read_u32_be(data, 20)?;
        let vfs_table_size = read_u32_be(data, 24)?;
        let cft_table_offset = read_u32_be(data, 28)?;
        let cft_table_size = read_u32_be(data, 32)?;
        let max_depth = read_u16_be(data, 36)?;
        let (est_table_offset, est_table_size) = if flags & 0x02 != 0 {
            (Some(read_u32_be(data, 38)?), Some(read_u32_be(data, 42)?))
        } else {
            (None, None)
        };

        for (offset, size, name) in [
            (path_table_offset, path_table_size, "path table"),
            (vfs_table_offset, vfs_table_size, "vfs table"),
            (cft_table_offset, cft_table_size, "container table"),
        ] {
            checked_end(offset as usize, size as usize, data.len(), name)?;
        }

        Ok(Self {
            format_version,
            header_size,
            ekey_size,
            pkey_size,
            flags,
            path_table_offset,
            path_table_size,
            vfs_table_offset,
            vfs_table_size,
            cft_table_offset,
            cft_table_size,
            max_depth,
            est_table_offset,
            est_table_size,
            cft_offset_size: offset_size(cft_table_size as usize),
        })
    }
}

fn parse_path_entries(
    data: &[u8],
    header: TvfsManifestHeader,
    mut cursor: usize,
    end: usize,
    mut path: String,
    entries: &mut Vec<TvfsManifestEntry>,
) -> TvfsResult<()> {
    while cursor < end {
        let saved_len = path.len();
        let entry = capture_path_entry(data, cursor, end)?;
        cursor = entry.next;

        if entry.separator_pre && !path.is_empty() && !path.ends_with('/') {
            path.push('/');
        }
        path.push_str(entry.name);
        if entry.separator_post && !path.ends_with('/') {
            path.push('/');
        }

        if let Some(node_value) = entry.node_value {
            if node_value & TVFS_FOLDER_NODE != 0 {
                let folder_len = (node_value & TVFS_FOLDER_SIZE_MASK) as usize;
                if folder_len < 4 {
                    return Err(invalid_node(
                        cursor,
                        "folder node is shorter than node value",
                    ));
                }
                let directory_end = checked_end(cursor, folder_len - 4, end, "folder node")?;
                parse_path_entries(data, header, cursor, directory_end, path.clone(), entries)?;
                cursor = directory_end;
            } else {
                append_vfs_entries(data, header, node_value as usize, &path, entries)?;
            }
            path.truncate(saved_len);
        }
    }
    Ok(())
}

struct PathEntry<'a> {
    name: &'a str,
    separator_pre: bool,
    separator_post: bool,
    node_value: Option<u32>,
    next: usize,
}

fn capture_path_entry(data: &[u8], mut cursor: usize, end: usize) -> TvfsResult<PathEntry<'_>> {
    let start = cursor;
    let separator_pre = cursor < end && data[cursor] == 0;
    if separator_pre {
        cursor += 1;
    }

    let name_start;
    let name_end;
    if cursor < end && data[cursor] != 0xff {
        let len = data[cursor] as usize;
        cursor += 1;
        name_start = cursor;
        name_end = checked_end(cursor, len, end, "path name")?;
        cursor = name_end;
    } else {
        name_start = cursor;
        name_end = cursor;
    }

    let mut separator_post = cursor < end && data[cursor] == 0;
    if separator_post {
        cursor += 1;
    }

    let node_value = if cursor < end && data[cursor] == 0xff {
        let value = read_u32_be(data, cursor + 1)?;
        cursor += 5;
        Some(value)
    } else {
        if cursor < end {
            separator_post = true;
        }
        None
    };

    let name = std::str::from_utf8(&data[name_start..name_end])
        .map_err(|e| invalid_node(start, format!("invalid UTF-8 path fragment: {e}")))?;

    Ok(PathEntry {
        name,
        separator_pre,
        separator_post,
        node_value,
        next: cursor,
    })
}

fn append_vfs_entries(
    data: &[u8],
    header: TvfsManifestHeader,
    vfs_offset: usize,
    path: &str,
    entries: &mut Vec<TvfsManifestEntry>,
) -> TvfsResult<()> {
    let vfs_start = header.vfs_table_offset as usize;
    let vfs_end = vfs_start + header.vfs_table_size as usize;
    let mut cursor = checked_end(vfs_start, vfs_offset, vfs_end, "vfs offset")?;
    if cursor >= vfs_end {
        return Err(invalid_node(cursor, "VFS offset points past table"));
    }

    let span_count = data[cursor] as usize;
    cursor += 1;
    if !(1..=224).contains(&span_count) {
        return Ok(());
    }

    let generic = parse_wow_generic_path(path);
    for _ in 0..span_count {
        let span_size = 8 + header.cft_offset_size;
        checked_end(cursor, span_size, vfs_end, "VFS span")?;
        let file_offset = read_u32_be(data, cursor)?;
        let file_size = read_u32_be(data, cursor + 4)?;
        let cft_offset =
            read_uint_be(&data[cursor + 8..cursor + 8 + header.cft_offset_size]) as usize;
        cursor += span_size;

        let cft_cursor = checked_end(
            header.cft_table_offset as usize,
            cft_offset,
            data.len(),
            "CFT offset",
        )?;
        let cft_end = header.cft_table_offset as usize + header.cft_table_size as usize;
        checked_end(
            cft_cursor,
            header.ekey_size as usize,
            cft_end,
            "CFT encoding key",
        )?;

        let mut ekey = [0u8; 16];
        let ekey_size = header.ekey_size as usize;
        ekey[..ekey_size].copy_from_slice(&data[cft_cursor..cft_cursor + ekey_size]);

        entries.push(TvfsManifestEntry {
            path: path.trim_end_matches('/').to_string(),
            file_data_id: generic.as_ref().map(|g| g.file_data_id),
            locale_flags: generic.as_ref().map(|g| g.locale_flags),
            content_flags: generic.as_ref().map(|g| g.content_flags),
            content_key: generic.as_ref().map(|g| g.content_key),
            encoding_key: EncodingKey::from_bytes(ekey),
            file_offset,
            file_size,
        });
    }

    Ok(())
}

#[derive(Clone, Copy)]
struct WowGenericPath {
    locale_flags: u32,
    content_flags: u32,
    file_data_id: u32,
    content_key: ContentKey,
}

fn parse_wow_generic_path(path: &str) -> Option<WowGenericPath> {
    let path = path.trim_end_matches('/');
    if path.len() == 40 {
        let file_data_id = u32::from_str_radix(&path[..8], 16).ok()?;
        let content_key = ContentKey::from_hex(&path[8..40]).ok()?;
        return Some(WowGenericPath {
            locale_flags: 0,
            content_flags: 0,
            file_data_id,
            content_key,
        });
    }

    let (prefix, payload) = path.split_once(':')?;
    if !matches!(prefix.len(), 12 | 16) || payload.len() < 40 {
        return None;
    }
    let locale_flags = u32::from_str_radix(&prefix[..8], 16).ok()?;
    let content_flags = u32::from_str_radix(&prefix[8..], 16).ok()?;
    let file_data_id = u32::from_str_radix(&payload[..8], 16).ok()?;
    let content_key = ContentKey::from_hex(&payload[8..40]).ok()?;
    Some(WowGenericPath {
        locale_flags,
        content_flags,
        file_data_id,
        content_key,
    })
}

fn offset_size(size: usize) -> usize {
    match size {
        0 => 0,
        1..=0xff => 1,
        0x100..=0xffff => 2,
        0x1_0000..=0xff_ffff => 3,
        _ => 4,
    }
}

fn read_u16_be(data: &[u8], offset: usize) -> TvfsResult<u16> {
    let end = checked_end(offset, 2, data.len(), "u16")?;
    let bytes: [u8; 2] = data[offset..end]
        .try_into()
        .map_err(|_| invalid_node(offset, "invalid u16 range"))?;
    Ok(u16::from_be_bytes(bytes))
}

fn read_u32_be(data: &[u8], offset: usize) -> TvfsResult<u32> {
    let end = checked_end(offset, 4, data.len(), "u32")?;
    let bytes: [u8; 4] = data[offset..end]
        .try_into()
        .map_err(|_| invalid_node(offset, "invalid u32 range"))?;
    Ok(u32::from_be_bytes(bytes))
}

fn read_uint_be(bytes: &[u8]) -> u32 {
    bytes
        .iter()
        .fold(0u32, |value, byte| (value << 8) | u32::from(*byte))
}

fn checked_end(start: usize, len: usize, limit: usize, what: &str) -> TvfsResult<usize> {
    let end = start
        .checked_add(len)
        .ok_or_else(|| invalid_node(start, format!("{what} range overflows")))?;
    if end > limit {
        return Err(invalid_node(
            start,
            format!("{what} range {start}..{end} exceeds {limit}"),
        ));
    }
    Ok(end)
}

fn invalid_node(offset: usize, message: impl Into<String>) -> TvfsError {
    TvfsError::InvalidPathNode(offset, message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn push_entry(buf: &mut Vec<u8>, name: &str, node_value: u32) {
        buf.push(name.len() as u8);
        buf.extend_from_slice(name.as_bytes());
        buf.push(0xff);
        buf.extend_from_slice(&node_value.to_be_bytes());
    }

    #[test]
    fn parses_wow_generic_manifest_entry() {
        let path = "0000000000000000:0009661887cd491cd119e8c7aa48b562d7482ed8";
        let mut path_table = vec![0xff];
        let folder_len_pos = path_table.len();
        path_table.extend_from_slice(&0u32.to_be_bytes());
        let folder_start = path_table.len();
        push_entry(&mut path_table, path, 0);
        let folder_len = (path_table.len() - folder_start + 4) as u32 | TVFS_FOLDER_NODE;
        path_table[folder_len_pos..folder_len_pos + 4].copy_from_slice(&folder_len.to_be_bytes());

        let mut vfs_table = vec![1];
        vfs_table.extend_from_slice(&0u32.to_be_bytes());
        vfs_table.extend_from_slice(&0x9800u32.to_be_bytes());
        vfs_table.push(0);

        let mut cft_table = vec![0xdb, 0x47, 0x2f, 0xf5, 0xca, 0x74, 0x46, 0x5b, 0xaa];

        let header_size = 46usize;
        let path_table_offset = header_size;
        let vfs_table_offset = path_table_offset + path_table.len();
        let cft_table_offset = vfs_table_offset + vfs_table.len();

        let mut data = Vec::new();
        data.extend_from_slice(b"TVFS");
        data.extend_from_slice(&[1, 46, 9, 9]);
        data.extend_from_slice(&0x07u32.to_be_bytes());
        data.extend_from_slice(&(path_table_offset as u32).to_be_bytes());
        data.extend_from_slice(&(path_table.len() as u32).to_be_bytes());
        data.extend_from_slice(&(vfs_table_offset as u32).to_be_bytes());
        data.extend_from_slice(&(vfs_table.len() as u32).to_be_bytes());
        data.extend_from_slice(&(cft_table_offset as u32).to_be_bytes());
        data.extend_from_slice(&(cft_table.len() as u32).to_be_bytes());
        data.extend_from_slice(&1u16.to_be_bytes());
        data.extend_from_slice(&0u32.to_be_bytes());
        data.extend_from_slice(&0u32.to_be_bytes());
        data.append(&mut path_table);
        data.append(&mut vfs_table);
        data.append(&mut cft_table);

        let manifest = TvfsManifest::parse(&data).expect("manifest should parse");
        assert_eq!(manifest.entries.len(), 1);
        let entry = &manifest.entries[0];
        assert_eq!(entry.file_data_id, Some(615960));
        assert_eq!(
            entry.content_key.map(|key| key.to_hex()).as_deref(),
            Some("87cd491cd119e8c7aa48b562d7482ed8")
        );
        assert_eq!(
            entry.encoding_key.as_bytes()[..9],
            [0xdb, 0x47, 0x2f, 0xf5, 0xca, 0x74, 0x46, 0x5b, 0xaa]
        );
        assert_eq!(entry.file_size, 0x9800);
    }

    #[test]
    fn parses_wow_short_generic_manifest_entry() {
        let generic = parse_wow_generic_path("0009661887cd491cd119e8c7aa48b562d7482ed8").unwrap();
        assert_eq!(generic.file_data_id, 615960);
        assert_eq!(generic.locale_flags, 0);
        assert_eq!(generic.content_flags, 0);
        assert_eq!(
            generic.content_key.to_hex(),
            "87cd491cd119e8c7aa48b562d7482ed8"
        );
    }
}
