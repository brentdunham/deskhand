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
        working_dir: None,
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

/// Playwright's MCP server, pinned to a release at least two weeks old,
/// using the installed Edge with an in-memory profile.
const PLAYWRIGHT_ARGS: &[&str] = &[
    "-y",
    "@playwright/mcp@0.0.83",
    "--browser",
    "msedge",
    "--isolated",
    "--headless",
    "--image-responses",
    "allow",
];

#[test]
#[ignore = "starts Playwright MCP with Edge, opens https://example.com, and takes a screenshot"]
fn playwright_screenshot_arrives_as_an_image() {
    let work = tempfile::tempdir().unwrap();
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let server = StdioServer {
        command: "npx".into(),
        args: PLAYWRIGHT_ARGS.iter().map(|s| s.to_string()).collect(),
        env: BTreeMap::new(),
        // Playwright saves screenshot files here; deleted with the TempDir.
        working_dir: Some(work.path().to_owned()),
    };
    let started = std::time::Instant::now();
    let session = rt.block_on(connect_stdio(&server)).expect("connect");
    println!("connected in {:?}", started.elapsed());
    let (list, skipped) = rt
        .block_on(tools("playwright", session, rt.handle().clone()))
        .expect("list tools");
    println!(
        "{} tools, skipped {skipped:?}: {:?}",
        list.len(),
        list.iter().map(|t| t.name()).collect::<Vec<_>>()
    );
    let mut tb = Toolbox::new();
    for t in list {
        tb.register(t).unwrap();
    }

    let mut taint = ContextTaint::new();
    let mut call = |tool: &str, args: serde_json::Value| {
        let permit = match tb.authorize(tool, &args, &taint) {
            Authorization::Permitted(p) => p,
            Authorization::NeedsConfirmation(p) => {
                println!("{tool}: needs confirmation (approved by the test)");
                p.approve()
            }
            Authorization::Denied(r) => panic!("{tool} denied: {r:?}"),
        };
        tb.execute(permit, args, &mut taint)
    };

    let nav = call(
        "playwright__browser_navigate",
        json!({ "url": "https://example.com" }),
    );
    assert!(nav.result.is_ok(), "{:?}", nav.result);

    let shot = call("playwright__browser_take_screenshot", json!({}));
    let content = shot.result.expect("screenshot");
    println!("screenshot content: {content}");
    assert_eq!(shot.images.len(), 1, "expected one image");
    let image = &shot.images[0];
    println!(
        "image: {} ({} base64 chars)",
        image.media_type,
        image.base64.len()
    );
    assert!(image.media_type.starts_with("image/"));
    assert!(image.base64.len() > 1000);
    assert_eq!(shot.untrusted, Some(Source::ToolOutput));
}
