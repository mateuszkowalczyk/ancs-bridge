use bluer::{
    adv::Advertisement,
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
pub const ANCS_SERVICE_UUID: Uuid = Uuid::from_u128(0x7905f431_b5ce_4e99_a40f_4b1e122d00d0);
const HID_INFORMATION_UUID: Uuid = Uuid::from_u128(0x00002a4a_0000_1000_8000_00805f9b34fb);
const REPORT_MAP_UUID: Uuid = Uuid::from_u128(0x00002a4b_0000_1000_8000_00805f9b34fb);
const HID_CONTROL_POINT_UUID: Uuid = Uuid::from_u128(0x00002a4c_0000_1000_8000_00805f9b34fb);
const REPORT_UUID: Uuid = Uuid::from_u128(0x00002a4d_0000_1000_8000_00805f9b34fb);
const PROTOCOL_MODE_UUID: Uuid = Uuid::from_u128(0x00002a4e_0000_1000_8000_00805f9b34fb);
const REPORT_REFERENCE_UUID: Uuid = Uuid::from_u128(0x00002908_0000_1000_8000_00805f9b34fb);

const HID_INFORMATION: &[u8] = &[0x11, 0x01, 0x00, 0x02];
const EMPTY_REPORT: &[u8] = &[0; 8];
const KEYBOARD_REPORT_MAP: &[u8] = &[
    0x05, 0x01, 0x09, 0x06, 0xa1, 0x01, 0x85, 0x01, 0x05, 0x07, 0x19, 0xe0, 0x29, 0xe7, 0x15, 0x00,
    0x25, 0x01, 0x75, 0x01, 0x95, 0x08, 0x81, 0x02, 0x75, 0x08, 0x95, 0x01, 0x81, 0x01, 0x05, 0x07,
    0x19, 0x00, 0x29, 0x65, 0x15, 0x00, 0x25, 0x65, 0x75, 0x08, 0x95, 0x06, 0x81, 0x00, 0xc0,
];

pub fn application() -> Application {
    let protocol = Arc::new(Mutex::new(1_u8));
    let read_protocol = Arc::clone(&protocol);
    let write_protocol = Arc::clone(&protocol);
    Application {
        services: vec![Service {
            uuid: HID_SERVICE_UUID,
            primary: true,
            characteristics: vec![
                encrypted_read(HID_INFORMATION_UUID, HID_INFORMATION),
                encrypted_read(REPORT_MAP_UUID, KEYBOARD_REPORT_MAP),
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
                            let value = *read_protocol.lock().expect("protocol lock poisoned");
                            async move { read_at_offset(&[value], request.offset) }.boxed()
                        }),
                        ..Default::default()
                    }),
                    write: Some(CharacteristicWrite {
                        write_without_response: true,
                        encrypt_write: true,
                        method: CharacteristicWriteMethod::Fun(Box::new(move |value, _| {
                            let protocol = Arc::clone(&write_protocol);
                            async move {
                                let [value] = value.as_slice() else {
                                    return Err(ReqError::InvalidValueLength);
                                };
                                if *value > 1 {
                                    return Err(ReqError::NotSupported);
                                }
                                *protocol.lock().expect("protocol lock poisoned") = *value;
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
                    read: Some(CharacteristicRead {
                        read: true,
                        encrypt_read: true,
                        fun: Box::new(|request| {
                            async move { read_at_offset(EMPTY_REPORT, request.offset) }.boxed()
                        }),
                        ..Default::default()
                    }),
                    notify: Some(CharacteristicNotify {
                        notify: true,
                        method: CharacteristicNotifyMethod::Fun(Box::new(|notifier| {
                            async move {
                                // There is deliberately no call that sends a report.
                                notifier.stopped().await;
                            }
                            .boxed()
                        })),
                        ..Default::default()
                    }),
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
                    ..Default::default()
                },
            ],
            ..Default::default()
        }],
        ..Default::default()
    }
}

pub fn runtime_advertisement() -> Advertisement {
    Advertisement {
        service_uuids: [HID_SERVICE_UUID].into_iter().collect(),
        solicit_uuids: [ANCS_SERVICE_UUID].into_iter().collect(),
        discoverable: Some(false),
        local_name: Some("ANCS Bridge".into()),
        appearance: Some(0x03c1),
        ..Default::default()
    }
}

pub fn setup_advertisement() -> Advertisement {
    Advertisement {
        discoverable: Some(true),
        ..runtime_advertisement()
    }
}

fn encrypted_read(uuid: Uuid, value: &'static [u8]) -> Characteristic {
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
    fn complete_encrypted_hid_shape_has_no_emission_path() {
        let app = application();
        let service = &app.services[0];
        assert_eq!(service.characteristics.len(), 5);
        assert_eq!(
            service
                .characteristics
                .iter()
                .map(|item| item.uuid)
                .collect::<Vec<_>>(),
            vec![
                HID_INFORMATION_UUID,
                REPORT_MAP_UUID,
                HID_CONTROL_POINT_UUID,
                PROTOCOL_MODE_UUID,
                REPORT_UUID
            ]
        );
        for index in [0, 1, 3, 4] {
            assert!(service.characteristics[index]
                .read
                .as_ref()
                .is_some_and(|read| read.encrypt_read));
        }
        for index in [2, 3] {
            assert!(service.characteristics[index]
                .write
                .as_ref()
                .is_some_and(|write| write.encrypt_write));
        }
        let report = &service.characteristics[4];
        assert!(report.notify.is_some());
        assert_eq!(report.descriptors.len(), 1);
        assert!(report.descriptors[0]
            .read
            .as_ref()
            .is_some_and(|read| read.encrypt_read));
    }

    #[test]
    fn runtime_advertisement_is_connectable_not_discoverable_and_solicits_ancs() {
        let advertisement = runtime_advertisement();
        assert_eq!(
            advertisement.advertisement_type,
            bluer::adv::Type::Peripheral
        );
        assert_eq!(advertisement.discoverable, Some(false));
        assert!(advertisement.service_uuids.contains(&HID_SERVICE_UUID));
        assert!(advertisement.solicit_uuids.contains(&ANCS_SERVICE_UUID));
        assert!(advertisement.discoverable_timeout.is_none());
    }

    #[test]
    fn setup_advertisement_is_discoverable_connectable_and_solicits_ancs() {
        let advertisement = setup_advertisement();
        assert_eq!(
            advertisement.advertisement_type,
            bluer::adv::Type::Peripheral
        );
        assert_eq!(advertisement.discoverable, Some(true));
        assert!(advertisement.service_uuids.contains(&HID_SERVICE_UUID));
        assert!(advertisement.solicit_uuids.contains(&ANCS_SERVICE_UUID));
    }
}
