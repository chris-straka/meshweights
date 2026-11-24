use std::path::PathBuf;
use weightforge_core as wf;
use wf::metrics::CtxOpts;

pub struct Args {
    pub pos: Vec<String>,
    pub flags: Vec<(String, Option<String>)>,
}

const VALUED: &[&str] = &[
    "--poses",
    "--voxels",
    "--out",
    "--cols",
    "--method",
    "--source",
    "--candidate",
    "--sheet",
    "--report",
    "--vertex-json",
    "--poses-b",
    "--rows",
    "--label-a",
    "--label-b",
    "--clips",
    "--number-from",
    "--motion",
    "--motion-clips",
    "--kind",
    "--top",
    "--seed",
    "--camera",
    "--metrics",
    "--answers",
    "--key",
];

impl Args {
    pub fn parse(raw: Vec<String>) -> Result<Args, String> {
        let mut pos = Vec::new();
        let mut flags = Vec::new();
        let mut it = raw.into_iter();
        while let Some(a) = it.next() {
            if a.starts_with("--") {
                if VALUED.contains(&a.as_str()) {
                    let v = it.next().ok_or_else(|| format!("{a} needs a value"))?;
                    flags.push((a, Some(v)));
                } else {
                    flags.push((a, None));
                }
            } else {
                pos.push(a);
            }
        }
        Ok(Args { pos, flags })
    }

    pub fn has(&self, f: &str) -> bool {
        self.flags.iter().any(|(k, _)| k == f)
    }

    pub fn get(&self, f: &str) -> Option<&str> {
        self.flags.iter().rev().find(|(k, _)| k == f).and_then(|(_, v)| v.as_deref())
    }

    pub fn many(&self, f: &str) -> Vec<&str> {
        self.flags.iter().filter(|(k, _)| k == f).filter_map(|(_, v)| v.as_deref()).collect()
    }
}

pub fn motion_spec(a: &Args) -> wf::MotionSpec {
    wf::MotionSpec {
        source: a.get("--motion").map(PathBuf::from),
        clips: a.get("--motion-clips").map(|c| c.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()),
    }
}

pub fn parse_kinds(a: &Args) -> Result<Option<Vec<String>>, String> {
    match a.get("--kind") {
        None => Ok(None),
        Some(raw) => {
            let kinds: Vec<String> = raw.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
            if kinds.is_empty() {
                return Err("--kind needs at least one finding kind".into());
            }
            Ok(Some(kinds))
        }
    }
}

pub fn parse_pos_usize(a: &Args, flag: &str, default: usize) -> Result<usize, String> {
    match a.get(flag) {
        None => Ok(default),
        Some(v) => match v.parse::<usize>() {
            Ok(n) if n >= 1 => Ok(n),
            _ => Err(format!("{flag} needs a positive integer")),
        },
    }
}

pub fn parse_range_usize(a: &Args, flag: &str, default: usize, lo: usize, hi: usize) -> Result<usize, String> {
    match a.get(flag) {
        None => Ok(default),
        Some(v) => match v.parse::<usize>() {
            Ok(n) if (lo..=hi).contains(&n) => Ok(n),
            _ => Err(format!("{flag} must be {lo}..{hi}")),
        },
    }
}

pub fn parse_usize(a: &Args, flag: &str, default: usize) -> Result<usize, String> {
    match a.get(flag) {
        None => Ok(default),
        Some(v) => v.parse::<usize>().map_err(|_| format!("{flag} needs an integer")),
    }
}

pub fn parse_u64(a: &Args, flag: &str, default: u64) -> Result<u64, String> {
    match a.get(flag) {
        None => Ok(default),
        Some(v) => v.parse::<u64>().map_err(|_| format!("{flag} needs an integer")),
    }
}

pub fn ctx_opts<'a>(a: &Args, set: Option<&'a wf::poses::PoseSet>) -> Result<CtxOpts<'a>, String> {
    let mut o = CtxOpts { poses: set, clips: !a.has("--no-clips"), ..Default::default() };
    if let Some(v) = a.get("--voxels") {
        o.voxel_res = v.parse().map_err(|_| "--voxels needs an integer".to_string())?;
        if !(16..=1024).contains(&o.voxel_res) {
            return Err("--voxels must be 16..1024".into());
        }
    }
    Ok(o)
}

pub fn load_poses(a: &Args) -> Result<Option<wf::poses::PoseSet>, String> {
    load_poses_from(a, "--poses")
}

pub fn load_poses_from(a: &Args, flag: &str) -> Result<Option<wf::poses::PoseSet>, String> {
    match a.get(flag) {
        Some(p) => {
            let text = std::fs::read_to_string(p).map_err(|e| format!("{p}: {e}"))?;
            wf::poses::parse_pose_set(&text).map(Some)
        }
        None => Ok(None),
    }
}
