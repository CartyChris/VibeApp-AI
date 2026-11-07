//! The full-stack selector.
//!
//! One table drives everything: which options exist, which targets and
//! languages they work with, and what they conflict with. Availability is a
//! pure function of that table plus the current selection, so an invalid stack
//! is not merely discouraged — it cannot be expressed.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Layers, in the order the user picks them. Each layer filters the next.
pub const LAYERS: &[(&str, &str)] = &[
    ("target", "What you're shipping"),
    ("language", "Primary language"),
    ("frontend", "User interface"),
    ("backend", "Server / API"),
    ("database", "Data"),
    ("styling", "Styling"),
    ("packaging", "How it ships"),
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StackOption {
    pub id: String,
    pub layer: String,
    pub label: String,
    pub blurb: String,
    /// Targets this works with. Empty means "any target".
    pub targets: Vec<String>,
    /// Languages this works with. Empty means "any language".
    pub langs: Vec<String>,
    /// Options that cannot be selected alongside this one.
    pub conflicts: Vec<String>,
    /// Command that must exist on the machine to build with this.
    pub tool: String,
    /// True when this is the sensible default for its layer.
    pub common: bool,
}

macro_rules! opt {
    ($id:expr, $layer:expr, $label:expr, $blurb:expr,
     targets: [$($t:expr),*], langs: [$($l:expr),*],
     conflicts: [$($c:expr),*], tool: $tool:expr, common: $common:expr) => {
        StackOption {
            id: $id.into(), layer: $layer.into(), label: $label.into(), blurb: $blurb.into(),
            targets: vec![$($t.to_string()),*],
            langs: vec![$($l.to_string()),*],
            conflicts: vec![$($c.to_string()),*],
            tool: $tool.into(), common: $common,
        }
    };
}

/// Every option the selector can offer.
pub fn options() -> Vec<StackOption> {
    vec![
        // ---------------- target ----------------
        opt!("ios", "target", "iOS app", "iPhone and iPad, shipped through the App Store.",
             targets: [], langs: [], conflicts: [], tool: "", common: true),
        opt!("android", "target", "Android app", "Phones and tablets, shipped through Google Play.",
             targets: [], langs: [], conflicts: [], tool: "", common: true),
        opt!("desktop", "target", "Desktop app", "A real macOS / Windows / Linux application.",
             targets: [], langs: [], conflicts: [], tool: "", common: true),
        opt!("web", "target", "Web app", "Runs in a browser; installable as a PWA.",
             targets: [], langs: [], conflicts: [], tool: "", common: true),
        opt!("server", "target", "Backend service", "An API or worker with no user interface.",
             targets: [], langs: [], conflicts: [], tool: "", common: false),
        opt!("cli", "target", "Command-line tool", "A terminal program.",
             targets: [], langs: [], conflicts: [], tool: "", common: false),

        // ---------------- language ----------------
        opt!("ts", "language", "TypeScript / JavaScript", "The widest ecosystem; runs everywhere.",
             targets: ["ios","android","desktop","web","server","cli"], langs: [],
             conflicts: [], tool: "node", common: true),
        opt!("python", "language", "Python", "Fastest to write; strongest for data and AI.",
             targets: ["desktop","web","server","cli","android"], langs: [],
             conflicts: [], tool: "python3", common: true),
        opt!("rust", "language", "Rust", "Fast and memory-safe; tiny binaries.",
             targets: ["desktop","web","server","cli"], langs: [],
             conflicts: [], tool: "cargo", common: false),
        opt!("go", "language", "Go", "Simple concurrency; single static binary.",
             targets: ["desktop","server","cli"], langs: [],
             conflicts: [], tool: "go", common: false),
        opt!("swift", "language", "Swift", "Native Apple platforms, best performance and feel.",
             targets: ["ios","desktop"], langs: [], conflicts: [], tool: "swift", common: false),
        opt!("kotlin", "language", "Kotlin", "Native Android.",
             targets: ["android"], langs: [], conflicts: [], tool: "", common: false),

        // ---------------- frontend ----------------
        opt!("none-ui", "frontend", "No interface", "Headless service or CLI.",
             targets: ["server","cli"], langs: [], conflicts: [], tool: "", common: true),
        opt!("vanilla", "frontend", "HTML + CSS + JS", "No framework. Smallest, fastest, zero build step.",
             targets: ["ios","android","desktop","web"], langs: ["ts"],
             conflicts: [], tool: "", common: true),
        opt!("react", "frontend", "React", "Component model with the largest ecosystem.",
             targets: ["ios","android","desktop","web"], langs: ["ts"],
             conflicts: [], tool: "node", common: true),
        opt!("svelte", "frontend", "Svelte", "Compiles away; very little runtime.",
             targets: ["desktop","web"], langs: ["ts"], conflicts: [], tool: "node", common: false),
        opt!("react-native", "frontend", "React Native", "Real native views driven from React.",
             targets: ["ios","android"], langs: ["ts"],
             conflicts: ["electron","tauri","capacitor"], tool: "node", common: false),
        opt!("swiftui", "frontend", "SwiftUI", "Apple's native UI framework.",
             targets: ["ios","desktop"], langs: ["swift"], conflicts: [], tool: "swift", common: true),
        opt!("compose", "frontend", "Jetpack Compose", "Android's native UI toolkit.",
             targets: ["android"], langs: ["kotlin"], conflicts: [], tool: "", common: true),
        opt!("flet", "frontend", "Flet", "Flutter UI driven from Python. Desktop, web and mobile.",
             targets: ["desktop","web","android"], langs: ["python"], conflicts: [], tool: "python3", common: true),
        opt!("kivy", "frontend", "Kivy", "Python UI toolkit with touch support.",
             targets: ["desktop","android"], langs: ["python"], conflicts: [], tool: "python3", common: false),
        opt!("egui", "frontend", "egui", "Immediate-mode Rust UI. No web layer at all.",
             targets: ["desktop"], langs: ["rust"], conflicts: ["tauri"], tool: "cargo", common: false),
        opt!("slint", "frontend", "Slint", "Declarative native Rust UI.",
             targets: ["desktop"], langs: ["rust"], conflicts: ["tauri"], tool: "cargo", common: false),

        // ---------------- backend ----------------
        opt!("none-be", "backend", "No backend", "Everything runs on the device. Simplest and cheapest.",
             targets: [], langs: [], conflicts: [], tool: "", common: true),
        opt!("express", "backend", "Express", "The default Node server.",
             targets: [], langs: ["ts"], conflicts: [], tool: "node", common: true),
        opt!("fastify", "backend", "Fastify", "Faster Node server with schema validation.",
             targets: [], langs: ["ts"], conflicts: [], tool: "node", common: false),
        opt!("fastapi", "backend", "FastAPI", "Typed async Python with automatic docs.",
             targets: [], langs: ["python"], conflicts: [], tool: "python3", common: true),
        opt!("django", "backend", "Django", "Batteries-included Python, admin and ORM built in.",
             targets: [], langs: ["python"], conflicts: [], tool: "python3", common: false),
        opt!("axum", "backend", "Axum", "Async Rust on Tokio.",
             targets: [], langs: ["rust"], conflicts: [], tool: "cargo", common: true),
        opt!("gin", "backend", "Gin", "Fast, minimal Go HTTP framework.",
             targets: [], langs: ["go"], conflicts: [], tool: "go", common: true),
        opt!("supabase", "backend", "Supabase", "Hosted Postgres with auth and storage. No server to run.",
             targets: [], langs: [], conflicts: [], tool: "", common: true),

        // ---------------- database ----------------
        opt!("none-db", "database", "No database", "Nothing to persist beyond local state.",
             targets: [], langs: [], conflicts: [], tool: "", common: true),
        opt!("localstorage", "database", "Local storage", "Key/value on the device. Zero setup.",
             targets: ["ios","android","desktop","web"], langs: [], conflicts: [], tool: "", common: true),
        opt!("sqlite", "database", "SQLite", "A real database in a single file, on the device.",
             targets: [], langs: [], conflicts: [], tool: "", common: true),
        opt!("postgres", "database", "PostgreSQL", "The default serious relational database.",
             targets: ["desktop","web","server"], langs: [], conflicts: [], tool: "", common: true),
        opt!("mongodb", "database", "MongoDB", "Document store; flexible schema.",
             targets: ["desktop","web","server"], langs: [], conflicts: [], tool: "", common: false),

        // ---------------- styling ----------------
        opt!("none-css", "styling", "Not applicable", "No visual layer.",
             targets: ["server","cli"], langs: [], conflicts: [], tool: "", common: true),
        opt!("plain-css", "styling", "Plain CSS", "Custom properties and modern CSS. No dependency.",
             targets: ["ios","android","desktop","web"], langs: ["ts"], conflicts: [], tool: "", common: true),
        opt!("tailwind", "styling", "Tailwind", "Utility classes; very fast to iterate.",
             targets: ["ios","android","desktop","web"], langs: ["ts"],
             conflicts: [], tool: "node", common: true),
        // CSS-in-JS needs a component framework to scope to; there is nothing
        // for it to attach to in a plain HTML page.
        opt!("styled", "styling", "Styled Components", "CSS-in-JS scoped to components.",
             targets: ["ios","android","desktop","web"], langs: ["ts"],
             conflicts: ["vanilla","svelte"], tool: "node", common: false),
        opt!("native-style", "styling", "Native styling", "The platform's own styling system.",
             targets: ["ios","android","desktop"], langs: ["swift","kotlin","rust","python"],
             conflicts: [], tool: "", common: true),

        // ---------------- packaging ----------------
        opt!("xcode", "packaging", "Xcode project", "A real .xcodeproj you open and Run.",
             targets: ["ios"], langs: [], conflicts: [], tool: "xcodebuild", common: true),
        opt!("capacitor", "packaging", "Capacitor", "Wraps a web app as a native iOS/Android app.",
             targets: ["ios","android"], langs: ["ts","python"],
             conflicts: ["react-native"], tool: "node", common: true),
        opt!("gradle", "packaging", "Gradle / Android Studio", "The native Android build.",
             targets: ["android"], langs: ["kotlin","ts"], conflicts: [], tool: "gradle", common: true),
        opt!("electron", "packaging", "Electron", "Cross-platform desktop from web tech. Large but reliable.",
             targets: ["desktop"], langs: ["ts"], conflicts: ["react-native"], tool: "node", common: true),
        opt!("tauri", "packaging", "Tauri", "Desktop with a Rust core and the system webview. Tiny binaries.",
             targets: ["desktop"], langs: ["ts","rust"],
             conflicts: ["egui","slint","react-native"], tool: "cargo", common: true),
        opt!("wails", "packaging", "Wails", "Desktop with a Go core and the system webview.",
             targets: ["desktop"], langs: ["go"], conflicts: [], tool: "go", common: true),
        opt!("cargo-bin", "packaging", "Cargo binary", "A single native executable.",
             targets: ["desktop","cli","server"], langs: ["rust"], conflicts: [], tool: "cargo", common: true),
        opt!("go-bin", "packaging", "Go binary", "One static executable, no runtime needed.",
             targets: ["desktop","cli","server"], langs: ["go"], conflicts: [], tool: "go", common: true),
        opt!("pyinstaller", "packaging", "PyInstaller", "Bundles Python into an executable.",
             targets: ["desktop","cli"], langs: ["python"], conflicts: [], tool: "python3", common: true),
        opt!("briefcase", "packaging", "Briefcase", "Packages Python for desktop and mobile.",
             targets: ["desktop","android"], langs: ["python"], conflicts: [], tool: "python3", common: false),
        opt!("static", "packaging", "Static site", "Plain files on any host.",
             targets: ["web"], langs: ["ts"], conflicts: [], tool: "", common: true),
        opt!("pwa", "packaging", "PWA", "Installable, works offline, no app store.",
             targets: ["web"], langs: ["ts"], conflicts: [], tool: "", common: true),
        opt!("docker", "packaging", "Docker", "A container image to deploy anywhere.",
             targets: ["server"], langs: [], conflicts: [], tool: "docker", common: true),
    ]
}

pub type Selection = BTreeMap<String, String>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptionState {
    #[serde(flatten)]
    pub option: StackOption,
    pub enabled: bool,
    /// Why it can't be picked. Empty when enabled.
    pub reason: String,
    pub selected: bool,
}

fn get<'a>(sel: &'a Selection, layer: &str) -> Option<&'a str> {
    sel.get(layer).map(|s| s.as_str()).filter(|s| !s.is_empty())
}

