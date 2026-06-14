//! Conversion between the IIM Date/Time Created datasets and ISO-8601, for `photoshop:DateCreated`.
//!
//! IIM splits the creation timestamp across two datasets: Date Created (2:55, `CCYYMMDD`) and Time
//! Created (2:60, `HHMMSS±HHMM`) per IPTC-IIM 4.2. IPTC Photo Metadata carries it as a single
//! ISO-8601 `photoshop:DateCreated` string. Unknown components are `00`/`0000` in IIM and truncate
//! the ISO value to `CCYY` or `CCYY-MM` accordingly (IPTC-IIM 4.2 dataset 2:55). These helpers are
//! internal to [`crate::reconcile`].

/// Whether `s` is exactly `n` ASCII digits.
fn digits(s: &str, n: usize) -> bool {
    s.len() == n && s.bytes().all(|b| b.is_ascii_digit())
}

/// Combines IIM Date Created (2:55) octets and optional Time Created (2:60) octets into an ISO-8601
/// string, or `None` if the date is not the expected digit form (or the year is unknown). A
/// malformed time is dropped, keeping the valid date.
pub(crate) fn iim_to_iso(date: &[u8], time: Option<&[u8]>) -> Option<String> {
    let date = core::str::from_utf8(date).ok()?;
    if !digits(date, 8) {
        return None;
    }
    let (year, month, day) = (&date[0..4], &date[4..6], &date[6..8]);
    if year == "0000" {
        return None;
    }
    let mut out = year.to_owned();
    if month == "00" {
        return Some(out);
    }
    out.push('-');
    out.push_str(month);
    if day == "00" {
        return Some(out);
    }
    out.push('-');
    out.push_str(day);
    if let Some(time) = time.and_then(format_iim_time) {
        out.push('T');
        out.push_str(&time);
    }
    Some(out)
}

/// Formats IIM Time Created octets (`HHMMSS` or `HHMMSS±HHMM`) as an ISO clock (`hh:mm:ss` or
/// `hh:mm:ss±hh:mm`).
fn format_iim_time(time: &[u8]) -> Option<String> {
    let t = core::str::from_utf8(time).ok()?;
    if !t.is_ascii() {
        return None; // keep the byte-indexed splits below on char boundaries
    }
    let (hms, zone) = t.split_at(t.len().min(6));
    if !digits(hms, 6) {
        return None;
    }
    let mut out = format!("{}:{}:{}", &hms[0..2], &hms[2..4], &hms[4..6]);
    if !zone.is_empty() {
        let sign = &zone[0..1];
        if (sign != "+" && sign != "-") || !digits(&zone[1..], 4) {
            return None;
        }
        out.push_str(sign);
        out.push_str(&zone[1..3]);
        out.push(':');
        out.push_str(&zone[3..5]);
    }
    Some(out)
}

/// Splits an ISO-8601 `photoshop:DateCreated` string into IIM Date Created (8 octets) and optional
/// Time Created (6 or 11 octets), or `None` if it is not a recognised date/date-time form.
pub(crate) fn iso_to_iim(iso: &str) -> Option<(Vec<u8>, Option<Vec<u8>>)> {
    let (date_part, time_part) = match iso.split_once('T') {
        Some((d, t)) => (d, Some(t)),
        None => (iso, None),
    };
    let date = parse_iso_date(date_part)?;
    let time = match time_part {
        Some(t) => Some(parse_iso_time(t)?.into_bytes()),
        None => None,
    };
    Some((date.into_bytes(), time))
}

/// Parses `CCYY`, `CCYY-MM`, or `CCYY-MM-DD` into the 8-octet `CCYYMMDD` form (`00` for absent
/// month/day).
fn parse_iso_date(s: &str) -> Option<String> {
    let mut parts = s.split('-');
    let year = parts.next()?;
    let month = parts.next().unwrap_or("00");
    let day = parts.next().unwrap_or("00");
    if parts.next().is_some() || !digits(year, 4) {
        return None;
    }
    if !digits(month, 2) || !digits(day, 2) {
        return None;
    }
    Some(format!("{year}{month}{day}"))
}

