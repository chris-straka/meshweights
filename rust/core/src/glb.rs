//! GLB container + accessor IO on raw JSON, so unknown fields, extensions,
//! materials, and animations survive a round trip untouched. Writing only
//! ever replaces skinning attribute data (weightforge never moves vertices).

use glbkit::accessor::{Component, Desc, Layout, LayoutError, View, append_aligned};
use serde_json::{Value, json};
use std::fmt;

#[derive(Debug)]
pub struct Error(pub String);
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for Error {}
pub type Result<T> = std::result::Result<T, Error>;

pub fn err<T>(msg: impl Into<String>) -> Result<T> {
    Err(Error(msg.into()))
}

pub struct Glb {
    pub json: Value,
    pub bin: Vec<u8>,
    /// The JSON chunk as read, with its parse, so an unchanged JSON is
    /// written back verbatim (exporters format floats their own way).
    pub json_raw: Option<(Vec<u8>, Value)>,
}

impl Glb {
    pub fn read_path(path: &std::path::Path) -> Result<Glb> {
        let bytes = std::fs::read(path).map_err(|e| Error(format!("{}: {e}", path.display())))?;
        Glb::parse(&bytes).map_err(|e| Error(format!("{}: {e}", path.display())))
    }

    pub fn parse(b: &[u8]) -> Result<Glb> {
        let c = glbkit::container::parse(b).map_err(|e| Error(format!("GLB: {e}")))?;
        let json: Value = serde_json::from_slice(c.json).map_err(|e| Error(format!("GLB JSON chunk: {e}")))?;
        Ok(Glb { json_raw: Some((c.json.to_vec(), json.clone())), json, bin: c.bin.to_vec() })
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let js = match &self.json_raw {
            Some((raw, parsed)) if *parsed == self.json => raw.clone(),
            _ => serde_json::to_vec(&self.json).expect("json"),
        };
        glbkit::container::write(&js, &self.bin).expect("GLB under 4 GiB")
    }