fn label_of(id: &str) -> String {
    options()
        .into_iter()
        .find(|o| o.id == id)
        .map(|o| o.label)
        .unwrap_or_else(|| id.to_string())
}

/// Why `opt` cannot be chosen given `sel`, or `None` if it can.
fn blocked_by(opt: &StackOption, sel: &Selection) -> Option<String> {
    if let Some(t) = get(sel, "target") {
        if !opt.targets.is_empty() && !opt.targets.iter().any(|x| x == t) {
            return Some(format!("not available when targeting {}", label_of(t)));
        }
    }
    if let Some(l) = get(sel, "language") {
        if !opt.langs.is_empty() && !opt.langs.iter().any(|x| x == l) {
            return Some(format!("needs a different language — this is {} only",
                                opt.langs.iter().map(|x| label_of(x)).collect::<Vec<_>>().join(" / ")));
        }
    }
    // Conflicts are symmetric: either side declaring it blocks the pair.
    for (layer, chosen) in sel.iter() {
        if layer == &opt.layer || chosen.is_empty() {
            continue;
        }
        if opt.conflicts.iter().any(|c| c == chosen) {
            return Some(format!("cannot be combined with {}", label_of(chosen)));
        }
        if let Some(other) = options().into_iter().find(|o| &o.id == chosen) {
            if other.conflicts.iter().any(|c| c == &opt.id) {
                return Some(format!("cannot be combined with {}", other.label));
            }
        }
    }
    None
}

