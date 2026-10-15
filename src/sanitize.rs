//! Privacy sanitization for forum / bug-report exports.
//! MACs are masked to their OUI prefix, serials are replaced with a short
//! stable hash so two reports from the same machine still correlate,
//! and the hostname is redacted.
use sha2::{Digest, Sha256};

pub fn hash_id(label: &str, value: &str) -> String {
    let mut h = Sha256::new();
    h.update(label.as_bytes()); h.update(b":"); h.update(value.as_bytes());
    let digest = h.finalize();
    format!("sha256:{}", digest.iter().take(6).map(|b| format!("{b:02x}")).collect::<String>())
}

pub fn mask_mac(mac: &str) -> String {
    let parts: Vec<&str> = mac.split(':').collect();
    if parts.len() == 6 { format!("{}:{}:{}:**:**:**", parts[0], parts[1], parts[2]) }
    else { "**masked**".to_string() }
}

/// Mask one IP (optionally with a /prefix suffix) for export.
/// Private, loopback, link-local and ULA addresses stay readable (they're
/// LAN-local and useful when debugging); PUBLIC addresses lose their host
/// part: IPv4 keeps the /24 network, IPv6 keeps the /64 prefix. Parsing via
/// std::net::IpAddr — the old string-prefix checks let global IPv6 through
/// untouched (Gemini review, confirmed 2026-10-13).
pub fn mask_ip(raw: &str) -> String {
    use std::net::IpAddr;
    let (ip_part, cidr) = match raw.split_once('/') {
        Some((i, c)) => (i, Some(c)),
        None => (raw, None),
    };
    let suffix = cidr.map(|c| format!("/{c}")).unwrap_or_default();
    let Ok(addr) = ip_part.parse::<IpAddr>() else {
        return format!("**masked-ip**{suffix}");
    };
    let masked = match addr {
        IpAddr::V4(v4) => {
            if v4.is_private() || v4.is_loopback() || v4.is_link_local() { v4.to_string() }
            else { let o = v4.octets(); format!("{}.{}.{}.*", o[0], o[1], o[2]) }
        }
        IpAddr::V6(v6) => {
            let seg = v6.segments();
            let ula = (seg[0] & 0xfe00) == 0xfc00;
            let link_local = (seg[0] & 0xffc0) == 0xfe80;
            if v6.is_loopback() || v6.is_unspecified() || ula || link_local { v6.to_string() }
            else { format!("{:x}:{:x}:{:x}:{:x}::*", seg[0], seg[1], seg[2], seg[3]) }
        }
    };
    format!("{masked}{suffix}")
}

/// Replace "/home/<name>/…" with "/home/<user>/…" — home paths show up in
/// mount points, volumes and elsewhere and leak the account name.
pub fn mask_home_paths(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(idx) = rest.find("/home/") {
        out.push_str(&rest[..idx]);
        out.push_str("/home/");
        let after = &rest[idx + 6..];
        let seg_end = after.find(|c| c == '/' || c == ' ' || c == '"' || c == '\'').unwrap_or(after.len());
        let seg = &after[..seg_end];
        if seg.is_empty() || seg == "<user>" { out.push_str(seg); } else { out.push_str("<user>"); }
        rest = &after[seg_end..];
    }
    out.push_str(rest);
    out
}

/// True when the 36 bytes at `i` form a canonical UUID
/// (xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx, hex digits, either case).
fn is_uuid_at(b: &[u8], i: usize) -> bool {
    if i + 36 > b.len() { return false; }
    b[i + 8] == b'-' && b[i + 13] == b'-' && b[i + 18] == b'-' && b[i + 23] == b'-'
        && (0..36).all(|j| matches!(j, 8 | 13 | 18 | 23) || b[i + j].is_ascii_hexdigit())
}

/// Replace every canonical UUID appearing in a string — standalone OR
/// embedded, e.g. `/dev/disk/by-uuid/<uuid>` paths and `luks-<uuid>`
/// volume names — with its stable hash (Gemini audit 2026-10-14: UUIDs and
/// product/machine identifiers rode through exports unscrubbed).
pub fn mask_uuids_in(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0; let mut last = 0;
    while i + 36 <= b.len() {
        // Bounded by non-hex on both sides (a longer hex run isn't a UUID;
        // a '-' neighbour is fine — `luks-<uuid>` names must still mask).
        let bounded = (i == 0 || !b[i - 1].is_ascii_hexdigit())
            && (i + 36 == b.len() || !b[i + 36].is_ascii_hexdigit());
        if bounded && is_uuid_at(b, i) {
            out.push_str(&s[last..i]);
            out.push_str(&hash_id("uuid", &s[i..i + 36]));
            i += 36; last = i;
        } else { i += 1; }
    }
    if last == 0 { return s.to_string(); }
    out.push_str(&s[last..]);
    out
}

