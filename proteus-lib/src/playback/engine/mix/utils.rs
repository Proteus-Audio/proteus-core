#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitionDirection {
    Up,
    Down,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cover {
    Overlap((usize, usize)),
    Underlay((usize, usize)),
    Transition((TransitionDirection, (usize, usize))),
}

fn normalize_ranges(mut ranges: Vec<(usize, usize)>) -> Vec<(usize, usize)> {
    ranges.retain(|&(s, e)| s < e);
    if ranges.is_empty() {
        return ranges;
    }
    ranges.sort_by_key(|&(s, _)| s);

    let mut out = Vec::with_capacity(ranges.len());
    let mut cur = ranges[0];
    for &(s, e) in &ranges[1..] {
        if s <= cur.1 {
            cur.1 = cur.1.max(e);
        } else {
            out.push(cur);
            cur = (s, e);
        }
    }
    out.push(cur);
    out
}

fn push_cover(out: &mut Vec<Cover>, c: Cover) {
    let (s, e) = match c {
        Cover::Overlap((s, e)) => (s, e),
        Cover::Underlay((s, e)) => (s, e),
        Cover::Transition((_, (s, e))) => (s, e),
    };
    if s >= e {
        return;
    }

    // Coalesce adjacent Underlay/Overlap segments
    match (out.last_mut(), c) {
        (Some(Cover::Underlay((ls, le))), Cover::Underlay((s, e))) if *le == s => {
            *le = e;
        }
        (Some(Cover::Overlap((ls, le))), Cover::Overlap((s, e))) if *le == s => {
            *le = e;
        }
        _ => out.push(c),
    }
}

/// Build a full cover map of Underlay/Overlap (+ optional Transitions).
///
/// Ranges are half-open: (start, end) means start..end.
///
/// Rules implemented per your examples:
/// - transition_size must be even.
/// - For an overlap to be kept:
///   - if it needs both Up and Down (i.e., 0 < start and end < sample_len), it must have len >= transition_size.
///   - if it touches an edge (only one transition needed), it must have len >= transition_size/2.
/// - “Partial transitions” happen only when the transition would extend outside 0..sample_len.
///   In that case, the edge side shortens, but the other side still takes transition_size/2.
/// - If an underlay gap between two overlaps is < transition_size, we cannot fit Down then Up,
///   so we skip that underlay and merge the overlaps into one continuous overlap block.
pub fn map_cover(
    overlap: &Vec<(usize, usize)>,
    sample_len: usize,
    transition_size: Option<usize>,
) -> Vec<Cover> {
    if sample_len == 0 {
        return vec![];
    }

    // Clamp + normalize (merge overlaps/touching)
    let overlaps = normalize_ranges(
        overlap
            .iter()
            .map(|&(s, e)| (s.min(sample_len), e.min(sample_len)))
            .collect(),
    );

    // No transition => simple interleave underlay/overlap
    let Some(t) = transition_size else {
        if overlaps.is_empty() {
            return vec![Cover::Underlay((0, sample_len))];
        }
        let mut out = Vec::new();
        let mut cur = 0usize;
        for (s, e) in overlaps {
            push_cover(&mut out, Cover::Underlay((cur, s)));
            push_cover(&mut out, Cover::Overlap((s, e)));
            cur = e;
        }
        push_cover(&mut out, Cover::Underlay((cur, sample_len)));
        return out;
    };

    assert!(t % 2 == 0, "transition_size must be divisible by 2");
    let half = t / 2;

    if overlaps.is_empty() {
        return vec![Cover::Underlay((0, sample_len))];
    }

    // 1) Drop overlaps that cannot support the required transitions.
    //    - internal overlap: needs both Up & Down => len >= t
    //    - edge overlap: needs only one => len >= half
    let mut viable: Vec<(usize, usize)> = Vec::new();
    for (s, e) in overlaps {
        let len = e - s;
        let needs_up = s > 0;
        let needs_down = e < sample_len;

        let ok = if needs_up && needs_down {
            len >= t
        } else {
            // touches an edge, only one transition needed (or none if it covers full range)
            if !needs_up && !needs_down {
                true // covers whole sample
            } else {
                len >= half
            }
        };

        if ok {
            viable.push((s, e));
        }
    }

    if viable.is_empty() {
        return vec![Cover::Underlay((0, sample_len))];
    }

    // 2) If the underlay gap between consecutive overlaps is too small (< t),
    //    we cannot do Down then Up fully, so we skip that underlay and merge overlaps.
    viable.sort_by_key(|&(s, _)| s);
    let mut merged: Vec<(usize, usize)> = Vec::new();
    let mut cur = viable[0];

    for &(ns, ne) in &viable[1..] {
        let gap = ns.saturating_sub(cur.1);
        if gap < t {
            // skip underlay gap -> treat as continuous overlap
            cur.1 = cur.1.max(ne);
        } else {
            merged.push(cur);
            cur = (ns, ne);
        }
    }
    merged.push(cur);

    // 3) Emit covers with transitions.
    let mut out: Vec<Cover> = Vec::new();
    let mut cursor = 0usize;

    for (s, e) in merged {
        // Special: overlap covers entire range
        if s == 0 && e == sample_len {
            push_cover(&mut out, Cover::Overlap((0, sample_len)));
            return out;
        }

        let needs_up = s > 0;
        let needs_down = e < sample_len;

        // Up transition region
        let (up_start, up_end) = if needs_up {
            // Full half from overlap side: [s, s+half]
            // Underlay side wants [s-half, s], but clamp to 0 for partial-at-edge behavior.
            let start = s.saturating_sub(half);
            let end = s + half; // safe because overlap side is guaranteed by viability
            (start, end)
        } else {
            (s, s) // none
        };

        // Down transition region
        let (down_start, down_end) = if needs_down {
            // Full half from overlap side: [e-half, e]
            // Underlay side wants [e, e+half], but clamp to sample_len for partial-at-edge behavior.
            let start = e - half;
            let end = (e + half).min(sample_len);
            (start, end)
        } else {
            (e, e) // none
        };

        // Underlay before Up transition (or before overlap if no Up)
        let before_overlap_start = if needs_up { up_start } else { s };
        push_cover(&mut out, Cover::Underlay((cursor, before_overlap_start)));

        // Up transition
        if needs_up {
            push_cover(
                &mut out,
                Cover::Transition((TransitionDirection::Up, (up_start, up_end))),
            );
        }

        // Main overlap (may be empty if transitions touch)
        let overlap_start = if needs_up { up_end } else { s };
        let overlap_end = if needs_down { down_start } else { e };
        push_cover(&mut out, Cover::Overlap((overlap_start, overlap_end)));

        // Down transition
        if needs_down {
            push_cover(
                &mut out,
                Cover::Transition((TransitionDirection::Down, (down_start, down_end))),
            );
            cursor = down_end;
        } else {
            cursor = e;
        }
    }

    // Tail underlay
    push_cover(&mut out, Cover::Underlay((cursor, sample_len)));

    // If everything collapsed into underlay or overlap, coalescing already handled adjacency.
    out
}
