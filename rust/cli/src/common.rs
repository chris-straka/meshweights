use std::path::{Path, PathBuf};
use std::process::ExitCode;
use weightforge_core as wf;

pub fn fail(msg: impl std::fmt::Display) -> ExitCode {
    eprintln!("weights: {msg}");
    ExitCode::from(2)
}

pub fn fails(r: &wf::report::Report) -> usize {
    r.findings.iter().filter(|f| f.severity == "fail").count()
}

pub fn write_png(out: &Path, img: &wf::render::Image) -> Result<(), String> {
    let bytes = img.try_png_bytes().map_err(|e| format!("{}: {e}", out.display()))?;
    std::fs::write(out, bytes).map_err(|e| format!("{}: {e}", out.display()))
}

pub fn write_json(out: &Path, res: serde_json::Result<Vec<u8>>) -> Result<(), String> {
    let bytes = res.map_err(|e| format!("{}: json: {e}", out.display()))?;
    std::fs::write(out, bytes).map_err(|e| format!("{}: {e}", out.display()))
}

pub fn write_json_pretty(out: &Path, res: serde_json::Result<String>) -> Result<(), String> {
    let text = res.map_err(|e| format!("{}: json: {e}", out.display()))?;
    std::fs::write(out, text).map_err(|e| format!("{}: {e}", out.display()))
}

/// Writes a batch of output files; if any write fails, removes every file
/// created by earlier entries so no half-written outputs remain on disk.
pub fn write_files_clean(files: &[(&Path, &[u8])]) -> Result<(), String> {
    let mut created: Vec<&Path> = Vec::with_capacity(files.len());
    for &(path, bytes) in files {
        let existed = path.exists();
        if let Err(e) = std::fs::write(path, bytes) {
            if !existed {
                let _ = std::fs::remove_file(path);
            }
            for p in created.into_iter().rev() {
                let _ = std::fs::remove_file(p);
            }
            return Err(format!("{}: {e}", path.display()));
        }
        if !existed {
            created.push(path);
        }
    }
    Ok(())
}

pub fn print_json_pretty(res: serde_json::Result<String>) -> Result<(), String> {
    let text = res.map_err(|e| format!("json: {e}"))?;
    println!("{text}");
    Ok(())
}

pub fn ffmpeg(args: &[&str]) -> bool {
    std::process::Command::new("ffmpeg").args(["-y", "-loglevel", "error"]).args(args).status().map(|s| s.success()).unwrap_or(false)
}

/// A directory of NNN.png frames into an mp4; the frames are removed once
/// it is written. False when ffmpeg is missing or fails.
pub fn encode_frames(frames: &Path, mp4: &Path) -> bool {
    let fps = wf::render::CLIP_FPS.to_string();
    let pattern = frames.join("%03d.png");
    let ok = ffmpeg(&[
        "-framerate",
        &fps,
        "-i",
        &pattern.to_string_lossy(),
        "-c:v",
        "libx264",
        "-pix_fmt",
        "yuv420p",
        "-crf",
        "18",
        "-movflags",
        "+faststart",
        &mp4.to_string_lossy(),
    ]);
    if ok {
        let _ = std::fs::remove_dir_all(frames);
    }
    ok
}

pub fn winner_of(x: &str, y: &str, vx: f64, vy: f64, eps: f64, lower_wins: bool) -> String {
    if (vx - vy).abs() <= eps {
        "tie".into()
    } else if (vx < vy) == lower_wins {
        x.into()
    } else {
        y.into()
    }
}

/// Locate the `gk` binary via `GK_BIN`, `PATH`, `GAMES_TOOLS_DIR`, or `~/.cargo/bin/gk`.
pub fn find_gk() -> Result<PathBuf, String> {
    if let Some(p) = std::env::var_os("GK_BIN")
        && !p.is_empty()
    {
        let pb = PathBuf::from(&p);
        if pb.is_file() {
            return Ok(pb);
        }
        return Err(format!("gk not found at GK_BIN={}", pb.display()));
    }
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            let cand = dir.join("gk");
            if cand.is_file() {
                return Ok(cand);
            }
        }
    }
    let mut fallbacks: Vec<PathBuf> = Vec::new();
    if let Some(td) = std::env::var_os("GAMES_TOOLS_DIR").filter(|s| !s.is_empty()) {
        fallbacks.push(PathBuf::from(&td).join("gk/target/release/gk"));
        fallbacks.push(PathBuf::from(&td).join("gk/target/debug/gk"));
    }
    if let Some(home) = std::env::var_os("HOME") {
        fallbacks.push(PathBuf::from(home).join(".cargo/bin/gk"));
    }
    for cand in fallbacks {
        if cand.is_file() {
            return Ok(cand);
        }
    }
    Err("gk binary not found (install gk or set GK_BIN)".into())
}

pub fn cmd_err(out: &std::process::Output) -> String {
    let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
    let std_out = String::from_utf8_lossy(&out.stdout).trim().to_string();
    match (err.is_empty(), std_out.is_empty()) {
        (false, false) => format!("{err} | {std_out}"),
        (false, true) => err,
        (true, false) => std_out,
        (true, true) => String::new(),
    }
}

pub fn format_sub_err(label: &str, out: &std::process::Output) -> String {
    let msg = cmd_err(out);
    if msg.is_empty() { format!("{label} ({})", out.status) } else { format!("{label} ({}): {msg}", out.status) }
}
