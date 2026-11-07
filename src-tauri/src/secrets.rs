//! Credential scanner for generated project files.
//!
//! People paste a real API key into a chat window to "just test it", and the
//! model writes it straight into `index.html`. That file then goes into a zip,
//! an Xcode project, a GitHub repo, or a published PWA, and the key is public
//! forever. Catching it before export is the difference between an awkward
//! moment and a rotated credential.
//!
//! **Nothing here ever returns a secret.** A hit carries a masked preview —
//! enough to recognise which key it is, never enough to use it. A scanner that
//! logs the thing it found is worse than no scanner, because now the secret is
//! in a log file too.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    OpenAiKey,
    AnthropicKey,
    AwsAccessKey,
    GitHubToken,
    GoogleApiKey,
    SlackToken,
    BearerToken,
    PrivateKey,
    AssignedSecret,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::OpenAiKey => "an OpenAI-style API key",
            Kind::AnthropicKey => "an Anthropic API key",
            Kind::AwsAccessKey => "an AWS access key id",
            Kind::GitHubToken => "a GitHub token",
            Kind::GoogleApiKey => "a Google API key",
            Kind::SlackToken => "a Slack token",
            Kind::BearerToken => "a bearer token",
            Kind::PrivateKey => "a private key block",
            Kind::AssignedSecret => "a credential assigned to a variable",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Hit {
    pub path: String,
    /// 1-indexed.
    pub line: usize,
    pub kind: Kind,
    /// First four and last two characters only. Never the whole value.
    pub preview: String,
    pub message: String,
}

/// Show just enough to identify which key this is.
///
/// Four leading characters keeps the recognisable prefix (`sk-p`, `AKIA`,
/// `ghp_`); two trailing characters let someone match it against a list they
/// hold. The middle is never revealed at any length.
pub fn mask(secret: &str) -> String {
    let chars: Vec<char> = secret.chars().collect();
    if chars.len() <= 8 {
        return "•".repeat(chars.len().max(3));
    }
    let head: String = chars[..4].iter().collect();
    let tail: String = chars[chars.len() - 2..].iter().collect();
    format!("{head}…{tail} ({} chars)", chars.len())
}

/// Values that look like credentials but are placeholders.
///
/// This list is the whole reason the scanner is usable. Generated code is full
/// of `YOUR_API_KEY` and `process.env.OPENAI_KEY`, and a scanner that cries
/// wolf on those gets switched off within a day — after which it catches
/// nothing at all.
fn is_placeholder(v: &str) -> bool {
    let t = v.trim();
    if t.len() < 8 {
        return true;
    }
    let low = t.to_ascii_lowercase();

    // Anything read from the environment is by definition not a literal.
    if low.contains("process.env")
        || low.contains("import.meta.env")
        || low.starts_with("env.")
        || low.contains("os.environ")
        || low.contains("getenv")
    {
        return true;
    }
    // Angle-bracket and brace placeholders: <your key>, {{API_KEY}}, ${KEY}
    if (t.starts_with('<') && t.ends_with('>'))
        || t.contains("{{")
        || t.contains("${")
    {
        return true;
    }
    for marker in [
        "your", "yourkey", "example", "placeholder", "changeme", "change_me",
        "todo", "fixme", "insert", "replace", "dummy", "sample", "test_key",
        "xxxx", "abc123", "123456", "paste", "here", "redacted", "none",
        "my-key", "my_key", "apikey", "api-key", "api_key", "secret", "token",
    ] {
        if low.contains(marker) {
            return true;
        }
    }
    // All one character, or no variety at all — never a real key.
    let distinct: std::collections::HashSet<char> = t.chars().collect();
    if distinct.len() <= 3 {
        return true;
    }
    // A real credential mixes cases or includes digits. `averylongsentence`
    // does not.
    let has_digit = t.chars().any(|c| c.is_ascii_digit());
    let has_upper = t.chars().any(|c| c.is_ascii_uppercase());
    !has_digit && !has_upper
}

fn is_key_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// Longest run of key-ish characters starting at `from`.
fn token_at(line: &str, from: usize) -> &str {
    let bytes = line.as_bytes();
    let mut end = from;
    while end < bytes.len() && is_key_char(bytes[end] as char) {
        end += 1;
    }
    &line[from..end]
}

