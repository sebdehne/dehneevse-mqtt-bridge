pub fn bytes_to_hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::new();
    for byte in bytes {
        write!(&mut s, "{:02X}", byte).expect("Unable to write");
    }
    s
}
