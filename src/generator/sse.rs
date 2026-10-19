//! Minimal Server-Sent Events decoder, fed with arbitrary byte chunks.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    pub event: Option<String>,
    pub data: String,
}

#[derive(Debug, Default)]
pub struct SseDecoder {
    buffer: Vec<u8>,
}

impl SseDecoder {
    /// Feeds a chunk and returns every event it completed. Buffering bytes
    /// (not `String`) keeps multi-byte UTF-8 characters split across chunks intact.
    pub fn push(&mut self, chunk: &[u8]) -> Vec<SseEvent> {
        self.buffer.extend(chunk.iter().filter(|&&b| b != b'\r'));
        let mut events = Vec::new();
        while let Some(end) = self.buffer.windows(2).position(|w| w == b"\n\n") {
            let raw: Vec<u8> = self.buffer.drain(..end + 2).collect();
            if let Some(event) = parse_event(&String::from_utf8_lossy(&raw)) {
                events.push(event);
            }
        }
        events
    }
}

fn parse_event(block: &str) -> Option<SseEvent> {
    let mut event = None;
    let mut data: Vec<&str> = Vec::new();
    for line in block.lines() {
        if let Some(value) = line.strip_prefix("event:") {
            event = Some(value.trim().to_owned());
        } else if let Some(value) = line.strip_prefix("data:") {
            data.push(value.strip_prefix(' ').unwrap_or(value));
        }
    }
    (event.is_some() || !data.is_empty()).then(|| SseEvent {
        event,
        data: data.join("\n"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_events_split_across_chunks() {
        let mut decoder = SseDecoder::default();
        assert!(decoder.push(b"event: ping\nda").is_empty());
        let events = decoder.push(b"ta: {\"a\":1}\n\nevent: x\ndata: \xc3");
        assert_eq!(
            events,
            vec![SseEvent {
                event: Some("ping".into()),
                data: "{\"a\":1}".into()
            }]
        );
        // "é" = C3 A9, split across two chunks.
        let events = decoder.push(b"\xa9\r\n\r\n");
        assert_eq!(events[0].data, "é");
    }
}
