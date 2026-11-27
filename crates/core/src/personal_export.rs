//! Copy of the phone's contacts, call log and SMS to the computer, without a
//! Google account and without root: Android's shell user may read these
//! providers (`content query`). Read-only: nothing is changed or deleted on
//! the phone. The files stay on this computer: contacts as vCard 3.0 (opens
//! in any address book, imports on any phone), calls and messages as CSV
//! (UTF-8 with a BOM, for spreadsheets).
//!
//! Only text: contact photos and MMS (pictures, group messages) are not
//! copied.

use crate::i18n::tr;

/// What can be copied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Contacts,
    Calls,
    Messages,
}

impl Kind {
    pub const ALL: [Kind; 3] = [Kind::Contacts, Kind::Calls, Kind::Messages];

    pub fn key(self) -> &'static str {
        match self {
            Kind::Contacts => "contacts",
            Kind::Calls => "calls",
            Kind::Messages => "messages",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Kind::Contacts => tr("Contacts"),
            Kind::Calls => tr("Call log"),
            Kind::Messages => tr("Text messages (SMS)"),
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Kind::Contacts => "vcf",
            _ => "csv",
        }
    }

    /// Columns asked for, in this order.
    pub fn columns(self) -> &'static [&'static str] {
        match self {
            Kind::Contacts => &["contact_id", "mimetype", "data2", "data3", "data4", "data1"],
            Kind::Calls => &["date", "type", "duration", "number", "name"],
            Kind::Messages => &["date", "type", "read", "address", "body"],
        }
    }

    /// Arguments for `adb shell` (read-only `content query`).
    pub fn query(self) -> Vec<String> {
        let (uri, sort) = match self {
            Kind::Contacts => ("content://com.android.contacts/data", "contact_id ASC"),
            Kind::Calls => ("content://call_log/calls", "date DESC"),
            Kind::Messages => ("content://sms", "date DESC"),
        };
        ["content", "query", "--uri", uri, "--projection", &self.columns().join(":"), "--sort", sort].iter().map(|s| s.to_string()).collect()
    }
}

/// One row: the value of each column (`None`: `NULL`).
pub type Row = Vec<Option<String>>;

/// `content query` output: `Row: 0 a=1, b=text, with commas, c=NULL`. The
/// columns are known and come in order, so a value ends where `, <next
/// column>=` starts; the last one takes the rest (it may span lines).
pub fn parse_rows(text: &str, columns: &[&str]) -> Vec<Row> {
    let mut starts: Vec<usize> = Vec::new();
    let mut pos = 0;
    for line in text.split_inclusive('\n') {
        if line.starts_with("Row: ") && line[5..].split(' ').next().is_some_and(|n| n.bytes().all(|b| b.is_ascii_digit()) && !n.is_empty()) {
            starts.push(pos);
        }
        pos += line.len();
    }
    let mut out = Vec::new();
    for (i, &start) in starts.iter().enumerate() {
        let end = starts.get(i + 1).copied().unwrap_or(text.len());
        let raw = &text[start..end];
        // After `Row: N `.
        let Some(body) = raw.splitn(3, ' ').nth(2) else { continue };
        let body = body.strip_suffix('\n').unwrap_or(body);
        let body = body.strip_suffix('\r').unwrap_or(body);
        let mut row: Row = Vec::with_capacity(columns.len());
        let mut rest = body;
        for (c, col) in columns.iter().enumerate() {
            let Some(after) = rest.strip_prefix(&format!("{col}=")) else {
                row.clear();
                break;
            };
            let (value, next) = match columns.get(c + 1) {
                Some(next) => {
                    let marker = format!(", {next}=");
                    match after.find(&marker) {
                        Some(at) => (&after[..at], &after[at + 2..]),
                        None => (after, ""),
                    }
                }
                None => (after, ""),
            };
            row.push(if value == "NULL" { None } else { Some(value.to_string()) });
            rest = next;
        }
        if row.len() == columns.len() {
            out.push(row);
        }
    }
    out
}

/// `2026-11-26 16:20:05`, local time, from Unix milliseconds.
fn local_time(ms: &str) -> String {
    let Ok(ms) = ms.trim().parse::<i64>() else { return String::new() };
    let (y, m, d, hh, mm, ss) = crate::format::civil_from_unix(ms / 1000, crate::format::local_offset_secs());
    format!("{y}-{m:02}-{d:02} {hh:02}:{mm:02}:{ss:02}")
}

fn csv_field(v: &str) -> String {
    if v.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", v.replace('"', "\"\""))
    } else {
        v.to_string()
    }
}

fn csv(header: &[&str], rows: impl Iterator<Item = Vec<String>>) -> String {
    // A BOM so that spreadsheets read UTF-8 (accents, emoji).
    let mut out = String::from("\u{feff}");
    out.push_str(&header.join(","));
    out.push_str("\r\n");
    for r in rows {
        out.push_str(&r.iter().map(|v| csv_field(v)).collect::<Vec<_>>().join(","));
        out.push_str("\r\n");
    }
    out
}

