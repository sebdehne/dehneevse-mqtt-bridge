
use byteorder::{BigEndian, ByteOrder};
use config::Config;
use log::{error, info};
use std::io::ErrorKind;
use std::net::SocketAddr;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpStream};
use tokio::sync::broadcast;
use crate::protocol::{MqttMessage, MqttMessageType, MqttMessageHandshake, mqtt_to_evse, evse_to_mqtt};
use crate::utils::bytes_to_hex;


pub async fn handle_evse(
    mut mqtt_evse_rx: broadcast::Receiver<MqttMessage>,
    evse_mqtt_tx: broadcast::Sender<MqttMessage>,
    mut socket: TcpStream,
    peer_addr: SocketAddr,
    mut shutdown_rx: broadcast::Receiver<bool>,
    settings: Config
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
    let client_id = settings.get_string(&format!("evse_name.id_{}", client_serial)).unwrap_or(client_serial);

    let mut buf: [u8; 1] = [0];
    tcp_rx
        .read_exact(&mut buf)
        .await
        .expect("Error reading the firmware version");
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

    evse_mqtt_tx.send(payload).unwrap();

    let mut message_header_buf = [0u8; 5];

    loop {
        tokio::select! {
            receive = shutdown_rx.recv() => {
                receive.unwrap();
                info!("EVSE: Closing connection to {} due to shutdown", client_id);
                break;
            }
            receive = mqtt_evse_rx.recv() => {
                let mqtt_message = receive.unwrap();
                if mqtt_message.client_id == client_id {
                    info!("EVSE: Sending msg to EVSE {:?}", mqtt_message);
                    let v: Vec<u8> = mqtt_to_evse(mqtt_message);
                    match tcp_tx.write_all(&v[..]).await {
                        Err(err) => {
                            error!("EVSE: Closing connection to client {} because of write error: {}", client_id, err);
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
                                info!("EVSE: {} disconnected", client_id);
                                break;
                            }
                            Err(e) => {
                                error!("EVSE: error while reading from {}: {}", client_id, e);
                                break;
                            },
                            _ => {}
                        };

                        let msg = evse_to_mqtt(
                            client_id.clone(),
                            msg_type,
                            msg_length,
                            &payload_buf[..]
                        );

                        evse_mqtt_tx.send(msg).unwrap();
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
}


