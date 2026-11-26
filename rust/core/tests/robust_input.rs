//! Core robust-input tests for `Glb::parse`, `Model::from_glb`, `Model::write_weights`,
//! and `paint::read` on malformed, truncated, non-finite, and empty inputs.

use serde_json::{Value, json};
use weightforge_core::glb::{Glb, f32_bytes};
use weightforge_core::paint;
use weightforge_core::scene::Model;

fn pack_glb(json_bytes: &[u8], bin_bytes: &[u8]) -> Vec<u8> {
    let mut jb = json_bytes.to_vec();
    while !jb.len().is_multiple_of(4) {
        jb.push(b' ');
    }
    let mut bb = bin_bytes.to_vec();
    while !bb.len().is_multiple_of(4) {
        bb.push(0);
    }
    let total = 12 + 8 + jb.len() + 8 + bb.len();
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(jb.len() as u32).to_le_bytes());
    out.extend_from_slice(&0x4E4F_534Au32.to_le_bytes());
    out.extend_from_slice(&jb);
    out.extend_from_slice(&(bb.len() as u32).to_le_bytes());
    out.extend_from_slice(&0x004E_4942u32.to_le_bytes());
    out.extend_from_slice(&bb);
    out
}

struct MinimalSpec {
    node0: Value,
    node1: Value,
    ibm: Vec<f32>,
    positions: Vec<f32>,
    weights: [[f32; 4]; 3],
    indices: Vec<u16>,
    skins: Value,
}

impl Default for MinimalSpec {
    fn default() -> Self {
        let ident = [1.0f32, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0];
        let mut ibm = Vec::with_capacity(32);
        ibm.extend_from_slice(&ident);
        ibm.extend_from_slice(&ident);
        Self {
            node0: json!({"name": "root", "translation": [0.0, 0.0, 0.0], "children": [1]}),
            node1: json!({"name": "tip", "translation": [0.0, 1.0, 0.0]}),
            ibm,
            positions: vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0],
            weights: [[1.0, 0.0, 0.0, 0.0]; 3],
            indices: vec![0, 1, 2],
            skins: json!([{"joints": [0, 1], "inverseBindMatrices": 0}]),
        }
    }
}

fn build_glb_bytes(spec: &MinimalSpec) -> Vec<u8> {
    let mut bin = Vec::new();
    let mut add_view = |data: &[u8], target: Option<u32>| -> Value {
        while !bin.len().is_multiple_of(4) {
            bin.push(0);
        }
        let off = bin.len();
        bin.extend_from_slice(data);
        let mut bv = json!({"buffer": 0, "byteOffset": off, "byteLength": data.len()});
        if let Some(t) = target {
            bv["target"] = json!(t);
        }
        bv
    };

    let ibm_bytes = f32_bytes(&spec.ibm);
    let pos_bytes = f32_bytes(&spec.positions);
    let jnt_bytes = [0u8; 12];
    let wgt_flat: Vec<f32> = spec.weights.iter().flatten().copied().collect();
    let wgt_bytes = f32_bytes(&wgt_flat);
    let idx_bytes: Vec<u8> =
        if spec.indices.is_empty() { vec![0u8; 6] } else { spec.indices.iter().flat_map(|i| i.to_le_bytes()).collect() };

    let bvs = vec![
        add_view(&ibm_bytes, None),
        add_view(&pos_bytes, Some(34962)),
        add_view(&jnt_bytes, Some(34962)),
        add_view(&wgt_bytes, Some(34962)),
        add_view(&idx_bytes, Some(34963)),
    ];
    let accessors = json!([
        {"bufferView": 0, "componentType": 5126, "count": 2, "type": "MAT4"},
        {"bufferView": 1, "componentType": 5126, "count": 3, "type": "VEC3", "min": [0.0, 0.0, 0.0], "max": [1.0, 1.0, 0.0]},
        {"bufferView": 2, "componentType": 5121, "count": 3, "type": "VEC4"},
        {"bufferView": 3, "componentType": 5126, "count": 3, "type": "VEC4"},
        {"bufferView": 4, "componentType": 5123, "count": spec.indices.len(), "type": "SCALAR"},
    ]);
    let doc = json!({
        "asset": {"version": "2.0"},
        "scene": 0,
        "scenes": [{"nodes": [0, 2]}],
        "nodes": [spec.node0, spec.node1, {"name": "MeshNode", "mesh": 0, "skin": 0}],
        "skins": spec.skins,
        "meshes": [{
            "name": "Tri",
            "primitives": [{
                "attributes": {"POSITION": 1, "JOINTS_0": 2, "WEIGHTS_0": 3},
                "indices": 4,
                "mode": 4
            }]
        }],
        "buffers": [{"byteLength": bin.len()}],
        "bufferViews": bvs,
        "accessors": accessors
    });
    let js = serde_json::to_vec(&doc).unwrap();
    pack_glb(&js, &bin)
}

