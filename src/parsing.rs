//! Binary parsing utilities for MeshCore protocol

use std::collections::HashMap;

use crate::error::Error;
use crate::events::{
    AclEntry, AdvertResponseData, AdvertisementData, BatteryInfo, ChannelInfoData, ChannelMessage,
    Contact, ContactMessage, CoreStatsData, DeviceInfoData, DiscoverEntry, MeshPacketHeader,
    MmaEntry, MsgSentInfo, Neighbour, NeighboursData, PacketStatsData, PathDiscoveryResponseData,
    PathUpdateData, RadioStatsData, RawAdvertisement, SelfInfo, StatsCategory, StatsData,
    StatusData, TraceHop, TraceInfo,
};
use crate::packets::{PayloadType, RouteType};
use crate::{Result, CHANNEL_NAME_LEN, CHANNEL_SECRET_LEN};

/// Read a little-endian u16 from a byte slice
pub fn read_u16_le(data: &[u8], offset: usize) -> Result<u16> {
    if offset
        .checked_add(2)
        .ok_or_else(|| Error::protocol("Offset overflow"))?
        > data.len()
    {
        return Err(Error::protocol("Buffer too short for u16"));
    }
    Ok(u16::from_le_bytes([data[offset], data[offset + 1]]))
}

/// Read a little-endian i16 from a byte slice
pub fn read_i16_le(data: &[u8], offset: usize) -> Result<i16> {
    if offset
        .checked_add(2)
        .ok_or_else(|| Error::protocol("Offset overflow"))?
        > data.len()
    {
        return Err(Error::protocol("Buffer too short for i16"));
    }
    Ok(i16::from_le_bytes([data[offset], data[offset + 1]]))
}

/// Read a little-endian u32 from a byte slice
pub fn read_u32_le(data: &[u8], offset: usize) -> Result<u32> {
    if offset
        .checked_add(4) // we can index up to 4 bytes into data
        .ok_or_else(|| Error::protocol("Offset overflow"))?
        > data.len()
    {
        return Err(Error::protocol("Buffer too short for u32"));
    }
    Ok(u32::from_le_bytes([
        data[offset],     // jonesy:allow(bounds)
        data[offset + 1], // jonesy:allow(overflow)
        data[offset + 2], // jonesy:allow(bounds)
        data[offset + 3], // jonesy:allow(bounds, overflow)
    ]))
}

/// Read a little-endian i32 from a byte slice
pub fn read_i32_le(data: &[u8], offset: usize) -> Result<i32> {
    if offset
        .checked_add(4) // we can index up to 4 bytes into data
        .ok_or_else(|| Error::protocol("Offset overflow"))?
        > data.len()
    {
        return Err(Error::protocol("Buffer too short for i32"));
    }
    Ok(i32::from_le_bytes([
        data[offset],     // jonesy:allow(bounds)
        data[offset + 1], // jonesy:allow(bounds, overflow)
        data[offset + 2], // jonesy:allow(bounds, overflow)
        data[offset + 3], // jonesy:allow(bounds, overflow)
    ]))
}

/// Read a null-terminated or fixed-length UTF-8 string.
///
/// A device's name/label field is not always null-terminated or
/// zero-padded within its fixed-size slot on the wire (observed on real
/// advertisement pushes) — bytes past the logical end of the string can
/// be leftover garbage. Reading `max_len` raw bytes in that case
/// previously surfaced as a visibly corrupted string: stray control
/// characters and U+FFFD replacement characters from invalid UTF-8
/// sequences. Stop at the first NUL byte, control character, or
/// invalid-UTF-8 byte, whichever comes first — a legitimate name never
/// contains any of those.
pub fn read_string(data: &[u8], offset: usize, max_len: usize) -> String {
    // limit indexing to the size of the data buffer
    // jonesy:allow(overflow)
    let end = (offset + max_len).min(data.len());
    let slice = &data[offset..end];

    // Find null terminator
    let null_pos = slice.iter().position(|&b| b == 0).unwrap_or(slice.len());

    String::from_utf8_lossy(&slice[..null_pos])
        .chars()
        .take_while(|&c| !c.is_control() && c != '\u{FFFD}')
        .collect::<String>()
        .trim()
        .to_string()
}

