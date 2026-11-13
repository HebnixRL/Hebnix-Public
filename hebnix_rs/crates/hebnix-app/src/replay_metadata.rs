use serde_json::{Map, Value};
use std::{fs::File, io::Read, path::Path};

#[derive(Clone, Debug)]
pub struct ReplayMetadata {
    pub frames: i64,
    pub fps: f64,
    pub title: String,
    pub goals: Vec<(i64, String, i64)>,
    pub ids: Vec<String>,
    pub match_guid: Option<String>,
}

// Read the bounded Unreal property header only; never deserialize the network stream.
pub fn read(path: &Path) -> Result<ReplayMetadata, String> {
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    let mut prefix = [0; 8];
    file.read_exact(&mut prefix).map_err(|e| e.to_string())?;
    let size = u32::from_le_bytes(prefix[..4].try_into().unwrap()) as usize;
    if !(16..=4 * 1024 * 1024).contains(&size) {
        return Err("Invalid replay header size".into());
    }
    let mut bytes = vec![0; size];
    file.read_exact(&mut bytes).map_err(|e| e.to_string())?;
    let mut r = Reader {
        bytes: &bytes,
        pos: 0,
    };
    let major = r.int()?;
    let minor = r.int()?;
    if major > 865 && minor > 17 {
        r.int()?;
    }
    if !r.text()?.starts_with("TAGame.Replay") {
        return Err("Not a Rocket League replay".into());
    }
    let props = r.properties(0)?;
    let frames = props.get("NumFrames").and_then(Value::as_i64).unwrap_or(0);
    let fps = props
        .get("RecordFPS")
        .and_then(Value::as_f64)
        .unwrap_or(30.0);
    if frames < 2 || frames > 10_000_000 || !fps.is_finite() || !(1.0..=240.0).contains(&fps) {
        return Err("Replay has invalid frame metadata".into());
    }
    let title = props
        .get("ReplayName")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .or_else(|| props.get("Date").and_then(Value::as_str))
        .unwrap_or_else(|| {
            path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("Replay")
        })
        .to_owned();
    let goals = props
        .get("Goals")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|g| {
            Some((
                g.get("frame")?.as_i64()?,
                g.get("PlayerName")?.as_str()?.to_owned(),
                g.get("PlayerTeam")?.as_i64()?,
            ))
        })
        .collect();
    let ids = ["MatchGUID", "MatchGuid", "Id"]
        .iter()
        .filter_map(|key| props.get(*key).and_then(Value::as_str).map(str::to_owned))
        .collect();
    Ok(ReplayMetadata {
        frames: frames - 1,
        fps,
        title,
        goals,
        ids,
        match_guid: ["MatchGUID", "MatchGuid"]
            .iter()
            .find_map(|key| props.get(*key).and_then(Value::as_str))
            .map(str::to_owned),
    })
}

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}
impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], String> {
        let end = self.pos.checked_add(n).ok_or("Replay header overflow")?;
        let data = self
            .bytes
            .get(self.pos..end)
            .ok_or("Truncated replay header")?;
        self.pos = end;
        Ok(data)
    }
    fn int(&mut self) -> Result<i32, String> {
        Ok(i32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn text(&mut self) -> Result<String, String> {
        let n = self.int()?;
        if n.unsigned_abs() > 1_048_576 {
            return Err("Replay string too long".into());
        }
        if n < 0 {
            let data = self.take(n.unsigned_abs() as usize * 2)?;
            Ok(String::from_utf16_lossy(
                &data
                    .chunks_exact(2)
                    .map(|v| u16::from_le_bytes([v[0], v[1]]))
                    .collect::<Vec<_>>(),
            )
            .trim_end_matches('\0')
            .into())
        } else {
            Ok(String::from_utf8_lossy(self.take(n as usize)?)
                .trim_end_matches('\0')
                .into())
        }
    }
    fn properties(&mut self, depth: usize) -> Result<Map<String, Value>, String> {
        if depth > 16 {
            return Err("Replay header nesting too deep".into());
        }
        let mut out = Map::new();
        for _ in 0..10000 {
            let name = self.text()?;
            if name == "None" {
                return Ok(out);
            }
            let kind = self.text()?;
            self.int()?;
            self.int()?;
            let value = match kind.as_str() {
                "IntProperty" => Value::from(self.int()?),
                "FloatProperty" => {
                    Value::from(f32::from_le_bytes(self.take(4)?.try_into().unwrap()) as f64)
                }
                "QWordProperty" => {
                    Value::from(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
                }
                "BoolProperty" => Value::from(self.take(1)?[0] != 0),
                "NameProperty" | "StrProperty" => Value::from(self.text()?),
                "ByteProperty" => {
                    if self.text()? == "None" {
                        Value::from(self.take(1)?[0])
                    } else {
                        Value::from(self.text()?)
                    }
                }
                "StructProperty" => {
                    self.text()?;
                    Value::Object(self.properties(depth + 1)?)
                }
                "ArrayProperty" => {
                    let count = self.int()?;
                    if !(0..=10000).contains(&count) {
                        return Err("Replay array too large".into());
                    }
                    let mut values = Vec::new();
                    for _ in 0..count {
                        values.push(Value::Object(self.properties(depth + 1)?));
                    }
                    Value::Array(values)
                }
                _ => return Err(format!("Unsupported replay header property: {kind}")),
            };
            out.insert(name, value);
        }
        Err("Too many replay properties".into())
    }
}