#[test]
fn minimal_valid_glb_loads_cleanly() {
    let bytes = build_glb_bytes(&MinimalSpec::default());
    let glb = Glb::parse(&bytes).unwrap();
    let model = Model::from_glb(glb).unwrap();
    assert_eq!(model.nverts(), 3);
    assert_eq!(model.tris.len(), 1);
    assert_eq!(model.njoints(), 2);
    assert_eq!(model.bad_weights, 0);
}

#[test]
fn truncated_glbs_are_rejected() {
    let valid = build_glb_bytes(&MinimalSpec::default());
    let jl = u32::from_le_bytes(valid[12..16].try_into().unwrap()) as usize;
    let bin_start = 20 + jl + 8;

    assert!(Glb::parse(&[]).is_err(), "empty payload must fail");
    assert!(Glb::parse(&valid[..8]).is_err(), "short header must fail");
    assert!(Glb::parse(&valid[..40]).is_err(), "truncated JSON chunk must fail");
    assert!(Glb::parse(&valid[..bin_start + 4]).is_err(), "truncated BIN chunk must fail");
}

#[test]
fn malformed_and_overflowing_node_transforms_are_rejected() {
    // 1. Literal JSON NaN and Infinity in node translation.
    let valid = build_glb_bytes(&MinimalSpec::default());
    let jl = u32::from_le_bytes(valid[12..16].try_into().unwrap()) as usize;
    let bin_start = 20 + jl + 8;
    let json_str = std::str::from_utf8(&valid[20..20 + jl]).unwrap();
    let bin = &valid[bin_start..];
    for literal in ["NaN", "Infinity"] {
        let patched = json_str.replacen("[0.0,1.0,0.0]", &format!("[{literal},0.0,0.0]"), 1);
        assert_ne!(patched, json_str, "expected translation placeholder to match");
        let raw = pack_glb(patched.as_bytes(), bin);
        assert!(Glb::parse(&raw).is_err(), "literal JSON {literal} must fail to parse");
    }

    // 2. Non-numeric JSON elements and wrong lengths in translation/scale/rotation/matrix.
    let bad_nodes = [
        json!({"name": "tip", "translation": ["NaN", 0.0, 0.0]}),
        json!({"name": "tip", "translation": [null, 0.0, 0.0]}),
        json!({"name": "tip", "translation": "bad"}),
        json!({"name": "tip", "translation": [0.0, 1.0]}),
        json!({"name": "tip", "scale": [1.0, "bad", 1.0]}),
        json!({"name": "tip", "rotation": [0.0, 0.0, 1.0]}),
        json!({"name": "tip", "matrix": [1.0, 0.0, 0.0]}),
    ];
    for node1 in bad_nodes {
        let spec = MinimalSpec { node1: node1.clone(), ..Default::default() };
        let glb = Glb::parse(&build_glb_bytes(&spec)).unwrap();
        let err = Model::from_glb(glb).err().unwrap_or_else(|| panic!("expected error for node1={node1}"));
        let msg = err.to_string();
        assert!(msg.contains("node 1") && msg.contains("tip"), "error should name node index and name: {msg}");
    }

    // 3. Chained [1e308, 0, 0] overflowing global transform to Inf.
    let spec_global = MinimalSpec {
        node0: json!({"name": "root", "translation": [1e308, 0.0, 0.0], "children": [1]}),
        node1: json!({"name": "tip", "translation": [1e308, 0.0, 0.0]}),
        ..Default::default()
    };
    let glb = Glb::parse(&build_glb_bytes(&spec_global)).unwrap();
    assert!(Model::from_glb(glb).is_err(), "chained 1e308 translation overflow must fail");

    // 4. Child joint at [1e200, 0, 0] overflowing bone length and tail to Inf/NaN.
    let spec_tail = MinimalSpec { node1: json!({"name": "tip", "translation": [1e200, 0.0, 0.0]}), ..Default::default() };
    let glb = Glb::parse(&build_glb_bytes(&spec_tail)).unwrap();
    let err = Model::from_glb(glb).err().expect("child joint at 1e200 must fail");
    assert!(err.to_string().contains("joint"), "tail overflow error should name joint: {err}");
}