/// Parses `hh:mm:ss` with an optional `Z` or `±hh:mm` zone into the IIM `HHMMSS±HHMM` form (or
/// `HHMMSS` with no zone).
fn parse_iso_time(s: &str) -> Option<String> {
    let (clock, zone) = if let Some(c) = s.strip_suffix('Z') {
        (c, Some("+0000".to_owned()))
    } else if let Some(pos) = s.find(['+', '-']) {
        // `pos` is at an ASCII sign, so `&z[1..]` below stays on a char boundary.
        let (c, z) = s.split_at(pos);
        let z: String = z.chars().filter(|&ch| ch != ':').collect();
        if !digits(&z[1..], 4) {
            return None;
        }
        (c, Some(z))
    } else {
        (s, None)
    };
    let mut parts = clock.split(':');
    let (h, m, sec) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() {
        return None;
    }
    for p in [h, m, sec] {
        if !digits(p, 2) {
            return None;
        }
    }
    let mut out = format!("{h}{m}{sec}");
    if let Some(z) = zone {
        out.push_str(&z);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_datetime_roundtrips() {
        let date = b"19900127";
        let time = b"133015+0100";
        let iso = iim_to_iso(date, Some(time)).unwrap();
        assert_eq!(iso, "1990-01-27T13:30:15+01:00");
        assert_eq!(iso_to_iim(&iso), Some((date.to_vec(), Some(time.to_vec()))));
    }

    #[test]
    fn date_without_time() {
        assert_eq!(iim_to_iso(b"20240615", None).unwrap(), "2024-06-15");
        assert_eq!(iso_to_iim("2024-06-15"), Some((b"20240615".to_vec(), None)));
    }

    #[test]
    fn time_without_zone() {
        assert_eq!(
            iim_to_iso(b"20240615", Some(b"120000")).unwrap(),
            "2024-06-15T12:00:00"
        );
        assert_eq!(
            iso_to_iim("2024-06-15T12:00:00"),
            Some((b"20240615".to_vec(), Some(b"120000".to_vec())))
        );
    }

    #[test]
    fn partial_dates_truncate() {
        // Unknown day -> CCYY-MM; unknown month -> CCYY.
        assert_eq!(iim_to_iso(b"20240600", None).unwrap(), "2024-06");
        assert_eq!(iim_to_iso(b"20240000", None).unwrap(), "2024");
        assert_eq!(iso_to_iim("2024-06"), Some((b"20240600".to_vec(), None)));
        assert_eq!(iso_to_iim("2024"), Some((b"20240000".to_vec(), None)));
        // A partial date drops any time, since ISO cannot carry time without a full date.
        assert_eq!(iim_to_iso(b"20240600", Some(b"120000")).unwrap(), "2024-06");
    }

    #[test]
    fn zulu_and_offset_zones() {
        assert_eq!(
            iim_to_iso(b"20240615", Some(b"120000-0500")).unwrap(),
            "2024-06-15T12:00:00-05:00"
        );
        // ISO 'Z' maps to a +0000 IIM offset.
        assert_eq!(
            iso_to_iim("2024-06-15T12:00:00Z"),
            Some((b"20240615".to_vec(), Some(b"120000+0000".to_vec())))
        );
    }

    #[test]
    fn malformed_time_is_dropped_keeping_the_date() {
        // A bad Time Created must not lose the valid Date Created.
        assert_eq!(
            iim_to_iso(b"20240615", Some(b"1200")).unwrap(),
            "2024-06-15"
        ); // short
        assert_eq!(
            iim_to_iso(b"20240615", Some(b"99XX99")).unwrap(),
            "2024-06-15"
        ); // non-digit hms
        assert_eq!(
            iim_to_iso(b"20240615", Some(b"133015X0100")).unwrap(),
            "2024-06-15"
        ); // bad zone sign
        assert_eq!(
            iim_to_iso(b"20240615", Some(b"133015+01X0")).unwrap(),
            "2024-06-15"
        ); // bad zone digit
        // Non-ASCII time octets are rejected without panicking on the split.
        assert_eq!(
            iim_to_iso(b"20240615", Some("12345é".as_bytes())).unwrap(),
            "2024-06-15"
        );
    }

    #[test]
    fn rejects_malformed_dates() {
        assert_eq!(iim_to_iso(b"2024", None), None); // wrong length
        assert_eq!(iim_to_iso(b"20X40615", None), None); // non-digit
        assert_eq!(iim_to_iso(b"00000000", None), None); // unknown year
        assert_eq!(iso_to_iim("2024-06-15-01"), None); // too many components
        assert_eq!(iso_to_iim("24-06-15"), None); // 2-digit year
        assert_eq!(iso_to_iim("2024-6-15"), None); // 1-digit month
        assert_eq!(iso_to_iim("2024-06-5"), None); // 1-digit day
    }

    #[test]
    fn rejects_malformed_times() {
        assert_eq!(iso_to_iim("2024-06-15T12:00"), None); // missing seconds
        assert_eq!(iso_to_iim("2024-06-15T123:00:00"), None); // 3-digit hour
        assert_eq!(iso_to_iim("2024-06-15T12:0:00"), None); // 1-digit minute
        assert_eq!(iso_to_iim("2024-06-15T12:00:00+1"), None); // short zone
    }
}
