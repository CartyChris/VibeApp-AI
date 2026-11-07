//! Project scaffolds for every packaging target the stack selector offers.
//!
//! Each generator writes a small but *real* project: a valid manifest, an
//! entry point that runs, a build script, and a README that says exactly what
//! to install. The AI then builds into that skeleton, which is why the
//! manifests matter more than the amount of code.

use anyhow::{anyhow, Result};
use std::fs;
use std::path::Path;

use crate::stack::Selection;
use crate::wrap::{sanitize_bundle_id, ExportResult};

fn slug(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect();
    let s = s.split('-').filter(|x| !x.is_empty()).collect::<Vec<_>>().join("-");
    if s.is_empty() { "app".into() } else { s }
}

/// A Go module path / Rust crate name must not start with a digit.
fn ident(name: &str) -> String {
    let s = slug(name).replace('-', "_");
    if s.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(true) {
        format!("app_{s}")
    } else {
        s
    }
}

fn write(root: &Path, rel: &str, body: &str) -> Result<()> {
    let p = root.join(rel);
    if let Some(d) = p.parent() {
        fs::create_dir_all(d)?;
    }
    fs::write(p, body)?;
    Ok(())
}

fn exec(root: &Path, rel: &str, body: &str) -> Result<()> {
    write(root, rel, body)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(root.join(rel), fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

/// Copy the project's web assets in, for scaffolds that host a web layer.
fn copy_web(src: &Path, dst: &Path) -> Result<usize> {
    fs::create_dir_all(dst)?;
    let mut n = 0;
    for e in walkdir::WalkDir::new(src).into_iter().filter_map(|e| e.ok()) {
        if !e.file_type().is_file() {
            continue;
        }
        let rel = e.path().strip_prefix(src)?;
        let target = dst.join(rel);
        if let Some(p) = target.parent() {
            fs::create_dir_all(p)?;
        }
        fs::copy(e.path(), &target)?;
        n += 1;
    }
    Ok(n)
}

/// Dispatch on the stack's packaging choice.
pub fn export_stack(pid: &str, sel: &Selection, dst: &Path) -> Result<ExportResult> {
    let packaging = sel.get("packaging").map(|s| s.as_str()).unwrap_or("");
    match packaging {
        "tauri" => tauri_app(pid, dst),
        "cargo-bin" => rust_binary(pid, sel, dst),
        "wails" => wails_app(pid, dst),
        "go-bin" => go_binary(pid, sel, dst),
        "pyinstaller" | "briefcase" => python_app(pid, sel, dst, packaging),
        "capacitor" | "gradle" => android_app(pid, sel, dst, packaging),
        "static" | "pwa" => crate::wrap::export_web(pid, dst),
        "electron" => crate::wrap::export_electron(pid, dst),
        "xcode" => crate::wrap::export_ios(pid, dst),
        "docker" => docker_service(pid, sel, dst),
        "" => Err(anyhow!("Pick a packaging option in the Stack panel first.")),
        other => Err(anyhow!("No scaffold for '{other}' yet.")),
    }
}

/// React + Vite, generated when the stack asks for React on the web.
pub fn react_app(pid: &str, dst: &Path) -> Result<ExportResult> {
    let p = crate::project::load(pid)?;
    let name = slug(&p.name);
    let root = dst.join(&name);
    fs::create_dir_all(&root)?;

    let pkg = serde_json::json!({
        "name": name, "private": true, "version": "1.0.0", "type": "module",
        "scripts": { "dev": "vite", "build": "vite build", "preview": "vite preview" },
        "dependencies": { "react": "^18.3.1", "react-dom": "^18.3.1" },
        "devDependencies": { "@vitejs/plugin-react": "^4.3.1", "vite": "^5.4.0" }
    });
    write(&root, "package.json", &serde_json::to_string_pretty(&pkg)?)?;
    write(&root, "vite.config.js",
        "import { defineConfig } from 'vite';\nimport react from '@vitejs/plugin-react';\n\nexport default defineConfig({ plugins: [react()] });\n")?;
    write(&root, "index.html", &format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1, viewport-fit=cover\">\n\
         <title>{}</title>\n</head>\n<body>\n<div id=\"root\"></div>\n\
         <script type=\"module\" src=\"/src/main.jsx\"></script>\n</body>\n</html>\n", p.name))?;
    write(&root, "src/main.jsx",
        "import React from 'react';\nimport { createRoot } from 'react-dom/client';\nimport App from './App.jsx';\nimport './index.css';\n\ncreateRoot(document.getElementById('root')).render(<App />);\n")?;
    write(&root, "src/App.jsx", &format!(
        "export default function App() {{\n  return (\n    <main className=\"app\">\n      <h1>{}</h1>\n      <p>Built with VibeApp AI.</p>\n    </main>\n  );\n}}\n", p.name))?;
    write(&root, "src/index.css",
        ":root{color-scheme:dark}\n*{box-sizing:border-box}\nbody{margin:0;font:15px/1.5 -apple-system,system-ui,sans-serif;background:#0b0b14;color:#eee}\n.app{padding:max(24px,env(safe-area-inset-top)) 24px 24px}\n")?;
    readme(&root, &p.name, "React + Vite", &["npm install", "npm run dev", "npm run build"], &["node"])?;

    Ok(res("react", &root, 6, vec!["Vite dev server with hot reload.".into()],
           vec!["cd in, then `npm install && npm run dev`.".into()]))
}

fn tauri_app(pid: &str, dst: &Path) -> Result<ExportResult> {
    let p = crate::project::load(pid)?;
    let name = slug(&p.name);
    let root = dst.join(format!("{name}-tauri"));
    fs::create_dir_all(&root)?;
    let n = copy_web(&crate::project::files_dir(pid), &root.join("src"))?;

    let pkg = serde_json::json!({
        "name": name, "private": true, "version": "1.0.0",
        "scripts": { "dev": "tauri dev", "build": "tauri build" },
        "devDependencies": { "@tauri-apps/cli": "^2" }
    });
    write(&root, "package.json", &serde_json::to_string_pretty(&pkg)?)?;
    write(&root, "src-tauri/Cargo.toml", &format!(
        "[package]\nname = \"{}\"\nversion = \"1.0.0\"\nedition = \"2021\"\n\n\
         [build-dependencies]\ntauri-build = {{ version = \"2\" }}\n\n\
         [dependencies]\ntauri = {{ version = \"2\" }}\nserde = {{ version = \"1\", features = [\"derive\"] }}\nserde_json = \"1\"\n\n\
         [profile.release]\nlto = true\nstrip = true\nopt-level = \"s\"\n", ident(&p.name)))?;
    write(&root, "src-tauri/build.rs", "fn main() { tauri_build::build() }\n")?;
    write(&root, "src-tauri/src/main.rs",
        "#![cfg_attr(not(debug_assertions), windows_subsystem = \"windows\")]\n\n\
         fn main() {\n    tauri::Builder::default()\n        .run(tauri::generate_context!())\n        .expect(\"error while running app\");\n}\n")?;
    let conf = serde_json::json!({
        "$schema": "https://schema.tauri.app/config/2",
        "productName": p.name, "version": "1.0.0",
        "identifier": sanitize_bundle_id(&p.name),
        "build": { "frontendDist": "../src" },
        "app": { "windows": [{ "title": p.name, "width": 1000, "height": 720 }],
                 "security": { "csp": null } },
        "bundle": { "active": true, "targets": ["dmg", "app", "nsis", "appimage"] }
    });
    write(&root, "src-tauri/tauri.conf.json", &serde_json::to_string_pretty(&conf)?)?;
    readme(&root, &p.name, "Tauri (Rust core + system webview)",
           &["npm install", "npm run dev", "npm run build"], &["node", "cargo"])?;

    Ok(res("tauri", &root, n + 6,
        vec!["Rust core with the system webview — binaries are a few MB, not a few hundred.".into()],
        vec!["`npm install && npm run dev`. Needs the Rust toolchain.".into()]))
}

fn rust_binary(pid: &str, sel: &Selection, dst: &Path) -> Result<ExportResult> {
    let p = crate::project::load(pid)?;
    let name = ident(&p.name);
    let root = dst.join(format!("{}-rs", slug(&p.name)));
    fs::create_dir_all(&root)?;

    let ui = sel.get("frontend").map(|s| s.as_str()).unwrap_or("none-ui");
    let backend = sel.get("backend").map(|s| s.as_str()).unwrap_or("none-be");

    let mut deps = String::new();
    let main_body;
    match (ui, backend) {
        ("egui", _) => {
            deps.push_str("eframe = \"0.28\"\negui = \"0.28\"\n");
            main_body = format!(
                "use eframe::egui;\n\nfn main() -> eframe::Result<()> {{\n    \
                 let opts = eframe::NativeOptions::default();\n    \
                 eframe::run_native(\"{}\", opts, Box::new(|_cc| Box::<App>::default()))\n}}\n\n\
                 #[derive(Default)]\nstruct App {{ clicks: u32 }}\n\n\
                 impl eframe::App for App {{\n    \
                 fn update(&mut self, ctx: &egui::Context, _f: &mut eframe::Frame) {{\n        \
                 egui::CentralPanel::default().show(ctx, |ui| {{\n            \
                 ui.heading(\"{}\");\n            \
                 if ui.button(format!(\"Clicked {{}} times\", self.clicks)).clicked() {{\n                \
                 self.clicks += 1;\n            }}\n        }});\n    }}\n}}\n", p.name, p.name);
        }
        (_, "axum") => {
            deps.push_str("axum = \"0.7\"\ntokio = { version = \"1\", features = [\"full\"] }\n");
            main_body = "use axum::{routing::get, Router};\n\n#[tokio::main]\nasync fn main() {\n    \
                 let app = Router::new().route(\"/\", get(|| async { \"ok\" }));\n    \
                 let l = tokio::net::TcpListener::bind(\"0.0.0.0:3000\").await.unwrap();\n    \
                 println!(\"listening on http://localhost:3000\");\n    \
                 axum::serve(l, app).await.unwrap();\n}\n".to_string();
        }
        _ => {
            main_body = format!("fn main() {{\n    println!(\"{} — built with VibeApp AI\");\n}}\n", p.name);
        }
    }

    write(&root, "Cargo.toml", &format!(
        "[package]\nname = \"{name}\"\nversion = \"1.0.0\"\nedition = \"2021\"\n\n\
         [dependencies]\n{deps}\n[profile.release]\nlto = true\nstrip = true\nopt-level = \"s\"\n"))?;
    write(&root, "src/main.rs", &main_body)?;
    readme(&root, &p.name, "Rust binary", &["cargo run", "cargo build --release"], &["cargo"])?;

    Ok(res("rust", &root, 3,
        vec![format!("Cargo crate `{name}`, release profile tuned for size.")],
        vec!["`cargo run` to start, `cargo build --release` for the binary.".into()]))
}

fn wails_app(pid: &str, dst: &Path) -> Result<ExportResult> {
    let p = crate::project::load(pid)?;
    let name = slug(&p.name);
    let root = dst.join(format!("{name}-wails"));
    fs::create_dir_all(&root)?;
    let n = copy_web(&crate::project::files_dir(pid), &root.join("frontend/dist"))?;

    write(&root, "go.mod", &format!(
        "module {}\n\ngo 1.22\n\nrequire github.com/wailsapp/wails/v2 v2.9.1\n", ident(&p.name)))?;
    write(&root, "main.go", &format!(
        "package main\n\nimport (\n\t\"embed\"\n\n\t\"github.com/wailsapp/wails/v2\"\n\t\"github.com/wailsapp/wails/v2/pkg/options\"\n)\n\n\
         //go:embed all:frontend/dist\nvar assets embed.FS\n\n\
         func main() {{\n\terr := wails.Run(&options.App{{\n\t\tTitle:  \"{}\",\n\t\tWidth:  1000,\n\t\tHeight: 720,\n\t\tAssetServer: &options.AssetServer{{Assets: assets}},\n\t}})\n\tif err != nil {{\n\t\tpanic(err)\n\t}}\n}}\n", p.name))?;
    let conf = serde_json::json!({
        "$schema": "https://wails.io/schemas/config.v2.json",
        "name": name, "outputfilename": name,
        "frontend:dir": "./frontend", "wailsjsdir": "./frontend"
    });
    write(&root, "wails.json", &serde_json::to_string_pretty(&conf)?)?;
    readme(&root, &p.name, "Wails (Go core + system webview)",
           &["go mod tidy", "wails dev", "wails build"], &["go", "wails"])?;

    Ok(res("wails", &root, n + 3,
        vec!["Go core with the system webview; the frontend is embedded in the binary.".into()],
        vec!["Install Wails: `go install github.com/wailsapp/wails/v2/cmd/wails@latest`, then `wails dev`.".into()]))
}

fn go_binary(pid: &str, sel: &Selection, dst: &Path) -> Result<ExportResult> {
    let p = crate::project::load(pid)?;
    let root = dst.join(format!("{}-go", slug(&p.name)));
    fs::create_dir_all(&root)?;
    let gin = sel.get("backend").map(|s| s.as_str()) == Some("gin");

    write(&root, "go.mod", &format!(
        "module {}\n\ngo 1.22\n{}", ident(&p.name),
        if gin { "\nrequire github.com/gin-gonic/gin v1.10.0\n" } else { "" }))?;
    write(&root, "main.go", &if gin {
        "package main\n\nimport \"github.com/gin-gonic/gin\"\n\n\
         func main() {\n\tr := gin.Default()\n\t\
         r.GET(\"/\", func(c *gin.Context) { c.JSON(200, gin.H{\"status\": \"ok\"}) })\n\t\
         r.Run(\":3000\")\n}\n".to_string()
    } else {
        format!("package main\n\nimport \"fmt\"\n\nfunc main() {{\n\tfmt.Println(\"{} — built with VibeApp AI\")\n}}\n", p.name)
    })?;
    readme(&root, &p.name, "Go binary", &["go mod tidy", "go run .", "go build -o app ."], &["go"])?;

    Ok(res("go", &root, 3,
        vec!["A single static binary with no runtime to install.".into()],
        vec!["`go mod tidy && go run .`".into()]))
}

fn python_app(pid: &str, sel: &Selection, dst: &Path, packaging: &str) -> Result<ExportResult> {
    let p = crate::project::load(pid)?;
    let name = slug(&p.name);
    let root = dst.join(format!("{name}-py"));
    fs::create_dir_all(&root)?;

    let ui = sel.get("frontend").map(|s| s.as_str()).unwrap_or("none-ui");
    let backend = sel.get("backend").map(|s| s.as_str()).unwrap_or("none-be");

    let (mut reqs, main_body) = match (ui, backend) {
        ("flet", _) => (
            vec!["flet>=0.24"],
            format!(
                "import flet as ft\n\n\ndef main(page: ft.Page):\n    \
                 page.title = \"{}\"\n    page.theme_mode = ft.ThemeMode.DARK\n    \
                 page.padding = 24\n\n    count = ft.Text(\"0\", size = 40)\n\n    \
                 def bump(_):\n        count.value = str(int(count.value) + 1)\n        page.update()\n\n    \
                 page.add(\n        ft.Text(\"{}\", size = 24, weight = ft.FontWeight.BOLD),\n        \
                 count,\n        ft.FilledButton(\"Tap me\", on_click = bump),\n    )\n\n\n\
                 ft.app(target = main)\n", p.name, p.name)),
        ("kivy", _) => (
            vec!["kivy>=2.3"],
            format!(
                "from kivy.app import App\nfrom kivy.uix.button import Button\n\n\n\
                 class {}App(App):\n    def build(self):\n        return Button(text = \"{}\")\n\n\n\
                 if __name__ == \"__main__\":\n    {}App().run()\n",
                ident(&p.name), p.name, ident(&p.name))),
        (_, "fastapi") => (
            vec!["fastapi>=0.115", "uvicorn[standard]>=0.30"],
            "from fastapi import FastAPI\n\napp = FastAPI()\n\n\n@app.get(\"/\")\ndef root():\n    return {\"status\": \"ok\"}\n\n\n\
             if __name__ == \"__main__\":\n    import uvicorn\n    uvicorn.run(app, host = \"0.0.0.0\", port = 8000)\n".to_string()),
        _ => (vec![], format!("def main():\n    print(\"{} — built with VibeApp AI\")\n\n\nif __name__ == \"__main__\":\n    main()\n", p.name)),
    };
    if packaging == "pyinstaller" {
        reqs.push("pyinstaller>=6.10");
    } else {
        reqs.push("briefcase>=0.3.20");
    }

    write(&root, "requirements.txt", &format!("{}\n", reqs.join("\n")))?;
    write(&root, "main.py", &main_body)?;
    write(&root, "pyproject.toml", &format!(
        "[project]\nname = \"{name}\"\nversion = \"1.0.0\"\nrequires-python = \">=3.10\"\n"))?;
    exec(&root, "build.sh", &format!(
        "#!/bin/bash\nset -euo pipefail\ncd \"$(dirname \"$0\")\"\n\
         python3 -m venv .venv\nsource .venv/bin/activate\n\
         pip install -r requirements.txt\n{}\n",
        if packaging == "pyinstaller" {
            "pyinstaller --noconfirm --windowed --name \"$(basename \"$PWD\")\" main.py\necho '==> dist/ contains the app'"
        } else {
            "briefcase create\nbriefcase build\necho '==> briefcase package  # to make an installer'"
        }))?;
    readme(&root, &p.name,
           if packaging == "pyinstaller" { "Python + PyInstaller" } else { "Python + Briefcase" },
           &["./build.sh"], &["python3"])?;

    Ok(res("python", &root, 5,
        vec![format!("Packaged with {packaging}. A virtualenv is created by build.sh.")],
        vec!["Run `./build.sh`. Python 3.10 or newer.".into()]))
}

fn android_app(pid: &str, _sel: &Selection, dst: &Path, packaging: &str) -> Result<ExportResult> {
    let p = crate::project::load(pid)?;
    let name = slug(&p.name);
    let root = dst.join(format!("{name}-android"));
    fs::create_dir_all(&root)?;
    let n = copy_web(&crate::project::files_dir(pid), &root.join("www"))?;
    let app_id = sanitize_bundle_id(&p.name).replace("ai.vibeapp.", "ai.vibeapp.android.");

    if packaging == "capacitor" {
        let pkg = serde_json::json!({
            "name": name, "private": true, "version": "1.0.0",
            "scripts": {
                "init": "cap add android",
                "sync": "cap sync android",
                "open": "cap open android",
                "run": "cap run android"
            },
            "dependencies": { "@capacitor/android": "^6", "@capacitor/core": "^6" },
            "devDependencies": { "@capacitor/cli": "^6" }
        });
        write(&root, "package.json", &serde_json::to_string_pretty(&pkg)?)?;
        let conf = serde_json::json!({
            "appId": app_id, "appName": p.name, "webDir": "www",
            "server": { "androidScheme": "https" }
        });
        write(&root, "capacitor.config.json", &serde_json::to_string_pretty(&conf)?)?;
        exec(&root, "run-android.sh",
            "#!/bin/bash\nset -euo pipefail\ncd \"$(dirname \"$0\")\"\n\
             npm install\nnpx cap add android 2>/dev/null || true\nnpx cap sync android\n\
             npx cap open android   # opens Android Studio\n")?;
        readme(&root, &p.name, "Android via Capacitor",
               &["npm install", "npx cap add android", "npx cap sync android", "npx cap open android"],
               &["node", "Android Studio"])?;
    } else {
        write(&root, "settings.gradle.kts", &format!("rootProject.name = \"{name}\"\ninclude(\":app\")\n"))?;
        write(&root, "app/build.gradle.kts", &format!(
            "plugins {{ id(\"com.android.application\") }}\n\nandroid {{\n    \
             namespace = \"{app_id}\"\n    compileSdk = 35\n    defaultConfig {{\n        \
             applicationId = \"{app_id}\"\n        minSdk = 24\n        targetSdk = 35\n        \
             versionCode = 1\n        versionName = \"1.0\"\n    }}\n}}\n"))?;
        readme(&root, &p.name, "Android via Gradle",
               &["./gradlew assembleDebug"], &["Android Studio", "JDK 17"])?;
    }

    Ok(res("android", &root, n + 3,
        vec![format!("Application id `{app_id}`."),
             "Android Studio does the actual build; this is the project it opens.".into()],
        vec!["Install Android Studio, then run the script in the folder.".into()]))
}

fn docker_service(pid: &str, sel: &Selection, dst: &Path) -> Result<ExportResult> {
    let p = crate::project::load(pid)?;
    let root = dst.join(format!("{}-service", slug(&p.name)));
    fs::create_dir_all(&root)?;
    let lang = sel.get("language").map(|s| s.as_str()).unwrap_or("ts");

    let dockerfile = match lang {
        "python" => "FROM python:3.12-slim\nWORKDIR /app\nCOPY requirements.txt .\nRUN pip install --no-cache-dir -r requirements.txt\nCOPY . .\nEXPOSE 8000\nCMD [\"python\", \"main.py\"]\n",
        "rust" => "FROM rust:1-slim AS build\nWORKDIR /src\nCOPY . .\nRUN cargo build --release\n\nFROM debian:stable-slim\nCOPY --from=build /src/target/release/app /usr/local/bin/app\nEXPOSE 3000\nCMD [\"app\"]\n",
        "go" => "FROM golang:1.22 AS build\nWORKDIR /src\nCOPY . .\nRUN go build -o /app .\n\nFROM gcr.io/distroless/base-debian12\nCOPY --from=build /app /app\nEXPOSE 3000\nCMD [\"/app\"]\n",
        _ => "FROM node:22-slim\nWORKDIR /app\nCOPY package*.json ./\nRUN npm ci --omit=dev\nCOPY . .\nEXPOSE 3000\nCMD [\"node\", \"server.js\"]\n",
    };
    write(&root, "Dockerfile", dockerfile)?;
    write(&root, ".dockerignore", "node_modules\ntarget\n.venv\n__pycache__\n.git\n")?;
    write(&root, "compose.yaml", &format!(
        "services:\n  app:\n    build: .\n    ports:\n      - \"3000:3000\"\n    environment:\n      - NODE_ENV=production\n    restart: unless-stopped\n# {}\n", p.name))?;
    readme(&root, &p.name, "Docker service", &["docker compose up --build"], &["docker"])?;

    Ok(res("docker", &root, 4,
        vec![format!("Multi-stage build for {lang}.")],
        vec!["`docker compose up --build`".into()]))
}

fn readme(root: &Path, name: &str, stack: &str, steps: &[&str], needs: &[&str]) -> Result<()> {
    write(root, "README.md", &format!(
        "# {name}\n\nGenerated by VibeApp AI.\n\n**Stack:** {stack}\n\n\
         ## You need\n\n{}\n\n## Run it\n\n```bash\n{}\n```\n\n\
         ## Notes\n\n- This is a working skeleton, not a finished app. Build into it.\n\
         - Keep it self-contained: no CDN links, so it still works offline.\n",
        needs.iter().map(|n| format!("- {n}")).collect::<Vec<_>>().join("\n"),
        steps.join("\n")))
}

fn res(kind: &str, root: &Path, files: usize, notes: Vec<String>, next: Vec<String>) -> ExportResult {
    ExportResult {
        kind: kind.into(),
        path: root.display().to_string(),
        files,
        notes,
        next_steps: next,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project;

    fn scratch(name: &str) -> String {
        let id = format!("test-{}", uuid::Uuid::new_v4());
        fs::create_dir_all(project::files_dir(&id)).unwrap();
        project::save(&project::Project {
            id: id.clone(), name: name.into(), entry: "index.html".into(), ..Default::default()
        }).unwrap();
        project::write_file(&id, "index.html", "<h1>hi</h1>").unwrap();
        id
    }

    fn sel(pairs: &[(&str, &str)]) -> Selection {
        pairs.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
    }

    fn out(id: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("vibe-scaffold-{id}"))
    }

    #[test]
    fn identifiers_are_always_legal() {
        assert_eq!(slug("My Cool App"), "my-cool-app");
        assert_eq!(slug("!!!"), "app");
        assert_eq!(ident("2048 Game"), "app_2048_game");
        assert_eq!(ident("My App"), "my_app");
        // Never empty, never starting with a digit — both break Cargo and Go.
        for n in ["", "9", "---", "Ünïcode", "a"] {
            let i = ident(n);
            assert!(!i.is_empty());
            assert!(!i.chars().next().unwrap().is_ascii_digit(), "{n} -> {i}");
        }
    }

    #[test]
    fn every_packaging_option_produces_a_scaffold_that_parses() {
        // Each entry: packaging, plus enough of a stack to be realistic.
        let cases: Vec<(&str, Vec<(&str, &str)>)> = vec![
            ("tauri",       vec![("target","desktop"),("language","ts"),("packaging","tauri")]),
            ("cargo-bin",   vec![("target","desktop"),("language","rust"),("frontend","egui"),("packaging","cargo-bin")]),
            ("wails",       vec![("target","desktop"),("language","go"),("packaging","wails")]),
            ("go-bin",      vec![("target","server"),("language","go"),("backend","gin"),("packaging","go-bin")]),
            ("pyinstaller", vec![("target","desktop"),("language","python"),("frontend","flet"),("packaging","pyinstaller")]),
            ("briefcase",   vec![("target","desktop"),("language","python"),("frontend","kivy"),("packaging","briefcase")]),
            ("capacitor",   vec![("target","android"),("language","ts"),("packaging","capacitor")]),
            ("gradle",      vec![("target","android"),("language","kotlin"),("packaging","gradle")]),
            ("docker",      vec![("target","server"),("language","python"),("packaging","docker")]),
            ("static",      vec![("target","web"),("language","ts"),("packaging","static")]),
            ("electron",    vec![("target","desktop"),("language","ts"),("packaging","electron")]),
            ("xcode",       vec![("target","ios"),("language","swift"),("packaging","xcode")]),
        ];

        for (packaging, pairs) in cases {
            let id = scratch("Wave Rider");
            let dir = out(&id);
            let r = export_stack(&id, &sel(&pairs), &dir)
                .unwrap_or_else(|e| panic!("{packaging}: {e}"));
            let root = std::path::PathBuf::from(&r.path);
            assert!(root.exists(), "{packaging}: nothing written");
            assert!(r.files > 0, "{packaging}: zero files");
            assert!(!r.next_steps.is_empty(), "{packaging}: no instructions");

            // Whatever manifest it produced must actually parse.
            for (f, kind) in [("package.json", "json"), ("capacitor.config.json", "json"),
                              ("wails.json", "json"), ("src-tauri/tauri.conf.json", "json")] {
                let p = root.join(f);
                if p.exists() && kind == "json" {
                    let body = fs::read_to_string(&p).unwrap();
                    serde_json::from_str::<serde_json::Value>(&body)
                        .unwrap_or_else(|e| panic!("{packaging}/{f} is not valid JSON: {e}"));
                }
            }
            // Cargo/Go manifests must at least name the module.
            if root.join("Cargo.toml").exists() {
                let c = fs::read_to_string(root.join("Cargo.toml")).unwrap();
                assert!(c.contains("[package]") && c.contains("name = \"wave_rider\""), "{packaging}: {c}");
            }
            if root.join("go.mod").exists() {
                let g = fs::read_to_string(root.join("go.mod")).unwrap();
                assert!(g.starts_with("module "), "{packaging}: {g}");
            }
            // Everything except the pre-existing exporters ships a README.
            if !["static", "electron", "xcode"].contains(&packaging) {
                assert!(root.join("README.md").exists(), "{packaging}: no README");
            }
            let _ = fs::remove_dir_all(&dir);
            let _ = fs::remove_dir_all(project::dir(&id));
        }
    }

    #[test]
    fn scaffolds_adapt_to_the_rest_of_the_stack() {
        // Rust + egui gets a GUI; Rust + axum gets a server.
        let id = scratch("Demo");
        let dir = out(&id);
        let gui = export_stack(&id, &sel(&[("language","rust"),("frontend","egui"),("packaging","cargo-bin")]), &dir).unwrap();
        let main = fs::read_to_string(std::path::PathBuf::from(&gui.path).join("src/main.rs")).unwrap();
        assert!(main.contains("eframe"), "egui stack should scaffold a GUI");
        let _ = fs::remove_dir_all(&dir);

        let dir2 = out(&format!("{id}-2"));
        let srv = export_stack(&id, &sel(&[("language","rust"),("backend","axum"),("packaging","cargo-bin")]), &dir2).unwrap();
        let main2 = fs::read_to_string(std::path::PathBuf::from(&srv.path).join("src/main.rs")).unwrap();
        assert!(main2.contains("axum"), "axum stack should scaffold a server");
        assert!(fs::read_to_string(std::path::PathBuf::from(&srv.path).join("Cargo.toml")).unwrap().contains("axum"));
        let _ = fs::remove_dir_all(&dir2);

        // Python + Flet gets Flet; Python + FastAPI gets uvicorn.
        let dir3 = out(&format!("{id}-3"));
        let flet = export_stack(&id, &sel(&[("language","python"),("frontend","flet"),("packaging","pyinstaller")]), &dir3).unwrap();
        let py = fs::read_to_string(std::path::PathBuf::from(&flet.path).join("main.py")).unwrap();
        assert!(py.contains("import flet"));
        assert!(fs::read_to_string(std::path::PathBuf::from(&flet.path).join("requirements.txt")).unwrap().contains("pyinstaller"));
        let _ = fs::remove_dir_all(&dir3);

        let _ = fs::remove_dir_all(project::dir(&id));
    }

    #[test]
    fn unknown_or_missing_packaging_fails_with_a_useful_message() {
        let id = scratch("X");
        let dir = out(&id);
        let e = export_stack(&id, &Selection::new(), &dir).unwrap_err().to_string();
        assert!(e.contains("Stack panel"), "got: {e}");
        let e2 = export_stack(&id, &sel(&[("packaging", "nonsense")]), &dir).unwrap_err().to_string();
        assert!(e2.contains("No scaffold"));
        let _ = fs::remove_dir_all(project::dir(&id));
    }

    #[test]
    fn react_scaffold_is_a_working_vite_project() {
        let id = scratch("Note Pad");
        let dir = out(&id);
        let r = react_app(&id, &dir).unwrap();
        let root = std::path::PathBuf::from(&r.path);
        let pkg: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(root.join("package.json")).unwrap()).unwrap();
        assert_eq!(pkg["type"], "module");
        assert!(pkg["dependencies"]["react"].is_string());
        assert!(pkg["scripts"]["dev"].as_str().unwrap().contains("vite"));
        assert!(root.join("src/App.jsx").exists());
        assert!(root.join("src/main.jsx").exists());
        assert!(fs::read_to_string(root.join("index.html")).unwrap().contains("viewport-fit=cover"));
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(project::dir(&id));
    }
}
