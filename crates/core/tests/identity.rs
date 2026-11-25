//! Phone data. The formats are Android 16's real ones (REDMAGIC 11S Pro), but
//! **all the values are examples**: no real identifier goes into the
//! repository. 490154203237518 is the sample IMEI from public documentation;
//! the rest are generated here with Luhn.

use adbscan_core::identity::*;
use adbscan_core::models::Snapshot;

/// Encodes a string the way `service call` prints it (status, length, UTF-16 in
/// 32-bit words, four per line, with the ASCII view).
fn parcel(value: &str) -> String {
    let units: Vec<u16> = value.encode_utf16().collect();
    let mut words = vec![0u32, units.len() as u32];
    for pair in units.chunks(2) {
        let lo = pair[0] as u32;
        let hi = pair.get(1).copied().unwrap_or(0) as u32;
        words.push(lo | (hi << 16));
    }
    let mut out = String::from("Result: Parcel(\t\n");
    for (i, chunk) in words.chunks(4).enumerate() {
        let hex: Vec<String> = chunk.iter().map(|w| format!("{w:08x}")).collect();
        out.push_str(&format!("0x{:08x}: {:<35} '................'\n", i * 16, hex.join(" ")));
    }
    out.push(')');
    out
}

/// Exception as Android returns it (code with wrong arguments).
const EXCEPTION: &str = "Result: Parcel(\t\n0x00000000: fffffffe 0000002f 00000000 00000000 '..../...P.a.r.c.'\n0x00000010: 0065006c 00640020 00740061 00200061 'e.l. .d.a.t.a. .')";

/// Completes a number with its Luhn digit.
fn with_check(base: &str) -> String {
    (0..10).map(|d| format!("{base}{d}")).find(|c| luhn_ok(c)).unwrap()
}

#[test]
fn parcel_strings_round_trip_and_exceptions_are_none() {
    assert_eq!(parse_parcel_string(&parcel("490154203237518")).as_deref(), Some("490154203237518"));
    assert_eq!(parse_parcel_string(&parcel("+52")).as_deref(), Some("+52"));
    assert_eq!(parse_parcel_string(EXCEPTION), None);
    // Null string (length -1) and a short reply on a single line.
    assert_eq!(parse_parcel_string("Result: Parcel(00000000 ffffffff   '........')"), None);
    assert_eq!(parse_parcel_string(""), None);
}

#[test]
fn imei_imsi_and_iccid_are_told_apart_by_shape() {
    let codes = vec!["334020".to_string()];
    assert!(luhn_ok("490154203237518"));
    assert!(is_imei("490154203237518", &codes));
    assert!(!is_imei("490154203237519", &codes), "wrong check digit");
    // A 15-digit IMSI that passes Luhn is not mistaken for an IMEI.
    let imsi_luhn = with_check("33402012345678");
    assert!(luhn_ok(&imsi_luhn));
    assert!(!is_imei(&imsi_luhn, &codes));
    assert!(is_imsi(&imsi_luhn, &codes));
    assert!(!is_imsi("310260123456789", &codes), "another carrier");
    assert!(is_iccid(&with_check("895202123456789012")));
    assert!(!is_iccid("123456789012345678"));
}

fn sample_output(imei0: &str, imei1: &str, imsi: &str, iccid: &str) -> String {
    format!(
        "@@@IMEI0@@@\n{}\n@@@IMEI1@@@\n{}\n@@@IMEI@@@\n{}\n@@@IMSI@@@\n{}\n@@@ICCID@@@\n{}\n@@@LINE1@@@\n{}\n\
         @@@SERIAL@@@\nABC123DEF\n@@@ANDROIDID@@@\n0123456789abcdef\n@@@DEVNAME@@@\nCafé phone\n\
         @@@BTNAME@@@\nnull\n@@@BTMAC@@@\ndc:f0:12:34:ac:56\n\
         @@@SIM@@@\nLOADED,ABSENT\nTELCEL,\n334020,\nmx,\n\
         @@@ACCOUNTS@@@\n    Account {{name=ana@example.com, type=com.google}}\n    Account {{name=ana@example.com, type=com.google}}\n    Account {{name=5512345678, type=com.whatsapp}}\n@@@END@@@\n",
        parcel(imei0),
        parcel(imei1),
        parcel(imei0),
        parcel(imsi),
        parcel(iccid),
        parcel("+52"),
    )
}

