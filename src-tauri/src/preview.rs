//! The `vibeapp://` URI scheme that serves a project's files to the preview
//! webview, plus the runtime shim injected into every HTML page.
//!
//! Serving over a real scheme (rather than `srcdoc`) means relative paths,
//! ES modules, fetch, canvas, audio and images all behave exactly as they will
//! in the shipped app.

use crate::project;

/// Injected at the top of every previewed page. Captures errors for the Doctor
/// panel, stubs the native haptics bridge, and reports element clicks so the
/// user can click-to-edit any part of their app.
pub const RUNTIME_SHIM: &str = r#"<script>(function(){
  if (window.__vibeShim) return; window.__vibeShim = 1;
  var send = function(type, payload){
    try { parent.postMessage({__vibe:1, type:type, payload:payload, path:location.pathname}, '*'); } catch(e){}
  };
  // The simulated Android Back button: the standard `backbutton` event first
  // (Cordova and Capacitor apps listen for it), then history, then Escape.
  window.addEventListener('message', function(e){
    if (!e.data || e.data.__vibeCmd !== 'back') return;
    var ev = new Event('backbutton', {cancelable: true});
    document.dispatchEvent(ev);
    if (ev.defaultPrevented) return;
    if (history.length > 1) { history.back(); return; }
    var k = {key: 'Escape', code: 'Escape', bubbles: true, cancelable: true};
    (document.activeElement || document.body || document).dispatchEvent(new KeyboardEvent('keydown', k));
  });
  // This script sits above the page's own code, so every line in the document
  // is pushed down by its length. Report the line as it is in the file.
  var here = location.href.split('#')[0].split('?')[0];
  var rel = function(u){
    try { return decodeURIComponent(new URL(u, location.href).pathname).replace(/^[/]/, ''); }
    catch(e){ return u || ''; }
  };

  // --- console + error capture (feeds the Logs and Doctor panels) ---
  ['log','info','warn','error','debug'].forEach(function(level){
    var orig = console[level];
    console[level] = function(){
      var args = [].slice.call(arguments).map(function(a){
        try {
          if (a instanceof Error) return a.stack || String(a);
          return typeof a === 'object' ? JSON.stringify(a) : String(a);
        } catch(e){ return String(a); }
      });
      send('console', {level: level, text: args.join(' '), at: Date.now()});
      try { orig.apply(console, arguments); } catch(e){}
    };
  });

  window.addEventListener('error', function(e){
    var t = e.target;
    if (t && t !== window && (t.src || t.href)) {
      send('error', {message: 'Could not load ' + rel(t.src || t.href), source: rel(here),
                     line: 0, col: 0, stack: '', at: Date.now()});
      return;
    }
    var file = (e.filename || '').split('#')[0].split('?')[0];
    var shift = (file === here) ? (window.__vibeLineShift || 0) : 0;
    send('error', {
      message: e.message || 'Script error',
      source: rel(e.filename || ''), line: Math.max(0, (e.lineno || 0) - shift), col: e.colno || 0,
      stack: (e.error && e.error.stack) || '', at: Date.now()
    });
  }, true);

  // --- blank screen: nothing visible once the page has settled ---
  window.addEventListener('load', function(){
    setTimeout(function(){
      try {
        var b = document.body;
        if (b && (b.innerText || '').trim()) return;
        var all = b ? b.getElementsByTagName('*') : [];
        for (var i = 0; i < all.length && i < 800; i++) {
          var n = all[i], r = n.getBoundingClientRect();
          if (r.width < 2 || r.height < 2) continue;
          if (/^(CANVAS|IMG|SVG|VIDEO|IFRAME|INPUT|BUTTON|SELECT|TEXTAREA)$/i.test(n.tagName)) return;
          var s = getComputedStyle(n);
          if (s.backgroundImage !== 'none') return;
          if (s.backgroundColor && !/rgba[(]0, 0, 0, 0[)]|transparent/.test(s.backgroundColor)) return;
        }
        send('blank', {at: Date.now()});
      } catch(e){}
    }, 2500);
  });

  window.addEventListener('unhandledrejection', function(e){
    var r = e.reason;
    send('error', {
      message: 'Unhandled promise rejection: ' + ((r && r.message) || r),
      stack: (r && r.stack) || '', source: '', line: 0, col: 0, at: Date.now()
    });
  });

  // --- native haptics bridge (no-op in preview, real on device) ---
  if (!window.haptics) {
    window.haptics = {
      impact: function(s){ send('haptic', {kind:'impact', style:s||'medium'}); if(navigator.vibrate) navigator.vibrate(s==='heavy'?24:s==='light'?6:12); },
      selection: function(){ send('haptic', {kind:'selection'}); if(navigator.vibrate) navigator.vibrate(4); },
      notify: function(s){ send('haptic', {kind:'notify', style:s||'success'}); if(navigator.vibrate) navigator.vibrate([12,40,12]); }
    };
  }

  // --- click-to-edit: hold ⌥ and click any element to target it in chat ---
  document.addEventListener('click', function(e){
    if (!e.altKey) return;
    e.preventDefault(); e.stopPropagation();
    var el = e.target, path = [], n = 0;
    while (el && el.nodeType === 1 && n++ < 6) {
      var seg = el.tagName.toLowerCase();
      if (el.id) { seg += '#' + el.id; path.unshift(seg); break; }
      if (el.className && typeof el.className === 'string') {
        seg += '.' + el.className.trim().split(/\s+/).slice(0,2).join('.');
      }
      path.unshift(seg); el = el.parentElement;
    }
    send('pick', {
      selector: path.join(' > '),
      text: (e.target.innerText || '').slice(0, 120),
      html: (e.target.outerHTML || '').slice(0, 400)
    });
  }, true);

  // --- network interception (feeds the Requests panel) ---
  var origFetch = window.fetch;
  if (origFetch) {
    window.fetch = function(input, init){
      var url = typeof input === 'string' ? input : (input && input.url) || String(input);
      var method = (init && init.method) || (input && input.method) || 'GET';
      var t0 = performance.now();
      var id = Math.random().toString(36).slice(2);
      send('net', {id:id, phase:'start', url:url, method:method, at:Date.now()});
      return origFetch.apply(this, arguments).then(function(res){
        send('net', {id:id, phase:'end', url:url, method:method, status:res.status,
                     ok:res.ok, ms:Math.round(performance.now()-t0)});
        return res;
      }).catch(function(err){
        send('net', {id:id, phase:'error', url:url, method:method, status:0,
                     ok:false, error:String(err), ms:Math.round(performance.now()-t0)});
        throw err;
      });
    };
  }
  var XO = XMLHttpRequest.prototype.open, XS = XMLHttpRequest.prototype.send;
  XMLHttpRequest.prototype.open = function(m, u){ this.__v = {m:m, u:u}; return XO.apply(this, arguments); };
  XMLHttpRequest.prototype.send = function(){
    var self = this, t0 = performance.now(), v = self.__v || {};
    var id = Math.random().toString(36).slice(2);
    send('net', {id:id, phase:'start', url:v.u||'', method:v.m||'GET', at:Date.now()});
    self.addEventListener('loadend', function(){
      send('net', {id:id, phase:'end', url:v.u||'', method:v.m||'GET', status:self.status,
                   ok:self.status>=200&&self.status<400, ms:Math.round(performance.now()-t0)});
    });
    return XS.apply(self, arguments);
  };

  // --- report first paint + viewport so the simulator can show real numbers ---
  var report = function(){
    send('vitals', {
      w: window.innerWidth, h: window.innerHeight,
      dpr: window.devicePixelRatio || 1,
      orientation: window.innerWidth > window.innerHeight ? 'landscape' : 'portrait',
      title: document.title || ''
    });
  };
  window.addEventListener('load', report);
  window.addEventListener('resize', report);
  setTimeout(report, 60);

  // --- frame-rate sampling for the game/perf readout ---
  var frames = 0, last = performance.now();
  (function loop(){
    frames++;
    var now = performance.now();
    if (now - last >= 1000) {
      send('fps', {fps: Math.round(frames * 1000 / (now - last))});
      frames = 0; last = now;
    }
    requestAnimationFrame(loop);
  })();
})();</script>
"#;

