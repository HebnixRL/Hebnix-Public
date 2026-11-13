//! Rewrite an UnrealScript function's bytecode in a package, growing it when
//! the new body is longer. Same-size edits in other patchers write bytes in
//! place; this relocates the whole function export (replace_export_payload)
//! and fixes its two size fields.
use super::upk_package::UpkPackage;

/// Offset of ScriptMemorySize in a function export: net index (4), the empty
/// property list's "None" (8), Next (4), then six 4-byte UStruct fields.
const SIZES: usize = 40;
/// First byte of the script: after ScriptMemorySize and ScriptSerialSize.
const SCRIPT: usize = SIZES + 8;

/// A function body being assembled. In memory every object reference takes 8
/// bytes where the file has 4, and ScriptMemorySize counts the memory form,
/// so `object` keeps count.
#[derive(Default, Clone)]
pub struct Script {
    bytes: Vec<u8>,
    refs: usize,
}

impl Script {
    /// Bytes with the number of object references inside them.
    pub fn from_parts(bytes: Vec<u8>, refs: usize) -> Self {
        Self { bytes, refs }
    }

    /// How many object references the bytes hold.
    pub fn refs(&self) -> usize {
        self.refs
    }

    pub fn new() -> Self {
        Self::default()
    }

    pub fn raw(&mut self, bytes: &[u8]) -> &mut Self {
        self.bytes.extend_from_slice(bytes);
        self
    }

    /// An object reference (package export index, negative for imports).
    pub fn object(&mut self, index: i32) -> &mut Self {
        self.bytes.extend_from_slice(&index.to_le_bytes());
        self.refs += 1;
        self
    }

    /// A name: index plus instance, 8 bytes in the file and in memory.
    pub fn name(&mut self, index: i32) -> &mut Self {
        self.bytes.extend_from_slice(&index.to_le_bytes());
        self.bytes.extend_from_slice(&[0; 4]);
        self
    }

    /// The script inside a function export. Its object references are the
    /// difference between memory and file size, 4 bytes each.
    pub fn of_function(payload: &[u8]) -> Result<Self, String> {
        let (mem, serial) = sizes(payload)?;
        Ok(Self { bytes: payload[SCRIPT..SCRIPT + serial].to_vec(), refs: (mem - serial) / 4 })
    }

    /// Add another script's bytes and references after this one's.
    pub fn append(&mut self, other: &Script) -> &mut Self {
        self.bytes.extend_from_slice(&other.bytes);
        self.refs += other.refs;
        self
    }

    /// Cut the last `n` bytes, which must hold no object references.
    pub fn split_off(&mut self, n: usize) -> Vec<u8> {
        self.bytes.split_off(self.bytes.len() - n)
    }

    /// Put `add` before the last `tail` bytes, which stay as they are.
    pub fn insert_before_tail(&mut self, tail: usize, add: &Script) -> &mut Self {
        let end = self.bytes.split_off(self.bytes.len() - tail);
        self.bytes.extend_from_slice(&add.bytes);
        self.bytes.extend_from_slice(&end);
        self.refs += add.refs;
        self
    }

    /// Cut `len` bytes at `start`, which hold `refs` object references.
    pub fn cut(&mut self, start: usize, len: usize, refs: usize) -> &mut Self {
        self.bytes.drain(start..start + len);
        self.refs -= refs;
        self
    }