/// Every option for `layer`, each marked enabled or with the reason it isn't.
pub fn layer_state(layer: &str, sel: &Selection) -> Vec<OptionState> {
    options()
        .into_iter()
        .filter(|o| o.layer == layer)
        .map(|o| {
            let reason = blocked_by(&o, sel);
            OptionState {
                selected: get(sel, layer) == Some(o.id.as_str()),
                enabled: reason.is_none(),
                reason: reason.unwrap_or_default(),
                option: o,
            }
        })
        .collect()
}

/// Drop any selection that the current choices have invalidated.
///
/// Called after every change, so switching target or language repairs the rest
/// of the stack instead of leaving a combination that cannot be built.
pub fn sanitize(sel: &Selection) -> Selection {
    let mut out = Selection::new();
    // Resolve in layer order so upstream choices constrain downstream ones.
    for (layer, _) in LAYERS {
        let Some(id) = get(sel, layer) else { continue };
        let Some(opt) = options().into_iter().find(|o| o.id == id && &o.layer == layer) else {
            continue;
        };
        if blocked_by(&opt, &out).is_none() {
            out.insert((*layer).to_string(), id.to_string());
        }
    }
    out
}

/// A complete, buildable default stack for a target.
pub fn preset(target: &str) -> Selection {
    let mut sel = Selection::new();
    sel.insert("target".into(), target.to_string());
    for (layer, _) in LAYERS.iter().skip(1) {
        let pick = layer_state(layer, &sel)
            .into_iter()
            .filter(|s| s.enabled)
            .min_by_key(|s| if s.option.common { 0 } else { 1 })
            .map(|s| s.option.id);
        if let Some(p) = pick {
            sel.insert((*layer).to_string(), p);
        }
    }
    sanitize(&sel)
}