/// Insert the shim (and the project's env vars) as early as possible so it can
/// catch errors thrown by the page's own scripts.
pub fn inject(html: &str, env_script: &str) -> String {
    let shift = env_script.matches('\n').count() + RUNTIME_SHIM.matches('\n').count();
    let payload = format!("<script>window.__vibeLineShift={shift};</script>{env_script}{RUNTIME_SHIM}");
    let lower = html.to_lowercase();

    if let Some(i) = lower.find("<head>") {
        let mut out = html.to_string();
        out.insert_str(i + 6, &payload);
        return out;
    }
    if let Some(i) = lower.find("<head ") {
        if let Some(j) = html[i..].find('>') {
            let mut out = html.to_string();
            out.insert_str(i + j + 1, &payload);
            return out;
        }
    }
    if let Some(i) = lower.find("<body") {
        let mut out = html.to_string();
        out.insert_str(i, &payload);
        return out;
    }
    format!("{payload}{html}")
}

/// The simulated device's safe-area insets, passed as `?sat=&sar=&sab=&sal=`.
///
/// A cross-origin iframe can't have CSS injected from the parent, so the
/// emulated insets are baked in here instead. `env(safe-area-inset-*)` is
/// overridden with the real point values for the selected iPhone, which is
/// what makes the preview honest about notch and home-indicator clearance.
fn insets_css(query: &str) -> String {
    let mut vals = [("sat", "0"), ("sar", "0"), ("sab", "0"), ("sal", "0")]
        .map(|(k, d)| (k, d.to_string()));
    let mut any = false;
    for pair in query.trim_start_matches('?').split('&') {
        let Some((k, v)) = pair.split_once('=') else { continue };
        // Only accept plain integers — this value lands inside a stylesheet.
        if !v.is_empty() && v.len() <= 4 && v.chars().all(|c| c.is_ascii_digit()) {
            for slot in vals.iter_mut() {
                if slot.0 == k {
                    slot.1 = v.to_string();
                    any = true;
                }
            }
        }
    }
    if !any {
        return String::new();
    }
    let g = |k: &str| vals.iter().find(|s| s.0 == k).map(|s| s.1.clone()).unwrap_or_default();
    format!(
        "<style id=\"vibeapp-insets\">:root{{\
--sat:{t}px;--sar:{r}px;--sab:{b}px;--sal:{l}px;\
--safe-area-inset-top:{t}px;--safe-area-inset-right:{r}px;\
--safe-area-inset-bottom:{b}px;--safe-area-inset-left:{l}px}}\
@supports(padding:env(safe-area-inset-top)){{:root{{--sat:{t}px}}}}</style>\
<script>(function(){{var I={{top:{t},right:{r},bottom:{b},left:{l}}};\
window.__safeArea=I;\
/* Rewrite env(safe-area-inset-*) in author stylesheets to the emulated values. */\
var fix=function(){{try{{for(var i=0;i<document.styleSheets.length;i++){{var ss=document.styleSheets[i];var rules;\
try{{rules=ss.cssRules}}catch(e){{continue}}if(!rules)continue;\
for(var j=0;j<rules.length;j++){{var r=rules[j];if(!r.style)continue;\
for(var k=0;k<r.style.length;k++){{var p=r.style[k];var v=r.style.getPropertyValue(p);\
if(v&&v.indexOf('safe-area-inset')>-1){{\
var nv=v.replace(/env\\(\\s*safe-area-inset-(top|right|bottom|left)\\s*(,[^)]*)?\\)/g,\
function(_,side){{return I[side]+'px'}});\
r.style.setProperty(p,nv,r.style.getPropertyPriority(p))}}}}}}}}}}catch(e){{}}}};\
document.addEventListener('DOMContentLoaded',fix);window.addEventListener('load',fix);setTimeout(fix,50);setTimeout(fix,300);\
}})();</script>",
        t = g("sat"),
        r = g("sar"),
        b = g("sab"),
        l = g("sal")
    )
}