fn get(row: &Row, i: usize) -> &str {
    row.get(i).and_then(|v| v.as_deref()).unwrap_or("")
}

/// `CallLog.Calls.TYPE`.
fn call_type(t: &str) -> &'static str {
    match t.trim() {
        "1" => tr("Incoming"),
        "2" => tr("Outgoing"),
        "3" => tr("Missed"),
        "4" => tr("Voicemail"),
        "5" => tr("Rejected"),
        "6" => tr("Blocked"),
        "7" => tr("Answered elsewhere"),
        _ => "",
    }
}

/// `Telephony.Sms.TYPE`.
fn sms_type(t: &str) -> &'static str {
    match t.trim() {
        "1" => tr("Received"),
        "2" => tr("Sent"),
        "3" => tr("Draft"),
        "4" => tr("Outbox"),
        "5" => tr("Failed"),
        "6" => tr("Queued"),
        _ => "",
    }
}

pub fn calls_csv(rows: &[Row]) -> String {
    let header = [tr("Date"), tr("Type"), tr("Duration (s)"), tr("Number"), tr("Name")];
    csv(&header, rows.iter().map(|r| vec![local_time(get(r, 0)), call_type(get(r, 1)).to_string(), get(r, 2).to_string(), get(r, 3).to_string(), get(r, 4).to_string()]))
}

pub fn messages_csv(rows: &[Row]) -> String {
    let header = [tr("Date"), tr("Type"), tr("Read"), tr("Number"), tr("Message")];
    csv(
        &header,
        rows.iter().map(|r| {
            let read = match get(r, 2) {
                "1" => tr("Yes"),
                "0" => tr("No"),
                _ => "",
            };
            vec![local_time(get(r, 0)), sms_type(get(r, 1)).to_string(), read.to_string(), get(r, 3).to_string(), get(r, 4).to_string()]
        }),
    )
}

/// vCard text value: `\`, `,`, `;` and line breaks escaped.
fn vcard_escape(v: &str) -> String {
    v.replace('\\', "\\\\").replace(',', "\\,").replace(';', "\\;").replace("\r\n", "\\n").replace('\n', "\\n")
}

/// `Phone.TYPE` → vCard `TYPE`.
fn phone_type(t: &str) -> &'static str {
    match t {
        "1" => "HOME",
        "2" => "CELL",
        "3" => "WORK",
        "4" | "5" => "FAX",
        "6" => "PAGER",
        _ => "VOICE",
    }
}

fn email_type(t: &str) -> &'static str {
    match t {
        "1" => "HOME",
        "2" => "WORK",
        _ => "INTERNET",
    }
}

/// Contacts (rows of the `data` table, grouped by `contact_id`) as vCard 3.0.
/// Returns the text and how many contacts it has.
pub fn contacts_vcf(rows: &[Row]) -> (String, usize) {
    // Rows come sorted by contact; kept in order of appearance.
    let mut order: Vec<&str> = Vec::new();
    let mut by_contact: std::collections::HashMap<&str, Vec<&Row>> = std::collections::HashMap::new();
    for r in rows {
        let id = get(r, 0);
        if id.is_empty() {
            continue;
        }
        if !by_contact.contains_key(id) {
            order.push(id);
        }
        by_contact.entry(id).or_default().push(r);
    }
    let mut out = String::new();
    let mut count = 0;
    for id in order {
        let mut lines: Vec<String> = Vec::new();
        let mut name: Option<(String, String, String)> = None;
        for r in &by_contact[id] {
            let (mime, d2, d3, d4, d1) = (get(r, 1), get(r, 2), get(r, 3), get(r, 4), get(r, 5));
            let kind = mime.rsplit('/').next().unwrap_or("");
            match kind {
                "name" if name.is_none() => name = Some((d1.to_string(), d2.to_string(), d3.to_string())),
                "phone_v2" if !d1.is_empty() => lines.push(format!("TEL;TYPE={}:{}", phone_type(d2), vcard_escape(d1))),
                "email_v2" if !d1.is_empty() => lines.push(format!("EMAIL;TYPE={}:{}", email_type(d2), vcard_escape(d1))),
                "organization" if !d1.is_empty() || !d4.is_empty() => {
                    if !d1.is_empty() {
                        lines.push(format!("ORG:{}", vcard_escape(d1)));
                    }
                    if !d4.is_empty() {
                        lines.push(format!("TITLE:{}", vcard_escape(d4)));
                    }
                }
                "postal-address_v2" if !d1.is_empty() => lines.push(format!("ADR:;;{};;;;", vcard_escape(d1))),
                "nickname" if !d1.is_empty() => lines.push(format!("NICKNAME:{}", vcard_escape(d1))),
                "note" if !d1.is_empty() => lines.push(format!("NOTE:{}", vcard_escape(d1))),
                "website" if !d1.is_empty() => lines.push(format!("URL:{}", vcard_escape(d1))),
                // Birthday (type 3).
                "contact_event" if d2 == "3" && !d1.is_empty() => lines.push(format!("BDAY:{}", vcard_escape(d1))),
                _ => {}
            }
        }
        let (full, given, family) = name.unwrap_or_default();
        // A contact without a name or anything to call or write is skipped.
        if full.is_empty() && lines.is_empty() {
            continue;
        }
        let shown = if full.is_empty() { lines.first().and_then(|l| l.split_once(':')).map(|(_, v)| v.to_string()).unwrap_or_default() } else { vcard_escape(&full) };
        out.push_str("BEGIN:VCARD\r\nVERSION:3.0\r\n");
        out.push_str(&format!("FN:{shown}\r\nN:{};{};;;\r\n", vcard_escape(&family), vcard_escape(&given)));
        lines.dedup();
        for l in lines {
            out.push_str(&l);
            out.push_str("\r\n");
        }
        out.push_str("END:VCARD\r\n");
        count += 1;
    }
    (out, count)
}