    /// Overwrite the two bytes at `at` (a jump target) with `value`.
    pub fn set_word(&mut self, at: usize, value: u16) -> &mut Self {
        self.bytes[at..at + 2].copy_from_slice(&value.to_le_bytes());
        self
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    /// Size of the script once loaded, for ScriptMemorySize and for the skip
    /// sizes inside context expressions.
    pub fn mem_size(&self) -> usize {
        self.bytes.len() + 4 * self.refs
    }
}

/// (ScriptMemorySize, ScriptSerialSize) of a function export, checked to fit.
fn sizes(old: &[u8]) -> Result<(usize, usize), String> {
    let read = |at: usize| -> Result<usize, String> {
        let raw = old.get(at..at + 4).ok_or("Function export is too short")?;
        Ok(u32::from_le_bytes(raw.try_into().unwrap()) as usize)
    };
    let (mem, serial) = (read(SIZES)?, read(SIZES + 4)?);
    if SCRIPT.checked_add(serial).is_none_or(|end| end > old.len()) {
        return Err("Function script runs past its export".into());
    }
    if mem < serial {
        return Err("Function memory size is smaller than its script".into());
    }
    Ok((mem, serial))
}

/// `old` (a function export's bytes) with its script replaced by `script`.
/// Everything after the script (flags, friendly name) is kept as it was.
pub fn function_payload(old: &[u8], script: &Script) -> Result<Vec<u8>, String> {
    let (_, serial) = sizes(old)?;
    let end = SCRIPT + serial;
    let mut out = old[..SIZES].to_vec();
    out.extend((script.mem_size() as u32).to_le_bytes());
    out.extend((script.len() as u32).to_le_bytes());
    out.extend(&script.bytes);
    out.extend(&old[end..]);
    Ok(out)
}

/// Replace the script of the function at `path` (e.g. "Ball_TA.SetExplosionFXActor").
pub fn replace_script(upk: &mut UpkPackage, path: &str, script: &Script) -> Result<(), String> {
    let index = upk
        .find_object(path)
        .filter(|index| *index > 0)
        .ok_or_else(|| format!("{path} was not found"))? as usize
        - 1;
    let entry = &upk.exports[index];
    let old = upk.image[entry.serial_offset..entry.serial_offset + entry.serial_size].to_vec();
    let payload = function_payload(&old, script)?;
    upk.replace_export_payload(index, &payload)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Team_TA.Forfeit from a real TAGame.upk, a 38-byte script. The tail
    /// after the script is not included here; the real-package test covers it.
    fn forfeit() -> Vec<u8> {
        "0a 29 01 00 e4 7a 00 00 00 00 00 00 ce 62 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 \
         32 00 00 00 26 00 00 00 \
         07 2f 00 81 2d 01 0f 62 00 00 16 14 2d 01 0f 62 00 00 27 42 00 07 62 00 00 f3 46 00 00 00 00 00 00 17 16 04 0b 4c 00"
            .split_whitespace()
            .map(|b| u8::from_str_radix(b, 16).unwrap())
            .collect()
    }

    #[test]
    fn longer_script_updates_both_sizes_and_keeps_the_tail() {
        let old = forfeit();
        let mut script = Script::new();
        script.raw(&[0x1b]).name(7).raw(&[0x25]).object(-3).raw(&[0x04, 0x0b, 0x4c]);
        let out = function_payload(&old, &script).unwrap();
        assert_eq!(&out[..SIZES], &old[..SIZES]);
        assert_eq!(u32::from_le_bytes(out[SIZES..SIZES + 4].try_into().unwrap()), 1 + 8 + 1 + 8 + 3);
        assert_eq!(u32::from_le_bytes(out[SIZES + 4..SCRIPT].try_into().unwrap()), 1 + 8 + 1 + 4 + 3);
        assert_eq!(&out[SCRIPT..SCRIPT + script.len()], &script.bytes[..]);
        assert_eq!(&out[SCRIPT + script.len()..], &old[SCRIPT + 38..]);
    }

    #[test]
    fn rejects_a_script_size_past_the_export() {
        let mut old = forfeit();
        old[SIZES + 4] = 0xff;
        assert!(function_payload(&old, &Script::new()).is_err());
    }

    /// Grow a function in a copy of the real package and read it back. Run
    /// with HEBNIX_UPK_TEST_SOURCE set to a TAGame.upk.
    #[test]
    #[ignore]
    fn real_package_roundtrip() {
        let source = std::env::var("HEBNIX_UPK_TEST_SOURCE").unwrap();
        let mut upk = UpkPackage::load(std::path::Path::new(&source)).unwrap();
        let path = "Team_TA.Forfeit";
        let index = upk.find_object(path).unwrap() as usize - 1;
        let before: Vec<u8> = {
            let e = &upk.exports[index];
            upk.image[e.serial_offset..e.serial_offset + e.serial_size].to_vec()
        };
        let mut script = Script::new();
        script.raw(&before[SCRIPT..SCRIPT + 38 - 4]).raw(&[0x0b; 40]).raw(&[0x04, 0x0b, 0x4c]);
        replace_script(&mut upk, path, &script).unwrap();
        let out = std::env::temp_dir().join("hebnix_script_patch_test.upk");
        upk.save(&out).unwrap();
        let reloaded = UpkPackage::load(&out).unwrap();
        let e = &reloaded.exports[index];
        let after = &reloaded.image[e.serial_offset..e.serial_offset + e.serial_size];
        assert_eq!(after.len(), before.len() + 40 - 1);
        assert_eq!(&after[SCRIPT..SCRIPT + script.len()], &script.bytes[..]);
        assert_eq!(&after[SCRIPT + script.len()..], &before[SCRIPT + 38..]);
        let _ = std::fs::remove_file(out);
    }
}
