//! Live check against a real stdio MCP server: the `fetch` server Claude
//! Desktop also uses. Opt-in: needs `uvx` on PATH and network access.
//!
//! ```text
//! cargo test -p deskhand-mcp --test live -- --ignored --nocapture
//! ```

use std::collections::BTreeMap;

use deskhand_mcp::{StdioServer, connect_stdio, tools};
use deskhand_policy::{Authorization, ContextTaint, Source};
use deskhand_tools::Toolbox;
use serde_json::json;

#[test]
#[ignore = "starts `uvx mcp-server-fetch` and fetches https://example.com"]
fn fetch_server_over_stdio() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let server = StdioServer {
        command: "uvx".into(),
        args: vec!["mcp-server-fetch".into()],
        env: BTreeMap::new(),
    };
    let started = std::time::Instant::now();
    let session = rt.block_on(connect_stdio(&server)).expect("connect");
    println!("connected in {:?}", started.elapsed());

    let (list, skipped) = rt
        .block_on(tools("fetch", session, rt.handle().clone()))
        .expect("list tools");
    println!(
        "tools: {:?}, skipped: {skipped:?}",
        list.iter().map(|t| t.name()).collect::<Vec<_>>()
    );
    let mut tb = Toolbox::new();
    for t in list {
        tb.register(t).unwrap();
    }

    // Reaching the web is taint-sensitive regardless of tier.
    let mut tainted = ContextTaint::new();
    tainted.ingest(Source::Screen);
    assert!(matches!(
        tb.authorize("fetch__fetch", &json!({}), &tainted),
        Authorization::NeedsConfirmation(_)
    ));

    let mut taint = ContextTaint::new();
    let args = json!({ "url": "https://example.com", "max_length": 500 });
    let permit = match tb.authorize("fetch__fetch", &args, &taint) {
        Authorization::Permitted(p) => p,
        Authorization::NeedsConfirmation(p) => p.approve(),
        Authorization::Denied(r) => panic!("denied: {r:?}"),
    };
    let out = tb.execute(permit, args, &mut taint);
    let content = out.result.expect("fetch call");
    let text = content["text"].as_str().unwrap_or_default();
    println!("fetched {} chars: {:.120}", text.len(), text);
    assert!(text.contains("Example Domain"), "{text}");
    assert!(taint.is_tainted());
}
