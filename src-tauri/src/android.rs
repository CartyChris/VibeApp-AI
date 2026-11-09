//! The real Android Emulator, alongside the in-app Android device frames.
//!
//! An Android Virtual Device can't load VibeApp's `vibeapp://` preview, so the
//! project is served over plain HTTP on 127.0.0.1 — never the network — and
//! opened in the emulator's browser at `10.0.2.2`, the emulator's name for the
//! Mac's loopback. Nothing here builds an APK; the Ship panel's Android export
//! does that.
//!
//! Every action is started by the user. Programs run directly with argv.

use anyhow::{anyhow, bail, Context, Result};
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub fn sdk() -> Option<PathBuf> {
    for var in ["ANDROID_HOME", "ANDROID_SDK_ROOT"] {
        if let Ok(v) = std::env::var(var) {
            let p = PathBuf::from(v);
            if p.is_dir() {
                return Some(p);
            }
        }
    }
    let p = dirs::home_dir()?.join("Library/Android/sdk");
    p.is_dir().then_some(p)
}

fn tool(rel: &str) -> Option<PathBuf> {
    let p = sdk()?.join(rel);
    p.exists().then_some(p)
}

pub fn adb() -> Option<PathBuf> {
    tool("platform-tools/adb")
}

pub fn emulator() -> Option<PathBuf> {
    tool("emulator/emulator")
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct Status {
    pub sdk: String,
    pub adb: bool,
    pub emulator: bool,
    pub avds: Vec<String>,
    /// Serials of running emulators or phones that adb sees.
    pub running: Vec<String>,
    pub studio: bool,
    pub install: String,
}

/// AVD names from `emulator -list-avds`, ignoring its log noise.
pub fn parse_avds(out: &str) -> Vec<String> {
    out.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("INFO") && !l.starts_with("WARNING") && !l.contains(' ') && !l.contains('|'))
        .map(String::from)
        .collect()
}

/// Serials in the `device` state from `adb devices`.
pub fn parse_devices(out: &str) -> Vec<String> {
    out.lines()
        .skip_while(|l| !l.starts_with("List of devices"))
        .skip(1)
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            let serial = it.next()?;
            (it.next()? == "device").then(|| serial.to_string())
        })
        .collect()
}

fn run(bin: &Path, args: &[&str]) -> String {
    std::process::Command::new(bin).args(args).output().map(|o| String::from_utf8_lossy(&o.stdout).to_string()).unwrap_or_default()
}

pub fn status() -> Status {
    let studio = Path::new("/Applications/Android Studio.app").exists();
    let em = emulator();
    let ad = adb();
    Status {
        sdk: sdk().map(|p| p.to_string_lossy().into()).unwrap_or_default(),
        adb: ad.is_some(),
        emulator: em.is_some(),
        avds: em.as_ref().map(|e| parse_avds(&run(e, &["-list-avds"]))).unwrap_or_default(),
        running: ad.as_ref().map(|a| parse_devices(&run(a, &["devices"]))).unwrap_or_default(),
        studio,
        install: if studio {
            "Open Android Studio → More Actions → Virtual Device Manager → Create device (a Pixel with a recent system image).".into()
        } else {
            "Install Android Studio (Setup does it in one click, or developer.android.com/studio), then create a virtual device in its Device Manager. About 6–10 GB with one system image.".into()
        },
    }
}

