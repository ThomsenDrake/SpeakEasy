//! Integration tests for `UreqSlackClient` against the local mock Slack API.

mod common;

use common::{temp_state_dir, MockSlackServer, TEST_TOKEN};
use serde_json::json;
use slack_approval_gateway::core::SlackClient;
use slack_approval_gateway::slack::UreqSlackClient;
use std::ffi::OsString;
use std::sync::Arc;

#[test]
fn post_message_sends_exact_text_destination_and_thread_with_bearer_token() {
    let mock = MockSlackServer::start();
    let text = "Deploy *v1.2.3* finished :tada:\n  second line — ünïcödé & <@U123> \"quoted\"  ";

    mock.client()
        .post_message("C0123ABC", Some("1699999999.000200"), text)
        .expect("post_message should succeed");

    let posts = mock.requests_to("/chat.postMessage");
    assert_eq!(posts.len(), 1);
    let req = &posts[0];
    assert_eq!(req.method, "POST");
    assert_eq!(req.authorization.as_deref(), Some("Bearer xoxb-test-token"));
    assert!(
        req.content_type.as_deref().unwrap_or("").starts_with("application/json"),
        "content type was {:?}",
        req.content_type
    );
    // Whole-body equality: exact values and no extra keys.
    assert_eq!(
        req.body,
        json!({ "channel": "C0123ABC", "text": text, "thread_ts": "1699999999.000200" })
    );
    assert_eq!(req.body_str("text"), Some(text), "text must be sent verbatim");
}

#[test]
fn thread_ts_key_is_absent_when_none() {
    let mock = MockSlackServer::start();

    mock.client()
        .post_message("#general", None, "hello")
        .expect("post_message should succeed");

    let posts = mock.requests_to("/chat.postMessage");
    assert_eq!(posts.len(), 1);
    let body = posts[0].body.as_object().expect("JSON object body");
    assert!(!body.contains_key("thread_ts"), "body was {body:?}");
    assert_eq!(posts[0].body, json!({ "channel": "#general", "text": "hello" }));
}

#[test]
fn returns_slack_send_from_post_message_and_get_permalink() {
    let mock = MockSlackServer::start();

    let sent = mock
        .client()
        .post_message("C0123ABC", None, "status update")
        .expect("post_message should succeed");

    assert_eq!(sent.channel, "C0123ABC");
    assert_eq!(sent.ts, "1700000000.000001");
    assert_eq!(
        sent.permalink,
        "https://mock.slack.com/archives/C0123ABC/p1700000000000001"
    );

    // chat.postMessage first, then chat.getPermalink for the returned ts.
    let requests = mock.requests();
    let paths: Vec<&str> = requests.iter().map(|r| r.path.as_str()).collect();
    assert_eq!(paths, ["/chat.postMessage", "/chat.getPermalink"]);
    let permalink_req = &requests[1];
    assert_eq!(permalink_req.method, "POST");
    assert_eq!(permalink_req.authorization.as_deref(), Some("Bearer xoxb-test-token"));
    assert_eq!(
        permalink_req.body,
        json!({ "channel": "C0123ABC", "message_ts": "1700000000.000001" })
    );
    let query = permalink_req.query.as_deref().unwrap_or("");
    assert!(query.contains("channel=C0123ABC"), "query was {query:?}");
    assert!(query.contains("message_ts=1700000000.000001"), "query was {query:?}");
    assert_eq!(mock.post_count(), 1);
}

#[test]
fn channel_name_destination_uses_slack_resolved_channel_id() {
    let mock = MockSlackServer::start();
    let client = mock.client();

    // Slack resolves `#general` to a channel ID; the result and the permalink
    // lookup must use the ID, while the post itself carries the exact name.
    let sent = client.post_message("#general", None, "hello").expect("post_message should succeed");

    assert_eq!(sent.channel, "CGENERAL");
    assert_eq!(
        sent.permalink,
        format!("https://mock.slack.com/archives/CGENERAL/p{}", sent.ts.replace('.', ""))
    );
    assert_eq!(mock.requests_to("/chat.postMessage")[0].body_str("channel"), Some("#general"));
    let permalink_req = &mock.requests_to("/chat.getPermalink")[0];
    assert_eq!(permalink_req.body_str("channel"), Some("CGENERAL"));
    assert!(
        permalink_req.query.as_deref().unwrap_or("").contains("channel=CGENERAL"),
        "query was {:?}",
        permalink_req.query
    );

    // The constructed fallback link also needs the ID, not `#general`.
    mock.set_permalink_error(Some("message_not_found"));
    let sent = client.post_message("#general", None, "again").expect("post_message should succeed");
    assert!(
        sent.permalink
            .ends_with(&format!(".slack.com/archives/CGENERAL/p{}", sent.ts.replace('.', ""))),
        "{}",
        sent.permalink
    );
}

