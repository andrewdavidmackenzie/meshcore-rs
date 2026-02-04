//! Binary parsing utilities for MeshCore protocol

use crate::events::{
    AclEntry, Contact, MmaEntry, Neighbour, NeighboursData, ReceivedMessage, SelfInfo, StatusData,
};
use crate::error::Error;
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
        return Err(Error::protocol(format!(
            "Buffer too short for {} bytes",
            N
        )));
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

    // Name is 32 bytes at offset 99
    let adv_name = read_string(data, 99, 32);

    // Timestamps and coordinates
    let last_advert = read_u32_le(data, 131)?;
    let adv_lat = read_i32_le(data, 135)?;
    let adv_lon = read_i32_le(data, 139)?;

    // Lastmod is optional (4 bytes at offset 143)
    let lastmod = if data.len() >= 149 {
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
        lastmod,
    })
}

/// Parse self info from raw bytes (109+ bytes)
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
    if s.len() % 2 != 0 {
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
    fn test_read_u32_le() {
        let data = [0x78, 0x56, 0x34, 0x12];
        assert_eq!(read_u32_le(&data, 0).unwrap(), 0x12345678);
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
    fn test_microdegrees() {
        let lat = 37.7749;
        let micro = to_microdegrees(lat);
        let back = from_microdegrees(micro);
        assert!((lat - back).abs() < 0.000001);
    }
}
