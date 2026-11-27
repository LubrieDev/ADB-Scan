//! Local APK reading without running anything: package, version, SDK,
//! permissions and display name (resolved from `resources.arsc`, preferring the
//! interface language). Only the binary manifest and the resource table are
//! read.

use crate::{i18n::tr, t};
use std::collections::HashMap;
use std::io::Read;
use std::path::Path;

use crate::error::{Error, Result};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ApkInfo {
    pub file_name: String,
    pub size_bytes: u64,
    pub package: String,
    pub label: String,
    pub version_name: String,
    pub version_code: String,
    pub min_sdk: Option<u32>,
    pub target_sdk: Option<u32>,
    pub permissions: Vec<String>,
    pub native_abis: Vec<String>,
    pub debuggable: bool,
    pub split: bool,
}

impl ApkInfo {
    /// Permissions considered dangerous (they require an explicit grant).
    pub fn dangerous_permissions(&self) -> Vec<&str> {
        self.permissions.iter().map(String::as_str).filter(|p| is_dangerous_permission(p)).collect()
    }
}

pub fn is_dangerous_permission(p: &str) -> bool {
    const DANGEROUS: [&str; 34] = [
        "READ_CONTACTS",
        "WRITE_CONTACTS",
        "GET_ACCOUNTS",
        "READ_CALENDAR",
        "WRITE_CALENDAR",
        "CAMERA",
        "RECORD_AUDIO",
        "ACCESS_FINE_LOCATION",
        "ACCESS_COARSE_LOCATION",
        "ACCESS_BACKGROUND_LOCATION",
        "READ_PHONE_STATE",
        "READ_PHONE_NUMBERS",
        "CALL_PHONE",
        "READ_CALL_LOG",
        "WRITE_CALL_LOG",
        "PROCESS_OUTGOING_CALLS",
        "BODY_SENSORS",
        "SEND_SMS",
        "RECEIVE_SMS",
        "READ_SMS",
        "RECEIVE_MMS",
        "READ_EXTERNAL_STORAGE",
        "WRITE_EXTERNAL_STORAGE",
        "MANAGE_EXTERNAL_STORAGE",
        "READ_MEDIA_IMAGES",
        "READ_MEDIA_VIDEO",
        "READ_MEDIA_AUDIO",
        "ACTIVITY_RECOGNITION",
        "SYSTEM_ALERT_WINDOW",
        "REQUEST_INSTALL_PACKAGES",
        "BIND_ACCESSIBILITY_SERVICE",
        "BIND_DEVICE_ADMIN",
        "BIND_NOTIFICATION_LISTENER_SERVICE",
        "QUERY_ALL_PACKAGES",
    ];
    let short = p.rsplit('.').next().unwrap_or(p);
    p.starts_with("android.permission.") && DANGEROUS.contains(&short)
}

// ---------------------------------------------------------------------------
// Binary reading
// ---------------------------------------------------------------------------