#[test]
fn unreadable_success_reply_warns_the_message_may_have_been_posted() {
    let mock = MockSlackServer::start();
    mock.set_post_message_raw_reply(Some("{\"ok\": tru"));

    let err = mock
        .client()
        .post_message("C0123ABC", None, "hello")
        .expect_err("an unreadable reply must be an error");

    assert!(err.0.contains("HTTP 200"), "error was {:?}", err.0);
    assert!(err.0.contains("may have taken effect"), "error was {:?}", err.0);
    assert!(mock.requests_to("/chat.getPermalink").is_empty());
}

#[test]
fn ok_false_maps_to_slack_error_with_slack_error_code() {
    let mock = MockSlackServer::start();
    mock.set_post_message_error(Some("channel_not_found"));

    let err = mock
        .client()
        .post_message("C_DOES_NOT_EXIST", None, "hello")
        .expect_err("ok:false must be an error");

    assert_eq!(err.0, "channel_not_found");
    assert_eq!(mock.post_count(), 1);
    assert!(
        mock.requests_to("/chat.getPermalink").is_empty(),
        "no permalink lookup after a failed post"
    );
}

#[test]
fn permalink_failure_falls_back_to_constructed_archives_url() {
    let mock = MockSlackServer::start();
    mock.set_permalink_error(Some("message_not_found"));

    let sent = mock
        .client()
        .post_message("C0123ABC", None, "hello")
        .expect("a permalink failure must not fail the send");

    assert_eq!(sent.channel, "C0123ABC");
    assert_eq!(sent.ts, "1700000000.000001");
    // The workspace domain comes from SLACK_WORKSPACE_DOMAIN, which the env
    // test below changes, so only the domain-independent parts are checked here.
    assert!(sent.permalink.starts_with("https://"), "{}", sent.permalink);
    assert!(
        sent.permalink
            .ends_with(".slack.com/archives/C0123ABC/p1700000000000001"),
        "{}",
        sent.permalink
    );
    assert_eq!(mock.post_count(), 1);
}

#[test]
fn http_error_status_maps_to_slack_error() {
    let mock = MockSlackServer::start();
    // The mock serves 404 for unknown paths.
    let client = UreqSlackClient::new(TEST_TOKEN, format!("{}/not-the-api", mock.base_url()));

    let err = client
        .post_message("C0123ABC", None, "hello")
        .expect_err("HTTP 404 must be an error");

    assert!(err.0.contains("404"), "error was {:?}", err.0);
    assert!(!err.0.contains(TEST_TOKEN), "the token must never appear in errors");
    assert_eq!(mock.requests().len(), 1);
}

#[test]
fn transport_error_maps_to_slack_error() {
    // Grab a free port, then close it so the connection is refused.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    let client = UreqSlackClient::new(TEST_TOKEN, format!("http://{addr}"));

    let err = client
        .post_message("C0123ABC", None, "hello")
        .expect_err("connection refused must be an error");

    assert!(err.0.starts_with("chat.postMessage:"), "error was {:?}", err.0);
    assert!(!err.0.contains(TEST_TOKEN), "the token must never appear in errors");
}

#[test]
fn new_strips_trailing_slash_and_debug_redacts_token() {
    let mock = MockSlackServer::start();
    let client = UreqSlackClient::new(TEST_TOKEN, format!("{}/", mock.base_url()));

    client.post_message("C1", None, "hi").expect("post_message should succeed");

    assert_eq!(mock.requests()[0].path, "/chat.postMessage");
    let debug = format!("{client:?}");
    assert!(!debug.contains(TEST_TOKEN), "Debug output leaked the token: {debug}");
}

#[test]
fn concurrent_posts_each_get_their_own_result() {
    let mock = MockSlackServer::start();
    let client = Arc::new(mock.client());

    let handles: Vec<_> = (0..8)
        .map(|i| {
            let client = Arc::clone(&client);
            std::thread::spawn(move || {
                let text = format!("message number {i}");
                let sent = client
                    .post_message(&format!("C{i}"), None, &text)
                    .expect("post_message should succeed");
                (i, sent)
            })
        })
        .collect();
    let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();

    assert_eq!(mock.post_count(), 8);
    let posts = mock.requests_to("/chat.postMessage");
    for (i, sent) in &results {
        assert_eq!(sent.channel, format!("C{i}"));
        assert_eq!(
            sent.permalink,
            format!("https://mock.slack.com/archives/C{i}/p{}", sent.ts.replace('.', ""))
        );
        let req = posts
            .iter()
            .find(|r| r.body_str("channel") == Some(sent.channel.as_str()))
            .expect("a recorded post for this channel");
        assert_eq!(req.body_str("text"), Some(format!("message number {i}").as_str()));
    }
    let mut ts: Vec<_> = results.iter().map(|(_, s)| s.ts.clone()).collect();
    ts.sort();
    ts.dedup();
    assert_eq!(ts.len(), 8, "every post must get a distinct ts");
}