/// Resolve a `vibeapp://<project-id>/<path>?<query>` request to bytes on disk.
/// Returns (mime, body). HTML is served with the runtime shim injected.
/// The simulated operating system, passed as `?os=android`. Only known values
/// are accepted; the page then sees an Android phone's user agent, so code that
/// branches on the platform takes its Android path in the preview.
fn os_script(query: &str) -> String {
    let os = query.trim_start_matches('?').split('&').find_map(|p| p.strip_prefix("os="));
    match os {
        Some("android") => "<script>(function(){var ua='Mozilla/5.0 (Linux; Android 15; Pixel 9) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Mobile Safari/537.36';\
try{Object.defineProperty(navigator,'userAgent',{get:function(){return ua}});\
Object.defineProperty(navigator,'platform',{get:function(){return 'Linux armv8l'}});\
Object.defineProperty(navigator,'vendor',{get:function(){return 'Google Inc.'}});\
Object.defineProperty(navigator,'maxTouchPoints',{get:function(){return 5}});}catch(e){}\
document.documentElement.classList.add('vibe-android');window.__vibeOS='android';})();</script>".to_string(),
        _ => String::new(),
    }
}

pub fn serve_with_query(project_id: &str, path: &str, query: &str) -> (String, Vec<u8>) {
    let (mime, body) = serve(project_id, path);
    if !mime.starts_with("text/html") || query.is_empty() {
        return (mime, body);
    }
    let css = format!("{}{}", os_script(query), insets_css(query));
    if css.is_empty() {
        return (mime, body);
    }
    // Append after the shim so it wins over anything the page declares.
    match String::from_utf8(body) {
        Ok(s) => {
            let lower = s.to_lowercase();
            let out = match lower.find("</head>") {
                Some(i) => {
                    let mut o = s.clone();
                    o.insert_str(i, &css);
                    o
                }
                None => format!("{css}{s}"),
            };
            (mime, out.into_bytes())
        }
        Err(e) => (mime, e.into_bytes()),
    }
}

