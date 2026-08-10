//! Packet types and protocol constants for MeshCore communication

/// Packet type identifiers received from the device
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum PacketType {
    // Command responses (0-31)
    Ok = 0,
    Error = 1,
    ContactStart = 2,
    Contact = 3,
    ContactEnd = 4,
    SelfInfo = 5,
    MsgSent = 6,
    ContactMsgRecv = 7,
    ChannelMsgRecv = 8,
    CurrentTime = 9,
    NoMoreMsgs = 10,
    ContactUri = 11,
    Battery = 12,
    DeviceInfo = 13,
    PrivateKey = 14,
    Disabled = 15,
    ContactMsgRecvV3 = 16,
    ChannelMsgRecvV3 = 17,
    ChannelInfo = 18,
    SignStart = 19,
    Signature = 20,
    CustomVars = 21,
    Stats = 24,
    AutoaddConfig = 25,

    // Binary/Control (50-55)
    BinaryReq = 50,
    FactoryReset = 51,
    PathDiscovery = 52,
    SetFloodScope = 54,
    SendControlData = 55,

    // Push notifications (0x80-0x8F)
    Advertisement = 0x80,
    PathUpdate = 0x81,
    Ack = 0x82,
    MessagesWaiting = 0x83,
    RawData = 0x84,
    LoginSuccess = 0x85,
    LoginFailed = 0x86,
    StatusResponse = 0x87,
    LogData = 0x88,
    TraceData = 0x89,
    PushCodeNewAdvert = 0x8A,
    TelemetryResponse = 0x8B,
    BinaryResponse = 0x8C,
    PathDiscoveryResponse = 0x8D,
    ControlData = 0x8E,
    AdvertResponse = 0x8F,

    /// Unknown packet type
    Unknown = 0xFF,
}

impl From<u8> for PacketType {
    fn from(value: u8) -> Self {
        match value {
            0 => PacketType::Ok,
            1 => PacketType::Error,
            2 => PacketType::ContactStart,
            3 => PacketType::Contact,
            4 => PacketType::ContactEnd,
            5 => PacketType::SelfInfo,
            6 => PacketType::MsgSent,
            7 => PacketType::ContactMsgRecv,
            8 => PacketType::ChannelMsgRecv,
            9 => PacketType::CurrentTime,
            10 => PacketType::NoMoreMsgs,
            11 => PacketType::ContactUri,
            12 => PacketType::Battery,
            13 => PacketType::DeviceInfo,
            14 => PacketType::PrivateKey,
            15 => PacketType::Disabled,
            16 => PacketType::ContactMsgRecvV3,
            17 => PacketType::ChannelMsgRecvV3,
            18 => PacketType::ChannelInfo,
            19 => PacketType::SignStart,
            20 => PacketType::Signature,
            21 => PacketType::CustomVars,
            24 => PacketType::Stats,
            25 => PacketType::AutoaddConfig,
            50 => PacketType::BinaryReq,
            51 => PacketType::FactoryReset,
            52 => PacketType::PathDiscovery,
            54 => PacketType::SetFloodScope,
            55 => PacketType::SendControlData,
            0x80 => PacketType::Advertisement,
            0x81 => PacketType::PathUpdate,
            0x82 => PacketType::Ack,
            0x83 => PacketType::MessagesWaiting,
            0x84 => PacketType::RawData,
            0x85 => PacketType::LoginSuccess,
            0x86 => PacketType::LoginFailed,
            0x87 => PacketType::StatusResponse,
            0x88 => PacketType::LogData,
            0x89 => PacketType::TraceData,
            0x8A => PacketType::PushCodeNewAdvert,
            0x8B => PacketType::TelemetryResponse,
            0x8C => PacketType::BinaryResponse,
            0x8D => PacketType::PathDiscoveryResponse,
            0x8E => PacketType::ControlData,
            0x8F => PacketType::AdvertResponse,
            _ => PacketType::Unknown,
        }
    }
}

/// Binary request types for the binary protocol
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum BinaryReqType {
    /// Device status query
    Status = 0x01,
    /// Keepalive/heartbeat
    KeepAlive = 0x02,
    /// Sensor telemetry data
    Telemetry = 0x03,
    /// Min/Max/Avg historical data
    Mma = 0x04,
    /// Access Control Lists
    Acl = 0x05,
    /// Network neighbor discovery
    Neighbours = 0x06,
}

