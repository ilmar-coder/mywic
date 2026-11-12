use ecoreset_core::models::{builtin, by_name};
use ecoreset_core::printer::write_frame;
use ecoreset_core::wiclog::{self, TRIAL_PERCENT};

fn hex_line(n: usize, rk: [u8; 2], wk: &[u8; 8], a: u16, v: u8) -> String {
    let f: Vec<String> = write_frame(rk, wk, a, v).iter().map(|b| format!("{b:02X}")).collect();
    format!("INFO RESET_DATA RESET DATA: {n} - {} REAL", f.join(" "))
}

fn make_log(writes: &[(u16, u8)], rk: [u8; 2], wk: &[u8; 8], model: &str) -> String {
    let mut lines = vec![format!("INFO RESET_GUID RESET GUID: {model} 123456 KEY trial"), "INFO Reset started.".into()];
    lines.extend(writes.iter().enumerate().map(|(i, (a, v))| hex_line(i, rk, wk, *a, *v)));
    lines.push("INFO Reset complete".into());
    lines.join("\n")
}

/// Reset trial de um modelo desconhecido: 2 contadores em 80%, reais zerados,
/// níveis de manutenção e uma flag em 0x100.
const TRIAL: &[(u16, u8)] = &[(20, 104), (21, 61), (22, 68), (23, 16), (24, 0), (25, 0), (59, 0), (60, 94), (61, 94), (0x100, 0x3E)];

#[test]
fn parse_keys_and_writes() {
    let p = wiclog::parse(&make_log(TRIAL, [11, 22], b"Testkey1", "XP-9999")).unwrap();
    assert_eq!(p.model.as_deref(), Some("XP-9999"));
    assert_eq!((p.read_key, p.write_key), (Some([11, 22]), Some(*b"Testkey1")));
    assert_eq!(p.writes, TRIAL);
}

#[test]
fn decimal_oid_lines() {
    let oid = ecoreset_core::printer::frame_oid(&write_frame([11, 22], b"Testkey1", 300, 7));
    let p = wiclog::parse(&format!("DEBUG snmp get {oid}")).unwrap();
    assert_eq!(p.writes, vec![(300, 7)]);
}

#[test]
fn learn_trial_zeroes_counters_and_estimates_divider() {
    let (m, warnings) = wiclog::learn(&make_log(TRIAL, [11, 22], b"Testkey1", "XP-9999"), None, TRIAL_PERCENT, &builtin()).unwrap();
    assert_eq!(m.name, "XP-9999");
    assert_eq!(m.raw_waste_reset, vec![(20, 0), (21, 0), (22, 0), (23, 0), (24, 0), (25, 0), (59, 0), (60, 94), (61, 94)]);
    let main = m.main_waste.unwrap();
    assert_eq!(main.oids, vec![20, 21]);
    assert_eq!(main.divider, (0x3D68 as f64 / 80.0 * 100.0).round() / 100.0);
    assert_eq!(m.borderless_waste.unwrap().oids, vec![22, 23]);
    assert!(warnings.iter().any(|w| w.contains("0x100")));
}

#[test]
fn same_keys_copy_model_other_keys_estimate() {
    let l3150 = by_name("L3150", &builtin()).unwrap().clone();
    let writes: Vec<(u16, u8)> = l3150.raw_waste_reset.iter().map(|(a, v)| (*a, if *v == 94 { 94 } else { 7 })).collect();
    let (m, w) = wiclog::learn(&make_log(&writes, [151, 7], b"Maribaya", "L3110"), None, 80.0, &builtin()).unwrap();
    assert_eq!(m.main_waste, l3150.main_waste);
    assert!(w.iter().any(|x| x.contains("L3150")));
    let mut writes: Vec<(u16, u8)> = l3150.raw_waste_reset.iter().map(|(a, v)| (*a, if *v == 94 { 94 } else { 0 })).collect();
    writes.extend([(48, 0x40), (49, 0x1F)]);
    let (m, _) = wiclog::learn(&make_log(&writes, [1, 2], b"Otherkey", "L9999"), None, 80.0, &builtin()).unwrap();
    let main = m.main_waste.unwrap();
    assert_eq!(main.oids, vec![48, 49, 47]);
    assert_eq!(main.divider, (0x1F40 as f64 / 80.0 * 100.0).round() / 100.0);
}

#[test]
fn refuses_empty_and_two_printers() {
    assert!(wiclog::learn("INFO nada\n7C 7C 07 00 0B 16 41 BE A0 30 00", None, 80.0, &builtin()).unwrap_err().message.contains("nenhuma gravação"));
    let two = format!("{}\n{}", make_log(&TRIAL[..2], [11, 22], b"Testkey1", "A"), make_log(&TRIAL[..2], [1, 2], b"Testkey1", "A"));
    assert!(wiclog::parse(&two).unwrap_err().message.contains("mais de uma"));
}

/// Log real do WICReset 5.59 (Mac): reset trial pelo USB numa L3250 com o
/// firmware XF26P8. Sem as leituras do número de série nem o endereço de rede.
#[test]
fn real_log_matches_builtin_l3250() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/wicreset_l3250_trial.log");
    let models = builtin();
    let (m, warnings) = wiclog::learn_file(path, None, 80.0, &models).unwrap();
    let r = by_name("ET-2812", &models).unwrap();
    assert_eq!(m.name, "L3250 SERIES");
    assert_eq!((m.read_key, m.write_key), (r.read_key, r.write_key));
    assert_eq!(m.raw_waste_reset, r.raw_waste_reset);
    assert_eq!(m.waste_reset_commit, vec![(0x100, 0xFE)]);
    assert!(warnings.iter().any(|w| w.contains("ET-2812")));
    let p = wiclog::parse(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert!(p.reads.contains(&0x100)); // leitura antes de limpar o bit 0
    for (_, c) in r.counters() {
        let raw = c.oids.iter().rev().fold(0u64, |acc, a| (acc << 8) | p.writes.iter().rev().find(|(x, _)| x == a).unwrap().1 as u64);
        assert!((raw as f64 / c.divider - 80.0).abs() < 0.1, "o trial deixa em 80%");
    }
}