#[test]
fn private_info_from_script_output() {
    let imei2 = with_check("35123456789012");
    let imsi = "334020123456789";
    let iccid = with_check("895202123456789012");
    let info = parse_private(&sample_output("490154203237518", &imei2, imsi, &iccid));
    assert_eq!(info.imei[0].as_deref(), Some("490154203237518"));
    assert_eq!(info.imei[1].as_deref(), Some(imei2.as_str()));
    assert_eq!(info.imsi.as_deref(), Some(imsi));
    assert_eq!(info.iccid.as_deref(), Some(iccid.as_str()));
    // “+52” without a number: the SIM does not store the line.
    assert_eq!(info.phone_number, None);
    assert_eq!(info.serial.as_deref(), Some("ABC123DEF"));
    assert_eq!(info.android_id.as_deref(), Some("0123456789abcdef"));
    assert_eq!(info.device_name.as_deref(), Some("Café phone"));
    assert_eq!(info.bluetooth_name, None);
    assert_eq!(info.bluetooth_mac.as_deref(), Some("DC:F0:12:34:AC:56"));
    assert_eq!(info.sims.len(), 2);
    assert_eq!(info.sims[0].operator, "TELCEL");
    assert_eq!(info.sims[0].country, "MX");
    assert!(info.sims[0].present());
    assert!(!info.sims[1].present());
    assert_eq!(info.sims[1].state_label(), "Empty");
    assert_eq!(info.accounts.len(), 2, "no repeats");
    assert_eq!(info.accounts[0].kind_label(), "Google");
    assert_eq!(info.accounts[1].kind_label(), "WhatsApp");
}

#[test]
fn account_types_have_readable_names() {
    let label = |kind: &str| Account { name: "x".into(), kind: kind.into() }.kind_label();
    assert_eq!(label("org.telegram.messenger"), "Telegram");
    assert_eq!(label("www.instagram.com"), "Instagram");
    assert_eq!(label("ai.x.grok"), "Grok");
    assert_eq!(label("com.whatsapp.w4b"), "WhatsApp Business");
    assert_eq!(label("com.google.android.apps.tachyon"), "Google Meet");
}

#[test]
fn wrong_shapes_are_dropped_not_shown() {
    // If another Android version returns something else for those codes, it is
    // not shown: the IMSI in the IMEI slot, a repeated IMEI, exceptions.
    let imsi = with_check("33402012345678");
    let mut text = sample_output(&imsi, "490154203237518", "990000000000000", "1234");
    text = text.replace(&parcel("490154203237518"), EXCEPTION);
    let info = parse_private(&text);
    assert_eq!(info.imei, [None, None]);
    assert_eq!(info.imsi, None);
    assert_eq!(info.iccid, None);
    // IMEI 2 cannot be the same as IMEI 1.
    let same = parse_private(&sample_output("490154203237518", "490154203237518", "", ""));
    assert_eq!(same.imei[1], None);
    // Empty output (no permissions or no telephony): everything empty, no
    // panic.
    assert_eq!(parse_private(""), PrivateInfo::default());
}

#[test]
fn macs_and_randomization() {
    assert_eq!(valid_mac("02:00:00:00:00:00"), None, "Android's placeholder");
    assert_eq!(valid_mac("zz:00:00:00:00:00"), None);
    assert!(is_randomized_mac("A2:11:22:33:44:55"));
    assert!(is_randomized_mac("de:ad:be:ef:00:01"));
    assert!(!is_randomized_mac("DC:F0:12:34:AC:56"));
}

#[test]
fn public_ip_answers() {
    assert_eq!(parse_public_ip("203.0.113.7\n").unwrap().to_string(), "203.0.113.7");
    assert!(parse_public_ip("2001:db8::1").is_ok());
    assert!(parse_public_ip("NO_CURL").unwrap_err().contains("curl"));
    assert!(parse_public_ip("").unwrap_err().contains("Internet"));
    assert!(parse_public_ip("<html>error</html>").is_err());
    assert!(public_ip_script().contains(PUBLIC_IP_SERVICE));
}

#[test]
fn public_sheet_has_no_identifiers() {
    let mut s = Snapshot::new("SERIALSECRETO");
    for (k, v) in [
        ("ro.product.manufacturer", "nubia"),
        ("ro.product.model", "NX809J"),
        ("ro.product.marketname", "REDMAGIC 11S Pro"),
        ("ro.serialno", "SERIALSECRETO"),
        ("ro.build.version.release", "16"),
        ("ro.build.version.sdk", "36"),
        ("ro.soc.manufacturer", "Qualcomm"),
    ] {
        s.props.insert(k.into(), v.into());
    }
    let groups = public_sheet(&s, "");
    let text = sheet_text("Sheet", &groups);
    assert!(text.contains("REDMAGIC 11S Pro"));
    assert!(text.contains("Android 16 (API 36)"));
    assert!(text.contains("Qualcomm"));
    assert!(!text.contains("SERIALSECRETO"), "the public sheet does not carry the serial number");
    assert_eq!(groups.iter().map(|g| g.title).collect::<Vec<_>>(), ["Model", "System", "Hardware", "Battery"]);
}

#[test]
fn commercial_name_falls_back_to_usb_product() {
    let mut s = Snapshot::new("SERIE1");
    s.props.insert("ro.product.model".into(), "NX809J".into());
    let row = |hint: &str| public_sheet(&s, hint)[0].rows.iter().find(|(l, _)| *l == "Marketing name").unwrap().1.clone();
    assert_eq!(row("REDMAGIC 11S Pro"), "REDMAGIC 11S Pro");
    // If the hint is the model or the serial number, it adds nothing.
    assert_eq!(row("NX809J"), "—");
    assert_eq!(row("SERIE1"), "—");
}
