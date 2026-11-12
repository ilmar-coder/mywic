mod common;

use common::{FakeIvecDevice, G2060_STATUS};
use ecoreset_core::canon::{self, CanonDiagnosis};
use ecoreset_core::errors::{explain_canon, explain_epson_error, search};
use ecoreset_core::ipp::{self, Value};
use ecoreset_core::ivec::{self, CanonUsb};

fn ipp_response(attrs: &[(u8, &str, Vec<Value>)]) -> Vec<u8> {
    let mut body = vec![1, 1, 0, 0, 0, 0, 0, 1, 0x04];
    for (tag, name, values) in attrs {
        for (i, v) in values.iter().enumerate() {
            let n = if i == 0 { *name } else { "" };
            body.push(*tag);
            body.extend((n.len() as u16).to_be_bytes());
            body.extend(n.as_bytes());
            let raw = match v {
                Value::Int(x) => x.to_be_bytes().to_vec(),
                Value::Text(t) => t.as_bytes().to_vec(),
                _ => vec![],
            };
            body.extend((raw.len() as u16).to_be_bytes());
            body.extend(raw);
        }
    }
    body.push(0x03);
    body
}

fn t(s: &str) -> Value {
    Value::Text(s.into())
}

#[test]
fn error_codes_from_official_lists() {
    assert_eq!(explain_canon("1726").title, "Caixa de manutenção cheia");
    assert!(!explain_canon("1300").service);
    assert!(explain_canon("5c30").service);
    assert!(explain_canon("9999").title.contains("não catalogado"));
    assert!(search("1300").iter().any(|p| p.code == "1300"));
    assert!(search("almofada").iter().any(|p| p.brand == "Epson"));
    assert_eq!(explain_epson_error(0x10).code, "0x10");
}

#[test]
fn ipp_parse_and_diagnose() {
    let data = ipp_response(&[
        (0x41, "printer-make-and-model", vec![t("Canon G2060 series")]),
        (0x23, "printer-state", vec![Value::Int(5)]),
        (0x44, "printer-state-reasons", vec![t("media-jam-error"), t("marker-supply-low-warning")]),
        (0x41, "printer-state-message", vec![t("Support Code: 1300")]),
        (0x41, "marker-names", vec![t("Black"), t("Color")]),
        (0x21, "marker-levels", vec![Value::Int(80), Value::Int(15)]),
    ]);
    let (status, attrs) = ipp::parse_response(&data).unwrap();
    assert_eq!(status, 0);
    assert_eq!(attrs["marker-levels"], vec![Value::Int(80), Value::Int(15)]);
    let d = canon::diagnose_attrs(&attrs);
    assert_eq!(d.estado, "Parada");
    assert_eq!(d.motivos, vec!["Papel enroscado", "Tinta baixa"]);
    assert_eq!(d.problemas.iter().map(|p| p.code.as_str()).collect::<Vec<_>>(), vec!["1300"]);
    assert_eq!(d.tintas, vec![("Black".into(), "80%".into()), ("Color".into(), "15%".into())]);
}

#[test]
fn command_sets_and_stopped_without_code() {
    let mut attrs = ipp::Attrs::new();
    attrs.insert("printer-device-id".into(), vec![t("MFG:Canon;CMD:IVEC,URF;SOJ:CHMPu;MDL:G2060 series;CLS:PRINTER;STA:10;")]);
    assert_eq!(canon::command_sets(&attrs), vec!["IVEC", "URF"]);
    let mut stopped = ipp::Attrs::new();
    stopped.insert("printer-state".into(), vec![Value::Int(5)]);
    stopped.insert("printer-state-reasons".into(), vec![t("other-error")]);
    stopped.insert("marker-names".into(), vec![t("Cyan"), t("MC")]);
    stopped.insert("marker-levels".into(), vec![Value::Int(-2), Value::Int(90)]);
    let d: CanonDiagnosis = canon::diagnose_attrs(&stopped);
    assert!(d.mensagem.contains("não informou o código"));
    assert_eq!(d.tintas, vec![("Cyan".into(), "sem sensor".into()), ("MC".into(), "90%".into())]);
}

