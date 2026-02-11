//! Binary parsing utilities for MeshCore protocol

use crate::error::Error;
use crate::events::{
    AclEntry, Contact, MmaEntry, Neighbour, NeighboursData, ReceivedMessage, SelfInfo, StatusData,
};
use crate::Result;

/// Read a little-endian u16 from a byte slice
pub fn read_u16_le(data: &[u8], offset: usize) -> Result<u16> {
    if offset + 2 > data.len() {
        return Err(Error::protocol("Buffer too short for u16"));
    }
    Ok(u16::from_le_bytes([data[offset], data[offset + 1]]))
}

/// Read a little-endian i16 from a byte slice
pub fn read_i16_le(data: &[u8], offset: usize) -> Result<i16> {
    if offset + 2 > data.len() {
        return Err(Error::protocol("Buffer too short for i16"));
    }
    Ok(i16::from_le_bytes([data[offset], data[offset + 1]]))
}

/// Read a little-endian u32 from a byte slice
pub fn read_u32_le(data: &[u8], offset: usize) -> Result<u32> {
    if offset + 4 > data.len() {
        return Err(Error::protocol("Buffer too short for u32"));
    }
    Ok(u32::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ]))
}

/// Read a little-endian i32 from a byte slice
pub fn read_i32_le(data: &[u8], offset: usize) -> Result<i32> {
    if offset + 4 > data.len() {
        return Err(Error::protocol("Buffer too short for i32"));
    }
    Ok(i32::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ]))
}

/// Read a null-terminated or fixed-length UTF-8 string
pub fn read_string(data: &[u8], offset: usize, max_len: usize) -> String {
    let end = (offset + max_len).min(data.len());
    let slice = &data[offset..end];

    // Find null terminator
    let null_pos = slice.iter().position(|&b| b == 0).unwrap_or(slice.len());

    String::from_utf8_lossy(&slice[..null_pos])
        .trim()
        .to_string()
}

/// Read a fixed-size byte array
pub fn read_bytes<const N: usize>(data: &[u8], offset: usize) -> Result<[u8; N]> {
    if offset + N > data.len() {
        return Err(Error::protocol(format!("Buffer too short for {} bytes", N)));
    }
    let mut arr = [0u8; N];
    arr.copy_from_slice(&data[offset..offset + N]);
    Ok(arr)
}

/// Parse a contact from raw bytes (149 bytes)
pub fn parse_contact(data: &[u8]) -> Result<Contact> {
    if data.len() < 145 {
        return Err(Error::protocol(format!(
            "Contact data too short: {} bytes",
            data.len()
        )));
    }

    let public_key: [u8; 32] = read_bytes(data, 0)?;
    let contact_type = data[32];
    let flags = data[33];
    let path_len = data[34] as i8;

    // Path is 64 bytes at offset 35
    let path_end = 35 + 64;
    let out_path = data[35..path_end]
        .iter()
        .take_while(|&&b| b != 0)
        .copied()
        .collect();

    // The Name is 32 bytes at offset 99
    let adv_name = read_string(data, 99, 32);

    // Timestamps and coordinates
    let last_advert = read_u32_le(data, 131)?;
    let adv_lat = read_i32_le(data, 135)?;
    let adv_lon = read_i32_le(data, 139)?;

    // The last modification timestamp is optional (4 bytes at offset 143)
    let last_modification_timestamp = if data.len() >= 149 {
        read_u32_le(data, 143).unwrap_or(0)
    } else {
        0
    };

    Ok(Contact {
        public_key,
        contact_type,
        flags,
        path_len,
        out_path,
        adv_name,
        last_advert,
        adv_lat,
        adv_lon,
        last_modification_timestamp,
    })
}

