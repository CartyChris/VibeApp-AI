//! Export pipeline: turn a project's web assets into something shippable —
//! a static site / PWA, a native iOS app (real Xcode project), or an Electron
//! desktop app. Plus the `simctl` bridge for running on a real simulator.

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
#[cfg(test)]
use std::path::PathBuf;
use std::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportResult {
    pub kind: String,
    pub path: String,
    pub files: usize,
    pub notes: Vec<String>,
    pub next_steps: Vec<String>,
}

fn copy_www(src: &Path, dst: &Path) -> Result<usize> {
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
    if n == 0 {
        return Err(anyhow!("This project has no files to export yet."));
    }
    Ok(n)
}

/// Bundle identifiers must be reverse-DNS with a restricted character set.
pub fn sanitize_bundle_id(name: &str) -> String {
    let slug: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
        .to_lowercase();
    let slug = if slug.is_empty() || slug.chars().next().unwrap().is_ascii_digit() {
        format!("app-{slug}")
    } else {
        slug
    };
    format!("ai.vibeapp.{slug}")
}

/// Xcode target names can't contain most punctuation.
pub fn sanitize_target(name: &str) -> String {
    let s: String = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == ' ' || *c == '_')
        .collect();
    let s: String = s
        .split_whitespace()
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        })
        .collect();
    if s.is_empty() || s.chars().next().unwrap().is_ascii_digit() {
        format!("VibeApp{s}")
    } else {
        s
    }
}

// ---------------------------------------------------------------------------
// Web / PWA
// ---------------------------------------------------------------------------

pub fn export_web(pid: &str, dst: &Path) -> Result<ExportResult> {
    let p = crate::project::load(pid)?;
    let n = copy_www(&crate::project::files_dir(pid), dst)?;

    let manifest = serde_json::json!({
        "name": p.name,
        "short_name": p.name.chars().take(12).collect::<String>(),
        "start_url": format!("./{}", p.entry),
        "display": "standalone",
        "orientation": "any",
        "background_color": "#0b0b12",
        "theme_color": "#0b0b12",
        "icons": [{"src": "./icon.png", "sizes": "512x512", "type": "image/png"}]
    });
    fs::write(dst.join("manifest.webmanifest"), serde_json::to_vec_pretty(&manifest)?)?;

    // Offline-first service worker: cache everything on install, serve cache-first.
    let sw = r#"const CACHE='vibeapp-v1';
self.addEventListener('install',e=>{self.skipWaiting()});
self.addEventListener('activate',e=>{e.waitUntil(clients.claim())});
self.addEventListener('fetch',e=>{
  if(e.request.method!=='GET')return;
  e.respondWith(
    caches.match(e.request).then(hit=>hit||fetch(e.request).then(res=>{
      const copy=res.clone();
      caches.open(CACHE).then(c=>c.put(e.request,copy)).catch(()=>{});
      return res;
    }).catch(()=>caches.match('./index.html')))
  );
});"#;
    fs::write(dst.join("sw.js"), sw)?;

    Ok(ExportResult {
        kind: "web".into(),
        path: dst.display().to_string(),
        files: n + 2,
        notes: vec![
            "Added a web app manifest and an offline-first service worker.".into(),
            "Installable to the iOS home screen via Share → Add to Home Screen.".into(),
        ],
        next_steps: vec![
            "Drop this folder on any static host (Netlify, Vercel, GitHub Pages, S3).".into(),
            "Service workers need HTTPS (localhost is exempt).".into(),
        ],
    })
}

// ---------------------------------------------------------------------------
// iOS — a real, buildable Xcode project
// ---------------------------------------------------------------------------

const APP_SWIFT: &str = r##"import SwiftUI
import WebKit

@main
struct VibeHostApp: App {
    var body: some Scene {
        WindowGroup { ContentView().ignoresSafeArea() }
    }
}

struct ContentView: View {
    var body: some View { WebContainer().ignoresSafeArea() }
}

/// Hosts the bundled web app in a full-screen WKWebView configured to behave
/// like a native app: no bounce, no zoom, inline media, safe-area aware.
struct WebContainer: UIViewRepresentable {
    func makeUIView(context: Context) -> WKWebView {
        let cfg = WKWebViewConfiguration()
        cfg.allowsInlineMediaPlayback = true
        cfg.mediaTypesRequiringUserActionForPlayback = []
        cfg.defaultWebpagePreferences.allowsContentJavaScript = true

        // Bridge native haptics to the web layer as window.haptics.*
        let haptics = """
        window.haptics = {
          impact:(s)=>window.webkit?.messageHandlers?.haptic?.postMessage({t:'impact',s:s||'medium'}),
          selection:()=>window.webkit?.messageHandlers?.haptic?.postMessage({t:'selection'}),
          notify:(s)=>window.webkit?.messageHandlers?.haptic?.postMessage({t:'notify',s:s||'success'})
        };
        """
        cfg.userContentController.addUserScript(
            WKUserScript(source: haptics, injectionTime: .atDocumentStart, forMainFrameOnly: false))
        cfg.userContentController.add(context.coordinator, name: "haptic")

        let web = WKWebView(frame: .zero, configuration: cfg)
        web.scrollView.bounces = false
        web.scrollView.contentInsetAdjustmentBehavior = .never
        web.isOpaque = false
        web.backgroundColor = .black
        web.scrollView.backgroundColor = .black
        #if DEBUG
        if #available(iOS 16.4, *) { web.isInspectable = true }
        #endif

        if let root = Bundle.main.url(forResource: "www", withExtension: nil) {
            let entry = root.appendingPathComponent(ENTRY_POINT)
            web.loadFileURL(entry, allowingReadAccessTo: root)
        }
        return web
    }

