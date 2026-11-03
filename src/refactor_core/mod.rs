//! Shared core of the two Go refactor engines (`go_refactor`, the CLI, and `refactor`, the MCP
//! engine): content hashing, bounded command execution, path guards and the write-ahead-logged
//! transaction that makes a multi-file edit all-or-nothing. Both engines plan differently but
//! must apply, verify and roll back identically, so none of that lives in either engine.

pub mod exec;
pub mod guard;
pub mod txn;

use sha2::{Digest, Sha256};

/// Lowercase hex SHA-256, the content identity used by plans and the transaction log.
pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
