use log::{error, info};
use mqtt::QOS_0;
use paho_mqtt as mqtt;
use paho_mqtt::AsyncClient;
use std::time::Duration;
use tokio::sync::broadcast;

use crate::protocol::MqttMessage;

pub async fn handle_mqtt(
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
            error!("Could not connect to MQTT-broker {}: {}", broker, err);
            return;
        }
        Ok(_) => info!("MQTT: Connected to broker {}", broker)
    }

    cli.subscribe(&topic_subscribe, mqtt::QOS_0).await.unwrap();
    info!("MQTT: Subscribed to {}", &topic_subscribe);

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