/// Tools this stack needs, and whether each is actually installed here.
pub fn required_tools(sel: &Selection) -> Vec<(String, bool)> {
    let mut seen: Vec<String> = Vec::new();
    for id in sel.values() {
        if let Some(o) = options().into_iter().find(|o| &o.id == id) {
            if !o.tool.is_empty() && !seen.contains(&o.tool) {
                seen.push(o.tool.clone());
            }
        }
    }
    seen.into_iter()
        .map(|t| {
            let found = std::process::Command::new("/usr/bin/which")
                .arg(&t)
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);
            (t, found)
        })
        .collect()
}

/// One-line description of the stack, for the system prompt and the UI.
pub fn describe(sel: &Selection) -> String {
    LAYERS
        .iter()
        .filter_map(|(layer, _)| {
            let id = get(sel, layer)?;
            let o = options().into_iter().find(|o| o.id == id)?;
            Some(format!("{}: {}", layer, o.label))
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

/// Direction appended to the build agent's system prompt.
pub fn prompt_fragment(sel: &Selection) -> String {
    if sel.is_empty() {
        return String::new();
    }
    let mut s = String::from("# Target stack\n\nBuild for exactly this stack. Do not substitute a different framework or language.\n\n");
    for (layer, _) in LAYERS {
        if let Some(id) = get(sel, layer) {
            if let Some(o) = options().into_iter().find(|o| o.id == id) {
                s.push_str(&format!("- **{}**: {} — {}\n", layer, o.label, o.blurb));
            }
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sel(pairs: &[(&str, &str)]) -> Selection {
        pairs.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
    }

    #[test]
    fn every_option_is_well_formed_and_conflicts_point_at_real_options() {
        let all = options();
        assert!(all.len() > 40, "only {} options", all.len());
        let ids: Vec<&str> = all.iter().map(|o| o.id.as_str()).collect();
        let layers: Vec<&str> = LAYERS.iter().map(|(l, _)| *l).collect();

        for o in &all {
            assert!(!o.label.is_empty() && !o.blurb.is_empty(), "{} is undescribed", o.id);
            assert!(layers.contains(&o.layer.as_str()), "{} has unknown layer {}", o.id, o.layer);
            for c in &o.conflicts {
                assert!(ids.contains(&c.as_str()), "{} conflicts with unknown {c}", o.id);
                assert_ne!(c, &o.id, "{} conflicts with itself", o.id);
            }
        }
        // ids are unique
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        let before = sorted.len();
        sorted.dedup();
        assert_eq!(before, sorted.len(), "duplicate option ids");

        // every layer has at least one option, and at least one common default
        for l in &layers {
            let in_layer: Vec<&StackOption> = all.iter().filter(|o| &o.layer == l).collect();
            assert!(!in_layer.is_empty(), "layer {l} is empty");
            if *l != "target" {
                assert!(in_layer.iter().any(|o| o.common), "layer {l} has no common default");
            }
        }
    }

    #[test]
    fn target_filters_what_is_even_offered() {
        // Swift is not an option for an Android app.
        let android = sel(&[("target", "android")]);
        let langs = layer_state("language", &android);
        let swift = langs.iter().find(|s| s.option.id == "swift").unwrap();
        assert!(!swift.enabled);
        assert!(swift.reason.contains("not available when targeting Android"),
                "reason: {}", swift.reason);
        // No mangled articles in user-facing text.
        assert!(!swift.reason.contains(" a ios"), "reason: {}", swift.reason);
        assert!(langs.iter().find(|s| s.option.id == "kotlin").unwrap().enabled);

        // Rust is not offered for iOS.
        let ios = sel(&[("target", "ios")]);
        assert!(!layer_state("language", &ios).iter().find(|s| s.option.id == "rust").unwrap().enabled);
        assert!(layer_state("language", &ios).iter().find(|s| s.option.id == "swift").unwrap().enabled);
    }

    #[test]
    fn language_filters_frameworks_with_a_readable_reason() {
        let s = sel(&[("target", "desktop"), ("language", "python")]);
        let fe = layer_state("frontend", &s);
        assert!(fe.iter().find(|x| x.option.id == "flet").unwrap().enabled);

        let react = fe.iter().find(|x| x.option.id == "react").unwrap();
        assert!(!react.enabled, "React is not a Python UI framework");
        assert!(react.reason.contains("TypeScript"), "reason: {}", react.reason);

        // Backends follow the same rule.
        let be = layer_state("backend", &s);
        assert!(be.iter().find(|x| x.option.id == "fastapi").unwrap().enabled);
        assert!(!be.iter().find(|x| x.option.id == "express").unwrap().enabled);
        assert!(!be.iter().find(|x| x.option.id == "axum").unwrap().enabled);
    }

    #[test]
    fn conflicts_are_symmetric_and_explained() {
        // Cross-layer: CSS-in-JS has no components to scope to in plain HTML.
        let a = sel(&[("target", "web"), ("language", "ts"), ("frontend", "vanilla")]);
        let styled = layer_state("styling", &a).into_iter()
            .find(|x| x.option.id == "styled").unwrap();
        assert!(!styled.enabled);
        assert!(styled.reason.contains("HTML"), "reason: {}", styled.reason);
        // With React it is perfectly valid.
        let b = sel(&[("target", "web"), ("language", "ts"), ("frontend", "react")]);
        assert!(layer_state("styling", &b).into_iter()
            .find(|x| x.option.id == "styled").unwrap().enabled);

        // Same-layer options need no conflict data — one pick per layer already
        // makes them exclusive. Assert the data does not pretend otherwise.
        for o in options() {
            for c in &o.conflicts {
                let other = options().into_iter().find(|x| &x.id == c).unwrap();
                assert_ne!(other.layer, o.layer,
                    "{} declares a same-layer conflict with {c}, which is vacuous", o.id);
            }
        }

        // React Native declares conflicts with Electron/Tauri/Capacitor, and the
        // block must apply from either direction.
        let rn = sel(&[("target", "ios"), ("language", "ts"), ("frontend", "react-native")]);
        let pkg = layer_state("packaging", &rn);
        let cap = pkg.iter().find(|x| x.option.id == "capacitor").unwrap();
        assert!(!cap.enabled, "Capacitor wraps a web app; React Native has no web layer");
        assert!(cap.reason.contains("React Native"), "reason: {}", cap.reason);
    }

    #[test]
    fn changing_target_repairs_the_rest_of_the_stack() {
        // A valid desktop/Rust/egui stack…
        let desktop = sel(&[
            ("target", "desktop"), ("language", "rust"),
            ("frontend", "egui"), ("packaging", "cargo-bin"),
        ]);
        assert_eq!(sanitize(&desktop), desktop, "a valid stack must survive untouched");

        // …becomes impossible when the target changes to iOS, and the invalid
        // parts are dropped rather than silently kept.
        let mut moved = desktop.clone();
        moved.insert("target".into(), "ios".into());
        let fixed = sanitize(&moved);
        assert_eq!(fixed.get("target").unwrap(), "ios");
        assert!(!fixed.contains_key("language"), "Rust is not an iOS language");
        assert!(!fixed.contains_key("frontend"), "egui cannot survive to iOS");
        assert!(!fixed.contains_key("packaging"));
    }

    #[test]
    fn sanitize_is_idempotent_and_never_invents_a_selection() {
        for t in ["ios", "android", "desktop", "web", "server", "cli"] {
            let p = preset(t);
            assert_eq!(sanitize(&p), p, "{t}: preset should already be clean");
            assert_eq!(sanitize(&sanitize(&p)), p, "{t}: sanitize must be idempotent");
        }
        assert!(sanitize(&Selection::new()).is_empty());
        // Junk in, nothing out — never a fabricated pick.
        assert!(sanitize(&sel(&[("target", "not-a-target")])).is_empty());
    }

    #[test]
    fn every_preset_is_internally_consistent_and_complete() {
        for t in ["ios", "android", "desktop", "web", "server", "cli"] {
            let p = preset(t);
            assert_eq!(p.get("target").map(|s| s.as_str()), Some(t));
            // Every layer that has any enabled option must be filled.
            for (layer, _) in LAYERS.iter().skip(1) {
                let any = layer_state(layer, &p).into_iter().any(|s| s.enabled);
                if any {
                    assert!(p.contains_key(*layer), "{t}: layer {layer} left empty");
                }
            }
            // And every pick must still be legal alongside the others.
            for (layer, id) in p.iter() {
                let o = options().into_iter().find(|o| &o.id == id).unwrap();
                let mut others = p.clone();
                others.remove(layer);
                assert!(blocked_by(&o, &others).is_none(),
                        "{t}: {id} is illegal in its own preset");
            }
            assert!(!describe(&p).is_empty());
            assert!(prompt_fragment(&p).contains("Target stack"));
        }
    }

    #[test]
    fn presets_pick_sensible_things() {
        let ios = preset("ios");
        assert_eq!(ios.get("target").unwrap(), "ios");
        // iOS should not default to a headless UI.
        assert_ne!(ios.get("frontend").map(|s| s.as_str()), Some("none-ui"));

        let server = preset("server");
        assert_eq!(server.get("frontend").map(|s| s.as_str()), Some("none-ui"));
        assert_eq!(server.get("styling").map(|s| s.as_str()), Some("none-css"));

        let cli = preset("cli");
        assert_eq!(cli.get("frontend").map(|s| s.as_str()), Some("none-ui"));
    }

    #[test]
    fn prompt_fragment_pins_the_stack_and_is_empty_when_unset() {
        let p = preset("web");
        let f = prompt_fragment(&p);
        assert!(f.contains("Do not substitute a different framework"));
        assert!(f.contains("**target**"));
        assert!(prompt_fragment(&Selection::new()).is_empty());
    }
}
