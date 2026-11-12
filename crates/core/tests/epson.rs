mod common;

use std::sync::atomic::AtomicBool;

use common::FakePrinter;
use ecoreset_core::models::{builtin, find_model};
use ecoreset_core::printer::{Backup, frame_oid, load_image, parse_status, read_frame, shift_key, waste_percent, write_frame};
use ecoreset_core::{keyfinder, snapshot, snmp};

fn bytes_hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02X}")).collect::<Vec<_>>().join(" ")
}

#[test]
fn read_oid_matches_documented_example() {
    assert_eq!(frame_oid(&read_frame([25, 7], 173)), "1.3.6.1.4.1.1248.1.2.2.44.1.1.2.1.124.124.7.0.25.7.65.190.160.173.0");
}

#[test]
fn write_frame_matches_documented_bytes() {
    assert_eq!(
        bytes_hex(&write_frame([73, 8], b"Arantifo", 48, 0x1A)),
        "7C 7C 10 00 49 08 42 BD 21 30 00 1A 42 73 62 6F 75 6A 67 70"
    );
}

#[test]
fn shift_key_and_waste_percent() {
    assert_eq!(bytes_hex(&shift_key(b"Gazania*")), "48 62 7B 62 6F 6A 62 2B");
    assert_eq!(waste_percent(&[0xA4, 0x2A], 100.0), (0x2AA4 as f64 / 100.0 * 100.0).round() / 100.0);
}

#[test]
fn oid_roundtrip_and_long_length() {
    let oid = "1.3.6.1.4.1.1248.255.128.300.70000";
    let enc = snmp::encode_oid(oid).unwrap();
    assert_eq!(snmp::decode_oid(&enc[2..]), oid);
    let long = format!("1.3.6.1.{}", vec!["200"; 100].join("."));
    let packet = snmp::build_get(&long, "public", 1).unwrap();
    let (_, _, end) = snmp::decode_tlv(&packet, 0).unwrap();
    assert_eq!(end, packet.len());
}

#[test]
fn find_model_and_consistency() {
    let m = builtin();
    assert_eq!(find_model("EPSON L3150 Series", &m).unwrap().name, "L3150");
    assert_eq!(find_model("EPSON L4150 Series", &m).unwrap().name, "L4160");
    assert_eq!(find_model("EPSON L395 Series", &m).unwrap().name, "ET-2600");
    assert_eq!(find_model("EPSON L3250 Series", &m).unwrap().name, "ET-2812");
    assert!(find_model("EPSON L31500", &m).is_none());
    assert_eq!(m.len(), 14);
    assert!(m.iter().all(|x| x.main_waste.is_some()));
}

#[test]
fn detect_levels_and_device_id() {
    let fake = FakePrinter::l3150();
    let mut p = fake.serve_snmp();
    let (text, name) = p.detect(&builtin()).unwrap();
    assert_eq!((text.as_str(), name.as_deref()), ("EPSON L3150 Series", Some("L3150")));
    let levels = p.waste_levels().unwrap();
    assert_eq!(levels[0], ("principal".into(), (0x0C10 as f64 / 63.46 * 100.0).round() / 100.0));
    assert_eq!(levels[1].0, "sem margem");
    assert_eq!(p.device_id().unwrap()["MDL"], "EPSON L3150 Series");
}

#[test]
fn reset_writes_plan_backup_and_restore() {
    let fake = FakePrinter::l3150();
    let mut p = fake.serve_snmp();
    p.set_model(&builtin()[1]);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("b.json");
    p.reset_waste(Some(path.to_str().unwrap())).unwrap();
    let backup = Backup::load(path.to_str().unwrap()).unwrap();
    assert!(backup.eeprom.contains(&(48, 0x10)));
    for (a, v) in &builtin()[1].raw_waste_reset {
        assert_eq!(fake.ee(*a), *v);
    }
    assert_eq!(p.waste_levels().unwrap()[0].1, 0.0);
    p.restore(&backup).unwrap();
    assert_eq!(fake.ee(49), 0x0C);
}

