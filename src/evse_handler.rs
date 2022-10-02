use crate::protocol::{
    evse_to_mqtt, mqtt_to_evse, MqttMessage, MqttMessageHandshake, MqttMessageType,
};
use crate::utils::bytes_to_hex;
use byteorder::{BigEndian, ByteOrder};
use config::Config;
use log::{error, info};
use std::error;
use std::io::{Error, ErrorKind};
use std::net::SocketAddr;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::broadcast;

type Result<T> = std::result::Result<T, Box<dyn error::Error>>;

pub async fn handle_evse(
    mut mqtt_evse_rx: broadcast::Receiver<MqttMessage>,
    evse_mqtt_tx: broadcast::Sender<MqttMessage>,
    mut socket: TcpStream,
    peer_addr: SocketAddr,
    mut shutdown_rx: broadcast::Receiver<bool>,
    settings: Config,
) -> Result<()> {
    info!("EVSE: Accepted new EVSE connection from {}", peer_addr);

    let (mut tcp_rx, mut tcp_tx) = socket.split();

    // client starts by sending welcome message:
    let mut buf: [u8; 16] = [0; 16];
    tcp_rx.read_exact(&mut buf).await.map_err(|err| {
        Error::new(
            ErrorKind::InvalidData,
            format!("Error reading the serial ID: {}", err),
        )
    })?;
    let client_serial = bytes_to_hex(&buf);
    let client_id = settings
        .get_string(&format!("evse_name.id_{}", client_serial))
        .unwrap_or(client_serial);

    let mut buf: [u8; 1] = [0];
    tcp_rx.read_exact(&mut buf).await.map_err(|err| {
        Error::new(
            ErrorKind::InvalidData,
            format!("Error reading the firmware version: {}", err),
        )
    })?;
    let firmware_version = buf[0];

    let payload = MqttMessage {
        message_type: MqttMessageType::new_connection,
        client_id: client_id.clone(),
        handshake: Some(MqttMessageHandshake { firmware_version }),
        firmware: None,
        pwm_percent: None,
        contactor_state: None,
        measurements: None,
    };

    evse_mqtt_tx.send(payload)?;

    let mut message_header_buf = [0u8; 5];
    let mut bytes_to_write: Vec<u8> = vec![];

    loop {
        tokio::select! {
            Ok(_) = shutdown_rx.recv() => {
                info!("EVSE: Closing connection to {} due to shutdown", client_id);
                break;
            }
            sendt = tcp_tx.write_all(&bytes_to_write[..]), if bytes_to_write.len() > 0 => {
                match sendt {
                    Err(err) => {
                        error!("EVSE: Error writing to EVSE {}: {}", client_id, err);
                        break;
                    }
                    Ok(_) => bytes_to_write = vec![]
                }
            }
            receive = mqtt_evse_rx.recv() => {
                match receive {
                    Err(err) => error!("EVSE: Error reading from MQTT {}: {}", client_id, err),
                    Ok(mqtt_message) => {
                        if mqtt_message.client_id == client_id {
                            info!("EVSE: Sending msg to EVSE {:?}", mqtt_message);
                            match mqtt_to_evse(mqtt_message) {
                                Ok(v) => bytes_to_write = v,
                                Err(err) => {
                                    error!("EVSE: Error translating MQTT-message to EVSE-message {}: {}", client_id, err);
                                }
                            }
                        }
                    }
                }
            }
            reading_result = tcp_rx.read_exact(&mut message_header_buf) => {
                match reading_result {
                    Ok(_size) => {
                        let msg_type = message_header_buf[0];
                        let msg_length = BigEndian::read_u32(&message_header_buf[1..5]);

                        if msg_length > 1024 {
                            error!("EVSE: received msg_length={} is too large, disconnecting", msg_length);
                            break;
                        }

                        let mut payload_buf: Vec<u8> = Vec::with_capacity(msg_length as usize);
                        match tcp_rx.read_exact(&mut payload_buf).await {
                            Err(ref e) if e.kind() == ErrorKind::UnexpectedEof => {
                                info!("EVSE: {} disconnected", client_id);
                                break;
                            }
                            Err(e) => {
                                error!("EVSE: error while reading from {}: {}", client_id, e);
                                break;
                            },
                            _ => {}
                        };

                        let msg = match evse_to_mqtt(
                            client_id.clone(),
                            msg_type,
                            msg_length,
                            &payload_buf[..]
                        ) {
                            Ok(msg) => msg,
                            Err(e) => {
                                error!("EVSE: error while parsing data from {}: {}", client_id, e);
                                break;
                            }
                        };

                        evse_mqtt_tx.send(msg)?;
                    }
                    Err(ref e) if e.kind() == ErrorKind::UnexpectedEof => {
                        info!("EVSE: {} disconnected", client_id);
                        break;
                    }
                    Err(e) => {
                        error!("EVSE: error while reading from {}: {}", client_id, e);
                        break;
                    }
                }
            }
        }
    }
    Ok(())
}
