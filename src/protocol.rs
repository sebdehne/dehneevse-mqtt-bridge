use byteorder::{BigEndian, ByteOrder};
use serde::{__private::from_utf8_lossy, Serialize, Deserialize};

#[allow(unused_variables)]
pub fn evse_to_mqtt(
    client_id: String,
    message_type: u8,
    payload_length: u32,
    payload: &[u8],
) -> MqttMessage {
    match message_type {
        RESPONSE_TYPE_PONG => MqttMessage {
            message_type: MqttMessageType::response_ping,
            client_id,
            handshake: None,
            firmware: None,
            pwm_percent: None,
            contactor_state: None,
            measurements: None,
        },
        NOTIFY => MqttMessage {
            message_type: MqttMessageType::notify,
            client_id,
            handshake: None,
            firmware: None,
            pwm_percent: None,
            contactor_state: None,
            measurements: None,
        },
        RESPONSE_TYPE_SET_PWM_PERCENT => MqttMessage {
            message_type: MqttMessageType::response_set_pwm_percent,
            client_id,
            handshake: None,
            firmware: None,
            pwm_percent: None,
            contactor_state: None,
            measurements: None,
        },
        RESPONSE_TYPE_SET_CONTACTOR_STATE => MqttMessage {
            message_type: MqttMessageType::response_set_contactor_state,
            client_id,
            handshake: None,
            firmware: None,
            pwm_percent: None,
            contactor_state: None,
            measurements: None,
        },
        RESPONSE_TYPE_COLLECT_DATA => {
            let contactor_state = if payload[0] == 1 { true } else { false };
            let pwm_percent = payload[1];
            let pilot_volt = payload[2];
            let proximity_pilot_amps = payload[3];

            let measurements = MqttMessageMeasurements {
                pilot_voltage: match pilot_volt {
                    0 => PilotVoltage::volt_12,
                    1 => PilotVoltage::volt_9,
                    2 => PilotVoltage::volt_6,
                    3 => PilotVoltage::volt_3,
                    4 => PilotVoltage::fault,
                    _ => panic!("Unsupported pilot_volt={}", pilot_volt),
                },
                proximity_pilot_amps: match proximity_pilot_amps {
                    0 => ProximityPilotAmps::amp_13,
                    1 => ProximityPilotAmps::amp_20,
                    2 => ProximityPilotAmps::amp_32,
                    3 => ProximityPilotAmps::no_cable,
                    _ => panic!("Unsupported proximity_pilot_amps={}", pilot_volt),
                },
                phase1_millivolts: BigEndian::read_u32(&payload[4..]),
                phase2_millivolts: BigEndian::read_u32(&payload[8..]),
                phase3_millivolts: BigEndian::read_u32(&payload[12..]),
                phase1_milliamps: BigEndian::read_u32(&payload[16..]),
                phase2_milliamps: BigEndian::read_u32(&payload[20..]),
                phase3_milliamps: BigEndian::read_u32(&payload[24..]),
                wifi_rssi: BigEndian::read_i32(&payload[28..]),
                uptime_milliseconds: BigEndian::read_i32(&payload[32..]),
                current_control_pilot_adc: BigEndian::read_u32(&payload[36..]),
                current_proximity_pilot_adc: BigEndian::read_u32(&payload[40..]),
                logging_buffer: from_utf8_lossy(&payload[44..]).into_owned(),
            };

            MqttMessage {
                message_type: MqttMessageType::request_data_collection,
                client_id,
                handshake: None,
                firmware: None,
                pwm_percent: Some(pwm_percent),
                contactor_state: Some(contactor_state),
                measurements: Some(measurements),
            }
        }

        _ => panic!("Unpported message type {}", message_type),
    }
}