/// Restores the given environment variables when dropped (even on panic).
struct EnvGuard(Vec<(&'static str, Option<OsString>)>);

impl EnvGuard {
    fn save(names: &[&'static str]) -> Self {
        Self(names.iter().map(|n| (*n, std::env::var_os(n))).collect())
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (name, value) in &self.0 {
            match value {
                Some(v) => std::env::set_var(name, v),
                None => std::env::remove_var(name),
            }
        }
    }
}

/// The only test in this binary that touches environment variables, so all
/// env-dependent checks live here and run sequentially.
#[test]
fn from_env_reads_token_file_api_base_and_workspace_domain() {
    let _guard = EnvGuard::save(&["SLACK_BOT_TOKEN", "SLACK_API_BASE", "SLACK_WORKSPACE_DOMAIN"]);
    let mock = MockSlackServer::start();
    std::env::remove_var("SLACK_BOT_TOKEN");
    std::env::remove_var("SLACK_WORKSPACE_DOMAIN");
    // Trailing slash must be stripped.
    std::env::set_var("SLACK_API_BASE", format!("{}/", mock.base_url()));

    // No env token and no token file: error.
    let dir = temp_state_dir("from-env");
    let err = UreqSlackClient::from_env(&dir).expect_err("no token anywhere must be an error");
    assert!(err.0.contains("SLACK_BOT_TOKEN"), "error was {:?}", err.0);

    // Blank token file: error.
    let token_path = dir.join("slack_token");
    std::fs::write(&token_path, "  \n").unwrap();
    assert!(UreqSlackClient::from_env(&dir).is_err(), "blank token file must be an error");

    // Token file, trimmed of surrounding whitespace.
    std::fs::write(&token_path, "  xoxb-from-file \n\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&token_path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    let client = UreqSlackClient::from_env(&dir).expect("token file should be used");
    client.post_message("C1", None, "from file").expect("post_message should succeed");
    let req = mock.requests().into_iter().next().expect("a request");
    assert_eq!(req.path, "/chat.postMessage");
    assert_eq!(req.authorization.as_deref(), Some("Bearer xoxb-from-file"));

    // SLACK_BOT_TOKEN takes precedence over the file.
    std::env::set_var("SLACK_BOT_TOKEN", "xoxb-from-env");
    let client = UreqSlackClient::from_env(&dir).expect("env token should be used");
    client.post_message("C2", None, "from env").expect("post_message should succeed");
    let last = mock.requests_to("/chat.postMessage").pop().unwrap();
    assert_eq!(last.authorization.as_deref(), Some("Bearer xoxb-from-env"));

    // Fallback permalink uses SLACK_WORKSPACE_DOMAIN, else "workspace".
    mock.set_permalink_error(Some("message_not_found"));
    std::env::set_var("SLACK_WORKSPACE_DOMAIN", "acme");
    let sent = client.post_message("C3", None, "domain").unwrap();
    assert_eq!(
        sent.permalink,
        format!("https://acme.slack.com/archives/C3/p{}", sent.ts.replace('.', ""))
    );
    std::env::remove_var("SLACK_WORKSPACE_DOMAIN");
    let sent = client.post_message("C4", None, "no domain").unwrap();
    assert_eq!(
        sent.permalink,
        format!("https://workspace.slack.com/archives/C4/p{}", sent.ts.replace('.', ""))
    );
}

/// Checks the ~15s request timeout against a server that never replies.
/// Slow, so ignored by default: `cargo test --test slack_client_test -- --ignored`.
#[test]
#[ignore = "takes about 15 seconds"]
fn unresponsive_server_times_out() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    // Accept and hold connections open without replying.
    std::thread::spawn(move || {
        let mut held = Vec::new();
        for stream in listener.incoming().flatten() {
            held.push(stream);
        }
    });
    let client = UreqSlackClient::new(TEST_TOKEN, format!("http://{addr}"));

    let started = std::time::Instant::now();
    let err = client
        .post_message("C1", None, "hello")
        .expect_err("an unresponsive server must time out");

    let elapsed = started.elapsed();
    assert!(elapsed.as_secs() >= 10 && elapsed.as_secs() <= 30, "took {elapsed:?}");
    assert!(err.0.contains("no reply from Slack"), "error was {:?}", err.0);
}