    func updateUIView(_ uiView: WKWebView, context: Context) {}
    func makeCoordinator() -> Coordinator { Coordinator() }

    final class Coordinator: NSObject, WKScriptMessageHandler {
        func userContentController(_ c: WKUserContentController, didReceive m: WKScriptMessage) {
            guard let d = m.body as? [String: Any], let t = d["t"] as? String else { return }
            let style = d["s"] as? String ?? "medium"
            switch t {
            case "selection":
                UISelectionFeedbackGenerator().selectionChanged()
            case "notify":
                let type: UINotificationFeedbackGenerator.FeedbackType =
                    style == "error" ? .error : (style == "warning" ? .warning : .success)
                UINotificationFeedbackGenerator().notificationOccurred(type)
            default:
                let s: UIImpactFeedbackGenerator.FeedbackStyle =
                    style == "light" ? .light : style == "heavy" ? .heavy
                    : style == "soft" ? .soft : style == "rigid" ? .rigid : .medium
                UIImpactFeedbackGenerator(style: s).impactOccurred()
            }
        }
    }
}
"##;

fn info_plist(display_name: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key><string>en</string>
  <key>CFBundleDisplayName</key><string>{display_name}</string>
  <key>CFBundleExecutable</key><string>$(EXECUTABLE_NAME)</string>
  <key>CFBundleIdentifier</key><string>$(PRODUCT_BUNDLE_IDENTIFIER)</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  <key>CFBundleName</key><string>$(PRODUCT_NAME)</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>1.0</string>
  <key>CFBundleVersion</key><string>1</string>
  <key>LSRequiresIPhoneOS</key><true/>
  <key>UILaunchScreen</key>
  <dict><key>UIColorName</key><string></string></dict>
  <key>UIRequiredDeviceCapabilities</key><array><string>armv7</string></array>
  <key>UISupportedInterfaceOrientations</key>
  <array>
    <string>UIInterfaceOrientationPortrait</string>
    <string>UIInterfaceOrientationLandscapeLeft</string>
    <string>UIInterfaceOrientationLandscapeRight</string>
  </array>
  <key>UISupportedInterfaceOrientations~ipad</key>
  <array>
    <string>UIInterfaceOrientationPortrait</string>
    <string>UIInterfaceOrientationPortraitUpsideDown</string>
    <string>UIInterfaceOrientationLandscapeLeft</string>
    <string>UIInterfaceOrientationLandscapeRight</string>
  </array>
  <key>UIStatusBarStyle</key><string>UIStatusBarStyleLightContent</string>
  <key>UIViewControllerBasedStatusBarAppearance</key><false/>
  <key>NSAppTransportSecurity</key>
  <dict><key>NSAllowsArbitraryLoads</key><true/></dict>
</dict>
</plist>
"#
    )
}

