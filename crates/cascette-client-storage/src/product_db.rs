//! Reads the selected product's active build from the local Battle.net `.product.db`.
//!
//! The minimal protobuf tags follow TACTLib's generated `ProtoDatabase.cs`:
//! <https://github.com/overtools/TACTLib/blob/master/TACTLib/Agent/Protobuf/ProtoDatabase.cs>.
//! Unknown fields are intentionally ignored. Active-key selection is this library's policy,
//! not a claim of native Battle.net conformance or complete-install validation.

use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

use prost::Message;
use thiserror::Error;

const MAX_DATABASE_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Message)]
struct Database {
    #[prost(message, repeated, tag = "1")]
    product_install: Vec<ProductInstall>,
}

#[derive(Message)]
struct ProductInstall {
    #[prost(string, tag = "2")]
    product_code: String,
    #[prost(message, optional, tag = "4")]
    cached_product_state: Option<CachedProductState>,
}

#[derive(Message)]
struct CachedProductState {
    #[prost(message, optional, tag = "1")]
    base_product_state: Option<BaseProductState>,
}

#[derive(Message)]
struct BaseProductState {
    #[prost(string, tag = "7")]
    current_version_str: String,
    #[prost(string, tag = "14")]
    active_build_key: String,
    #[prost(string, tag = "16")]
    active_install_key: String,
}

/// Installed product's locally selected active metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledProduct {
    /// Requested product code.
    pub product: String,
    /// Current version string in the cached product state.
    pub version: String,
    /// Active build configuration key (32 hex characters).
    pub build_key: String,
    /// Active install key (empty when absent, otherwise 32 hex characters).
    pub install_key: String,
}

/// Failure reading or selecting a product from `.product.db`.
#[derive(Debug, Error)]
pub enum ProductDbError {
    /// Database file could not be read.
    #[error("product {product} in {}: {source}", path.display())]
    Io {
        /// Database path.
        path: PathBuf,
        /// Requested product.
        product: String,
        /// Filesystem error.
        #[source]
        source: std::io::Error,
    },
    /// Database exceeded the read limit.
    #[error("product {product} in {}: .product.db exceeds {MAX_DATABASE_BYTES} bytes", path.display())]
    TooLarge {
        /// Database path.
        path: PathBuf,
        /// Requested product.
        product: String,
    },
    /// Database did not contain a valid protobuf message.
    #[error("product {product} in {}: invalid protobuf: {source}", path.display())]
    Decode {
        /// Database path.
        path: PathBuf,
        /// Requested product.
        product: String,
        /// Protobuf decode error.
        #[source]
        source: prost::DecodeError,
    },
    /// No record matched the requested product.
    #[error("product {product} not found in {}", path.display())]
    NotFound {
        /// Database path.
        path: PathBuf,
        /// Requested product.
        product: String,
    },
    /// Multiple records matched the requested product.
    #[error("duplicate product {product} in {}", path.display())]
    Duplicate {
        /// Database path.
        path: PathBuf,
        /// Requested product.
        product: String,
    },
    /// Selected product's metadata was absent or invalid.
    #[error("product {product} in {}: invalid {field}", path.display())]
    InvalidField {
        /// Database path.
        path: PathBuf,
        /// Requested product.
        product: String,
        /// Name of the invalid field.
        field: &'static str,
    },
}

/// Read only `<install_root>/.product.db` and select exactly one matching product.
///
/// Selection uses `activeBuildKey` rather than completed or incomplete build keys.
/// `playable` is not checked: local metadata remains readable when it is false.
/// This does not check whether CASC content is complete or executable.
///
/// # Errors
///
/// Returns a contextual error on I/O, oversized or malformed data, absent or
/// duplicate products, or invalid active metadata.
pub fn read_installed_product(
    install_root: &Path,
    product: &str,
) -> Result<InstalledProduct, ProductDbError> {
    let path = install_root.join(".product.db");
    let mut bytes = Vec::new();
    File::open(&path)
        .and_then(|file| file.take(MAX_DATABASE_BYTES + 1).read_to_end(&mut bytes))
        .map_err(|source| ProductDbError::Io {
            path: path.clone(),
            product: product.to_owned(),
            source,
        })?;
    if bytes.len() as u64 > MAX_DATABASE_BYTES {
        return Err(ProductDbError::TooLarge {
            path,
            product: product.to_owned(),
        });
    }
    let database = Database::decode(bytes.as_slice()).map_err(|source| ProductDbError::Decode {
        path: path.clone(),
        product: product.to_owned(),
        source,
    })?;
    let mut matches = database
        .product_install
        .into_iter()
        .filter(|entry| entry.product_code == product);
    let selected = matches.next().ok_or_else(|| ProductDbError::NotFound {
        path: path.clone(),
        product: product.to_owned(),
    })?;
    if matches.next().is_some() {
        return Err(ProductDbError::Duplicate {
            path,
            product: product.to_owned(),
        });
    }
    let invalid = |field| ProductDbError::InvalidField {
        path: path.clone(),
        product: product.to_owned(),
        field,
    };
    let base = selected
        .cached_product_state
        .and_then(|state| state.base_product_state)
        .ok_or_else(|| invalid("cached_product_state.base_product_state"))?;
    if base.current_version_str.is_empty() {
        return Err(invalid("version"));
    }
    if !is_hash(&base.active_build_key) {
        return Err(invalid("build_key"));
    }
    if !base.active_install_key.is_empty() && !is_hash(&base.active_install_key) {
        return Err(invalid("install_key"));
    }
    Ok(InstalledProduct {
        product: product.to_owned(),
        version: base.current_version_str,
        build_key: base.active_build_key,
        install_key: base.active_install_key,
    })
}

fn is_hash(key: &str) -> bool {
    key.len() == 32 && key.bytes().all(|byte| byte.is_ascii_hexdigit())
}
