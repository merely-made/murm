//! `linkboy qualify`: the bench step that turns a vendor release URL into a draft entry.
//!
//! It fetches the file once (only when a person runs it), reports its length, sha256 and the
//! layout Linkboy detects, and writes a draft manifest whose human fields say FILL. A draft is
//! not a package: a person completes it, checks it with `linkboy inspect`, and records what
//! was checked in its `[qualification]` table (firmware/packages/QUALIFYING.md).

use crate::download::{DownloadError, Fetch, fetch_capped};
use crate::package::{FlashRange, uf2_extent};

/// The largest release `qualify` will read. Meshtastic's per-board ESP32-S3 littlefs image,
/// the largest single file named in the assessment, is 3,538,944 bytes.
pub const QUALIFY_LIMIT: u64 = 64 * 1024 * 1024;

/// What the bytes look like to Linkboy's own checks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DetectedLayout {
    /// Every block valid; `nrf52840` when every block carries the nRF52840 family id.
    Uf2 {
        nrf52840: bool,
        write_bytes: u64,
        ranges: Vec<FlashRange>,
    },
    /// Starts with the ESP image header magic `0xE9`.
    EspImage,
    Unknown,
}

impl DetectedLayout {
    pub fn detect(bytes: &[u8]) -> Self {
        for nrf52840 in [true, false] {
            if let Ok((write_bytes, ranges)) = uf2_extent(bytes, nrf52840) {
                return Self::Uf2 {
                    nrf52840,
                    write_bytes,
                    ranges,
                };
            }
        }
        if bytes.first() == Some(&0xE9) {
            Self::EspImage
        } else {
            Self::Unknown
        }
    }

    pub fn describe(&self) -> String {
        match self {
            Self::Uf2 {
                nrf52840,
                write_bytes,
                ranges,
            } => format!(
                "UF2, {} family id, {write_bytes} payload bytes, blocks span {}",
                if *nrf52840 { "nRF52840" } else { "no nRF52840" },
                ranges
                    .iter()
                    .map(|range| format!("{:#x}+{:#x}", range.start, range.length))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::EspImage => "ESP image (header magic 0xE9 at byte 0)".into(),
            Self::Unknown => "not a layout Linkboy recognises".into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Draft {
    pub url: String,
    pub byte_length: u64,
    pub sha256: String,
    pub layout: DetectedLayout,
    /// A draft manifest; every FILL value is for a person to complete.
    pub toml: String,
}

/// Fetch `url` once and draft an entry for it, dated `recorded_on` (`YYYY-MM-DD`).
pub fn qualify(fetcher: &dyn Fetch, url: &str, recorded_on: &str) -> Result<Draft, DownloadError> {
    let fetched = fetch_capped(fetcher, url, QUALIFY_LIMIT)?;
    Ok(draft(url, &fetched.bytes, &fetched.sha256, recorded_on))
}

/// Draft an entry from bytes already in hand.
pub fn draft(url: &str, bytes: &[u8], sha256: &str, recorded_on: &str) -> Draft {
    let layout = DetectedLayout::detect(bytes);
    let byte_length = bytes.len() as u64;
    let (format, write_bytes, ranges, structure) = match &layout {
        DetectedLayout::Uf2 {
            nrf52840,
            write_bytes,
            ranges,
        } => (
            "uf2",
            *write_bytes,
            ranges.clone(),
            format!(
                "Linkboy's UF2 layout check: every block's magic, {}a complete block sequence, {write_bytes} payload bytes, block span equal to write_ranges",
                if *nrf52840 {
                    "the nRF52840 family id, "
                } else {
                    ""
                }
            ),
        ),
        _ => ("FILL", byte_length, Vec::new(), "FILL".to_string()),
    };
    let mut toml = format!(
        "# Draft written by `linkboy qualify` on {recorded_on}. It is not a package until a\n\
         # person completes every FILL value and `linkboy inspect` accepts it\n\
         # (firmware/packages/QUALIFYING.md). Detected layout: {}.\n\
         schema = 2\n\
         package_id = \"FILL\"\n\
         display_name = \"FILL\"\n\
         version = \"FILL\"\n\
         publisher = \"FILL\"\n\
         regions = []\n\
         channel_capabilities = []\n\
         state_impact = \"unknown\"\n\
         license = \"FILL\"\n\
         notices = \"FILL\"\n\
         source_revision = \"FILL\"\n\
         source_url = \"FILL\"\n\
         origin_url = \"FILL\"\n\n\
         [payload]\n\
         url = \"{url}\"\n\
         format = \"{format}\"\n\
         byte_length = {byte_length}\n\
         sha256 = \"{sha256}\"\n\
         write_bytes = {write_bytes}\n",
        layout.describe()
    );
    for range in &ranges {
        toml.push_str(&format!(
            "\n[[write_ranges]]\nstart = {:#x}\nlength = {:#x}\n",
            range.start, range.length
        ));
    }
    toml.push_str(&format!(
        "\n[qualification]\n\
         hash_source = \"Length and sha256 computed by linkboy qualify from {url} on {recorded_on}; vendor digest cross-checked: FILL\"\n\
         hash_recorded_on = \"{recorded_on}\"\n\
         structure_checks = \"{structure}\"\n\
         companion_checked = false\n"
    ));
    Draft {
        url: url.into(),
        byte_length,
        sha256: sha256.into(),
        layout,
        toml,
    }
}

/// Today's UTC date as `YYYY-MM-DD`, from the system clock.
pub fn today() -> String {
    let days = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() / 86_400)
        .unwrap_or(0) as i64;
    civil_date(days)
}

/// Days since 1970-01-01 to a proleptic Gregorian date (Howard Hinnant's algorithm).
fn civil_date(days: i64) -> String {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_dates_match_known_days() {
        assert_eq!(civil_date(0), "1970-01-01");
        assert_eq!(civil_date(20_406), "2025-11-14");
        assert_eq!(civil_date(20_771), "2026-11-14");
    }
}
