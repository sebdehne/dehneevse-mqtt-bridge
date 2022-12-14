use std::error::Error;
use clap::Parser;
use cli::Cli;
use config::Config;
use env_logger::{Builder, Target};
use log::{info, error, LevelFilter};
use tokio::net::TcpListener;
use tokio::sync::broadcast;
use utils::{USizeCountDownLatch, CountDownLatch};

use crate::evse_handler::handle_evse;
use crate::mqtt_handler::handle_mqtt;

mod cli;
mod evse_handler;
mod mqtt_handler;
mod protocol;
mod utils;

/*
 * TODO:
 * - timeout handling on EVSE/TCP-side
 * - code cleanup
 * - documentation
 * - release binary / install instructions ?
 */

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // setup logging:
    let mut builder = Builder::from_default_env();
    builder.target(Target::Stdout);
    builder.filter_level(LevelFilter::Info);
    builder.init();

    // parse CLI arguments
    let args: Cli = Cli::parse();

    // load configuration
    let settings = Config::builder()
        .set_default("evse.bind_address", args.evse_listen_addr)?
        .set_default("evse.bind_port", args.evse_listen_port)?
        .set_default("mqtt.broker", args.mqtt_broker)?
        .set_default("mqtt.topic_subscribe", args.mqtt_topic_subscribe)?
        .set_default("mqtt.topic_publish", args.mqtt_topic_publish)?
        .add_source(config::File::with_name(&args.configuration_file).required(false))
        .add_source(config::Environment::with_prefix("DEHNEEVSE").separator("_"))
        .build()?;

    // Communication channels between threads:
    // EVSE connections -> MQTT
    let (evse_mqtt_tx, evse_mqtt_rx) = broadcast::channel(32);
    // MQTT -> EVSE connections
    let (mqtt_evse_tx, mqtt_evse_rx) = broadcast::channel(32);
    // Shutdown
    let (shutdown_tx, _shutdown_rx) = broadcast::channel::<bool>(32);

    // Register SIGTERM signal handler
    let shutdown_tx_clone = shutdown_tx.clone();
    ctrlc::set_handler(move || {
        shutdown_tx_clone.send(true).unwrap();
    })
    .expect("Error setting Ctrl-C handler");

    // setup shared counter to track number of active "threads"
    let active_threads = USizeCountDownLatch::new();

    // Start by listening on the TCP port for EVSE stations to connect to
    let listener = TcpListener::bind(format!(
        "{}:{}",
        settings.get_string("evse.bind_address")?,
        settings.get_int("evse.bind_port")?
    ))
    .await
    .unwrap_or_else(|err| {
        panic!(
            "Could not listen on addr={} port={} error={}",
            settings.get_string("evse.bind_address").unwrap(),
            settings.get_int("evse.bind_port").unwrap(),
            err
        )
    });
    info!(
        "EVSE: Listening on {}:{}",
        settings.get_string("evse.bind_address")?,
        settings.get_int("evse.bind_port")?
    );

    let mqtt_evse_tx_clone = mqtt_evse_tx.clone();
    let evse_mqtt_rx_clone = evse_mqtt_rx.resubscribe();
    let shutdown_rx_clone = shutdown_tx.subscribe();
    let shutdown_tx_clone = shutdown_tx.clone();
    let settings_clone = settings.clone();
    let active_threads_clone = active_threads.clone();
    tokio::spawn(async move {
        active_threads_clone.count_up();
        
        handle_mqtt(
            settings_clone.get_string("mqtt.broker").unwrap(),
            settings_clone.get_string("mqtt.topic_subscribe").unwrap(),
            settings_clone.get_string("mqtt.topic_publish").unwrap(),
            mqtt_evse_tx_clone,
            evse_mqtt_rx_clone,
            shutdown_rx_clone,
        ).await.unwrap_or_else(|err| {
            shutdown_tx_clone.send(true).unwrap();
            error!("MQTT: handling failed: {}", err);
        });

        active_threads_clone.count_down();
    });

    let mut shutdown_rx = shutdown_tx.subscribe();
    loop {
        tokio::select! {
            accept = listener.accept() => {
                let (socket, peer_addr) = accept.unwrap();
                let mqtt_evse_rx_clone = mqtt_evse_rx.resubscribe();
                let evse_mqtt_tx_clone = evse_mqtt_tx.clone();
                let shutdown_rx_clone = shutdown_tx.subscribe();
                let active_threads_clone = active_threads.clone();
                let settings_clone = settings.clone();
                tokio::spawn(async move {

                    active_threads_clone.count_up();

                    handle_evse(
                        mqtt_evse_rx_clone,
                        evse_mqtt_tx_clone,
                        socket,
                        peer_addr,
                        shutdown_rx_clone,
                        settings_clone.clone()
                    )
                    .await.unwrap_or_else(|err| {
                        error!("EVSE: connection failed: {}", err);
                    });

                    active_threads_clone.count_down();
                });
            }
            Ok(_) = shutdown_rx.recv() => { break }
        }
    }

    active_threads.wait_for_zero();

    info!("Bye");
    Ok(())
}