/// Read a fixed-size byte array
pub fn read_bytes<const N: usize>(data: &[u8], offset: usize) -> Result<[u8; N]> {
    // check that we can index N bytes into data
    if offset
        .checked_add(N)
        .ok_or_else(|| Error::protocol("Offset overflow"))?
        > data.len()
    {
        return Err(Error::protocol(format!("Buffer too short for {} bytes", N)));
    }
    let mut arr = [0u8; N];
    // jonesy:allow(overflow)
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
    // We can index safely into data up to 52 bytes
    if data.len() < 52 {
        return Err(Error::protocol(format!(
            "SelfInfo data too short: {} bytes",
            data.len()
        )));
    }

    let adv_type = data[0]; // jonesy:allow(bounds)
    let tx_power = data[1];
    let max_tx_power = data[2]; // jonesy:allow(bounds)

    let public_key: [u8; 32] = read_bytes(data, 3)?;

    let adv_lat = read_i32_le(data, 35)?;
    let adv_lon = read_i32_le(data, 39)?;

    let multi_acks = data[43]; // jonesy:allow(bounds)
    let adv_loc_policy = data[44]; // jonesy:allow(bounds)

    // Telemetry mode is bit-packed
    let telemetry_byte = data[45]; // jonesy:allow(bounds)
    let telemetry_mode_base = telemetry_byte & 0x03;
    let telemetry_mode_loc = (telemetry_byte >> 2) & 0x03;
    let telemetry_mode_env = (telemetry_byte >> 4) & 0x03;

    let manual_add_contacts = data[46] != 0; // jonesy:allow(bounds)

    let radio_freq = read_u32_le(data, 47)?;
    let radio_bw = read_u32_le(data, 51)?;

    // check if we can go beyond 55 bytes
    let (sf, cr, name) = if data.len() >= 55 {
        let sf = data[55]; // jonesy:allow(bounds)
        let cr = data[56]; // jonesy:allow(bounds)

        // check data is longer and read string from byte 57 to the end
        let name = if data.len() > 57 {
            // jonesy:allow(overflow) - if larger than 57, can't underflow
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

/// Parse device info response
///
/// Format (after response code byte):
/// - Byte 0: Firmware version code
/// - Byte 1: Max contacts / 2 (v3+)
/// - Byte 2: Max channels (v3+)
/// - Bytes 3-6: BLE PIN (u32 LE, v3+)
/// - Bytes 7-18: Firmware build date (12 bytes, null-terminated, v3+)
/// - Bytes 19-58: Model/manufacturer (40 bytes, null-terminated, v3+)
/// - Bytes 59-78: Version string (20 bytes, null-terminated, v3+)
/// - Byte 79: Repeat setting (v9+)
pub fn parse_device_info(data: &[u8]) -> Result<DeviceInfoData> {
    // Minimum: 1 byte for fw_version_code
    if data.is_empty() {
        return Err(Error::protocol("DeviceInfo payload too short"));
    }

    let fw_version_code = data[0];

    // Version 3+ fields require fw_version_code >= 3 and sufficient data
    if fw_version_code < 3 || data.len() < 2 {
        return Ok(DeviceInfoData {
            fw_version_code,
            max_contacts: None,
            max_channels: None,
            ble_pin: None,
            fw_build: None,
            model: None,
            version: None,
            repeat: None,
        });
    }

    // Parse v3+ fields
    let max_contacts = if data.len() > 1 {
        Some(data[1].saturating_mul(2))
    } else {
        None
    };

    let max_channels = if data.len() > 2 { Some(data[2]) } else { None };

    let ble_pin = if data.len() >= 7 {
        read_u32_le(data, 3).ok()
    } else {
        None
    };

    let fw_build = if data.len() >= 19 {
        Some(read_string(data, 7, 12))
    } else {
        None
    };

    let model = if data.len() >= 59 {
        Some(read_string(data, 19, 40))
    } else {
        None
    };

    let version = if data.len() >= 79 {
        Some(read_string(data, 59, 20))
    } else {
        None
    };

    // v9+ repeat field
    let repeat = if data.len() >= 80 {
        Some(data[79] != 0)
    } else {
        None
    };

    Ok(DeviceInfoData {
        fw_version_code,
        max_contacts,
        max_channels,
        ble_pin,
        fw_build,
        model,
        version,
        repeat,
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

    let battery_mv = read_u16_le(data, 0)?;
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
        battery_mv,
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

/// Parse a contact message (v2 format)
pub fn parse_contact_msg(data: &[u8]) -> Result<ContactMessage> {
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

    Ok(ContactMessage {
        sender_prefix,
        path_len,
        txt_type,
        sender_timestamp,
        text,
        snr: None,
        signature,
    })
}

/// Parse a contact message v3 format (with SNR)
pub fn parse_contact_msg_v3(data: &[u8]) -> Result<ContactMessage> {
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

    Ok(ContactMessage {
        sender_prefix,
        path_len,
        txt_type,
        sender_timestamp,
        text,
        snr: Some(snr),
        signature,
    })
}

/// Parse a channel message (v2 format)
pub fn parse_channel_msg(data: &[u8]) -> Result<ChannelMessage> {
    if data.len() < 8 {
        return Err(Error::protocol("Channel message too short"));
    }

    let channel_idx = data[0];
    let path_len = data[1];
    let txt_type = data[2];
    let sender_timestamp = read_u32_le(data, 3)?;

    let text = if data.len() > 7 {
        String::from_utf8_lossy(&data[7..]).to_string()
    } else {
        String::new()
    };

    Ok(ChannelMessage {
        channel_idx,
        path_len,
        txt_type,
        sender_timestamp,
        text,
        snr: None,
    })
}

/// Parse a channel message v3 format (with SNR)
pub fn parse_channel_msg_v3(data: &[u8]) -> Result<ChannelMessage> {
    if data.len() < 11 {
        return Err(Error::protocol("Channel message v3 too short"));
    }

    let snr_raw = data[0] as i8;
    let snr = snr_raw as f32 / 4.0;
    // bytes 1-2 are reserved

    let channel_idx = data[3];
    let path_len = data[4];
    let txt_type = data[5];
    let sender_timestamp = read_u32_le(data, 6)?;

    let text = if data.len() > 10 {
        String::from_utf8_lossy(&data[10..]).to_string()
    } else {
        String::new()
    };

    Ok(ChannelMessage {
        channel_idx,
        path_len,
        txt_type,
        sender_timestamp,
        text,
        snr: Some(snr),
    })
}

/// Parse ACL entries (7 bytes each)
pub fn parse_acl(data: &[u8]) -> Vec<AclEntry> {
    let mut entries = Vec::new();
    let mut offset = 0;

    // avoid walking off the end of the data buffer
    // jonesy:allow(overflow)
    while offset + 7 <= data.len() {
        let mut prefix = [0u8; 6];
        let end = offset + 6; // jonesy:allow(overflow)
                              // jonesy:allow(bounds, overflow) - we checked we have at least 6 above
        prefix.copy_from_slice(&data[offset..end]);
        let permissions = data[end]; // jonesy:allow(bounds)

        entries.push(AclEntry {
            prefix,
            permissions,
        });

        offset += 7; // jonesy:allow(overflow)
    }

    entries
}

/// Parse neighbours response
pub fn parse_neighbours(data: &[u8], pubkey_len: usize) -> Result<NeighboursData> {
    // Validate we can index up to 4 bytes in
    if data.len() < 4 {
        return Err(Error::protocol("Neighbours data too short"));
    }

    let total = read_u16_le(data, 0)?;
    let count = read_u16_le(data, 2)?;

    // jonesy:allow(overflow) - pubkey_len is usually 128
    let entry_size = pubkey_len + 5; // pubkey + 4 bytes secs_ago + 1 byte snr
    let mut neighbours = Vec::new();
    let mut offset = 4;

    // walk through the rest of the data buffer in chunks of entry_size size
    for _ in 0..count {
        // make sure we don't walk off the end of the data buffer
        // jonesy:allow(overflow)
        if offset + entry_size > data.len() {
            // jonesy:allow(overflow)
            break;
        }

        // pubkey_len is less than `entry_size` which has already been checked
        // jonesy:allow(overflow)
        let pubkey = data[offset..offset + pubkey_len].to_vec();
        // jonesy:allow(overflow)
        let secs_ago = read_i32_le(data, offset + pubkey_len)?;
        // included in the `entry_size` check above so can be indexed
        // jonesy:allow(overflow, bounds)
        let snr_raw = data[offset + pubkey_len + 4] as i8;
        let snr = snr_raw as f32 / 4.0;

        neighbours.push(Neighbour {
            pubkey,
            secs_ago,
            snr,
        });
        // jonesy:allow(overflow)
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

    // advance 14 bytes at a time - being careful to not overrun data size
    // jonesy:allow(overflow)
    while offset + 14 <= data.len() {
        let channel = data[offset]; // jonesy:allow(bounds)
        let entry_type = data[offset + 1]; // jonesy:allow(bounds, overflow)

        // Values are typically floats encoded as fixed-point or raw floats
        let min_raw = read_i32_le(data, offset + 2).unwrap_or(0); // jonesy:allow(overflow)
        let max_raw = read_i32_le(data, offset + 6).unwrap_or(0); // jonesy:allow(overflow)
        let avg_raw = read_i32_le(data, offset + 10).unwrap_or(0); // jonesy:allow(overflow)

        entries.push(MmaEntry {
            channel,
            entry_type,
            min: min_raw as f32,
            max: max_raw as f32,
            avg: avg_raw as f32,
        });

        offset += 14; // jonesy:allow(overflow)
    }

    entries
}

/// Bit position of the payload type field within a packet header byte (see
/// `parse_mesh_packet_header`); route type occupies bits 0-1 below it.
const PAYLOAD_TYPE_SHIFT: u8 = 2;
/// Bit position of the payload format version field within a packet header
/// byte.
const PAYLOAD_VERSION_SHIFT: u8 = 6;

/// Length of the packet header byte itself.
const HEADER_BYTE_LEN: usize = 1;
/// Length of the optional transport code, present only for
/// `TransportFlood`/`TransportDirect` routes.
const TRANSPORT_CODE_LEN: usize = 4;
/// Length of the path-length/hash-size byte that follows the header (and
/// the transport code, if present).
const PATH_BYTE_LEN: usize = 1;

/// Parse the MeshCore over-the-air packet header (route type, payload type
/// and path) from the start of a buffer, as embedded in RAW_DATA/LOG_DATA
/// captures.
///
/// Returns the decoded header together with the remaining, unparsed inner
/// packet payload, or an error if `data` is too short to contain a header
/// and a path byte.
pub fn parse_mesh_packet_header(data: &[u8]) -> Result<(MeshPacketHeader, &[u8])> {
    // Header byte layout: bits 0-1 = route type, bits 2-5 = payload type,
    // bits 6-7 = payload format version.
    let header_byte = *data
        .first()
        .ok_or_else(|| Error::protocol("MeshPacketHeader payload too short"))?;
    let route_type = RouteType::from(header_byte);
    let payload_type = PayloadType::from(header_byte >> PAYLOAD_TYPE_SHIFT);
    let payload_version = (header_byte & 0xc0) >> PAYLOAD_VERSION_SHIFT;

    let mut offset = HEADER_BYTE_LEN;

    let transport_code = if matches!(
        route_type,
        RouteType::TransportFlood | RouteType::TransportDirect
    ) {
        let code: [u8; 4] = read_bytes(data, offset)?;
        offset = offset
            .checked_add(TRANSPORT_CODE_LEN)
            .ok_or_else(|| Error::protocol("MeshPacketHeader offset overflow"))?;
        Some(code)
    } else {
        None
    };

    let path_byte = *data
        .get(offset)
        .ok_or_else(|| Error::protocol("MeshPacketHeader missing path byte"))?;
    offset = offset
        .checked_add(PATH_BYTE_LEN)
        .ok_or_else(|| Error::protocol("MeshPacketHeader offset overflow"))?;

    // path_hash_size is bounded to 1-4 (2-bit field + 1) and path_len to
    // 0-63 (6-bit field), so their product is bounded to 252, far under
    // usize::MAX -- neither line below can overflow.
    let path_hash_size = ((path_byte & 0xC0) >> 6) + 1; // jonesy:allow(overflow)
    let path_len = path_byte & 0x3F;
    let path_bytes_len = path_len as usize * path_hash_size as usize; // jonesy:allow(overflow)

    let path_end = offset
        .checked_add(path_bytes_len)
        .ok_or_else(|| Error::protocol("MeshPacketHeader path length overflow"))?;
    if path_end > data.len() {
        return Err(Error::protocol("MeshPacketHeader path truncated"));
    }
    let path = data[offset..path_end].to_vec(); // jonesy:allow(bounds) -- checked above
    offset = path_end;

    let header = MeshPacketHeader {
        route_type,
        payload_type,
        payload_version,
        transport_code,
        path_len,
        path_hash_size,
        path,
    };

    Ok((header, &data[offset..]))
}

const PUBLIC_KEY_LEN: usize = 32;
const TIMESTAMP_LEN: usize = 4;
const SIGNATURE_LEN: usize = 64;
const FLAGS_LEN: usize = 1;

const TIMESTAMP_OFFSET: usize = PUBLIC_KEY_LEN;
const SIGNATURE_OFFSET: usize = TIMESTAMP_OFFSET + TIMESTAMP_LEN;
const FLAGS_OFFSET: usize = SIGNATURE_OFFSET + SIGNATURE_LEN;
/// Minimum length of a raw ADVERT payload (public key + timestamp +
/// signature + flags), before any of the optional trailing fields.
const MIN_LEN: usize = FLAGS_OFFSET + FLAGS_LEN;

// Flag bits of the byte at FLAGS_OFFSET.
const FLAG_ADV_TYPE_MASK: u8 = 0x0F; // advertiser type (see Contact::contact_type)
const FLAG_HAS_LOCATION: u8 = 0x10; // 8 bytes: lat (i32 LE) + lon (i32 LE)
const FLAG_HAS_FEATURE1: u8 = 0x20; // 2 bytes, not currently decoded
const FLAG_HAS_FEATURE2: u8 = 0x40; // 2 bytes, not currently decoded
const FLAG_HAS_NAME: u8 = 0x80; // remaining bytes, UTF-8 name

const LOCATION_LEN: usize = 8;
const FEATURE_BLOCK_LEN: usize = 2;

/// Parse a raw ADVERT payload (public key, timestamp, signature, flags and
/// optional location/name), as carried by a [`PayloadType::Advert`] packet.
pub fn parse_raw_advertisement(data: &[u8]) -> Result<RawAdvertisement> {
    if data.len() < MIN_LEN {
        return Err(Error::protocol("RawAdvertisement payload too short"));
    }

    let public_key: [u8; 32] = read_bytes(data, 0)?;
    let timestamp = read_u32_le(data, TIMESTAMP_OFFSET)?;
    let signature: [u8; 64] = read_bytes(data, SIGNATURE_OFFSET)?;
    let flags = *data
        .get(FLAGS_OFFSET)
        .ok_or_else(|| Error::protocol("RawAdvertisement missing flags"))?;
    let adv_type = flags & FLAG_ADV_TYPE_MASK;

    let mut offset = MIN_LEN;

    let (lat, lon) = if flags & FLAG_HAS_LOCATION != 0 {
        // A declared location that does not fit means the capture is
        // truncated; every later offset would be wrong.
        let lon_offset = offset
            .checked_add(4)
            .ok_or_else(|| Error::protocol("RawAdvertisement offset overflow"))?;
        let lat = read_i32_le(data, offset)?;
        let lon = read_i32_le(data, lon_offset)?;
        offset = offset
            .checked_add(LOCATION_LEN)
            .ok_or_else(|| Error::protocol("RawAdvertisement offset overflow"))?;
        (Some(lat), Some(lon))
    } else {
        (None, None)
    };

    if flags & FLAG_HAS_FEATURE1 != 0 {
        offset = offset
            .checked_add(FEATURE_BLOCK_LEN)
            .ok_or_else(|| Error::protocol("RawAdvertisement offset overflow"))?;
    }
    if flags & FLAG_HAS_FEATURE2 != 0 {
        offset = offset
            .checked_add(FEATURE_BLOCK_LEN)
            .ok_or_else(|| Error::protocol("RawAdvertisement offset overflow"))?;
    }

    let name = if flags & FLAG_HAS_NAME != 0 && data.len() > offset {
        // jonesy:allow(overflow) -- guarded by `data.len() > offset` above
        Some(read_string(data, offset, data.len() - offset))
    } else {
        None
    };

    Ok(RawAdvertisement {
        public_key,
        timestamp,
        signature,
        adv_type,
        lat,
        lon,
        name,
    })
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
            u8::from_str_radix(&s[i..i + 2], 16) // jonesy:allow(overflow)
                .map_err(|_| Error::invalid_param("Invalid hex character"))
        })
        .collect()
}

/// Encode bytes as a hex string
pub fn hex_encode(data: &[u8]) -> String {
    data.iter().map(|b| format!("{:02x}", b)).collect()
}

// --- AdvertResponse wire-format layout ---
//
// Byte offsets and sizes for the PacketType::AdvertResponse payload,
// returned by the device in reply to an advert request.

/// Length of the request tag echoed back in the response.
const ADVERT_RESP_TAG_LEN: usize = 4;
/// Length of the 32-byte public key of the advertiser.
const ADVERT_RESP_PUBKEY_LEN: usize = 32;
/// Length of the advertisement type field.
const ADVERT_RESP_ADV_TYPE_LEN: usize = 1;
/// Fixed width of the node name field (padded / null-terminated).
const ADVERT_RESP_NODE_NAME_LEN: usize = 32;
/// Length of the timestamp field (u32 LE).
const ADVERT_RESP_TIMESTAMP_LEN: usize = 4;
/// Length of the flags field.
const ADVERT_RESP_FLAGS_LEN: usize = 1;

const ADVERT_RESP_TAG_OFFSET: usize = 0;
const ADVERT_RESP_PUBKEY_OFFSET: usize = ADVERT_RESP_TAG_OFFSET + ADVERT_RESP_TAG_LEN;
const ADVERT_RESP_ADV_TYPE_OFFSET: usize = ADVERT_RESP_PUBKEY_OFFSET + ADVERT_RESP_PUBKEY_LEN;
const ADVERT_RESP_NODE_NAME_OFFSET: usize = ADVERT_RESP_ADV_TYPE_OFFSET + ADVERT_RESP_ADV_TYPE_LEN;
const ADVERT_RESP_TIMESTAMP_OFFSET: usize =
    ADVERT_RESP_NODE_NAME_OFFSET + ADVERT_RESP_NODE_NAME_LEN;
const ADVERT_RESP_FLAGS_OFFSET: usize = ADVERT_RESP_TIMESTAMP_OFFSET + ADVERT_RESP_TIMESTAMP_LEN;

/// Minimum payload length: tag + pubkey + adv_type + name + timestamp + flags.
const ADVERT_RESP_MIN_LEN: usize = ADVERT_RESP_FLAGS_OFFSET + ADVERT_RESP_FLAGS_LEN;

/// Offset of the optional latitude field (i32 LE), present when the
/// payload extends beyond the flags byte.
const ADVERT_RESP_LAT_OFFSET: usize = ADVERT_RESP_MIN_LEN;
/// Length of one coordinate field (i32 LE).
const ADVERT_RESP_COORD_LEN: usize = 4;
/// Offset of the optional longitude field (i32 LE).
const ADVERT_RESP_LON_OFFSET: usize = ADVERT_RESP_LAT_OFFSET + ADVERT_RESP_COORD_LEN;
/// Minimum payload length to include both lat and lon.
const ADVERT_RESP_LATLON_MIN_LEN: usize = ADVERT_RESP_LON_OFFSET + ADVERT_RESP_COORD_LEN;
/// Offset of the optional node description field, present when the
/// payload extends beyond lat/lon.
const ADVERT_RESP_NODE_DESC_OFFSET: usize = ADVERT_RESP_LATLON_MIN_LEN;
/// Fixed width of the optional node description field.
const ADVERT_RESP_NODE_DESC_LEN: usize = 32;

/// Parse an [`AdvertResponseData`] from a raw `PacketType::AdvertResponse`
/// payload.
///
/// Returns an error if the payload is too short to contain the mandatory
/// fields (tag + pubkey + adv_type + node_name + timestamp + flags = 74
/// bytes).
pub fn parse_advert_response(payload: &[u8]) -> Result<AdvertResponseData> {
    if payload.len() < ADVERT_RESP_MIN_LEN {
        return Err(Error::protocol("AdvertResponse payload too short"));
    }

    let tag: [u8; 4] = read_bytes(payload, ADVERT_RESP_TAG_OFFSET)?;
    let pubkey: [u8; 32] = read_bytes(payload, ADVERT_RESP_PUBKEY_OFFSET)?;
    let adv_type = payload[ADVERT_RESP_ADV_TYPE_OFFSET]; // jonesy:allow(bounds) -- checked >= ADVERT_RESP_MIN_LEN above
    let node_name = read_string(
        payload,
        ADVERT_RESP_NODE_NAME_OFFSET,
        ADVERT_RESP_NODE_NAME_LEN,
    );
    let timestamp = read_u32_le(payload, ADVERT_RESP_TIMESTAMP_OFFSET).unwrap_or(0);
    let flags = payload.get(ADVERT_RESP_FLAGS_OFFSET).copied().unwrap_or(0);

    let (lat, lon, node_desc) = if payload.len() >= ADVERT_RESP_LATLON_MIN_LEN {
        let lat = Some(read_i32_le(payload, ADVERT_RESP_LAT_OFFSET).unwrap_or(0));
        let lon = Some(read_i32_le(payload, ADVERT_RESP_LON_OFFSET).unwrap_or(0));
        let desc = if payload.len() > ADVERT_RESP_NODE_DESC_OFFSET {
            Some(read_string(
                payload,
                ADVERT_RESP_NODE_DESC_OFFSET,
                ADVERT_RESP_NODE_DESC_LEN,
            ))
        } else {
            None
        };
        (lat, lon, desc)
    } else {
        (None, None, None)
    };

    Ok(AdvertResponseData {
        tag,
        pubkey,
        adv_type,
        node_name,
        timestamp,
        flags,
        lat,
        lon,
        node_desc,
    })
}

// --- Battery payload layout ---

/// Minimum length for a Battery payload (just the voltage field).
const BATTERY_MIN_LEN: usize = 2;
/// Minimum length to include optional storage fields (used_kb + total_kb).
const BATTERY_STORAGE_MIN_LEN: usize = 10;
const BATTERY_USED_KB_OFFSET: usize = 2;
const BATTERY_TOTAL_KB_OFFSET: usize = 6;

/// Parse a [`BatteryInfo`] from a `PacketType::Battery` payload.
///
/// Returns an error if the payload is too short to contain the voltage field.
pub fn parse_battery(payload: &[u8]) -> Result<BatteryInfo> {
    if payload.len() < BATTERY_MIN_LEN {
        return Err(Error::protocol("Battery payload too short"));
    }

    let battery_mv = read_u16_le(payload, 0)?;
    let (used_kb, total_kb) = if payload.len() >= BATTERY_STORAGE_MIN_LEN {
        (
            Some(read_u32_le(payload, BATTERY_USED_KB_OFFSET).unwrap_or(0)),
            Some(read_u32_le(payload, BATTERY_TOTAL_KB_OFFSET).unwrap_or(0)),
        )
    } else {
        (None, None)
    };

    Ok(BatteryInfo {
        battery_mv,
        used_kb,
        total_kb,
    })
}

// --- MsgSent payload layout ---

/// Minimum length for a MsgSent payload.
const MSG_SENT_MIN_LEN: usize = 9;
const MSG_SENT_ACK_OFFSET: usize = 1;
const MSG_SENT_TIMEOUT_OFFSET: usize = 5;

/// Parse a [`MsgSentInfo`] from a `PacketType::MsgSent` payload.
///
/// Returns an error if the payload is too short.
pub fn parse_msg_sent(payload: &[u8]) -> Result<MsgSentInfo> {
    if payload.len() < MSG_SENT_MIN_LEN {
        return Err(Error::protocol("MsgSent payload too short"));
    }

    let message_type = payload[0]; // jonesy:allow(bounds) -- checked >= MSG_SENT_MIN_LEN above
    let expected_ack: [u8; 4] = read_bytes(payload, MSG_SENT_ACK_OFFSET)?;
    let suggested_timeout = read_u32_le(payload, MSG_SENT_TIMEOUT_OFFSET).unwrap_or(5000);

    Ok(MsgSentInfo {
        message_type,
        expected_ack,
        suggested_timeout,
    })
}

// --- ChannelInfo payload layout ---

/// Parse a [`ChannelInfoData`] from a `PacketType::ChannelInfo` payload.
///
/// The firmware always sends `CHANNEL_INFO_LEN` bytes:
/// 1 (idx) + CHANNEL_NAME_LEN (name) + CHANNEL_SECRET_LEN (secret).
///
/// Returns an error if the payload is too short.
pub fn parse_channel_info(payload: &[u8]) -> Result<ChannelInfoData> {
    let min_len = 1 + CHANNEL_NAME_LEN + CHANNEL_SECRET_LEN;
    if payload.len() < min_len {
        return Err(Error::protocol("ChannelInfo payload too short"));
    }

    let channel_idx = payload[0]; // jonesy:allow(bounds) -- checked >= min_len above
    let name = read_string(payload, 1, CHANNEL_NAME_LEN);
    let secret: [u8; CHANNEL_SECRET_LEN] =
        read_bytes(payload, 1 + CHANNEL_NAME_LEN).unwrap_or([0; CHANNEL_SECRET_LEN]);

    Ok(ChannelInfoData {
        channel_idx,
        name,
        secret,
    })
}

// --- Stats payload layout ---

/// Parse a [`StatsData`] (with its [`StatsCategory`]) from a
/// `PacketType::Stats` payload.
///
/// Returns an error if the payload is empty.
pub fn parse_stats(payload: &[u8]) -> Result<StatsData> {
    if payload.is_empty() {
        return Err(Error::protocol("Stats payload too short"));
    }

    let category = match payload[0] {
        // jonesy:allow(bounds) -- checked !is_empty() above
        0 => StatsCategory::Core,
        1 => StatsCategory::Radio,
        2 => StatsCategory::Packets,
        _ => return Err(Error::protocol("Unknown stats category")),
    };

    Ok(StatsData {
        category,
        raw: payload[1..].to_vec(),
    })
}

/// Length of a [`StatsCategory::Core`] payload (`raw`, i.e. after the
/// stats-type byte): `battery_mv:u16, uptime_secs:u32, errors:u16,
/// queue_len:u8`.
const CORE_STATS_LEN: usize = 9;

/// Parses a [`StatsCategory::Core`] payload's `raw` bytes into
/// [`CoreStatsData`].
pub fn parse_core_stats(data: &[u8]) -> Result<CoreStatsData> {
    if data.len() < CORE_STATS_LEN {
        return Err(Error::protocol("Core stats payload too short"));
    }
    Ok(CoreStatsData {
        battery_mv: read_u16_le(data, 0)?,
        uptime_secs: read_u32_le(data, 2)?,
        errors: read_u16_le(data, 6)?,
        queue_len: *data
            .get(8)
            .ok_or_else(|| Error::protocol("Core stats payload too short"))?,
    })
}

/// Length of a [`StatsCategory::Radio`] payload (`raw`): `noise_floor:i16,
/// last_rssi:i8, last_snr_scaled:i8, tx_air_secs:u32, rx_air_secs:u32`.
const RADIO_STATS_LEN: usize = 12;

/// Parses a [`StatsCategory::Radio`] payload's `raw` bytes into
/// [`RadioStatsData`]. `last_snr` is unscaled from the firmware's ×4
/// wire encoding (`last_snr_scaled as f32 / 4.0`).
pub fn parse_radio_stats(data: &[u8]) -> Result<RadioStatsData> {
    if data.len() < RADIO_STATS_LEN {
        return Err(Error::protocol("Radio stats payload too short"));
    }
    let last_rssi = *data
        .get(2)
        .ok_or_else(|| Error::protocol("Radio stats payload too short"))? as i8;
    let last_snr_scaled = *data
        .get(3)
        .ok_or_else(|| Error::protocol("Radio stats payload too short"))?
        as i8;
    Ok(RadioStatsData {
        noise_floor: read_i16_le(data, 0)?,
        last_rssi,
        last_snr: last_snr_scaled as f32 / 4.0,
        tx_air_secs: read_u32_le(data, 4)?,
        rx_air_secs: read_u32_le(data, 8)?,
    })
}

/// Length of a [`StatsCategory::Packets`] payload (`raw`) without the
/// optional trailing `recv_errors:u32` (older firmware).
const PACKET_STATS_LEN: usize = 24;
/// Length of a [`StatsCategory::Packets`] payload (`raw`) including
/// `recv_errors:u32` (newer firmware).
const PACKET_STATS_LEN_WITH_ERRORS: usize = 28;

/// Parses a [`StatsCategory::Packets`] payload's `raw` bytes into
/// [`PacketStatsData`]. Accepts either the legacy 24-byte frame
/// (`recv_errors` becomes `None`) or the newer 28-byte one.
pub fn parse_packet_stats(data: &[u8]) -> Result<PacketStatsData> {
    if data.len() < PACKET_STATS_LEN {
        return Err(Error::protocol("Packet stats payload too short"));
    }
    let recv_errors = if data.len() >= PACKET_STATS_LEN_WITH_ERRORS {
        Some(read_u32_le(data, 24)?)
    } else {
        None
    };
    Ok(PacketStatsData {
        recv: read_u32_le(data, 0)?,
        sent: read_u32_le(data, 4)?,
        flood_tx: read_u32_le(data, 8)?,
        direct_tx: read_u32_le(data, 12)?,
        flood_rx: read_u32_le(data, 16)?,
        direct_rx: read_u32_le(data, 20)?,
        recv_errors,
    })
}

// --- Advertisement payload layout ---

/// Length of the advertiser's 6-byte public key prefix.
const ADVERT_PREFIX_LEN: usize = 6;
/// Fixed width of the advertisement name field.
const ADVERT_NAME_LEN: usize = 32;
/// Minimum payload length (prefix + name header).
const ADVERT_MIN_LEN: usize = ADVERT_PREFIX_LEN + 8; // prefix(6) + at least some name bytes
const ADVERT_NAME_OFFSET: usize = ADVERT_PREFIX_LEN;
/// Offset of the optional latitude field.
const ADVERT_LAT_OFFSET: usize = ADVERT_NAME_OFFSET + ADVERT_NAME_LEN;
/// Offset of the optional longitude field.
const ADVERT_LON_OFFSET: usize = ADVERT_LAT_OFFSET + 4;

/// Parse an [`AdvertisementData`] from a `PacketType::Advertisement` payload.
///
/// Returns an error if the payload is too short to contain the prefix and
/// at least part of the name.
pub fn parse_advertisement(payload: &[u8]) -> Result<AdvertisementData> {
    if payload.len() < ADVERT_MIN_LEN {
        return Err(Error::protocol("Advertisement payload too short"));
    }

    let prefix: [u8; 6] = read_bytes(payload, 0)?;
    let name = read_string(payload, ADVERT_NAME_OFFSET, ADVERT_NAME_LEN);
    let lat = if payload.len() >= ADVERT_LAT_OFFSET + 4 {
        read_i32_le(payload, ADVERT_LAT_OFFSET).unwrap_or(0)
    } else {
        0
    };
    let lon = if payload.len() >= ADVERT_LON_OFFSET + 4 {
        read_i32_le(payload, ADVERT_LON_OFFSET).unwrap_or(0)
    } else {
        0
    };

    Ok(AdvertisementData {
        prefix,
        name,
        lat,
        lon,
    })
}

// --- PathUpdate payload layout ---

/// Minimum length for a PathUpdate payload (prefix + path_len byte).
const PATH_UPDATE_MIN_LEN: usize = 7;
const PATH_UPDATE_PATH_LEN_OFFSET: usize = 6;
const PATH_UPDATE_PATH_OFFSET: usize = 7;

/// Parse a [`PathUpdateData`] from a `PacketType::PathUpdate` payload.
///
/// Returns an error if the payload is too short.
pub fn parse_path_update(payload: &[u8]) -> Result<PathUpdateData> {
    if payload.len() < PATH_UPDATE_MIN_LEN {
        return Err(Error::protocol("PathUpdate payload too short"));
    }

    let prefix: [u8; 6] = read_bytes(payload, 0)?;
    let path_len = payload[PATH_UPDATE_PATH_LEN_OFFSET] as i8; // jonesy:allow(bounds) -- checked >= PATH_UPDATE_MIN_LEN above
    let path = if payload.len() > PATH_UPDATE_PATH_OFFSET {
        payload[PATH_UPDATE_PATH_OFFSET..].to_vec()
    } else {
        Vec::new()
    };

    Ok(PathUpdateData {
        prefix,
        path_len,
        path,
    })
}

// --- TraceData payload layout ---

/// Size of each trace hop entry (6-byte prefix + 1-byte SNR).
const TRACE_HOP_LEN: usize = 7;
/// Offset of the SNR byte within a hop entry.
const TRACE_HOP_SNR_OFFSET: usize = 6;

/// Parse a [`TraceInfo`] from a `PacketType::TraceData` payload.
///
/// Returns a (possibly empty) list of trace hops.
pub fn parse_trace_data(payload: &[u8]) -> TraceInfo {
    let mut hops = Vec::new();
    let mut offset = 0;
    while offset + TRACE_HOP_LEN <= payload.len() {
        // jonesy:allow(overflow)
        let prefix: [u8; 6] = read_bytes(payload, offset).unwrap_or([0; 6]);
        let snr_raw = payload[offset + TRACE_HOP_SNR_OFFSET] as i8; // jonesy:allow(bounds, overflow) -- loop guard ensures offset + 7 <= len
        let snr = snr_raw as f32 / 4.0;
        hops.push(TraceHop { prefix, snr });
        offset += TRACE_HOP_LEN; // jonesy:allow(overflow) -- bounded by payload.len()
    }
    TraceInfo { hops }
}

// --- ControlData / DiscoverResponse layout ---

/// Size of one discover entry (32-byte pubkey + 32-byte name).
const DISCOVER_ENTRY_LEN: usize = 64;
/// Minimum readable portion of an entry (pubkey + at least some name).
const DISCOVER_ENTRY_MIN_LEN: usize = 38;
/// Offset of the name within an entry.
const DISCOVER_NAME_OFFSET: usize = 32;
/// Maximum name length in a discover entry.
const DISCOVER_NAME_LEN: usize = 32;

/// Parse a list of [`DiscoverEntry`] from a
/// `ControlType::NodeDiscoverResp` payload.
///
/// The `payload` should start *after* the control-type byte (i.e. at the
/// first entry).
pub fn parse_discover_response(payload: &[u8]) -> Vec<DiscoverEntry> {
    let mut entries = Vec::new();
    let mut offset = 0;
    while offset + DISCOVER_ENTRY_MIN_LEN <= payload.len() {
        // jonesy:allow(overflow)
        let pubkey = payload[offset..offset + DISCOVER_NAME_OFFSET].to_vec(); // jonesy:allow(overflow)
        let name = read_string(payload, offset + DISCOVER_NAME_OFFSET, DISCOVER_NAME_LEN); // jonesy:allow(overflow)
        entries.push(DiscoverEntry { pubkey, name });
        offset += DISCOVER_ENTRY_LEN; // jonesy:allow(overflow) -- bounded by payload.len()
    }
    entries
}

// --- CurrentTime payload layout ---

/// Minimum length for a CurrentTime payload.
const CURRENT_TIME_MIN_LEN: usize = 4;

/// Parse a Unix timestamp from a `PacketType::CurrentTime` payload.
pub fn parse_current_time(payload: &[u8]) -> Result<u32> {
    if payload.len() < CURRENT_TIME_MIN_LEN {
        return Err(Error::protocol("CurrentTime payload too short"));
    }
    read_u32_le(payload, 0)
}

// --- PrivateKey payload layout ---

/// Length of a private key.
const PRIVATE_KEY_LEN: usize = 64;

/// Parse a 64-byte private key from a `PacketType::PrivateKey` payload.
pub fn parse_private_key(payload: &[u8]) -> Result<[u8; 64]> {
    if payload.len() < PRIVATE_KEY_LEN {
        return Err(Error::protocol("PrivateKey payload too short"));
    }
    read_bytes(payload, 0)
}

// --- SignStart payload layout ---

/// Minimum length for a SignStart payload.
const SIGN_START_MIN_LEN: usize = 4;

/// Parse the `max_length` from a `PacketType::SignStart` payload.
pub fn parse_sign_start(payload: &[u8]) -> Result<u32> {
    if payload.len() < SIGN_START_MIN_LEN {
        return Err(Error::protocol("SignStart payload too short"));
    }
    read_u32_le(payload, 0)
}

// --- Ack payload layout ---

/// Minimum length for an Ack payload.
const ACK_MIN_LEN: usize = 4;

/// Parse a 4-byte tag from a `PacketType::Ack` payload.
pub fn parse_ack(payload: &[u8]) -> Result<[u8; 4]> {
    if payload.len() < ACK_MIN_LEN {
        return Err(Error::protocol("Ack payload too short"));
    }
    read_bytes(payload, 0)
}

// --- ContactEnd payload layout ---

/// Minimum length for a ContactEnd payload to contain a
/// `last_modification_timestamp`.
const CONTACT_END_TIMESTAMP_LEN: usize = 4;

/// Parse the optional `last_modification_timestamp` from a
/// `PacketType::ContactEnd` payload.
///
/// Returns `Ok(None)` if the payload is too short to contain the
/// timestamp (the timestamp is optional in the protocol).
pub fn parse_contact_end_timestamp(payload: &[u8]) -> Result<Option<u32>> {
    if payload.len() >= CONTACT_END_TIMESTAMP_LEN {
        Ok(Some(read_u32_le(payload, 0)?))
    } else {
        Ok(None)
    }
}

// --- StatusResponse payload layout ---

/// Length of the sender prefix in a StatusResponse payload.
const STATUS_RESP_PREFIX_LEN: usize = 6;
/// Minimum length for a StatusResponse payload (prefix + status data).
const STATUS_RESP_MIN_LEN: usize = 58;

/// Parsed status response: sender prefix and status data.
pub struct StatusResponseFrame {
    /// 6-byte public key prefix of the sender.
    pub sender_prefix: [u8; 6],
    /// Parsed status data.
    pub status: StatusData,
}

/// Parse a [`StatusResponseFrame`] from a `PacketType::StatusResponse`
/// payload.
///
/// Returns an error if the payload is too short or the status data cannot
/// be parsed.
pub fn parse_status_response(payload: &[u8]) -> Result<StatusResponseFrame> {
    if payload.len() < STATUS_RESP_MIN_LEN {
        return Err(Error::protocol("StatusResponse payload too short"));
    }

    let sender_prefix: [u8; 6] = read_bytes(payload, 0)?;
    let status = parse_status(&payload[STATUS_RESP_PREFIX_LEN..], sender_prefix)?;

    Ok(StatusResponseFrame {
        sender_prefix,
        status,
    })
}

// --- TelemetryResponse payload layout ---

/// Minimum length for a TelemetryResponse payload (4-byte tag).
const TELEMETRY_RESP_MIN_LEN: usize = 4;
/// Offset where the LPP telemetry data begins.
const TELEMETRY_RESP_DATA_OFFSET: usize = 4;

/// Parsed telemetry response: tag and LPP data.
pub struct TelemetryResponseFrame {
    /// The 4-byte request tag.
    pub tag: [u8; 4],
    /// LPP telemetry data.
    pub data: Vec<u8>,
}

/// Parse a [`TelemetryResponseFrame`] from a
/// `PacketType::TelemetryResponse` payload.
///
/// Returns an error if the payload is too short.
pub fn parse_telemetry_response(payload: &[u8]) -> Result<TelemetryResponseFrame> {
    if payload.len() < TELEMETRY_RESP_MIN_LEN {
        return Err(Error::protocol("TelemetryResponse payload too short"));
    }

    let tag: [u8; 4] = read_bytes(payload, 0)?;
    let data = payload[TELEMETRY_RESP_DATA_OFFSET..].to_vec();

    Ok(TelemetryResponseFrame { tag, data })
}

// --- CustomVars payload ---

/// Parse custom variables from a `PacketType::CustomVars` payload.
///
/// The payload is UTF-8 text with `key=value` pairs separated by
/// newlines.
pub fn parse_custom_vars(payload: &[u8]) -> HashMap<String, String> {
    let mut vars = HashMap::new();
    let text = String::from_utf8_lossy(payload);
    for line in text.lines() {
        if let Some((key, value)) = line.split_once('=') {
            vars.insert(key.to_string(), value.to_string());
        }
    }
    vars
}

// --- BinaryResponse frame layout ---
//
// Firmware layout: [subtype: 1][tag: 4][data...]
// The subtype byte is skipped; the tag lives at offset 1 and the
// response data starts at offset 5.

/// Length of the subtype byte that precedes the tag.
const BINARY_RESP_SUBTYPE_LEN: usize = 1;
/// Length of the 4-byte request tag echoed back in the response.
const BINARY_RESP_TAG_LEN: usize = 4;
/// Offset of the tag within the BinaryResponse payload.
const BINARY_RESP_TAG_OFFSET: usize = BINARY_RESP_SUBTYPE_LEN;
/// Offset where the response data begins.
const BINARY_RESP_DATA_OFFSET: usize = BINARY_RESP_TAG_OFFSET + BINARY_RESP_TAG_LEN;
/// Minimum payload length (subtype + tag).
const BINARY_RESP_MIN_LEN: usize = BINARY_RESP_DATA_OFFSET;

/// Parsed binary response frame: tag and data extracted from the raw
/// `PacketType::BinaryResponse` payload.
pub struct BinaryResponseFrame {
    /// The 4-byte request tag echoed by the firmware.
    pub tag: [u8; 4],
    /// The response data (everything after the tag).
    pub data: Vec<u8>,
}

/// Parse the frame envelope of a `PacketType::BinaryResponse` payload,
/// extracting the tag and data.
///
/// Returns an error if the payload is too short to contain the subtype
/// byte and the 4-byte tag.
pub fn parse_binary_response_frame(payload: &[u8]) -> Result<BinaryResponseFrame> {
    if payload.len() < BINARY_RESP_MIN_LEN {
        return Err(Error::protocol("BinaryResponse payload too short"));
    }

    let tag: [u8; 4] = read_bytes(payload, BINARY_RESP_TAG_OFFSET)?;
    let data = payload[BINARY_RESP_DATA_OFFSET..].to_vec();

    Ok(BinaryResponseFrame { tag, data })
}

// --- PathDiscoveryResponse payload layout ---
//
// Firmware layout: [reserved: 1][pubkey_prefix: 6][out_path_byte: 1][out_path...][in_path_byte: 1][in_path...]
// The path byte encodes hash_len in bits 6-7 and hop count in bits 0-5.

/// Length of the reserved byte at the start of the response.
const PATH_DISC_RESERVED_LEN: usize = 1;
/// Length of the public key prefix.
const PATH_DISC_PREFIX_LEN: usize = 6;
/// Minimum payload length: reserved + prefix + out_path_byte + in_path_byte.
const PATH_DISC_MIN_LEN: usize = PATH_DISC_RESERVED_LEN + PATH_DISC_PREFIX_LEN + 1 + 1;
/// Offset of the public key prefix.
const PATH_DISC_PREFIX_OFFSET: usize = PATH_DISC_RESERVED_LEN;
/// Offset of the outbound path descriptor byte.
const PATH_DISC_OUT_PATH_OFFSET: usize = PATH_DISC_PREFIX_OFFSET + PATH_DISC_PREFIX_LEN;

/// Parse a [`PathDiscoveryResponseData`] from a
/// `PacketType::PathDiscoveryResponse` payload.
///
/// Returns an error if the payload is too short.
pub fn parse_path_discovery_response(payload: &[u8]) -> Result<PathDiscoveryResponseData> {
    if payload.len() < PATH_DISC_MIN_LEN {
        return Err(Error::protocol("PathDiscoveryResponse payload too short"));
    }

    let pubkey_prefix: [u8; 6] = read_bytes(payload, PATH_DISC_PREFIX_OFFSET)?;

    // Outbound path
    let out_path_byte = payload[PATH_DISC_OUT_PATH_OFFSET]; // jonesy:allow(bounds) -- checked >= PATH_DISC_MIN_LEN
    let out_path_hash_len = ((out_path_byte & 0xC0) >> 6) + 1; // jonesy:allow(overflow)
    let out_path_len = out_path_byte & 0x3F;
    let out_path_bytes = out_path_len as usize * out_path_hash_len as usize; // jonesy:allow(overflow)

    let out_path_start = PATH_DISC_OUT_PATH_OFFSET + 1; // jonesy:allow(overflow)
    let out_path_end = out_path_start
        .checked_add(out_path_bytes)
        .ok_or_else(|| Error::protocol("PathDiscoveryResponse outbound path overflow"))?;
    if out_path_end >= payload.len() {
        return Err(Error::protocol(
            "PathDiscoveryResponse outbound path truncated",
        ));
    }
    let out_path = payload[out_path_start..out_path_end].to_vec(); // jonesy:allow(bounds) -- checked above

    // Inbound path
    let in_path_byte_offset = out_path_end;
    let in_path_byte = *payload
        .get(in_path_byte_offset)
        .ok_or_else(|| Error::protocol("PathDiscoveryResponse missing inbound path byte"))?;
    let in_path_hash_len = ((in_path_byte & 0xC0) >> 6) + 1; // jonesy:allow(overflow)
    let in_path_len = in_path_byte & 0x3F;
    let in_path_bytes = in_path_len as usize * in_path_hash_len as usize; // jonesy:allow(overflow)

    let in_path_start = in_path_byte_offset + 1; // jonesy:allow(overflow)
    let in_path_end = in_path_start
        .checked_add(in_path_bytes)
        .ok_or_else(|| Error::protocol("PathDiscoveryResponse inbound path overflow"))?;
    if in_path_end > payload.len() {
        return Err(Error::protocol(
            "PathDiscoveryResponse inbound path truncated",
        ));
    }
    let in_path = payload[in_path_start..in_path_end].to_vec(); // jonesy:allow(bounds) -- checked above

    Ok(PathDiscoveryResponseData {
        pubkey_prefix,
        out_path_len,
        out_path_hash_len,
        out_path,
        in_path_len,
        in_path_hash_len,
        in_path,
    })
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
    fn test_read_string_stops_at_control_character_when_not_null_terminated() {
        // No NUL anywhere in the 32-byte window (a real firmware quirk
        // observed on some advertisement pushes) — trailing bytes are
        // leftover garbage including raw control bytes.
        let mut data = b"39-HTJURA-YAN-RPT3".to_vec();
        data.extend_from_slice(&[0x01, 0x02, 0xFF, 0x03]); // garbage, no 0x00
        data.resize(32, 0x7F); // pad with DEL (a control character), not NUL
        assert_eq!(read_string(&data, 0, 32), "39-HTJURA-YAN-RPT3");
    }

    #[test]
    fn test_read_string_stops_at_invalid_utf8_when_not_null_terminated() {
        let mut data = b"Node".to_vec();
        data.push(0x80); // invalid UTF-8 continuation byte with no leader
        data.extend_from_slice(&[0x41; 10]); // more bytes after the anomaly
        assert_eq!(read_string(&data, 0, data.len()), "Node");
    }

    #[test]
    fn test_read_string_all_garbage_returns_empty() {
        let data = [0x01, 0x02, 0x03, 0x04];
        assert_eq!(read_string(&data, 0, 4), "");
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
        // battery_mv at 0 (4.2V = 4200mV)
        data[0..2].copy_from_slice(&4200u16.to_le_bytes());
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

        assert_eq!(status.battery_mv, 4200);
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
        let mut data = Vec::new();
        data.push(5); // channel_idx
        data.push(1); // path_len
        data.push(0); // txt_type
        data.extend_from_slice(&1234567890u32.to_le_bytes());
        data.extend_from_slice(b"Channel");

        let msg = parse_channel_msg(&data).unwrap();
        assert_eq!(msg.channel_idx, 5);
        assert_eq!(msg.path_len, 1);
        assert_eq!(msg.text, "Channel");
    }

    #[test]
    fn test_parse_channel_msg_too_short() {
        let data = vec![0u8; 5];
        assert!(parse_channel_msg(&data).is_err());
    }

    #[test]
    fn test_parse_channel_msg_v3() {
        let mut data = Vec::new();
        data.push(40); // snr_raw = 40, SNR = 10.0
        data.extend_from_slice(&[0x00, 0x00]); // reserved bytes
        data.push(5); // channel_idx
        data.push(2); // path_len
        data.push(0); // txt_type
        data.extend_from_slice(&1234567890u32.to_le_bytes());
        data.extend_from_slice(b"V3 chan");

        let msg = parse_channel_msg_v3(&data).unwrap();
        assert_eq!(msg.snr, Some(10.0));
        assert_eq!(msg.channel_idx, 5);
        assert_eq!(msg.path_len, 2);
        assert_eq!(msg.text, "V3 chan");
    }

    #[test]
    fn test_parse_channel_msg_v3_too_short() {
        let data = vec![0u8; 8];
        assert!(parse_channel_msg_v3(&data).is_err());
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

    #[test]
    fn test_parse_device_info_empty() {
        assert!(parse_device_info(&[]).is_err());
    }

    #[test]
    fn test_parse_device_info_v2() {
        // Pre-v3 firmware only has version code
        let data = [2u8]; // fw_version_code = 2
        let info = parse_device_info(&data).unwrap();
        assert_eq!(info.fw_version_code, 2);
        assert!(info.max_contacts.is_none());
        assert!(info.max_channels.is_none());
        assert!(info.ble_pin.is_none());
    }

    #[test]
    fn test_parse_device_info_v3_partial() {
        // v3+ but not all fields present
        let mut data = vec![0u8; 10];
        data[0] = 3; // fw_version_code
        data[1] = 25; // max_contacts / 2
        data[2] = 4; // max_channels
        data[3..7].copy_from_slice(&5678u32.to_le_bytes()); // ble_pin

        let info = parse_device_info(&data).unwrap();
        assert_eq!(info.fw_version_code, 3);
        assert_eq!(info.max_contacts, Some(50)); // 25 * 2
        assert_eq!(info.max_channels, Some(4));
        assert_eq!(info.ble_pin, Some(5678));
        assert!(info.fw_build.is_none()); // Not enough data
        assert!(info.model.is_none());
        assert!(info.version.is_none());
        assert!(info.repeat.is_none());
    }

    #[test]
    fn test_parse_device_info_full() {
        // Full v9+ device info
        let mut data = vec![0u8; 80];
        data[0] = 9; // fw_version_code
        data[1] = 50; // max_contacts / 2 = 100
        data[2] = 8; // max_channels
        data[3..7].copy_from_slice(&1234u32.to_le_bytes()); // ble_pin

        // fw_build at offset 7 (12 bytes)
        data[7..18].copy_from_slice(b"Feb 15 2025");

        // model at offset 19 (40 bytes)
        data[19..29].copy_from_slice(b"T-Deck Pro");

        // version at offset 59 (20 bytes)
        data[59..64].copy_from_slice(b"1.2.3");

        // repeat at offset 79
        data[79] = 1;

        let info = parse_device_info(&data).unwrap();
        assert_eq!(info.fw_version_code, 9);
        assert_eq!(info.max_contacts, Some(100));
        assert_eq!(info.max_channels, Some(8));
        assert_eq!(info.ble_pin, Some(1234));
        assert_eq!(info.fw_build.as_deref(), Some("Feb 15 2025"));
        assert_eq!(info.model.as_deref(), Some("T-Deck Pro"));
        assert_eq!(info.version.as_deref(), Some("1.2.3"));
        assert_eq!(info.repeat, Some(true));
    }

    #[test]
    fn test_parse_device_info_repeat_false() {
        let mut data = vec![0u8; 80];
        data[0] = 9;
        data[79] = 0; // repeat disabled

        let info = parse_device_info(&data).unwrap();
        assert_eq!(info.repeat, Some(false));
    }

    #[test]
    fn test_parse_device_info_max_contacts_overflow() {
        // Test that max_contacts * 2 doesn't overflow
        let mut data = vec![0u8; 3];
        data[0] = 3;
        data[1] = 200; // 200 * 2 = 400, but u8 max is 255, so saturates to 255

        let info = parse_device_info(&data).unwrap();
        // 200 * 2 would overflow u8, but we use saturating_mul
        assert_eq!(info.max_contacts, Some(255)); // Saturated
    }

    #[test]
    fn test_parse_mesh_packet_header_flood_no_transport() {
        // route=Flood, payload_type=TextMsg, payload_ver=0
        let header_byte: u8 =
            ((PayloadType::TextMsg as u8) << PAYLOAD_TYPE_SHIFT) | RouteType::Flood as u8;
        // path_hash_size=1 (bits 6-7 = 0b00), path_len=2
        let path_byte = 0b00_000010;
        let data = [header_byte, path_byte, 0xAA, 0xBB, 0xCC, 0xDD];

        let (header, remaining) = parse_mesh_packet_header(&data).unwrap();
        assert_eq!(header.route_type, RouteType::Flood);
        assert_eq!(header.payload_type, PayloadType::TextMsg);
        assert_eq!(header.payload_version, 0);
        assert!(header.transport_code.is_none());
        assert_eq!(header.path_len, 2);
        assert_eq!(header.path_hash_size, 1);
        assert_eq!(header.path, vec![0xAA, 0xBB]);
        assert_eq!(remaining, &[0xCC, 0xDD]);
    }

    #[test]
    fn test_parse_mesh_packet_header_with_transport_code() {
        // route=TransportFlood, payload_type=Advert, payload_ver=1
        let header_byte = (1u8 << PAYLOAD_VERSION_SHIFT)
            | ((PayloadType::Advert as u8) << PAYLOAD_TYPE_SHIFT)
            | RouteType::TransportFlood as u8;
        // path_hash_size=2 (bits 6-7 = 0b01 -> +1), path_len=1
        let path_byte = 0b01_000001;
        let data = [
            header_byte, // header
            0x11,
            0x22,
            0x33,
            0x44,      // transport code
            path_byte, // path descriptor
            0x01,
            0x02, // path (1 hop * 2 bytes)
            0x99, // remaining inner payload
        ];

        let (header, remaining) = parse_mesh_packet_header(&data).unwrap();
        assert_eq!(header.route_type, RouteType::TransportFlood);
        assert_eq!(header.payload_type, PayloadType::Advert);
        assert_eq!(header.payload_version, 1);
        assert_eq!(header.transport_code, Some([0x11, 0x22, 0x33, 0x44]));
        assert_eq!(header.path_len, 1);
        assert_eq!(header.path_hash_size, 2);
        assert_eq!(header.path, vec![0x01, 0x02]);
        assert_eq!(remaining, &[0x99]);
    }

    #[test]
    fn test_parse_mesh_packet_header_empty() {
        assert!(parse_mesh_packet_header(&[]).is_err());
    }

    #[test]
    fn test_parse_mesh_packet_header_missing_path_byte() {
        // Direct route, no transport code, but no path byte follows
        let data = [2u8];
        assert!(parse_mesh_packet_header(&data).is_err());
    }

    #[test]
    fn test_parse_mesh_packet_header_path_truncated() {
        // path_len=5, path_hash_size=1 declared, but no path bytes follow
        let data = [1u8, 0b00_000101];
        assert!(parse_mesh_packet_header(&data).is_err());
    }

    #[test]
    fn test_parse_mesh_packet_header_missing_transport_code() {
        // TransportDirect route requires a 4-byte transport code that isn't present
        let data = [3u8, 0x11, 0x22];
        assert!(parse_mesh_packet_header(&data).is_err());
    }

    #[test]
    fn test_parse_raw_advertisement_with_name() {
        let mut data = vec![0u8; MIN_LEN];
        data[0..6].copy_from_slice(&[0x01, 0x02, 0x03, 0x04, 0x05, 0x06]); // pubkey prefix
        data[TIMESTAMP_OFFSET..TIMESTAMP_OFFSET + 4].copy_from_slice(&123456u32.to_le_bytes());
        data[SIGNATURE_OFFSET..SIGNATURE_OFFSET + 4].copy_from_slice(&[0xAA, 0xBB, 0xCC, 0xDD]); // signature prefix
        data[FLAGS_OFFSET] = FLAG_HAS_NAME;
        data.extend_from_slice(b"Node1");

        let adv = parse_raw_advertisement(&data).unwrap();
        assert_eq!(&adv.public_key[0..6], &[0x01, 0x02, 0x03, 0x04, 0x05, 0x06]);
        assert_eq!(adv.timestamp, 123456);
        assert_eq!(&adv.signature[0..4], &[0xAA, 0xBB, 0xCC, 0xDD]);
        assert_eq!(adv.adv_type, 0);
        assert!(adv.lat.is_none());
        assert!(adv.lon.is_none());
        assert_eq!(adv.name.as_deref(), Some("Node1"));
    }

    #[test]
    fn test_parse_raw_advertisement_with_location() {
        let mut data = vec![0u8; MIN_LEN];
        data[FLAGS_OFFSET] = 1 | FLAG_HAS_LOCATION; // adv_type=1, has location
        data.extend_from_slice(&37774900i32.to_le_bytes());
        data.extend_from_slice(&(-122419400i32).to_le_bytes());

        let adv = parse_raw_advertisement(&data).unwrap();
        assert_eq!(adv.adv_type, 1);
        assert_eq!(adv.lat, Some(37774900));
        assert_eq!(adv.lon, Some(-122419400));
        assert!(adv.name.is_none());
    }

    #[test]
    fn test_parse_raw_advertisement_skips_unknown_feature_blocks() {
        // flags: feature1 + feature2 + name, no location. Each feature
        // block is 2 bytes we don't decode but must still skip correctly,
        // otherwise the name would be read from the wrong offset (garbled,
        // or misaligned into the feature bytes).
        let mut data = vec![0u8; MIN_LEN];
        data[FLAGS_OFFSET] = FLAG_HAS_FEATURE1 | FLAG_HAS_FEATURE2 | FLAG_HAS_NAME;
        data.extend_from_slice(&[0xAA, 0xAA]); // feature1, skipped
        data.extend_from_slice(&[0xBB, 0xBB]); // feature2, skipped
        data.extend_from_slice(b"Node2");

        let adv = parse_raw_advertisement(&data).unwrap();
        assert!(adv.lat.is_none());
        assert!(adv.lon.is_none());
        assert_eq!(adv.name.as_deref(), Some("Node2"));
    }

    #[test]
    fn test_parse_raw_advertisement_too_short() {
        let data = vec![0u8; MIN_LEN - 1];
        assert!(parse_raw_advertisement(&data).is_err());
    }

    #[test]
    fn test_parse_raw_advertisement_truncated_location_fails() {
        // flags: has location (0x10) and name (0x80), but only 4 trailing
        // bytes — not enough for the 8-byte lat/lon. Must be rejected as a
        // parse failure rather than silently misreading the name from
        // inside the truncated location bytes.
        let mut data = vec![0u8; MIN_LEN];
        data[FLAGS_OFFSET] = FLAG_HAS_LOCATION | FLAG_HAS_NAME;
        data.extend_from_slice(&[0xAA, 0xBB, 0xCC, 0xDD]);

        assert!(parse_raw_advertisement(&data).is_err());
    }

    // ========== parse_advert_response tests ==========

    /// Build a minimal valid AdvertResponse payload (74 bytes: tag + pubkey
    /// + adv_type + node_name + timestamp + flags).
    fn build_advert_response_payload() -> Vec<u8> {
        let mut data = Vec::new();
        data.extend_from_slice(&[0x01, 0x02, 0x03, 0x04]); // tag
        data.extend_from_slice(&[0xAA; 32]); // pubkey
        data.push(2); // adv_type
        let mut name = [0u8; 32];
        name[..5].copy_from_slice(b"TestN");
        data.extend_from_slice(&name); // node_name (32 bytes)
        data.extend_from_slice(&1700000000u32.to_le_bytes()); // timestamp
        data.push(0x05); // flags
        data
    }

    #[test]
    fn test_parse_advert_response_too_short() {
        let data = vec![0u8; ADVERT_RESP_MIN_LEN - 1];
        assert!(parse_advert_response(&data).is_err());
    }

    #[test]
    fn test_parse_advert_response_empty() {
        assert!(parse_advert_response(&[]).is_err());
    }

    #[test]
    fn test_parse_advert_response_minimal() {
        let data = build_advert_response_payload();
        assert_eq!(data.len(), ADVERT_RESP_MIN_LEN);

        let resp = parse_advert_response(&data).unwrap();
        assert_eq!(resp.tag, [0x01, 0x02, 0x03, 0x04]);
        assert_eq!(resp.pubkey, [0xAA; 32]);
        assert_eq!(resp.adv_type, 2);
        assert_eq!(resp.node_name, "TestN");
        assert_eq!(resp.timestamp, 1700000000);
        assert_eq!(resp.flags, 0x05);
        assert!(resp.lat.is_none());
        assert!(resp.lon.is_none());
        assert!(resp.node_desc.is_none());
    }

    #[test]
    fn test_parse_advert_response_with_latlon() {
        let mut data = build_advert_response_payload();
        data.extend_from_slice(&37774900i32.to_le_bytes()); // lat
        data.extend_from_slice(&(-122419400i32).to_le_bytes()); // lon
        assert_eq!(data.len(), ADVERT_RESP_LATLON_MIN_LEN);

        let resp = parse_advert_response(&data).unwrap();
        assert_eq!(resp.lat, Some(37774900));
        assert_eq!(resp.lon, Some(-122419400));
        assert!(resp.node_desc.is_none());
    }

    #[test]
    fn test_parse_advert_response_with_latlon_and_desc() {
        let mut data = build_advert_response_payload();
        data.extend_from_slice(&37774900i32.to_le_bytes()); // lat
        data.extend_from_slice(&(-122419400i32).to_le_bytes()); // lon
        let mut desc = [0u8; 32];
        desc[..6].copy_from_slice(b"Relay1");
        data.extend_from_slice(&desc); // node_desc

        let resp = parse_advert_response(&data).unwrap();
        assert_eq!(resp.lat, Some(37774900));
        assert_eq!(resp.lon, Some(-122419400));
        assert_eq!(resp.node_desc.as_deref(), Some("Relay1"));
    }

    #[test]
    fn test_parse_advert_response_partial_latlon_ignored() {
        // Payload extends past flags but not enough for both lat and lon
        let mut data = build_advert_response_payload();
        data.extend_from_slice(&[0xFF; 4]); // only 4 extra bytes, need 8

        let resp = parse_advert_response(&data).unwrap();
        // Not enough for lat+lon, so they remain None
        assert!(resp.lat.is_none());
        assert!(resp.lon.is_none());
    }

    #[test]
    fn test_parse_advert_response_latlon_exact_no_desc() {
        // Exactly at LATLON_MIN_LEN: lat+lon present but no description
        let mut data = build_advert_response_payload();
        data.extend_from_slice(&0i32.to_le_bytes()); // lat = 0
        data.extend_from_slice(&0i32.to_le_bytes()); // lon = 0

        let resp = parse_advert_response(&data).unwrap();
        assert_eq!(resp.lat, Some(0));
        assert_eq!(resp.lon, Some(0));
        assert!(resp.node_desc.is_none());
    }

    #[test]
    fn test_parse_advert_response_offset_constants() {
        // Verify the computed constants match the documented byte layout
        assert_eq!(ADVERT_RESP_TAG_OFFSET, 0);
        assert_eq!(ADVERT_RESP_PUBKEY_OFFSET, 4);
        assert_eq!(ADVERT_RESP_ADV_TYPE_OFFSET, 36);
        assert_eq!(ADVERT_RESP_NODE_NAME_OFFSET, 37);
        assert_eq!(ADVERT_RESP_TIMESTAMP_OFFSET, 69);
        assert_eq!(ADVERT_RESP_FLAGS_OFFSET, 73);
        assert_eq!(ADVERT_RESP_MIN_LEN, 74);
        assert_eq!(ADVERT_RESP_LAT_OFFSET, 74);
        assert_eq!(ADVERT_RESP_LON_OFFSET, 78);
        assert_eq!(ADVERT_RESP_LATLON_MIN_LEN, 82);
        assert_eq!(ADVERT_RESP_NODE_DESC_OFFSET, 82);
    }

    // ========== parse_battery tests ==========

    #[test]
    fn test_parse_battery_too_short() {
        assert!(parse_battery(&[]).is_err());
        assert!(parse_battery(&[0x01]).is_err());
    }

    #[test]
    fn test_parse_battery_voltage_only() {
        let data = 3700u16.to_le_bytes();
        let info = parse_battery(&data).unwrap();
        assert_eq!(info.battery_mv, 3700);
        assert!(info.used_kb.is_none());
        assert!(info.total_kb.is_none());
    }

    #[test]
    fn test_parse_battery_with_storage() {
        let mut data = Vec::new();
        data.extend_from_slice(&3800u16.to_le_bytes());
        data.extend_from_slice(&1024u32.to_le_bytes()); // used_kb
        data.extend_from_slice(&4096u32.to_le_bytes()); // total_kb
        let info = parse_battery(&data).unwrap();
        assert_eq!(info.battery_mv, 3800);
        assert_eq!(info.used_kb, Some(1024));
        assert_eq!(info.total_kb, Some(4096));
    }

    #[test]
    fn test_parse_battery_partial_storage_ignored() {
        // 2 bytes voltage + 4 bytes used_kb, but no total_kb (6 bytes < 10)
        let mut data = Vec::new();
        data.extend_from_slice(&3500u16.to_le_bytes());
        data.extend_from_slice(&512u32.to_le_bytes());
        let info = parse_battery(&data).unwrap();
        assert_eq!(info.battery_mv, 3500);
        assert!(info.used_kb.is_none());
        assert!(info.total_kb.is_none());
    }

    // ========== parse_msg_sent tests ==========

    #[test]
    fn test_parse_msg_sent_too_short() {
        assert!(parse_msg_sent(&[]).is_err());
        assert!(parse_msg_sent(&[0; 8]).is_err());
    }

    #[test]
    fn test_parse_msg_sent_valid() {
        let mut data = vec![0x02]; // message_type
        data.extend_from_slice(&[0xAA, 0xBB, 0xCC, 0xDD]); // expected_ack
        data.extend_from_slice(&10000u32.to_le_bytes()); // suggested_timeout
        let info = parse_msg_sent(&data).unwrap();
        assert_eq!(info.message_type, 0x02);
        assert_eq!(info.expected_ack, [0xAA, 0xBB, 0xCC, 0xDD]);
        assert_eq!(info.suggested_timeout, 10000);
    }

    // ========== parse_channel_info tests ==========

    #[test]
    fn test_parse_channel_info_too_short() {
        assert!(parse_channel_info(&[]).is_err());
        assert!(parse_channel_info(&[0; 48]).is_err()); // need 49
    }

    #[test]
    fn test_parse_channel_info_valid() {
        let mut data = vec![3u8]; // channel_idx
        let mut name = [0u8; CHANNEL_NAME_LEN];
        name[..4].copy_from_slice(b"Test");
        data.extend_from_slice(&name);
        data.extend_from_slice(&[0xCC; CHANNEL_SECRET_LEN]);
        let info = parse_channel_info(&data).unwrap();
        assert_eq!(info.channel_idx, 3);
        assert_eq!(info.name, "Test");
        assert_eq!(info.secret, [0xCC; CHANNEL_SECRET_LEN]);
    }

    // ========== parse_stats tests ==========

    #[test]
    fn test_parse_stats_empty() {
        assert!(parse_stats(&[]).is_err());
    }

    #[test]
    fn test_parse_stats_core() {
        let data = [0, 0x11, 0x22];
        let stats = parse_stats(&data).unwrap();
        assert_eq!(stats.category, StatsCategory::Core);
        assert_eq!(stats.raw, vec![0x11, 0x22]);
    }

    #[test]
    fn test_parse_stats_radio() {
        let data = [1, 0xAA];
        let stats = parse_stats(&data).unwrap();
        assert_eq!(stats.category, StatsCategory::Radio);
    }

    #[test]
    fn test_parse_stats_packets() {
        let data = [2];
        let stats = parse_stats(&data).unwrap();
        assert_eq!(stats.category, StatsCategory::Packets);
        assert!(stats.raw.is_empty());
    }

    #[test]
    fn test_parse_stats_unknown_category_errors() {
        let data = [99, 0xFF];
        assert!(parse_stats(&data).is_err());
    }

    // ========== parse_core_stats tests ==========

    #[test]
    fn test_parse_core_stats_too_short() {
        assert!(parse_core_stats(&[]).is_err());
        assert!(parse_core_stats(&[0; 8]).is_err()); // need 9
    }

    #[test]
    fn test_parse_core_stats_valid() {
        let mut data = Vec::new();
        data.extend_from_slice(&4012u16.to_le_bytes()); // battery_mv
        data.extend_from_slice(&123456u32.to_le_bytes()); // uptime_secs
        data.extend_from_slice(&7u16.to_le_bytes()); // errors
        data.push(3); // queue_len

        let stats = parse_core_stats(&data).unwrap();
        assert_eq!(stats.battery_mv, 4012);
        assert_eq!(stats.uptime_secs, 123456);
        assert_eq!(stats.errors, 7);
        assert_eq!(stats.queue_len, 3);
    }

    // ========== parse_radio_stats tests ==========

    #[test]
    fn test_parse_radio_stats_too_short() {
        assert!(parse_radio_stats(&[]).is_err());
        assert!(parse_radio_stats(&[0; 11]).is_err()); // need 12
    }

    #[test]
    fn test_parse_radio_stats_valid() {
        let mut data = Vec::new();
        data.extend_from_slice(&(-120i16).to_le_bytes()); // noise_floor
        data.push((-80i8) as u8); // last_rssi
        data.push((33i8) as u8); // last_snr_scaled (33 / 4.0 = 8.25 dB)
        data.extend_from_slice(&120u32.to_le_bytes()); // tx_air_secs
        data.extend_from_slice(&340u32.to_le_bytes()); // rx_air_secs

        let stats = parse_radio_stats(&data).unwrap();
        assert_eq!(stats.noise_floor, -120);
        assert_eq!(stats.last_rssi, -80);
        assert_eq!(stats.last_snr, 8.25);
        assert_eq!(stats.tx_air_secs, 120);
        assert_eq!(stats.rx_air_secs, 340);
    }

    // ========== parse_packet_stats tests ==========

    #[test]
    fn test_parse_packet_stats_too_short() {
        assert!(parse_packet_stats(&[]).is_err());
        assert!(parse_packet_stats(&[0; 23]).is_err()); // need at least 24
    }

    #[test]
    fn test_parse_packet_stats_legacy_26_byte_frame_has_no_recv_errors() {
        let mut data = Vec::new();
        data.extend_from_slice(&1000u32.to_le_bytes()); // recv
        data.extend_from_slice(&500u32.to_le_bytes()); // sent
        data.extend_from_slice(&100u32.to_le_bytes()); // flood_tx
        data.extend_from_slice(&400u32.to_le_bytes()); // direct_tx
        data.extend_from_slice(&200u32.to_le_bytes()); // flood_rx
        data.extend_from_slice(&800u32.to_le_bytes()); // direct_rx
        assert_eq!(data.len(), 24);

        let stats = parse_packet_stats(&data).unwrap();
        assert_eq!(stats.recv, 1000);
        assert_eq!(stats.sent, 500);
        assert_eq!(stats.flood_tx, 100);
        assert_eq!(stats.direct_tx, 400);
        assert_eq!(stats.flood_rx, 200);
        assert_eq!(stats.direct_rx, 800);
        assert_eq!(stats.recv_errors, None);
    }

    #[test]
    fn test_parse_packet_stats_30_byte_frame_has_recv_errors() {
        let mut data = Vec::new();
        data.extend_from_slice(&1000u32.to_le_bytes());
        data.extend_from_slice(&500u32.to_le_bytes());
        data.extend_from_slice(&100u32.to_le_bytes());
        data.extend_from_slice(&400u32.to_le_bytes());
        data.extend_from_slice(&200u32.to_le_bytes());
        data.extend_from_slice(&800u32.to_le_bytes());
        data.extend_from_slice(&5u32.to_le_bytes()); // recv_errors
        assert_eq!(data.len(), 28);

        let stats = parse_packet_stats(&data).unwrap();
        assert_eq!(stats.recv_errors, Some(5));
    }

    // ========== parse_advertisement tests ==========

    #[test]
    fn test_parse_advertisement_too_short() {
        assert!(parse_advertisement(&[]).is_err());
        assert!(parse_advertisement(&[0; 13]).is_err());
    }

    #[test]
    fn test_parse_advertisement_minimal() {
        let mut data = vec![0x01, 0x02, 0x03, 0x04, 0x05, 0x06]; // prefix
        let mut name = [0u8; 32];
        name[..4].copy_from_slice(b"Node");
        data.extend_from_slice(&name);
        // No lat/lon
        let advert = parse_advertisement(&data).unwrap();
        assert_eq!(advert.prefix, [0x01, 0x02, 0x03, 0x04, 0x05, 0x06]);
        assert_eq!(advert.name, "Node");
        assert_eq!(advert.lat, 0);
        assert_eq!(advert.lon, 0);
    }

    #[test]
    fn test_parse_advertisement_with_latlon() {
        let mut data = vec![0xAA; 6]; // prefix
        let mut name = [0u8; 32];
        name[..5].copy_from_slice(b"Hello");
        data.extend_from_slice(&name);
        data.extend_from_slice(&37774900i32.to_le_bytes()); // lat
        data.extend_from_slice(&(-122419400i32).to_le_bytes()); // lon
        let advert = parse_advertisement(&data).unwrap();
        assert_eq!(advert.name, "Hello");
        assert_eq!(advert.lat, 37774900);
        assert_eq!(advert.lon, -122419400);
    }

    // ========== parse_path_update tests ==========

    #[test]
    fn test_parse_path_update_too_short() {
        assert!(parse_path_update(&[]).is_err());
        assert!(parse_path_update(&[0; 6]).is_err());
    }

    #[test]
    fn test_parse_path_update_no_path() {
        let mut data = vec![0x11, 0x22, 0x33, 0x44, 0x55, 0x66]; // prefix
        data.push(0x03); // path_len = 3
        let update = parse_path_update(&data).unwrap();
        assert_eq!(update.prefix, [0x11, 0x22, 0x33, 0x44, 0x55, 0x66]);
        assert_eq!(update.path_len, 3);
        assert!(update.path.is_empty());
    }

    #[test]
    fn test_parse_path_update_with_path() {
        let mut data = vec![0xAA; 6]; // prefix
        data.push(0x02); // path_len = 2
        data.extend_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]);
        let update = parse_path_update(&data).unwrap();
        assert_eq!(update.path_len, 2);
        assert_eq!(update.path, vec![0xDE, 0xAD, 0xBE, 0xEF]);
    }

    #[test]
    fn test_parse_path_update_negative_path_len() {
        let mut data = vec![0xBB; 6]; // prefix
        data.push(0xFF); // path_len as i8 = -1 (flood)
        let update = parse_path_update(&data).unwrap();
        assert_eq!(update.path_len, -1);
    }

    // ========== parse_trace_data tests ==========

    #[test]
    fn test_parse_trace_data_empty() {
        let trace = parse_trace_data(&[]);
        assert!(trace.hops.is_empty());
    }

    #[test]
    fn test_parse_trace_data_single_hop() {
        let mut data = vec![0x01, 0x02, 0x03, 0x04, 0x05, 0x06]; // prefix
        data.push(40); // snr_raw = 40, snr = 10.0
        let trace = parse_trace_data(&data);
        assert_eq!(trace.hops.len(), 1);
        assert_eq!(trace.hops[0].prefix, [0x01, 0x02, 0x03, 0x04, 0x05, 0x06]);
        assert_eq!(trace.hops[0].snr, 10.0);
    }

    #[test]
    fn test_parse_trace_data_multiple_hops() {
        let mut data = Vec::new();
        data.extend_from_slice(&[0xAA; 6]);
        data.push(20); // snr = 5.0
        data.extend_from_slice(&[0xBB; 6]);
        data.push((-8i8) as u8); // snr = -2.0
        let trace = parse_trace_data(&data);
        assert_eq!(trace.hops.len(), 2);
        assert_eq!(trace.hops[0].snr, 5.0);
        assert_eq!(trace.hops[1].snr, -2.0);
    }

    #[test]
    fn test_parse_trace_data_trailing_bytes_ignored() {
        // 7 bytes for one hop + 3 trailing bytes (not enough for another hop)
        let mut data = vec![0xCC; 6];
        data.push(0);
        data.extend_from_slice(&[0xFF, 0xFF, 0xFF]); // trailing
        let trace = parse_trace_data(&data);
        assert_eq!(trace.hops.len(), 1);
    }

    // ========== parse_discover_response tests ==========

    #[test]
    fn test_parse_discover_response_empty() {
        let entries = parse_discover_response(&[]);
        assert!(entries.is_empty());
    }

    #[test]
    fn test_parse_discover_response_single_entry() {
        let mut data = vec![0xAA; 32]; // pubkey
        let mut name = [0u8; 32];
        name[..5].copy_from_slice(b"Peer1");
        data.extend_from_slice(&name);
        let entries = parse_discover_response(&data);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].pubkey, vec![0xAA; 32]);
        assert_eq!(entries[0].name, "Peer1");
    }

    #[test]
    fn test_parse_discover_response_multiple_entries() {
        let mut data = Vec::new();
        // Entry 1
        data.extend_from_slice(&[0x11; 32]);
        let mut name1 = [0u8; 32];
        name1[..2].copy_from_slice(b"A1");
        data.extend_from_slice(&name1);
        // Entry 2
        data.extend_from_slice(&[0x22; 32]);
        let mut name2 = [0u8; 32];
        name2[..2].copy_from_slice(b"B2");
        data.extend_from_slice(&name2);
        let entries = parse_discover_response(&data);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "A1");
        assert_eq!(entries[1].name, "B2");
    }

    #[test]
    fn test_parse_discover_response_too_short_entry_ignored() {
        // Less than 38 bytes, so no entries parsed
        let data = vec![0u8; 37];
        let entries = parse_discover_response(&data);
        assert!(entries.is_empty());
    }

    // ========== parse_status_response tests ==========

    #[test]
    fn test_parse_status_response_too_short() {
        assert!(parse_status_response(&[]).is_err());
        assert!(parse_status_response(&[0; 57]).is_err());
    }

    #[test]
    fn test_parse_status_response_valid() {
        // Build a valid StatusResponse payload: 6-byte prefix + status data
        let mut data = vec![0x11, 0x22, 0x33, 0x44, 0x55, 0x66]; // sender prefix
                                                                 // Status data: battery_mv(2) + tx_queue(2) + noise(2) + rssi(2) + recv(4)
                                                                 // + sent(4) + airtime(4) + uptime(4) + flood(4) + direct(4) + snr(1)
                                                                 // + dup(4) + rx_airtime(4) + padding... = at least 52 bytes
        let mut status_data = vec![0u8; 52];
        // battery_mv = 3700 at offset 0
        status_data[0..2].copy_from_slice(&3700u16.to_le_bytes());
        data.extend_from_slice(&status_data);
        let frame = parse_status_response(&data).unwrap();
        assert_eq!(frame.sender_prefix, [0x11, 0x22, 0x33, 0x44, 0x55, 0x66]);
        assert_eq!(frame.status.battery_mv, 3700);
    }

    // ========== parse_telemetry_response tests ==========

    #[test]
    fn test_parse_telemetry_response_too_short() {
        assert!(parse_telemetry_response(&[]).is_err());
        assert!(parse_telemetry_response(&[0; 3]).is_err());
    }

    #[test]
    fn test_parse_telemetry_response_tag_only() {
        let data = [0xAA, 0xBB, 0xCC, 0xDD];
        let frame = parse_telemetry_response(&data).unwrap();
        assert_eq!(frame.tag, [0xAA, 0xBB, 0xCC, 0xDD]);
        assert!(frame.data.is_empty());
    }

    #[test]
    fn test_parse_telemetry_response_with_data() {
        let mut data = vec![0x01, 0x02, 0x03, 0x04]; // tag
        data.extend_from_slice(&[0x10, 0x20, 0x30]); // LPP data
        let frame = parse_telemetry_response(&data).unwrap();
        assert_eq!(frame.tag, [0x01, 0x02, 0x03, 0x04]);
        assert_eq!(frame.data, vec![0x10, 0x20, 0x30]);
    }

    // ========== parse_custom_vars tests ==========

    #[test]
    fn test_parse_custom_vars_empty() {
        let vars = parse_custom_vars(&[]);
        assert!(vars.is_empty());
    }

    #[test]
    fn test_parse_custom_vars_single() {
        let vars = parse_custom_vars(b"key=value");
        assert_eq!(vars.get("key").unwrap(), "value");
    }

    #[test]
    fn test_parse_custom_vars_multiple() {
        let vars = parse_custom_vars(b"a=1\nb=2\nc=3");
        assert_eq!(vars.len(), 3);
        assert_eq!(vars.get("a").unwrap(), "1");
        assert_eq!(vars.get("b").unwrap(), "2");
        assert_eq!(vars.get("c").unwrap(), "3");
    }

    #[test]
    fn test_parse_custom_vars_no_equals_skipped() {
        let vars = parse_custom_vars(b"good=yes\nbadline\nalso=ok");
        assert_eq!(vars.len(), 2);
        assert_eq!(vars.get("good").unwrap(), "yes");
        assert_eq!(vars.get("also").unwrap(), "ok");
    }

    // ========== parse_binary_response_frame tests ==========

    #[test]
    fn test_parse_binary_response_frame_too_short() {
        assert!(parse_binary_response_frame(&[]).is_err());
        assert!(parse_binary_response_frame(&[0; 4]).is_err());
    }

    #[test]
    fn test_parse_binary_response_frame_minimal() {
        // subtype(1) + tag(4) = 5 bytes, no data
        let data = [0xFF, 0x01, 0x02, 0x03, 0x04];
        let frame = parse_binary_response_frame(&data).unwrap();
        assert_eq!(frame.tag, [0x01, 0x02, 0x03, 0x04]);
        assert!(frame.data.is_empty());
    }

    #[test]
    fn test_parse_binary_response_frame_with_data() {
        let mut data = vec![0x00]; // subtype (skipped)
        data.extend_from_slice(&[0xAA, 0xBB, 0xCC, 0xDD]); // tag
        data.extend_from_slice(&[0x11, 0x22, 0x33]); // response data
        let frame = parse_binary_response_frame(&data).unwrap();
        assert_eq!(frame.tag, [0xAA, 0xBB, 0xCC, 0xDD]);
        assert_eq!(frame.data, vec![0x11, 0x22, 0x33]);
    }

    #[test]
    fn test_parse_binary_response_frame_constants() {
        assert_eq!(BINARY_RESP_TAG_OFFSET, 1);
        assert_eq!(BINARY_RESP_DATA_OFFSET, 5);
        assert_eq!(BINARY_RESP_MIN_LEN, 5);
    }

    // ========== parse_contact_end_timestamp tests ==========

    #[test]
    fn test_parse_contact_end_timestamp_empty() {
        assert_eq!(parse_contact_end_timestamp(&[]).unwrap(), None);
    }

    #[test]
    fn test_parse_contact_end_timestamp_too_short() {
        assert_eq!(
            parse_contact_end_timestamp(&[0x01, 0x02, 0x03]).unwrap(),
            None
        );
    }

    #[test]
    fn test_parse_contact_end_timestamp_valid() {
        let data = 1700000000u32.to_le_bytes();
        assert_eq!(
            parse_contact_end_timestamp(&data).unwrap(),
            Some(1700000000)
        );
    }

    #[test]
    fn test_parse_contact_end_timestamp_with_extra_bytes() {
        let mut data = 42u32.to_le_bytes().to_vec();
        data.extend_from_slice(&[0xFF, 0xFF]); // extra trailing bytes
        assert_eq!(parse_contact_end_timestamp(&data).unwrap(), Some(42));
    }

    // ========== parse_current_time tests ==========

    #[test]
    fn test_parse_current_time_too_short() {
        assert!(parse_current_time(&[]).is_err());
        assert!(parse_current_time(&[0x01, 0x02, 0x03]).is_err());
    }

    #[test]
    fn test_parse_current_time_valid() {
        let data = 1700000000u32.to_le_bytes();
        assert_eq!(parse_current_time(&data).unwrap(), 1700000000);
    }

    // ========== parse_private_key tests ==========

    #[test]
    fn test_parse_private_key_too_short() {
        assert!(parse_private_key(&[]).is_err());
        assert!(parse_private_key(&[0; 63]).is_err());
    }

    #[test]
    fn test_parse_private_key_valid() {
        let data = [0xAA; 64];
        assert_eq!(parse_private_key(&data).unwrap(), [0xAA; 64]);
    }

    // ========== parse_sign_start tests ==========

    #[test]
    fn test_parse_sign_start_too_short() {
        assert!(parse_sign_start(&[]).is_err());
        assert!(parse_sign_start(&[0x01, 0x02, 0x03]).is_err());
    }

    #[test]
    fn test_parse_sign_start_valid() {
        let data = 4096u32.to_le_bytes();
        assert_eq!(parse_sign_start(&data).unwrap(), 4096);
    }

    // ========== parse_ack tests ==========

    #[test]
    fn test_parse_ack_too_short() {
        assert!(parse_ack(&[]).is_err());
        assert!(parse_ack(&[0x01, 0x02, 0x03]).is_err());
    }

    #[test]
    fn test_parse_ack_valid() {
        let data = [0x11, 0x22, 0x33, 0x44];
        assert_eq!(parse_ack(&data).unwrap(), [0x11, 0x22, 0x33, 0x44]);
    }

    // ========== parse_path_discovery_response tests ==========

    #[test]
    fn test_parse_path_discovery_response_too_short() {
        assert!(parse_path_discovery_response(&[]).is_err());
        assert!(parse_path_discovery_response(&[0; 7]).is_err()); // need at least 9
    }

    #[test]
    fn test_parse_path_discovery_response_no_paths() {
        let mut data = vec![0x00]; // reserved
        data.extend_from_slice(&[0x11, 0x22, 0x33, 0x44, 0x55, 0x66]); // pubkey prefix
        data.push(0x00); // out_path: hash_len=1, path_len=0
        data.push(0x00); // in_path: hash_len=1, path_len=0

        let resp = parse_path_discovery_response(&data).unwrap();
        assert_eq!(resp.pubkey_prefix, [0x11, 0x22, 0x33, 0x44, 0x55, 0x66]);
        assert_eq!(resp.out_path_len, 0);
        assert_eq!(resp.out_path_hash_len, 1);
        assert!(resp.out_path.is_empty());
        assert_eq!(resp.in_path_len, 0);
        assert_eq!(resp.in_path_hash_len, 1);
        assert!(resp.in_path.is_empty());
    }

    #[test]
    fn test_parse_path_discovery_response_with_paths() {
        let mut data = vec![0x00]; // reserved
        data.extend_from_slice(&[0xAA; 6]); // pubkey prefix
                                            // out_path: hash_len=2 (bits 6-7 = 0b01 -> +1 = 2), path_len=2
        data.push(0b01_000010);
        data.extend_from_slice(&[0x01, 0x02, 0x03, 0x04]); // 2 hops * 2 bytes = 4
                                                           // in_path: hash_len=1 (bits 6-7 = 0b00 -> +1 = 1), path_len=3
        data.push(0b00_000011);
        data.extend_from_slice(&[0xA1, 0xA2, 0xA3]); // 3 hops * 1 byte = 3

        let resp = parse_path_discovery_response(&data).unwrap();
        assert_eq!(resp.out_path_len, 2);
        assert_eq!(resp.out_path_hash_len, 2);
        assert_eq!(resp.out_path, vec![0x01, 0x02, 0x03, 0x04]);
        assert_eq!(resp.in_path_len, 3);
        assert_eq!(resp.in_path_hash_len, 1);
        assert_eq!(resp.in_path, vec![0xA1, 0xA2, 0xA3]);
    }

    #[test]
    fn test_parse_path_discovery_response_outbound_truncated() {
        let mut data = vec![0x00]; // reserved
        data.extend_from_slice(&[0xBB; 6]); // pubkey prefix
                                            // out_path: hash_len=1, path_len=5 (needs 5 bytes)
        data.push(0b00_000101);
        data.extend_from_slice(&[0x01, 0x02]); // only 2 bytes, need 5

        assert!(parse_path_discovery_response(&data).is_err());
    }

    #[test]
    fn test_parse_path_discovery_response_inbound_truncated() {
        let mut data = vec![0x00]; // reserved
        data.extend_from_slice(&[0xCC; 6]); // pubkey prefix
        data.push(0x00); // out_path: no hops
                         // in_path: hash_len=1, path_len=2 (needs 2 bytes)
        data.push(0b00_000010);
        data.push(0xDD); // only 1 byte, need 2

        assert!(parse_path_discovery_response(&data).is_err());
    }
}
