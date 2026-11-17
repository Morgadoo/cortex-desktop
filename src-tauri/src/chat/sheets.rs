//! Spreadsheets, as in Vibe Work: the model creates and edits tables as `sheet` canvases (CSV),
//! the user edits them as a grid, and they download as Excel workbooks. Uploaded `.xlsx`,
//! `.xls` and `.ods` files are read into CSV so the model can analyse them (with `run_code` for
//! anything heavy).

use std::path::Path;

/// Rows read from an uploaded workbook, per sheet.
const MAX_ROWS: usize = 5_000;

/// Parses CSV (quoted fields, embedded commas, quotes and newlines) into rows.
pub fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.trim_start_matches('\u{feff}').chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            match c {
                '"' if chars.peek() == Some(&'"') => {
                    field.push('"');
                    chars.next();
                }
                '"' => quoted = false,
                _ => field.push(c),
            }
            continue;
        }
        match c {
            '"' if field.is_empty() => quoted = true,
            ',' => row.push(std::mem::take(&mut field)),
            '\r' => {}
            '\n' => {
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
            }
            _ => field.push(c),
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows
}

fn csv_field(value: &str) -> String {
    if value.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

pub fn to_csv(rows: &[Vec<String>]) -> String {
    rows.iter().map(|r| r.iter().map(|v| csv_field(v)).collect::<Vec<_>>().join(",")).collect::<Vec<_>>().join("\n")
}

/// A sheet canvas's content: the CSV, unwrapped from a ```csv fence if the model added one.
pub fn sheet_csv(content: &str) -> String {
    let trimmed = content.trim();
    trimmed
        .strip_prefix("```csv")
        .or_else(|| trimmed.strip_prefix("```"))
        .and_then(|rest| rest.trim_end().strip_suffix("```"))
        .map_or(trimmed, str::trim)
        .to_string()
}

/// Writes rows as an Excel workbook: numbers stay numbers, the first row is a bold header.
pub fn write_xlsx(path: &Path, sheet_name: &str, rows: &[Vec<String>]) -> Result<(), String> {
    use rust_xlsxwriter::{Format, Workbook};
    let mut workbook = Workbook::new();
    let sheet = workbook.add_worksheet();
    let name: String = sheet_name.chars().filter(|c| !"[]:*?/\\".contains(*c)).take(31).collect();
    if !name.trim().is_empty() {
        sheet.set_name(name.trim()).map_err(|e| e.to_string())?;
    }
    let bold = Format::new().set_bold();
    for (r, row) in rows.iter().enumerate() {
        for (c, value) in row.iter().enumerate() {
            let (r, c) = (r as u32, c as u16);
            let number = value.trim().replace(' ', "").parse::<f64>().ok().filter(|n| n.is_finite());
            match (r, number) {
                (0, _) => sheet.write_string_with_format(r, c, value, &bold).map(|_| ()),
                (_, Some(n)) => sheet.write_number(r, c, n).map(|_| ()),
                _ => sheet.write_string(r, c, value).map(|_| ()),
            }
            .map_err(|e| e.to_string())?;
        }
    }
    if let Some(width) = rows.first().map(Vec::len) {
        for c in 0..width {
            let longest = rows.iter().filter_map(|r| r.get(c)).map(|v| v.chars().count()).max().unwrap_or(8);
            sheet.set_column_width(c as u16, (longest.clamp(6, 50) + 2) as f64).map_err(|e| e.to_string())?;
        }
    }
    workbook.save(path).map_err(|e| e.to_string())
}

/// Reads a workbook (`.xlsx`, `.xls`, `.ods`) into CSV text per sheet, for the model.
pub fn read_workbook(bytes: &[u8]) -> Result<Vec<(String, String)>, String> {
    use calamine::{open_workbook_auto_from_rs, Data, Reader};
    let mut workbook = open_workbook_auto_from_rs(std::io::Cursor::new(bytes.to_vec())).map_err(|e| format!("Not a readable spreadsheet: {e}"))?;
    let mut out = Vec::new();
    for name in workbook.sheet_names().to_vec() {
        let Ok(range) = workbook.worksheet_range(&name) else { continue };
        let rows: Vec<Vec<String>> = range
            .rows()
            .take(MAX_ROWS)
            .map(|row| {
                row.iter()
                    .map(|cell| match cell {
                        Data::Empty => String::new(),
                        Data::Float(f) if f.fract() == 0.0 && f.abs() < 1e15 => format!("{}", *f as i64),
                        other => other.to_string(),
                    })
                    .collect()
            })
            .collect();
        if !rows.is_empty() {
            out.push((name, to_csv(&rows)));
        }
    }
    if out.is_empty() {
        return Err("The spreadsheet is empty.".into());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_round_trips_quotes_commas_and_newlines() {
        let text = "Name,Note,Amount\nAna,\"Says \"\"hi\"\", ok\",12.5\nRui,\"two\nlines\",3\n";
        let rows = parse_csv(text);
        assert_eq!(rows, vec![
            vec!["Name".to_string(), "Note".into(), "Amount".into()],
            vec!["Ana".into(), "Says \"hi\", ok".into(), "12.5".into()],
            vec!["Rui".into(), "two\nlines".into(), "3".into()],
        ]);
        assert_eq!(parse_csv(&to_csv(&rows)), rows);
        assert_eq!(sheet_csv("```csv\na,b\n1,2\n```"), "a,b\n1,2");
    }

    #[test]
    fn writes_and_reads_back_a_workbook() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("budget.xlsx");
        let rows = parse_csv("Item,Cost\nRent,1200\nFood,350.5\n");
        write_xlsx(&path, "Budget: May", &rows).unwrap();
        let sheets = read_workbook(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(sheets.len(), 1);
        assert_eq!(sheets[0].0, "Budget May");
        assert_eq!(sheets[0].1, "Item,Cost\nRent,1200\nFood,350.5");
        assert!(read_workbook(b"not a workbook").is_err());
    }
}
