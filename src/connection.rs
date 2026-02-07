//! Connection utilities for MeshCore communication

use crate::packets::FRAME_START;

/// Frame a packet for transmission
///
/// Format: `[START: 0x3c][LENGTH_L][LENGTH_H][PAYLOAD]`
pub fn frame_packet(data: &[u8]) -> Vec<u8> {
    let len = data.len() as u16;
    let mut framed = Vec::with_capacity(3 + data.len());
    framed.push(FRAME_START);
    framed.push((len & 0xFF) as u8);
    framed.push((len >> 8) as u8);
    framed.extend_from_slice(data);
    framed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_frame_packet() {
        let data = vec![0x01, 0x02, 0x03];
        let framed = frame_packet(&data);

        assert_eq!(framed[0], FRAME_START);
        assert_eq!(framed[1], 0x03); // Length low byte
        assert_eq!(framed[2], 0x00); // Length high byte
        assert_eq!(&framed[3..], &data);
    }
}
