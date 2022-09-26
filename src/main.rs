use byteorder::{BigEndian, ByteOrder};
use clap::Parser;
use env_logger::{Builder, Target};
use log::{error, info, LevelFilter};
use mqtt::QOS_0;
use paho_mqtt as mqtt;
use paho_mqtt::AsyncClient;
use serde::__private::from_utf8_lossy;
use serde::{Deserialize, Serialize};
use std::io::ErrorKind;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast;

/*
 * TODO:
 * - EVSE names - not serial_ids
 * - auto-reconnect mqtt
 * - code cleanup
 * - no rustc warnings
 * - documentation
 * - release binary / install instructions ?
 */

#[tokio::main]
async fn main() {
    let mut builder = Builder::from_default_env();
    builder.target(Target::Stdout);
    builder.filter_level(LevelFilter::Info);
    builder.init();

    let args: Cli = Cli::parse();
    let addr = "[::]";

    // EVSE connections -> MQTT
    let (evse_mqtt_tx, evse_mqtt_rx) = broadcast::channel(32);

    // MQTT -> EVSE connections
    let (mqtt_evse_tx, mqtt_evse_rx) = broadcast::channel(32);

    // Shutdown
    let (shutdown_tx, _shutdown_rx) = broadcast::channel::<bool>(32);

    let shutdown_tx_clone = shutdown_tx.clone();
    ctrlc::set_handler(move || {
        shutdown_tx_clone.send(true).unwrap();
    })
    .expect("Error setting Ctrl-C handler");

    let listener = TcpListener::bind(format!("{}:{}", addr, args.evse_listen_port))
        .await
        .unwrap_or_else(|error| {
            panic!(
                "Could not listen on addr={} port={} error={}",
                addr, args.evse_listen_port, error
            )
        });
    info!("EVSE: Listening on {}:{}", addr, args.evse_listen_port);

    let mqtt_evse_tx_clone = mqtt_evse_tx.clone();
    let evse_mqtt_rx_clone = evse_mqtt_rx.resubscribe();
    let broker = args.mqtt_broker.clone();
    let shutdown_rx_clone = shutdown_tx.subscribe();
    let topic_subscribe = args.mqtt_topic_subscribe.clone();
    let topic_publish = args.mqtt_topic_publish.clone();
    let shutdown_tx_clone = shutdown_tx.clone();
    tokio::spawn(async move {
        handle_mqtt(
            broker,
            topic_subscribe,
            topic_publish,
            mqtt_evse_tx_clone,
            evse_mqtt_rx_clone,
            shutdown_rx_clone,
            shutdown_tx_clone,
        )
        .await;
    });

    let mut shutdown_rx = shutdown_tx.subscribe();
    loop {
        tokio::select! {
            accept = listener.accept() => {
                let (socket, peer_addr) = accept.unwrap();
                let mqtt_evse_rx_clone = mqtt_evse_rx.resubscribe();
                let evse_mqtt_tx_clone = evse_mqtt_tx.clone();
                let shutdown_rx_clone = shutdown_tx.subscribe();
                tokio::spawn(async move {
                    handle_evse(
                        mqtt_evse_rx_clone,
                        evse_mqtt_tx_clone,
                        socket,
                        peer_addr,
                        shutdown_rx_clone,
                    )
                    .await
                });
            }
            receive = shutdown_rx.recv() => {
                receive.unwrap();
                break;
            }
        }
    }
    // TODO wait here until all tasks/threads have finished
    info!("Bye");
}

async fn handle_mqtt(
    broker: String,
    topic_subscribe: String,
    topic_publish: String,
    mqtt_evse_tx: broadcast::Sender<MqttMessage>,
    mut evse_mqtt_rx: broadcast::Receiver<MqttMessage>,
    mut shutdown_rx: broadcast::Receiver<bool>,
    shutdown_tx: broadcast::Sender<bool>,
) {
    let create_opts = mqtt::CreateOptionsBuilder::new()
        .server_uri(&broker)
        .client_id("dehneevse_mqtt_bridge")
        .finalize();
    let mut cli = AsyncClient::new(create_opts).expect("Setting up MQTT-client failed");

    let conn_opts = mqtt::ConnectOptionsBuilder::new()
        .keep_alive_interval(Duration::from_secs(20))
        .clean_session(true)
        .finalize();

    let st = cli.get_stream(10);

    match cli.connect(conn_opts).await {
        Err(err) => {
            shutdown_tx.send(true).unwrap();
            error!("Could not connect to MQTT-broker: {}", err);
            return;
        }
        Ok(_) => info!("MQTT: Connected to broker {}", broker)
    }

    cli.subscribe(&topic_subscribe, mqtt::QOS_0).await.unwrap();

    loop {
        tokio::select! {
            receive = shutdown_rx.recv() => {
                receive.unwrap();
                info!("MQTT: Disconnecting due to shutdown");
                break;
            }
            receive = evse_mqtt_rx.recv() => {
                let msg = receive.unwrap();
                let json = serde_json::to_string(&msg).unwrap();
                info!("MQTT: publishing msg from EVSE: {}", json);
                let mqtt_msg = mqtt::Message::new(&topic_publish,json, QOS_0);
                cli.publish(mqtt_msg).await.unwrap();
            }
            receive = st.recv() => {
                match receive {
                    Ok(Some(msg)) => {
                        let payload_string = msg.payload_str().into_owned();
                        match serde_json::from_str::<MqttMessage>(&payload_string) {
                            Ok(json) => {
                                info!("MQTT: message received {}", payload_string);
                                mqtt_evse_tx.send(json).unwrap();
                            }
                            Err(err) => {
                                error!("MQTT: Could not read received mqtt-message: {}", err);
                            }
                        }
                    }
                    Ok(None) => {
                        error!("MQTT: Detected client disconnect");
                        // TODO re-connect
                    }
                    Err(error) => {
                        // TODO re-connect
                        panic!("MQTT: Error while receiving mqtt messages {}", error);
                    }
                }
            }
        }
    }
}