#[test]
fn non_finite_positions_and_inverse_bind_matrices_are_rejected() {
    for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let mut spec_pos = MinimalSpec::default();
        spec_pos.positions[3] = bad;
        let glb = Glb::parse(&build_glb_bytes(&spec_pos)).unwrap();
        assert!(Model::from_glb(glb).is_err(), "position with {bad} must fail");

        let mut spec_ibm = MinimalSpec::default();
        spec_ibm.ibm[12] = bad;
        let glb = Glb::parse(&build_glb_bytes(&spec_ibm)).unwrap();
        assert!(Model::from_glb(glb).is_err(), "inverse bind matrix with {bad} must fail");
    }
}

#[test]
fn non_finite_skin_weights_flag_bad_weights_and_do_not_leak_on_write() {
    for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        for row in [[bad, 0.0, 0.0, 0.0], [1.0, bad, 0.0, 0.0]] {
            let mut spec = MinimalSpec::default();
            spec.weights[0] = row;
            let glb = Glb::parse(&build_glb_bytes(&spec)).unwrap();
            let model = Model::from_glb(glb).unwrap();
            assert_eq!(model.bad_weights, 1, "expected bad_weights=1 for row={row:?}");
            assert!(model.weights[0].is_empty(), "vertex 0 with bad weight {row:?} must be unweighted");

            // write_weights must never copy non-finite raw rows verbatim.
            let written = model.write_weights(&model.weights).unwrap();
            let reparsed = Model::from_glb(Glb::parse(&written.try_to_bytes().unwrap()).unwrap()).unwrap();
            assert_eq!(reparsed.bad_weights, 0, "written GLB must not contain non-finite weights for row={row:?}");
            let wa = reparsed.prims[0].sets[0].1;
            let (_, vals) = reparsed.glb.read_f64(wa).unwrap();
            assert!(vals.iter().all(|x| x.is_finite()), "written WEIGHTS_0 must be finite for row={row:?}");
        }
    }
}

#[test]
fn zero_joints_are_rejected() {
    for skins in [json!([]), json!([{"joints": []}])] {
        let spec = MinimalSpec { skins: skins.clone(), ..Default::default() };
        let glb = Glb::parse(&build_glb_bytes(&spec)).unwrap();
        assert!(Model::from_glb(glb).is_err(), "zero joints ({skins}) must fail");
    }
}

#[test]
fn zero_triangles_are_rejected() {
    for indices in [vec![], vec![0, 1], vec![0, 0, 1]] {
        let spec = MinimalSpec { indices: indices.clone(), ..Default::default() };
        let glb = Glb::parse(&build_glb_bytes(&spec)).unwrap();
        let err = Model::from_glb(glb).err().unwrap_or_else(|| panic!("zero triangles ({indices:?}) must fail"));
        assert!(err.to_string().contains("triangles"), "error should mention triangles: {err}");
    }
}

#[test]
fn paint_handles_overflowing_image_buffer_view() {
    let mut glb = Glb::parse(&build_glb_bytes(&MinimalSpec::default())).unwrap();
    glb.json["bufferViews"].as_array_mut().unwrap().push(json!({"buffer": 0, "byteOffset": u64::MAX, "byteLength": 16}));
    let bv = glb.json["bufferViews"].as_array().unwrap().len() - 1;
    glb.json["images"] = json!([{"bufferView": bv, "mimeType": "image/png"}]);
    glb.json["textures"] = json!([{"source": 0}]);
    glb.json["materials"] = json!([{"pbrMetallicRoughness": {"baseColorTexture": {"index": 0}}}]);
    glb.json["meshes"][0]["primitives"][0]["material"] = json!(0);
    let model = Model::from_glb(glb).unwrap();
    let p = paint::read(&model);
    assert!(!p.textured);
}
