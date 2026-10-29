//! Incremental Server-Sent Events parser (the subset model APIs use).

/// One dispatched event. `event` is empty when the stream sends no `event:` line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    pub event: String,
    pub data: String,
}

/// Feed raw bytes as they arrive; complete events come out. Handles events
/// split across chunks, CRLF line endings, multi-line `data:`, and comments.
#[derive(Debug, Default)]
pub struct SseParser {
    buf: Vec<u8>,
    event: String,
    data: Vec<String>,
}

impl SseParser {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, chunk: &[u8]) -> Vec<SseEvent> {
        self.buf.extend_from_slice(chunk);
        let mut out = Vec::new();
        while let Some(pos) = self.buf.iter().position(|&b| b == b'\n') {
            let mut line: Vec<u8> = self.buf.drain(..=pos).collect();
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            let line = String::from_utf8_lossy(&line);
            self.line(&line, &mut out);
        }
        out
    }

    fn line(&mut self, line: &str, out: &mut Vec<SseEvent>) {
        if line.is_empty() {
            if !self.data.is_empty() {
                out.push(SseEvent {
                    event: std::mem::take(&mut self.event),
                    data: self.data.join("\n"),
                });
                self.data.clear();
            }
            self.event.clear();
            return;
        }
        if line.starts_with(':') {
            return;
        }
        let (field, value) = line.split_once(':').unwrap_or((line, ""));
        let value = value.strip_prefix(' ').unwrap_or(value);
        match field {
            "event" => self.event = value.to_owned(),
            "data" => self.data.push(value.to_owned()),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_events_split_across_chunks() {
        let mut p = SseParser::new();
        assert!(p.push(b"event: ping\nda").is_empty());
        let events = p.push(b"ta: {\"a\":1}\n\ndata: x\n\n");
        assert_eq!(
            events,
            [
                SseEvent {
                    event: "ping".into(),
                    data: "{\"a\":1}".into()
                },
                SseEvent {
                    event: String::new(),
                    data: "x".into()
                },
            ]
        );
    }

    #[test]
    fn handles_crlf_comments_and_multiline_data() {
        let mut p = SseParser::new();
        let events = p.push(b": keepalive\r\ndata: a\r\ndata: b\r\n\r\n");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, "a\nb");
    }

    #[test]
    fn ignores_blank_lines_without_data() {
        let mut p = SseParser::new();
        assert!(p.push(b"\n\nevent: x\n\n").is_empty());
    }
}