async fn handle_evse(
    mut mqtt_evse_rx: broadcast::Receiver<MqttMessage>,
    evse_mqtt_tx: broadcast::Sender<MqttMessage>,
    mut socket: TcpStream,
    peer_addr: SocketAddr,
    mut shutdown_rx: broadcast::Receiver<bool>,
) {
    info!("EVSE: Accepted new EVSE connection from {}", peer_addr);

    let (mut tcp_rx, mut tcp_tx) = socket.split();

    // client starts by sending welcome message:
    let mut buf: [u8; 16] = [0; 16];
    tcp_rx
        .read_exact(&mut buf)
        .await
        .expect("Error reading the serial ID");
    let client_serial = bytes_to_hex(&buf);

    let mut buf: [u8; 1] = [0];
    tcp_rx
        .read_exact(&mut buf)
        .await
        .expect("Error reading the firmware version");
    let firmware_version = buf[0];

    let payload = MqttMessage {
        message_type: MqttMessageType::new_connection,
        client_id: client_serial.clone(),
        handshake: Some(MqttMessageHandshake { firmware_version }),
        firmware: None,
        pwm_percent: None,
        contactor_state: None,
        measurements: None,
    };

    evse_mqtt_tx.send(payload).unwrap();

    let mut message_header_buf = [0u8; 5];

    loop {
        tokio::select! {
            receive = shutdown_rx.recv() => {
                receive.unwrap();
                info!("EVSE: Closing connection to {} due to shutdown", client_serial);
                break;
            }
            receive = mqtt_evse_rx.recv() => {
                let mqtt_message = receive.unwrap();
                if mqtt_message.client_id == client_serial {
                    info!("EVSE: Sending msg to EVSE {:?}", mqtt_message);
                    let v: Vec<u8> = mqtt_to_evse(mqtt_message);
                    match tcp_tx.write_all(&v[..]).await {
                        Err(err) => {
                            error!("EVSE: Closing connection to client {} because of write error: {}", client_serial, err);
                            break;
                        }
                        Ok(_) => {}
                    }
                }
            }
            reading_result = tcp_rx.read_exact(&mut message_header_buf) => {
                match reading_result {
                    Ok(_size) => {
                        let msg_type = message_header_buf[0];
                        let msg_length = BigEndian::read_u32(&message_header_buf[1..5]);

                        if msg_length > 1024 {
                            panic!("EVSE: received msg_length={} is too large", msg_length);
                        }

                        let mut payload_buf: Vec<u8> = Vec::with_capacity(msg_length as usize);
                        match tcp_rx.read_exact(&mut payload_buf).await {
                            Err(ref e) if e.kind() == ErrorKind::UnexpectedEof => {
                                info!("EVSE: {} disconnected", client_serial);
                                break;
                            }
                            Err(e) => {
                                error!("EVSE: error while reading from {}: {}", client_serial, e);
                                break;
                            },
                            _ => {}
                        };

                        let msg = evse_to_mqtt(
                            client_serial.clone(),
                            msg_type,
                            msg_length,
                            &payload_buf[..]
                        );

                        evse_mqtt_tx.send(msg).unwrap();
                    }
                    Err(ref e) if e.kind() == ErrorKind::UnexpectedEof => {
                        info!("EVSE: {} disconnected", client_serial);
                        break;
                    }
                    Err(e) => {
                        error!("EVSE: error while reading from {}: {}", client_serial, e);
                        break;
                    }
                }
            }
        }
    }
}

#[allow(unused_variables)]
fn evse_to_mqtt(
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

fn mqtt_to_evse(msg: MqttMessage) -> Vec<u8> {
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

fn bytes_to_hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::new();
    for byte in bytes {
        write!(&mut s, "{:02X}", byte).expect("Unable to write");
    }
    s
}

#[derive(Parser, Clone)]
struct Cli {
    #[clap(short = 'l', default_value = "9091")]
    evse_listen_port: u16,
    #[clap(
        short = 'h',
        long = "mqtt_host",
        default_value = "tcp://localhost:1883"
    )]
    mqtt_broker: String,
    #[clap(long = "mqtt_topic_subscribe", default_value = "to_dehneEVSE")]
    mqtt_topic_subscribe: String,
    #[clap(long = "mqtt_topic_publish", default_value = "from_dehneEVSE")]
    mqtt_topic_publish: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct MqttMessage {
    message_type: MqttMessageType,
    client_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    handshake: Option<MqttMessageHandshake>,
    #[serde(skip_serializing_if = "Option::is_none")]
    firmware: Option<MqttMessageFirmware>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pwm_percent: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    contactor_state: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    measurements: Option<MqttMessageMeasurements>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct MqttMessageHandshake {
    firmware_version: u8,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct MqttMessageFirmware {
    firmware_data_base64: String,
    firmware_crc32: u32,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct MqttMessageMeasurements {
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
enum MqttMessageType {
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
enum PilotVoltage {
    volt_12,
    volt_9,
    volt_6,
    volt_3,
    fault,
}

#[allow(non_camel_case_types)]
#[derive(Serialize, Deserialize, Debug, Clone)]
enum ProximityPilotAmps {
    amp_13,
    amp_20,
    amp_32,
    no_cable,
}