#[test]
fn wrong_key_and_locked_firmware() {
    let fake = FakePrinter::l3150();
    let mut p = fake.serve_snmp();
    p.set_model(&builtin()[0]); // L4160: chave errada para esta impressora
    assert!(p.read_eeprom(48).is_err());
    p.set_model(&builtin()[1]);
    fake.set(|s| s.locked = "write");
    assert!(p.write_eeprom(48, 0, true).unwrap_err().message.contains("recusou a gravação"));
}

#[test]
fn temporary_reset_and_serial_fields() {
    let fake = FakePrinter::l3150();
    fake.set(|s| s.locked = "all");
    let mut p = fake.serve_snmp();
    assert_eq!(p.temporary_reset(None).unwrap(), "X5NY012345");
    assert_eq!(fake.0.lock().unwrap().temp_resets, 1);
    assert!(p.temporary_reset(Some("ERRADO1234")).is_err());
    // L3250 real: série no campo 0x40
    let st = parse_status(b"\x00@BDC ST2\r\n\x10\x00\x01\x01\x04\x19\x01\x00\x40\x0aXAAB402930");
    assert_eq!(st[&0x40], b"XAAB402930");
    fake.set(|s| s.serial_field = 0x40);
    assert_eq!(p.serial_number().unwrap(), "X5NY012345");
}

#[test]
fn l3250_uses_et2812_and_commit_clears_bit0() {
    let fake = FakePrinter::new("EPSON L3250 Series", [74, 54], b"Maribaya", &[(48, 5), (0x100, 0x13), (254, 2)]);
    let mut p = fake.serve_snmp();
    assert_eq!(p.detect(&builtin()).unwrap().1.as_deref(), Some("ET-2812"));
    assert!(p.waste_levels().unwrap().iter().any(|(l, _)| l == "terceira"));
    p.reset_waste(None).unwrap();
    assert_eq!(fake.ee(0x100), 0x12);
    assert_eq!(fake.ee(254), 0);
    assert_eq!(fake.ee(255), 94);
}

#[test]
fn diagnose_reads_status() {
    let fake = FakePrinter::l3150();
    let mut p = fake.serve_snmp();
    let d = p.diagnose().unwrap();
    assert_eq!(d.estado, "Pronta");
    assert!(d.problemas.is_empty());
}

