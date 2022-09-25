use std::io::Read;
use std::net::{TcpListener, TcpStream};
use std::time::Duration;

fn main() {
    let port: u16 = 9091;
    let addr = "[::]";

    let listener = match TcpListener::bind(format!("[::]:{}", port)) {
        Err(error) => panic!("Could not listen on addr={} port={} error={}", addr, port, error),
        Ok(l) => {
            println!("Listening on {}:{}", addr, port);
            l
        }
    };

    for incoming in listener.incoming() {
        let stream = match incoming {
            Err(error) => {
                println!("Error accepting a new inboud TCP connection: {}", error);
                continue;
            }
            Ok(lk) => lk
        };
        handle_connection(stream)
    }

}

fn handle_connection(mut stream: TcpStream) {
    println!("New connection from {}", stream.peer_addr().unwrap());
    stream.set_read_timeout(Some(Duration::from_secs(5))).expect("Failed to configure read timeout");

    // client starts by sending welcome message:
    let mut buf: [u8; 16] = [0; 16];
    stream.read_exact(&mut buf).expect("Error reading the serial ID");
    let client_serial = bytes_to_hex(&buf);

    let mut buf: [u8; 1] = [0];
    stream.read_exact(&mut buf).expect("Error reading the firmware version");
    let firmware_version = buf[0];

    println!("Handshake done. client_serial={} firmware_version={}", client_serial, firmware_version);
}

fn bytes_to_hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::new();
    for byte in bytes {
        write!(&mut s, "{:X} ", byte).expect("Unable to write");
    }
    s
}