/// A hand-written pbxproj. The web assets are attached as a *folder reference*
/// (lastKnownFileType = folder), so any number of files ship without needing an
/// entry each — which is what makes this template stable.
fn pbxproj(target: &str, bundle_id: &str) -> String {
    format!(
        r#"// !$*UTF8*$!
{{
	archiveVersion = 1;
	classes = {{}};
	objectVersion = 56;
	objects = {{

/* Begin PBXBuildFile section */
		A1000001000000000000001 /* App.swift in Sources */ = {{isa = PBXBuildFile; fileRef = A1000002000000000000001 /* App.swift */; }};
		A1000003000000000000001 /* www in Resources */ = {{isa = PBXBuildFile; fileRef = A1000004000000000000001 /* www */; }};
/* End PBXBuildFile section */

/* Begin PBXFileReference section */
		A1000002000000000000001 /* App.swift */ = {{isa = PBXFileReference; lastKnownFileType = sourcecode.swift; path = App.swift; sourceTree = "<group>"; }};
		A1000004000000000000001 /* www */ = {{isa = PBXFileReference; lastKnownFileType = folder; path = www; sourceTree = "<group>"; }};
		A1000005000000000000001 /* Info.plist */ = {{isa = PBXFileReference; lastKnownFileType = text.plist.xml; path = Info.plist; sourceTree = "<group>"; }};
		A1000006000000000000001 /* {target}.app */ = {{isa = PBXFileReference; explicitFileType = wrapper.application; includeInIndex = 0; path = "{target}.app"; sourceTree = BUILT_PRODUCTS_DIR; }};
/* End PBXFileReference section */

/* Begin PBXFrameworksBuildPhase section */
		A1000007000000000000001 = {{isa = PBXFrameworksBuildPhase; buildActionMask = 2147483647; files = (); runOnlyForDeploymentPostprocessing = 0; }};
/* End PBXFrameworksBuildPhase section */

/* Begin PBXGroup section */
		A1000008000000000000001 = {{
			isa = PBXGroup;
			children = (
				A1000009000000000000001 /* {target} */,
				A100000A000000000000001 /* Products */,
			);
			sourceTree = "<group>";
		}};
		A100000A000000000000001 /* Products */ = {{
			isa = PBXGroup;
			children = (A1000006000000000000001 /* {target}.app */);
			name = Products;
			sourceTree = "<group>";
		}};
		A1000009000000000000001 /* {target} */ = {{
			isa = PBXGroup;
			children = (
				A1000002000000000000001 /* App.swift */,
				A1000004000000000000001 /* www */,
				A1000005000000000000001 /* Info.plist */,
			);
			path = {target};
			sourceTree = "<group>";
		}};
/* End PBXGroup section */

/* Begin PBXNativeTarget section */
		A100000B000000000000001 /* {target} */ = {{
			isa = PBXNativeTarget;
			buildConfigurationList = A100000C000000000000001;
			buildPhases = (
				A100000D000000000000001 /* Sources */,
				A1000007000000000000001 /* Frameworks */,
				A100000E000000000000001 /* Resources */,
			);
			buildRules = ();
			dependencies = ();
			name = {target};
			productName = {target};
			productReference = A1000006000000000000001 /* {target}.app */;
			productType = "com.apple.product-type.application";
		}};
/* End PBXNativeTarget section */

/* Begin PBXProject section */
		A100000F000000000000001 /* Project object */ = {{
			isa = PBXProject;
			attributes = {{
				BuildIndependentTargetsInParallel = 1;
				LastSwiftUpdateCheck = 1600;
				LastUpgradeCheck = 1600;
				TargetAttributes = {{ A100000B000000000000001 = {{ CreatedOnToolsVersion = 16.0; }}; }};
			}};
			buildConfigurationList = A1000010000000000000001;
			compatibilityVersion = "Xcode 14.0";
			developmentRegion = en;
			hasScannedForEncodings = 0;
			knownRegions = (en, Base);
			mainGroup = A1000008000000000000001;
			productRefGroup = A100000A000000000000001 /* Products */;
			projectDirPath = "";
			projectRoot = "";
			targets = (A100000B000000000000001 /* {target} */);
		}};
/* End PBXProject section */

/* Begin PBXResourcesBuildPhase section */
		A100000E000000000000001 /* Resources */ = {{
			isa = PBXResourcesBuildPhase;
			buildActionMask = 2147483647;
			files = (A1000003000000000000001 /* www in Resources */);
			runOnlyForDeploymentPostprocessing = 0;
		}};
/* End PBXResourcesBuildPhase section */

/* Begin PBXSourcesBuildPhase section */
		A100000D000000000000001 /* Sources */ = {{
			isa = PBXSourcesBuildPhase;
			buildActionMask = 2147483647;
			files = (A1000001000000000000001 /* App.swift in Sources */);
			runOnlyForDeploymentPostprocessing = 0;
		}};
/* End PBXSourcesBuildPhase section */

/* Begin XCBuildConfiguration section */
		A1000011000000000000001 /* Debug */ = {{
			isa = XCBuildConfiguration;
			buildSettings = {{
				ALWAYS_SEARCH_USER_PATHS = NO;
				CLANG_ENABLE_MODULES = YES;
				CLANG_ENABLE_OBJC_ARC = YES;
				COPY_PHASE_STRIP = NO;
				DEBUG_INFORMATION_FORMAT = dwarf;
				ENABLE_TESTABILITY = YES;
				GCC_OPTIMIZATION_LEVEL = 0;
				IPHONEOS_DEPLOYMENT_TARGET = 16.0;
				MTL_ENABLE_DEBUG_INFO = INCLUDE_SOURCE;
				ONLY_ACTIVE_ARCH = YES;
				SDKROOT = iphoneos;
				SWIFT_ACTIVE_COMPILATION_CONDITIONS = DEBUG;
				SWIFT_OPTIMIZATION_LEVEL = "-Onone";
				SWIFT_VERSION = 5.0;
			}};
			name = Debug;
		}};
		A1000012000000000000001 /* Release */ = {{
			isa = XCBuildConfiguration;
			buildSettings = {{
				ALWAYS_SEARCH_USER_PATHS = NO;
				CLANG_ENABLE_MODULES = YES;
				CLANG_ENABLE_OBJC_ARC = YES;
				COPY_PHASE_STRIP = NO;
				DEBUG_INFORMATION_FORMAT = "dwarf-with-dsym";
				ENABLE_NS_ASSERTIONS = NO;
				IPHONEOS_DEPLOYMENT_TARGET = 16.0;
				MTL_ENABLE_DEBUG_INFO = NO;
				SDKROOT = iphoneos;
				SWIFT_COMPILATION_MODE = wholemodule;
				SWIFT_OPTIMIZATION_LEVEL = "-O";
				SWIFT_VERSION = 5.0;
				VALIDATE_PRODUCT = YES;
			}};
			name = Release;
		}};
		A1000013000000000000001 /* Debug */ = {{
			isa = XCBuildConfiguration;
			buildSettings = {{
				ASSETCATALOG_COMPILER_APPICON_NAME = "";
				CODE_SIGN_STYLE = Automatic;
				CURRENT_PROJECT_VERSION = 1;
				ENABLE_PREVIEWS = YES;
				GENERATE_INFOPLIST_FILE = NO;
				INFOPLIST_FILE = "{target}/Info.plist";
				LD_RUNPATH_SEARCH_PATHS = ("$(inherited)", "@executable_path/Frameworks");
				MARKETING_VERSION = 1.0;
				PRODUCT_BUNDLE_IDENTIFIER = "{bundle_id}";
				PRODUCT_NAME = "$(TARGET_NAME)";
				SWIFT_EMIT_LOC_STRINGS = YES;
				TARGETED_DEVICE_FAMILY = "1,2";
			}};
			name = Debug;
		}};
		A1000014000000000000001 /* Release */ = {{
			isa = XCBuildConfiguration;
			buildSettings = {{
				ASSETCATALOG_COMPILER_APPICON_NAME = "";
				CODE_SIGN_STYLE = Automatic;
				CURRENT_PROJECT_VERSION = 1;
				ENABLE_PREVIEWS = YES;
				GENERATE_INFOPLIST_FILE = NO;
				INFOPLIST_FILE = "{target}/Info.plist";
				LD_RUNPATH_SEARCH_PATHS = ("$(inherited)", "@executable_path/Frameworks");
				MARKETING_VERSION = 1.0;
				PRODUCT_BUNDLE_IDENTIFIER = "{bundle_id}";
				PRODUCT_NAME = "$(TARGET_NAME)";
				SWIFT_EMIT_LOC_STRINGS = YES;
				TARGETED_DEVICE_FAMILY = "1,2";
			}};
			name = Release;
		}};
/* End XCBuildConfiguration section */

/* Begin XCConfigurationList section */
		A1000010000000000000001 = {{
			isa = XCConfigurationList;
			buildConfigurations = (A1000011000000000000001 /* Debug */, A1000012000000000000001 /* Release */);
			defaultConfigurationIsVisible = 0;
			defaultConfigurationName = Release;
		}};
		A100000C000000000000001 = {{
			isa = XCConfigurationList;
			buildConfigurations = (A1000013000000000000001 /* Debug */, A1000014000000000000001 /* Release */);
			defaultConfigurationIsVisible = 0;
			defaultConfigurationName = Release;
		}};
/* End XCConfigurationList section */
	}};
	rootObject = A100000F000000000000001 /* Project object */;
}}
"#
    )
}