    pub fn arr(&self, key: &str) -> &[Value] {
        self.json.get(key).and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[])
    }

    fn accessor_layout(&self, idx: usize) -> Result<Layout> {
        let acc = self.arr("accessors").get(idx).ok_or_else(|| Error(format!("accessor {idx} missing")))?;
        if acc.get("sparse").is_some() {
            return err(format!("accessor {idx} is sparse (unsupported)"));
        }
        let num = |v: &Value, k: &str| v.get(k).and_then(Value::as_u64).map(|n| n as usize);
        let bv_idx = num(acc, "bufferView");
        let view = match bv_idx {
            Some(i) => {
                let bv = self.arr("bufferViews").get(i).ok_or_else(|| Error(format!("bufferView {i} missing")))?;
                Some(View {
                    buffer: num(bv, "buffer").unwrap_or(0),
                    byte_offset: num(bv, "byteOffset").unwrap_or(0),
                    byte_length: num(bv, "byteLength").unwrap_or(0),
                    byte_stride: num(bv, "byteStride"),
                })
            }
            None => None,
        };
        let desc = Desc {
            count: num(acc, "count").unwrap_or(0),
            component_type: acc.get("componentType").and_then(Value::as_u64).unwrap_or(0),
            kind: acc.get("type").and_then(Value::as_str).unwrap_or(""),
            normalized: acc.get("normalized").and_then(Value::as_bool).unwrap_or(false),
            byte_offset: num(acc, "byteOffset").unwrap_or(0),
            view,
        };
        let layout = Layout::resolve(&desc, self.bin.len()).map_err(|e| match e {
            LayoutError::ExternalBuffer(_) => Error("only the GLB-embedded buffer 0 is supported".into()),
            LayoutError::BadComponent(c) => Error(format!("accessor {idx}: bad componentType {c}")),
            LayoutError::BadType(t) => Error(format!("accessor {idx}: unsupported type {t}")),
            _ => Error(format!("accessor {idx} exceeds its buffer ({e})")),
        })?;
        Ok(match bv_idx {
            Some(i) => layout.with_view(i),
            None => layout,
        })
    }

    /// Reads any numeric accessor as f64 rows of `ncomp` values
    /// (normalized integers are mapped to [0,1] / [-1,1]).
    pub fn read_f64(&self, idx: usize) -> Result<(usize, Vec<f64>)> {
        let l = self.accessor_layout(idx)?;
        let mut out = Vec::with_capacity(l.count * l.width);
        for i in 0..l.count {
            for c in 0..l.width {
                out.push(l.read_f64(&self.bin, i, c).ok_or_else(|| Error(format!("accessor {idx} exceeds its buffer")))?);
            }
        }
        Ok((l.width, out))
    }

    pub fn read_vec3(&self, idx: usize) -> Result<Vec<[f32; 3]>> {
        let (n, v) = self.read_f64(idx)?;
        if n != 3 {
            return err(format!("accessor {idx} is not VEC3"));
        }
        Ok(v.chunks(3).map(|c| [c[0] as f32, c[1] as f32, c[2] as f32]).collect())
    }

    pub fn read_indices(&self, idx: usize) -> Result<Vec<u32>> {
        let (n, v) = self.read_f64(idx)?;
        if n != 1 {
            return err(format!("index accessor {idx} is not SCALAR"));
        }
        Ok(v.into_iter().map(|x| x as u32).collect())
    }

    /// Overwrites skinning data of a VEC4 JOINTS/WEIGHTS accessor in place.
    /// Returns false, leaving the BIN untouched, when the accessor's storage
    /// cannot hold the values (component type too small, zero-filled); the
    /// caller then appends a fresh accessor instead.
    pub fn overwrite_vec4(&mut self, idx: usize, rows: &[[f64; 4]]) -> Result<bool> {
        let l = self.accessor_layout(idx)?;
        if l.zero || l.width != 4 || l.count != rows.len() || !matches!(l.component, Component::U8 | Component::U16 | Component::F32) {
            return Ok(false);
        }
        // Dry run on a scratch copy of one element first: a refused value
        // must not leave a half-written accessor behind.
        let mut probe = [0u8; 16];
        let probe_layout = Layout { base: 0, stride: 0, count: 1, view: None, ..l };
        if rows.iter().flatten().any(|&v| !probe_layout.write_f64(&mut probe, 0, 0, v)) {
            return Ok(false);
        }
        for (i, row) in rows.iter().enumerate() {
            for (c, &v) in row.iter().enumerate() {
                if !l.write_f64(&mut self.bin, i, c, v) {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }

    pub fn component_type(&self, idx: usize) -> Option<(u32, bool)> {
        let a = self.arr("accessors").get(idx)?;
        Some((a.get("componentType")?.as_u64()? as u32, a.get("normalized").and_then(Value::as_bool).unwrap_or(false)))
    }

    fn check_mut_schema(&self, need_views: bool, need_accs: bool) -> Result<()> {
        let obj = self.json.as_object().ok_or_else(|| Error("GLB JSON root is not an object".into()))?;
        if let Some(bufs) = obj.get("buffers") {
            let arr = bufs.as_array().ok_or_else(|| Error("GLB JSON buffers is not an array".into()))?;
            if let Some(b0) = arr.first() {
                if !b0.is_object() {
                    return err("GLB JSON buffers[0] is not an object");
                }
            }
        }
        if need_views && obj.get("bufferViews").is_some_and(|v| !v.is_array()) {
            return err("GLB JSON bufferViews is not an array");
        }
        if need_accs && obj.get("accessors").is_some_and(|v| !v.is_array()) {
            return err("GLB JSON accessors is not an array");
        }
        Ok(())
    }

    /// Appends a tightly packed accessor and returns its index.
    /// `ctype` 5123 (u16) or 5126 (f32).
    pub fn append_vec4(&mut self, rows: &[[f64; 4]], ctype: u32) -> Result<usize> {
        self.check_mut_schema(true, true)?;
        let mut bytes = Vec::with_capacity(rows.len() * 16);
        for row in rows {
            for &v in row {
                match ctype {
                    5123 => bytes.extend_from_slice(&(v.round().clamp(0.0, 65535.0) as u16).to_le_bytes()),
                    _ => bytes.extend_from_slice(&(v as f32).to_le_bytes()),
                }
            }
        }
        let off = append_aligned(&mut self.bin, &bytes);
        let len = bytes.len();
        let obj = self.json.as_object_mut().ok_or_else(|| Error("GLB JSON root is not an object".into()))?;
        let bvs = obj
            .entry("bufferViews")
            .or_insert_with(|| json!([]))
            .as_array_mut()
            .ok_or_else(|| Error("GLB JSON bufferViews is not an array".into()))?;
        bvs.push(json!({"buffer": 0, "byteOffset": off, "byteLength": len}));
        let bv = bvs.len() - 1;
        let accs = obj
            .entry("accessors")
            .or_insert_with(|| json!([]))
            .as_array_mut()
            .ok_or_else(|| Error("GLB JSON accessors is not an array".into()))?;
        accs.push(json!({"bufferView": bv, "componentType": ctype, "count": rows.len(), "type": "VEC4"}));
        let a = accs.len() - 1;
        self.sync_buffer_len()?;
        Ok(a)
    }

    pub fn sync_buffer_len(&mut self) -> Result<()> {
        let len = self.bin.len();
        let obj = self.json.as_object_mut().ok_or_else(|| Error("GLB JSON root is not an object".into()))?;
        let bufs = obj
            .entry("buffers")
            .or_insert_with(|| json!([{}]))
            .as_array_mut()
            .ok_or_else(|| Error("GLB JSON buffers is not an array".into()))?;
        if bufs.is_empty() {
            bufs.push(json!({}));
        }
        let buf0 = bufs[0].as_object_mut().ok_or_else(|| Error("GLB JSON buffers[0] is not an object".into()))?;
        buf0.insert("byteLength".into(), json!(len));
        Ok(())
    }

    /// Appends raw bytes as a bufferView (4-byte aligned) and returns its index.
    pub fn append_view(&mut self, bytes: &[u8], target: Option<u32>) -> Result<usize> {
        self.check_mut_schema(true, false)?;
        let off = append_aligned(&mut self.bin, bytes);
        let obj = self.json.as_object_mut().ok_or_else(|| Error("GLB JSON root is not an object".into()))?;
        let bvs = obj
            .entry("bufferViews")
            .or_insert_with(|| json!([]))
            .as_array_mut()
            .ok_or_else(|| Error("GLB JSON bufferViews is not an array".into()))?;
        let mut bv = json!({"buffer": 0, "byteOffset": off, "byteLength": bytes.len()});
        if let Some(t) = target {
            bv["target"] = json!(t);
        }
        bvs.push(bv);
        let i = bvs.len() - 1;
        self.sync_buffer_len()?;
        Ok(i)
    }

    pub fn push_accessor(&mut self, acc: Value) -> Result<usize> {
        let obj = self.json.as_object_mut().ok_or_else(|| Error("GLB JSON root is not an object".into()))?;
        let accs = obj
            .entry("accessors")
            .or_insert_with(|| json!([]))
            .as_array_mut()
            .ok_or_else(|| Error("GLB JSON accessors is not an array".into()))?;
        accs.push(acc);
        Ok(accs.len() - 1)
    }
}

pub fn f32_bytes(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_json_structures_return_errors_instead_of_panicking() {
        for bad in [
            json!(123),
            json!({"bufferViews": "not-an-array"}),
            json!({"accessors": {}}),
            json!({"buffers": "not-an-array"}),
            json!({"buffers": [42]}),
        ] {
            let mut g_vec4 = Glb { json: bad.clone(), bin: vec![], json_raw: None };
            assert!(g_vec4.append_vec4(&[[0.0, 0.0, 0.0, 1.0]], 5126).is_err(), "expected error on append_vec4 for {bad}");
            assert!(g_vec4.bin.is_empty(), "append_vec4 leaked bytes into bin for {bad}");

            let mut g_view = Glb { json: bad.clone(), bin: vec![], json_raw: None };
            let r_view = g_view.append_view(&[1, 2, 3, 4], None);
            if r_view.is_err() {
                assert!(g_view.bin.is_empty(), "append_view leaked bytes into bin for {bad}");
            }

            let mut g_sync = Glb { json: bad.clone(), bin: vec![], json_raw: None };
            let _ = g_sync.sync_buffer_len();
            let mut g_acc = Glb { json: bad.clone(), bin: vec![], json_raw: None };
            let _ = g_acc.push_accessor(json!({}));
        }
    }
}