/// Builds the file for `kind` from the query's output: (text, how many).
pub fn build(kind: Kind, output: &str) -> (String, usize) {
    let rows = parse_rows(output, kind.columns());
    match kind {
        Kind::Contacts => contacts_vcf(&rows),
        Kind::Calls => (calls_csv(&rows), rows.len()),
        Kind::Messages => (messages_csv(&rows), rows.len()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_with_commas_and_lines() {
        let out = "Row: 0 date=1795700000000, type=1, read=1, address=+5215550001, body=Hola, ¿cómo estás?\nTodo bien, gracias\nRow: 1 date=1795600000000, type=2, read=1, address=NULL, body=OK\n";
        let rows = parse_rows(out, Kind::Messages.columns());
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0][4].as_deref(), Some("Hola, ¿cómo estás?\nTodo bien, gracias"));
        assert_eq!(rows[1][3], None);
        assert!(parse_rows("No result found.\n", Kind::Messages.columns()).is_empty());
        let csv = messages_csv(&rows);
        assert!(csv.starts_with('\u{feff}'));
        assert!(csv.contains(",\"Hola, ¿cómo estás?\nTodo bien, gracias\"\r\n"));
        assert!(csv.contains("Received") && csv.contains("Sent"));
    }

    #[test]
    fn calls() {
        let out = "Row: 0 date=1795700000000, type=3, duration=0, number=5550001, name=NULL\nRow: 1 date=1795600000000, type=2, duration=125, number=5550002, name=Ana, \"la vecina\"\n";
        let (text, n) = build(Kind::Calls, out);
        assert_eq!(n, 2);
        assert!(text.contains(",Missed,0,5550001,\r\n"));
        assert!(text.contains(",Outgoing,125,5550002,\"Ana, \"\"la vecina\"\"\"\r\n"));
    }

    #[test]
    fn contacts_as_vcard() {
        let out = "Row: 0 contact_id=7, mimetype=vnd.android.cursor.item/name, data2=Ana, data3=López, data4=NULL, data1=Ana López\n\
Row: 1 contact_id=7, mimetype=vnd.android.cursor.item/phone_v2, data2=2, data3=NULL, data4=NULL, data1=+52 55 5000 0001\n\
Row: 2 contact_id=7, mimetype=vnd.android.cursor.item/email_v2, data2=1, data3=NULL, data4=NULL, data1=ana@example.com\n\
Row: 3 contact_id=7, mimetype=vnd.android.cursor.item/note, data2=NULL, data3=NULL, data4=NULL, data1=Línea 1; línea 2\nfin\n\
Row: 4 contact_id=9, mimetype=vnd.android.cursor.item/phone_v2, data2=1, data3=NULL, data4=NULL, data1=5550009\n\
Row: 5 contact_id=11, mimetype=vnd.android.cursor.item/photo, data2=NULL, data3=NULL, data4=NULL, data1=NULL\n";
        let (vcf, n) = build(Kind::Contacts, out);
        assert_eq!(n, 2);
        assert!(vcf.contains("BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Ana López\r\nN:López;Ana;;;\r\n"));
        assert!(vcf.contains("TEL;TYPE=CELL:+52 55 5000 0001\r\n"));
        assert!(vcf.contains("EMAIL;TYPE=HOME:ana@example.com\r\n"));
        assert!(vcf.contains("NOTE:Línea 1\\; línea 2\\nfin\r\n"));
        // Without a name: the number is shown.
        assert!(vcf.contains("FN:5550009\r\n"));
        assert_eq!(vcf.matches("END:VCARD").count(), 2);
    }

    #[test]
    fn queries_only_read() {
        for k in Kind::ALL {
            let q = k.query();
            assert_eq!(&q[..2], ["content", "query"]);
            assert!(!q.iter().any(|a| a == "delete" || a == "update" || a == "insert"));
        }
    }
}