/// The project and file a preview request names. macOS and Linux send
/// `vibeapp://<project>/<path>`; Windows (WebView2) sends
/// `http://vibeapp.localhost/<project>/<path>`.
pub fn split_uri(host: &str, path: &str) -> (String, String) {
    if host.eq_ignore_ascii_case("vibeapp.localhost") || host.eq_ignore_ascii_case("vibeapp") {
        let rest = path.trim_start_matches('/');
        let (pid, file) = rest.split_once('/').unwrap_or((rest, ""));
        return (pid.to_string(), format!("/{file}"));
    }
    (host.to_string(), path.to_string())
}

/// Resolve a `vibeapp://<project-id>/<path>` request to bytes on disk.
/// Returns (mime, body). HTML is served with the runtime shim injected.
pub fn serve(project_id: &str, path: &str) -> (String, Vec<u8>) {
    let rel = if path.is_empty() || path == "/" {
        project::load(project_id)
            .map(|p| p.entry)
            .unwrap_or_else(|_| "index.html".into())
    } else {
        path.trim_start_matches('/').to_string()
    };
    let rel = urlencoding::decode(&rel).map(|s| s.to_string()).unwrap_or(rel);

    // An arena entry previews from its own copy. The safe_join below still
    // guards the path inside it.
    let (root, rel) = match crate::arena::route(project_id, &rel) {
        Some(found) => found,
        None => (project::live_dir(project_id), rel),
    };
    let full = match crate::state::safe_join(&root, &rel) {
        Ok(p) => p,
        Err(_) => return not_found(&rel),
    };

    let Ok(bytes) = std::fs::read(&full) else {
        return not_found(&rel);
    };

    let mime = project::mime_of(&rel);
    if mime.starts_with("text/html") {
        if let Ok(s) = std::str::from_utf8(&bytes) {
            let env = project::env_script(project_id);
            return (mime.to_string(), inject(s, &env).into_bytes());
        }
    }
    (mime.to_string(), bytes)
}

