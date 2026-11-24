//! Bounded decoding of the observed XZ-compressed Mo record wrapper.
//!
//! Encoding 31 stores a big-endian decoded byte count followed by one XZ
//! stream. The decoded bytes include the complete inner Mo header. This does
//! not handle other compression encodings, encryption, signatures or archives.

use lzma_rust2::{Action, CheckType, Status, XzStream};

use super::log_image::{MO_HEADER_SIZE, MoRecord, MoRecordHeader};

pub const XZ_RECORD_ENCODING: u8 = 31;

/// Decode one encoding-31 record and validate the complete plain inner record.
///
/// The declared full output length is checked against `max_output_bytes` before
/// allocating. `max_memory_kib` separately bounds each XZ block's dictionary and
/// the decoder's estimated LZMA2 overhead; it does not include the output buffer.
/// Zero and `u32::MAX` are rejected because the latter disables the decoder's
/// memory limit. No recursive transformation is attempted.
///
/// A checked XZ stream must consume the entire wrapper payload: concatenated
/// streams, trailing junk and optional standalone-XZ stream padding are rejected.
/// The inner Mo type and byte-2 control must match the wrapper, encoding must be
/// zero, and its declared payload must fill the decoded span exactly.
pub fn decode_xz_record(
    record: &MoRecord<'_>,
    max_output_bytes: usize,
    max_memory_kib: u32,
) -> Result<Vec<u8>, String> {
    if record.header.encoding != XZ_RECORD_ENCODING {
        return Err(format!(
            "unsupported Mo transformation encoding {}; expected {XZ_RECORD_ENCODING}",
            record.header.encoding
        ));
    }
    if usize::try_from(record.header.payload_len).ok() != Some(record.payload.len()) {
        return Err("Mo wrapper payload length does not match its supplied span".into());
    }
    if max_memory_kib == 0 || max_memory_kib == u32::MAX {
        return Err("XZ decoder memory limit must be between 1 and u32::MAX-1 KiB".into());
    }
    let prefix: [u8; 4] = record
        .payload
        .get(..4)
        .ok_or("XZ Mo wrapper is missing its four-byte decoded length")?
        .try_into()
        .unwrap();
    let declared = usize::try_from(u32::from_be_bytes(prefix))
        .map_err(|_| "XZ decoded length exceeds the addressable size")?;
    if declared > max_output_bytes {
        return Err(format!(
            "XZ record declares {declared} decoded bytes, exceeding the {max_output_bytes}-byte output limit"
        ));
    }
    if declared < MO_HEADER_SIZE {
        return Err(format!(
            "XZ decoded length {declared} cannot contain a {MO_HEADER_SIZE}-byte Mo header"
        ));
    }
    let compressed = &record.payload[4..];
    if compressed.len() < 12 || &compressed[..6] != b"\xfd7zXZ\0" {
        return Err("XZ record decoding failed: invalid or truncated XZ stream header".into());
    }
    if compressed[6] == 0
        && (compressed[7] & 0xf0) == 0
        && !matches!(compressed[7] & 0x0f, 0x01 | 0x04 | 0x0a)
    {
        return Err("XZ stream has no supported content integrity check".into());
    }
    let mut output = Vec::new();
    let chunk_size = declared.min(64 * 1024);
    let mut chunk = Vec::new();
    chunk
        .try_reserve_exact(chunk_size)
        .map_err(|_| "could not allocate the bounded XZ output buffer")?;
    chunk.resize(chunk_size, 0);
    let mut decoder = XzStream::new_mem_limit(false, max_memory_kib);
    let mut input_position = 0;
    let mut overflow_probe = [0; 1];
    loop {
        // Once the declared output is full, a one-byte stack probe lets the
        // decoder finish checks/index/footer without allocating extra output.
        let remaining = declared - output.len();
        let output_full = remaining == 0;
        let destination = if output_full {
            overflow_probe.as_mut_slice()
        } else {
            let want = remaining.min(chunk.len());
            &mut chunk[..want]
        };
        let step = decoder
            .process(&compressed[input_position..], destination, Action::Finish)
            .map_err(|error| format!("XZ record decoding failed: {error}"))?;
        input_position += step.bytes_consumed;
        if output_full && step.bytes_produced != 0 {
            return Err(format!(
                "XZ output exceeds its declared {declared}-byte length"
            ));
        }
        if step.bytes_produced != 0 {
            output
                .try_reserve(step.bytes_produced)
                .map_err(|_| "could not allocate the bounded XZ output buffer")?;
            output.extend_from_slice(&chunk[..step.bytes_produced]);
        }
        if step.status == Status::StreamEnd {
            break;
        }
        if step.bytes_consumed == 0 && step.bytes_produced == 0 {
            return Err("XZ stream is truncated or cannot make progress".into());
        }
    }
    if input_position != compressed.len() {
        return Err(format!(
            "XZ wrapper contains {} trailing bytes after its single stream",
            compressed.len() - input_position
        ));
    }
    if output.len() != declared {
        return Err(format!(
            "XZ output length {} does not match its declared {declared} bytes",
            output.len()
        ));
    }
    if !matches!(
        decoder.check_type(),
        Some(CheckType::Crc32 | CheckType::Crc64 | CheckType::Sha256)
    ) {
        return Err("XZ stream has no supported content integrity check".into());
    }
    let inner = MoRecordHeader::parse(&output)
        .map_err(|error| format!("invalid decoded Mo header: {error}"))?;
    if inner.record_type != record.header.record_type || inner.flags != record.header.flags {
        return Err("decoded Mo type or byte-2 control does not match its XZ wrapper".into());
    }
    if inner.encoding != 0 {
        return Err(format!(
            "decoded Mo record has encoding {}; expected a plain record",
            inner.encoding
        ));
    }
    let inner_length = usize::try_from(inner.payload_len)
        .ok()
        .and_then(|length| MO_HEADER_SIZE.checked_add(length));
    if inner_length != Some(output.len()) {
        return Err("decoded Mo payload length does not fill its exact decoded span".into());
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::m1::log_image::{MoRecordIter, RawLogImage};

    const KIND: u32 = 0x0201_002c;
    const INDEX: u8 = 7;
    const MEMORY_KIB: u32 = 1024;

    fn plain_record() -> Vec<u8> {
        let payload = b"<?xml version=\"1.0\"?><test/>";
        let mut bytes = b"Mo".to_vec();
        bytes.extend_from_slice(&[INDEX, 0]);
        bytes.extend_from_slice(&KIND.to_be_bytes());
        bytes.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        bytes.extend_from_slice(payload);
        bytes
    }

    fn varint(mut value: usize, output: &mut Vec<u8>) {
        while value >= 128 {
            output.push((value as u8 & 0x7f) | 0x80);
            value >>= 7;
        }
        output.push(value as u8);
    }

    // Independent test encoder: one uncompressed LZMA2 chunk in a valid XZ
    // container. This exercises the production decoder without enabling the
    // crate's encoder feature or checking private fixture bytes into the repo.
    fn synthetic_xz(data: &[u8], check: bool) -> Vec<u8> {
        assert!(!data.is_empty() && data.len() <= 65_536);
        let flags = [0, u8::from(check)]; // None or CRC32.
        let mut xz = b"\xfd7zXZ\0".to_vec();
        xz.extend_from_slice(&flags);
        xz.extend_from_slice(&crc32fast::hash(&flags).to_le_bytes());
        let header = [2, 0, 0x21, 1, 0, 0, 0, 0]; // LZMA2, 4-KiB dictionary.
        xz.extend_from_slice(&header);
        xz.extend_from_slice(&crc32fast::hash(&header).to_le_bytes());
        xz.push(1); // Uncompressed chunk with dictionary reset.
        xz.extend_from_slice(&((data.len() - 1) as u16).to_be_bytes());
        xz.extend_from_slice(data);
        xz.push(0); // LZMA2 end marker.
        let compressed_size = data.len() + 4;
        xz.resize(xz.len().next_multiple_of(4), 0);
        if check {
            xz.extend_from_slice(&crc32fast::hash(data).to_le_bytes());
        }
        let mut index = vec![0, 1]; // Index indicator and one block.
        varint(12 + compressed_size + usize::from(check) * 4, &mut index);
        varint(data.len(), &mut index);
        index.resize(index.len().next_multiple_of(4), 0);
        index.extend_from_slice(&crc32fast::hash(&index).to_le_bytes());
        let backward_size = (index.len() / 4 - 1) as u32;
        xz.extend_from_slice(&index);
        let mut footer = backward_size.to_le_bytes().to_vec();
        footer.extend_from_slice(&flags);
        xz.extend_from_slice(&crc32fast::hash(&footer).to_le_bytes());
        xz.extend_from_slice(&footer);
        xz.extend_from_slice(b"YZ");
        xz
    }

    fn wrapper_payload(inner: &[u8]) -> Vec<u8> {
        let mut bytes = (inner.len() as u32).to_be_bytes().to_vec();
        bytes.extend_from_slice(&synthetic_xz(inner, true));
        bytes
    }

    fn wrapper(payload: &[u8]) -> MoRecord<'_> {
        MoRecord {
            offset: 0,
            header: MoRecordHeader {
                flags: INDEX,
                encoding: XZ_RECORD_ENCODING,
                record_type: KIND,
                payload_len: payload.len() as u32,
            },
            payload,
        }
    }

    #[test]
    fn decodes_one_checked_stream_to_the_exact_plain_record() {
        let plain = plain_record();
        let payload = wrapper_payload(&plain);
        assert_eq!(
            decode_xz_record(&wrapper(&payload), plain.len(), MEMORY_KIB).unwrap(),
            plain
        );
    }

    #[test]
    fn output_cap_and_prefix_are_checked_before_decoding() {
        let mut payload = wrapper_payload(&plain_record());
        payload[4] = 0; // Invalid XZ must not be touched after a size rejection.
        assert!(
            decode_xz_record(&wrapper(&payload), 12, MEMORY_KIB)
                .unwrap_err()
                .contains("output limit")
        );
        payload[..4].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(
            decode_xz_record(&wrapper(&payload), 1024, MEMORY_KIB)
                .unwrap_err()
                .contains("output limit")
        );
        for length in 0..4 {
            assert!(
                decode_xz_record(&wrapper(&payload[..length]), 1024, MEMORY_KIB)
                    .unwrap_err()
                    .contains("four-byte")
            );
        }
        for length in [0u32, 11] {
            payload[..4].copy_from_slice(&length.to_be_bytes());
            assert!(
                decode_xz_record(&wrapper(&payload), 1024, MEMORY_KIB)
                    .unwrap_err()
                    .contains("Mo header")
            );
        }
    }

    #[test]
    fn rejects_both_directions_of_declared_output_length_mismatch() {
        let plain = plain_record();
        let mut payload = wrapper_payload(&plain);
        for length in [plain.len() - 1, plain.len() + 1] {
            payload[..4].copy_from_slice(&(length as u32).to_be_bytes());
            assert!(decode_xz_record(&wrapper(&payload), 1024, MEMORY_KIB).is_err());
        }
    }

    #[test]
    fn rejects_corruption_and_every_truncated_stream_prefix() {
        let payload = wrapper_payload(&plain_record());
        for length in 4..payload.len() {
            assert!(
                decode_xz_record(&wrapper(&payload[..length]), 1024, MEMORY_KIB).is_err(),
                "accepted truncated payload length {length}"
            );
        }
        for offset in [4, 12, 4 + 27 + MO_HEADER_SIZE, payload.len() - 6] {
            let mut corrupted = payload.clone();
            corrupted[offset] ^= 1;
            assert!(decode_xz_record(&wrapper(&corrupted), 1024, MEMORY_KIB).is_err());
        }
    }

    #[test]
    fn rejects_extra_streams_junk_padding_and_unchecked_content() {
        let plain = plain_record();
        let payload = wrapper_payload(&plain);
        for tail in [
            vec![0],
            vec![0; 4],
            b"junk".to_vec(),
            synthetic_xz(&plain, true),
        ] {
            let mut extra = payload.clone();
            extra.extend_from_slice(&tail);
            assert!(
                decode_xz_record(&wrapper(&extra), 1024, MEMORY_KIB)
                    .unwrap_err()
                    .contains("trailing bytes")
            );
        }
        let mut unchecked = (plain.len() as u32).to_be_bytes().to_vec();
        unchecked.extend_from_slice(&synthetic_xz(&plain, false));
        assert!(
            decode_xz_record(&wrapper(&unchecked), 1024, MEMORY_KIB)
                .unwrap_err()
                .contains("integrity check")
        );
    }

    #[test]
    fn dictionary_limit_rejects_large_claims_before_dictionary_allocation() {
        let mut payload = wrapper_payload(&plain_record());
        assert!(decode_xz_record(&wrapper(&payload), 1024, 1).is_err());
        // LZMA2 property40 requests the maximum dictionary. Recompute the block
        // header CRC so the rejection tests the memory bound, not corruption.
        payload[4 + 12 + 4] = 40;
        let checksum = crc32fast::hash(&payload[4 + 12..4 + 20]);
        payload[4 + 20..4 + 24].copy_from_slice(&checksum.to_le_bytes());
        let error = decode_xz_record(&wrapper(&payload), 1024, MEMORY_KIB).unwrap_err();
        assert!(error.contains("memory"), "{error}");
        for memory in [0, u32::MAX] {
            assert!(
                decode_xz_record(&wrapper(&payload), 1024, memory)
                    .unwrap_err()
                    .contains("memory limit")
            );
        }
    }

    #[test]
    fn rejects_unsupported_wrappers_and_mismatched_or_nested_inner_headers() {
        let plain = plain_record();
        let payload = wrapper_payload(&plain);
        let mut outer = wrapper(&payload);
        outer.header.encoding = 30;
        assert!(
            decode_xz_record(&outer, 1024, MEMORY_KIB)
                .unwrap_err()
                .contains("unsupported")
        );
        outer.header.encoding = XZ_RECORD_ENCODING;
        outer.header.payload_len -= 1;
        assert!(
            decode_xz_record(&outer, 1024, MEMORY_KIB)
                .unwrap_err()
                .contains("supplied span")
        );
        for offset in [0, 2, 3, 7, 11] {
            let mut invalid = plain.clone();
            invalid[offset] ^= 1;
            let payload = wrapper_payload(&invalid);
            assert!(
                decode_xz_record(&wrapper(&payload), 1024, MEMORY_KIB).is_err(),
                "accepted invalid inner header byte {offset}"
            );
        }
        let mut trailing = plain;
        trailing.push(0);
        let payload = wrapper_payload(&trailing);
        assert!(
            decode_xz_record(&wrapper(&payload), 1024, MEMORY_KIB)
                .unwrap_err()
                .contains("exact decoded span")
        );
    }

    #[test]
    #[ignore = "requires local M1 logged-data image; set I3RS_M1_IMAGE"]
    fn captured_package_xml_decodes_with_matching_plain_header() {
        let path = std::env::var_os("I3RS_M1_IMAGE")
            .expect("set I3RS_M1_IMAGE to the local M1 logged-data image");
        let bytes = std::fs::read(path).unwrap();
        RawLogImage::parse(&bytes).unwrap();
        let offset = 0x2cd2d4;
        let record = MoRecordIter::new(&bytes[offset..], 1)
            .next()
            .unwrap()
            .unwrap();
        assert_eq!(record.header.record_type, 0x0100_0004);
        assert_eq!(record.header.encoding, XZ_RECORD_ENCODING);
        let decoded = decode_xz_record(&record, 2 * 1024 * 1024, 64 * 1024).unwrap();
        assert_eq!(decoded.len(), 1_800_578);
        let header = MoRecordHeader::parse(&decoded).unwrap();
        assert_eq!(header.flags, record.header.flags);
        assert_eq!(header.encoding, 0);
        assert_eq!(header.record_type, record.header.record_type);
        assert_eq!(header.payload_len as usize + MO_HEADER_SIZE, decoded.len());
        let folder = MoRecordIter::new(&decoded[MO_HEADER_SIZE..], 4)
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let file = folder
            .iter()
            .find(|record| record.header.record_type == 0x0100_0003)
            .unwrap();
        let contents = MoRecordIter::new(file.payload, 4)
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let xml = contents
            .iter()
            .find(|record| record.header.record_type == 0x0001_0007)
            .unwrap();
        assert!(
            std::str::from_utf8(xml.payload)
                .unwrap()
                .starts_with("<?xml")
        );
    }
}