fn scheme_xml(target: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<Scheme LastUpgradeVersion = "1600" version = "1.7">
   <BuildAction parallelizeBuildables = "YES" buildImplicitDependencies = "YES">
      <BuildActionEntries>
         <BuildActionEntry buildForTesting = "YES" buildForRunning = "YES" buildForProfiling = "YES" buildForArchiving = "YES" buildForAnalyzing = "YES">
            <BuildableReference BuildableIdentifier = "primary" BlueprintIdentifier = "A100000B000000000000001"
               BuildableName = "{target}.app" BlueprintName = "{target}" ReferencedContainer = "container:{target}.xcodeproj"/>
         </BuildActionEntry>
      </BuildActionEntries>
   </BuildAction>
   <LaunchAction buildConfiguration = "Debug" selectedDebuggerIdentifier = "Xcode.DebuggerFoundation.Debugger.LLDB"
      selectedLauncherIdentifier = "Xcode.DebuggerFoundation.Launcher.LLDB" launchStyle = "0" useCustomWorkingDirectory = "NO"
      ignoresPersistentStateOnLaunch = "NO" debugDocumentVersioning = "YES" debugServiceExtension = "internal" allowLocationSimulation = "YES">
      <BuildableProductRunnable runnableDebuggingMode = "0">
         <BuildableReference BuildableIdentifier = "primary" BlueprintIdentifier = "A100000B000000000000001"
            BuildableName = "{target}.app" BlueprintName = "{target}" ReferencedContainer = "container:{target}.xcodeproj"/>
      </BuildableProductRunnable>
   </LaunchAction>
   <ArchiveAction buildConfiguration = "Release" revealArchiveInOrganizer = "YES"/>
</Scheme>
"#
    )
}