/// Hex-identifier masking beyond canonical UUIDs (fix-claim #5 was
/// PARTIAL at v0.8.7 — Gemini re-audit 2026-10-15). Catches:
///  - raw 32-char hex UUIDs (also the /etc/machine-id shape) standing
///    alone as tokens;
///  - FAT volume serials, XXXX-XXXX hex;
///  - any hex/dash identifier sitting in a by-uuid/ or by-partuuid/
///    path segment (NTFS 16-hex volume IDs, odd variants).
/// Deliberately BOUNDED so legitimate hex survives: candidates are
/// standalone tokens only — a 40/64-char hex run (a git SHA, a Docker
/// image ID) is ONE long token and never matches; 12-char short hashes
/// (including this module's own `sha256:…` outputs) are too short to
/// match; and the FAT pattern requires at least one a–f letter, so
/// date-like digit pairs ("2024-1024") are not redacted.
pub fn mask_hex_ids_in(s: &str) -> String {
    let b = s.as_bytes();
    let is_tok = |c: u8| c.is_ascii_hexdigit() || c == b'-';
    let mut out = String::with_capacity(s.len());
    let mut i = 0; let mut last = 0;
    while i < b.len() {
        if is_tok(b[i]) && (i == 0 || !is_tok(b[i - 1])) {
            // Token start: consume the maximal hex/dash run, then trim
            // separator dashes off both ends for classification.
            let start = i;
            while i < b.len() && is_tok(b[i]) { i += 1; }
            let mut cs = start; let mut ce = i;
            while cs < ce && b[cs] == b'-' { cs += 1; }
            while ce > cs && b[ce - 1] == b'-' { ce -= 1; }
            let tok = &s[cs..ce];
            let tb = tok.as_bytes();
            let in_id_path = s[..cs].ends_with("by-uuid/") || s[..cs].ends_with("by-partuuid/");
            let replacement = if in_id_path && (8..=36).contains(&tok.len())
                && tb.iter().all(|c| c.is_ascii_hexdigit() || *c == b'-') {
                Some(hash_id("uuid", tok))
            } else if tok.len() == 32 && tb.iter().all(|c| c.is_ascii_hexdigit()) {
                Some(hash_id("uuid", tok))
            } else if tok.len() == 9 && tb[4] == b'-'
                && tb.iter().enumerate().all(|(j, c)| j == 4 || c.is_ascii_hexdigit())
                && tb.iter().any(|c| c.is_ascii_alphabetic()) {
                Some(hash_id("volserial", tok))
            } else { None };
            if let Some(r) = replacement {
                out.push_str(&s[last..cs]);
                out.push_str(&r);
                last = ce;
            }
        } else { i += 1; }
    }
    if last == 0 { return s.to_string(); }
    out.push_str(&s[last..]);
    out
}

/// If the whole string is a MAC address (6 or 8 hex byte pairs, ':' or '-'
/// separated), return it masked; otherwise None. Key-independent: bridge
/// and permanent hardware addresses hide under many key names.
fn mask_mac_value(s: &str) -> Option<String> {
    let sep = if s.contains(':') { ':' } else if s.contains('-') { '-' } else { return None };
    let parts: Vec<&str> = s.split(sep).collect();
    if (parts.len() == 6 || parts.len() == 8)
        && parts.iter().all(|p| p.len() == 2 && p.chars().all(|c| c.is_ascii_hexdigit())) {
        let keep = &parts[..3];
        let stars = vec!["**"; parts.len() - 3];
        Some(format!("{}:{}", keep.join(":"), stars.join(":")))
    } else { None }
}

/// Identifier scrubbing for any string value, whatever key it sits under:
/// embedded UUIDs hashed, other identifier-shaped hex masked, and a
/// whole-string MAC masked.
fn scrub_string(s: &str) -> String {
    let s = mask_hex_ids_in(&mask_uuids_in(s));
    if let Some(m) = mask_mac_value(&s) { return m; }
    s
}