impl From<u8> for BinaryReqType {
    fn from(value: u8) -> Self {
        match value {
            0x01 => BinaryReqType::Status,
            0x02 => BinaryReqType::KeepAlive,
            0x03 => BinaryReqType::Telemetry,
            0x04 => BinaryReqType::Mma,
            0x05 => BinaryReqType::Acl,
            0x06 => BinaryReqType::Neighbours,
            _ => BinaryReqType::Status, // Default
        }
    }
}

/// Anonymous request types (for remote queries)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum AnonReqType {
    /// Regional information
    Regions = 0x01,
    /// Device owner info
    Owner = 0x02,
    /// Remote clock (basic telemetry)
    Basic = 0x03,
}

impl From<u8> for AnonReqType {
    fn from(value: u8) -> Self {
        match value {
            0x01 => AnonReqType::Regions,
            0x02 => AnonReqType::Owner,
            0x03 => AnonReqType::Basic,
            _ => AnonReqType::Basic,
        }
    }
}

/// Control packet types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum ControlType {
    /// Node discovery request
    NodeDiscoverReq = 0x80,
    /// Node discovery response
    NodeDiscoverResp = 0x90,
}

impl From<u8> for ControlType {
    fn from(value: u8) -> Self {
        match value {
            0x80 => ControlType::NodeDiscoverReq,
            0x90 => ControlType::NodeDiscoverResp,
            _ => ControlType::NodeDiscoverReq,
        }
    }
}

/// Routing strategy of a MeshCore over-the-air packet, encoded in bits 0-1
/// of the packet header byte carried inside raw packet captures (see
/// [`crate::events::MeshPacketHeader`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum RouteType {
    /// Flood-routed packet that also carries a transport code
    TransportFlood = 0,
    /// Flood-routed packet
    Flood = 1,
    /// Directly-routed packet along an explicit path
    Direct = 2,
    /// Directly-routed packet that also carries a transport code
    TransportDirect = 3,
}

impl From<u8> for RouteType {
    fn from(value: u8) -> Self {
        match value & 0x03 {
            0 => RouteType::TransportFlood,
            1 => RouteType::Flood,
            2 => RouteType::Direct,
            _ => RouteType::TransportDirect,
        }
    }
}

/// Payload type of a MeshCore over-the-air packet, encoded in bits 2-5 of
/// the packet header byte carried inside raw packet captures (see
/// [`crate::events::MeshPacketHeader`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum PayloadType {
    /// Request payload
    Req = 0,
    /// Response payload
    Response = 1,
    /// Direct text message
    TextMsg = 2,
    /// Acknowledgement
    Ack = 3,
    /// Node advertisement (broadcast identity)
    Advert = 4,
    /// Group/channel text message
    GroupText = 5,
    /// Group/channel data
    GroupData = 6,
    /// Anonymous request
    AnonReq = 7,
    /// Path/route information
    Path = 8,
    /// Trace packet
    Trace = 9,
    /// Multipart message fragment
    Multipart = 10,
    /// Control packet
    Control = 11,
    /// Unrecognized payload type
    Unknown = 0xFF,
}

impl From<u8> for PayloadType {
    fn from(value: u8) -> Self {
        match value & 0x0F {
            0 => PayloadType::Req,
            1 => PayloadType::Response,
            2 => PayloadType::TextMsg,
            3 => PayloadType::Ack,
            4 => PayloadType::Advert,
            5 => PayloadType::GroupText,
            6 => PayloadType::GroupData,
            7 => PayloadType::AnonReq,
            8 => PayloadType::Path,
            9 => PayloadType::Trace,
            10 => PayloadType::Multipart,
            11 => PayloadType::Control,
            _ => PayloadType::Unknown,
        }
    }
}

/// Frame start marker byte (app → radio, commands)
pub const FRAME_START: u8 = 0x3c;

/// Frame start marker byte (radio → app, responses)
pub const FRAME_START_RESP: u8 = 0x3e;

/// Default serial baud rate
pub const DEFAULT_BAUD_RATE: u32 = 115200;

/// BLE Service UUID for MeshCore devices
pub const BLE_SERVICE_UUID: &str = "6E400001-B5A3-F393-E0A9-E50E24DCCA9E";

/// BLE RX Characteristic UUID (for writing to the device)
pub const BLE_RX_CHAR_UUID: &str = "6E400002-B5A3-F393-E0A9-E50E24DCCA9E";