/// Parse self-info from raw bytes (109+ bytes)
pub fn parse_self_info(data: &[u8]) -> Result<SelfInfo> {
    if data.len() < 52 {
        return Err(Error::protocol(format!(
            "SelfInfo data too short: {} bytes",
            data.len()
        )));
    }

    let adv_type = data[0];
    let tx_power = data[1];
    let max_tx_power = data[2];

    let public_key: [u8; 32] = read_bytes(data, 3)?;

    let adv_lat = read_i32_le(data, 35)?;
    let adv_lon = read_i32_le(data, 39)?;

    let multi_acks = data[43];
    let adv_loc_policy = data[44];

    // Telemetry mode is bit-packed
    let telemetry_byte = data[45];
    let telemetry_mode_base = telemetry_byte & 0x03;
    let telemetry_mode_loc = (telemetry_byte >> 2) & 0x03;
    let telemetry_mode_env = (telemetry_byte >> 4) & 0x03;

    let manual_add_contacts = data[46] != 0;

    let radio_freq = read_u32_le(data, 47)?;
    let radio_bw = read_u32_le(data, 51)?;

    let (sf, cr, name) = if data.len() >= 55 {
        let sf = data[55];
        let cr = data[56];
        let name = if data.len() > 57 {
            read_string(data, 57, data.len() - 57)
        } else {
            String::new()
        };
        (sf, cr, name)
    } else {
        (0, 0, String::new())
    };

    Ok(SelfInfo {
        adv_type,
        tx_power,
        max_tx_power,
        public_key,
        adv_lat,
        adv_lon,
        multi_acks,
        adv_loc_policy,
        telemetry_mode_base,
        telemetry_mode_loc,
        telemetry_mode_env,
        manual_add_contacts,
        radio_freq,
        radio_bw,
        sf,
        cr,
        name,
    })
}

/// Parse status response (52+ bytes)
pub fn parse_status(data: &[u8], sender_prefix: [u8; 6]) -> Result<StatusData> {
    if data.len() < 52 {
        return Err(Error::protocol(format!(
            "Status data too short: {} bytes",
            data.len()
        )));
    }

    let battery = read_u16_le(data, 0)?;
    let tx_queue_len = read_u16_le(data, 2)?;
    let noise_floor = read_i16_le(data, 4)?;
    let last_rssi = read_i16_le(data, 6)?;
    let nb_recv = read_u32_le(data, 8)?;
    let nb_sent = read_u32_le(data, 12)?;
    let airtime = read_u32_le(data, 16)?;
    let uptime = read_u32_le(data, 20)?;
    let flood_sent = read_u32_le(data, 24)?;
    let direct_sent = read_u32_le(data, 28)?;

    let snr_raw = data[32] as i8;
    let snr = snr_raw as f32 / 4.0;

    let dup_count = read_u32_le(data, 36)?;
    let rx_airtime = read_u32_le(data, 40)?;

    Ok(StatusData {
        battery,
        tx_queue_len,
        noise_floor,
        last_rssi,
        nb_recv,
        nb_sent,
        airtime,
        uptime,
        flood_sent,
        direct_sent,
        snr,
        dup_count,
        rx_airtime,
        sender_prefix,
    })
}

/// Parse a received message (contact message v2 format)
pub fn parse_contact_msg(data: &[u8]) -> Result<ReceivedMessage> {
    if data.len() < 12 {
        return Err(Error::protocol("Contact message too short"));
    }

    let sender_prefix: [u8; 6] = read_bytes(data, 0)?;
    let path_len = data[6];
    let txt_type = data[7];
    let sender_timestamp = read_u32_le(data, 8)?;

    let (signature, text_start) = if txt_type == 2 && data.len() >= 16 {
        let sig: [u8; 4] = read_bytes(data, 12)?;
        (Some(sig), 16)
    } else {
        (None, 12)
    };

    let text = if data.len() > text_start {
        String::from_utf8_lossy(&data[text_start..]).to_string()
    } else {
        String::new()
    };

    Ok(ReceivedMessage {
        sender_prefix,
        path_len,
        txt_type,
        sender_timestamp,
        text,
        snr: None,
        signature,
        channel: None,
    })
}

