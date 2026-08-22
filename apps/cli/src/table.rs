//! Column output.
//!
//! Two spaces between columns, header in caps, nothing drawn — what `docker
//! ps` prints, and what `awk '{print $1}'` expects.

/// Print `headers` above `rows`, every column as wide as its widest cell.
pub fn print(headers: &[&str], rows: &[Vec<String>]) {
    let mut widths: Vec<usize> = headers.iter().map(|h| h.len()).collect();
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            if let Some(width) = widths.get_mut(i) {
                *width = (*width).max(cell.len());
            }
        }
    }

    let line = |cells: &[String]| {
        let mut out = String::new();
        for (i, cell) in cells.iter().enumerate() {
            if i + 1 == cells.len() {
                out.push_str(cell);
            } else {
                out.push_str(&format!("{cell:<width$}  ", width = widths[i]));
            }
        }
        println!("{}", out.trim_end());
    };

    line(&headers.iter().map(|h| h.to_string()).collect::<Vec<_>>());
    for row in rows {
        line(row);
    }
}

/// `90s`, `12m`, `3h`, `2d` — one unit, the largest that fits.
pub fn duration(secs: u64) -> String {
    match secs {
        s if s < 60 => format!("{s}s"),
        s if s < 3600 => format!("{}m", s / 60),
        s if s < 86400 => format!("{}h", s / 3600),
        s => format!("{}d", s / 86400),
    }
}

/// Cut to `max` characters so a column stays a column.
pub fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_owned();
    }
    let kept: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{kept}…")
}
