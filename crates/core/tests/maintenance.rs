use std::io::Read;
use std::net::TcpListener;

use ecoreset_core::maintenance::{self, EXIT_PACKET_MODE};

#[test]
fn jobs_end_like_escputil() {
    let mut want = EXIT_PACKET_MODE.to_vec();
    want.extend(b"\x1b@\x1b@\x1b(R\x08\x00\x00REMOTE1NC\x02\x00\x00\x00\x1b\x00\x00\x00\x1b\x00\x0c\x1b\x00\x1b\x00");
    let job = maintenance::nozzle_check_job();
    assert_eq!(job, want);
    // nada de ESC @ logo depois do modo remoto (suspeito do looping no Windows)
    assert!(!job.windows(6).any(|w| w == b"\x1b\x00\x00\x00\x1b@"));
    assert!(maintenance::head_cleaning_job().windows(13).any(|w| w == b"REMOTE1CH\x02\x00\x00\x00"));
}

#[test]
fn send_raw_network() {
    let server = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = server.local_addr().unwrap().port();
    let t = std::thread::spawn(move || {
        let (mut s, _) = server.accept().unwrap();
        let mut got = Vec::new();
        s.read_to_end(&mut got).unwrap();
        got
    });
    // a função usa a porta 9100; aqui conectamos direto para conferir o envio
    let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    std::io::Write::write_all(&mut s, b"abc").unwrap();
    drop(s);
    assert_eq!(t.join().unwrap(), b"abc");
    assert!(maintenance::send_raw_network("256.1.1.1", b"x").is_err());
}

#[test]
fn cups_queues_by_serial_then_model() {
    let text = "dispositivo para Canon_G2060_series_2: ippusb://Canon%20G2060%20series._ipp._tcp.local./?uuid=0\n\
                device for EPSON_L3250_Series: dnssd://EPSON%20L3250%20Series._ipps._tcp.local./\n\
                dispositivo para EPSON_L3250_Series_2: usb://EPSON/L3250%20Series?serial=584141423430323930\n\
                device for EPSON_L3250_Outra: usb://EPSON/L3250%20Series?serial=ZZZ";
    let devices = maintenance::parse_lpstat_v(text);
    assert_eq!(devices.len(), 4);
    let m = |s: &str, model: &str| maintenance::match_cups_queues(&devices, s, model);
    assert_eq!(m("584141423430323930", ""), vec!["EPSON_L3250_Series_2"]);
    assert_eq!(m("XAAB40290", ""), vec!["EPSON_L3250_Series_2"]); // hex no CUPS
    assert_eq!(m("", "L3250 SERIES"), vec!["EPSON_L3250_Series_2", "EPSON_L3250_Outra"]);
}
