pub const HEADER: &str = "orbit.ping-list";
pub const ROW: &str = "orbit.ping-row";
pub const MAX_ROWS: usize = 8;
pub const NAME_BYTES: usize = 24;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PingRow {
    pub name: String,
    pub milliseconds: Option<u32>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PingList {
    pub rows: Vec<PingRow>,
    pub total: usize,
}

pub fn encode_row(index: usize, name: &str, milliseconds: Option<u32>) -> [[f32; 8]; 2] {
    let mut bytes = [0u8; NAME_BYTES];
    let mut length = name.len().min(NAME_BYTES);
    while !name.is_char_boundary(length) {
        length -= 1;
    }
    bytes[..length].copy_from_slice(&name.as_bytes()[..length]);
    std::array::from_fn(|part| {
        let mut values = [0.0; 8];
        values[..4].copy_from_slice(&[
            1.0,
            index as f32,
            part as f32,
            milliseconds
                .filter(|ms| *ms <= 60_000)
                .map_or(-1.0, |ms| ms as f32),
        ]);
        for chunk in 0..4 {
            let at = part * 12 + chunk * 3;
            values[4 + chunk] =
                u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], 0]) as f32;
        }
        values
    })
}

pub fn decode<'a>(cues: impl Iterator<Item = (&'a str, &'a [f32])>) -> Option<PingList> {
    let mut total = None;
    let mut parts = [[None; 2]; MAX_ROWS];
    for (name, values) in cues {
        if name == HEADER {
            let [version, active, count] = values else {
                continue;
            };
            if *version == 1.0 && *active == 1.0 && integer(*count, 65_535).is_some() {
                total = Some(*count as usize);
            }
        } else if name == ROW {
            let [version, index, part, ms, a, b, c, d] = values else {
                continue;
            };
            if *version != 1.0 {
                continue;
            }
            let (Some(index), Some(part)) =
                (integer(*index, MAX_ROWS as u32 - 1), integer(*part, 1))
            else {
                continue;
            };
            let milliseconds = if *ms == -1.0 {
                None
            } else {
                Some(integer(*ms, 60_000)?)
            };
            let mut bytes = [0u8; 12];
            for (index, value) in [a, b, c, d].into_iter().enumerate() {
                let packed = integer(*value, 0x00ff_ffff)?.to_le_bytes();
                bytes[index * 3..index * 3 + 3].copy_from_slice(&packed[..3]);
            }
            parts[index as usize][part as usize] = Some((bytes, milliseconds));
        }
    }
    let total = total?;
    let mut rows = Vec::new();
    for row in parts.into_iter().take(total.min(MAX_ROWS)) {
        let [Some((first, ms)), Some((second, other_ms))] = row else {
            return None;
        };
        if ms != other_ms {
            return None;
        }
        let mut bytes = first.to_vec();
        bytes.extend_from_slice(&second);
        let end = bytes
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(bytes.len());
        let name = String::from_utf8(bytes[..end].to_vec()).ok()?;
        rows.push(PingRow {
            name,
            milliseconds: ms,
        });
    }
    Some(PingList { rows, total })
}

fn integer(value: f32, max: u32) -> Option<u32> {
    (value.is_finite() && value >= 0.0 && value <= max as f32 && value.fract() == 0.0)
        .then_some(value as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_known_or_missing_latency_round_trip() {
        for milliseconds in [None, Some(0), Some(87)] {
            let row = encode_row(0, "Player 雪", milliseconds);
            let header = [1.0, 1.0, 1.0];
            let list = decode(
                [
                    (HEADER, header.as_slice()),
                    (ROW, row[0].as_slice()),
                    (ROW, row[1].as_slice()),
                ]
                .into_iter(),
            )
            .unwrap();
            assert_eq!(
                list.rows,
                vec![PingRow {
                    name: "Player 雪".into(),
                    milliseconds
                }]
            );
        }
    }

    #[test]
    fn stale_incomplete_and_invalid_snapshots_are_unavailable() {
        assert_eq!(decode(std::iter::empty()), None);
        let header = [1.0, 1.0, 1.0];
        let row = encode_row(0, "Player", None);
        assert_eq!(
            decode([(HEADER, header.as_slice()), (ROW, row[0].as_slice())].into_iter()),
            None
        );
        assert_eq!(integer(f32::NAN, 100), None);
        assert_eq!(integer(1.5, 100), None);
        let long = encode_row(0, &"雪".repeat(20), None);
        let list = decode(
            [
                (HEADER, header.as_slice()),
                (ROW, long[0].as_slice()),
                (ROW, long[1].as_slice()),
            ]
            .into_iter(),
        )
        .unwrap();
        assert_eq!(list.rows[0].name, "雪".repeat(8));
    }
}
