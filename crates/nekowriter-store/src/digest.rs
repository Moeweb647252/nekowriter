//! SHA-256 digests as lowercase hexadecimal text.

use std::fmt::Write as _;

use sha2::{Digest, Sha256};

/// The SHA-256 of the concatenation of `parts`, as 64 lowercase hexadecimal digits.
pub(crate) fn sha256_hex<'a>(parts: impl IntoIterator<Item = &'a [u8]>) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part);
    }
    hasher
        .finalize()
        .iter()
        .fold(String::with_capacity(64), |mut hex, byte| {
            write!(hex, "{byte:02x}").expect("writing to a String cannot fail");
            hex
        })
}
