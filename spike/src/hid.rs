use bluer::{
    gatt::local::{
        Application, Characteristic, CharacteristicNotify, CharacteristicNotifyMethod,
        CharacteristicRead, CharacteristicWrite, CharacteristicWriteMethod, Descriptor,
        DescriptorRead, ReqError, Service,
    },
    Uuid,
};
use futures::FutureExt;
use std::sync::{Arc, Mutex};

pub const HID_SERVICE_UUID: Uuid = Uuid::from_u128(0x00001812_0000_1000_8000_00805f9b34fb);

const HID_INFORMATION_UUID: Uuid = Uuid::from_u128(0x00002a4a_0000_1000_8000_00805f9b34fb);
const REPORT_MAP_UUID: Uuid = Uuid::from_u128(0x00002a4b_0000_1000_8000_00805f9b34fb);
const HID_CONTROL_POINT_UUID: Uuid = Uuid::from_u128(0x00002a4c_0000_1000_8000_00805f9b34fb);
const REPORT_UUID: Uuid = Uuid::from_u128(0x00002a4d_0000_1000_8000_00805f9b34fb);
const PROTOCOL_MODE_UUID: Uuid = Uuid::from_u128(0x00002a4e_0000_1000_8000_00805f9b34fb);
const REPORT_REFERENCE_UUID: Uuid = Uuid::from_u128(0x00002908_0000_1000_8000_00805f9b34fb);

const HID_INFORMATION: &[u8] = &[0x11, 0x01, 0x00, 0x02];
const EMPTY_KEYBOARD_REPORT: &[u8] = &[0; 8];

// Report ID 1: modifiers, reserved byte, and six key slots. The spike exposes
// the input report but never emits a value or a keyboard event.
const KEYBOARD_REPORT_MAP: &[u8] = &[
    0x05, 0x01, 0x09, 0x06, 0xa1, 0x01, 0x85, 0x01, 0x05, 0x07, 0x19, 0xe0, 0x29, 0xe7, 0x15, 0x00,
    0x25, 0x01, 0x75, 0x01, 0x95, 0x08, 0x81, 0x02, 0x75, 0x08, 0x95, 0x01, 0x81, 0x01, 0x05, 0x07,
    0x19, 0x00, 0x29, 0x65, 0x15, 0x00, 0x25, 0x65, 0x75, 0x08, 0x95, 0x06, 0x81, 0x00, 0xc0,
];

pub fn application() -> Application {
    let protocol_mode = Arc::new(Mutex::new(1_u8));
    let protocol_mode_read = Arc::clone(&protocol_mode);
    let protocol_mode_write = Arc::clone(&protocol_mode);

    Application {
        services: vec![Service {
            uuid: HID_SERVICE_UUID,
            primary: true,
            characteristics: vec![
                read_only(HID_INFORMATION_UUID, HID_INFORMATION),
                read_only(REPORT_MAP_UUID, KEYBOARD_REPORT_MAP),
                Characteristic {
                    uuid: HID_CONTROL_POINT_UUID,
                    write: Some(CharacteristicWrite {
                        write_without_response: true,
                        encrypt_write: true,
                        method: CharacteristicWriteMethod::Fun(Box::new(|value, _| {
                            async move {
                                if matches!(value.as_slice(), [0] | [1]) {
                                    Ok(())
                                } else {
                                    Err(ReqError::InvalidValueLength)
                                }
                            }
                            .boxed()
                        })),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                Characteristic {
                    uuid: PROTOCOL_MODE_UUID,
                    read: Some(CharacteristicRead {
                        read: true,
                        encrypt_read: true,
                        fun: Box::new(move |request| {
                            let value = *protocol_mode_read
                                .lock()
                                .expect("protocol mode lock poisoned");
                            async move { read_at_offset(&[value], request.offset) }.boxed()
                        }),
                        ..Default::default()
                    }),
                    write: Some(CharacteristicWrite {
                        write_without_response: true,
                        encrypt_write: true,
                        method: CharacteristicWriteMethod::Fun(Box::new(move |value, _| {
                            let protocol_mode = Arc::clone(&protocol_mode_write);
                            async move {
                                let [value] = value.as_slice() else {
                                    return Err(ReqError::InvalidValueLength);
                                };
                                if *value > 1 {
                                    return Err(ReqError::NotSupported);
                                }
                                *protocol_mode.lock().expect("protocol mode lock poisoned") =
                                    *value;
                                Ok(())
                            }
                            .boxed()
                        })),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                Characteristic {
                    uuid: REPORT_UUID,
                    descriptors: vec![Descriptor {
                        uuid: REPORT_REFERENCE_UUID,
                        read: Some(DescriptorRead {
                            read: true,
                            encrypt_read: true,
                            fun: Box::new(|request| {
                                async move { read_at_offset(&[1, 1], request.offset) }.boxed()
                            }),
                            ..Default::default()
                        }),
                        ..Default::default()
                    }],
                    read: Some(CharacteristicRead {
                        read: true,
                        encrypt_read: true,
                        fun: Box::new(|request| {
                            async move { read_at_offset(EMPTY_KEYBOARD_REPORT, request.offset) }
                                .boxed()
                        }),
                        ..Default::default()
                    }),
                    notify: Some(CharacteristicNotify {
                        notify: true,
                        method: CharacteristicNotifyMethod::Fun(Box::new(|notifier| {
                            async move {
                                // Holding the notifier until the client unsubscribes advertises a
                                // valid input report without ever sending keyboard data.
                                notifier.stopped().await;
                            }
                            .boxed()
                        })),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn read_only(uuid: Uuid, value: &'static [u8]) -> Characteristic {
    Characteristic {
        uuid,
        read: Some(CharacteristicRead {
            read: true,
            encrypt_read: true,
            fun: Box::new(move |request| {
                async move { read_at_offset(value, request.offset) }.boxed()
            }),
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn read_at_offset(value: &[u8], offset: u16) -> Result<Vec<u8>, ReqError> {
    value
        .get(usize::from(offset)..)
        .map(ToOwned::to_owned)
        .ok_or(ReqError::InvalidOffset)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_required_hid_shape() {
        let app = application();
        assert_eq!(app.services.len(), 1);
        let service = &app.services[0];
        assert_eq!(service.uuid, HID_SERVICE_UUID);
        let uuids: Vec<_> = service
            .characteristics
            .iter()
            .map(|item| item.uuid)
            .collect();
        assert_eq!(
            uuids,
            vec![
                HID_INFORMATION_UUID,
                REPORT_MAP_UUID,
                HID_CONTROL_POINT_UUID,
                PROTOCOL_MODE_UUID,
                REPORT_UUID,
            ]
        );
        assert!(service.characteristics[4].notify.is_some());
        assert!(service.characteristics[0]
            .read
            .as_ref()
            .is_some_and(|read| read.encrypt_read));
        assert!(service.characteristics[2]
            .write
            .as_ref()
            .is_some_and(|write| write.encrypt_write));
        assert!(service.characteristics[4].descriptors[0]
            .read
            .as_ref()
            .is_some_and(|read| read.encrypt_read));
        assert_eq!(
            service.characteristics[4].descriptors[0].uuid,
            REPORT_REFERENCE_UUID
        );
    }

    #[test]
    fn static_reads_respect_offsets() {
        assert_eq!(read_at_offset(&[1, 2, 3], 1).unwrap(), vec![2, 3]);
        assert_eq!(read_at_offset(&[1, 2, 3], 3).unwrap(), Vec::<u8>::new());
        assert!(read_at_offset(&[1, 2, 3], 4).is_err());
    }
}
