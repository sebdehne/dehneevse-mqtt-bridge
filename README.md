# MQTT bridge for Dehne-EVSE
Provides MQTT communication for the Dehne-EVSE ([Hardware](https://github.com/sebdehne/DehneEVSE-Hardware)/[Firmware](https://github.com/sebdehne/DehneEVSE-Firmware)) by translating and bridging the binary protocol 
to JSON messages sent over MQTT. 

It listens on a configurable TCP-port, which multiple Dehne-EVSE charging stations can connect to, and connects to a MQTT-broker of your choice. 

Subscribe (listening for MQTT message) and publish (forwarding messages from the EVSE to MQTT) is done on seperate MQTT-topics, 
to avoid loops. See configuration options.

## Build
[Install rust](https://www.rust-lang.org/tools/install) and then build:

    $ cargo build

Then run:

    $ cargo run -- --help
    dehneevse_mqtt_bridge 

    USAGE:
        dehneevse_mqtt_bridge [OPTIONS]

    OPTIONS:
        -a <EVSE_LISTEN_ADDR>                                [default: [::]]
        -c <CONFIGURATION_FILE>                              [default: ./dehneevse_mqtt_bridge.toml]
        -h, --mqtt_host <MQTT_BROKER>                        [default: tcp://localhost:1883]
            --help                                           Print help information
            --mqtt_topic_publish <MQTT_TOPIC_PUBLISH>        [default: from_dehneEVSE]
            --mqtt_topic_subscribe <MQTT_TOPIC_SUBSCRIBE>    [default: to_dehneEVSE]
        -p <EVSE_LISTEN_PORT>                                [default: 9091]

## Types of messages

### 1. new connection
Once a Dehne-EVSE charing station connects to this bridge, it sends its serial-ID (128 bits as hex-string) 
and firmware-version (8 bits), which is published to MQTT as follows:

    {
      "message_type": "new_connection",
      "client_id": "10BA23AB50534D53302E3120FF162332",
      "handshake": {
        "firmware_version": 4
      }
    }

After this initial message, the charging station expects to receive some requests (se below) at least every 10 seconds,
otherwise it considers the connection to be dead and re-connects. You should send "request_data_collection"-requests
at a 5 second interval.

### 2. notify
As soon as the charging station detects some charge (f.eks cable plugged in), it sends a notification which
is published to MQTT as follows:

    {
      "message_type": "notify",
      "client_id": "10BA23AB50534D53302E3120FF162332"
    }

Note that this messages contains no information of what has changes, the server should collect this information 
by itself via a data readout request (see below).

### 3. ping request
Sending a ping-request is done by publishing the following message to MQTT:

    {
      "message_type": "request_ping",
      "client_id": "10BA23AB50534D53302E3120FF162332"
    }

The EVSE will then respond with the following MQTT-message:

    {
      "message_type": "response_ping",
      "client_id": "10BA23AB50534D53302E3120FF162332"
    }

### 4. Data collection
The following request will query all state and measurements from the EVSE:

    {
      "message_type": "request_data_collection",
      "client_id": "10BA23AB50534D53302E3120FF162332"
    }

The EVSE will then respond with the following MQTT-message:

    {
      "message_type": "response_collect_data",
      "client_id": "10BA23AB50534D53302E3120FF162332",
      "pwm_percent": 0,
      "contactor_state": false,
      "measurements": {
        "pilot_voltage": "volt_12",
        "proximity_pilot_amps": "no_cable",
        "phase1_millivolts": 0,
        "phase2_millivolts": 0,
        "phase3_millivolts": 0,
        "phase1_milliamps": 0,
        "phase2_milliamps": 0,
        "phase3_milliamps": 0,
        "wifi_rssi": -77,
        "uptime_milliseconds": 42,
        "current_control_pilot_adc": 0,
        "current_proximity_pilot_adc": 0,
        "logging_buffer": ""
      }
    }

### 5. Switching the contactor
Switching on/off the contactor is down via the following MQTT-request:

    {
      "message_type": "request_set_contactor_state",
      "client_id": "10BA23AB50534D53302E3120FF162332",
      "contactor_state": true
    }

The EVSE will then respond with the following MQTT-message:

    {
      "message_type": "response_set_contactor_state",
      "client_id": "10BA23AB50534D53302E3120FF162332"
    }

### 6. Setting the charge rate
Setting the allowed charge-rate is done by setting the PWN-duty cycle percentage via the following MQTT-request:

    {
      "message_type": "request_set_pwm_percent",
      "client_id": "10BA23AB50534D53302E3120FF162332",
      "pwm_percent": 50
    }

The EVSE will then respond with the following MQTT-message:

    {
      "message_type": "response_set_pwm_percent",
      "client_id": "10BA23AB50534D53302E3120FF162332"
    }

Use 100% for off (no charing allowed). Otherwise, use the mapping between ampere and duty cycle percentage:

- 10% =>  6A
- 15% => 10A
- 25% => 16A
- 40% => 25A
- 50% => 32A

Duty cycle = Amps / 0.6

See [https://www.ti.com/lit/ug/tidub87/tidub87.pdf](https://www.ti.com/lit/ug/tidub87/tidub87.pdf)

