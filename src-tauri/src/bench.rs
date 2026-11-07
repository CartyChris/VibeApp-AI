//! Timings for the paths every turn runs through. Not a test of correctness:
//! `cargo test --release --lib bench_hot_paths -- --ignored --nocapture`.
//! Set `VIBE_BENCH_PAGE` to a real page to include it in the project.

use crate::{agent, fixer, i18n, offline, a11y, perf, preview, project, quality, staging, verify};
use std::time::Instant;

fn median_ms<F: FnMut()>(runs: usize, mut f: F) -> f64 {
    let mut v: Vec<f64> = (0..runs)
        .map(|_| {
            let t = Instant::now();
            f();
            t.elapsed().as_secs_f64() * 1000.0
        })
        .collect();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

fn js(i: usize, lines: usize) -> String {
    let mut s = format!("// module {i}\nimport {{ helper{i} }} from './lib{i}.js';\n");
    for k in 0..lines {
        s.push_str(&format!(
            "export function fn{i}_{k}(a, b) {{ const v = a * {k} + b; if (v > 10) {{ return helper{i}(v, 'text {k}'); }} return v; }}\n"
        ));
    }
    s
}

fn css(i: usize, rules: usize) -> String {
    (0..rules).map(|k| format!(".c{i}-{k} {{ margin: {k}px; padding: 8px 12px; color: #333; }}\n")).collect()
}

fn html(i: usize, rows: usize) -> String {
    let mut s = String::from("<!doctype html><html><head><title>p</title></head><body>\n");
    for k in 0..rows {
        s.push_str(&format!("<div class=\"row r{k}\"><button id=\"b{i}-{k}\">Button {k}</button><img src=\"a{k}.png\" alt=\"\"></div>\n"));
    }
    s.push_str("<script>document.querySelectorAll('button').forEach(b => b.onclick = () => console.log(b.id));</script></body></html>\n");
    s
}

#[test]
#[ignore]
fn bench_hot_paths() {
    let id = format!("test-bench-{}", uuid::Uuid::new_v4());
    std::fs::create_dir_all(project::files_dir(&id)).unwrap();
    project::save(&project::Project { id: id.clone(), entry: "index.html".into(), kind: "game".into(), ..Default::default() }).unwrap();
    for i in 0..200 { project::write_file(&id, &format!("src/m{i}.js"), &js(i, 60)).unwrap(); }
    for i in 0..60 { project::write_file(&id, &format!("styles/s{i}.css"), &css(i, 60)).unwrap(); }
    for i in 0..40 { project::write_file(&id, &format!("pages/p{i}.html"), &html(i, 80)).unwrap(); }
    let page = std::env::var("VIBE_BENCH_PAGE").ok().and_then(|p| std::fs::read_to_string(p).ok())
        .unwrap_or_else(|| html(999, 2000));
    project::write_file(&id, "index.html", &page).unwrap();

    let files = crate::context_files(&id);
    let total: usize = files.iter().map(|(_, s)| s.len()).sum();
    println!("project: {} files, {:.1} MB; entry {} KB", files.len(), total as f64 / 1e6, page.len() / 1000);
    let r = |name: &str, ms: f64| println!("  {name:<34} {ms:>9.2} ms");

    r("list_live", median_ms(9, || { std::hint::black_box(project::list_live(&id)); }));
    r("read every file", median_ms(9, || { std::hint::black_box(project::read_many(&id, &project::file_paths(&id))); }));
    r("context_files (lib)", median_ms(9, || { std::hint::black_box(crate::context_files(&id)); }));
    r("repo_map", median_ms(9, || { std::hint::black_box(agent::repo_map(&files)); }));
    r("select_files (200k budget)", median_ms(9, || { std::hint::black_box(agent::select_files(&files, "index.html", 200_000, "make the castle gate open")); }));
    r("build_context_focused", median_ms(9, || { std::hint::black_box(agent::build_context_focused(&files, "index.html", 200_000, "make the castle gate open")); }));
    let proj = project::load(&id).unwrap();
    r("build_turn (whole prompt)", median_ms(9, || { std::hint::black_box(crate::build_turn(&id, &proj, None, "make the castle gate open", crate::TurnKind::Build, "")); }));
    r("search_files", median_ms(9, || { std::hint::black_box(project::search_files(&files, "helper17", 200)); }));
    r("quality::score_project", median_ms(5, || { std::hint::black_box(quality::score_project(&files)); }));
    r("a11y+offline+i18n+verify each file", median_ms(5, || {
        for (p, s) in &files {
            std::hint::black_box((a11y::audit(p, s), offline::audit(p, s), i18n::audit(p, s), verify::check_file(p, s)));
        }
    }));
    r("perf::audit_many", median_ms(5, || { std::hint::black_box(perf::audit_many(&files)); }));
    r("preview::inject entry", median_ms(9, || { std::hint::black_box(preview::inject(&page, "")); }));
    r("preview::serve entry", median_ms(9, || { std::hint::black_box(preview::serve(&id, "index.html")); }));
    r("snapshot", median_ms(5, || { project::snapshot(&id, "bench").unwrap(); }));
    let sid = project::snapshots(&id)[0].id.clone();
    project::write_file(&id, "src/m3.js", "changed\n").unwrap();
    r("diff_snapshot", median_ms(5, || { std::hint::black_box(project::diff_snapshot(&id, &sid).unwrap()); }));
    r("snapshots() list", median_ms(9, || { std::hint::black_box(project::snapshots(&id)); }));

    let mut ops = Vec::new();
    for k in 0..25 {
        ops.push(agent::FileOp::Replace { path: "src/m5.js".into(), find: format!("fn5_{k}(a, b)"), to: format!("fn5_{k}(a, b, c)") });
    }
    let cs = staging::stage(&id, &ops, "bench");
    r("stage 25 replaces", median_ms(9, || { std::hint::black_box(staging::stage(&id, &ops, "bench")); }));
    r("compacted (25 -> 1)", median_ms(9, || { std::hint::black_box(cs.clone().compacted()); }));

    let errs: Vec<fixer::RuntimeError> = (0..300).map(|k| fixer::RuntimeError {
        message: format!("TypeError: x{} is undefined", k % 7), source: "index.html".into(), line: 100 + k % 7, col: 3, stack: String::new() }).collect();
    r("fixer group+prompt (300 errors)", median_ms(9, || {
        let g = fixer::group(&errs);
        std::hint::black_box(fixer::fix_prompt(&g, false, &files));
    }));
    // A 200 KB reply arriving in 5-byte chunks, journalled the way chat_send does.
    let head = project::TurnJournal { stream: "s".into(), ask: "a".into(), ..Default::default() };
    let chunk = "abcd ";
    let n = 200_000 / chunk.len();
    r("journal a 200 KB reply", median_ms(3, || {
        let mut full = String::new();
        let mut journalled = 0usize;
        for _ in 0..n {
            full.push_str(chunk);
            if full.len() >= journalled + project::JOURNAL_EVERY {
                project::journal_write(&id, &head, &full, journalled);
                journalled = full.len();
            }
        }
    }));
    let back = project::journal_load(&id).map(|j| j.text.len()).unwrap_or(0);
    println!("  journal reads back {back} bytes");
    // Events that cross into the webview for one second of streaming.
    for (label, gap_us) in [("100 chunks/s (typical)", 10_000u64), ("1,000 chunks/s (fast host)", 1_000)] {
        let count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let c = count.clone();
        let d = crate::batch::Deltas::new(move |_, _| { c.fetch_add(1, std::sync::atomic::Ordering::Relaxed); });
        let ticker = d.clone();
        let h = std::thread::spawn(move || while ticker.tick() { std::thread::sleep(crate::batch::WINDOW); });
        let n = 1_000_000 / gap_us;
        for _ in 0..n { d.send("text", "tok ".into()); std::thread::sleep(std::time::Duration::from_micros(gap_us)); }
        d.finish();
        drop(d);
        h.join().unwrap();
        println!("  IPC events, {label:<26} {n:>5} chunks -> {:>4} events", count.load(std::sync::atomic::Ordering::Relaxed));
    }
    let _ = std::fs::remove_dir_all(project::dir(&id));
}