fn not_found(rel: &str) -> (String, Vec<u8>) {
    let safe: String = rel
        .chars()
        .map(|c| match c {
            '<' => "&lt;".to_string(),
            '>' => "&gt;".to_string(),
            '&' => "&amp;".to_string(),
            c => c.to_string(),
        })
        .collect();
    let body = format!(
        r#"<!doctype html><meta name="viewport" content="width=device-width,initial-scale=1,viewport-fit=cover">
<style>
html,body{{margin:0;height:100%;display:grid;place-items:center;background:#0b0b12;color:#8b8ba7;
font:15px/1.6 -apple-system,BlinkMacSystemFont,"SF Pro Text",system-ui,sans-serif;text-align:center;padding:24px}}
code{{color:#c9c9e8;background:#16162a;padding:3px 8px;border-radius:6px;font-size:13px}}
h2{{color:#e8e8f5;font-size:17px;margin:0 0 10px;font-weight:600}}
</style>
<div><h2>Nothing here yet</h2><p>No file at <code>{safe}</code>.</p>
<p style="opacity:.6;font-size:13px">Describe what you want to build and it will appear here.</p></div>"#
    );
    ("text/html; charset=utf-8".to_string(), body.into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shim_lands_inside_head_when_there_is_one() {
        let out = inject("<html><head><title>x</title></head><body>hi</body></html>", "");
        let head = out.find("<head>").unwrap();
        let title = out.find("<title>").unwrap();
        let shim = out.find("__vibeShim").unwrap();
        assert!(head < shim && shim < title, "shim must be the first thing in head");
        assert!(out.contains("<title>x</title>"));
        assert!(out.contains("hi"));
    }

    #[test]
    fn shim_is_injected_even_with_no_head_or_body() {
        assert!(inject("<div>x</div>", "").contains("__vibeShim"));
        assert!(inject("", "").contains("__vibeShim"));
        // attribute-bearing head tag is still handled
        let a = inject("<head lang=\"en\"><title>t</title></head>", "");
        assert!(a.find("__vibeShim").unwrap() < a.find("<title>").unwrap());
        // body-only document
        let b = inject("<body onload=\"go()\">x</body>", "");
        assert!(b.find("__vibeShim").unwrap() < b.find("<body").unwrap());
    }

    #[test]
    fn env_script_precedes_the_shim() {
        let out = inject("<head></head>", "<script>window.ENV={};</script>");
        assert!(out.find("window.ENV").unwrap() < out.find("__vibeShim").unwrap());
    }

    #[test]
    fn shim_covers_every_channel_the_panels_depend_on() {
        for hook in [
            "unhandledrejection",
            "addEventListener('error'",
            "console[level]",
            "window.haptics",
            "e.altKey",       // click-to-edit
            "requestAnimationFrame", // fps meter
            "window.fetch",   // network inspector
            "XMLHttpRequest.prototype.open",
            "orientation",
            "devicePixelRatio",
        ] {
            assert!(RUNTIME_SHIM.contains(hook), "runtime shim is missing {hook}");
        }
        assert!(RUNTIME_SHIM.contains("__vibeShim"), "must be idempotent-guarded");
    }

    #[test]
    fn preview_requests_resolve_on_every_platform() {
        assert_eq!(split_uri("p1", "/assets/a.js"), ("p1".to_string(), "/assets/a.js".to_string()));
        assert_eq!(split_uri("vibeapp.localhost", "/p1/assets/a.js"), ("p1".to_string(), "/assets/a.js".to_string()));
        assert_eq!(split_uri("vibeapp.localhost", "/p1"), ("p1".to_string(), "/".to_string()));
    }

    #[test]
    fn the_android_hint_is_strict_and_the_back_button_reaches_the_page() {
        assert!(os_script("?sat=48&os=android").contains("Android 15"));
        assert_eq!(os_script("?os=android<script>"), "");
        assert_eq!(os_script("?os=windows"), "");
        assert_eq!(os_script("?sat=1"), "");
        assert!(RUNTIME_SHIM.contains("__vibeCmd") && RUNTIME_SHIM.contains("'backbutton'") && RUNTIME_SHIM.contains("history.back()"));
    }

    #[test]
    fn inset_query_produces_css_only_for_valid_integers() {
        let css = insets_css("?sat=59&sar=0&sab=34&sal=0");
        assert!(css.contains("--sat:59px"));
        assert!(css.contains("--sab:34px"));
        assert!(css.contains("vibeapp-insets"));

        // no recognised params -> nothing injected
        assert_eq!(insets_css(""), "");
        assert_eq!(insets_css("?t=1735"), "");

        // non-numeric values are refused, so nothing can be smuggled into CSS
        let hostile = insets_css("?sat=1}</style><script>alert(1)</script>&sab=34");
        assert!(!hostile.contains("alert"), "value must not reach the stylesheet");
        // Exactly one style block and one script block — the template's own,
        // with nothing spliced in between by the hostile value.
        assert_eq!(hostile.matches("<style").count(), 1);
        assert_eq!(hostile.matches("<script").count(), 1);
        assert!(hostile.contains("--sat:0px"), "bad value falls back to 0");
        assert!(hostile.contains("--sab:34px"), "the valid sibling still applies");

        // absurdly long values are refused
        assert_eq!(insets_css("?sat=999999"), "");
    }

    #[test]
    fn serve_with_query_layers_insets_after_the_shim() {
        let id = format!("test-{}", uuid::Uuid::new_v4());
        std::fs::create_dir_all(project::files_dir(&id)).unwrap();
        project::save(&project::Project { id: id.clone(), ..Default::default() }).unwrap();
        project::write_file(&id, "index.html", "<html><head></head><body>x</body></html>").unwrap();

        let (_, b) = serve_with_query(&id, "index.html", "?sat=62&sab=34");
        let s = String::from_utf8(b).unwrap();
        assert!(s.contains("--sat:62px"));
        assert!(s.find("__vibeShim").unwrap() < s.find("vibeapp-insets").unwrap());

        // non-HTML must never be rewritten, whatever the query says
        project::write_file(&id, "a.css", "body{}").unwrap();
        let (m, b2) = serve_with_query(&id, "a.css", "?sat=62");
        assert!(m.starts_with("text/css"));
        assert_eq!(String::from_utf8(b2).unwrap(), "body{}");

        let _ = std::fs::remove_dir_all(project::dir(&id));
    }

    #[test]
    fn missing_files_return_a_readable_page_not_a_crash() {
        let (mime, body) = serve("no-such-project", "index.html");
        assert!(mime.starts_with("text/html"));
        let s = String::from_utf8(body).unwrap();
        assert!(s.contains("Nothing here yet"));
    }

    #[test]
    fn not_found_escapes_the_path_it_echoes_back() {
        let (_, body) = not_found("<img src=x onerror=alert(1)>");
        let s = String::from_utf8(body).unwrap();
        assert!(!s.contains("<img src=x"), "path must be HTML-escaped");
        assert!(s.contains("&lt;img"));
    }

    /// Writes pages through the real `inject`, to load in a real browser engine.
    /// `VIBE_FIXTURE_DIR=/tmp/x cargo test --lib emit_shim_fixtures -- --ignored`
    #[test]
    #[ignore]
    fn emit_shim_fixtures() {
        let dir = std::env::var("VIBE_FIXTURE_DIR").expect("VIBE_FIXTURE_DIR");
        std::fs::create_dir_all(&dir).unwrap();
        let pages = [
            ("throw", "<html><head>\n<title>t</title>\n</head>\n<body>\n<p>hi</p>\n<script>\nundefinedFn();\n</script>\n</body></html>"),
            ("syntax", "<html><head>\n</head>\n<body>\n<p>hi</p>\n<script>\nlet ok = 1;\nfunction broken( {\n</script>\n</body></html>"),
            ("blank", "<html><head></head><body><div></div></body></html>"),
            ("canvas", "<html><head></head><body><canvas width=300 height=300></canvas></body></html>"),
            ("cssart", "<html><head></head><body><div style='width:80px;height:80px;background:#f00'></div></body></html>"),
        ];
        for (name, html) in pages {
            std::fs::write(format!("{dir}/{name}.html"), inject(html, "")).unwrap();
        }
    }

    #[test]
    fn the_shim_is_told_exactly_how_far_it_pushed_the_page_down() {
        // Every reported line was off by this much until the shim subtracted it.
        let out = inject("<html><head>\n<title>x</title></head></html>", "");
        let want = RUNTIME_SHIM.matches('\n').count();
        assert!(out.contains(&format!("__vibeLineShift={want};")), "{}", &out[..80]);
        let original = "<html><head>\n<script>\nboom\n</script></head></html>";
        let served = inject(original, "");
        let line = served.lines().position(|l| l == "boom").unwrap() + 1;
        assert_eq!(line - want, 3, "subtracting the shift gives the line on disk");
    }

    #[test]
    fn env_lines_count_towards_the_shift_too() {
        let out = inject("<head></head>", "<script>\nvar e=1;\n</script>\n");
        let want = 3 + RUNTIME_SHIM.matches('\n').count();
        assert!(out.contains(&format!("__vibeLineShift={want};")));
    }

    #[test]
    fn the_insets_style_block_adds_no_lines_of_its_own() {
        // It goes in below the shim, so a newline here would shift lines again.
        assert!(!insets_css("sat=62&sab=34").contains('\n'));
    }

    #[test]
    fn an_arena_entry_is_served_from_its_own_copy_with_the_shim() {
        let id = format!("test-{}", uuid::Uuid::new_v4());
        std::fs::create_dir_all(project::files_dir(&id)).unwrap();
        project::write_file(&id, "index.html", "<head></head>project").unwrap();
        let dir = crate::arena::slot_dir(&id, "run1", "a");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("index.html"), "<head></head>entry").unwrap();
        let (_, body) = serve(&id, "/__arena/run1/a/index.html");
        let s = String::from_utf8(body).unwrap();
        assert!(s.contains("entry") && s.contains("__vibeShim"), "{}", &s[..60]);
        assert!(!s.contains("project"));
        let (_, escaped) = serve(&id, "/__arena/run1/a/../../../../files/index.html");
        assert!(String::from_utf8(escaped).unwrap().contains("Nothing here yet"));
        let _ = std::fs::remove_dir_all(project::dir(&id));
    }

    #[test]
    fn serve_reads_real_files_and_shims_only_html() {
        let id = format!("test-{}", uuid::Uuid::new_v4());
        std::fs::create_dir_all(project::files_dir(&id)).unwrap();
        project::save(&project::Project {
            id: id.clone(),
            entry: "index.html".into(),
            ..Default::default()
        })
        .unwrap();
        project::write_file(&id, "index.html", "<html><head></head><body>hello</body></html>").unwrap();
        project::write_file(&id, "app.js", "console.log(1)").unwrap();

        let (m, b) = serve(&id, "index.html");
        assert!(m.starts_with("text/html"));
        let s = String::from_utf8(b).unwrap();
        assert!(s.contains("hello") && s.contains("__vibeShim"));

        // JS must be served verbatim — injecting into it would break the file
        let (m2, b2) = serve(&id, "app.js");
        assert!(m2.starts_with("text/javascript"));
        assert_eq!(String::from_utf8(b2).unwrap(), "console.log(1)");

        // empty path resolves to the project entry
        let (_, b3) = serve(&id, "");
        assert!(String::from_utf8(b3).unwrap().contains("hello"));

        // traversal is refused
        let (_, b4) = serve(&id, "../../../etc/passwd");
        assert!(String::from_utf8(b4).unwrap().contains("Nothing here yet"));

        let _ = std::fs::remove_dir_all(project::dir(&id));
    }
}
