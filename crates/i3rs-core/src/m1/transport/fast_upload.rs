//! MoTeC's indexed, read-only Fast Upload extension.

use super::*;

const FAST_UPLOAD_HEADER: usize = 4;
const MAX_FAST_UPLOAD_BYTES: usize = 512 * 1024;

impl XcpClient {
    pub(crate) fn can_fast_upload(&self, length: usize) -> bool {
        let parameters = self.parameters();
        // Small requests stay on the standard path. Use the ECU's DTO width
        // for offsets, with a bounded burst and receive buffer.
        (512..=DATAGRAM_CAPACITY - super::super::xcp::HEADER_LENGTH)
            .contains(&usize::from(parameters.max_dto))
            && length > upload_capacity(parameters) * parameters.address_granularity.bytes()
            && length <= MAX_FAST_UPLOAD_BYTES
    }

    pub(super) fn fast_upload_exact(
        &mut self,
        length: usize,
        bytes: &mut Vec<u8>,
    ) -> Result<(), XcpError> {
        let parameters = self.parameters();
        let byte_order = parameters.byte_order;
        let width = usize::from(parameters.max_dto) - FAST_UPLOAD_HEADER;
        let mut request = [0xf1, 0xff, 0, 0, 0, 0, 0, 0];
        request[4..].copy_from_slice(&byte_order.write_u32(length as u32));

        let start = bytes.len();
        bytes.resize(start + length, 0);
        let destination = &mut bytes[start..];
        let mut seen = vec![false; length.div_ceil(width)];
        let mut missing = seen.len();
        // One stateful request advances the device's read position for the
        // block. Charge it even if a response is lost.
        self.requested_upload_bytes += length as u64;
        let (counter, deadline) = self.send_command(&request)?;
        while missing != 0 {
            let size = self.receive_datagram(deadline)?;
            for frame in decode_datagram(&self.datagram[..size])? {
                if frame.counter != counter {
                    continue;
                }
                let reply = frame.payload;
                match reply.first() {
                    Some(0xfe) => {
                        if let Err(ReplyError::DeviceError { code }) = check_reply(reply) {
                            return Err(XcpError::CommandRejected {
                                command: "Fast Upload",
                                code,
                            });
                        }
                        return Err(XcpError::InvalidFastUpload(
                            "expected an indexed data reply",
                        ));
                    }
                    Some(0xff) => {}
                    Some(_) => continue, // Unrelated event or DAQ packet.
                    None => return Err(XcpError::InvalidFastUpload("empty reply")),
                }
                if reply.len() < FAST_UPLOAD_HEADER {
                    return Err(XcpError::InvalidFastUpload("truncated chunk header"));
                }
                let index = usize::from(byte_order.read_u16([reply[2], reply[3]]));
                let Some(received) = seen.get_mut(index) else {
                    return Err(XcpError::InvalidFastUpload(
                        "chunk index exceeds the request",
                    ));
                };
                let offset = index * width;
                let data = &reply[FAST_UPLOAD_HEADER..];
                let expected = width.min(length - offset);
                if data.len() != expected {
                    return Err(XcpError::InvalidFastUpload("wrong chunk length"));
                }
                let slot = &mut destination[offset..offset + expected];
                if *received {
                    if slot != data {
                        return Err(XcpError::InvalidFastUpload("conflicting duplicate chunk"));
                    }
                    continue;
                }
                slot.copy_from_slice(data);
                *received = true;
                missing -= 1;
            }
        }
        Ok(())
    }
}