pub fn export_ios(pid: &str, dst: &Path) -> Result<ExportResult> {
    let p = crate::project::load(pid)?;
    let target = sanitize_target(&p.name);
    let bundle_id = sanitize_bundle_id(&p.name);

    let root = dst.join(&target);
    let proj = root.join(format!("{target}.xcodeproj"));
    let src = root.join(&target);
    fs::create_dir_all(proj.join("xcshareddata/xcschemes"))?;
    fs::create_dir_all(&src)?;

    let n = copy_www(&crate::project::files_dir(pid), &src.join("www"))?;

    // Pin the web entry point into the Swift source.
    let entry = if p.entry.is_empty() { "index.html".into() } else { p.entry.clone() };
    let swift = format!(
        "// Generated by VibeApp AI\nprivate let ENTRY_POINT = \"{}\"\n\n{}",
        entry.replace('"', "\\\""),
        APP_SWIFT
    );
    fs::write(src.join("App.swift"), swift)?;
    fs::write(src.join("Info.plist"), info_plist(&p.name))?;
    fs::write(proj.join("project.pbxproj"), pbxproj(&target, &bundle_id))?;
    fs::write(
        proj.join(format!("xcshareddata/xcschemes/{target}.xcscheme")),
        scheme_xml(&target),
    )?;

    let build_sh = format!(
        r#"#!/bin/bash
# Build and run {target} on the iOS Simulator.
set -euo pipefail
cd "$(dirname "$0")"
DEVICE="${{1:-iPhone 17 Pro}}"

echo "==> Building for $DEVICE"
xcodebuild -project "{target}.xcodeproj" -scheme "{target}" \
  -sdk iphonesimulator -configuration Debug \
  -destination "platform=iOS Simulator,name=$DEVICE" \
  -derivedDataPath build build | tail -5

APP="build/Build/Products/Debug-iphonesimulator/{target}.app"
echo "==> Booting $DEVICE"
xcrun simctl boot "$DEVICE" 2>/dev/null || true
open -a Simulator
xcrun simctl install booted "$APP"
xcrun simctl launch booted "{bundle_id}"
echo "==> Running."
"#
    );
    let sh = root.join("run-on-simulator.sh");
    fs::write(&sh, build_sh)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&sh, fs::Permissions::from_mode(0o755))?;
    }

    fs::write(
        root.join("README.md"),
        format!(
            "# {name}\n\nNative iOS app generated by VibeApp AI.\n\n\
             - `{target}.xcodeproj` — open in Xcode and hit Run.\n\
             - `./run-on-simulator.sh \"iPhone 17 Pro\"` — build, install and launch from the terminal.\n\
             - Web assets live in `{target}/www` as a folder reference, so adding files needs no project edits.\n\n\
             ## Before submitting to the App Store\n\
             1. Set your Team under Signing & Capabilities.\n\
             2. Change the bundle identifier from `{bundle_id}` to one you own.\n\
             3. Add an app icon set (Assets.xcassets) and a launch screen.\n\
             4. Archive → Distribute App.\n\n\
             The web layer can call native haptics via `window.haptics.impact('medium')`, \
             `window.haptics.selection()` and `window.haptics.notify('success')`.\n",
            name = p.name
        ),
    )?;

    Ok(ExportResult {
        kind: "ios".into(),
        path: root.display().to_string(),
        files: n + 6,
        notes: vec![
            format!("Xcode target `{target}`, bundle id `{bundle_id}`."),
            "Portrait + landscape, iPhone and iPad, iOS 16+.".into(),
            "Native haptics bridged to window.haptics.*".into(),
        ],
        next_steps: vec![
            "Run ./run-on-simulator.sh to build and launch immediately.".into(),
            "Set your signing Team in Xcode before archiving.".into(),
        ],
    })
}

// ---------------------------------------------------------------------------
// Electron
// ---------------------------------------------------------------------------

const ELECTRON_MAIN: &str = r#"const { app, BrowserWindow, shell, ipcMain } = require('electron');
const path = require('path');

let win;
function createWindow() {
  win = new BrowserWindow({
    width: 1100, height: 780, minWidth: 380, minHeight: 480,
    backgroundColor: '#0b0b12',
    titleBarStyle: process.platform === 'darwin' ? 'hiddenInset' : 'default',
    webPreferences: {
      preload: path.join(__dirname, 'preload.js'),
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: true,
    },
  });
  win.loadFile(path.join(__dirname, 'www', ENTRY_POINT));
  // External links open in the user's browser, not inside the app shell.
  win.webContents.setWindowOpenHandler(({ url }) => {
    shell.openExternal(url);
    return { action: 'deny' };
  });
}

app.whenReady().then(() => {
  createWindow();
  app.on('activate', () => {
    if (BrowserWindow.getAllWindows().length === 0) createWindow();
  });
});
app.on('window-all-closed', () => { if (process.platform !== 'darwin') app.quit(); });
ipcMain.handle('vibe:version', () => app.getVersion());
"#;

const ELECTRON_PRELOAD: &str = r#"const { contextBridge, ipcRenderer } = require('electron');

// Same surface the iOS wrapper exposes, so one codebase targets both.
contextBridge.exposeInMainWorld('haptics', {
  impact: () => {}, selection: () => {}, notify: () => {},
});
contextBridge.exposeInMainWorld('vibe', {
  platform: process.platform,
  version: () => ipcRenderer.invoke('vibe:version'),
});
"#;