/// BLE TX Characteristic UUID (for reading from the device)
pub const BLE_TX_CHAR_UUID: &str = "6E400003-B5A3-F393-E0A9-E50E24DCCA9E";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_packet_type_from_u8_command_responses() {
        assert_eq!(PacketType::from(0), PacketType::Ok);
        assert_eq!(PacketType::from(1), PacketType::Error);
        assert_eq!(PacketType::from(2), PacketType::ContactStart);
        assert_eq!(PacketType::from(3), PacketType::Contact);
        assert_eq!(PacketType::from(4), PacketType::ContactEnd);
        assert_eq!(PacketType::from(5), PacketType::SelfInfo);
        assert_eq!(PacketType::from(6), PacketType::MsgSent);
        assert_eq!(PacketType::from(7), PacketType::ContactMsgRecv);
        assert_eq!(PacketType::from(8), PacketType::ChannelMsgRecv);
        assert_eq!(PacketType::from(9), PacketType::CurrentTime);
        assert_eq!(PacketType::from(10), PacketType::NoMoreMsgs);
        assert_eq!(PacketType::from(11), PacketType::ContactUri);
        assert_eq!(PacketType::from(12), PacketType::Battery);
        assert_eq!(PacketType::from(13), PacketType::DeviceInfo);
        assert_eq!(PacketType::from(14), PacketType::PrivateKey);
        assert_eq!(PacketType::from(15), PacketType::Disabled);
        assert_eq!(PacketType::from(16), PacketType::ContactMsgRecvV3);
        assert_eq!(PacketType::from(17), PacketType::ChannelMsgRecvV3);
        assert_eq!(PacketType::from(18), PacketType::ChannelInfo);
        assert_eq!(PacketType::from(19), PacketType::SignStart);
        assert_eq!(PacketType::from(20), PacketType::Signature);
        assert_eq!(PacketType::from(21), PacketType::CustomVars);
        assert_eq!(PacketType::from(24), PacketType::Stats);
        assert_eq!(PacketType::from(25), PacketType::AutoaddConfig);
    }

    #[test]
    fn test_packet_type_from_u8_binary_control() {
        assert_eq!(PacketType::from(50), PacketType::BinaryReq);
        assert_eq!(PacketType::from(51), PacketType::FactoryReset);
        assert_eq!(PacketType::from(52), PacketType::PathDiscovery);
        assert_eq!(PacketType::from(54), PacketType::SetFloodScope);
        assert_eq!(PacketType::from(55), PacketType::SendControlData);
    }

    #[test]
    fn test_packet_type_from_u8_push_notifications() {
        assert_eq!(PacketType::from(0x80), PacketType::Advertisement);
        assert_eq!(PacketType::from(0x81), PacketType::PathUpdate);
        assert_eq!(PacketType::from(0x82), PacketType::Ack);
        assert_eq!(PacketType::from(0x83), PacketType::MessagesWaiting);
        assert_eq!(PacketType::from(0x84), PacketType::RawData);
        assert_eq!(PacketType::from(0x85), PacketType::LoginSuccess);
        assert_eq!(PacketType::from(0x86), PacketType::LoginFailed);
        assert_eq!(PacketType::from(0x87), PacketType::StatusResponse);
        assert_eq!(PacketType::from(0x88), PacketType::LogData);
        assert_eq!(PacketType::from(0x89), PacketType::TraceData);
        assert_eq!(PacketType::from(0x8A), PacketType::PushCodeNewAdvert);
        assert_eq!(PacketType::from(0x8B), PacketType::TelemetryResponse);
        assert_eq!(PacketType::from(0x8C), PacketType::BinaryResponse);
        assert_eq!(PacketType::from(0x8D), PacketType::PathDiscoveryResponse);
        assert_eq!(PacketType::from(0x8E), PacketType::ControlData);
        assert_eq!(PacketType::from(0x8F), PacketType::AdvertResponse);
    }

    #[test]
    fn test_packet_type_unknown() {
        assert_eq!(PacketType::from(99), PacketType::Unknown);
        assert_eq!(PacketType::from(0xFF), PacketType::Unknown);
        assert_eq!(PacketType::from(100), PacketType::Unknown);
    }

    #[test]
    fn test_binary_req_type_from_u8() {
        assert_eq!(BinaryReqType::from(0x01), BinaryReqType::Status);
        assert_eq!(BinaryReqType::from(0x02), BinaryReqType::KeepAlive);
        assert_eq!(BinaryReqType::from(0x03), BinaryReqType::Telemetry);
        assert_eq!(BinaryReqType::from(0x04), BinaryReqType::Mma);
        assert_eq!(BinaryReqType::from(0x05), BinaryReqType::Acl);
        assert_eq!(BinaryReqType::from(0x06), BinaryReqType::Neighbours);
        // Unknown defaults to Status
        assert_eq!(BinaryReqType::from(0xFF), BinaryReqType::Status);
    }

    #[test]
    fn test_anon_req_type_from_u8() {
        assert_eq!(AnonReqType::from(0x01), AnonReqType::Regions);
        assert_eq!(AnonReqType::from(0x02), AnonReqType::Owner);
        assert_eq!(AnonReqType::from(0x03), AnonReqType::Basic);
        // Unknown defaults to Basic
        assert_eq!(AnonReqType::from(0xFF), AnonReqType::Basic);
    }

    #[test]
    fn test_control_type_from_u8() {
        assert_eq!(ControlType::from(0x80), ControlType::NodeDiscoverReq);
        assert_eq!(ControlType::from(0x90), ControlType::NodeDiscoverResp);
        // Unknown defaults to NodeDiscoverReq
        assert_eq!(ControlType::from(0xFF), ControlType::NodeDiscoverReq);
    }

    #[test]
    fn test_constants() {
        assert_eq!(FRAME_START, 0x3c);
        assert_eq!(FRAME_START_RESP, 0x3e);
        assert_eq!(FRAME_START_RESP, b'>');
        assert_eq!(DEFAULT_BAUD_RATE, 115200);
        assert_eq!(BLE_SERVICE_UUID, "6E400001-B5A3-F393-E0A9-E50E24DCCA9E");
        assert_eq!(BLE_RX_CHAR_UUID, "6E400002-B5A3-F393-E0A9-E50E24DCCA9E");
        assert_eq!(BLE_TX_CHAR_UUID, "6E400003-B5A3-F393-E0A9-E50E24DCCA9E");
    }

    #[test]
    fn test_packet_type_repr() {
        // Verify repr values match
        assert_eq!(PacketType::Ok as u8, 0);
        assert_eq!(PacketType::Error as u8, 1);
        assert_eq!(PacketType::Advertisement as u8, 0x80);
    }

    #[test]
    fn test_binary_req_type_repr() {
        assert_eq!(BinaryReqType::Status as u8, 0x01);
        assert_eq!(BinaryReqType::KeepAlive as u8, 0x02);
        assert_eq!(BinaryReqType::Telemetry as u8, 0x03);
    }

    #[test]
    fn test_packet_type_debug() {
        let packet = PacketType::Ok;
        assert_eq!(format!("{:?}", packet), "Ok");
    }

    #[test]
    fn test_packet_type_clone_eq() {
        let p1 = PacketType::SelfInfo;
        let p2 = p1;
        assert_eq!(p1, p2);
    }

    #[test]
    fn test_route_type_from_u8() {
        assert_eq!(RouteType::from(0), RouteType::TransportFlood);
        assert_eq!(RouteType::from(1), RouteType::Flood);
        assert_eq!(RouteType::from(2), RouteType::Direct);
        assert_eq!(RouteType::from(3), RouteType::TransportDirect);
    }

    #[test]
    fn test_route_type_from_u8_masks_upper_bits() {
        // Only bits 0-1 are significant; upper bits must be ignored.
        assert_eq!(RouteType::from(0b1111_1100), RouteType::TransportFlood);
        assert_eq!(RouteType::from(0b1111_1101), RouteType::Flood);
    }

    #[test]
    fn test_payload_type_from_u8() {
        assert_eq!(PayloadType::from(0), PayloadType::Req);
        assert_eq!(PayloadType::from(1), PayloadType::Response);
        assert_eq!(PayloadType::from(2), PayloadType::TextMsg);
        assert_eq!(PayloadType::from(3), PayloadType::Ack);
        assert_eq!(PayloadType::from(4), PayloadType::Advert);
        assert_eq!(PayloadType::from(5), PayloadType::GroupText);
        assert_eq!(PayloadType::from(6), PayloadType::GroupData);
        assert_eq!(PayloadType::from(7), PayloadType::AnonReq);
        assert_eq!(PayloadType::from(8), PayloadType::Path);
        assert_eq!(PayloadType::from(9), PayloadType::Trace);
        assert_eq!(PayloadType::from(10), PayloadType::Multipart);
        assert_eq!(PayloadType::from(11), PayloadType::Control);
    }

    #[test]
    fn test_payload_type_from_u8_unknown() {
        assert_eq!(PayloadType::from(12), PayloadType::Unknown);
        assert_eq!(PayloadType::from(15), PayloadType::Unknown);
    }

    #[test]
    fn test_payload_type_from_u8_masks_upper_bits() {
        // Only bits 0-3 are significant; upper bits must be ignored.
        assert_eq!(PayloadType::from(0b1111_0100), PayloadType::Advert);
    }
}
