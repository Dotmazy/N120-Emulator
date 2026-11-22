//! Bounded, versioned binary encoding for emulator save states.

use bincode::Options;
use serde::{Serialize, de::DeserializeOwned};

const MAX_PAYLOAD: u64 = 64 * 1024 * 1024;
const HEADER_SIZE: usize = 20;
pub const MAX_FILE_SIZE: usize = MAX_PAYLOAD as usize + HEADER_SIZE;

#[derive(Debug)]
pub struct SnapshotError(pub String);

impl std::fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SnapshotError {}

pub fn encode<T: Serialize + ?Sized>(value: &T) -> Result<Vec<u8>, SnapshotError> {
    bincode::DefaultOptions::new()
        .with_fixint_encoding()
        .with_limit(MAX_PAYLOAD)
        .serialize(value)
        .map_err(|error| SnapshotError(error.to_string()))
}

pub fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, SnapshotError> {
    bincode::DefaultOptions::new()
        .with_fixint_encoding()
        .with_limit(MAX_PAYLOAD)
        .reject_trailing_bytes()
        .deserialize(bytes)
        .map_err(|error| SnapshotError(error.to_string()))
}

/// Encode an eight-byte identifier, version, payload length, CRC32 and payload.
pub fn pack<T: Serialize>(
    identifier: [u8; 8],
    version: u32,
    value: &T,
) -> Result<Vec<u8>, SnapshotError> {
    let payload = encode(value)?;
    let mut bytes = Vec::with_capacity(HEADER_SIZE + payload.len());
    bytes.extend(identifier);
    bytes.extend(version.to_le_bytes());
    bytes.extend((payload.len() as u32).to_le_bytes());
    bytes.extend(crc32fast::hash(&payload).to_le_bytes());
    bytes.extend(payload);
    Ok(bytes)
}

pub fn unpack<T: DeserializeOwned>(
    identifier: [u8; 8],
    version: u32,
    bytes: &[u8],
) -> Result<T, SnapshotError> {
    if bytes.len() < HEADER_SIZE || bytes[..8] != identifier {
        return Err(SnapshotError("unrecognized save-state header".into()));
    }
    let word = |offset| u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
    if word(8) != version {
        return Err(SnapshotError("unsupported save-state version".into()));
    }
    let payload = &bytes[HEADER_SIZE..];
    if u64::from(word(12)) > MAX_PAYLOAD || word(12) as usize != payload.len() {
        return Err(SnapshotError("invalid save-state payload length".into()));
    }
    if word(16) != crc32fast::hash(payload) {
        return Err(SnapshotError("save-state checksum mismatch".into()));
    }
    decode(payload)
}

/// Serde helper for fixed arrays larger than its built-in 32-element support.
pub mod array {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<T: Serialize, S: Serializer, const N: usize>(
        value: &[T; N],
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        value.as_slice().serialize(serializer)
    }

    pub fn deserialize<'de, T: Deserialize<'de>, D: Deserializer<'de>, const N: usize>(
        deserializer: D,
    ) -> Result<[T; N], D::Error> {
        Vec::<T>::deserialize(deserializer)?
            .try_into()
            .map_err(|values: Vec<T>| {
                serde::de::Error::invalid_length(values.len(), &"a fixed-length state array")
            })
    }
}
