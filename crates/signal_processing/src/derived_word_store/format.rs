use super::errors::{CodecError, CodecResult};

pub(crate) const FORMAT_VERSION: u32 = 1;
const BLOCK_MAGIC: &[u8; 4] = b"DWBL";
pub(crate) const BLOCK_FLAG_HAS_DURATIONS: u16 = 1 << 0;
pub(crate) const BLOCK_FLAG_HAS_PAYLOADS: u16 = 1 << 1;
pub(crate) const BLOCK_FLAG_GROUPED_TIMESTAMPS: u16 = 1 << 2;
pub(crate) const BLOCK_HEADER_SIZE: usize = 72;
pub(crate) const RESTART_ENTRY_SIZE: usize = 16;
pub(crate) const BLOCK_CHECKSUM_OFFSET: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WordBlockHeader {
    pub flags: u16,
    pub sequence: u64,
    pub first_timestamp_ns: u64,
    pub last_timestamp_ns: u64,
    pub word_count: u32,
    pub value_bytes: u8,
    pub record_payload_len: u32,
    pub restart_count: u32,
    pub restart_table_offset: u32,
    pub duration_count: u32,
    pub duration_table_offset: u32,
    pub block_len: u32,
    pub crc32c: u32,
}

impl WordBlockHeader {
    pub(crate) fn write_to(self, bytes: &mut [u8]) {
        debug_assert!(bytes.len() >= BLOCK_HEADER_SIZE);
        bytes[..BLOCK_HEADER_SIZE].fill(0);
        bytes[..4].copy_from_slice(BLOCK_MAGIC);
        put_u16(bytes, 4, BLOCK_HEADER_SIZE as u16);
        put_u16(bytes, 6, self.flags);
        put_u64(bytes, 8, self.sequence);
        put_u64(bytes, 16, self.first_timestamp_ns);
        put_u64(bytes, 24, self.last_timestamp_ns);
        put_u32(bytes, 32, self.word_count);
        bytes[36] = self.value_bytes;
        put_u32(bytes, 40, self.record_payload_len);
        put_u32(bytes, 44, self.restart_count);
        put_u32(bytes, 48, self.restart_table_offset);
        put_u32(bytes, 52, self.duration_count);
        put_u32(bytes, 56, self.duration_table_offset);
        put_u32(bytes, 60, self.block_len);
        put_u32(bytes, BLOCK_CHECKSUM_OFFSET, self.crc32c);
    }

    pub(crate) fn from_bytes(bytes: &[u8]) -> CodecResult<Self> {
        if bytes.len() < BLOCK_HEADER_SIZE {
            return Err(CodecError::Truncated);
        }
        if &bytes[..4] != BLOCK_MAGIC {
            return Err(invalid("invalid word-block magic"));
        }
        if get_u16(bytes, 4)? as usize != BLOCK_HEADER_SIZE {
            return Err(invalid("invalid word-block header size"));
        }
        if bytes[37..40].iter().any(|&byte| byte != 0)
            || bytes[68..72].iter().any(|&byte| byte != 0)
        {
            return Err(invalid("non-zero reserved word-block header bytes"));
        }
        Ok(Self {
            flags: get_u16(bytes, 6)?,
            sequence: get_u64(bytes, 8)?,
            first_timestamp_ns: get_u64(bytes, 16)?,
            last_timestamp_ns: get_u64(bytes, 24)?,
            word_count: get_u32(bytes, 32)?,
            value_bytes: bytes[36],
            record_payload_len: get_u32(bytes, 40)?,
            restart_count: get_u32(bytes, 44)?,
            restart_table_offset: get_u32(bytes, 48)?,
            duration_count: get_u32(bytes, 52)?,
            duration_table_offset: get_u32(bytes, 56)?,
            block_len: get_u32(bytes, 60)?,
            crc32c: get_u32(bytes, BLOCK_CHECKSUM_OFFSET)?,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RestartEntry {
    pub timestamp_ns: u64,
    /// Byte offset relative to the start of the record payload.
    pub payload_offset: u32,
    pub record_index: u32,
}

/// One fully written block published by the live store.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BlockDirectoryEntry {
    pub sequence: u64,
    pub first_timestamp_ns: u64,
    pub last_timestamp_ns: u64,
    pub data_offset: u64,
    pub block_len: u32,
    pub word_count: u32,
    pub value_bytes: u8,
    pub flags: u8,
}

impl RestartEntry {
    pub(crate) fn append_to(self, output: &mut Vec<u8>) {
        output.extend_from_slice(&self.timestamp_ns.to_le_bytes());
        output.extend_from_slice(&self.payload_offset.to_le_bytes());
        output.extend_from_slice(&self.record_index.to_le_bytes());
    }

    pub(crate) fn read_from(bytes: &[u8], offset: usize) -> CodecResult<Self> {
        Ok(Self {
            timestamp_ns: get_u64(bytes, offset)?,
            payload_offset: get_u32(bytes, offset + 8)?,
            record_index: get_u32(bytes, offset + 12)?,
        })
    }
}

fn invalid(message: &str) -> CodecError {
    CodecError::InvalidFormat(message.to_string())
}

fn put_u16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn put_u64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

fn get_u16(bytes: &[u8], offset: usize) -> CodecResult<u16> {
    let value = bytes
        .get(offset..offset + 2)
        .ok_or(CodecError::Truncated)?
        .try_into()
        .expect("fixed-size slice");
    Ok(u16::from_le_bytes(value))
}

fn get_u32(bytes: &[u8], offset: usize) -> CodecResult<u32> {
    let value = bytes
        .get(offset..offset + 4)
        .ok_or(CodecError::Truncated)?
        .try_into()
        .expect("fixed-size slice");
    Ok(u32::from_le_bytes(value))
}

fn get_u64(bytes: &[u8], offset: usize) -> CodecResult<u64> {
    let value = bytes
        .get(offset..offset + 8)
        .ok_or(CodecError::Truncated)?
        .try_into()
        .expect("fixed-size slice");
    Ok(u64::from_le_bytes(value))
}