/// Parse a received message v3 format (with SNR)
pub fn parse_contact_msg_v3(data: &[u8]) -> Result<ReceivedMessage> {
    if data.len() < 15 {
        return Err(Error::protocol("Contact message v3 too short"));
    }

    let snr_raw = data[0] as i8;
    let snr = snr_raw as f32 / 4.0;
    // bytes 1-2 are reserved

    let sender_prefix: [u8; 6] = read_bytes(data, 3)?;
    let path_len = data[9];
    let txt_type = data[10];
    let sender_timestamp = read_u32_le(data, 11)?;

    let (signature, text_start) = if txt_type == 2 && data.len() >= 19 {
        let sig: [u8; 4] = read_bytes(data, 15)?;
        (Some(sig), 19)
    } else {
        (None, 15)
    };

    let text = if data.len() > text_start {
        String::from_utf8_lossy(&data[text_start..]).to_string()
    } else {
        String::new()
    };

    Ok(ReceivedMessage {
        sender_prefix,
        path_len,
        txt_type,
        sender_timestamp,
        text,
        snr: Some(snr),
        signature,
        channel: None,
    })
}

/// Parse a channel message
pub fn parse_channel_msg(data: &[u8]) -> Result<ReceivedMessage> {
    if data.len() < 13 {
        return Err(Error::protocol("Channel message too short"));
    }

    let channel = data[0];
    let sender_prefix: [u8; 6] = read_bytes(data, 1)?;
    let path_len = data[7];
    let txt_type = data[8];
    let sender_timestamp = read_u32_le(data, 9)?;

    let text = if data.len() > 13 {
        String::from_utf8_lossy(&data[13..]).to_string()
    } else {
        String::new()
    };

    Ok(ReceivedMessage {
        sender_prefix,
        path_len,
        txt_type,
        sender_timestamp,
        text,
        snr: None,
        signature: None,
        channel: Some(channel),
    })
}

/// Parse ACL entries (7 bytes each)
pub fn parse_acl(data: &[u8]) -> Vec<AclEntry> {
    let mut entries = Vec::new();
    let mut offset = 0;

    while offset + 7 <= data.len() {
        let mut prefix = [0u8; 6];
        prefix.copy_from_slice(&data[offset..offset + 6]);
        let permissions = data[offset + 6];

        entries.push(AclEntry {
            prefix,
            permissions,
        });

        offset += 7;
    }

    entries
}

/// Parse neighbours response
pub fn parse_neighbours(data: &[u8], pubkey_len: usize) -> Result<NeighboursData> {
    if data.len() < 4 {
        return Err(Error::protocol("Neighbours data too short"));
    }

    let total = read_u16_le(data, 0)?;
    let count = read_u16_le(data, 2)?;

    let entry_size = pubkey_len + 5; // pubkey + 4 bytes secs_ago + 1 byte snr
    let mut neighbours = Vec::new();
    let mut offset = 4;

    for _ in 0..count {
        if offset + entry_size > data.len() {
            break;
        }

        let pubkey = data[offset..offset + pubkey_len].to_vec();
        let secs_ago = read_i32_le(data, offset + pubkey_len)?;
        let snr_raw = data[offset + pubkey_len + 4] as i8;
        let snr = snr_raw as f32 / 4.0;

        neighbours.push(Neighbour {
            pubkey,
            secs_ago,
            snr,
        });

        offset += entry_size;
    }

    Ok(NeighboursData { total, neighbours })
}

/// Parse MMA (Min/Max/Avg) entries
pub fn parse_mma(data: &[u8]) -> Vec<MmaEntry> {
    // MMA format varies - this is a basic implementation
    // Each entry is: channel (1) + type (1) + min (4) + max (4) + avg (4) = 14 bytes
    let mut entries = Vec::new();
    let mut offset = 0;

    while offset + 14 <= data.len() {
        let channel = data[offset];
        let entry_type = data[offset + 1];

        // Values are typically floats encoded as fixed-point or raw floats
        let min_raw = read_i32_le(data, offset + 2).unwrap_or(0);
        let max_raw = read_i32_le(data, offset + 6).unwrap_or(0);
        let avg_raw = read_i32_le(data, offset + 10).unwrap_or(0);

        entries.push(MmaEntry {
            channel,
            entry_type,
            min: min_raw as f32,
            max: max_raw as f32,
            avg: avg_raw as f32,
        });

        offset += 14;
    }

    entries
}

