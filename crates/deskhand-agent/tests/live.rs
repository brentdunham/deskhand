//! Live end-to-end tests against real model endpoints and the real window
//! tools. Opt-in, read-only (the prompt only lists windows; any confirmation
//! request is declined), and skipped when the endpoint is not configured.
//!
//! ```text
//! $env:ANTHROPIC_API_KEY = "..."
//! cargo test -p deskhand-agent --test live -- --ignored --nocapture
//!
//! # Optional, with Ollama running and a tool-capable model pulled:
//! $env:DESKHAND_OLLAMA_MODEL = "qwen3"
//! ```
#![cfg(windows)]

use std::sync::{Arc, Mutex};

use deskhand_agent::anthropic::{Anthropic, AnthropicConfig};
use deskhand_agent::openai::{OLLAMA_BASE_URL, OpenAiCompatible, OpenAiConfig};
use deskhand_agent::{
    Agent, AgentConfig, AgentEvent, AgentHost, ConfirmationRequest, Provider, Session, StopReason,
};
use deskhand_tools::{Toolbox, window_tools};

const PROMPT: &str =
    "Use the list_windows tool once, then reply with only the number of windows it returned.";

#[derive(Default)]
struct Recorder {
    events: Mutex<Vec<AgentEvent>>,
}

impl AgentHost for Recorder {
    fn on_event(&self, event: AgentEvent) {
        if let AgentEvent::TextDelta(t) = &event {
            print!("{t}");
        }
        self.events.lock().unwrap().push(event);
    }

    async fn confirm(&self, request: ConfirmationRequest) -> bool {
        println!("\n[declining confirmation for {}]", request.tool);
        false
    }
}

async fn run(provider: impl Provider) {
    let mut toolbox = Toolbox::new();
    window_tools::register(&mut toolbox).unwrap();
    let agent = Agent::new(
        provider,
        Arc::new(Mutex::new(toolbox)),
        AgentConfig::default(),
    );
    let host = Recorder::default();
    let mut session = Session::new();

    let stop = agent.run_turn(&mut session, PROMPT, &host).await.unwrap();
    println!();

    assert_eq!(stop, StopReason::EndTurn);
    let events = host.events.lock().unwrap();
    assert!(
        events.iter().any(|e| matches!(
            e,
            AgentEvent::ToolFinished { tool, result: Ok(_), .. } if tool == "list_windows"
        )),
        "model did not call list_windows successfully: {events:#?}"
    );
    assert!(session.taint().is_tainted());
}

fn client() -> reqwest::Client {
    reqwest::Client::new()
}

#[tokio::test]
#[ignore = "calls the Anthropic API; set ANTHROPIC_API_KEY"]
async fn anthropic_lists_windows() {
    let Ok(key) = std::env::var("ANTHROPIC_API_KEY") else {
        eprintln!("ANTHROPIC_API_KEY not set; skipping");
        return;
    };
    run(Anthropic::new(client(), AnthropicConfig::new(key))).await;
}

#[tokio::test]
#[ignore = "calls a local Ollama server; set DESKHAND_OLLAMA_MODEL"]
async fn ollama_lists_windows() {
    let Ok(model) = std::env::var("DESKHAND_OLLAMA_MODEL") else {
        eprintln!("DESKHAND_OLLAMA_MODEL not set; skipping");
        return;
    };
    run(OpenAiCompatible::new(
        client(),
        OpenAiConfig::new(OLLAMA_BASE_URL, model),
    ))
    .await;
}