/// Start an emulator window. Returns at once; it takes a while to boot.
pub fn boot(avd: &str) -> Result<()> {
    if avd.is_empty() || !avd.chars().all(|c| c.is_ascii_alphanumeric() || "_-.".contains(c)) {
        bail!("Not an emulator name.");
    }
    let em = emulator().ok_or_else(|| anyhow!("The Android emulator isn't installed."))?;
    std::process::Command::new(em)
        .args(["-avd", avd, "-netdelay", "none", "-netspeed", "full"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("starting the emulator")?;
    Ok(())
}

/// Open a URL in the default browser of a running emulator or phone.
pub fn open_url(serial: &str, url: &str) -> Result<String> {
    if !url.starts_with("http://10.0.2.2:") || url.contains(['\'', '"', ' ', ';', '&', '|', '`', '$']) {
        bail!("Only the project's own local address can be opened.");
    }
    let adb = adb().ok_or_else(|| anyhow!("adb isn't installed (Android SDK platform-tools)."))?;
    let out = std::process::Command::new(adb)
        .args(["-s", serial, "shell", "am", "start", "-a", "android.intent.action.VIEW", "-d", url])
        .output()
        .context("running adb")?;
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    if !out.status.success() || text.contains("Error") {
        bail!("adb could not open it: {}", text.trim());
    }
    Ok(text.trim().to_string())
}

// ---------------------------------------------------------------------------
// A loopback file server for one folder
// ---------------------------------------------------------------------------

fn servers() -> &'static Mutex<HashMap<PathBuf, u16>> {
    static S: OnceLock<Mutex<HashMap<PathBuf, u16>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The response for one request line, with the file's bytes. Pure apart from
/// reading the file, so it can be tested without sockets.
pub fn respond(root: &Path, request: &str, entry: &str) -> (u16, String, Vec<u8>) {
    let first = request.lines().next().unwrap_or("");
    let mut parts = first.split_whitespace();
    let method = parts.next().unwrap_or("");
    let target = parts.next().unwrap_or("/");
    if method != "GET" && method != "HEAD" {
        return (405, "text/plain".into(), b"Method not allowed".to_vec());
    }
    let path = target.split(['?', '#']).next().unwrap_or("/");
    let path = urlencoding::decode(path).map(|c| c.to_string()).unwrap_or_default();
    let rel = path.trim_start_matches('/');
    let rel = if rel.is_empty() { entry } else { rel };
    let Ok(file) = crate::state::safe_join(root, rel) else {
        return (404, "text/plain".into(), b"Not found".to_vec());
    };
    let file = if file.is_dir() { file.join("index.html") } else { file };
    match std::fs::read(&file) {
        Ok(bytes) => (200, crate::project::mime_of(&file.to_string_lossy()).into(), if method == "HEAD" { vec![] } else { bytes }),
        Err(_) => (404, "text/plain".into(), b"Not found".to_vec()),
    }
}

/// Serve `root` on 127.0.0.1 and return the port; reuses a running server.
pub async fn serve_dir(root: PathBuf, entry: String) -> Result<u16> {
    if let Some(p) = servers().lock().ok().and_then(|m| m.get(&root).copied()) {
        return Ok(p);
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    if let Ok(mut m) = servers().lock() {
        m.insert(root.clone(), port);
    }
    tokio::spawn(async move {
        while let Ok((mut sock, _)) = listener.accept().await {
            let root = root.clone();
            let entry = entry.clone();
            tokio::spawn(async move {
                let mut buf = vec![0u8; 8192];
                let n = match tokio::time::timeout(std::time::Duration::from_secs(10), sock.read(&mut buf)).await {
                    Ok(Ok(n)) if n > 0 => n,
                    _ => return,
                };
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                let (code, mime, body) = respond(&root, &req, &entry);
                let reason = match code { 200 => "OK", 404 => "Not Found", _ => "Method Not Allowed" };
                let head = format!(
                    "HTTP/1.1 {code} {reason}\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = sock.write_all(head.as_bytes()).await;
                let _ = sock.write_all(&body).await;
                let _ = sock.shutdown().await;
            });
        }
    });
    Ok(port)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adb_and_emulator_output_is_read_correctly() {
        assert_eq!(parse_avds("INFO    | Storing crashdata\nPixel_9_API_35\nMedium_Phone\n\n"), ["Pixel_9_API_35", "Medium_Phone"]);
        let devs = "* daemon started successfully\nList of devices attached\nemulator-5554\tdevice\nR58N12345\tunauthorized\n\n";
        assert_eq!(parse_devices(devs), ["emulator-5554"]);
        assert!(parse_devices("List of devices attached\n\n").is_empty());
    }

    #[test]
    fn only_the_projects_own_address_can_be_opened_and_only_real_avds_booted() {
        assert!(open_url("emulator-5554", "https://evil.example").is_err());
        assert!(open_url("emulator-5554", "http://10.0.2.2:5555/x'; reboot").is_err());
        assert!(boot("Pixel 9; rm -rf ~").is_err());
    }

    #[tokio::test]
    async fn the_loopback_server_serves_the_folder_and_nothing_outside_it() {
        let root = std::env::temp_dir().join(format!("vibe-android-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("assets")).unwrap();
        std::fs::write(root.join("index.html"), "<h1>Hi</h1>").unwrap();
        std::fs::write(root.join("assets/a b.css"), "body{}").unwrap();
        let (c, m, b) = respond(&root, "GET / HTTP/1.1\r\n", "index.html");
        assert_eq!((c, m.as_str(), b.as_slice()), (200, "text/html; charset=utf-8", b"<h1>Hi</h1>".as_slice()));
        assert_eq!(respond(&root, "GET /assets/a%20b.css?v=1 HTTP/1.1\r\n", "index.html").0, 200);
        assert_eq!(respond(&root, "GET /../../etc/passwd HTTP/1.1\r\n", "index.html").0, 404);
        assert_eq!(respond(&root, "GET /%2e%2e/%2e%2e/etc/passwd HTTP/1.1\r\n", "index.html").0, 404);
        assert_eq!(respond(&root, "POST / HTTP/1.1\r\n", "index.html").0, 405);
        assert!(respond(&root, "HEAD / HTTP/1.1\r\n", "index.html").2.is_empty());

        // And over a real socket, on loopback only.
        let port = serve_dir(root.clone(), "index.html".into()).await.unwrap();
        assert_eq!(serve_dir(root.clone(), "index.html".into()).await.unwrap(), port, "one server per folder");
        let body = reqwest::get(format!("http://127.0.0.1:{port}/")).await.unwrap().text().await.unwrap();
        assert_eq!(body, "<h1>Hi</h1>");
        std::fs::remove_dir_all(root).unwrap();
    }
}