#[test]
fn json_image_plan_protects_serial_and_mac() {
    let fake = FakePrinter::new("EPSON L3250 Series", [74, 54], b"Maribaya", &[(48, 5), (49, 6), (1604, b'X'), (0x780, 0xAA)]);
    let mut p = fake.serve_snmp();
    p.detect(&builtin()).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("img.json");
    std::fs::write(&path, r#"{"model": "ET-2812", "eeprom": {"48": 0, "49": 6, "1604": 89, "0x780": 1}}"#).unwrap();
    let (image, model) = load_image(path.to_str().unwrap(), 0).unwrap();
    assert_eq!(model.as_deref(), Some("ET-2812"));
    let (changes, skipped) = p.plan_image(&image, false).unwrap();
    assert_eq!(changes, vec![(48, 5, 0)]);
    assert_eq!(skipped, vec![1604, 0x780]);
    let backup = dir.path().join("antes.json");
    p.write_image(&changes, Some(backup.to_str().unwrap())).unwrap();
    assert_eq!(fake.ee(48), 0);
    assert_eq!(Backup::load(backup.to_str().unwrap()).unwrap().eeprom, vec![(48, 5)]);
    let bin = dir.path().join("dump.bin");
    std::fs::write(&bin, [1, 2, 3]).unwrap();
    let (image, _) = load_image(bin.to_str().unwrap(), 47).unwrap();
    assert_eq!(image.into_iter().collect::<Vec<_>>(), vec![(47, 1), (48, 2), (49, 3)]);
}

#[test]
fn snapshot_take_compare_and_suggest() {
    let fake = FakePrinter::l3150();
    let mut p = fake.serve_snmp();
    p.set_model(&builtin()[1]);
    let before = snapshot::take(&mut p, 0, 63, "antes", &mut |_, _| {}).unwrap();
    assert_eq!(before.eeprom.len(), 64);
    fake.set(|s| {
        s.eeprom.insert(48, 0);
        s.eeprom.insert(49, 0);
        s.eeprom.insert(54, 94);
        s.eeprom.insert(60, 7);
    });
    let after = snapshot::take(&mut p, 0, 63, "depois", &mut |_, _| {}).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.json");
    after.save(path.to_str().unwrap()).unwrap();
    assert_eq!(snapshot::Snapshot::load(path.to_str().unwrap()).unwrap(), after);
    // a leitura também serve de imagem para regravar
    assert_eq!(load_image(path.to_str().unwrap(), 0).unwrap().0.len(), 64);
    let (changes, warnings) = snapshot::compare(&before, &after);
    assert!(warnings.is_empty());
    assert_eq!(changes, vec![(48, 0x10, 0), (49, 0x0C, 0), (54, 0, 94), (60, 0, 7)]);
    assert_eq!(snapshot::classify(48, 0x10, 0), "provável contador zerado");
    let s = snapshot::suggest_model(&before, &after, &changes, &[]);
    assert_eq!(s["raw_waste_reset"], serde_json::json!({"48": 0, "49": 0, "54": 94}));
    assert_eq!(s["ignorados"], serde_json::json!([60]));
}

#[test]
fn snapshot_wrong_key_stops_early_and_warns_on_other_printer() {
    let fake = FakePrinter::l3150();
    let mut p = fake.serve_snmp();
    p.read_key = Some([1, 1]);
    assert!(snapshot::take(&mut p, 0, 100, "", &mut |_, _| {}).unwrap_err().message.contains("16 primeiros"));
    let a = snapshot::Snapshot { serial: "A".into(), ..Default::default() };
    let b = snapshot::Snapshot { serial: "B".into(), ..Default::default() };
    assert!(snapshot::compare(&a, &b).1[0].contains("diferentes"));
}

#[test]
fn keyfinder_known_new_cancel_any_and_write() {
    let models = builtin();
    let fake = FakePrinter::l3150();
    let mut p = fake.serve_snmp();
    let r = keyfinder::find_read_key(&mut p, &models, 0, &mut |_, _, _| {}, &AtomicBool::new(false)).unwrap().unwrap();
    assert_eq!(r.read_key, [151, 7]);
    assert!(r.known && r.confirmed && !r.accepts_any);
    assert!(r.tries <= 10);
    assert_eq!(keyfinder::find_write_key(&mut p, &models, 0).unwrap(), Some(*b"Maribaya"));
    // chave nova: precisa varrer
    let fake = FakePrinter::new("EPSON XP-1", [3, 0], b"Novakey1", &[]);
    let mut p = fake.serve_snmp();
    let r = keyfinder::find_read_key(&mut p, &models, 0, &mut |_, _, _| {}, &AtomicBool::new(false)).unwrap().unwrap();
    assert_eq!(r.read_key, [3, 0]);
    assert!(!r.known);
    assert_eq!(keyfinder::find_write_key(&mut p, &models, 0).unwrap(), None);
    // cancelar volta a chave de antes
    p.read_key = Some([9, 9]);
    let cancel = AtomicBool::new(true);
    assert!(keyfinder::find_read_key(&mut p, &models, 0, &mut |_, _, _| {}, &cancel).unwrap().is_none());
    assert_eq!(p.read_key, Some([9, 9]));
    // firmware que aceita qualquer chave
    fake.set(|s| s.any_key = true);
    let r = keyfinder::find_read_key(&mut p, &models, 0, &mut |_, _, _| {}, &AtomicBool::new(false)).unwrap().unwrap();
    assert!(r.accepts_any);
    let keys = keyfinder::candidate_keys(&models);
    let mut unique = keys.clone();
    unique.sort();
    unique.dedup();
    assert_eq!((keys.len(), unique.len()), (65536, 65536));
}

#[test]
fn snmp_timeout_is_reported() {
    let sock = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let mut c = snmp::SnmpClient::new("127.0.0.1");
    c.port = sock.local_addr().unwrap().port();
    c.timeout = std::time::Duration::from_millis(200);
    c.retries = 0;
    assert!(c.get("1.3.6.1.2.1.1.1.0").unwrap_err().message.contains("sem resposta"));
}
