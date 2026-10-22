//! Bound canonical JSON serialization before constructing a duplicate payload.
use serde_json::Value;
use std::io::{self, Write};

struct Bounded {
    bytes: Vec<u8>,
    maximum: usize,
}
impl Write for Bounded {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.maximum.saturating_sub(self.bytes.len()) {
            return Err(io::Error::other("governed-json-limit"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(super) fn canonical(value: &Value, maximum: usize) -> Result<Vec<u8>, String> {
    let mut output = Bounded {
        bytes: Vec::new(),
        maximum,
    };
    serde_json::to_writer(&mut output, value).map_err(|_| "governed-input-limit-or-invalid")?;
    Ok(output.bytes)
}