/// Encode coordinates as microdegrees
pub fn to_microdegrees(degrees: f64) -> i32 {
    (degrees * 1_000_000.0) as i32
}

/// Decode microdegrees to decimal degrees
pub fn from_microdegrees(micro: i32) -> f64 {
    micro as f64 / 1_000_000.0
}

/// Encode a hex string to bytes
pub fn hex_decode(s: &str) -> Result<Vec<u8>> {
    let s = s.trim_start_matches("0x");
    if !s.len().is_multiple_of(2) {
        return Err(Error::invalid_param("Hex string must have even length"));
    }

    (0..s.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&s[i..i + 2], 16)
                .map_err(|_| Error::invalid_param("Invalid hex character"))
        })
        .collect()
}

/// Encode bytes as a hex string
pub fn hex_encode(data: &[u8]) -> String {
    data.iter().map(|b| format!("{:02x}", b)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_read_u16_le() {
        let data = [0x34, 0x12];
        assert_eq!(read_u16_le(&data, 0).unwrap(), 0x1234);
    }

    #[test]
    fn test_read_u16_le_with_offset() {
        let data = [0x00, 0x00, 0x34, 0x12];
        assert_eq!(read_u16_le(&data, 2).unwrap(), 0x1234);
    }

    #[test]
    fn test_read_u16_le_buffer_too_short() {
        let data = [0x34];
        assert!(read_u16_le(&data, 0).is_err());
    }

    #[test]
    fn test_read_i16_le() {
        // Test positive value
        let data = [0x34, 0x12];
        assert_eq!(read_i16_le(&data, 0).unwrap(), 0x1234);

        // Test negative value (-1)
        let data = [0xFF, 0xFF];
        assert_eq!(read_i16_le(&data, 0).unwrap(), -1);

        // Test negative value (-100)
        let data = (-100i16).to_le_bytes();
        assert_eq!(read_i16_le(&data, 0).unwrap(), -100);
    }

    #[test]
    fn test_read_i16_le_buffer_too_short() {
        let data = [0x34];
        assert!(read_i16_le(&data, 0).is_err());
    }

    #[test]
    fn test_read_u32_le() {
        let data = [0x78, 0x56, 0x34, 0x12];
        assert_eq!(read_u32_le(&data, 0).unwrap(), 0x12345678);
    }

    #[test]
    fn test_read_u32_le_with_offset() {
        let data = [0x00, 0x00, 0x78, 0x56, 0x34, 0x12];
        assert_eq!(read_u32_le(&data, 2).unwrap(), 0x12345678);
    }

    #[test]
    fn test_read_u32_le_buffer_too_short() {
        let data = [0x78, 0x56, 0x34];
        assert!(read_u32_le(&data, 0).is_err());
    }

    #[test]
    fn test_read_i32_le() {
        // Test positive value
        let data = [0x78, 0x56, 0x34, 0x12];
        assert_eq!(read_i32_le(&data, 0).unwrap(), 0x12345678);

        // Test negative value (-1)
        let data = [0xFF, 0xFF, 0xFF, 0xFF];
        assert_eq!(read_i32_le(&data, 0).unwrap(), -1);

        // Test negative value (-1000000 for microdegrees)
        let data = (-1000000i32).to_le_bytes();
        assert_eq!(read_i32_le(&data, 0).unwrap(), -1000000);
    }

    #[test]
    fn test_read_i32_le_buffer_too_short() {
        let data = [0x78, 0x56, 0x34];
        assert!(read_i32_le(&data, 0).is_err());
    }

    #[test]
    fn test_read_string_null_terminated() {
        let data = b"hello\0world";
        assert_eq!(read_string(data, 0, 11), "hello");
    }

    #[test]
    fn test_read_string_fixed_length() {
        let data = b"hello world";
        assert_eq!(read_string(data, 0, 5), "hello");
    }

    #[test]
    fn test_read_string_with_offset() {
        let data = b"XXXhello\0";
        assert_eq!(read_string(data, 3, 6), "hello");
    }

    #[test]
    fn test_read_string_empty() {
        let data = b"\0hello";
        assert_eq!(read_string(data, 0, 6), "");
    }

    #[test]
    fn test_read_string_trims_whitespace() {
        let data = b"  hello  \0";
        assert_eq!(read_string(data, 0, 10), "hello");
    }

    #[test]
    fn test_read_bytes() {
        let data = [0x01, 0x02, 0x03, 0x04, 0x05, 0x06];
        let result: [u8; 4] = read_bytes(&data, 1).unwrap();
        assert_eq!(result, [0x02, 0x03, 0x04, 0x05]);
    }

    #[test]
    fn test_read_bytes_buffer_too_short() {
        let data = [0x01, 0x02];
        let result: Result<[u8; 4]> = read_bytes(&data, 0);
        assert!(result.is_err());
    }

    #[test]
    fn test_hex_encode_decode() {
        let original = vec![0xde, 0xad, 0xbe, 0xef];
        let encoded = hex_encode(&original);
        assert_eq!(encoded, "deadbeef");

        let decoded = hex_decode(&encoded).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn test_hex_decode_with_0x_prefix() {
        let decoded = hex_decode("0xdeadbeef").unwrap();
        assert_eq!(decoded, vec![0xde, 0xad, 0xbe, 0xef]);
    }

    #[test]
    fn test_hex_decode_odd_length() {
        assert!(hex_decode("abc").is_err());
    }

    #[test]
    fn test_hex_decode_invalid_char() {
        assert!(hex_decode("ghij").is_err());
    }

    #[test]
    fn test_hex_encode_empty() {
        assert_eq!(hex_encode(&[]), "");
    }

    #[test]
    fn test_hex_decode_empty() {
        assert_eq!(hex_decode("").unwrap(), vec![]);
    }

    #[test]
    fn test_microdegrees() {
        let lat = 37.7749;
        let micro = to_microdegrees(lat);
        let back = from_microdegrees(micro);
        assert!((lat - back).abs() < 0.000001);
    }

    #[test]
    fn test_microdegrees_negative() {
        let lon = -122.4194;
        let micro = to_microdegrees(lon);
        let back = from_microdegrees(micro);
        assert!((lon - back).abs() < 0.000001);
    }

    #[test]
    fn test_parse_contact() {
        // Create a minimal valid contact buffer (145+ bytes)
        let mut data = vec![0u8; 149];
        // Public key (32 bytes)
        data[0..6].copy_from_slice(&[0x01, 0x02, 0x03, 0x04, 0x05, 0x06]);
        // contact_type
        data[32] = 1;
        // flags
        data[33] = 0x02;
        // path_len
        data[34] = 3;
        // out_path (starts at 35, 64 bytes)
        data[35..38].copy_from_slice(&[0x0A, 0x0B, 0x0C]);
        // adv_name (starts at 99, 32 bytes)
        data[99..104].copy_from_slice(b"Test\0");
        // last_advert (at 131, 4 bytes)
        data[131..135].copy_from_slice(&1000u32.to_le_bytes());
        // adv_lat (at 135, 4 bytes)
        data[135..139].copy_from_slice(&37774900i32.to_le_bytes());
        // adv_lon (at 139, 4 bytes)
        data[139..143].copy_from_slice(&(-122419400i32).to_le_bytes());
        // last_modification_timestamp (at 143, 4 bytes)
        data[143..147].copy_from_slice(&2000u32.to_le_bytes());

        let contact = parse_contact(&data).unwrap();
        assert_eq!(contact.contact_type, 1);
        assert_eq!(contact.flags, 0x02);
        assert_eq!(contact.path_len, 3);
        assert_eq!(contact.out_path, vec![0x0A, 0x0B, 0x0C]);
        assert_eq!(contact.adv_name, "Test");
        assert_eq!(contact.last_advert, 1000);
        assert_eq!(contact.adv_lat, 37774900);
        assert_eq!(contact.adv_lon, -122419400);
        assert_eq!(contact.last_modification_timestamp, 2000);
    }

    #[test]
    fn test_parse_contact_too_short() {
        let data = vec![0u8; 100];
        assert!(parse_contact(&data).is_err());
    }

    #[test]
    fn test_parse_self_info() {
        // Create a minimal valid self_info buffer (52+ bytes)
        let mut data = vec![0u8; 60];
        data[0] = 1; // adv_type
        data[1] = 20; // tx_power
        data[2] = 30; // max_tx_power
                      // public_key at 3
        data[3..6].copy_from_slice(&[0xAA, 0xBB, 0xCC]);
        // adv_lat at 35
        data[35..39].copy_from_slice(&37774900i32.to_le_bytes());
        // adv_lon at 39
        data[39..43].copy_from_slice(&(-122419400i32).to_le_bytes());
        data[43] = 2; // multi_acks
        data[44] = 1; // adv_loc_policy
        data[45] = 0b00_01_10_11; // telemetry modes packed
        data[46] = 1; // manual_add_contacts
                      // radio_freq at 47
        data[47..51].copy_from_slice(&915000000u32.to_le_bytes());
        // radio_bw at 51
        data[51..55].copy_from_slice(&125000u32.to_le_bytes());
        data[55] = 7; // sf
        data[56] = 5; // cr
                      // name at 57
        data[57..60].copy_from_slice(b"Dev");

        let info = parse_self_info(&data).unwrap();
        assert_eq!(info.adv_type, 1);
        assert_eq!(info.tx_power, 20);
        assert_eq!(info.max_tx_power, 30);
        assert_eq!(info.adv_lat, 37774900);
        assert_eq!(info.adv_lon, -122419400);
        assert_eq!(info.multi_acks, 2);
        assert_eq!(info.telemetry_mode_base, 0b11);
        assert_eq!(info.telemetry_mode_loc, 0b10);
        assert_eq!(info.telemetry_mode_env, 0b01);
        assert!(info.manual_add_contacts);
        assert_eq!(info.radio_freq, 915000000);
        assert_eq!(info.sf, 7);
        assert_eq!(info.cr, 5);
    }

    #[test]
    fn test_parse_self_info_too_short() {
        let data = vec![0u8; 40];
        assert!(parse_self_info(&data).is_err());
    }

    #[test]
    fn test_parse_status() {
        let mut data = vec![0u8; 52];
        // battery at 0
        data[0..2].copy_from_slice(&100u16.to_le_bytes());
        // tx_queue_len at 2
        data[2..4].copy_from_slice(&5u16.to_le_bytes());
        // noise_floor at 4
        data[4..6].copy_from_slice(&(-90i16).to_le_bytes());
        // last_rssi at 6
        data[6..8].copy_from_slice(&(-50i16).to_le_bytes());
        // nb_recv at 8
        data[8..12].copy_from_slice(&1000u32.to_le_bytes());
        // nb_sent at 12
        data[12..16].copy_from_slice(&500u32.to_le_bytes());
        // airtime at 16
        data[16..20].copy_from_slice(&3600000u32.to_le_bytes());
        // uptime at 20
        data[20..24].copy_from_slice(&86400u32.to_le_bytes());
        // flood_sent at 24
        data[24..28].copy_from_slice(&100u32.to_le_bytes());
        // direct_sent at 28
        data[28..32].copy_from_slice(&400u32.to_le_bytes());
        // snr at 32 (raw, multiplied by 4)
        data[32] = 40; // SNR = 10.0
                       // dup_count at 36
        data[36..40].copy_from_slice(&10u32.to_le_bytes());
        // rx_airtime at 40
        data[40..44].copy_from_slice(&1800000u32.to_le_bytes());

        let sender = [0x01, 0x02, 0x03, 0x04, 0x05, 0x06];
        let status = parse_status(&data, sender).unwrap();

        assert_eq!(status.battery, 100);
        assert_eq!(status.tx_queue_len, 5);
        assert_eq!(status.noise_floor, -90);
        assert_eq!(status.last_rssi, -50);
        assert_eq!(status.nb_recv, 1000);
        assert_eq!(status.nb_sent, 500);
        assert_eq!(status.uptime, 86400);
        assert_eq!(status.snr, 10.0);
        assert_eq!(status.sender_prefix, sender);
    }

    #[test]
    fn test_parse_status_too_short() {
        let data = vec![0u8; 40];
        assert!(parse_status(&data, [0; 6]).is_err());
    }

    #[test]
    fn test_parse_contact_msg() {
        let mut data = vec![0u8; 20];
        // sender_prefix at 0
        data[0..6].copy_from_slice(&[0x01, 0x02, 0x03, 0x04, 0x05, 0x06]);
        data[6] = 2; // path_len
        data[7] = 1; // txt_type
                     // sender_timestamp at 8
        data[8..12].copy_from_slice(&1234567890u32.to_le_bytes());
        // text at 12
        data[12..20].copy_from_slice(b"Hi there");

        let msg = parse_contact_msg(&data).unwrap();
        assert_eq!(msg.sender_prefix, [0x01, 0x02, 0x03, 0x04, 0x05, 0x06]);
        assert_eq!(msg.path_len, 2);
        assert_eq!(msg.txt_type, 1);
        assert_eq!(msg.sender_timestamp, 1234567890);
        assert_eq!(msg.text, "Hi there");
        assert!(msg.signature.is_none());
    }

    #[test]
    fn test_parse_contact_msg_with_signature() {
        let mut data = vec![0u8; 24];
        data[0..6].copy_from_slice(&[0x01, 0x02, 0x03, 0x04, 0x05, 0x06]);
        data[6] = 2;
        data[7] = 2; // txt_type = 2 means signed
        data[8..12].copy_from_slice(&1234567890u32.to_le_bytes());
        // signature at 12 (4 bytes)
        data[12..16].copy_from_slice(&[0xAA, 0xBB, 0xCC, 0xDD]);
        // text at 16
        data[16..24].copy_from_slice(b"Signed!!");

        let msg = parse_contact_msg(&data).unwrap();
        assert_eq!(msg.txt_type, 2);
        assert_eq!(msg.signature, Some([0xAA, 0xBB, 0xCC, 0xDD]));
        assert_eq!(msg.text, "Signed!!");
    }

    #[test]
    fn test_parse_contact_msg_too_short() {
        let data = vec![0u8; 8];
        assert!(parse_contact_msg(&data).is_err());
    }

    #[test]
    fn test_parse_contact_msg_v3() {
        let mut data = vec![0u8; 23];
        data[0] = 40; // snr_raw = 40, SNR = 10.0
                      // reserved bytes at 1-2
                      // sender_prefix at 3
        data[3..9].copy_from_slice(&[0x01, 0x02, 0x03, 0x04, 0x05, 0x06]);
        data[9] = 3; // path_len
        data[10] = 1; // txt_type
                      // sender_timestamp at 11
        data[11..15].copy_from_slice(&1234567890u32.to_le_bytes());
        // text at 15
        data[15..23].copy_from_slice(b"V3 msg!!");

        let msg = parse_contact_msg_v3(&data).unwrap();
        assert_eq!(msg.snr, Some(10.0));
        assert_eq!(msg.sender_prefix, [0x01, 0x02, 0x03, 0x04, 0x05, 0x06]);
        assert_eq!(msg.path_len, 3);
        assert_eq!(msg.text, "V3 msg!!");
    }

    #[test]
    fn test_parse_contact_msg_v3_too_short() {
        let data = vec![0u8; 10];
        assert!(parse_contact_msg_v3(&data).is_err());
    }

    #[test]
    fn test_parse_channel_msg() {
        let mut data = vec![0u8; 20];
        data[0] = 5; // channel
        data[1..7].copy_from_slice(&[0x01, 0x02, 0x03, 0x04, 0x05, 0x06]);
        data[7] = 1; // path_len
        data[8] = 0; // txt_type
        data[9..13].copy_from_slice(&1234567890u32.to_le_bytes());
        data[13..20].copy_from_slice(b"Channel");

        let msg = parse_channel_msg(&data).unwrap();
        assert_eq!(msg.channel, Some(5));
        assert_eq!(msg.sender_prefix, [0x01, 0x02, 0x03, 0x04, 0x05, 0x06]);
        assert_eq!(msg.text, "Channel");
    }

    #[test]
    fn test_parse_channel_msg_too_short() {
        let data = vec![0u8; 10];
        assert!(parse_channel_msg(&data).is_err());
    }

    #[test]
    fn test_parse_acl() {
        let mut data = vec![0u8; 21]; // 3 entries
                                      // Entry 1
        data[0..6].copy_from_slice(&[0x01, 0x02, 0x03, 0x04, 0x05, 0x06]);
        data[6] = 0x01; // permissions
                        // Entry 2
        data[7..13].copy_from_slice(&[0x11, 0x12, 0x13, 0x14, 0x15, 0x16]);
        data[13] = 0x02;
        // Entry 3
        data[14..20].copy_from_slice(&[0x21, 0x22, 0x23, 0x24, 0x25, 0x26]);
        data[20] = 0x03;

        let entries = parse_acl(&data);
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].prefix, [0x01, 0x02, 0x03, 0x04, 0x05, 0x06]);
        assert_eq!(entries[0].permissions, 0x01);
        assert_eq!(entries[2].permissions, 0x03);
    }

    #[test]
    fn test_parse_acl_empty() {
        let entries = parse_acl(&[]);
        assert!(entries.is_empty());
    }

    #[test]
    fn test_parse_neighbours() {
        let mut data = vec![0u8; 15];
        // total at 0
        data[0..2].copy_from_slice(&10u16.to_le_bytes());
        // count at 2
        data[2..4].copy_from_slice(&1u16.to_le_bytes());
        // Entry: pubkey (6) + secs_ago (4) + snr (1) = 11 bytes
        data[4..10].copy_from_slice(&[0x01, 0x02, 0x03, 0x04, 0x05, 0x06]);
        data[10..14].copy_from_slice(&300i32.to_le_bytes());
        data[14] = 40; // snr_raw = 40, SNR = 10.0

        let result = parse_neighbours(&data, 6).unwrap();
        assert_eq!(result.total, 10);
        assert_eq!(result.neighbours.len(), 1);
        assert_eq!(
            result.neighbours[0].pubkey,
            vec![0x01, 0x02, 0x03, 0x04, 0x05, 0x06]
        );
        assert_eq!(result.neighbours[0].secs_ago, 300);
        assert_eq!(result.neighbours[0].snr, 10.0);
    }

    #[test]
    fn test_parse_neighbours_too_short() {
        let data = vec![0u8; 2];
        assert!(parse_neighbours(&data, 6).is_err());
    }

    #[test]
    fn test_parse_mma() {
        let mut data = vec![0u8; 28]; // 2 entries
                                      // Entry 1
        data[0] = 1; // channel
        data[1] = 2; // entry_type
        data[2..6].copy_from_slice(&100i32.to_le_bytes()); // min
        data[6..10].copy_from_slice(&200i32.to_le_bytes()); // max
        data[10..14].copy_from_slice(&150i32.to_le_bytes()); // avg
                                                             // Entry 2
        data[14] = 2;
        data[15] = 3;
        data[16..20].copy_from_slice(&50i32.to_le_bytes());
        data[20..24].copy_from_slice(&100i32.to_le_bytes());
        data[24..28].copy_from_slice(&75i32.to_le_bytes());

        let entries = parse_mma(&data);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].channel, 1);
        assert_eq!(entries[0].entry_type, 2);
        assert_eq!(entries[0].min, 100.0);
        assert_eq!(entries[0].max, 200.0);
        assert_eq!(entries[0].avg, 150.0);
    }

    #[test]
    fn test_parse_mma_empty() {
        let entries = parse_mma(&[]);
        assert!(entries.is_empty());
    }
}