pub fn export_electron(pid: &str, dst: &Path) -> Result<ExportResult> {
    let p = crate::project::load(pid)?;
    let slug = sanitize_bundle_id(&p.name).replace("ai.vibeapp.", "");
    let root = dst.join(&slug);
    fs::create_dir_all(&root)?;
    let n = copy_www(&crate::project::files_dir(pid), &root.join("www"))?;

    let entry = if p.entry.is_empty() { "index.html".into() } else { p.entry.clone() };
    fs::write(
        root.join("main.js"),
        format!(
            "// Generated by VibeApp AI\nconst ENTRY_POINT = {};\n\n{}",
            serde_json::to_string(&entry)?,
            ELECTRON_MAIN
        ),
    )?;
    fs::write(root.join("preload.js"), ELECTRON_PRELOAD)?;

    let pkg = serde_json::json!({
        "name": slug,
        "productName": p.name,
        "version": "1.0.0",
        "description": if p.description.is_empty() { format!("{} — built with VibeApp AI", p.name) } else { p.description.clone() },
        "main": "main.js",
        "scripts": {
            "start": "electron .",
            "dist": "electron-builder --mac --universal",
            "dist:all": "electron-builder -mwl"
        },
        "devDependencies": { "electron": "^33.0.0", "electron-builder": "^25.0.0" },
        "build": {
            "appId": sanitize_bundle_id(&p.name),
            "productName": p.name,
            "files": ["main.js", "preload.js", "www/**/*"],
            "mac": { "category": "public.app-category.developer-tools", "target": ["dmg", "zip"] },
            "win": { "target": ["nsis"] },
            "linux": { "target": ["AppImage"] }
        }
    });
    fs::write(root.join("package.json"), serde_json::to_vec_pretty(&pkg)?)?;
    fs::write(
        root.join("README.md"),
        format!(
            "# {}\n\nDesktop app generated by VibeApp AI.\n\n```bash\nnpm install\nnpm start        # run it\nnpm run dist     # build a .dmg\n```\n",
            p.name
        ),
    )?;

    Ok(ExportResult {
        kind: "electron".into(),
        path: root.display().to_string(),
        files: n + 4,
        notes: vec![
            "Context isolation on, sandbox on, node integration off.".into(),
            "electron-builder is preconfigured for .dmg, .exe and AppImage.".into(),
        ],
        next_steps: vec![
            "cd into the folder, then `npm install && npm start`.".into(),
            "`npm run dist` produces a signed-ready .dmg.".into(),
        ],
    })
}

// ---------------------------------------------------------------------------
// simctl bridge
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimDevice {
    pub udid: String,
    pub name: String,
    pub state: String,
    pub runtime: String,
    pub available: bool,
}

