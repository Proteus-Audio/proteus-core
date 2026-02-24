use std::collections::HashMap;
use std::fs;
use std::io::{self, Write};

#[derive(Debug, Default)]
struct TypeData {
    // timestamps in the order they were encountered for this type
    timestamps: Vec<String>,
    // item ids (i0, i1, ...) in first-seen order for stable output
    items_order: Vec<String>,
    // item_id -> frames (one per timestamp, in timestamp order)
    item_frames: HashMap<String, Vec<String>>,
    // maximum segment width for this type (computed from frames)
    seg_width: usize,
}

fn is_type_line(s: &str) -> bool {
    let s = s.trim();
    if s.len() < 2 {
        return false;
    }
    let mut chars = s.chars();
    matches!(chars.next(), Some('T')) && chars.all(|c| c.is_ascii_digit())
}

fn parse_item_line(line: &str) -> Result<(String, String), String> {
    // Expect something like: "[_____]" <- i0
    let parts: Vec<&str> = line.split("<-").collect();
    if parts.len() != 2 {
        return Err(format!("Bad item line (missing '<-'): {line}"));
    }
    let frame = parts[0].trim_end().to_string();
    let item = parts[1].trim().to_string();
    if item.is_empty() {
        return Err(format!("Bad item line (empty item id): {line}"));
    }
    Ok((frame, item))
}

pub fn pivot_buffer() -> Result<(), Box<dyn std::error::Error>> {
    let in_path = String::from("log.txt");
    let out_path = Some(String::from("log-fmt.txt"));

    let input = fs::read_to_string(&in_path)?;

    // Read all lines (keep original content, but we will ignore empty lines)
    let lines: Vec<String> = input.lines().map(|l| l.to_string()).collect();

    // Preserve first-seen order of T-keys
    let mut type_order: Vec<String> = Vec::new();
    let mut types: HashMap<String, TypeData> = HashMap::new();

    let mut i = 0usize;
    while i < lines.len() {
        let line = lines[i].trim();
        if line.is_empty() {
            i += 1;
            continue;
        }

        if !is_type_line(line) {
            return Err(format!("Expected a type line like 'T0', got: '{}'", lines[i]).into());
        }
        let tkey = line.to_string();

        if !types.contains_key(&tkey) {
            type_order.push(tkey.clone());
            types.insert(tkey.clone(), TypeData::default());
        }

        i += 1;
        // Skip empty lines
        while i < lines.len() && lines[i].trim().is_empty() {
            i += 1;
        }
        if i >= lines.len() {
            return Err(format!("Missing timestamp after type {}", tkey).into());
        }

        let ts_line = lines[i].trim();
        // Validate it's an integer-like timestamp (but keep as string)
        if ts_line.parse::<i64>().is_err() {
            return Err(format!("Bad timestamp '{}' after type {}", ts_line, tkey).into());
        }
        let timestamp = ts_line.to_string();

        let tdata = types.get_mut(&tkey).unwrap();

        // Add timestamp if it's new at the end (typical case).
        // (If repeats or out-of-order occur, you can tighten this logic.)
        if tdata
            .timestamps
            .last()
            .map(|s| s != &timestamp)
            .unwrap_or(true)
        {
            tdata.timestamps.push(timestamp);
        }
        let current_ts_index = tdata.timestamps.len() - 1;

        i += 1;

        // Consume item lines until next type line or EOF
        while i < lines.len() {
            let s = lines[i].trim();
            if s.is_empty() {
                i += 1;
                continue;
            }
            if is_type_line(s) {
                break; // next block
            }

            let (frame, item_id) =
                parse_item_line(&lines[i]).map_err(|e| format!("At line {}: {}", i + 1, e))?;

            // Track segment width
            tdata.seg_width = tdata.seg_width.max(frame.chars().count());

            if !tdata.item_frames.contains_key(&item_id) {
                tdata.items_order.push(item_id.clone());
                tdata.item_frames.insert(item_id.clone(), Vec::new());
            }

            let frames = tdata.item_frames.get_mut(&item_id).unwrap();

            // Ensure frames vector is long enough up to current timestamp index
            while frames.len() < current_ts_index {
                frames.push(String::new()); // placeholder for missing earlier timestamps
            }
            if frames.len() == current_ts_index {
                frames.push(frame);
            } else {
                // If we ever see multiple entries for same item at same timestamp, overwrite.
                frames[current_ts_index] = frame;
            }

            i += 1;
        }
    }

    // Build output
    let mut out = String::new();

    for tkey in &type_order {
        let tdata = types.get(tkey).unwrap();
        let seg_width = tdata.seg_width.max(1);

        out.push_str(tkey);
        out.push('\n');

        // Header: timestamps aligned to seg_width, concatenated
        for ts in &tdata.timestamps {
            // left-align timestamp inside seg width
            out.push_str(&format!("{:<width$}", ts, width = seg_width));
        }
        out.push('\n');

        // Each item: concatenate frames across timestamps
        for item_id in &tdata.items_order {
            let frames = tdata.item_frames.get(item_id).unwrap();
            for idx in 0..tdata.timestamps.len() {
                let seg = frames.get(idx).cloned().unwrap_or_default();
                if seg.is_empty() {
                    // missing frame: spaces of seg_width
                    out.push_str(&" ".repeat(seg_width));
                } else {
                    // pad / trim to seg_width by char count
                    let seg_chars: Vec<char> = seg.chars().collect();
                    if seg_chars.len() == seg_width {
                        out.push_str(&seg);
                    } else if seg_chars.len() < seg_width {
                        out.push_str(&seg);
                        out.push_str(&" ".repeat(seg_width - seg_chars.len()));
                    } else {
                        // truncate if longer than expected
                        out.push_str(&seg_chars[..seg_width].iter().collect::<String>());
                    }
                }
            }
            out.push_str(" <- ");
            out.push_str(item_id);
            out.push('\n');
        }
    }

    match out_path {
        Some(p) => fs::write(p, out)?,
        None => {
            let mut stdout = io::BufWriter::new(io::stdout());
            stdout.write_all(out.as_bytes())?;
        }
    }

    Ok(())
}
