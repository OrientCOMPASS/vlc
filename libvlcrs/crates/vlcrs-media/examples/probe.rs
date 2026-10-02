//! `cargo run -p vlcrs-media --example probe -- <file>…`
//!
//! Prints what the prober sees: container, tracks, duration and the spherical
//! metadata that drives the `Auto` projection mode.  Handy when validating the
//! CI-generated test media or a user-supplied file.

use vlcrs_media::{probe_path, ProjectionKind};
use vlcrs_vr::ResolvedProjection;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("usage: probe <file>…");
        std::process::exit(2);
    }
    let mut failed = false;
    for path in &args {
        println!("=== {path} ===");
        match probe_path(path) {
            Ok(info) => {
                println!("summary: {}", info.summary());
                for t in &info.tracks {
                    println!(
                        "  track {} {:?} codec={:?} {}x{} rot={} {}Hz/{}ch dur={:?}ms",
                        t.track_id,
                        t.kind,
                        t.codec,
                        t.width,
                        t.height,
                        t.rotation,
                        t.sample_rate,
                        t.channels,
                        t.duration_ms
                    );
                    if let Some(s) = t.spherical {
                        let crop = match s.crop {
                            Some(c) => format!(
                                "(t{:.3} b{:.3} l{:.3} r{:.3})",
                                c.top, c.bottom, c.cropped_left, c.right
                            ),
                            None => "none".to_string(),
                        };
                        println!(
                            "    spherical: {:?} stereo={:?} swap={} crop={} pose={:?} version={:?} coverage={:?} deg={:?}",
                            s.projection,
                            s.stereo,
                            s.swap_eyes,
                            crop,
                            s.pose,
                            s.version,
                            s.coverage(t.width, t.height),
                            s.coverage_degrees()
                        );
                    }
                }
                let hints = info.hints();
                println!(
                    "hints: {}x{} rot={} stereo={:?} swap={} coverage={:?} pose={:?}",
                    hints.width,
                    hints.height,
                    hints.rotation,
                    hints.stereo,
                    hints.swap_eyes,
                    hints.coverage,
                    hints.pose
                );
                for mode in vlcrs_vr::projection::ALL_MODES {
                    let r = ResolvedProjection::resolve(mode, &hints, vlcrs_vr::Eye::Left);
                    println!("  {mode:<10} -> {}", r.describe());
                    if matches!(r.mode, vlcrs_vr::ProjectionMode::Auto)
                        && matches!(
                            info.video().and_then(|v| v.spherical).map(|s| s.projection),
                            Some(ProjectionKind::Cubemap) | Some(ProjectionKind::Mesh)
                        )
                    {
                        println!("    (projection not renderable by libvlcrs)");
                    }
                }
            }
            Err(e) => {
                println!("probe error: {e}");
                failed = true;
            }
        }
        println!();
    }
    if failed {
        std::process::exit(1);
    }
}