fn count_matching(s: &str, f: impl Fn(char) -> bool) -> usize {
    s.chars().filter(|c| f(*c)).count()
}

/// Scan one file. An empty result means nothing that is definitely a
/// credential was found — not that the file is guaranteed clean.
pub fn scan(path: &str, src: &str) -> Vec<Hit> {
    let mut out: Vec<Hit> = Vec::new();

    for (n, raw) in src.lines().enumerate() {
        let line = raw;
        let lineno = n + 1;
        // Builds a hit rather than pushing one, so the PEM branch below can
        // also append without two live mutable borrows of `out`.
        let make = |kind: Kind, value: &str| -> Option<Hit> {
            if is_placeholder(value) {
                return None;
            }
            Some(Hit {
                path: path.to_string(),
                line: lineno,
                kind,
                preview: mask(value),
                message: format!(
                    "{} is written into this file. Move it to the Keys panel or an \
                     environment variable, then rotate this one — anything committed or \
                     exported is public.",
                    kind.label()
                ),
            })
        };
        let mut found: Vec<Hit> = Vec::new();

        // ---- prefixed vendor keys -------------------------------------
        for (needle, kind, min_len) in [
            ("sk-ant-", Kind::AnthropicKey, 24usize),
            ("sk-", Kind::OpenAiKey, 24),
            ("AKIA", Kind::AwsAccessKey, 20),
            ("ghp_", Kind::GitHubToken, 36),
            ("gho_", Kind::GitHubToken, 36),
            ("ghs_", Kind::GitHubToken, 36),
            ("AIza", Kind::GoogleApiKey, 35),
            ("xoxb-", Kind::SlackToken, 24),
            ("xoxp-", Kind::SlackToken, 24),
        ] {
            let mut from = 0usize;
            while let Some(rel) = line[from..].find(needle) {
                let start = from + rel;
                let tok = token_at(line, start);
                // `sk-ant-` is also matched by `sk-`; the longer prefix wins,
                // so skip the generic one when the specific one applies.
                let shadowed = needle == "sk-" && tok.starts_with("sk-ant-");
                if !shadowed && tok.len() >= min_len {
                    found.extend(make(kind, tok));
                }
                from = start + needle.len();
            }
        }

        // ---- PEM block -------------------------------------------------
        if line.contains("-----BEGIN") && line.contains("PRIVATE KEY-----") {
            found.push(Hit {
                path: path.to_string(),
                line: lineno,
                kind: Kind::PrivateKey,
                preview: "-----BEGIN … PRIVATE KEY-----".into(),
                message: "A private key block is embedded in this file. It must never ship \
                          inside an app bundle — anyone can extract it."
                    .into(),
            });
        }

        // ---- Authorization: Bearer <token> -----------------------------
        if let Some(i) = line.to_ascii_lowercase().find("bearer ") {
            let after = i + "bearer ".len();
            if after < line.len() {
                let tok = token_at(line, after);
                let entropy = count_matching(tok, |c| c.is_ascii_digit())
                    + count_matching(tok, |c| c.is_ascii_uppercase());
                if tok.len() >= 40 && entropy >= 4 {
                    found.extend(make(Kind::BearerToken, tok));
                }
            }
        }

        // ---- key = "literal" -------------------------------------------
        // Only fires when the *name* says credential and the value is a quoted
        // literal, so `const apiUrl = "https://…"` cannot trip it.
        let low = line.to_ascii_lowercase();
        let names = ["api_key", "apikey", "api-key", "secret", "password", "passwd", "token"];
        if names.iter().any(|k| low.contains(k)) {
            for quote in ['"', '\''] {
                let mut from = 0usize;
                while let Some(rel) = line[from..].find(quote) {
                    let open = from + rel + 1;
                    let Some(close_rel) = line[open..].find(quote) else { break };
                    let value = &line[open..open + close_rel];
                    // The name must appear *before* the literal, or this is a
                    // string that merely mentions the word.
                    let before = line[..open].to_ascii_lowercase();
                    let named = names.iter().any(|k| before.contains(k));
                    let assigned = before.contains('=') || before.contains(':');
                    if named
                        && assigned
                        && value.len() >= 12
                        && value.chars().all(is_key_char)
                        && !is_placeholder(value)
                    {
                        found.extend(make(Kind::AssignedSecret, value));
                    }
                    from = open + close_rel + 1;
                }
            }
        }
        out.extend(found);
    }

    // One line can match several patterns; report each line once.
    out.dedup_by(|a, b| a.line == b.line && a.kind == b.kind);
    out
}