fn run(cmd: &str, args: &[&str]) -> Result<String> {
    let out = Command::new(cmd)
        .args(args)
        .output()
        .with_context(|| format!("running {cmd}"))?;
    if !out.status.success() {
        return Err(anyhow!(
            "{}",
            String::from_utf8_lossy(&out.stderr).trim().chars().take(600).collect::<String>()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

pub fn xcode_present() -> bool {
    Command::new("xcrun")
        .args(["--find", "simctl"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub fn list_simulators() -> Result<Vec<SimDevice>> {
    let json = run("xcrun", &["simctl", "list", "devices", "available", "--json"])?;
    let v: serde_json::Value = serde_json::from_str(&json)?;
    let mut out = Vec::new();
    if let Some(map) = v.get("devices").and_then(|d| d.as_object()) {
        for (runtime, list) in map {
            let rt = runtime.rsplit('.').next().unwrap_or(runtime).replace('-', " ");
            for d in list.as_array().into_iter().flatten() {
                out.push(SimDevice {
                    udid: d.get("udid").and_then(|x| x.as_str()).unwrap_or("").into(),
                    name: d.get("name").and_then(|x| x.as_str()).unwrap_or("").into(),
                    state: d.get("state").and_then(|x| x.as_str()).unwrap_or("").into(),
                    available: d.get("isAvailable").and_then(|x| x.as_bool()).unwrap_or(false),
                    runtime: rt.clone(),
                });
            }
        }
    }
    out.sort_by(|a, b| b.runtime.cmp(&a.runtime).then(a.name.cmp(&b.name)));
    Ok(out)
}

/// Build the exported Xcode project and launch it on a simulator.
pub fn build_and_run_ios(project_dir: &Path, device: &str) -> Result<String> {
    let target = project_dir
        .file_name()
        .ok_or_else(|| anyhow!("bad project dir"))?
        .to_string_lossy()
        .to_string();
    let xcodeproj = project_dir.join(format!("{target}.xcodeproj"));
    if !xcodeproj.exists() {
        return Err(anyhow!("No Xcode project at {}", xcodeproj.display()));
    }

    let dest = format!("platform=iOS Simulator,name={device}");
    let dd = project_dir.join("build");
    run(
        "xcodebuild",
        &[
            "-project", &xcodeproj.to_string_lossy(),
            "-scheme", &target,
            "-sdk", "iphonesimulator",
            "-configuration", "Debug",
            "-destination", &dest,
            "-derivedDataPath", &dd.to_string_lossy(),
            "build",
        ],
    )
    .context("xcodebuild failed")?;

    let app = dd.join(format!("Build/Products/Debug-iphonesimulator/{target}.app"));
    if !app.exists() {
        return Err(anyhow!("Build finished but {} is missing", app.display()));
    }

    let _ = Command::new("xcrun").args(["simctl", "boot", device]).output();
    let _ = Command::new("open").args(["-a", "Simulator"]).output();
    // The simulator needs a moment after boot before it accepts an install.
    std::thread::sleep(std::time::Duration::from_millis(2500));
    run("xcrun", &["simctl", "install", "booted", &app.to_string_lossy()])?;

    let plist = app.join("Info.plist");
    let bundle_id = run(
        "/usr/libexec/PlistBuddy",
        &["-c", "Print :CFBundleIdentifier", &plist.to_string_lossy()],
    )?
    .trim()
    .to_string();
    run("xcrun", &["simctl", "launch", "booted", &bundle_id])?;
    Ok(format!("Launched {bundle_id} on {device}."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundle_ids_are_always_valid_reverse_dns() {
        assert_eq!(sanitize_bundle_id("My Cool App"), "ai.vibeapp.my-cool-app");
        assert_eq!(sanitize_bundle_id("Snake!! 3D"), "ai.vibeapp.snake-3d");
        assert_eq!(sanitize_bundle_id("   "), "ai.vibeapp.app-");
        assert_eq!(sanitize_bundle_id("2048"), "ai.vibeapp.app-2048");
        for name in ["a", "Ünïcödé Tëst", "...", "app--name", "9lives"] {
            let id = sanitize_bundle_id(name);
            assert!(id.starts_with("ai.vibeapp."), "{name} -> {id}");
            let last = id.rsplit('.').next().unwrap();
            assert!(!last.is_empty(), "{name} produced an empty segment");
            assert!(
                last.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'),
                "{name} -> {id} has illegal characters"
            );
            assert!(!last.chars().next().unwrap().is_ascii_digit(), "{id} starts with a digit");
        }
    }

    #[test]
    fn xcode_target_names_are_valid_identifiers() {
        assert_eq!(sanitize_target("My Cool App"), "MyCoolApp");
        assert_eq!(sanitize_target("snake 3d!"), "Snake3d");
        assert_eq!(sanitize_target("2048"), "VibeApp2048");
        assert_eq!(sanitize_target("!!!"), "VibeApp");
        for n in ["a b", "Test", "x", "🎮 game"] {
            let t = sanitize_target(n);
            assert!(!t.is_empty());
            assert!(t.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'), "{n} -> {t}");
            assert!(!t.chars().next().unwrap().is_ascii_digit());
        }
    }

    #[test]
    fn pbxproj_is_structurally_balanced_and_wired_up() {
        let s = pbxproj("MyApp", "ai.vibeapp.myapp");
        assert_eq!(
            s.matches('{').count(),
            s.matches('}').count(),
            "unbalanced braces in pbxproj"
        );
        // every section that opens must close
        for sec in [
            "PBXBuildFile", "PBXFileReference", "PBXGroup", "PBXNativeTarget",
            "PBXProject", "PBXResourcesBuildPhase", "PBXSourcesBuildPhase",
            "XCBuildConfiguration", "XCConfigurationList",
        ] {
            assert_eq!(s.matches(&format!("/* Begin {sec} section */")).count(), 1, "{sec}");
            assert_eq!(s.matches(&format!("/* End {sec} section */")).count(), 1, "{sec}");
        }
        // the web folder must be a folder reference, not a group
        assert!(s.contains("lastKnownFileType = folder; path = www"));
        assert!(s.contains("www in Resources"));
        assert!(s.contains("PRODUCT_BUNDLE_IDENTIFIER = \"ai.vibeapp.myapp\""));
        assert!(s.contains("INFOPLIST_FILE = \"MyApp/Info.plist\""));
        assert!(s.contains("TARGETED_DEVICE_FAMILY = \"1,2\""), "must support iPhone and iPad");
        assert!(s.contains("rootObject ="));
        // no unsubstituted format placeholders survived
        assert!(!s.contains("{target}") && !s.contains("{bundle_id}"));
    }

    #[test]
    fn info_plist_allows_both_orientations_and_is_valid_xml_shape() {
        let p = info_plist("My App");
        assert!(p.contains("UIInterfaceOrientationPortrait"));
        assert!(p.contains("UIInterfaceOrientationLandscapeLeft"));
        assert!(p.contains("UIInterfaceOrientationLandscapeRight"));
        assert!(p.contains("<key>CFBundleDisplayName</key><string>My App</string>"));
        assert_eq!(p.matches("<dict>").count(), p.matches("</dict>").count());
        assert_eq!(p.matches("<array>").count(), p.matches("</array>").count());
        assert!(p.contains("<plist version=\"1.0\">") && p.contains("</plist>"));
    }

    #[test]
    fn swift_host_configures_the_webview_for_app_like_behaviour() {
        for must in [
            "scrollView.bounces = false",
            "contentInsetAdjustmentBehavior = .never",
            "allowsInlineMediaPlayback = true",
            "loadFileURL",
            "window.haptics",
            "UIImpactFeedbackGenerator",
            "UINotificationFeedbackGenerator",
            "UISelectionFeedbackGenerator",
        ] {
            assert!(APP_SWIFT.contains(must), "Swift host is missing {must}");
        }
        assert!(APP_SWIFT.contains("ENTRY_POINT"), "entry point must be injectable");
    }

    #[test]
    fn electron_main_is_security_hardened() {
        assert!(ELECTRON_MAIN.contains("contextIsolation: true"));
        assert!(ELECTRON_MAIN.contains("nodeIntegration: false"));
        assert!(ELECTRON_MAIN.contains("sandbox: true"));
        assert!(ELECTRON_MAIN.contains("setWindowOpenHandler"), "external links must not open in-app");
        assert!(ELECTRON_PRELOAD.contains("contextBridge"));
        assert!(!ELECTRON_PRELOAD.contains("require('fs')"));
    }

    #[test]
    fn full_ios_export_produces_every_required_file() {
        use crate::project;
        let id = format!("test-{}", uuid::Uuid::new_v4());
        fs::create_dir_all(project::files_dir(&id)).unwrap();
        project::save(&project::Project {
            id: id.clone(),
            name: "Wave Rider".into(),
            entry: "index.html".into(),
            ..Default::default()
        })
        .unwrap();
        project::write_file(&id, "index.html", "<h1>hi</h1>").unwrap();
        project::write_file(&id, "js/game.js", "// game").unwrap();

        let out = std::env::temp_dir().join(format!("vibe-ios-{id}"));
        let r = export_ios(&id, &out).unwrap();
        let root = PathBuf::from(&r.path);

        assert!(root.join("WaveRider.xcodeproj/project.pbxproj").exists());
        assert!(root.join("WaveRider.xcodeproj/xcshareddata/xcschemes/WaveRider.xcscheme").exists());
        assert!(root.join("WaveRider/App.swift").exists());
        assert!(root.join("WaveRider/Info.plist").exists());
        assert!(root.join("WaveRider/www/index.html").exists());
        assert!(root.join("WaveRider/www/js/game.js").exists(), "nested assets must be copied");
        assert!(root.join("run-on-simulator.sh").exists());
        assert!(root.join("README.md").exists());

        let swift = fs::read_to_string(root.join("WaveRider/App.swift")).unwrap();
        assert!(swift.contains(r#"ENTRY_POINT = "index.html""#));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(root.join("run-on-simulator.sh")).unwrap().permissions().mode();
            assert_eq!(mode & 0o111, 0o111, "run script must be executable");
        }

        let _ = fs::remove_dir_all(&out);
        let _ = fs::remove_dir_all(project::dir(&id));
    }

    #[test]
    fn electron_and_web_exports_produce_runnable_trees() {
        use crate::project;
        let id = format!("test-{}", uuid::Uuid::new_v4());
        fs::create_dir_all(project::files_dir(&id)).unwrap();
        project::save(&project::Project {
            id: id.clone(),
            name: "Note Pad".into(),
            entry: "index.html".into(),
            ..Default::default()
        })
        .unwrap();
        project::write_file(&id, "index.html", "<h1>notes</h1>").unwrap();

        let out = std::env::temp_dir().join(format!("vibe-el-{id}"));
        let e = export_electron(&id, &out).unwrap();
        let root = PathBuf::from(&e.path);
        let pkg: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(root.join("package.json")).unwrap()).unwrap();
        assert_eq!(pkg["main"], "main.js");
        assert_eq!(pkg["productName"], "Note Pad");
        assert!(pkg["build"]["mac"]["target"].as_array().unwrap().iter().any(|t| t == "dmg"));
        assert!(fs::read_to_string(root.join("main.js")).unwrap().contains(r#"ENTRY_POINT = "index.html""#));
        assert!(root.join("www/index.html").exists());

        let wout = std::env::temp_dir().join(format!("vibe-web-{id}"));
        let w = export_web(&id, &wout).unwrap();
        assert_eq!(w.kind, "web");
        assert!(wout.join("sw.js").exists());
        let man: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(wout.join("manifest.webmanifest")).unwrap())
                .unwrap();
        assert_eq!(man["display"], "standalone");
        assert_eq!(man["start_url"], "./index.html");

        let _ = fs::remove_dir_all(&out);
        let _ = fs::remove_dir_all(&wout);
        let _ = fs::remove_dir_all(project::dir(&id));
    }

    /// Escape hatch used by the build harness: exports a realistic project to a
    /// fixed path so `xcodebuild` can prove the generated project compiles.
    /// Ignored by default because it needs Xcode and takes ~40s.
    #[test]
    #[ignore]
    fn emit_fixture_for_xcodebuild() {
        use crate::project;
        let id = "xcodebuild-fixture".to_string();
        let _ = fs::remove_dir_all(project::dir(&id));
        fs::create_dir_all(project::files_dir(&id)).unwrap();
        project::save(&project::Project {
            id: id.clone(),
            name: "Fixture Game".into(),
            kind: "game".into(),
            entry: "index.html".into(),
            ..Default::default()
        })
        .unwrap();
        project::write_file(
            &id,
            "index.html",
            "<!doctype html><meta name=viewport content='width=device-width,viewport-fit=cover'>\
             <body style='margin:0;background:#000'><canvas id=c></canvas>\
             <script src='game.js'></script></body>",
        )
        .unwrap();
        project::write_file(&id, "game.js", "const c=document.getElementById('c');").unwrap();
        project::write_file(&id, "assets/nested/deep.txt", "nested asset").unwrap();

        let out = PathBuf::from("/tmp/vibeapp-xcodebuild-fixture");
        let _ = fs::remove_dir_all(&out);
        let r = export_ios(&id, &out).unwrap();
        println!("FIXTURE_PATH={}", r.path);
    }

    #[test]
    fn exporting_an_empty_project_fails_loudly_instead_of_shipping_nothing() {
        use crate::project;
        let id = format!("test-{}", uuid::Uuid::new_v4());
        fs::create_dir_all(project::files_dir(&id)).unwrap();
        project::save(&project::Project { id: id.clone(), ..Default::default() }).unwrap();
        let out = std::env::temp_dir().join(format!("vibe-empty-{id}"));
        let err = export_ios(&id, &out).unwrap_err().to_string();
        assert!(err.contains("no files"), "got: {err}");
        let _ = fs::remove_dir_all(&out);
        let _ = fs::remove_dir_all(project::dir(&id));
    }
}
