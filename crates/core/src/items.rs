//! The `items` column of courses.csv: which playlist positions to download.
//!
//! Syntax (positions are 1-based playlist indexes, the same `#` as in the plan files):
//!   all            every video
//!   1-5,8          only these
//!   all-2,15       everything except these (a range is allowed after the dash: all-3-5,9)

use anyhow::{bail, Context, Result};
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemSpec {
    All,
    Only(Vec<(u32, u32)>),
    AllExcept(Vec<(u32, u32)>),
}

fn parse_ranges(s: &str) -> Result<Vec<(u32, u32)>> {
    let mut out = Vec::new();
    for part in s.split(',') {
        let part = part.trim();
        if part.is_empty() {
            bail!("empty entry (stray comma?)");
        }
        let (lo, hi) = match part.split_once('-') {
            Some((a, b)) => (
                a.trim().parse::<u32>().with_context(|| format!("bad number '{a}'"))?,
                b.trim().parse::<u32>().with_context(|| format!("bad number '{b}'"))?,
            ),
            None => {
                let n = part.parse::<u32>().with_context(|| format!("bad number '{part}'"))?;
                (n, n)
            }
        };
        if lo == 0 || hi == 0 {
            bail!("positions start at 1, got '{part}'");
        }
        if lo > hi {
            bail!("range '{part}' is backwards");
        }
        out.push((lo, hi));
    }
    Ok(out)
}

impl ItemSpec {
    pub fn parse(s: &str) -> Result<ItemSpec> {
        let t = s.trim().to_ascii_lowercase();
        if t.is_empty() {
            bail!("items is empty (use 'all' or e.g. '1-5,8')");
        }
        if t == "all" {
            return Ok(ItemSpec::All);
        }
        if let Some(rest) = t.strip_prefix("all") {
            let rest = rest.trim_start();
            if let Some(ex) = rest.strip_prefix('-') {
                return Ok(ItemSpec::AllExcept(parse_ranges(ex)?));
            }
            bail!("'{s}': after 'all' write '-' then what to leave out, e.g. all-2,15");
        }
        Ok(ItemSpec::Only(parse_ranges(&t)?))
    }

    /// The wanted positions, given how many videos the playlist has. Positions past `total` are dropped.
    pub fn resolve(&self, total: u32) -> Vec<u32> {
        let expand = |rs: &[(u32, u32)]| -> BTreeSet<u32> {
            rs.iter().flat_map(|&(a, b)| a..=b.min(total)).collect()
        };
        match self {
            ItemSpec::All => (1..=total).collect(),
            ItemSpec::Only(r) => expand(r).into_iter().collect(),
            ItemSpec::AllExcept(r) => {
                let skip = expand(r);
                (1..=total).filter(|i| !skip.contains(i)).collect()
            }
        }
    }

    /// Highest position named explicitly (for warning when a playlist is shorter than the CSV expects).
    pub fn max_named(&self) -> Option<u32> {
        match self {
            ItemSpec::All => None,
            ItemSpec::Only(r) | ItemSpec::AllExcept(r) => r.iter().map(|&(_, b)| b).max(),
        }
    }
}

/// Compress positions into yt-dlp's `--playlist-items` syntax: 1,3-5,9
pub fn to_ytdlp(items: &[u32]) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < items.len() {
        let start = items[i];
        let mut end = start;
        while i + 1 < items.len() && items[i + 1] == end + 1 {
            i += 1;
            end = items[i];
        }
        out.push(if start == end { start.to_string() } else { format!("{start}-{end}") });
        i += 1;
    }
    out.join(",")
}

/// Compress positions into the CSV `items` syntax (same text as yt-dlp's).
pub fn to_csv_items(items: &[u32]) -> String {
    to_ytdlp(items)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all() {
        assert_eq!(ItemSpec::parse(" ALL ").unwrap(), ItemSpec::All);
        assert_eq!(ItemSpec::All.resolve(4), vec![1, 2, 3, 4]);
    }

    #[test]
    fn parses_lists_and_ranges() {
        let s = ItemSpec::parse("1-3, 8,5").unwrap();
        assert_eq!(s.resolve(100), vec![1, 2, 3, 5, 8]);
        assert_eq!(s.max_named(), Some(8));
    }

    #[test]
    fn clamps_to_playlist_length() {
        let s = ItemSpec::parse("2-10").unwrap();
        assert_eq!(s.resolve(4), vec![2, 3, 4]);
    }

    #[test]
    fn all_except() {
        let s = ItemSpec::parse("all-2,4").unwrap();
        assert_eq!(s.resolve(5), vec![1, 3, 5]);
        let s = ItemSpec::parse("all-2-3,5").unwrap();
        assert_eq!(s.resolve(6), vec![1, 4, 6]);
    }

    #[test]
    fn rejects_bad() {
        for bad in ["", "0", "3-1", "a", "1,,2", "allx", "all2", "1-"] {
            assert!(ItemSpec::parse(bad).is_err(), "{bad} should fail");
        }
    }

    #[test]
    fn compresses() {
        assert_eq!(to_ytdlp(&[1, 2, 3, 5, 7, 8]), "1-3,5,7-8");
        assert_eq!(to_ytdlp(&[]), "");
    }
}
