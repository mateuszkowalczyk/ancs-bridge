use anyhow::{bail, Context, Result};

pub const MAX_RESPONSE_BYTES: usize = 64 * 1024;

const COMMAND_GET_NOTIFICATION_ATTRIBUTES: u8 = 0;
const ATTRIBUTE_APP_IDENTIFIER: u8 = 0;
const ATTRIBUTE_TITLE: u8 = 1;
const ATTRIBUTE_MESSAGE: u8 = 3;
const TITLE_MAX_BYTES: u16 = 256;
const MESSAGE_MAX_BYTES: u16 = 2048;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EventKind {
    Added,
    Modified,
    Removed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NotificationEvent {
    pub kind: EventKind,
    pub flags: u8,
    pub category_id: u8,
    pub category_count: u8,
    pub uid: u32,
}

impl NotificationEvent {
    pub fn parse(value: &[u8]) -> Result<Self> {
        if value.len() != 8 {
            bail!("notification event must contain exactly 8 bytes");
        }
        let kind = match value[0] {
            0 => EventKind::Added,
            1 => EventKind::Modified,
            2 => EventKind::Removed,
            other => bail!("unsupported ANCS event ID {other}"),
        };
        let uid = u32::from_le_bytes(value[4..8].try_into().expect("length checked"));
        Ok(Self {
            kind,
            flags: value[1],
            category_id: value[2],
            category_count: value[3],
            uid,
        })
    }

    pub fn is_pre_existing(self) -> bool {
        self.flags & 0x04 != 0
    }
}

pub fn notification_attributes_request(uid: u32) -> Vec<u8> {
    let mut request = Vec::with_capacity(12);
    request.push(COMMAND_GET_NOTIFICATION_ATTRIBUTES);
    request.extend_from_slice(&uid.to_le_bytes());
    request.push(ATTRIBUTE_APP_IDENTIFIER);
    request.push(ATTRIBUTE_TITLE);
    request.extend_from_slice(&TITLE_MAX_BYTES.to_le_bytes());
    request.push(ATTRIBUTE_MESSAGE);
    request.extend_from_slice(&MESSAGE_MAX_BYTES.to_le_bytes());
    request
}

#[derive(Debug, Eq, PartialEq)]
pub struct NotificationAttributes {
    pub uid: u32,
    pub app_identifier: String,
    pub title: String,
    pub message: String,
}

#[derive(Default)]
pub struct ResponseAssembler {
    bytes: Vec<u8>,
}

impl ResponseAssembler {
    pub fn push(
        &mut self,
        fragment: &[u8],
        expected_uid: u32,
    ) -> Result<Option<NotificationAttributes>> {
        let new_len = self
            .bytes
            .len()
            .checked_add(fragment.len())
            .context("ANCS response length overflow")?;
        if new_len > MAX_RESPONSE_BYTES {
            bail!("ANCS response exceeded the 64 KiB spike limit");
        }
        self.bytes.extend_from_slice(fragment);
        parse_response(&self.bytes, expected_uid)
    }
}

fn parse_response(bytes: &[u8], expected_uid: u32) -> Result<Option<NotificationAttributes>> {
    if bytes.len() < 5 {
        return Ok(None);
    }
    if bytes[0] != COMMAND_GET_NOTIFICATION_ATTRIBUTES {
        bail!("unexpected ANCS response command ID {}", bytes[0]);
    }
    let uid = u32::from_le_bytes(bytes[1..5].try_into().expect("length checked"));
    if uid != expected_uid {
        bail!("ANCS response UID does not match the outstanding request");
    }

    let mut cursor = 5;
    let Some(app_identifier) = parse_attribute(bytes, &mut cursor, ATTRIBUTE_APP_IDENTIFIER)?
    else {
        return Ok(None);
    };
    let Some(title) = parse_attribute(bytes, &mut cursor, ATTRIBUTE_TITLE)? else {
        return Ok(None);
    };
    let Some(message) = parse_attribute(bytes, &mut cursor, ATTRIBUTE_MESSAGE)? else {
        return Ok(None);
    };

    if cursor != bytes.len() {
        bail!("ANCS response contained unexpected trailing bytes");
    }

    Ok(Some(NotificationAttributes {
        uid,
        app_identifier,
        title,
        message,
    }))
}

fn parse_attribute(bytes: &[u8], cursor: &mut usize, expected_id: u8) -> Result<Option<String>> {
    if bytes.len().saturating_sub(*cursor) < 3 {
        return Ok(None);
    }
    let attribute_id = bytes[*cursor];
    if attribute_id != expected_id {
        bail!("unexpected ANCS attribute ID {attribute_id}; expected {expected_id}");
    }
    let len = u16::from_le_bytes([bytes[*cursor + 1], bytes[*cursor + 2]]) as usize;
    let value_start = *cursor + 3;
    let Some(value_end) = value_start.checked_add(len) else {
        bail!("ANCS attribute length overflow");
    };
    if value_end > bytes.len() {
        return Ok(None);
    }
    let value = std::str::from_utf8(&bytes[value_start..value_end])
        .context("ANCS attribute was not valid UTF-8")?
        .to_owned();
    *cursor = value_end;
    Ok(Some(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response(uid: u32) -> Vec<u8> {
        let mut bytes = vec![0];
        bytes.extend_from_slice(&uid.to_le_bytes());
        for (id, value) in [(0, "com.example"), (1, "Title"), (3, "Message")] {
            bytes.push(id);
            bytes.extend_from_slice(&(value.len() as u16).to_le_bytes());
            bytes.extend_from_slice(value.as_bytes());
        }
        bytes
    }

    #[test]
    fn parses_notification_event() {
        assert_eq!(
            NotificationEvent::parse(&[0, 4, 6, 2, 0x78, 0x56, 0x34, 0x12]).unwrap(),
            NotificationEvent {
                kind: EventKind::Added,
                flags: 4,
                category_id: 6,
                category_count: 2,
                uid: 0x1234_5678,
            }
        );
    }

    #[test]
    fn rejects_malformed_notification_events() {
        assert!(NotificationEvent::parse(&[0; 7]).is_err());
        assert!(NotificationEvent::parse(&[3, 0, 0, 0, 0, 0, 0, 0]).is_err());
    }

    #[test]
    fn builds_bounded_attribute_request() {
        assert_eq!(
            notification_attributes_request(0x1234_5678),
            vec![0, 0x78, 0x56, 0x34, 0x12, 0, 1, 0, 1, 3, 0, 8]
        );
    }

    #[test]
    fn reassembles_response_split_at_every_boundary() {
        let uid = 42;
        let complete = response(uid);
        for split in 0..=complete.len() {
            let mut assembler = ResponseAssembler::default();
            let first = assembler.push(&complete[..split], uid).unwrap();
            if split < complete.len() {
                assert!(first.is_none());
            }
            let parsed = assembler.push(&complete[split..], uid).unwrap().unwrap();
            assert_eq!(parsed.app_identifier, "com.example");
            assert_eq!(parsed.title, "Title");
            assert_eq!(parsed.message, "Message");
        }
    }

    #[test]
    fn rejects_invalid_utf8_wrong_ids_and_trailing_data() {
        let uid = 7;
        let mut invalid_utf8 = response(uid);
        *invalid_utf8.last_mut().unwrap() = 0xff;
        assert!(ResponseAssembler::default()
            .push(&invalid_utf8, uid)
            .is_err());

        let mut wrong_attribute = response(uid);
        wrong_attribute[5] = 9;
        assert!(ResponseAssembler::default()
            .push(&wrong_attribute, uid)
            .is_err());

        let mut trailing = response(uid);
        trailing.push(0);
        assert!(ResponseAssembler::default().push(&trailing, uid).is_err());
    }

    #[test]
    fn enforces_response_size_cap() {
        let mut assembler = ResponseAssembler::default();
        assert!(assembler.push(&vec![0; MAX_RESPONSE_BYTES + 1], 0).is_err());
    }
}