pub fn mqtt_to_evse(msg: MqttMessage) -> Vec<u8> {
    let mut vec = Vec::new();
    match msg.message_type {
        MqttMessageType::request_ping => {
            vec.push(REQUEST_TYPE_PING);
            byteorder::WriteBytesExt::write_u32::<BigEndian>(&mut vec, 0).unwrap();
        }
        MqttMessageType::request_firmware => {
            let firmware = msg.firmware.unwrap();
            let firmware_data = base64::decode(firmware.firmware_data_base64).unwrap();
            vec.push(REQUEST_TYPE_FIRMWARE);
            byteorder::WriteBytesExt::write_u32::<BigEndian>(
                &mut vec,
                firmware_data.len() as u32 + 4,
            )
            .unwrap();
            byteorder::WriteBytesExt::write_u32::<BigEndian>(&mut vec, firmware.firmware_crc32)
                .unwrap();
            std::io::Write::write_all(&mut vec, &firmware_data[..]).unwrap();
        }
        MqttMessageType::request_data_collection => {
            vec.push(REQUEST_TYPE_COLLECT_DATA);
            byteorder::WriteBytesExt::write_u32::<BigEndian>(&mut vec, 0).unwrap();
        }
        MqttMessageType::request_set_pwm_percent => {
            let percent = msg.pwm_percent.unwrap();
            vec.push(REQUEST_TYPE_SET_PWM_PERCENT);
            byteorder::WriteBytesExt::write_u32::<BigEndian>(&mut vec, 1).unwrap();
            byteorder::WriteBytesExt::write_u8(&mut vec, percent).unwrap();
        }
        MqttMessageType::request_set_contactor_state => {
            let contactor_state = if msg.contactor_state.unwrap() { 1 } else { 0 };
            vec.push(REQUEST_TYPE_SET_CONTACTOR_STATE);
            byteorder::WriteBytesExt::write_u32::<BigEndian>(&mut vec, 1).unwrap();
            byteorder::WriteBytesExt::write_u8(&mut vec, contactor_state).unwrap();
        }
        _ => {
            panic!(
                "Do not know how to send message_type: {:?}",
                msg.message_type
            )
        }
    };
    vec
}

const RESPONSE_TYPE_PONG: u8 = 1;
const RESPONSE_TYPE_COLLECT_DATA: u8 = 2;
const RESPONSE_TYPE_SET_PWM_PERCENT: u8 = 3;
const RESPONSE_TYPE_SET_CONTACTOR_STATE: u8 = 4;
const NOTIFY: u8 = 100;

const REQUEST_TYPE_PING: u8 = 1;
const REQUEST_TYPE_FIRMWARE: u8 = 2;
const REQUEST_TYPE_COLLECT_DATA: u8 = 3;
const REQUEST_TYPE_SET_PWM_PERCENT: u8 = 4;
const REQUEST_TYPE_SET_CONTACTOR_STATE: u8 = 5;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MqttMessage {
    pub message_type: MqttMessageType,
    pub client_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub handshake: Option<MqttMessageHandshake>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub firmware: Option<MqttMessageFirmware>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pwm_percent: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contactor_state: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub measurements: Option<MqttMessageMeasurements>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MqttMessageHandshake {
    pub firmware_version: u8,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MqttMessageFirmware {
    firmware_data_base64: String,
    firmware_crc32: u32,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MqttMessageMeasurements {
    pilot_voltage: PilotVoltage,
    proximity_pilot_amps: ProximityPilotAmps,
    phase1_millivolts: u32,
    phase2_millivolts: u32,
    phase3_millivolts: u32,
    phase1_milliamps: u32,
    phase2_milliamps: u32,
    phase3_milliamps: u32,
    wifi_rssi: i32,
    uptime_milliseconds: i32,
    current_control_pilot_adc: u32,
    current_proximity_pilot_adc: u32,
    logging_buffer: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[allow(non_camel_case_types)]
pub enum MqttMessageType {
    new_connection,
    notify,

    response_ping,
    response_collect_data,
    response_set_pwm_percent,
    response_set_contactor_state,

    request_ping,
    request_data_collection,
    request_firmware,
    request_set_pwm_percent,
    request_set_contactor_state,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[allow(non_camel_case_types)]
pub enum PilotVoltage {
    volt_12,
    volt_9,
    volt_6,
    volt_3,
    fault,
}

#[allow(non_camel_case_types)]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum ProximityPilotAmps {
    amp_13,
    amp_20,
    amp_32,
    no_cable,
}
