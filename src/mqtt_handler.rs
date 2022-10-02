use log::{error, info};
use mqtt::QOS_0;
use paho_mqtt as mqtt;
use paho_mqtt::AsyncClient;
use std::error;
use std::time::Duration;
use tokio::sync::broadcast;

use crate::protocol::MqttMessage;

type Result<T> = std::result::Result<T, Box<dyn error::Error>>;

pub async fn handle_mqtt(
    broker: String,
    topic_subscribe: String,
    topic_publish: String,
    mqtt_evse_tx: broadcast::Sender<MqttMessage>,
    mut evse_mqtt_rx: broadcast::Receiver<MqttMessage>,
    mut shutdown_rx: broadcast::Receiver<bool>,
) -> Result<()> {
    let create_opts = mqtt::CreateOptionsBuilder::new()
        .server_uri(&broker)
        .client_id("dehneevse_mqtt_bridge")
        .finalize();
    let mut cli = AsyncClient::new(create_opts)?;

    let conn_opts = mqtt::ConnectOptionsBuilder::new()
        .keep_alive_interval(Duration::from_secs(20))
        .clean_session(true)
        .finalize();

    let st = cli.get_stream(10);

    let mut connected = false;
    let mut subscribed = true;
    let mut need_reconnect = false;
    let mut need_sleep = false;
    let mut publish_msg: Option<mqtt::Message> = None;

    loop {
        let connected_ok = connected && !need_reconnect && subscribed && !need_sleep;

        let publish_to_mqtt = async { cli.publish(publish_msg.clone().unwrap()).await };

        tokio::select! {
            connect = cli.connect(conn_opts.clone()), if !connected => {
                match connect {
                    Ok(_) => {
                        info!("MQTT: Connected to broker {}", broker);
                        connected = true;
                        subscribed = false;
                    }
                    Err(err) => {
                        error!("MQTT: Could not connect to broker: {}", err);
                        connected = true;
                        need_reconnect = true;
                        need_sleep = true;
                    },
                }
            }
            publish = publish_to_mqtt, if publish_msg.is_some() => {
                if let Err(err) = publish {
                    error!("Could not send json to MQTT: {}", err);
                }
                publish_msg = None;
            }
            _ = tokio::time::sleep(Duration::from_secs(10)), if need_sleep => {need_sleep = false}
            Ok(_) = shutdown_rx.recv() => {
                info!("MQTT: Disconnecting due to shutdown");
                break;
            }
            connect = cli.reconnect(), if need_reconnect && !need_sleep => {
                match connect {
                    Ok(_) => {
                        info!("MQTT: Re-connected to broker {}", broker);
                        need_reconnect = false;
                        subscribed = false;
                    }
                    Err(err) => {
                        error!("MQTT: Could not re-connect to broker: {}", err);
                        need_sleep = true;
                    },
                }
            }
            subscribe = cli.subscribe(&topic_subscribe, mqtt::QOS_0), if !subscribed => {
                match subscribe {
                    Ok(_) => {
                        info!("MQTT: Subscribed to {}", &topic_subscribe);
                        subscribed = true;
                    }
                    Err(err) => {
                        error!("MQTT: Could not subscribe to topic {}: {}", &topic_subscribe, err);
                        need_reconnect = true;
                        need_sleep = true;
                        let disconnect_options = mqtt::DisconnectOptionsBuilder::new()
                            .timeout(Duration::from_secs(5))
                            .finalize();
                        cli.disconnect(disconnect_options).await?;
                    },
                }
            }
            receive = evse_mqtt_rx.recv(), if connected_ok && publish_msg.is_none() => {if let Ok(msg) = receive {
                match serde_json::to_string(&msg) {
                    Err(err) => error!("MQTT: Could not serialize message to JSON: {:?}: {}", &msg, err),
                    Ok(json) => {
                        info!("MQTT: publishing msg from EVSE: {}", json);
                        publish_msg = Some(mqtt::Message::new(&topic_publish,json, QOS_0));
                    }
                }
            }}
            receive = st.recv(), if connected_ok => {
                match receive {
                    Ok(Some(msg)) => {
                        let payload_string = msg.payload_str().into_owned();
                        match serde_json::from_str::<MqttMessage>(&payload_string) {
                            Ok(json) => {
                                info!("MQTT: message received {}", payload_string);
                                mqtt_evse_tx.send(json).unwrap_or_else(|err| {
                                    error!("Could not forward message to EVSE-side: {}", err);
                                    0
                                });
                            }
                            Err(err) => {
                                error!("MQTT: Could not parse received mqtt-message {}. Error={}", &payload_string, err);
                            }
                        }
                    }
                    Ok(None) => {
                        error!("MQTT: Disconnected");
                        if !cli.is_connected() {
                            need_reconnect = true;
                            need_sleep = true;
                        }
                    }
                    Err(error) => {
                        error!("MQTT: Error while receiving mqtt messages {}", error);
                        if !cli.is_connected() {
                            need_reconnect = true;
                            need_sleep = true;
                        }
                    }
                }
            }
            else => { break }
        }
    }
    Ok(())
}
