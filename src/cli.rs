use clap::Parser;

#[derive(Parser, Clone)]
pub struct Cli {
    #[clap(short = 'c', default_value = "./dehneevse_mqtt_bridge.toml")]
    pub configuration_file: String,

    #[clap(short = 'a', default_value = "[::]")]
    pub evse_listen_addr: String,
    #[clap(short = 'p', default_value = "9091")]
    pub evse_listen_port: u16,

    #[clap(
        short = 'h',
        long = "mqtt_host",
        default_value = "tcp://localhost:1883"
    )]
    pub mqtt_broker: String,
    #[clap(long = "mqtt_topic_subscribe", default_value = "to_dehneEVSE")]
    pub mqtt_topic_subscribe: String,
    #[clap(long = "mqtt_topic_publish", default_value = "from_dehneEVSE")]
    pub mqtt_topic_publish: String,
}
