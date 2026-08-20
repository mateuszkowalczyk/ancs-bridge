use std::{collections::VecDeque, error::Error, fmt};

pub const TITLE_MAX_BYTES: u16 = 256;
pub const MESSAGE_MAX_BYTES: u16 = 2048;
pub const MAX_RESPONSE_BYTES: usize = 64 * 1024;

const GET_NOTIFICATION_ATTRIBUTES: u8 = 0;
const GET_APP_ATTRIBUTES: u8 = 1;
const APP_IDENTIFIER: u8 = 0;
const TITLE: u8 = 1;
const MESSAGE: u8 = 3;
const DISPLAY_NAME: u8 = 0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EventKind {
    Added,
    Modified,
    Removed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Category {
    Other,
    IncomingCall,
    MissedCall,
    Voicemail,
    Social,
    Schedule,
    Email,
    News,
    HealthAndFitness,
    BusinessAndFinance,
    Location,
    Entertainment,
    Reserved(u8),
}

impl TryFrom<u8> for Category {
    type Error = CodecError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        Ok(match value {
            0 => Self::Other,
            1 => Self::IncomingCall,
            2 => Self::MissedCall,
            3 => Self::Voicemail,
            4 => Self::Social,
            5 => Self::Schedule,
            6 => Self::Email,
            7 => Self::News,
            8 => Self::HealthAndFitness,
            9 => Self::BusinessAndFinance,
            10 => Self::Location,
            11 => Self::Entertainment,
            other => Self::Reserved(other),
        })
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct EventFlags(u8);

impl EventFlags {
    pub const SILENT: u8 = 1 << 0;
    pub const IMPORTANT: u8 = 1 << 1;
    pub const PRE_EXISTING: u8 = 1 << 2;
    pub const POSITIVE_ACTION: u8 = 1 << 3;
    pub const NEGATIVE_ACTION: u8 = 1 << 4;

    pub fn bits(self) -> u8 {
        self.0
    }

    pub fn is_pre_existing(self) -> bool {
        self.0 & Self::PRE_EXISTING != 0
    }
}

impl fmt::Debug for EventFlags {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "EventFlags(0x{:02x})", self.0)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NotificationEvent {
    pub kind: EventKind,
    pub flags: EventFlags,
    pub category: Category,
    pub category_count: u8,
    pub uid: u32,
}

impl NotificationEvent {
    pub fn parse(bytes: &[u8]) -> Result<Self, CodecError> {
        if bytes.len() != 8 {
            return Err(CodecError::InvalidEventLength(bytes.len()));
        }
        let kind = match bytes[0] {
            0 => EventKind::Added,
            1 => EventKind::Modified,
            2 => EventKind::Removed,
            other => return Err(CodecError::InvalidEvent(other)),
        };
        Ok(Self {
            kind,
            flags: EventFlags(bytes[1]),
            category: bytes[2].try_into()?,
            category_count: bytes[3],
            uid: u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WriteRequirement {
    WithResponse,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ControlPointRequest {
    pub bytes: Vec<u8>,
    pub write: WriteRequirement,
}

pub fn notification_attributes_request(uid: u32) -> ControlPointRequest {
    let mut bytes = Vec::with_capacity(13);
    bytes.push(GET_NOTIFICATION_ATTRIBUTES);
    bytes.extend_from_slice(&uid.to_le_bytes());
    bytes.push(APP_IDENTIFIER);
    bytes.push(TITLE);
    bytes.extend_from_slice(&TITLE_MAX_BYTES.to_le_bytes());
    bytes.push(MESSAGE);
    bytes.extend_from_slice(&MESSAGE_MAX_BYTES.to_le_bytes());
    ControlPointRequest {
        bytes,
        write: WriteRequirement::WithResponse,
    }
}

pub fn app_attributes_request(app_identifier: &str) -> Result<ControlPointRequest, CodecError> {
    if app_identifier.is_empty() || app_identifier.as_bytes().contains(&0) {
        return Err(CodecError::InvalidAppIdentifier);
    }
    let mut bytes = Vec::with_capacity(app_identifier.len() + 3);
    bytes.push(GET_APP_ATTRIBUTES);
    bytes.extend_from_slice(app_identifier.as_bytes());
    bytes.push(0);
    bytes.push(DISPLAY_NAME);
    Ok(ControlPointRequest {
        bytes,
        write: WriteRequirement::WithResponse,
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ResponseExpectation {
    Notification { uid: u32 },
    App { app_identifier: String },
}

#[derive(Eq, PartialEq)]
pub enum DecodedResponse {
    Notification(NotificationAttributes),
    App(AppAttributes),
}

impl fmt::Debug for DecodedResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Notification(value) => formatter
                .debug_struct("Notification")
                .field("uid", &value.uid)
                .finish_non_exhaustive(),
            Self::App(_) => formatter.debug_struct("App").finish_non_exhaustive(),
        }
    }
}

#[derive(Eq, PartialEq)]
pub struct NotificationAttributes {
    pub uid: u32,
    pub app_identifier: String,
    pub title: String,
    pub message: String,
}

#[derive(Eq, PartialEq)]
pub struct AppAttributes {
    pub app_identifier: String,
    pub display_name: String,
}

#[derive(Default)]
pub struct DataSourceDecoder {
    bytes: Vec<u8>,
    expected: VecDeque<ResponseExpectation>,
}

impl DataSourceDecoder {
    pub fn expect(&mut self, expected: ResponseExpectation) {
        self.expected.push_back(expected);
    }

    pub fn clear(&mut self) {
        self.bytes.clear();
        self.expected.clear();
    }

    pub fn buffered_len(&self) -> usize {
        self.bytes.len()
    }

    pub fn push(&mut self, fragment: &[u8]) -> Result<Vec<DecodedResponse>, CodecError> {
        let length = self
            .bytes
            .len()
            .checked_add(fragment.len())
            .ok_or(CodecError::Oversized)?;
        if length > MAX_RESPONSE_BYTES {
            self.clear();
            return Err(CodecError::Oversized);
        }
        self.bytes.extend_from_slice(fragment);
        let mut decoded = Vec::new();
        while let Some(expected) = self.expected.front() {
            let parsed = match expected {
                ResponseExpectation::Notification { uid } => parse_notification(&self.bytes, *uid)?,
                ResponseExpectation::App { app_identifier } => {
                    parse_app(&self.bytes, app_identifier)?
                }
            };
            let Some((value, consumed)) = parsed else {
                break;
            };
            self.bytes.drain(..consumed);
            self.expected.pop_front();
            decoded.push(value);
        }
        if self.expected.is_empty() && !self.bytes.is_empty() {
            self.clear();
            return Err(CodecError::UnexpectedTrailingData);
        }
        Ok(decoded)
    }
}

fn parse_notification(
    bytes: &[u8],
    expected_uid: u32,
) -> Result<Option<(DecodedResponse, usize)>, CodecError> {
    if bytes.len() < 5 {
        return Ok(None);
    }
    if bytes[0] != GET_NOTIFICATION_ATTRIBUTES {
        return Err(CodecError::UnexpectedCommand(bytes[0]));
    }
    let uid = u32::from_le_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]);
    if uid != expected_uid {
        return Err(CodecError::WrongUid { expected_uid, uid });
    }
    let mut cursor = 5;
    let Some(app_identifier) = parse_attribute(bytes, &mut cursor, APP_IDENTIFIER)? else {
        return Ok(None);
    };
    let Some(title) = parse_attribute(bytes, &mut cursor, TITLE)? else {
        return Ok(None);
    };
    let Some(message) = parse_attribute(bytes, &mut cursor, MESSAGE)? else {
        return Ok(None);
    };
    Ok(Some((
        DecodedResponse::Notification(NotificationAttributes {
            uid,
            app_identifier,
            title,
            message,
        }),
        cursor,
    )))
}

fn parse_app(
    bytes: &[u8],
    expected_app: &str,
) -> Result<Option<(DecodedResponse, usize)>, CodecError> {
    if bytes.is_empty() {
        return Ok(None);
    }
    if bytes[0] != GET_APP_ATTRIBUTES {
        return Err(CodecError::UnexpectedCommand(bytes[0]));
    }
    let Some(nul) = bytes[1..].iter().position(|byte| *byte == 0) else {
        return Ok(None);
    };
    let app_end = nul + 1;
    let app_identifier =
        std::str::from_utf8(&bytes[1..app_end]).map_err(|_| CodecError::InvalidUtf8)?;
    if app_identifier != expected_app {
        return Err(CodecError::WrongAppIdentifier);
    }
    let mut cursor = app_end + 1;
    let Some(display_name) = parse_attribute(bytes, &mut cursor, DISPLAY_NAME)? else {
        return Ok(None);
    };
    Ok(Some((
        DecodedResponse::App(AppAttributes {
            app_identifier: app_identifier.to_owned(),
            display_name,
        }),
        cursor,
    )))
}

fn parse_attribute(
    bytes: &[u8],
    cursor: &mut usize,
    expected_id: u8,
) -> Result<Option<String>, CodecError> {
    if bytes.len().saturating_sub(*cursor) < 3 {
        return Ok(None);
    }
    let id = bytes[*cursor];
    if id != expected_id {
        return Err(CodecError::InvalidAttribute {
            expected: expected_id,
            actual: id,
        });
    }
    let length = u16::from_le_bytes([bytes[*cursor + 1], bytes[*cursor + 2]]) as usize;
    let start = *cursor + 3;
    let end = start.checked_add(length).ok_or(CodecError::InvalidLength)?;
    if end > MAX_RESPONSE_BYTES {
        return Err(CodecError::InvalidLength);
    }
    if end > bytes.len() {
        return Ok(None);
    }
    let value = std::str::from_utf8(&bytes[start..end])
        .map_err(|_| CodecError::InvalidUtf8)?
        .to_owned();
    *cursor = end;
    Ok(Some(value))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CodecError {
    InvalidEventLength(usize),
    InvalidEvent(u8),
    InvalidAppIdentifier,
    UnexpectedCommand(u8),
    WrongUid { expected_uid: u32, uid: u32 },
    WrongAppIdentifier,
    InvalidAttribute { expected: u8, actual: u8 },
    InvalidLength,
    InvalidUtf8,
    UnexpectedTrailingData,
    Oversized,
}

impl fmt::Display for CodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "ANCS codec error: {self:?}")
    }
}

impl Error for CodecError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn attribute(id: u8, value: &[u8]) -> Vec<u8> {
        let mut bytes = vec![id];
        bytes.extend_from_slice(&(value.len() as u16).to_le_bytes());
        bytes.extend_from_slice(value);
        bytes
    }

    fn notification_response(uid: u32) -> Vec<u8> {
        let mut bytes = vec![0];
        bytes.extend_from_slice(&uid.to_le_bytes());
        bytes.extend(attribute(0, b"com.example"));
        bytes.extend(attribute(1, b"Title"));
        bytes.extend(attribute(3, b"Message"));
        bytes
    }

    fn app_response() -> Vec<u8> {
        let mut bytes = vec![1];
        bytes.extend_from_slice(b"com.example\0");
        bytes.extend(attribute(0, b"Example"));
        bytes
    }

    #[test]
    fn event_golden_vectors_and_validation() {
        for (raw, kind) in [
            (0, EventKind::Added),
            (1, EventKind::Modified),
            (2, EventKind::Removed),
        ] {
            let event =
                NotificationEvent::parse(&[raw, 0x1f, 11, 2, 0x78, 0x56, 0x34, 0x12]).unwrap();
            assert_eq!(event.kind, kind);
            assert_eq!(event.flags.bits(), 0x1f);
            assert_eq!(event.category, Category::Entertainment);
            assert_eq!(event.uid, 0x1234_5678);
        }
        assert!(matches!(
            NotificationEvent::parse(&[0; 7]),
            Err(CodecError::InvalidEventLength(7))
        ));
        assert!(matches!(
            NotificationEvent::parse(&[3, 0, 0, 0, 0, 0, 0, 0]),
            Err(CodecError::InvalidEvent(3))
        ));
        let future_flags = NotificationEvent::parse(&[0, 0x35, 0, 0, 0, 0, 0, 0]).unwrap();
        assert_eq!(future_flags.flags.bits(), 0x35);
        assert!(future_flags.flags.is_pre_existing());
        let future_category = NotificationEvent::parse(&[0, 0, 12, 0, 0, 0, 0, 0]).unwrap();
        assert_eq!(future_category.category, Category::Reserved(12));
    }

    #[test]
    fn command_golden_vectors_require_response() {
        assert_eq!(
            notification_attributes_request(0x1234_5678),
            ControlPointRequest {
                bytes: vec![0, 0x78, 0x56, 0x34, 0x12, 0, 1, 0, 1, 3, 0, 8],
                write: WriteRequirement::WithResponse,
            }
        );
        assert_eq!(
            app_attributes_request("com.example").unwrap(),
            ControlPointRequest {
                bytes: b"\x01com.example\0\0".to_vec(),
                write: WriteRequirement::WithResponse,
            }
        );
        assert!(app_attributes_request("").is_err());
        assert!(app_attributes_request("a\0b").is_err());
    }

    #[test]
    fn fragments_notification_at_every_boundary() {
        let response = notification_response(42);
        for split in 0..=response.len() {
            let mut decoder = DataSourceDecoder::default();
            decoder.expect(ResponseExpectation::Notification { uid: 42 });
            let first = decoder.push(&response[..split]).unwrap();
            if split < response.len() {
                assert!(first.is_empty());
                let values = decoder.push(&response[split..]).unwrap();
                assert_eq!(values.len(), 1);
            } else {
                assert_eq!(first.len(), 1);
            }
        }
    }

    #[test]
    fn decodes_combined_responses() {
        let mut decoder = DataSourceDecoder::default();
        decoder.expect(ResponseExpectation::Notification { uid: 42 });
        decoder.expect(ResponseExpectation::App {
            app_identifier: "com.example".into(),
        });
        let mut combined = notification_response(42);
        combined.extend(app_response());
        assert_eq!(decoder.push(&combined).unwrap().len(), 2);
        assert_eq!(decoder.buffered_len(), 0);
    }

    #[test]
    fn fragments_app_response_at_every_boundary() {
        let response = app_response();
        for split in 0..=response.len() {
            let mut decoder = DataSourceDecoder::default();
            decoder.expect(ResponseExpectation::App {
                app_identifier: "com.example".into(),
            });
            let first = decoder.push(&response[..split]).unwrap();
            if split < response.len() {
                assert!(first.is_empty());
                assert_eq!(decoder.push(&response[split..]).unwrap().len(), 1);
            } else {
                assert_eq!(first.len(), 1);
            }
        }
    }

    #[test]
    fn rejects_wrong_app_unknown_command_invalid_app_attribute_and_trailing_bytes() {
        let cases = [
            (
                app_response(),
                ResponseExpectation::App {
                    app_identifier: "wrong.app".into(),
                },
            ),
            (
                {
                    let mut value = app_response();
                    value[0] = 7;
                    value
                },
                ResponseExpectation::App {
                    app_identifier: "com.example".into(),
                },
            ),
            (
                {
                    let mut value = app_response();
                    value[b"\x01com.example\0".len()] = 9;
                    value
                },
                ResponseExpectation::App {
                    app_identifier: "com.example".into(),
                },
            ),
            (
                {
                    let mut value = app_response();
                    value.push(0);
                    value
                },
                ResponseExpectation::App {
                    app_identifier: "com.example".into(),
                },
            ),
        ];
        for (bytes, expected) in cases {
            let mut decoder = DataSourceDecoder::default();
            decoder.expect(expected);
            assert!(decoder.push(&bytes).is_err());
        }
    }

    #[test]
    fn rejects_malformed_and_bounded_input_without_panics() {
        let cases: Vec<(Vec<u8>, ResponseExpectation)> = vec![
            (
                vec![9, 0, 0, 0, 0],
                ResponseExpectation::Notification { uid: 0 },
            ),
            (
                notification_response(7),
                ResponseExpectation::Notification { uid: 8 },
            ),
            (
                {
                    let mut value = notification_response(7);
                    value[5] = 9;
                    value
                },
                ResponseExpectation::Notification { uid: 7 },
            ),
            (
                {
                    let mut value = notification_response(7);
                    *value.last_mut().unwrap() = 0xff;
                    value
                },
                ResponseExpectation::Notification { uid: 7 },
            ),
        ];
        for (bytes, expectation) in cases {
            let outcome = std::panic::catch_unwind(|| {
                let mut decoder = DataSourceDecoder::default();
                decoder.expect(expectation);
                decoder.push(&bytes)
            });
            assert!(outcome.is_ok());
            assert!(outcome.unwrap().is_err());
        }
        let mut decoder = DataSourceDecoder::default();
        decoder.expect(ResponseExpectation::Notification { uid: 0 });
        assert_eq!(
            decoder.push(&vec![0; MAX_RESPONSE_BYTES + 1]),
            Err(CodecError::Oversized)
        );
        assert_eq!(decoder.buffered_len(), 0);
    }

    #[test]
    fn truncated_and_invalid_length_remain_bounded() {
        let mut decoder = DataSourceDecoder::default();
        decoder.expect(ResponseExpectation::Notification { uid: 1 });
        let truncated = [0, 1, 0, 0, 0, 0, 32, 0];
        assert!(decoder.push(&truncated).unwrap().is_empty());
        assert_eq!(decoder.buffered_len(), truncated.len());
        assert_eq!(
            decoder.push(&vec![0; MAX_RESPONSE_BYTES]),
            Err(CodecError::Oversized)
        );
    }
}