#[test]
fn ipp_request_and_bjl_jobs() {
    let req = ipp::build_request("ipp://1.2.3.4/ipp/print", 1);
    assert_eq!(&req[..4], b"\x01\x01\x00\x0b");
    assert!(req.ends_with(b"ipp://1.2.3.4/ipp/print\x03"));
    assert_eq!(canon::nozzle_check_job(), b"\x1b[K\x02\x00\x00\x1fBJLSTART\n@TestPrint=NozzleCheck\nBJLEND\n");
    assert!(canon::head_cleaning_job(false).windows(15).any(|w| w == b"@Cleaning=1ALL\n"));
}

#[test]
fn parse_real_g2060_status() {
    let st = ivec::parse_status(G2060_STATUS).unwrap();
    assert_eq!(st.status, "notready");
    assert_eq!(st.support_code, "7800");
    assert_eq!(st.maintenance_box, vec![("MC-G02".into(), Some(90), "warning".into())]);
    assert!(st.error_codes.is_empty());
}

#[test]
fn maintenance_job_layout() {
    let job = String::from_utf8(ivec::maintenance_job("limpeza", "20261025170000", "x").unwrap()).unwrap();
    let ops: Vec<&str> = regex::Regex::new(r"<ivec:operation>(\w+)</ivec:operation>")
        .unwrap()
        .captures_iter(&job)
        .map(|c| c.get(1).unwrap().as_str())
        .collect();
    assert_eq!(ops, vec!["StartJob", "SetJobConfiguration", "Cleaning", "EndJob"]);
    assert!(job.contains("<ivec:param_set servicetype=\"maintenance\"><ivec:inkgroup>all</ivec:inkgroup><ivec:type>regular</ivec:type><ivec:jobID>00000002</ivec:jobID>"));
    assert!(job.contains("<ivec:datetime>20261025170000</ivec:datetime>"));
    assert_eq!(job.matches("<?xml").count(), 4);
    let nozzle = String::from_utf8(ivec::maintenance_job("teste-bicos", "1", "x").unwrap()).unwrap();
    assert!(nozzle.contains("<ivec:type>nozzle_check</ivec:type>") && !nozzle.contains("inkgroup"));
}

#[test]
fn maintenance_refused_on_error_and_sent_when_idle() {
    let dev = FakeIvecDevice::new(G2060_STATUS);
    let written = dev.written.clone();
    let mut c = CanonUsb::from_io(Box::new(dev), "G2060 series");
    assert!(c.maintenance("limpeza", false).unwrap_err().message.contains("7800"));
    assert!(written.lock().unwrap().is_empty());
    let idle = String::from_utf8_lossy(G2060_STATUS)
        .replace("notready", "idle")
        .replace("<ivec:current_support_code>7800", "<ivec:current_support_code>0")
        .replace("<ivec:error_code>7800", "<ivec:error_code>0");
    let dev = FakeIvecDevice::new(idle.as_bytes());
    let written = dev.written.clone();
    let mut c = CanonUsb::from_io(Box::new(dev), "G2060 series");
    c.maintenance("teste-bicos", false).unwrap();
    assert!(String::from_utf8_lossy(written.lock().unwrap().last().unwrap()).contains("nozzle_check"));
}

#[test]
fn diagnose_explains_code() {
    let mut c = CanonUsb::from_io(Box::new(FakeIvecDevice::new(G2060_STATUS)), "G2060 series");
    let d = c.diagnose().unwrap();
    assert_eq!(d.estado, "Não está pronta");
    assert_eq!(d.problemas[0].code, "7800");
    assert_eq!(d.tintas, vec![("Caixa de manutenção MC-G02".into(), "90%".into())]);
}
