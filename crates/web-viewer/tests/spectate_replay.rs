//! Disc-gated check that spectator mode reproduces an AI run exactly.
//!
//! `LEGAIA_SPECTATE_STREAM` names a stream the full-game ladder recorded
//! (`LEGAIA_FGL_RECORD`), and `LEGAIA_SPECTATE_EXPECT` the ladder's
//! `[replay]` fingerprint for it. The page's runtime loads the disc, begins
//! spectating, applies the whole stream in page-sized steps and must land on
//! the same fingerprint as the AI's own session. Skips + passes when any of
//! `LEGAIA_DISC_BIN` / the two variables is unset.

#![cfg(not(target_arch = "wasm32"))]

use legaia_web_viewer::runtime::LegaiaRuntime;

#[test]
fn spectator_replay_matches_the_ai_session() {
    let (Ok(disc), Ok(stream), Ok(expect)) = (
        std::env::var("LEGAIA_DISC_BIN"),
        std::env::var("LEGAIA_SPECTATE_STREAM"),
        std::env::var("LEGAIA_SPECTATE_EXPECT"),
    ) else {
        eprintln!("[skip] LEGAIA_DISC_BIN / LEGAIA_SPECTATE_STREAM / LEGAIA_SPECTATE_EXPECT unset");
        return;
    };
    let mut rt = LegaiaRuntime::new();
    rt.load_disc(std::fs::read(&disc).expect("read disc"), String::new())
        .expect("load disc");
    let opening = rt.spectate_begin().expect("spectate_begin");
    eprintln!("[spectate] began in {opening}");
    let text = std::fs::read_to_string(&stream).expect("read stream");
    let queued = rt.spectate_feed(&text).expect("feed stream");
    eprintln!("[spectate] queued {queued} events");

    // Page-sized steps: a display frame drains a couple of sim ticks, and a
    // scene entry ends a step early so the page can rebuild.
    let mut scenes = vec![opening];
    loop {
        let r: serde_json::Value = serde_json::from_str(&rt.spectate_step(2)).expect("step json");
        if let Some(s) = r["entered"].as_str().filter(|s| !s.is_empty()) {
            scenes.push(s.to_string());
        }
        if r["pending"].as_u64() == Some(0) {
            eprintln!(
                "[spectate] {} ticks, scenes {}",
                r["total_ticks"],
                scenes.join(">")
            );
            break;
        }
    }
    let got = rt.spectate_fingerprint();
    eprintln!("[spectate] want {expect}");
    eprintln!("[spectate] got  {got}");
    assert_eq!(got, expect, "spectator replay diverged from the AI session");
}
