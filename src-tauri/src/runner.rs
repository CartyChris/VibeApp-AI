//! Driving one model until its reply is actually finished.
//!
//! A single request is not a turn. Output can be cut off, and a model can ask to
//! read a file before it edits. `chat_send` has always handled both, but the
//! swarm and the arena made one request and took whatever came back, so a long
//! file from a builder arrived unterminated and was silently not applied.

use crate::agent;
use crate::providers::{self, ChatMessage, ChatOptions, Chunk, ProviderError, TokenUsage};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

#[derive(Debug, Default)]
pub struct Driven {
    /// The final reply. Retrieval rounds are not part of it.
    pub text: String,
    pub usage: TokenUsage,
    pub continuations: u32,
    pub retrievals: u32,
    /// Set if a request failed. `text` still holds whatever arrived before it.
    pub error: Option<ProviderError>,
}

impl Driven {
    /// True when a file block never closed, so it must not be treated as done.
    pub fn unfinished(&self) -> bool {
        agent::has_open_block(&self.text)
    }
}

/// Stream until the reply is complete or something fails.
///
/// `read` answers `<vibe:read>` requests, and `on_text` sees text as it arrives.
pub async fn drive(
    http: &reqwest::Client,
    key: &str,
    start: &[ChatMessage],
    opts: &ChatOptions,
    cancel: Arc<AtomicBool>,
    read: impl Fn(&[String]) -> (Vec<(String, String)>, Vec<String>),
    mut on_text: impl FnMut(&str),
) -> Driven {
    let mut msgs = start.to_vec();
    let mut out = Driven::default();
    loop {
        let mut turn = msgs.clone();
        if !out.text.is_empty() {
            turn.push(ChatMessage { role: "assistant".into(), content: out.text.clone(), images: vec![] });
            turn.push(ChatMessage { role: "user".into(), content: agent::CONTINUE_PROMPT.into(), images: vec![] });
        }
        let text = &mut out.text;
        let res = providers::stream_chat_resilient(http, key, &turn, opts, cancel.clone(), |c| {
            if let Chunk::Text(t) = c {
                text.push_str(&t);
                on_text(&t);
            }
        })
        .await;

        match res {
            Err(e) => {
                out.error = Some(e);
                return out;
            }
            Ok(st) => {
                out.usage.prompt_tokens += st.usage.prompt_tokens;
                out.usage.completion_tokens += st.usage.completion_tokens;
                out.usage.cached_tokens += st.usage.cached_tokens;

                let needs_more = st.truncated() || agent::has_open_block(&out.text);
                if needs_more && out.continuations < agent::MAX_CONTINUATIONS {
                    out.continuations += 1;
                    continue;
                }
                let parsed = agent::parse_turn(&out.text);
                // Only when it made no edits: the reply is discarded below, and
                // discarding edits to answer a read would lose them.
                if !needs_more
                    && parsed.ops.is_empty()
                    && !parsed.reads.is_empty()
                    && out.retrievals < agent::MAX_RETRIEVALS
                {
                    out.retrievals += 1;
                    let (found, missing) = read(&parsed.reads);
                    msgs.push(ChatMessage { role: "assistant".into(), content: out.text.clone(), images: vec![] });
                    msgs.push(ChatMessage {
                        role: "user".into(),
                        content: agent::retrieval_reply(&found, &missing),
                        images: vec![],
                    });
                    out.text.clear();
                    continue;
                }
                return out;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    //! These run the real HTTP and SSE path against a local mock provider, so
    //! continuation and retrieval are exercised end to end at no cost.
    use super::*;
    use std::sync::Mutex;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    fn sse(text: &str, finish: &str) -> String {
        let mut body = String::new();
        // Split so a word boundary is not assumed to line up with a frame.
        for part in text.as_bytes().chunks(11) {
            let piece = String::from_utf8_lossy(part);
            let frame = serde_json::json!({"choices":[{"delta":{"content":piece}}]});
            body.push_str(&format!("data: {frame}\n\n"));
        }
        let end = serde_json::json!({
            "choices":[{"delta":{},"finish_reason":finish}],
            "usage":{"prompt_tokens":10,"completion_tokens":5}
        });
        body.push_str(&format!("data: {end}\n\ndata: [DONE]\n\n"));
        format!(
            "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\ncontent-length: {}\r\n\r\n{body}",
            body.len()
        )
    }

    /// Serves canned responses in order, and records each request body.
    async fn mock(replies: Vec<String>) -> (u16, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        tokio::spawn(async move {
            for reply in replies {
                let Ok((mut sock, _)) = listener.accept().await else { return };
                let mut buf = Vec::new();
                let mut tmp = [0u8; 8192];
                loop {
                    let n = sock.read(&mut tmp).await.unwrap_or(0);
                    if n == 0 { break; }
                    buf.extend_from_slice(&tmp[..n]);
                    let text = String::from_utf8_lossy(&buf).to_string();
                    if let Some(h) = text.find("\r\n\r\n") {
                        let want = text[..h].lines()
                            .find_map(|l| l.to_lowercase().strip_prefix("content-length:")
                                .map(|v| v.trim().parse::<usize>().unwrap_or(0)))
                            .unwrap_or(0);
                        if buf.len() >= h + 4 + want {
                            log.lock().unwrap().push(text[h + 4..].to_string());
                            break;
                        }
                    }
                }
                let _ = sock.write_all(reply.as_bytes()).await;
                let _ = sock.shutdown().await;
            }
        });
        (port, seen)
    }

    fn opts(port: u16) -> ChatOptions {
        ChatOptions { model: "m".into(), route: format!("mock:{port}"), ..Default::default() }
    }

    fn user(t: &str) -> Vec<ChatMessage> {
        vec![ChatMessage { role: "user".into(), content: t.into(), images: vec![] }]
    }

    fn no_read(_: &[String]) -> (Vec<(String, String)>, Vec<String>) {
        (vec![], vec![])
    }

    async fn run(port: u16, read: impl Fn(&[String]) -> (Vec<(String, String)>, Vec<String>)) -> Driven {
        let http = reqwest::Client::new();
        let cancel = Arc::new(AtomicBool::new(false));
        drive(&http, "", &user("go"), &opts(port), cancel, read, |_| {}).await
    }

    #[tokio::test]
    async fn a_finished_reply_takes_one_request() {
        let (port, seen) = mock(vec![sse("<vibe:write path=\"a.js\">x</vibe:write>", "stop")]).await;
        let d = run(port, no_read).await;
        assert!(d.error.is_none(), "{:?}", d.error);
        assert_eq!(seen.lock().unwrap().len(), 1);
        assert_eq!(agent::parse_turn(&d.text).ops.len(), 1);
    }

    #[tokio::test]
    async fn a_file_cut_off_mid_block_is_continued_and_reassembled_into_one_write() {
        let (port, seen) = mock(vec![
            sse("<vibe:write path=\"a.js\">let a", "length"),
            sse(" = 1;</vibe:write>", "stop"),
        ]).await;
        let d = run(port, no_read).await;
        assert_eq!(d.continuations, 1);
        let parsed = agent::parse_turn(&d.text);
        assert_eq!(parsed.ops.len(), 1, "{}", d.text);
        match &parsed.ops[0] {
            agent::FileOp::Write { content, .. } => assert_eq!(content, "let a = 1;"),
            o => panic!("{o:?}"),
        }
        // The second request must carry what was written so far, and say continue.
        let second = seen.lock().unwrap()[1].clone();
        assert!(second.contains("let a"), "{second}");
        assert!(second.contains("cut off"), "{second}");
    }

    #[tokio::test]
    async fn a_provider_that_says_stop_with_a_block_still_open_is_not_believed() {
        let (port, seen) = mock(vec![
            sse("<vibe:write path=\"a.js\">half", "stop"),
            sse(" done</vibe:write>", "stop"),
        ]).await;
        let d = run(port, no_read).await;
        assert_eq!(seen.lock().unwrap().len(), 2);
        assert_eq!(agent::parse_turn(&d.text).ops.len(), 1);
    }

    #[tokio::test]
    async fn a_read_request_is_answered_and_the_model_then_edits() {
        let (port, seen) = mock(vec![
            sse("I need it. <vibe:read path=\"src/game.js\" />", "stop"),
            sse("<vibe:replace path=\"src/game.js\"><vibe:find>a</vibe:find><vibe:to>b</vibe:to></vibe:replace>", "stop"),
        ]).await;
        let d = run(port, |paths| {
            assert_eq!(paths, ["src/game.js".to_string()]);
            (vec![("src/game.js".to_string(), "SECRET_MARKER_CONTENT".to_string())], vec![])
        }).await;
        assert_eq!(d.retrievals, 1);
        assert_eq!(agent::parse_turn(&d.text).ops.len(), 1);
        assert!(!d.text.contains("vibe:read"), "the request is not part of the final reply");
        assert!(seen.lock().unwrap()[1].contains("SECRET_MARKER_CONTENT"));
    }

    #[tokio::test]
    async fn a_read_alongside_an_edit_keeps_the_edit() {
        let (port, seen) = mock(vec![sse(
            "<vibe:write path=\"a.js\">x</vibe:write><vibe:read path=\"b.js\" />", "stop",
        )]).await;
        let d = run(port, no_read).await;
        assert_eq!(seen.lock().unwrap().len(), 1, "no second request");
        assert_eq!(agent::parse_turn(&d.text).ops.len(), 1);
    }

    #[tokio::test]
    async fn a_model_that_only_ever_asks_to_read_cannot_spin_forever() {
        let replies: Vec<String> = (0..agent::MAX_RETRIEVALS + 1)
            .map(|_| sse("<vibe:read path=\"a.js\" />", "stop"))
            .collect();
        let (port, seen) = mock(replies).await;
        let d = run(port, no_read).await;
        assert_eq!(d.retrievals, agent::MAX_RETRIEVALS);
        assert_eq!(seen.lock().unwrap().len(), (agent::MAX_RETRIEVALS + 1) as usize);
    }

    #[tokio::test]
    async fn a_reply_that_never_closes_stops_at_the_limit_and_says_it_is_unfinished() {
        let replies: Vec<String> = (0..agent::MAX_CONTINUATIONS + 1)
            .map(|i| if i == 0 { sse("<vibe:write path=\"a.js\">x", "length") } else { sse("y", "length") })
            .collect();
        let (port, _) = mock(replies).await;
        let d = run(port, no_read).await;
        assert_eq!(d.continuations, agent::MAX_CONTINUATIONS);
        assert!(d.unfinished());
        assert!(agent::parse_turn(&d.text).ops.is_empty(), "half a file must never apply");
    }

    #[tokio::test]
    async fn usage_is_added_up_across_every_request_in_the_turn() {
        let (port, _) = mock(vec![
            sse("<vibe:write path=\"a\">x", "length"),
            sse("</vibe:write>", "stop"),
        ]).await;
        let d = run(port, no_read).await;
        assert_eq!(d.usage.prompt_tokens, 20);
        assert_eq!(d.usage.completion_tokens, 10);
    }

    #[tokio::test]
    async fn a_refused_request_reports_the_error_and_keeps_what_arrived() {
        let bad = "HTTP/1.1 401 Unauthorized\r\ncontent-length: 2\r\nconnection: close\r\n\r\nno".to_string();
        let (port, _) = mock(vec![bad]).await;
        let d = run(port, no_read).await;
        assert_eq!(d.error.as_ref().and_then(|e| e.status), Some(401));
    }
}