/// Scan a whole project.
pub fn scan_many(files: &[(String, String)]) -> Vec<Hit> {
    let mut out = Vec::new();
    for (path, src) in files {
        out.extend(scan(path, src));
        if out.len() >= 50 {
            break;
        }
    }
    out
}

/// One sentence per hit, for the export blocker. `None` means it is safe to
/// ship as far as this check can tell.
pub fn blocker(hits: &[Hit]) -> Option<String> {
    if hits.is_empty() {
        return None;
    }
    let mut s = String::from(
        "Credentials were found in files that are about to leave this machine. \
         Remove them before exporting, and rotate anything real.\n\n",
    );
    for h in hits.iter().take(20) {
        s.push_str(&format!("- {}:{} — {} ({})\n", h.path, h.line, h.kind.label(), h.preview));
    }
    Some(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_real_looking_openai_key_is_caught() {
        let hits = scan("app.js", concat!("const c = new OpenAI({apiKey: 'sk-proj-", "9Xk2Lm4Qw8Rt5Yu1Ip3Az6Bc7Df0Gh'});"));
        assert!(hits.iter().any(|h| h.kind == Kind::OpenAiKey), "{hits:?}");
    }

    #[test]
    fn the_secret_itself_is_never_returned() {
        // The whole point: a scanner that echoes the key has leaked it again.
        let secret = concat!("sk-proj-", "9Xk2Lm4Qw8Rt5Yu1Ip3Az6Bc7Df0Gh");
        let hits = scan("a.js", &format!("key = '{secret}'"));
        assert!(!hits.is_empty());
        for h in &hits {
            assert!(!h.preview.contains(secret), "the full secret leaked into the preview");
            assert!(!h.message.contains(secret), "the full secret leaked into the message");
            assert!(h.preview.len() < secret.len(), "{}", h.preview);
        }
    }

    #[test]
    fn a_mask_shows_the_prefix_and_a_tail_and_nothing_between() {
        let m = mask(concat!("sk-proj-", "abcdefghijklmnop"));
        assert!(m.starts_with("sk-p"), "{m}");
        assert!(m.ends_with("(24 chars)"), "{m}");
        assert!(!m.contains("defghijk"), "{m}");
    }

    #[test]
    fn a_short_value_is_masked_entirely() {
        assert!(!mask("abc123").contains('a'));
    }

    #[test]
    fn an_anthropic_key_is_not_double_reported_as_an_openai_key() {
        let hits = scan("a.js", concat!("K='sk-ant-", "api03-Xy9Zt4Rw8Qm2Nb6Vc1Ld0Ks7Jh5Gf3'"));
        let kinds: Vec<Kind> = hits.iter().map(|h| h.kind).collect();
        assert!(kinds.contains(&Kind::AnthropicKey), "{hits:?}");
        assert!(!kinds.contains(&Kind::OpenAiKey), "the generic sk- prefix must not double-fire");
    }

    #[test]
    fn aws_github_google_and_slack_shapes_are_each_recognised() {
        assert_eq!(scan("a", concat!("id=AKIA", "2X4YQ7ZLMN8PQR3T"))[0].kind, Kind::AwsAccessKey);
        assert_eq!(scan("a", concat!("t=ghp_", "A1b2C3d4E5f6G7h8I9j0K1l2M3n4O5p6Q7r8"))[0].kind, Kind::GitHubToken);
        assert_eq!(scan("a", concat!("k=AIza", "SyD3xK9Lm2Qw8Rt5Yu1Ip4Az6Bc7Df0Gh9J"))[0].kind, Kind::GoogleApiKey);
        // Assembled at runtime so the fixture is not itself a token in the
        // source: GitHub push protection rejects the literal.
        let slack = format!("s={}-2934871-A9k2Lm4Qw8Rt5Yu1Ip3", "xoxb");
        assert_eq!(scan("a", &slack)[0].kind, Kind::SlackToken);
    }

    #[test]
    fn a_private_key_block_is_reported_without_any_of_its_body() {
        let hits = scan("k.pem", concat!("-----BEGIN RSA ", "PRIVATE KEY-----"));
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].kind, Kind::PrivateKey);
        assert!(hits[0].preview.contains("BEGIN"));
    }

    #[test]
    fn an_assigned_credential_literal_is_caught() {
        let hits = scan("c.js", "const password = \"Hunter2Hunter2Hunter2\";");
        assert!(hits.iter().any(|h| h.kind == Kind::AssignedSecret), "{hits:?}");
    }

    // ---- the false positives that decide whether anyone keeps it on -----

    #[test]
    fn an_environment_lookup_is_never_a_hardcoded_secret() {
        for src in [
            "const key = process.env.OPENAI_API_KEY;",
            "const key = import.meta.env.VITE_API_KEY;",
            "api_key = os.environ.get('API_KEY')",
        ] {
            assert!(scan("a.js", src).is_empty(), "{src}");
        }
    }

    #[test]
    fn obvious_placeholders_are_never_reported() {
        for src in [
            "const apiKey = 'YOUR_API_KEY_HERE';",
            "const apiKey = '<your key>';",
            "const token = 'xxxxxxxxxxxxxxxx';",
            "const secret = 'changeme12345';",
            "const apiKey = '{{API_KEY}}';",
            "const apiKey = `${API_KEY}`;",
            "password: 'example-password'",
        ] {
            assert!(scan("a.js", src).is_empty(), "{src} should be a placeholder");
        }
    }

    #[test]
    fn ordinary_generated_code_is_left_completely_alone() {
        // This is close to what this app writes every day. One false positive
        // here and the user turns the scanner off for good.
        let src = r#"
<meta name="viewport" content="width=device-width, initial-scale=1">
const API_URL = "https://api.example.com/v1/items";
const tokenizer = new Tokenizer({ mode: 'word' });
function refreshToken() { return fetch(API_URL).then(r => r.json()); }
localStorage.setItem('myapp:v1', JSON.stringify(state));
// The password field must have autocomplete="current-password"
<input type="password" id="pw" autocomplete="current-password">
const secretSauce = computeLayout(width, height);
"#;
        assert_eq!(scan("index.html", src), vec![], "clean code must pass untouched");
    }

    #[test]
    fn a_url_assigned_to_an_api_variable_is_not_a_secret() {
        assert!(scan("a.js", "const apiUrl = 'https://api.example.com/v1';").is_empty());
    }

    #[test]
    fn a_bearer_header_with_a_real_token_fires_but_a_short_one_does_not() {
        let long = scan("a.js", concat!("headers: {Authorization: 'Bearer eyJ", "0A1b2C3d4E5f6G7h8I9j0K1l2M3n4O5p6Q7r8S9t0'}"));
        assert!(long.iter().any(|h| h.kind == Kind::BearerToken), "{long:?}");
        assert!(scan("a.js", "'Bearer abc'").is_empty());
    }

    #[test]
    fn a_line_number_points_at_the_offending_line() {
        let src = concat!("line one\nline two\nconst k = 'sk-proj-", "9Xk2Lm4Qw8Rt5Yu1Ip3Az6Bc';\n");
        assert_eq!(scan("a.js", src)[0].line, 3);
    }

    #[test]
    fn nothing_found_means_no_export_blocker() {
        assert!(blocker(&[]).is_none());
    }

    #[test]
    fn the_export_blocker_lists_each_hit_without_any_secret_in_it() {
        let secret = concat!("sk-proj-", "9Xk2Lm4Qw8Rt5Yu1Ip3Az6Bc7Df0Gh");
        let hits = scan("a.js", &format!("key='{secret}'"));
        let b = blocker(&hits).unwrap();
        assert!(b.contains("a.js:1"));
        assert!(b.contains("rotate"));
        assert!(!b.contains(secret), "the blocker text leaked the secret");
    }

    #[test]
    fn scanning_many_files_stops_before_it_floods() {
        let files: Vec<(String, String)> = (0..80)
            .map(|i| (format!("f{i}.js"), concat!("k='sk-proj-", "9Xk2Lm4Qw8Rt5Yu1Ip3Az6Bc7'").to_string()))
            .collect();
        assert!(scan_many(&files).len() <= 60);
    }
}