fn u16_at(b: &[u8], o: usize) -> Option<u16> {
    b.get(o..o + 2).map(|s| u16::from_le_bytes([s[0], s[1]]))
}
fn u32_at(b: &[u8], o: usize) -> Option<u32> {
    b.get(o..o + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

/// String pool (`ResStringPool`), UTF-8 or UTF-16.
pub fn parse_string_pool(b: &[u8], start: usize) -> Option<Vec<String>> {
    let header_size = u16_at(b, start + 2)? as usize;
    let count = u32_at(b, start + 8)? as usize;
    let flags = u32_at(b, start + 16)?;
    let strings_start = u32_at(b, start + 20)? as usize;
    let utf8 = flags & 0x100 != 0;
    if count > 1_000_000 {
        return None;
    }
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let off = u32_at(b, start + header_size + i * 4)? as usize;
        let mut p = start + strings_start + off;
        if utf8 {
            // UTF-16 length (ignored) and length in bytes
            let skip = |p: &mut usize| -> Option<usize> {
                let a = *b.get(*p)? as usize;
                *p += 1;
                if a & 0x80 != 0 {
                    let c = *b.get(*p)? as usize;
                    *p += 1;
                    Some(((a & 0x7f) << 8) | c)
                } else {
                    Some(a)
                }
            };
            skip(&mut p)?;
            let len = skip(&mut p)?;
            out.push(String::from_utf8_lossy(b.get(p..p + len)?).into_owned());
        } else {
            let mut len = u16_at(b, p)? as usize;
            p += 2;
            if len & 0x8000 != 0 {
                len = ((len & 0x7fff) << 16) | u16_at(b, p)? as usize;
                p += 2;
            }
            let units: Vec<u16> = (0..len).filter_map(|k| u16_at(b, p + k * 2)).collect();
            out.push(String::from_utf16_lossy(&units));
        }
    }
    Some(out)
}

#[derive(Debug, Clone, Copy)]
struct TypedValue {
    kind: u8,
    data: u32,
}

const TYPE_REFERENCE: u8 = 0x01;
const TYPE_STRING: u8 = 0x03;
const TYPE_INT_DEC: u8 = 0x10;
const TYPE_INT_HEX: u8 = 0x11;
const TYPE_INT_BOOLEAN: u8 = 0x12;
/// Colours: #AARRGGBB, #RRGGBB, #ARGB and #RGB (the value is always ARGB).
const TYPE_COLOR_ARGB8: u8 = 0x1c;
const TYPE_COLOR_RGB4: u8 = 0x1f;

#[derive(Debug, Clone)]
struct Attr {
    name: String,
    res_id: Option<u32>,
    raw: Option<String>,
    value: TypedValue,
}

#[derive(Debug, Clone)]
struct Element {
    name: String,
    attrs: Vec<Attr>,
    /// Depth in the tree (`manifest` = 0, `application` = 1…).
    depth: usize,
}

const ATTR_LABEL: u32 = 0x0101_0001;
const ATTR_NAME: u32 = 0x0101_0003;
const ATTR_DEBUGGABLE: u32 = 0x0101_000f;
const ATTR_VERSION_CODE: u32 = 0x0101_021b;
const ATTR_VERSION_NAME: u32 = 0x0101_021c;
const ATTR_MIN_SDK: u32 = 0x0101_020c;
const ATTR_TARGET_SDK: u32 = 0x0101_0270;

/// Walks an Android binary XML and returns the start elements.
fn parse_axml(b: &[u8]) -> Option<Vec<Element>> {
    if u16_at(b, 0)? != 0x0003 {
        return None;
    }
    let header_size = u16_at(b, 2)? as usize;
    let total = (u32_at(b, 4)? as usize).min(b.len());
    let mut strings: Vec<String> = Vec::new();
    let mut res_map: Vec<u32> = Vec::new();
    let mut elements = Vec::new();
    let mut depth = 0usize;
    let mut p = header_size;
    while p + 8 <= total {
        let kind = u16_at(b, p)?;
        let hsize = u16_at(b, p + 2)? as usize;
        let size = u32_at(b, p + 4)? as usize;
        if size < 8 || p + size > total {
            break;
        }
        match kind {
            0x0001 => strings = parse_string_pool(b, p)?,
            0x0180 => res_map = (0..(size - hsize) / 4).filter_map(|i| u32_at(b, p + hsize + i * 4)).collect(),
            0x0102 => {
                let ext = p + hsize;
                let name_idx = u32_at(b, ext + 4)? as usize;
                let attr_start = u16_at(b, ext + 8)? as usize;
                let attr_size = u16_at(b, ext + 10)? as usize;
                let attr_count = u16_at(b, ext + 12)? as usize;
                let mut attrs = Vec::with_capacity(attr_count);
                for i in 0..attr_count {
                    let a = ext + attr_start + i * attr_size;
                    let an = u32_at(b, a + 4)? as usize;
                    let raw = u32_at(b, a + 8)?;
                    let kind = *b.get(a + 15)?;
                    let data = u32_at(b, a + 16)?;
                    attrs.push(Attr {
                        name: strings.get(an).cloned().unwrap_or_default(),
                        res_id: res_map.get(an).copied(),
                        raw: if raw == u32::MAX { None } else { strings.get(raw as usize).cloned() },
                        value: TypedValue { kind, data },
                    });
                }
                elements.push(Element { name: strings.get(name_idx).cloned().unwrap_or_default(), attrs, depth });
                depth += 1;
            }
            0x0103 => depth = depth.saturating_sub(1),
            _ => {}
        }
        p += size;
    }
    Some(elements)
}

impl Element {
    fn attr(&self, res_id: u32, name: &str) -> Option<&Attr> {
        self.attrs.iter().find(|a| a.res_id == Some(res_id)).or_else(|| self.attrs.iter().find(|a| a.name == name))
    }
    fn string(&self, res_id: u32, name: &str, strings_res: &dyn Fn(u32) -> Option<String>) -> Option<String> {
        let a = self.attr(res_id, name)?;
        match a.value.kind {
            TYPE_STRING => a.raw.clone(),
            TYPE_REFERENCE => strings_res(a.value.data),
            TYPE_INT_DEC | TYPE_INT_HEX => Some(a.value.data.to_string()),
            _ => a.raw.clone(),
        }
    }
    fn int(&self, res_id: u32, name: &str) -> Option<u32> {
        let a = self.attr(res_id, name)?;
        match a.value.kind {
            TYPE_INT_DEC | TYPE_INT_HEX => Some(a.value.data),
            TYPE_STRING => a.raw.as_deref().and_then(|s| s.parse().ok()),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// resources.arsc (strings, file paths and colours)
// ---------------------------------------------------------------------------

/// A variant of a resource: language, density (dpi; 0 unspecified, 0xfffe
/// “anydpi”, 0xffff “nodpi”) and value.
type Variant = (String, u16, TypedValue);

#[derive(Default)]
struct ResTable {
    strings: Vec<String>,
    /// Resource identifier (0xPPTTEEEE) → variants. A table can bring several
    /// packages (the app's and others, e.g. 0x7f and 0x80).
    entries: HashMap<u32, Vec<Variant>>,
}

fn parse_arsc(b: &[u8]) -> Option<ResTable> {
    if u16_at(b, 0)? != 0x0002 {
        return None;
    }
    let header = u16_at(b, 2)? as usize;
    let total = (u32_at(b, 4)? as usize).min(b.len());
    let mut table = ResTable::default();
    let mut p = header;
    while p + 8 <= total {
        let kind = u16_at(b, p)?;
        let hsize = u16_at(b, p + 2)? as usize;
        let size = u32_at(b, p + 4)? as usize;
        if size < 8 || p + size > total {
            break;
        }
        match kind {
            0x0001 => table.strings = parse_string_pool(b, p)?,
            0x0200 => {
                let package_id = u32_at(b, p + 8)? as u8;
                let mut q = p + hsize;
                let end = p + size;
                while q + 8 <= end {
                    let k = u16_at(b, q)?;
                    let hs = u16_at(b, q + 2)? as usize;
                    let sz = u32_at(b, q + 4)? as usize;
                    if sz < 8 || q + sz > end {
                        break;
                    }
                    if k == 0x0201 {
                        parse_type_chunk(b, q, hs, package_id, &mut table);
                    }
                    q += sz;
                }
            }
            _ => {}
        }
        p += size;
    }
    Some(table)
}

fn parse_type_chunk(b: &[u8], p: usize, hsize: usize, package_id: u8, table: &mut ResTable) -> Option<()> {
    let type_id = *b.get(p + 8)?;
    let flags = *b.get(p + 9)?;
    let entry_count = u32_at(b, p + 12)? as usize;
    let entries_start = u32_at(b, p + 16)? as usize;
    // ResTable_config: size at p+20; language at +8 (2 bytes) and country at
    // +10.
    let cfg = p + 20;
    let lang = b.get(cfg + 8..cfg + 10).map(|l| if l[0] == 0 { String::new() } else { String::from_utf8_lossy(l).into_owned() }).unwrap_or_default();
    // Pseudo-locales (en-XA “[Çåļéñðåŕ…]”, ar-XB) exist only to test
    // translations: never a name to show.
    if matches!(b.get(cfg + 10..cfg + 12), Some(b"XA" | b"XB")) {
        return Some(());
    }
    // Density at +14 (after orientation and touchscreen).
    let density = u16_at(b, cfg + 14).unwrap_or(0);
    let sparse = flags & 0x01 != 0;
    let offset16 = flags & 0x02 != 0;
    let index_start = p + hsize;
    let id = |idx: u16| (u32::from(package_id) << 24) | (u32::from(type_id) << 16) | u32::from(idx);
    let mut read_entry = |idx: u16, offset: usize| -> Option<()> {
        let e = p + entries_start + offset;
        let esize = u16_at(b, e)? as usize;
        let eflags = u16_at(b, e + 2)?;
        if eflags & 0x0001 != 0 {
            return Some(()); // complex (styles, arrays)
        }
        if eflags & 0x0008 != 0 {
            // FLAG_COMPACT (Android 14): the value goes in the header.
            let kind = (eflags >> 8) as u8;
            let data = u32_at(b, e + 4)?;
            table.entries.entry(id(idx)).or_default().push((lang.clone(), density, TypedValue { kind, data }));
            return Some(());
        }
        let v = e + esize;
        let kind = *b.get(v + 3)?;
        let data = u32_at(b, v + 4)?;
        table.entries.entry(id(idx)).or_default().push((lang.clone(), density, TypedValue { kind, data }));
        Some(())
    };
    if sparse {
        for i in 0..entry_count {
            let idx = u16_at(b, index_start + i * 4)?;
            let off = u16_at(b, index_start + i * 4 + 2)? as usize * 4;
            read_entry(idx, off);
        }
    } else {
        for i in 0..entry_count {
            let off = if offset16 {
                match u16_at(b, index_start + i * 2)? {
                    0xffff => continue,
                    o => o as usize * 4,
                }
            } else {
                match u32_at(b, index_start + i * 4)? {
                    u32::MAX => continue,
                    o => o as usize,
                }
            };
            read_entry(i as u16, off);
        }
    }
    Some(())
}

impl ResTable {
    fn resolve_string(&self, id: u32, depth: u8) -> Option<String> {
        if depth > 5 {
            return None;
        }
        let values = self.entries.get(&id)?;
        let lang = crate::i18n::language().code();
        let pick = values.iter().find(|(l, _, _)| l == lang).or_else(|| values.iter().find(|(l, _, _)| l.is_empty())).or_else(|| values.iter().find(|(l, _, _)| l == "en")).or(values.first())?;
        match pick.2.kind {
            TYPE_STRING => self.strings.get(pick.2.data as usize).cloned(),
            TYPE_REFERENCE => self.resolve_string(pick.2.data, depth + 1),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// API
// ---------------------------------------------------------------------------

fn read_entry<R: Read + std::io::Seek>(zip: &mut zip::ZipArchive<R>, name: &str, limit: u64) -> Option<Vec<u8>> {
    let f = zip.by_name(name).ok()?;
    if f.size() > limit {
        return None;
    }
    let mut buf = Vec::with_capacity(f.size() as usize);
    f.take(limit).read_to_end(&mut buf).ok()?;
    Some(buf)
}

/// Display name from the manifest and resource table already extracted (e.g.
/// with `unzip -p` on the phone itself).
pub fn label_from_parts(manifest: &[u8], arsc: Option<&[u8]>) -> Option<String> {
    let elements = parse_axml(manifest)?;
    let table = arsc.and_then(parse_arsc);
    let resolve = |id: u32| table.as_ref().and_then(|t| t.resolve_string(id, 0));
    elements.iter().find(|e| e.name == "application").and_then(|e| e.string(ATTR_LABEL, "label", &resolve)).filter(|l| !l.is_empty() && !l.starts_with("res/"))
}

const ATTR_ICON: u32 = 0x0101_0002;
const ATTR_ROUND_ICON: u32 = 0x0101_052c;
const ATTR_DRAWABLE: u32 = 0x0101_0199;

/// A layer of an icon, ready to draw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IconLayer {
    /// PNG, WebP or SVG (Android vector drawables are converted to SVG).
    Image(Vec<u8>),
    /// ARGB.
    Color(u32),
}

/// An app's icon as it is drawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedIcon {
    /// Adaptive icon (Android 8+): `front` has a margin of 18 out of 108 units
    /// and is clipped over `background`.
    pub adaptive: bool,
    pub front: Vec<u8>,
    pub background: Option<IconLayer>,
    /// Some layer is a converted vector (some detail SVG lacks may be missing;
    /// if there is an image, the image is better).
    pub converted: bool,
}

/// Resource tables of an APK and its splits (`split_config.xxhdpi.apk`…): each
/// density's images may be in a separate split.
struct Tables(Vec<ResTable>);

impl Tables {
    /// Variants of a resource across all tables, following references
    /// (`@mipmap/x` → file). Each one with its table's index.
    fn variants(&self, id: u32, depth: u8) -> Vec<(usize, u16, TypedValue)> {
        let mut out = Vec::new();
        if depth > 6 {
            return out;
        }
        for (ti, t) in self.0.iter().enumerate() {
            for (_, density, v) in t.entries.get(&id).into_iter().flatten() {
                if v.kind == TYPE_REFERENCE {
                    out.extend(self.variants(v.data, depth + 1));
                } else {
                    out.push((ti, *density, *v));
                }
            }
        }
        out
    }

    fn string(&self, ti: usize, v: TypedValue) -> Option<&str> {
        (v.kind == TYPE_STRING).then(|| self.0.get(ti)?.strings.get(v.data as usize).map(String::as_str)).flatten()
    }
}

/// System colours used by some icons (`@android:color/…`).
fn framework_color(id: u32) -> Option<u32> {
    match id {
        0x0106_000b => Some(0xffff_ffff), // white
        0x0106_000c => Some(0xff00_0000), // black
        0x0106_000d => Some(0x0000_0000), // transparent
        _ => None,
    }
}

/// File that may be an image. With obfuscated resource names (`res/QtC`) the
/// extension of images is lost, not that of XML files; what it is gets
/// confirmed when reading it (see [`IconReader::load`]).
fn is_raster(f: &str) -> bool {
    !f.ends_with(".xml")
}

/// A resource file that was read, according to its content.
enum Loaded {
    Image(Vec<u8>),
    Xml(Vec<Element>),
}

/// What a drawable resolves to before reading its file.
enum Source<'a> {
    Color(u32),
    File(usize, &'a str),
}

/// Finds the app's icon and prepares it for drawing. `fetch(i, path)` reads a
/// file from APK `i` (0: the main one; then its splits, in the order of
/// `arscs`).
pub fn resolve_icon(manifest: &[u8], arscs: &[&[u8]], fetch: &dyn Fn(usize, &str) -> Option<Vec<u8>>) -> Option<ResolvedIcon> {
    let elements = parse_axml(manifest)?;
    // An unreadable table stays empty so the `fetch` indices do not shift.
    let tables = Tables(arscs.iter().map(|a| parse_arsc(a).unwrap_or_default()).collect());
    let app = elements.iter().find(|e| e.name == "application")?;
    let ids: Vec<u32> = [app.attr(ATTR_ICON, "icon"), app.attr(ATTR_ROUND_ICON, "roundIcon")].into_iter().flatten().filter(|a| a.value.kind == TYPE_REFERENCE).map(|a| a.value.data).collect();
    let r = IconReader { tables: &tables, fetch };
    // The normal icon before the round one; an image before a vector.
    let mut fallback = None;
    for id in ids {
        if let Some(icon) = r.icon(id) {
            if !icon.converted {
                return Some(icon);
            }
            fallback.get_or_insert(icon);
        }
    }
    fallback
}

struct IconReader<'a> {
    tables: &'a Tables,
    fetch: &'a dyn Fn(usize, &str) -> Option<Vec<u8>>,
}

impl IconReader<'_> {
    /// The best source of a resource: a colour, the sharpest image up to
    /// xxxhdpi or, if there is no image, its XML (the adaptive one before any
    /// other).
    fn source(&self, id: u32) -> Option<Source<'_>> {
        if let Some(c) = framework_color(id) {
            return Some(Source::Color(c));
        }
        let vars = self.tables.variants(id, 0);
        let files: Vec<(usize, u16, &str)> = vars.iter().filter_map(|(ti, d, v)| Some((*ti, *d, self.tables.string(*ti, *v)?))).collect();
        // Effective density: “anydpi”/“nodpi” count as low for an image.
        let rank = |d: u16| if d >= 0xfffe || d > 640 { 0 } else { d };
        // An image before an XML, and either of them before a colour (some
        // icons bring a colour as the default variant).
        files
            .iter()
            .filter(|(_, _, f)| is_raster(f))
            .max_by_key(|(_, d, _)| rank(*d))
            .or_else(|| files.iter().filter(|(_, _, f)| f.ends_with(".xml")).max_by_key(|(_, d, _)| *d == 0xfffe))
            .map(|(ti, _, f)| Source::File(*ti, f))
            .or_else(|| vars.iter().find(|(_, _, v)| (TYPE_COLOR_ARGB8..=TYPE_COLOR_RGB4).contains(&v.kind)).map(|(_, _, v)| Source::Color(v.data)))
    }

    fn icon(&self, id: u32) -> Option<ResolvedIcon> {
        let Source::File(ti, f) = self.source(id)? else {
            return None;
        };
        match self.load(ti, f)? {
            Loaded::Image(front) => Some(ResolvedIcon { adaptive: false, front, background: None, converted: false }),
            Loaded::Xml(els) => {
                if els.first()?.name != "adaptive-icon" {
                    let (layer, converted) = self.element(&els, 0, 0)?;
                    let IconLayer::Image(front) = layer else {
                        return None;
                    };
                    return Some(ResolvedIcon { adaptive: false, front, background: None, converted });
                }
                let part = |tag: &str| els.iter().position(|e| e.name == tag && e.depth == 1).and_then(|i| self.slot(&els, i, 0));
                let (front, fc) = part("foreground")?;
                let IconLayer::Image(front) = front else {
                    return None;
                };
                let back = part("background");
                let converted = fc || back.as_ref().is_some_and(|b| b.1);
                Some(ResolvedIcon { adaptive: true, front, background: back.map(|b| b.0), converted })
            }
        }
    }

    /// Reads a file and checks what it is: an image (PNG, WebP, JPEG) or binary
    /// XML.
    fn load(&self, ti: usize, f: &str) -> Option<Loaded> {
        let b = (self.fetch)(ti, f)?;
        let image = b.starts_with(b"\x89PNG") || b.starts_with(&[0xff, 0xd8]) || (b.len() > 12 && &b[..4] == b"RIFF" && &b[8..12] == b"WEBP");
        if image {
            Some(Loaded::Image(b))
        } else {
            parse_axml(&b).map(Loaded::Xml)
        }
    }

    /// Content of `<foreground>`, `<background>` or `<item>`: the `drawable`
    /// attribute or the element inside.
    fn slot(&self, els: &[Element], i: usize, depth: u8) -> Option<(IconLayer, bool)> {
        if let Some(a) = els[i].attr(ATTR_DRAWABLE, "drawable") {
            return self.value(a.value, depth);
        }
        els.get(i + 1).filter(|c| c.depth == els[i].depth + 1)?;
        self.element(els, i + 1, depth)
    }

    /// An attribute value that points to a drawable or is a colour.
    fn value(&self, v: TypedValue, depth: u8) -> Option<(IconLayer, bool)> {
        match v.kind {
            TYPE_COLOR_ARGB8..=TYPE_COLOR_RGB4 => Some((IconLayer::Color(v.data), false)),
            TYPE_REFERENCE => self.drawable(v.data, depth),
            _ => None,
        }
    }

    fn drawable(&self, id: u32, depth: u8) -> Option<(IconLayer, bool)> {
        if depth > 6 {
            return None;
        }
        match self.source(id)? {
            Source::Color(c) => Some((IconLayer::Color(c), false)),
            Source::File(ti, f) => match self.load(ti, f)? {
                Loaded::Image(b) => Some((IconLayer::Image(b), false)),
                Loaded::Xml(els) => self.element(&els, 0, depth + 1),
            },
        }
    }

    /// A drawable written in XML (`els[i]` and what it contains).
    fn element(&self, els: &[Element], i: usize, depth: u8) -> Option<(IconLayer, bool)> {
        let e = els.get(i)?;
        match e.name.as_str() {
            "vector" => Some((IconLayer::Image(self.vector_svg(els, i)?.into_bytes()), true)),
            "bitmap" | "nine-patch" => self.value(e.attr(0, "src")?.value, depth),
            // A margin around another drawable: the inner one is used.
            "inset" | "scale" | "rotate" | "clip" => self.slot(els, i, depth),
            "color" => self.color(e.attr(0, "color")?.value).map(|c| (IconLayer::Color(c), false)),
            "shape" => {
                let end = subtree_end(els, i);
                let solid = els[i + 1..end].iter().find(|c| c.name == "solid")?;
                self.color(solid.attr(0, "color")?.value).map(|c| (IconLayer::Color(c), false))
            }
            _ => None,
        }
    }

    /// Colour of a value: literal, `@color/x`, a system one or the first of a
    /// state list (`<selector>`).
    fn color(&self, v: TypedValue) -> Option<u32> {
        match v.kind {
            TYPE_COLOR_ARGB8..=TYPE_COLOR_RGB4 => Some(v.data),
            TYPE_REFERENCE => match self.source(v.data)? {
                Source::Color(c) => Some(c),
                Source::File(ti, f) => {
                    let Loaded::Xml(els) = self.load(ti, f)? else {
                        return None;
                    };
                    // Only a state list: a gradient also has `<item>`.
                    if els.first()?.name != "selector" {
                        return None;
                    }
                    let item = els.iter().find(|e| e.name == "item")?;
                    self.color(item.attr(0, "color")?.value)
                }
            },
            _ => None,
        }
    }

    /// Paint of a stroke or fill: a colour or a gradient (`<gradient>` in a
    /// separate XML, as aapt leaves it). Returns the `fill`/`stroke` value and
    /// its opacity.
    fn paint(&self, v: TypedValue, defs: &mut String, n: &mut usize) -> Option<(String, f32)> {
        if let Some(c) = self.color(v) {
            return Some((hex_rgb(c), alpha(c)));
        }
        let Some(Source::File(ti, f)) = (v.kind == TYPE_REFERENCE).then(|| self.source(v.data)).flatten() else {
            return None;
        };
        let Loaded::Xml(els) = self.load(ti, f)? else {
            return None;
        };
        let g = els.first().filter(|e| e.name == "gradient")?;
        let num = |name: &str| g.attr(0, name).and_then(|a| float_value(a.value));
        *n += 1;
        let id = format!("g{n}");
        let mut stops = String::new();
        let items: Vec<&Element> = els.iter().filter(|e| e.name == "item").collect();
        if items.is_empty() {
            let mut cols = vec![(0.0, "startColor")];
            if g.attr(0, "centerColor").is_some() {
                cols.push((0.5, "centerColor"));
            }
            cols.push((1.0, "endColor"));
            for (off, name) in cols {
                let c = g.attr(0, name).and_then(|a| self.color(a.value)).unwrap_or(0);
                stops += &format!(r#"<stop offset="{off}" stop-color="{}" stop-opacity="{}"/>"#, hex_rgb(c), alpha(c));
            }
        } else {
            for it in items {
                let off = it.attr(0, "offset").and_then(|a| float_value(a.value)).unwrap_or(0.0);
                let c = it.attr(0, "color").and_then(|a| self.color(a.value)).unwrap_or(0);
                stops += &format!(r#"<stop offset="{off}" stop-color="{}" stop-opacity="{}"/>"#, hex_rgb(c), alpha(c));
            }
        }
        let kind = g.attr(0, "type").map(|a| a.value.data).unwrap_or(0);
        if kind == 1 {
            let (cx, cy, r) = (num("centerX").unwrap_or(0.0), num("centerY").unwrap_or(0.0), num("gradientRadius").unwrap_or(1.0));
            *defs += &format!(r#"<radialGradient id="{id}" gradientUnits="userSpaceOnUse" cx="{cx}" cy="{cy}" r="{r}">{stops}</radialGradient>"#);
        } else {
            // Linear (the sweep one, which SVG lacks, is also drawn as linear).
            let (x1, y1, x2, y2) = (num("startX").unwrap_or(0.0), num("startY").unwrap_or(0.0), num("endX").unwrap_or(0.0), num("endY").unwrap_or(0.0));
            *defs += &format!(r#"<linearGradient id="{id}" gradientUnits="userSpaceOnUse" x1="{x1}" y1="{y1}" x2="{x2}" y2="{y2}">{stops}</linearGradient>"#);
        }
        Some((format!("url(#{id})"), 1.0))
    }

    /// An Android `<vector>` converted to SVG: paths, groups with their
    /// transformations, clips and gradients.
    fn vector_svg(&self, els: &[Element], i: usize) -> Option<String> {
        let root = &els[i];
        let num = |e: &Element, name: &str| e.attr(0, name).and_then(|a| float_value(a.value));
        let (vw, vh) = (num(root, "viewportWidth")?, num(root, "viewportHeight")?);
        if vw <= 0.0 || vh <= 0.0 {
            return None;
        }
        let path_data = |e: &Element| -> Option<String> {
            let a = e.attr(0, "pathData")?;
            match a.value.kind {
                TYPE_REFERENCE => self.res_string(a.value.data),
                _ => a.raw.clone(),
            }
        };
        let (mut defs, mut body, mut n) = (String::new(), String::new(), 0usize);
        // Open tags and the depth from which they close.
        let mut open: Vec<usize> = Vec::new();
        let end = subtree_end(els, i);
        for e in &els[i + 1..end] {
            while open.last().is_some_and(|d| e.depth <= *d) {
                open.pop();
                body += "</g>";
            }
            match e.name.as_str() {
                "group" => {
                    let (px, py) = (num(e, "pivotX").unwrap_or(0.0), num(e, "pivotY").unwrap_or(0.0));
                    let (sx, sy) = (num(e, "scaleX").unwrap_or(1.0), num(e, "scaleY").unwrap_or(1.0));
                    let (tx, ty, rot) = (num(e, "translateX").unwrap_or(0.0), num(e, "translateY").unwrap_or(0.0), num(e, "rotation").unwrap_or(0.0));
                    // Android's order: to the pivot, scale, rotate and back.
                    body += &format!(r#"<g transform="translate({} {}) rotate({rot}) scale({sx} {sy}) translate({} {})">"#, tx + px, ty + py, -px, -py);
                    open.push(e.depth);
                }
                "clip-path" => {
                    // Clips what follows within the same group.
                    let d = path_data(e)?;
                    n += 1;
                    defs += &format!(r#"<clipPath id="c{n}"><path d="{}"/></clipPath>"#, xml_escape(&d));
                    body += &format!(r#"<g clip-path="url(#c{n})">"#);
                    open.push(e.depth - 1);
                }
                "path" => {
                    let Some(d) = path_data(e) else { continue };
                    let mut attrs = format!(r#"d="{}""#, xml_escape(&d));
                    match e.attr(0, "fillColor").and_then(|a| self.paint(a.value, &mut defs, &mut n)) {
                        Some((fill, op)) => {
                            let op = op * num(e, "fillAlpha").unwrap_or(1.0);
                            attrs += &format!(r#" fill="{fill}" fill-opacity="{op}""#);
                            if e.attr(0, "fillType").is_some_and(|a| a.value.data == 1) {
                                attrs += r#" fill-rule="evenodd""#;
                            }
                        }
                        None => attrs += r#" fill="none""#,
                    }
                    let width = num(e, "strokeWidth").unwrap_or(0.0);
                    if let Some((stroke, op)) = (width > 0.0).then(|| e.attr(0, "strokeColor").and_then(|a| self.paint(a.value, &mut defs, &mut n))).flatten() {
                        let op = op * num(e, "strokeAlpha").unwrap_or(1.0);
                        attrs += &format!(r#" stroke="{stroke}" stroke-opacity="{op}" stroke-width="{width}""#);
                        let cap = ["butt", "round", "square"].get(e.attr(0, "strokeLineCap").map_or(0, |a| a.value.data as usize)).unwrap_or(&"butt");
                        let join = ["miter", "round", "bevel"].get(e.attr(0, "strokeLineJoin").map_or(0, |a| a.value.data as usize)).unwrap_or(&"miter");
                        attrs += &format!(r#" stroke-linecap="{cap}" stroke-linejoin="{join}""#);
                        if let Some(m) = num(e, "strokeMiterLimit") {
                            attrs += &format!(r#" stroke-miterlimit="{m}""#);
                        }
                    }
                    body += &format!("<path {attrs}/>");
                }
                _ => {}
            }
        }
        for _ in open {
            body += "</g>";
        }
        let opacity = num(root, "alpha").unwrap_or(1.0);
        // Generous drawing size: it is scaled down when displayed.
        let (w, h) = (432.0, 432.0 * vh / vw);
        Some(format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {vw} {vh}"><defs>{defs}</defs><g opacity="{opacity}">{body}</g></svg>"#))
    }

    /// String of a `@string/…` resource (vector paths stored separately).
    fn res_string(&self, id: u32) -> Option<String> {
        self.tables.variants(id, 0).into_iter().find_map(|(ti, _, v)| self.tables.string(ti, v).map(String::from))
    }
}

/// End (exclusive) of the elements under `els[i]`.
fn subtree_end(els: &[Element], i: usize) -> usize {
    let d = els[i].depth;
    els[i + 1..].iter().position(|e| e.depth <= d).map_or(els.len(), |p| i + 1 + p)
}

const TYPE_FLOAT: u8 = 0x04;

fn float_value(v: TypedValue) -> Option<f32> {
    match v.kind {
        TYPE_FLOAT => Some(f32::from_bits(v.data)),
        TYPE_INT_DEC | TYPE_INT_HEX => Some(v.data as i32 as f32),
        _ => None,
    }
}

fn hex_rgb(argb: u32) -> String {
    format!("#{:06x}", argb & 0x00ff_ffff)
}

fn alpha(argb: u32) -> f32 {
    ((argb >> 24) & 0xff) as f32 / 255.0
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

const ATTR_PERMISSION: u32 = 0x0101_0006;
const ATTR_ENABLED: u32 = 0x0101_000e;
const ATTR_EXPORTED: u32 = 0x0101_0010;
const ATTR_AUTHORITIES: u32 = 0x0101_0018;
const ATTR_USES_CLEARTEXT: u32 = 0x0101_04ec;
const ATTR_NETWORK_SECURITY_CONFIG: u32 = 0x0101_0527;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub enum ComponentKind {
    Activity,
    Service,
    Receiver,
    Provider,
}

impl ComponentKind {
    pub const ALL: [ComponentKind; 4] = [Self::Activity, Self::Service, Self::Receiver, Self::Provider];

    pub fn plural(self) -> &'static str {
        match self {
            Self::Activity => tr("Activities"),
            Self::Service => tr("Services"),
            Self::Receiver => tr("Receivers"),
            Self::Provider => tr("Content providers"),
        }
    }

    fn from_tag(tag: &str) -> Option<Self> {
        Some(match tag {
            "activity" | "activity-alias" => Self::Activity,
            "service" => Self::Service,
            "receiver" => Self::Receiver,
            "provider" => Self::Provider,
            _ => return None,
        })
    }
}

/// A component declared in the manifest.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ManifestComponent {
    pub kind: ComponentKind,
    /// Fully qualified class name (relative ones are completed with the
    /// package).
    pub name: String,
    /// `android:exported` as written; `None` if it is not declared or depends
    /// on a resource.
    pub exported_attr: Option<bool>,
    pub has_intent_filter: bool,
    /// Permission the caller must hold.
    pub permission: Option<String>,
    /// `android:enabled="false"` in the manifest.
    pub disabled_in_manifest: bool,
    /// Authorities (`content://…`) of the providers.
    pub authorities: Option<String>,
}

impl ManifestComponent {
    /// Actually exported: the written value or, if missing, Android's rule
    /// (with intent filters → exported; providers → only if the app targets API
    /// < 17).
    pub fn is_exported(&self, target_sdk: Option<u32>) -> bool {
        match self.exported_attr {
            Some(e) => e,
            None if self.kind == ComponentKind::Provider => target_sdk.is_some_and(|t| t < 17),
            None => self.has_intent_filter,
        }
    }

    /// Exported and without a permission: any app on the phone can invoke it.
    pub fn is_open(&self, target_sdk: Option<u32>) -> bool {
        self.is_exported(target_sdk) && self.permission.is_none()
    }

    /// Short name for lists (`com.app.ui.Main` → `ui.Main` within `com.app`).
    pub fn short_name(&self, package: &str) -> String {
        self.name.strip_prefix(package).map(|r| r.to_string()).filter(|r| r.starts_with('.')).unwrap_or_else(|| self.name.clone())
    }
}

/// What the manifest of an already installed app says.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ManifestSummary {
    pub package: String,
    pub target_sdk: Option<u32>,
    pub components: Vec<ManifestComponent>,
    /// `android:usesCleartextTraffic` (unencrypted HTTP) if declared.
    pub uses_cleartext: Option<bool>,
    /// It has its own network security configuration.
    pub network_security_config: bool,
}

impl ManifestSummary {
    pub fn of_kind(&self, kind: ComponentKind) -> impl Iterator<Item = &ManifestComponent> {
        self.components.iter().filter(move |c| c.kind == kind)
    }
}

fn bool_attr(el: &Element, res_id: u32, name: &str) -> Option<bool> {
    let a = el.attr(res_id, name)?;
    match a.value.kind {
        TYPE_INT_BOOLEAN => Some(a.value.data != 0),
        TYPE_STRING => a.raw.as_deref().and_then(|s| s.parse().ok()),
        // A reference to a @bool/… resource depends on the configuration.
        _ => None,
    }
}

/// Components and network settings from the binary manifest (without
/// `resources.arsc`: class names and permissions are literal strings).
pub fn manifest_summary(manifest: &[u8]) -> Option<ManifestSummary> {
    let elements = parse_axml(manifest)?;
    let no_res = |_: u32| None;
    let mut out = ManifestSummary::default();
    // (index of the open component, its depth)
    let mut open: Option<(usize, usize)> = None;
    for el in &elements {
        if let Some((_, d)) = open {
            if el.depth <= d {
                open = None;
            }
        }
        match el.name.as_str() {
            "manifest" => out.package = el.attrs.iter().find(|a| a.name == "package").and_then(|a| a.raw.clone()).unwrap_or_default(),
            "uses-sdk" => out.target_sdk = el.int(ATTR_TARGET_SDK, "targetSdkVersion"),
            "application" => {
                out.uses_cleartext = bool_attr(el, ATTR_USES_CLEARTEXT, "usesCleartextTraffic");
                out.network_security_config = el.attr(ATTR_NETWORK_SECURITY_CONFIG, "networkSecurityConfig").is_some();
            }
            "intent-filter" => {
                if let Some((i, d)) = open {
                    if el.depth == d + 1 {
                        out.components[i].has_intent_filter = true;
                    }
                }
            }
            tag => {
                let Some(kind) = ComponentKind::from_tag(tag) else {
                    continue;
                };
                let Some(raw) = el.string(ATTR_NAME, "name", &no_res).filter(|n| !n.is_empty()) else {
                    continue;
                };
                let name = if raw.starts_with('.') {
                    format!("{}{raw}", out.package)
                } else if !raw.contains('.') {
                    format!("{}.{raw}", out.package)
                } else {
                    raw
                };
                out.components.push(ManifestComponent {
                    kind,
                    name,
                    exported_attr: bool_attr(el, ATTR_EXPORTED, "exported"),
                    has_intent_filter: false,
                    permission: el.string(ATTR_PERMISSION, "permission", &no_res).filter(|p| !p.is_empty()),
                    disabled_in_manifest: bool_attr(el, ATTR_ENABLED, "enabled") == Some(false),
                    authorities: if kind == ComponentKind::Provider { el.string(ATTR_AUTHORITIES, "authorities", &no_res).filter(|a| !a.is_empty()) } else { None },
                });
                open = Some((out.components.len() - 1, el.depth));
            }
        }
    }
    Some(out)
}

pub fn inspect(path: &Path) -> Result<ApkInfo> {
    let meta = std::fs::metadata(path)?;
    let file = std::fs::File::open(path)?;
    let mut zip = zip::ZipArchive::new(std::io::BufReader::new(file)).map_err(|e| Error::Invalid(t!("not a valid APK: {}", e)))?;
    let manifest = read_entry(&mut zip, "AndroidManifest.xml", 16 << 20).ok_or_else(|| Error::Invalid(tr("the APK does not contain AndroidManifest.xml").into()))?;
    let elements = parse_axml(&manifest).ok_or_else(|| Error::Invalid(tr("unrecognised binary manifest").into()))?;
    let table = read_entry(&mut zip, "resources.arsc", 64 << 20).and_then(|b| parse_arsc(&b));
    let resolve = |id: u32| table.as_ref().and_then(|t| t.resolve_string(id, 0));

    let mut info = ApkInfo { file_name: path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), size_bytes: meta.len(), ..Default::default() };
    for el in &elements {
        match el.name.as_str() {
            "manifest" => {
                info.package = el.attrs.iter().find(|a| a.name == "package").and_then(|a| a.raw.clone()).unwrap_or_default();
                info.version_code = el.string(ATTR_VERSION_CODE, "versionCode", &resolve).unwrap_or_default();
                info.version_name = el.string(ATTR_VERSION_NAME, "versionName", &resolve).unwrap_or_default();
                info.split = el.attrs.iter().any(|a| a.name == "split");
            }
            "uses-sdk" => {
                info.min_sdk = el.int(ATTR_MIN_SDK, "minSdkVersion");
                info.target_sdk = el.int(ATTR_TARGET_SDK, "targetSdkVersion");
            }
            "application" => {
                info.label = el.string(ATTR_LABEL, "label", &resolve).unwrap_or_default();
                info.debuggable = el.attr(ATTR_DEBUGGABLE, "debuggable").is_some_and(|a| a.value.kind == TYPE_INT_BOOLEAN && a.value.data != 0);
            }
            "uses-permission" | "uses-permission-sdk-23" => {
                if let Some(n) = el.string(ATTR_NAME, "name", &resolve) {
                    if !info.permissions.contains(&n) {
                        info.permissions.push(n);
                    }
                }
            }
            _ => {}
        }
    }
    let mut abis: Vec<String> = zip.file_names().filter_map(|n| n.strip_prefix("lib/").and_then(|r| r.split('/').next()).map(String::from)).collect();
    abis.sort();
    abis.dedup();
    info.native_abis = abis;
    info.permissions.sort();
    if info.package.is_empty() {
        return Err(Error::Invalid(tr("could not read the package name").into()));
    }
    Ok(info)
}

/// `unzip -l` output: the size of `entry` (“ 72079304  2009-01-01 00:00   resources.arsc”).
pub fn listed_size(text: &str, entry: &str) -> Option<u64> {
    text.lines().find_map(|l| {
        let mut parts = l.split_whitespace();
        let size = parts.next()?.parse().ok()?;
        (parts.last()? == entry).then_some(size)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_string_pool() {
        // Pool with two UTF-8 strings: "hola" and "añ"
        let mut b = vec![0u8; 28];
        b[0..2].copy_from_slice(&1u16.to_le_bytes());
        b[2..4].copy_from_slice(&28u16.to_le_bytes());
        b[8..12].copy_from_slice(&2u32.to_le_bytes());
        b[16..20].copy_from_slice(&0x100u32.to_le_bytes());
        b[20..24].copy_from_slice(&36u32.to_le_bytes()); // strings_start = 28 + 8 offsets
        b.extend_from_slice(&0u32.to_le_bytes());
        b.extend_from_slice(&7u32.to_le_bytes());
        b.extend_from_slice(&[4, 4, b'h', b'o', b'l', b'a', 0]);
        b.extend_from_slice(&[2, 3, b'a', 0xc3, 0xb1, 0]);
        let total = b.len() as u32;
        b[4..8].copy_from_slice(&total.to_le_bytes());
        assert_eq!(parse_string_pool(&b, 0).unwrap(), vec!["hola", "añ"]);
    }

    #[test]
    fn dangerous() {
        assert!(is_dangerous_permission("android.permission.CAMERA"));
        assert!(!is_dangerous_permission("android.permission.INTERNET"));
        assert!(!is_dangerous_permission("com.x.CAMERA"));
    }

    /// Attribute value for [`axml`]: text (also sent as `raw`), floating-point
    /// number, integer or colour.
    enum V {
        S(&'static str),
        F(f32),
        I(u32),
        C(u32),
    }

    /// Minimal Android binary XML: `(depth, tag, attributes)` in document
    /// order.
    /// An element for [`axml`]: depth, tag and attributes.
    type El = (usize, &'static str, Vec<(&'static str, V)>);

    fn axml(elements: &[El]) -> Vec<u8> {
        let mut strings: Vec<&str> = Vec::new();
        let mut idx = |t: &'static str| -> u32 {
            if let Some(i) = strings.iter().position(|x| *x == t) {
                return i as u32;
            }
            strings.push(t);
            strings.len() as u32 - 1
        };
        let mut body = Vec::new();
        let mut open: Vec<(usize, u32)> = Vec::new();
        let close = |body: &mut Vec<u8>, name: u32| {
            body.extend_from_slice(&0x0103u16.to_le_bytes());
            body.extend_from_slice(&16u16.to_le_bytes());
            body.extend_from_slice(&24u32.to_le_bytes());
            body.extend_from_slice(&[0; 8]);
            body.extend_from_slice(&u32::MAX.to_le_bytes());
            body.extend_from_slice(&name.to_le_bytes());
        };
        for (depth, tag, attrs) in elements {
            while open.last().is_some_and(|(d, _)| d >= depth) {
                close(&mut body, open.pop().unwrap().1);
            }
            let name = idx(tag);
            let size = 16 + 20 + 20 * attrs.len();
            body.extend_from_slice(&0x0102u16.to_le_bytes());
            body.extend_from_slice(&16u16.to_le_bytes());
            body.extend_from_slice(&(size as u32).to_le_bytes());
            body.extend_from_slice(&[0; 8]);
            body.extend_from_slice(&u32::MAX.to_le_bytes());
            body.extend_from_slice(&name.to_le_bytes());
            body.extend_from_slice(&20u16.to_le_bytes());
            body.extend_from_slice(&20u16.to_le_bytes());
            body.extend_from_slice(&(attrs.len() as u16).to_le_bytes());
            body.extend_from_slice(&[0; 6]);
            for (an, v) in attrs {
                let an = idx(an);
                let (raw, kind, data) = match v {
                    V::S(t) => {
                        let i = idx(t);
                        (i, TYPE_STRING, i)
                    }
                    V::F(f) => (u32::MAX, TYPE_FLOAT, f.to_bits()),
                    V::I(n) => (u32::MAX, TYPE_INT_DEC, *n),
                    V::C(c) => (u32::MAX, TYPE_COLOR_ARGB8, *c),
                };
                body.extend_from_slice(&u32::MAX.to_le_bytes());
                body.extend_from_slice(&an.to_le_bytes());
                body.extend_from_slice(&raw.to_le_bytes());
                body.extend_from_slice(&8u16.to_le_bytes());
                body.push(0);
                body.push(kind);
                body.extend_from_slice(&data.to_le_bytes());
            }
            open.push((*depth, name));
        }
        while let Some((_, name)) = open.pop() {
            close(&mut body, name);
        }
        // UTF-8 string pool (short strings: one-byte lengths).
        let mut data = Vec::new();
        let mut offsets = Vec::new();
        for t in &strings {
            offsets.push(data.len() as u32);
            data.extend_from_slice(&[t.chars().count() as u8, t.len() as u8]);
            data.extend_from_slice(t.as_bytes());
            data.push(0);
        }
        while data.len() % 4 != 0 {
            data.push(0);
        }
        let mut pool = Vec::new();
        let start = 28 + 4 * strings.len();
        pool.extend_from_slice(&1u16.to_le_bytes());
        pool.extend_from_slice(&28u16.to_le_bytes());
        pool.extend_from_slice(&((start + data.len()) as u32).to_le_bytes());
        pool.extend_from_slice(&(strings.len() as u32).to_le_bytes());
        pool.extend_from_slice(&0u32.to_le_bytes());
        pool.extend_from_slice(&0x100u32.to_le_bytes());
        pool.extend_from_slice(&(start as u32).to_le_bytes());
        pool.extend_from_slice(&0u32.to_le_bytes());
        for o in offsets {
            pool.extend_from_slice(&o.to_le_bytes());
        }
        pool.extend_from_slice(&data);
        let mut out = Vec::new();
        out.extend_from_slice(&3u16.to_le_bytes());
        out.extend_from_slice(&8u16.to_le_bytes());
        out.extend_from_slice(&((8 + pool.len() + body.len()) as u32).to_le_bytes());
        out.extend_from_slice(&pool);
        out.extend_from_slice(&body);
        out
    }

    fn convert(xml: &[u8]) -> Option<(IconLayer, bool)> {
        let tables = Tables(Vec::new());
        let reader = IconReader { tables: &tables, fetch: &|_, _| None };
        reader.element(&parse_axml(xml)?, 0, 0)
    }

    fn svg_of(xml: &[u8]) -> String {
        match convert(xml) {
            Some((IconLayer::Image(b), true)) => String::from_utf8(b).unwrap(),
            other => panic!("expected an SVG: {other:?}"),
        }
    }

    #[test]
    fn vector_to_svg() {
        let xml = axml(&[
            (0, "vector", vec![("viewportWidth", V::F(108.0)), ("viewportHeight", V::F(108.0)), ("alpha", V::F(0.5))]),
            (1, "group", vec![("pivotX", V::F(54.0)), ("pivotY", V::F(54.0)), ("scaleX", V::F(2.0)), ("translateX", V::F(3.0))]),
            (2, "clip-path", vec![("pathData", V::S("M0,0h10v10z"))]),
            (2, "path", vec![("pathData", V::S("M1,1L5,5")), ("fillColor", V::C(0x80ff0000)), ("fillType", V::I(1))]),
            (1, "path", vec![("pathData", V::S("M2,2")), ("strokeColor", V::C(0xff00ff00)), ("strokeWidth", V::F(2.0)), ("strokeLineCap", V::I(1))]),
        ]);
        let svg = svg_of(&xml);
        assert!(svg.contains(r#"viewBox="0 0 108 108""#), "{svg}");
        assert!(svg.contains(r#"<g opacity="0.5">"#), "{svg}");
        // Pivot: to the origin, scale, rotate and back (plus the translation).
        assert!(svg.contains(r#"transform="translate(57 54) rotate(0) scale(2 1) translate(-54 -54)""#), "{svg}");
        assert!(svg.contains(r#"<clipPath id="c1"><path d="M0,0h10v10z"/></clipPath>"#), "{svg}");
        assert!(svg.contains(r##"<path d="M1,1L5,5" fill="#ff0000" fill-opacity="0.5019608" fill-rule="evenodd"/>"##), "{svg}");
        // Without `fillColor` there is no fill; the stroke, with its width and
        // cap.
        assert!(svg.contains(r##"fill="none" stroke="#00ff00" stroke-opacity="1" stroke-width="2" stroke-linecap="round""##), "{svg}");
        // The clip closes with its group: the second path is outside both.
        let second = svg.find(r#"<path d="M2,2""#).unwrap();
        assert_eq!(svg[..second].matches("<g").count() - 1, svg[..second].matches("</g>").count(), "{svg}");
        assert!(svg.ends_with("/></g></svg>"), "{svg}");
    }

    #[test]
    fn inline_layers() {
        // Solid-colour `<shape>`: a colour, not an image.
        let shape = axml(&[(0, "shape", vec![]), (1, "solid", vec![("color", V::C(0xff123456))])]);
        assert_eq!(convert(&shape).map(|l| l.0), Some(IconLayer::Color(0xff123456)));
        // `<inset>` with the drawable inside: the inner one is used.
        let inset = axml(&[(0, "inset", vec![]), (1, "vector", vec![("viewportWidth", V::F(24.0)), ("viewportHeight", V::F(24.0))])]);
        assert!(svg_of(&inset).contains(r#"viewBox="0 0 24 24""#));
        // Without a viewport size there is no vector to convert.
        assert!(convert(&axml(&[(0, "vector", vec![])])).is_none());
    }

    #[test]
    fn subtree_bounds() {
        let els = parse_axml(&axml(&[(0, "a", vec![]), (1, "b", vec![]), (2, "c", vec![]), (1, "d", vec![])])).unwrap();
        assert_eq!(els.iter().map(|e| e.depth).collect::<Vec<_>>(), vec![0, 1, 2, 1]);
        assert_eq!(subtree_end(&els, 1), 3);
        assert_eq!(subtree_end(&els, 0), 4);
        assert!(is_raster("res/QtC") && is_raster("res/a.webp") && !is_raster("res/a.xml"));
    }

    /// Test with a real APK if `ADBSCAN_TEST_APK=/path.apk` is set.
    #[test]
    fn real_apk_if_available() {
        let Ok(path) = std::env::var("ADBSCAN_TEST_APK") else {
            return;
        };
        let info = inspect(Path::new(&path)).expect("readable APK");
        eprintln!("{info:#?}");
        assert!(!info.package.is_empty());
        assert!(!info.label.is_empty(), "must resolve the display name");
        assert!(info.target_sdk.is_some());
    }

    #[test]
    fn listed_entry_size() {
        let out = "Archive:  /system_ext/priv-app/Settings/Settings.apk\n  Length      Date    Time    Name\n---------  ---------- -----   ----\n 72079304  2009-01-01 00:00   resources.arsc\n---------                     -------\n 72079304                     1 file\n";
        assert_eq!(listed_size(out, "resources.arsc"), Some(72079304));
        assert_eq!(listed_size(out, "AndroidManifest.xml"), None);
        assert_eq!(listed_size("", "resources.arsc"), None);
    }
}