pub fn sanitize_json(v: &mut serde_json::Value) {
    match v {
        serde_json::Value::Object(map) => {
            for (k, val) in map.iter_mut() {
                let key = k.to_lowercase();
                if key == "accounts" {
                    // Usernames + home paths + last-login lines are personal:
                    // hash names (stable correlation), drop the rest.
                    if let Some(arr) = val.as_array_mut() {
                        for entry in arr.iter_mut() {
                            if let Some(obj) = entry.as_object_mut() {
                                if let Some(n) = obj.get("name").and_then(|x| x.as_str()).map(|x| x.to_string()) {
                                    if n != "root" { obj.insert("name".into(), serde_json::Value::String(hash_id("user", &n))); }
                                }
                                obj.insert("home".into(), serde_json::Value::Null);
                                obj.insert("last_login".into(), serde_json::Value::Null);
                            }
                        }
                    }
                } else if key == "address" {
                    if let Some(s) = val.as_str() { if s.contains(':') { *val = serde_json::Value::String(mask_mac(s)); } }
                } else if key == "mac" {
                    if let Some(s) = val.as_str() { *val = serde_json::Value::String(mask_mac(s)); }
                } else if key.contains("serial") || key == "hostname"
                    || key.contains("uuid") || key.contains("machine_id") || key.contains("machineid") {
                    if let Some(s) = val.as_str() {
                        if !s.is_empty() { *val = serde_json::Value::String(hash_id(&key, s)); }
                    }
                } else if key == "addresses" {
                    if let Some(arr) = val.as_array_mut() {
                        for a in arr.iter_mut() {
                            if let Some(s) = a.as_str() { *a = serde_json::Value::String(mask_ip(s)); }
                        }
                    }
                }
                // Home paths can ride along in any string field (mounts, volumes…),
                // and so can UUIDs and MACs under keys nobody anticipated.
                if let serde_json::Value::String(s) = val {
                    let m = scrub_string(&mask_home_paths(s));
                    if m != *s { *val = serde_json::Value::String(m); }
                }
                sanitize_json(val);
            }
        }
        serde_json::Value::Array(arr) => for item in arr.iter_mut() {
            if let serde_json::Value::String(s) = item {
                let m = scrub_string(s);
                if m != *s { *item = serde_json::Value::String(m); }
            }
            sanitize_json(item);
        },
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn mac_masked() { assert_eq!(mask_mac("aa:bb:cc:dd:ee:ff"), "aa:bb:cc:**:**:**"); }
    #[test] fn hash_stable() { assert_eq!(hash_id("serial", "ABC123"), hash_id("serial", "ABC123")); assert_ne!(hash_id("serial", "ABC123"), hash_id("serial", "XYZ")); }
    #[test] fn sanitize_redacts_accounts() {
        let mut v = serde_json::json!({"accounts":[{"name":"dickles","uid":1000,"home":"/home/dickles","last_login":"dickles pts/0 192.168.1.5 Mon"}],
            "bluetooth":[{"name":"Mouse","address":"AA:BB:CC:DD:EE:FF"}]});
        sanitize_json(&mut v);
        let s = v.to_string();
        assert!(!s.contains("dickles")); assert!(!s.contains("192.168.1.5")); assert!(!s.contains("DD:EE:FF"));
        assert!(s.contains("Mouse"));
    }
    #[test] fn ip_masking() {
        assert_eq!(mask_ip("192.168.1.20"), "192.168.1.20");           // private kept
        assert_eq!(mask_ip("10.0.0.5/24"), "10.0.0.5/24");
        assert_eq!(mask_ip("172.20.3.4"), "172.20.3.4");               // private 172.16/12
        assert_eq!(mask_ip("8.8.8.8"), "8.8.8.*");                     // public v4
        assert_eq!(mask_ip("2600:1700:1234:5678:9abc:def0:1111:2222"), "2600:1700:1234:5678::*");
        assert_eq!(mask_ip("2600:1700:1234:5678::1/64"), "2600:1700:1234:5678::*/64");
        assert_eq!(mask_ip("fe80::1"), "fe80::1");                     // link-local kept
        assert_eq!(mask_ip("fd00::42"), "fd00::42");                   // ULA kept
        assert_eq!(mask_ip("::1"), "::1");
        assert_eq!(mask_ip("not-an-ip"), "**masked-ip**");
    }
    #[test] fn home_paths_masked() {
        assert_eq!(mask_home_paths("/home/dickles/Videos"), "/home/<user>/Videos");
        assert_eq!(mask_home_paths("/dev/nvme0n1p2"), "/dev/nvme0n1p2");
        let mut v = serde_json::json!({"volumes":[{"mount":"/home/dickles"}]});
        sanitize_json(&mut v);
        assert!(!v.to_string().contains("dickles"));
    }
    #[test] fn sanitize_redacts_uuids_and_machine_ids() {
        let mut v = serde_json::json!({
            "system": {"product_uuid": "123e4567-e89b-12d3-a456-426614174000", "machine_id": "9f8e7d6c5b4a39281706f5e4d3c2b1a0"},
            "storage": [{"device": "/dev/disk/by-uuid/123e4567-e89b-12d3-a456-426614174999", "name": "luks-123e4567-e89b-12d3-a456-426614174888"}],
            "network": [{"name": "br0", "permanent_hw": "AA:BB:CC:11:22:33"}],
            "memory": [{"part_number": "LD4AS016G-3200ST", "type": "DDR4"}]
        });
        sanitize_json(&mut v);
        let s = v.to_string();
        assert!(!s.contains("123e4567"), "{s}");
        assert!(!s.contains("9f8e7d6c5b4a39281706f5e4d3c2b1a0"), "{s}");
        assert!(!s.contains("11:22:33"), "{s}");
        // Non-identifier strings survive untouched.
        assert!(s.contains("LD4AS016G-3200ST"), "{s}");
        assert!(s.contains("DDR4"), "{s}");
        assert!(s.contains("/dev/disk/by-uuid/sha256:"), "{s}");
    }
    #[test] fn sanitize_redacts_serial_and_hostname() {
        let mut v = serde_json::json!({"hostname":"mybox","storage":[{"serial":"WD-SECRET123","model":"_NVMe"}]});
        sanitize_json(&mut v);
        let s = v.to_string();
        assert!(!s.contains("WD-SECRET123")); assert!(!s.contains("mybox")); assert!(s.contains("_NVMe"));
    }
    #[test] fn hex_id_variants_masked_under_any_key() {
        // The v0.8.7 PARTIAL: raw-hex UUIDs, FAT serials and by-uuid
        // path IDs rode through under non-keyword keys.
        let mut v = serde_json::json!({
            "misc": [
                "disk 9f8e7d6c5b4a39281706f5e4d3c2b1a0 failed",
                "vol ABCD-1234 mounted",
                "/dev/disk/by-uuid/A2B4C6D8E0F1A2B3",
                "/dev/disk/by-partuuid/01234567-89ab-cdef-0123-456789abcdef"
            ]
        });
        sanitize_json(&mut v);
        let s = v.to_string();
        assert!(!s.contains("9f8e7d6c5b4a39281706f5e4d3c2b1a0"), "{s}");
        assert!(!s.contains("ABCD-1234"), "{s}");
        assert!(!s.contains("A2B4C6D8E0F1A2B3"), "{s}");
        assert!(!s.contains("01234567-89ab"), "{s}");
        assert!(s.contains("sha256:"), "{s}");
    }
    #[test] fn legit_hex_and_versions_survive() {
        // The other half of the contract: bounded patterns must NOT
        // over-redact content the report intentionally shows.
        let keep = [
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855", // 64-hex docker/sha256 id
            "da39a3ee5e6b4b0d3255bfef95601890afd80709",                        // 40-hex SHA-1
            "sha256:9f8e7d6c5b4a",                                          // own short-hash output (12 hex)
            "2024-1024",                                                    // date-like digits, no hex letter
            "6.8.0-45-generic",                                             // kernel version
            "LD4AS016G-3200ST",                                             // memory part number
            "image 12345678-9abc-def0-1234-56789abcdef0 pulled",             // canonical UUID handled by uuid pass
        ];
        for k in keep {
            let masked = mask_hex_ids_in(k);
            if !k.contains("12345678-9abc") {
                assert_eq!(masked, k, "over-redacted: {k}");
            }
        }
        // The FAT pattern needs a hex letter: all-digit pairs survive.
        assert_eq!(mask_hex_ids_in("1234-5678"), "1234-5678");
        assert_eq!(mask_hex_ids_in("AB12-34CD"), hash_id("volserial", "AB12-34CD"));
    }
}
